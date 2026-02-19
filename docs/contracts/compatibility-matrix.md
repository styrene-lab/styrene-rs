# Compatibility Matrix

Last updated: 2026-02-19

## Python LXMF -> Rust LXMF-rs

Top-level module status is tracked here; method-level parity is tracked and enforced from `docs/plans/lxmf-parity-matrix.md`.

| Python module | Rust module | Status | Source of truth |
| --- | --- | --- | --- |
| `LXMF/LXMF.py` | `crates/internal/lxmf-legacy/src/constants.rs`, `crates/internal/lxmf-legacy/src/helpers.rs` | done | `docs/plans/lxmf-parity-matrix.md` |
| `LXMF/LXMessage.py` | `crates/internal/lxmf-legacy/src/message/*` | done | `docs/plans/lxmf-parity-matrix.md` |
| `LXMF/LXMPeer.py` | `crates/internal/lxmf-legacy/src/peer/mod.rs` | done | `docs/plans/lxmf-parity-matrix.md` |
| `LXMF/LXMRouter.py` | `crates/internal/lxmf-legacy/src/router/mod.rs` | done | `docs/plans/lxmf-parity-matrix.md` |
| `LXMF/Handlers.py` | `crates/internal/lxmf-legacy/src/handlers.rs` | done | `docs/plans/lxmf-parity-matrix.md` |
| `LXMF/LXStamper.py` | `crates/internal/lxmf-legacy/src/stamper.rs`, `crates/internal/lxmf-legacy/src/ticket.rs` | done | `docs/plans/lxmf-parity-matrix.md` |

## Python Reticulum -> Rust Reticulum-rs

Detailed mapping and tests are tracked in `docs/plans/reticulum-parity-matrix.md`.

Release-track compatibility: `lxmf 0.3.0` targets `reticulum-rs 0.1.3` today (or pinned branch revisions during refactor).
