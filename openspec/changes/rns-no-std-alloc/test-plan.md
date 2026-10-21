# RNS no-std + alloc acceptance plan

This is a planned acceptance corpus for [issue #78](https://github.com/styrene-lab/styrene-rs/issues/78).
This plan defines verification for [the delta spec](specs/rns-core-portability.md).
Design decisions belong in [design.md](design.md). Execution work belongs in [tasks.md](tasks.md).

- Planning base: `ab46aa1f13de5663c684225dc27dfcba330d965f`.
- Planning branch: `fix/rns-no-std-78`.
- All initial case results are **NOT RUN** or **BLOCKED**. This document records no acceptance passes.
- [verification.md](verification.md) records environment readiness and reviewed execution evidence.
- **NOT RUN** means no case execution exists. **BLOCKED** identifies an unresolved prerequisite.
- Historical diagnostics below cannot satisfy a case against a changed revision.

## Preconditions and ownership

1. Use the approved D3 borrowed-state API and provider failure contract in [design.md](design.md#approved-entropy-and-api-boundary).
   Any API deviation requires coordinator review before implementation.
2. Use the user-approved `aarch64-unknown-none` target and the independently reviewed D2 QEMU harness contract.
   Target selection does not imply that its libraries or runner are installed.
   Record allocator, pointer-width/64-bit atomic support and clock assumptions against design decisions D1–D3.
3. Resolve Cargo and rustc to the installed, matching Rust **1.97.0** toolchain.
   Confirm target libraries, linker and runner availability separately. Missing prerequisites remain BLOCKED.
4. Do not install tools or targets implicitly. Do not change Nucleus system or Nix configuration through this plan.
   A later provisioning action needs explicit authorization and the owning configuration procedure.
5. Use an isolated Cargo home with reviewed cached dependencies and an external target directory.
   The commands below use `--offline --locked`; an unavailable cache or lockfile is a blocker.
6. Place the independent consumer outside the product workspace, with its own workspace and lockfile.
   Use resolver 3, edition 2024, Rust 1.97 and `publish = false`.
   Inherit edition/MSRV from its own workspace settings, or record an approved test-fixture policy exception.
   Review its path dependency, version constraint, feature forwarding and resolved sources before execution.
7. Decide and review the fixture's own lockfile policy before using `--locked`.
   The product lockfile does not govern an independent consumer. Do not silently generate or replace either lockfile.
8. Nucleus owns Linux checks and retained local evidence. Apple checks belong to the Apple host.
   Changes affecting Apple consumers require separately attributed Apple evidence.
9. The executor owns a unique `/tmp/opencode/` run directory and its cleanup.
    Retain source-controlled harness definitions separately from generated logs, binaries and caches.
    Do not commit build outputs, credentials, private identities or live traffic captures.
10. Preserve reviewed run evidence in the durable store defined by [execution-plan.md](execution-plan.md#evidence-custody).
    Scratch cleanup must not remove the only copy of acceptance evidence.

## Acceptance matrix

`RNS src` means `crates/libs/styrene-rns/src/`. `RNS tests` means `crates/libs/styrene-rns/tests/`.
Other paths are repository-relative. **Proposed** loci do not exist as implemented test APIs.

| Case | Requirement | Test locus | Objective pass criteria | Initial status |
|---|---|---|---|---|
| RNS-NOSTD-001 | Independent minimal dependency graph | **Proposed:** standalone consumer, RNS dependency with defaults disabled; resolved normal/build feature trees | Target library graph has no `std`, `rmp-serde`, `serde_bytes`, transport stack or OS `getrandom` dependency. Host proc macros/build scripts may use `std`; classify edges rather than rejecting every textual `std` occurrence. | NOT RUN |
| RNS-NOSTD-002 | Explicit allocation-backed profile | **Proposed:** same consumer with `alloc` forwarded to RNS | Minimal and explicit-alloc configurations expose the agreed allocation-backed contract and satisfy the same target graph exclusions. Neither claims an allocation-free profile. | NOT RUN |
| RNS-NOSTD-003 | Real std-less target compilation | **Proposed:** independent consumer library on approved target | Pinned Rust 1.97.0 checks succeed for minimal, `alloc`, and `alloc,fernet-aes128`. Capture actual target/compiler invocation. A native-host build cannot substitute. | BLOCKED: fixture and integrated implementation |
| RNS-NOSTD-004 | Caller-owned cryptographic randomness | Existing RNS src: `identity.rs`, `crypt/fernet.rs`, `destination/`, `ratchets.rs`; **proposed:** provider instrumentation | Non-Copy caller RNG reaches identity generation, ephemeral keys, IV/group encryption, announce random bytes and ratchet rotation. State advances across calls without duplicated-state replay. Approved provider failure behavior and legacy trait compatibility are exercised. Test RNGs remain synthetic-only. | NOT RUN |
| RNS-NOSTD-005 | Randomness-free authenticated decryption | Existing: Fernet, identity, group and ratchet tests; **proposed:** zero-request instrumentation | Every decrypt outcome makes zero RNG requests. Malformed, wrong-key, tampered, truncated and undersized-buffer cases preserve established error and output-buffer behavior where applicable. Authentication failure exposes no unauthenticated plaintext. | NOT RUN |
| RNS-NOSTD-006 | Embedded-time behavior | Existing: `crates/libs/styrene-rns/tests/embedded_time.rs`; proposed provider adaptation | Uninitialized or cleared time produces the existing typed error. Supplied/advanced time drives announce timestamps and ratchet rotation consistently. Existing unavailable-time and interval assertions remain effective. | NOT RUN |
| RNS-NOSTD-007 | Security and wire compatibility | Existing RNS tests: `common/mod.rs`, `fixture_index.rs`, `interop_*.rs`; crypto tests and shared interop runner | Canonical loader/index and digest-checked fixtures remain authoritative. Verify-before-decrypt and the constant-time verifier remain intact. Signature/tag, truncation and padding rejection cases pass for applicable default/alternate Fernet profiles. | NOT RUN |
| RNS-NOSTD-008 | Hosted feature/API compatibility | Existing: RNS tests and feature checks; **proposed:** profile assertions | Default and explicit `std` preserve hosted APIs; `transport` implies `std`; `serial` implies transport. Linux checks pass. Required Apple checks have separate evidence. Compilation establishes no serial-device or radio acceptance. | NOT RUN |
| RNS-NOSTD-009 | Hosted persistence and key fallback | Existing RNS src: `key_manager.rs`, `private_file.rs`, destination and transport ratchet-store tests | Existing persistence bytes remain readable and encoding-compatible. Private atomic-write behavior is retained. Key fallback remains restricted to established availability failures; rejection cannot silently become fallback. | NOT RUN |
| RNS-NOSTD-010 | Direct consumers and retained crypto features | Existing: LXMF, mesh, tunnel and daemon checks; **proposed:** resolved-feature review | Affected direct consumers compile with intended hosted features. Review confirms retained zeroization and other required crypto capabilities, including explicitly restored non-std defaults. No whole-stack no-std claim follows. | NOT RUN |
| RNS-NOSTD-011 | Independent consumer link and execution | **Proposed:** isolated std-less smoke binary, allocator, supplied clock and approved provider/runner | Binary links and executes bounded encoding, signing, encryption, decryption and ratchet assertions on the chosen target/runner. Record allocator/provider/clock setup and result. Report compile, link, emulated execution and hardware evidence separately. Never substitute a host run. | BLOCKED: harness implementation, integrated tests and code review |
| RNS-NOSTD-012 | CI gate validity and attributable evidence | Existing: `.github/workflows/ci.yml`, `justfile`; **proposed:** independent true-target job and disposable negative control | Positive consumer builds pass; enabling target `std` in the negative control fails for the intended std-less-target reason. Capture complete source/environment records. Failed, missing or blocked lanes cannot count as passes. | NOT RUN |

## Coverage details

- For 004, exercise repeated operations through one stateful provider and record request consumption.
  Detect copied/restarted provider state. A round-trip test alone cannot establish safe RNG ownership.
  Use a non-`Copy`, non-`Clone` provider and retain the D3 failure expectations.
  Compile an external implementation of the legacy identity traits and an immutable Fernet caller.
- For 005, instrument or trap RNG requests, including nested group and ratchet decryption paths.
  Record return values and caller-visible buffer contents for each applicable failure class.
- For 007, reuse the shared `styrene_interop_runner::rns_fixtures` loader and existing index.
  Do not copy vectors or create a competing canonical corpus.
  Combine negative tests with review of authentication order and constant-time verification primitives.
  Timing measurements alone do not prove constant-time behavior.
- For 009, compare known pre-change encodings and readability, not only new-code self-round-trips.
  Keep persistence tests isolated from operator stores and identities.
- For 011, a deterministic RNG is permitted only in an explicitly synthetic, non-production harness.
  Use no live identity, peer, network traffic or hardware operation in that harness.
  A linked artifact is not an execution result. An emulator result is not hardware acceptance.
  Review the fixture-only unsafe boundary against D2 without weakening product lints.
  Cover allocator alignment, exact fit, overflow/exhaustion, and live-allocation non-overlap.
  Exercise allocation-free panic/OOM failures and reject marker-only, zero-exit-only, and timeout outcomes.
  Semihosting must be limited to fixed output and termination, not host filesystem access.

## Existing focused commands — verified command surfaces, not new results

Run from the selected product source after the preconditions hold.
Resolve `cargo`/`rustc` to the matching pinned toolchain and export the approved external
`CARGO_HOME` and `CARGO_TARGET_DIR`. These commands do not provision prerequisites.

```sh
cargo check --offline --locked --lib -p styrene-rns --no-default-features
cargo clippy --offline --locked --lib --no-default-features --no-deps -p styrene-rns -- -D warnings
cargo test --offline --locked --no-default-features -p styrene-rns --test embedded_time
cargo test --offline --locked -p styrene-rns
cargo test --offline --locked -p styrene-rns --test fixture_index
cargo test --offline --locked -p styrene-rns --features interop-tests,transport
cargo test --offline --locked -p styrene-rns --features fernet-aes128 --lib crypt::fernet
cargo test --offline --locked -p styrene-interop-runner --test rns_fixtures --test rns_handoff_manifests --test pinned_evidence_record
```

The following use existing feature/package names for planned hosted matrix checks:

```sh
for profile in std alloc transport serial; do
  cargo check --offline --locked --lib -p styrene-rns --no-default-features --features "$profile" || exit
done
cargo check --offline --locked -p styrene-lxmf -p styrene-mesh -p styrene-tunnel -p styrened
```

Each invocation needs its own recorded exit status. Stop on failure rather than hiding it in a loop's final status.
The unit-test command above has one positional filter. Add separate invocations for other filters.
Repository-wide tests and `--all-targets` are useful hosted regressions, not independent no-std proof.
Dev-dependencies and other selected consumers can unify `std` features back into the graph.

## TEMPLATE: future independent consumer and true-target checks

These templates are **UNBOUND** until the fixture, target and environment are approved.
The proposed fixture forwards `alloc` and `fernet-aes128` to RNS, disables RNS defaults,
and has no default features of its own. No such harness is claimed implemented here.

```sh
: "${PINNED_CARGO:?UNBOUND: existing Cargo 1.97.0 executable}"
: "${RUSTC:?UNBOUND: matching Rust 1.97.0 rustc executable}"
: "${CONSUMER_MANIFEST:?UNBOUND: approved independent fixture Cargo.toml}"
: "${TARGET:?UNBOUND: approved std-less target with installed libraries}"
: "${CARGO_HOME:?UNBOUND: isolated reviewed cache}"
: "${CARGO_TARGET_DIR:?UNBOUND: external build-output directory}"
export RUSTC CARGO_HOME CARGO_TARGET_DIR
"$PINNED_CARGO" tree --offline --locked --manifest-path "$CONSUMER_MANIFEST" \
  --no-default-features --target "$TARGET" -e normal,build,features
"$PINNED_CARGO" check --offline --locked --manifest-path "$CONSUMER_MANIFEST" \
  --lib --no-default-features --target "$TARGET"
for profile in alloc alloc,fernet-aes128; do
  "$PINNED_CARGO" check --offline --locked --manifest-path "$CONSUMER_MANIFEST" \
    --lib --no-default-features --features "$profile" --target "$TARGET" || exit
done
```

Repeat the tree capture with `--features alloc` for 002 and retain both resolved graphs.
Do not accept every build failure as a regression demonstration: identify its causal diagnostic.

### TEMPLATE: link/execution and negative control

Linking requires an approved allocator, startup/panic arrangement, linker and memory layout.
The separate smoke manifest must forward the planned `alloc` feature and use the approved provider.

```sh
: "${SMOKE_MANIFEST:?UNBOUND: approved std-less executable manifest}"
: "${SMOKE_BIN:?UNBOUND: approved smoke binary target name}"
"$PINNED_CARGO" build --offline --locked --manifest-path "$SMOKE_MANIFEST" \
  --bin "$SMOKE_BIN" --no-default-features --features alloc --target "$TARGET"
```

Execution command: **UNBOUND**. Approve the emulator/runner invocation, exact linked artifact,
bounded timeout and expected result record before execution. No runner script or API is assumed.

For 012, use a disposable copy of the fixture, never a product-source mutation.
Its proposed `std-negative-control` feature enables target `std` and deliberately references it.

```sh
: "${NEGATIVE_MANIFEST:?UNBOUND: approved disposable negative-control manifest}"
"$PINNED_CARGO" check --offline --locked --manifest-path "$NEGATIVE_MANIFEST" \
  --lib --no-default-features --features std-negative-control --target "$TARGET"
```

The negative command must fail specifically because target `std` is unavailable or forbidden.
A missing lockfile, missing `core`, unavailable package or malformed manifest does not validate the gate.

## Regression-first sequence and closure

1. Approve blocked design/environment decisions and review fixture policy and lockfiles.
2. Capture the base revision's minimal graph and the smallest true-target failure with its cause.
   Capture hosted security, persistence and embedded-time behavior before changing those paths.
3. Add focused assertions for provider state, zero-randomness decrypt and existing failure outputs.
   Reuse canonical fixtures and retain baseline observations beside the future changed-source results.
4. After implementation, repeat the minimal case first, then its explicit-alloc and alternate-Fernet variants.
5. Run applicable hosted regressions, direct consumers, link/execution smoke and the gate's negative control.
6. Review every case against the spec. Close only when required cases have attributable passing evidence.
   A blocked required case keeps the change open. Any scope revision needs an explicit reviewed spec decision.

For every execution, retain: case ID, UTC time, host, source SHA, dirty/untracked state and content
fingerprint, product/fixture lock hashes, fixture revision/hash, resolved dependency sources,
Cargo/rustc versions, enabled features, target triple, exact command/cwd/environment overrides,
exit status, logs and artifact hashes. Record runner provenance and evidence class for 011.
Record blocked reasons and missing prerequisites explicitly. Do not retain credentials or secret material.

## Historical diagnostics — not acceptance results

The earlier assessment used an export of `ab46aa1f13de5663c684225dc27dfcba330d965f`,
with unchanged lockfile, under `/tmp/opencode/styrene-issue78-sxkdtf8r`.
Its Rust 1.97.0 native RNS no-default check succeeded. Its graphs still enabled target `std` dependencies.
A dependency-only Rust 1.98.1 `wasm32v1-none` check failed in `num-traits` for missing `std`.
That was neither the pinned-toolchain RNS target check nor a linked/executed consumer smoke.
Canonical CI run [33833175316](https://github.com/styrene-lab/styrene-rs/actions/runs/33833175316)
passed hosted checks on that SHA. These observations establish context only. All case statuses above remain unchanged.
