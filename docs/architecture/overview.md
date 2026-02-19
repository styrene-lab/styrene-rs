# LXMF-rs Architecture

## Core Principles
- Protocol code is isolated from operator workflows.
- Runtime behavior is explicit and testable.
- Public API surfaces are narrow and crate-scoped.

## Stable Public Crates
- `lxmf-core`
- `lxmf-router`
- `lxmf-runtime`
- `rns-core`
- `rns-transport`
- `rns-rpc`

## Layering Rules
- `crates/libs/*` must not depend on `crates/apps/*`.
- `lxmf-core` must not directly depend on `tokio`, `clap`, `ureq`, or `serde_json`.
- `rns-core` must not directly depend on `tokio` or `clap`.
- CLI/daemon concerns live in `crates/apps/*`.

## Transitional Note
Legacy implementation crates currently remain under `crates/internal/*` while the module split is completed.
