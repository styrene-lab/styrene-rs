# RNS no-std + alloc acceptance plan

This corpus defines acceptance criteria for [issue #78](https://github.com/styrene-lab/styrene-rs/issues/78)
and [the delta spec](specs/rns-core-portability.md). It references the implemented
fixture and runner. Commands below are procedures, not new execution results.

[design.md](design.md) owns design decisions. [execution-plan.md](execution-plan.md)
owns authority and delivery gates. [tasks.md](tasks.md) tracks execution work.
[verification.md](verification.md) is the sole ledger for current case results,
source revisions, environment readiness, review evidence, and required remote CI/macOS checks.
Historical diagnostics below cannot satisfy a case against a changed revision.

## Preconditions and ownership

1. Use the approved D3 borrowed-state API and provider failure contract in [design.md](design.md#approved-entropy-and-api-boundary).
   Any API deviation requires coordinator review before implementation.
2. Use the approved `aarch64-unknown-none` target with pointer-width and 64-bit atomics.
   Cortex-M is excluded. Record allocator, memory-attribute and clock assumptions against D1–D3.
3. Select matching Cargo/rustc **1.97.0** explicitly. On Nucleus, use the retained
   `~/.local/state/styrene-lab/profiles/rns-no-std` profile.
   Target execution requires QEMU **11.1.0** and the toolchain's bundled `rust-lld`.
   Missing tools, target libraries or cached dependencies remain blockers.
4. Keep host configuration under `/home/wilson/nix-config` ownership.
   The separately authorized target profile does not authorize further provisioning,
   default-toolchain changes, service changes, host activation or deployment.
5. Use reviewed cached dependencies and external outputs. Local validation is
   `--offline --locked`; cache acquisition is a separate, explicit CI setup step below.
6. Use the checked-in templates in `tests/compile/rns-no-std/` through the runner.
   It materializes an independent workspace outside the product tree with resolver 3,
   edition 2024, Rust 1.97 and `publish = false`.
   Review its exact RNS source path, `^0.1.0` requirement, disabled defaults,
   feature forwarding and resolved sources. Product Cargo configuration must not leak into it.
7. Use the fixture's reviewed lockfiles under the
   [fixture lock policy](../../../tests/compile/rns-no-std/README.md#fixture-and-lock-ownership).
   The product lockfile does not govern this independent consumer. Any refresh requires
   deliberate external generation and review, not an implicit validation-time update.
8. Nucleus owns Linux checks and retained local evidence. Apple checks belong to the Apple host.
   Changes affecting Apple consumers require separately attributed Apple evidence.
9. The runner owns a unique `/tmp/opencode/rns-no-std-*` directory per invocation.
   Keep generated logs, binaries and caches outside source. Do not commit build
   outputs, credentials, private identities or live traffic captures.
10. Preserve reviewed run evidence in the durable store defined by [execution-plan.md](execution-plan.md#evidence-custody).
    Scratch cleanup must not remove the only copy of acceptance evidence.

## Acceptance matrix

`RNS src` means `crates/libs/styrene-rns/src/`. `RNS tests` means `crates/libs/styrene-rns/tests/`.
`Fixture` means `tests/compile/rns-no-std/`. Other paths are repository-relative.
The runner is `scripts/check-rns-no-std.py`. This table defines criteria without recording outcomes.

| Case | Requirement | Test locus | Objective pass criteria |
|---|---|---|---|
| RNS-NOSTD-001 | Independent minimal dependency graph | Fixture `smoke/Cargo.toml.in`; runner `check` trees and metadata | Target library graph has no `std`, `rmp-serde`, `serde_bytes`, transport stack or OS `getrandom` dependency. Host proc macros/build scripts may use `std`; classify edges rather than rejecting every textual `std` occurrence. |
| RNS-NOSTD-002 | Explicit allocation-backed profile | Fixture `smoke` with `alloc` forwarded to RNS; runner profile matrix | Minimal and explicit-alloc configurations expose the agreed allocation-backed contract and satisfy the same target graph exclusions. Neither claims an allocation-free profile. |
| RNS-NOSTD-003 | Real std-less target compilation | Independent fixture `smoke` library; runner `check` | Pinned Rust 1.97.0 checks succeed for minimal, `alloc`, and `alloc,fernet-aes128`. Capture actual target/compiler invocation. A native-host build cannot substitute. |
| RNS-NOSTD-004 | Caller-owned cryptographic randomness | RNS tests: `portable_rng.rs`, `support/rng.rs`; fixture `smoke/src/lib.rs` | Non-Copy caller RNG reaches identity generation, ephemeral keys, IV/group encryption, announce random bytes and ratchet rotation. State advances across calls without duplicated-state replay. Approved provider failure behavior and legacy trait compatibility are exercised. Test RNGs remain synthetic-only. |
| RNS-NOSTD-005 | Randomness-free authenticated decryption | RNS tests: `decrypt_without_rng.rs`, `support/rng.rs`; Fernet, identity, group and ratchet tests | Every decrypt outcome makes zero RNG requests. Malformed, wrong-key, tampered, truncated and undersized-buffer cases preserve established error and output-buffer behavior where applicable. Authentication failure exposes no unauthenticated plaintext. |
| RNS-NOSTD-006 | Embedded-time behavior | RNS tests: `embedded_time.rs`; fixture `smoke/src/lib.rs` | Uninitialized or cleared time produces the existing typed error. Supplied/advanced time drives announce timestamps and ratchet rotation consistently. Existing unavailable-time and interval assertions remain effective. |
| RNS-NOSTD-007 | Security and wire compatibility | RNS tests: `common/mod.rs`, `fixture_index.rs`, `interop_*.rs`; crypto tests and shared interop runner | Canonical loader/index and digest-checked fixtures remain authoritative. Verify-before-decrypt and the constant-time verifier remain intact. Signature/tag, truncation and padding rejection cases pass for applicable default/alternate Fernet profiles. |
| RNS-NOSTD-008 | Hosted feature/API compatibility | RNS manifest, package tests and hosted profile checks below; fixture legacy callers | Default and explicit `std` preserve hosted APIs; `transport` implies `std`; `serial` implies transport. Linux checks pass. Required Apple checks have separate evidence. Compilation establishes no serial-device or radio acceptance. |
| RNS-NOSTD-009 | Hosted persistence and key fallback | RNS src: `key_manager.rs`, `private_file.rs`, destination and transport ratchet-store tests; RNS tests: `ratchet_persistence.rs` | Existing persistence bytes remain readable and encoding-compatible. Private atomic-write behavior is retained. Key fallback remains restricted to established availability failures; rejection cannot silently become fallback. |
| RNS-NOSTD-010 | Direct consumers and retained crypto features | LXMF, mesh, tunnel and daemon checks; RNS manifest and independent resolved-feature review | Affected direct consumers compile with intended hosted features. Review confirms retained zeroization and other required crypto capabilities, including explicitly restored non-std defaults. No whole-stack no-std claim follows. |
| RNS-NOSTD-011 | Independent consumer link and execution | Fixture `smoke` and `boot`; runner `execute` and boundary tests | Binary links and executes bounded encoding, signing, encryption, decryption and ratchet assertions on the chosen target/runner. Record allocator/provider/clock setup and result. Enforce the fault and runner conditions below. Report compile, link, emulated execution and hardware evidence separately. Never substitute a host run. |
| RNS-NOSTD-012 | CI gate validity and attributable evidence | `.github/workflows/ci.yml` job `rns-no-std`, `justfile`, runner/tests and fixture `negative` | Positive consumer builds pass; importing target `std` in the separate negative control fails with the intended E0463 diagnostic and exit 101. Capture complete source/environment records. Failed, missing or blocked lanes cannot count as passes. |

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
- For 011/012, retain the reviewed runner correction conditions:
  - Panic/OOM controls require exactly `RNS-NOSTD:ARMED-PANIC` or
    `RNS-NOSTD:ARMED-OOM`, then `RNS-NOSTD:PANIC-OR-OOM`, with exit 1 and no timeout.
    Emit the armed marker immediately before the intended fault operation.
    Reject unexpected allocation success (`RNS-NOSTD:OOM-RETURNED`, exit 3)
    and an earlier unarmed panic as evidence of the intended fault.
  - SIGTERM must stop the owned process group, restore the previous handler,
    record cancellation and return 143. Retain logs when finalization can complete.
    Cancellation must not become success or terminate unrelated processes.
  - Reject symlinks in RNS crate and fixture roots, their ancestors and descendants
    before resolution or content reads. Enforce source-root containment. Reject symlinked
    or source-overlapping outputs. Regular Git worktree `.git` pointer files remain valid.
    Any source or materialized-fixture mutation invalidates the run.

## Independent target commands

The [fixture README](../../../tests/compile/rns-no-std/README.md#run-existing-tools)
owns detailed CLI, lockfile, boot and evidence procedures. The
[runner source](../../../scripts/check-rns-no-std.py) defines its argument interface.
Run from the selected checkout using existing tools and reviewed cache data.

On Nucleus, select the retained Rust **1.97.0** profile explicitly:

```sh
PINNED_PROFILE="$HOME/.local/state/styrene-lab/profiles/rns-no-std"
PINNED_CARGO="$PINNED_PROFILE/bin/cargo"
PINNED_RUSTC="$PINNED_PROFILE/bin/rustc"
: "${REVIEWED_CARGO_HOME:?existing reviewed Cargo home containing registry cache}"

python3 -B scripts/check-rns-no-std-tests.py
python3 -B scripts/check-rns-no-std.py check \
  --cargo "$PINNED_CARGO" --rustc "$PINNED_RUSTC" --cache "$REVIEWED_CARGO_HOME"
```

`check` materializes the independent fixture and fixes the target to
`aarch64-unknown-none`. It runs host allocator tests, the separate negative
consumer, and trees, metadata and library checks for no defaults, `alloc`, and
`alloc,fernet-aes128`. Review the resolved graphs for 001/002/010.
Host-only build tools may use `std`. The runner does not infer graph acceptance
from a textual search for `std`.

For independent linking and synthetic execution:

```sh
QEMU="$PINNED_PROFILE/bin/qemu-system-aarch64"
python3 -B scripts/check-rns-no-std.py execute \
  --cargo "$PINNED_CARGO" --rustc "$PINNED_RUSTC" --cache "$REVIEWED_CARGO_HOME" \
  --qemu "$QEMU" --timeout 60
```

`execute` includes `check`, then links and runs the fault controls and all three
smoke profiles. It selects QEMU `virt-11.1`, Cortex-A53, single-thread TCG, one CPU
and 128 MiB RAM, with network, display, serial and monitor connections disabled.
Each positive smoke execution requires exactly one `RNS-NOSTD:COMPLETE` line,
no conflicting `RNS-NOSTD:` marker, exit zero and no timeout.
Review the retained ELF, linker map, startup and Normal-memory atomic assumptions.

The runner copies only public registry data from `--cache` into its own Cargo
home. It sets `CARGO_NET_OFFLINE=true` and uses `--offline --locked` for Cargo
validation. Outputs are fixed by the runner, not caller-supplied output flags:

- Scratch: `/tmp/opencode/<run-id>/`, where the run ID starts with `rns-no-std-`.
  Each run contains `fixture/`, `cargo-home/` and `target/`.
- Durable evidence: `~/.local/state/styrene-lab/issue-78/runs/validation/<run-id>/`.

By default, product and fixture inputs come from the runner's checkout. For a
separate reviewed product checkout, pass `--product-source` and its full
`--product-revision` as documented in the README. That checkout must be clean at
the exact SHA. Both source fingerprints must remain stable throughout validation.

`bootstrap` uses the same tool/cache/QEMU arguments as `execute`, but proves only
the boot substrate and fault controls. Its `RNS-NOSTD:BOOT-ONLY` marker cannot
satisfy product execution. The separate `negative` consumer has its own manifest
and lockfile; it is not a product feature or source mutation. Its only accepted
compiler errors are E0463 for missing `std` in `rns_no_std_negative`'s `src/lib.rs`,
with Cargo exit 101. Missing `core`, packages, lockfiles or unrelated errors do not validate the gate.

### CI acquisition and offline validation

The existing [CI workflow](../../../.github/workflows/ci.yml) owns the fresh-runner
sequence for job `rns-no-std`:

1. Select Rust 1.97.0 with `aarch64-unknown-none` target libraries.
2. Run `prepare` with explicit `--cargo` and `--rustc` paths. It prints the external
   scratch directory and probes prerequisites without compiling or fetching dependencies.
3. From that directory's `fixture/`, run `cargo fetch --locked --target aarch64-unknown-none`.
   Set `CARGO_HOME` to its `cargo-home/` and `CARGO_TARGET_DIR` to its `target/`.
   This network-capable acquisition step belongs to CI setup only.
4. Run `check` with the same tool paths and `--cache` set to the prepared `cargo-home/`.
   Validation is offline and QEMU-independent. Retain the workflow's evidence artifact.

This CI sequence does not authorize local fetching or provisioning. A cached
local rehearsal cannot establish fresh-runner acquisition, artifact upload or PR-head CI results.

## Focused hosted commands

Use the pinned executables selected above. Before direct Cargo commands, select
an isolated, reviewed offline `CARGO_HOME` and external `CARGO_TARGET_DIR`.
These inputs apply to direct Cargo calls; the independent runner creates its own.
Use the existing declared native-library environment for transport/consumer checks.

```sh
: "${CARGO_HOME:?isolated reviewed offline Cargo home}"
: "${CARGO_TARGET_DIR:?external build-output directory}"
export CARGO_HOME CARGO_TARGET_DIR
export RUSTC="$PINNED_RUSTC"
export RUSTDOC="$PINNED_PROFILE/bin/rustdoc"
export PATH="$PINNED_PROFILE/bin:$PATH"

"$PINNED_CARGO" check --offline --locked --lib -p styrene-rns --no-default-features
"$PINNED_CARGO" clippy --offline --locked --lib --no-default-features --no-deps -p styrene-rns -- -D warnings
"$PINNED_CARGO" test --offline --locked --no-default-features -p styrene-rns --test portable_rng --test decrypt_without_rng --test embedded_time
"$PINNED_CARGO" test --offline --locked -p styrene-rns
"$PINNED_CARGO" test --offline --locked --no-default-features --features std -p styrene-rns
"$PINNED_CARGO" test --offline --locked -p styrene-rns --test fixture_index
"$PINNED_CARGO" test --offline --locked -p styrene-rns --features interop-tests,transport
"$PINNED_CARGO" test --offline --locked -p styrene-rns --features fernet-aes128 --lib crypt::fernet
"$PINNED_CARGO" test --offline --locked -p styrene-interop-runner --test rns_fixtures --test rns_handoff_manifests --test pinned_evidence_record
```

Repeat the portable RNG/decrypt/time invocation with `--features alloc` and
`--features alloc,fernet-aes128`. Check hosted feature profiles and direct consumers:

```sh
for profile in std alloc transport serial; do
  "$PINNED_CARGO" check --offline --locked --lib -p styrene-rns --no-default-features --features "$profile" || exit
done
"$PINNED_CARGO" check --offline --locked -p styrene-lxmf -p styrene-mesh -p styrene-tunnel -p styrened
```

Record each invocation's exit status and stop on failure. The Fernet command has
one positional filter; use separate invocations for other filters.
Repository-wide tests and `--all-targets` are hosted regressions, not independent
no-std proof. Dev-dependencies and selected consumers can restore `std` features.

## Evidence review and closure

1. Bind validation to the reviewed integrated source and fixture state.
   Use the [exact-base regression record](verification.md#exact-base-regression-proof)
   for the historical failure and its cause. Preserve prior security, persistence
   and embedded-time observations when comparing changed-source results.
2. Run the independent minimal, explicit-alloc and alternate-Fernet matrix,
   focused hosted regressions, direct consumers and linked execution controls.
3. Review every case against the spec and retain attributable evidence in `verification.md`.
   Obtain required remote CI and separately attributed Apple evidence through the
   [delivery gates](execution-plan.md#review-and-delivery-gates).
4. Close only when required cases have passing evidence and the final acceptor
   authorizes closure. A blocked required case keeps the change open. Any scope
   revision needs an explicit reviewed spec decision.

For every execution, retain: case ID, UTC time, host, source SHA, dirty/untracked state and content
fingerprint, product/fixture lock hashes, fixture revision/hash, resolved dependency sources,
Cargo/rustc versions, enabled features, target triple, exact command/cwd/environment overrides,
exit status, logs and artifact hashes. Record runner provenance and evidence class for 011.
Record blocked reasons and missing prerequisites explicitly. Do not retain credentials or secret material.

## Historical diagnostics — not acceptance results

The original planning branch was `fix/rns-no-std-78` at product base
`ab46aa1f13de5663c684225dc27dfcba330d965f`.

The earlier assessment used an export of `ab46aa1f13de5663c684225dc27dfcba330d965f`,
with unchanged lockfile, under `/tmp/opencode/styrene-issue78-sxkdtf8r`.
Its Rust 1.97.0 native RNS no-default check succeeded. Its graphs still enabled target `std` dependencies.
A dependency-only Rust 1.98.1 `wasm32v1-none` check failed in `num-traits` for missing `std`.
That was neither the pinned-toolchain RNS target check nor a linked/executed consumer smoke.
Canonical CI run [33833175316](https://github.com/styrene-lab/styrene-rs/actions/runs/33833175316)
passed hosted checks on that SHA. These observations establish context only.
Current results and the pinned-target regression proof belong in `verification.md`.
