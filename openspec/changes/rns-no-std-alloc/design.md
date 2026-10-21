# RNS allocation-backed portability design

## Status and ownership

This design tracks supervised implementation of issue #78. The owning component
is `crates/libs/styrene-rns`. The [execution plan](execution-plan.md) defines
implementation authority, parallel ownership, and acceptance gates.

The base is `ab46aa1f13de5663c684225dc27dfcba330d965f`. The focused branch must not
absorb unrelated reconciliation history. Existing product and Identity work
remain independently owned.

## Existing architecture

`src/lib.rs` conditionally declares `no_std` and always imports `alloc`.
`Cargo.toml` defaults to `std`. The `alloc` feature is currently an empty compatibility
feature. `transport` implies `std`. Neither defaults-disabled profile is
allocation-free.

The intended boundary is:

```text
embedding: allocator + cryptographic RNG + supplied Unix time
                         |
                         v
           RNS allocation-backed protocol core
                         ^
                         |
std feature: OS conveniences + filesystem persistence
transport feature: std + hosted transport/runtime dependencies
```

The current manifest does not enforce that boundary. `rmp-serde` requires `std`.
Its RNS uses already reside in standard-library-dependent code. Other inherited
dependency defaults enable `std` independently.

Core RNG injection already exists for many operations. Remaining OS assumptions
include group encryption in `src/destination/primitives.rs`, rotation in
`src/destination/ratchet.rs`, and RNG-backed construction on decryption paths.
`src/crypt/fernet.rs` already contains RNG-free authenticated decryption helpers.

`src/time_source.rs` implements supplied Unix time with `AtomicBool` and
`AtomicU64`. Callback ownership also requires pointer-width atomics. Selecting a
target without those facilities would add synchronization work to this scope.

## Proposed dependency boundary

| Dependency | Allocation-backed core | Hosted feature |
|---|---|---|
| `rmp-serde`, `serde_bytes` | Excluded; existing uses are persistence/transport | Optional dependencies enabled by `std` |
| `ed25519-dalek` | Disable defaults; retain `alloc`, `fast`, `zeroize`, and `rand_core` | Forward its `std` feature |
| `hex` | Disable defaults; retain `alloc` | Forward its `std` feature |
| `serde` | Disable defaults; retain `alloc` and `derive` | Forward its `std` feature |
| `sha2` | Disable defaults | Forward its `std` feature |
| `rand_core` | RNG traits without an OS entropy backend | Enable the existing OS-backed capability |

Do not disable every dependency default indiscriminately. For example, X25519
defaults include zeroization rather than `std`. Review resolved normal/build
edges separately from dev-dependencies and host-only procedural macros.

Keep the change RNS-local where possible. Edition-2024 Cargo rejects disabling
defaults on an inherited dependency whose workspace declaration enables them.
Explicit RNS-local requirements can preserve the existing version constraints
without changing other consumers' defaults. Review the lockfile and policy
checks. Do not introduce an unrelated dependency upgrade.

Preserve additive feature relationships: default enables `std`, and transport
enables `std`. Keep both no-default and explicit-`alloc` profiles allocation-backed.
Do not introduce a mutually exclusive `no_std` feature or reinterpret no-default
as allocation-free operation.

## Approved entropy and API boundary

Complete the caller-supplied cryptographic RNG path instead of adding a global
entropy service. Keep current hosted convenience APIs under `std`. Add explicit
RNG entry points where an existing convenience API has no RNG argument.

Pass the supplied RNG through ratchet rotation rather than creating `OsRng`
inside that operation. Reuse authenticated decryption that does not construct
or invoke an RNG. Preserve authentication before plaintext decryption.

D3 uses borrowed advancing state for portable entry points while preserving
existing public trait signatures and hosted callers. A copied PRNG state can
repeat randomness. A copyable handle to advancing provider state is different.

The approved API contract is:

- Add `group_encrypt_with_rng<R: CryptoRngCore + ?Sized>(key, data, rng: &mut R)`.
  Keep `group_encrypt(key, data)` under `std` as an OS-backed wrapper. Keep
  `group_decrypt` available without an RNG. Update both existing export locations.
- Add inherent `encrypt_with_rng` methods to `Identity`, `PrivateIdentity`, and
  `EmptyIdentity`, with borrowed RNG, existing data/key arguments, and unchanged
  output-buffer lifetimes and errors.
- Add RNG-free inherent `decrypt_with_derived_key` methods to `PrivateIdentity`
  and `EmptyIdentity` with existing framing, buffer, and error behavior.
- Preserve exported `EncryptIdentity` and `DecryptIdentity` trait signatures,
  including their legacy `Copy` bounds. Do not add required trait methods.
  Delegate concrete implementations to the new paths without requesting entropy
  during decryption.
- Remove unnecessary `Copy` bounds from inherent/free key derivation, ratchet
  encryption, announce/path-response, and `CachedFernet::encrypt` functions.
  Preserve names, argument order, and returns. Callers can pass `&mut rng`.
