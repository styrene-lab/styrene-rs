# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Rust implementation of the RNS/LXMF protocol stack for the [Styrene](https://github.com/styrene-lab) mesh communications project. Forked from [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs) — the most complete non-Python Reticulum implementation.

This is a **parallel implementation** alongside the Python `styrened` daemon. The wire protocol is the shared contract — no FFI, no PyO3 bindings. Both implementations communicate over LXMF like any two Reticulum nodes.

## Build Commands

```bash
# Full validation (lint + format check + test)
just validate

# Individual commands
just test              # cargo test --workspace
just lint              # cargo clippy -- -D warnings
just format            # cargo fmt
just format-check      # cargo fmt --check
just build             # cargo build --workspace
just build-release     # cargo build --workspace --release
just docs              # cargo doc --workspace --no-deps

# Interop testing (requires Python RNS/LXMF)
just test-interop
```

## Crate Map

```
crates/
  libs/
    styrene-rns/            # RNS protocol core: identity (X25519+Ed25519),
                            # destinations, links, resources, ratchets, packets.
                            # Transport layer (TCP, UDP, Serial/KISS) behind
                            # `features = ["transport"]`
    styrene-lxmf/           # LXMF messaging: router, propagation, stamps,
                            # delivery pipeline, message packing.
                            # SDK domain types behind `features = ["sdk"]`
    styrene-mesh/           # Styrene wire protocol envelope format
                            # (must match styrened's styrene_wire.py byte-for-byte)
  apps/
    styrened-rs/            # Daemon binary + RPC server + test harness
                            # (lib name: reticulum_daemon)
```

## Relationship to Python styrened

| Concern | Python (styrened) | Rust (styrene-rs) |
|---------|-------------------|-------------------|
| Wire protocol authority | `styrene_wire.py` is reference | Must match byte-for-byte |
| Production status | Primary (all deployments) | Experimental (until interop gate) |
| TUI | styrene-tui (Textual) | Not planned |
| Target devices | Hub, operator workstation | Constrained edge (Pi Zero 2W) |
| Communication | Over LXMF mesh | Over LXMF mesh |

## Key Files

| File | Purpose |
|------|---------|
| `crates/libs/styrene-mesh/src/wire.rs` | Wire protocol — must match `styrene_wire.py` |
| `crates/libs/styrene-rns/src/transport/` | Transport layer (feature-gated) |
| `crates/libs/styrene-lxmf/src/sdk/` | SDK domain types (feature-gated) |
| `crates/apps/styrened-rs/src/rpc/` | RPC daemon + codec + HTTP |
| `tests/interop/fixtures/` | Binary test vectors generated from Python |
| `tests/interop/python/` | Python scripts generating test fixtures |
| `UPSTREAM.md` | Fork attribution and upstream tracking |

## Known Issues (Inherited from Fork)

| Issue | Severity | Status |
|-------|----------|--------|
| IFAC bug — multi-hop broken | Critical | Fix planned |
| HMAC timing oracle | High | Fixed (constant-time comparison) |
| `Identity.encrypt()` double-ephemeral | Medium | Fixed |

## Development Patterns

- **justfile** is the command runner (`just --list` for all recipes)
- **Workspace lints** enforced: `unsafe_code = "forbid"`, `clippy::unwrap_used = "warn"`
- **Boundary checks** prevent unauthorized inter-crate dependencies
- Changes to wire protocol must be synchronized with Python `styrened`
- All crates target `edition = "2021"`, `rust-version = "1.75"`
