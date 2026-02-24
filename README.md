# styrene-rs

Rust implementation of the [RNS](https://reticulum.network/) and [LXMF](https://github.com/markqvist/LXMF) protocol stack for the [Styrene](https://github.com/styrene-lab) mesh communications project.

Forked from [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs). See [UPSTREAM.md](UPSTREAM.md) for fork attribution.

## Status

**Phase 1: Fork and Foundation.** Python [styrened](https://github.com/styrene-lab/styrened) remains the primary implementation. This Rust stack is experimental until it passes the interop gate (Phase 3).

## Crates

| Crate | Description |
|-------|-------------|
| [`styrene-rns`](crates/libs/styrene-rns/) | RNS protocol core — identity, destinations, links, resources, ratchets |
| [`styrene-rns-transport`](crates/libs/styrene-rns-transport/) | Transport interfaces — TCP, UDP, future Serial/KISS |
| [`styrene-lxmf`](crates/libs/styrene-lxmf/) | LXMF messaging — router, propagation, stamps, delivery pipeline |
| [`styrene-mesh`](crates/libs/styrene-mesh/) | Styrene wire protocol envelope (matches Python `styrene_wire.py`) |
| [`styrene`](crates/meta/styrene/) | Meta-crate re-exporting all library crates |

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
│   ├── libs/          # Library crates (published to crates.io)
│   ├── apps/          # Binary crates (styrened-rs, interop-test)
│   └── meta/          # Meta-crate re-exports
├── tests/
│   └── interop/       # Python<->Rust interop test fixtures
├── justfile           # Build automation
├── UPSTREAM.md        # Fork attribution
└── CLAUDE.md          # Claude Code guidance
```

## Wire Protocol Contract

The wire protocol is the integration boundary between Python and Rust. Both implementations produce and consume identical byte sequences — no FFI, no shared memory. A Python styrened and a Rust styrened-rs coexist on the same Reticulum mesh without knowing about each other.

## License

MIT — see [LICENSE](LICENSE).
