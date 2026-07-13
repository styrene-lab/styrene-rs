# Workspace Validation Baseline

**Recorded:** 2026-07-12
**Scope:** Rust product installation, development, and substrate-adoption validation

## Decision

PyO3 and a local Python interpreter are **not requirements for building, installing, running, or validating the Styrene Rust product**.

The repository still contains the historical Python extension source at:

```text
crates/bindings/styrene-native
```

`styrene-native` comes from the superseded incremental Python-to-Rust migration plan documented in [`incremental-rust-migration.md`](incremental-rust-migration.md). That plan assumed a Python daemon would remain the product orchestrator and import Rust modules through PyO3. Rust is now the canonical distribution, so that architecture no longer applies.

On 2026-07-12, `styrene-native` was removed from `workspace.members`. Its source remains only as historical reference and is no longer resolved, built, tested, or published by workspace-wide Cargo commands.

The resulting boundary is intentional:

- `cargo test` validates the canonical default product set;
- `cargo test --workspace` validates every maintained workspace member;
- neither command resolves PyO3 or depends on the host Python ABI;
- the operator's Python version must not constrain Styrene's Rust toolchain.

The previous Python 3.14 failure was legacy workspace coupling, not a Styrene product failure.

Verified after removal on 2026-07-12:

```text
cargo metadata --locked: 24 workspace members; no styrene-native, pyo3, or pyo3-ffi
cargo test --workspace --exclude styrene-dx --locked: 1,252 passed; 0 failed; 11 ignored
```

`styrene-dx` is excluded on this macOS host because its desktop WebView dependency is Linux-specific. That platform exclusion is unrelated to Python and does not weaken the maintained non-GUI Rust boundary.

## Active validation boundary

Use the ordinary Cargo boundaries:

```bash
# Canonical maintained/default product set
cargo test

# Every maintained Rust workspace member
cargo test --workspace
```

Do not use `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1` for ordinary Styrene validation. It is neither required nor an acceptable way to hide accidental Python coupling.

Python-generated protocol fixtures and optional Python interoperability harnesses are separate concerns. Consuming checked-in fixtures does not require PyO3. Tests that intentionally launch Python must remain explicit, isolated compatibility tests rather than implicit requirements of the Rust product build.

## Historical binding policy

`crates/bindings/styrene-native` is not an active Cargo package from the workspace's perspective. Do not add it back to `workspace.members` or upgrade its PyO3 dependency without a new, explicit product decision to support a Python extension distribution.

If the historical source becomes misleading or costly to retain, remove or archive it in a separate change. Its presence on disk does not make it part of the maintained build graph.

## `styrene-ipc-server` timeout correction

A previous workspace run was terminated after the command exceeded a 600-second harness timeout while output happened to show `styrene-ipc-server` starting. That observation did **not** establish a test hang.

Focused verification on 2026-07-12 completed normally:

```text
cargo test -p styrene-ipc-server --lib -- --test-threads=1
9 passed; 0 failed; finished in 0.00s
```

Therefore:

- there is currently no reproduced `styrene-ipc-server` unit-test hang;
- the earlier event is classified as an inconclusive whole-workspace timeout;
- it must not be tracked as an IPC-server defect without a focused reproducer;
- long whole-workspace runs should be partitioned by package group or given a larger execution budget.

## Substrate-adoption gate

For each bounded FreeTAKTeam behavior-adoption slice:

1. run focused tests for the changed crate and paths;
2. run all tests and all-target checking for the affected product crate;
3. run relevant `styrene-e2e` tests;
4. run the maintained workspace boundary above when a full regression pass is warranted;
5. report historical or external compatibility targets separately from product validation.
