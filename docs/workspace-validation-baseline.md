# Workspace Validation Baseline

**Recorded:** 2026-07-12
**Scope:** Rust product installation, development, and substrate-adoption validation

## Decision

The maintained product and workspace are Rust-only. The validation boundary is intentional:

- `cargo test` validates the canonical default product set;
- `cargo test --workspace` validates every maintained workspace member;
- neither command depends on a host language runtime outside the Rust toolchain.

Current Cargo metadata reports 25 workspace members. Ordinary validation uses
the following exact matrix:

```bash
cargo check --workspace --all-targets --exclude styrene-dx
cargo clippy --workspace --all-targets --exclude styrene-dx --no-deps -- -D warnings
just test                    # explicit deterministic target matrix
just test-interop            # committed fixtures only
just test-validation-offline # recipes, workflows, targets, product registry
```

`styrene-dx` remains excluded from whole-workspace check and Clippy commands because its desktop WebView dependency is platform-specific. `just test` executes its deterministic component and Fixture tests explicitly without Python or network access.

## Active validation boundary

`just test` batches pure library packages once, then selects deterministic
targets for `styrened`, `styrene-e2e`, IPC server, TUI, CLI, and the DX component and Fixture suites.
The selected application targets include config, identity storage, LXMF
fidelity, NomadNet pages, TUI rendering/state units, and LXMF protocol gates.
Named listener, live-peer, Python, broker, and subprocess integration targets
remain explicit recipes or manually dispatched workflows.

The ordinary Rust `styrene-mesh` product registry test parses capability and
parity metadata, validates product-manifest references, and verifies every
committed fixture SHA-256 digest. `just validate-product` remains an explicit
Python cross-check and is not required by ordinary validation.

Use these broad Cargo boundaries only when the environment supports every
target:

```bash
# Canonical maintained/default product set
cargo test

# Every maintained Rust workspace member
cargo test --workspace
```

Upstream protocol interoperability harnesses remain explicit and isolated from ordinary product validation.

`just validate` uses `tests/offline-validation.toml` to separate target selection from
environment-dependent execution. Deterministic `styrened` page units, the
`styrene-e2e` LXMF protocol target, Micron/NomadNet conformance, and committed
interop fixtures remain ordinary gates. Listener, subprocess, broker, hardware,
and live-peer targets require an explicit recipe or manually dispatched workflow.
Reusable workflows must use immutable commit references; ordinary validation
rejects unmodeled Just, Cargo, or workflow trigger forms instead of assuming they
are safe.

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

## `styrene-e2e` execution budget

On 2026-08-21, full `styrene-e2e` runs were terminated by 120-second and
240-second harness limits. Both terminations were harness failures, not test
failures. The same complete command passed when its harness budget increased to
900 seconds:

```text
cargo test -p styrene-e2e
160 passed; 0 failed; 0 ignored
```

Use at least 900 seconds for a full `styrene-e2e` run. Do not report a timed-out
full run as partial validation. Rerun the complete command with the correct
budget. Focused E2E targets can use shorter budgets when their own deadline is
bounded.

## Substrate-adoption gate

For each bounded FreeTAKTeam behavior-adoption slice:

1. run focused tests for the changed crate and paths;
2. run all tests and all-target checking for the affected product crate;
3. run relevant `styrene-e2e` tests;
4. run the maintained workspace boundary above when a full regression pass is warranted;
5. report historical or external compatibility targets separately from product validation.