- Add a borrowed RNG argument to private `RatchetState::rotate_if_needed` and
  reborrow the caller's state across every multi-step operation.
- Preserve immutable legacy `Fernet::encrypt(&self, ...)` and its copyable-provider
  contract. Use `CachedFernet::encrypt` for portable mutable-provider encryption.
  Factor a private borrowed-RNG helper if needed, without adding interior mutability.

Document that legacy copyable-provider APIs are not safe abstractions for copying
PRNG state snapshots. The supported advancing-state path must accept a deliberately
non-`Copy`, non-`Clone` provider. Include a downstream legacy-trait implementation
compile test to prevent accidental signature breakage.

The embedding must supply a healthy, initialized cryptographic provider. Existing
infallible RNG operations may panic on provider failure. A surrounding `Result`
does not promise typed recovery from entropy loss. Never substitute weak or
deterministic entropy in production when an entropy source is unavailable.

Preserve existing key slicing, error mapping, authentication order, and secret
ownership during refactoring. Do not introduce unrelated cryptographic corrections
or stronger zeroization claims without separate evidence and review.

## Platform responsibilities and limits

The embedding supplies allocation, startup/linking, panic policy, a suitable RNG,
and updates to the existing Unix-time interface. This work does not promise
recoverable allocation failure or eliminate heap allocation.

Preserve the [embedded-time baseline](../../baseline/embedded-time.md).
Uninitialized or cleared time must remain an explicit error for time-dependent
operations. Do not replace it with zero timestamps or a hidden host clock.
Document clock update/reset concurrency assumptions. This change does not
silently introduce an atomic transaction across separate clock atomics.

A target with 64-bit and pointer-width atomics avoids a clock redesign. The
selected `aarch64-unknown-none` target is not a board or device acceptance claim.
The user approved the atomic-capable target scope and excluded Cortex-M.

The core does not provide embedded Tokio transport, filesystem persistence, or
the existing lock-based hosted key-manager implementations. Generic key-backend
contracts can remain available for embedding-owned adapters.

## Compatibility obligations

Preserve the [RNS security baseline](../../baseline/rns-security-hardening.md)
and [interop evidence baseline](../../baseline/interop-verification.md).
Retain constant-time authentication, verify-before-decrypt behavior, private
atomic persistence, and key-fallback error classification.

Use the existing canonical RNS fixture index and shared loader. Do not copy
vectors or create another authoritative protocol corpus. MessagePack remains
where RNS uses it. No wire or persistence migration is planned.

Changing RNG plumbing can change which entropy source a call consumes. Compare
deterministic fixtures with controlled test providers, but do not require random
outputs to match an earlier OS-generated sequence.

## Validation architecture

[The test plan](test-plan.md) owns command templates, pass criteria, and evidence
fields. The delta specification owns behavior, not the test runner's implementation.

Use an independent consumer with no workspace siblings or RNS dev-dependencies
to prove the target dependency boundary. The test fixture must select resolver 3,
edition 2024, the repository Rust requirement, and non-publication. Review its
workspace-policy treatment before adding it. Do not lower the MSRV to make it run.

Existing host tests remain necessary for behavior and compatibility. They are
insufficient as the target gate because dev-dependencies can restore dependency
features. Host-side build tools may use `std`. The target library graph may not.

Separate library compilation, executable linking, synthetic target execution,
and physical-device acceptance. The new target job must fail if a negative-control
consumer introduces target-side `std`. Run that control outside product sources.

## Open decisions

| ID | Decision | Proposed direction | Closure condition |
|---|---|---|---|
| D1 — resolved | Initial target and atomic requirements | User approved atomic-capable `aarch64-unknown-none`; Cortex-M excluded | Retain the compiler target configuration with validation evidence; target execution is still unverified |
| D2 | Executable smoke-test target and runner | Reuse D1 with an owned emulator/harness where practical | Record runner, allocator/startup approach, pinned toolchain, and bounded execution/cleanup procedure |
| D3 — resolved | Public RNG API and failure contract | Coordinator approved the borrowed-state and legacy-compatibility contract above after source review | Independent implementation review and tests must verify non-Copy callers, legacy trait compatibility, failure semantics, and unchanged protocol behavior |

D1 approval does not prove environment availability or resolve D2. Provisioning
a missing toolchain target or emulator requires an explicit environment action.
The Nucleus host configuration is outside this product change.

## Delivery sequence

1. Resolve the affected open decisions and retain a failing isolated baseline.
2. Repair dependency isolation and complete entropy plumbing in reviewable commits.
3. Add consumer and target gates while retaining hosted and protocol regression coverage.
4. Review evidence for every case before proposing issue closure or archival.

Do not close the issue solely because `rmp-serde` disappears from a host graph.
Do not archive the OpenSpec change while required target evidence is blocked.
