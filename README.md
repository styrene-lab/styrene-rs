# styrene-rs

Rust implementation of the [RNS](https://reticulum.network/) and [LXMF](https://github.com/markqvist/LXMF) protocol stack for the [Styrene](https://github.com/styrene-lab) mesh communications project.

Forked from [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs). See [UPSTREAM.md](UPSTREAM.md) for fork attribution.

## Status

**Canonical distribution.** styrene-rs is the primary implementation for new deployments. Python [styrened](https://github.com/styrene-lab/styrened) remains supported for existing installations. Both communicate over the same LXMF mesh — the wire protocol is the shared contract.

## Crates

| Crate | Description |
|-------|-------------|
| [`styrene-rns`](crates/libs/styrene-rns/) | RNS protocol core — identity, destinations, links, resources, ratchets. Transport layer (TCP, UDP, Serial/KISS) behind `transport` feature |
| [`styrene-lxmf`](crates/libs/styrene-lxmf/) | LXMF messaging — router, propagation, stamps, delivery pipeline. SDK domain types behind `sdk` feature |
| [`styrene-mesh`](crates/libs/styrene-mesh/) | Styrene wire protocol envelope (matches Python `styrene_wire.py`) |
| [`styrened`](crates/apps/styrened/) | Daemon binary — RPC server, message routing, identity management |

## Build

```bash
# Install just: brew install just / cargo install just
just validate    # format-check + lint + test
just test        # cargo test --workspace
just lint        # cargo clippy
just docs        # cargo doc --workspace
```

## Repository Layout

```
styrene-rs/
├── crates/
│   ├── libs/          # Library crates (styrene-rns, styrene-lxmf, styrene-mesh)
│   └── apps/          # Binary crates (styrened)
├── tests/
│   └── interop/       # Python<->Rust interop test fixtures
├── justfile           # Build automation
├── UPSTREAM.md        # Fork attribution
└── CLAUDE.md          # Claude Code guidance
```

## Wire Protocol Contract

The wire protocol is the integration boundary between Python and Rust. Both implementations produce and consume identical byte sequences — no FFI, no shared memory. A Python styrened and a Rust styrened coexist on the same Reticulum mesh without knowing about each other.

## License

MIT — see [LICENSE](LICENSE).
