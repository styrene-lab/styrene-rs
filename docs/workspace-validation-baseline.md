# Workspace Validation Baseline

**Recorded:** 2026-07-12
**Scope:** Rust product installation, development, and substrate-adoption validation

## Decision

PyO3 and a local Python interpreter are **not requirements for building, installing, running, or validating the Styrene Rust product**.

The only crate that depends on PyO3 is:

```text
crates/bindings/styrene-native
```

`styrene-native` is a Python extension from the superseded incremental Python-to-Rust migration plan documented in [`incremental-rust-migration.md`](incremental-rust-migration.md). That plan assumed a Python daemon would remain the product orchestrator and import Rust modules through PyO3. Rust is now the canonical distribution, so that architecture no longer applies.

The crate remains listed in `Cargo.toml` under `workspace.members`, although it is not in `workspace.default-members`. Consequently:

- `cargo test` does not select it through the default-member set;
- `cargo test --workspace` does select it;
- selecting it builds PyO3 and makes the result depend on the host Python ABI;
- on this machine, PyO3 0.23 rejects Python 3.14 because it declares support through Python 3.13.

That Python-version failure is therefore **legacy workspace coupling**, not a Styrene product failure and not a reason to constrain the host Python installation.

## Active validation boundary

Until `styrene-native` is removed from the active workspace, use one of these boundaries:

```bash
# Canonical maintained/default product set
cargo test

# Every active Rust workspace crate except the superseded Python extension
cargo test --workspace --exclude styrene-native
```

Do not use `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1` to make ordinary Styrene validation pass. That masks the obsolete workspace boundary instead of correcting it.

Python-generated protocol fixtures and optional Python interoperability harnesses are separate concerns. Consuming checked-in fixtures does not require PyO3. Tests that intentionally launch Python must remain explicit, isolated compatibility tests rather than implicit requirements of the Rust product build.

## Required cleanup

Treat removal of `crates/bindings/styrene-native` from `workspace.members` as workspace hygiene. The source may remain temporarily for historical reference, or move to an archive, but it must not gate product-wide Rust validation.

Removing it from the active workspace is preferred over upgrading PyO3: upgrading would maintain an abandoned integration architecture and retain an unnecessary host-language dependency.

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
5. report legacy/archived compatibility targets separately from product validation.
