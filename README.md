<div align="center">

# Styrene

**A local-first, identity-secure mesh communications stack built in Rust.**

[![CI](https://github.com/styrene-lab/styrene-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/styrene-lab/styrene-rs/actions/workflows/ci.yml)
[![Release](https://github.com/styrene-lab/styrene-rs/actions/workflows/release.yml/badge.svg)](https://github.com/styrene-lab/styrene-rs/actions/workflows/release.yml)
[![Rust 1.97](https://img.shields.io/badge/rust-1.97-2ab4c8?logo=rust)](rust-toolchain.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-1ab878.svg)](LICENSE)
[![RNS + LXMF](https://img.shields.io/badge/protocols-RNS%20%2B%20LXMF-1a8898)](https://reticulum.network/)

[Getting started](#getting-started) · [Runtime profiles](#runtime-profiles) · [Architecture](#architecture) · [Contributing](CONTRIBUTING.md) · [Releases](https://github.com/styrene-lab/styrene-rs/releases)

</div>

Styrene is a communications runtime for networks where the Internet is unavailable, undesirable, or untrusted. It combines an RNS/LXMF-compatible protocol stack, secure identities, messaging, content, tunnels, fleet services, and an operator TUI in one native application.

`styrene-rs` is the canonical Styrene implementation for new deployments. It descends from [FreeTAKTeam/LXMF-rs](https://github.com/FreeTAKTeam/LXMF-rs) and [BeechatNetworkSystemsLtd/Reticulum-rs](https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs); see [UPSTREAM.md](UPSTREAM.md) for lineage and tracking policy.

> **Release status:** active development and operator verification. Protocol and identity code is security-sensitive; this is not yet an independently audited cryptographic product.

## What ships

| Binary | Purpose |
|---|---|
| `styrene` | Canonical application: integrated TUI, embedded runtime, and operational CLI |
| `styrened` | Standalone/service-oriented daemon for managed and headless deployments |
| `styrene-tui` | Compatibility launcher for the terminal application |

The workspace also contains reusable crates for RNS, LXMF, IPC, identity, RBAC, content, telemetry, tunnels, and service composition.

## Getting started

### Prebuilt releases

Release archives are published for Linux x86-64 and macOS x86-64/Apple Silicon:

1. Download the archive for your platform from [GitHub Releases](https://github.com/styrene-lab/styrene-rs/releases).
2. Verify the accompanying checksum/provenance artifacts where supplied.
3. Place the binaries on your `PATH`.
4. Launch the application:

```bash
styrene
```

Every build exposes its source revision:

```console
$ styrene --version
styrene 0.1.0+ce025b424
```

### Build and install from source

Requirements:

- Git
- [Rustup](https://rustup.rs/) — the repository pins the exact stable toolchain
- [`just`](https://github.com/casey/just) (`brew install just` or `cargo install just`)

```bash
git clone https://github.com/styrene-lab/styrene-rs.git
cd styrene-rs
just install
styrene
```

`just install` performs a locked release build and atomically replaces `styrene`, `styrened`, and `styrene-tui` in `~/.cargo/bin`. To test another destination:

```bash
just install /path/to/bin
# or
STYRENE_INSTALL_DIR=/path/to/bin just install
```

## Runtime profiles

Identity lifetime and storage location are independent choices:

```bash
styrene                              # persistent platform installation
styrene --ghost                      # isolated ephemeral identity and session
styrene --portable /mnt/usb/styrene  # persistent, relocatable installation
styrene --portable /mnt/usb/styrene --ghost
```

Ghost sessions start the runtime in-process—no separately installed daemon is required. Session identity, database, and socket are isolated and removed on normal exit. Reusable non-secret ghost settings live outside the ephemeral session, so repeated invocations do not require another walkthrough.

See [Runtime Profiles](docs/runtime-profiles-portable-ghost.md) for precedence, filesystem layout, cleanup semantics, and environment variables.

## Common commands

```bash
styrene                 # terminal application
styrene status          # runtime and mesh status
styrene peers           # known peers
styrene identity        # local runtime identity
styrene --help           # complete command reference

just test               # workspace tests
just check              # compile all targets
just lint               # Clippy with warnings denied
just validate           # formatting, lint, and tests
just test-interop       # committed Python/Rust protocol fixtures
```

## Architecture

```text
styrene / styrene-tui
        │  typed IPC
        ▼
    styrened runtime
        │
        ├── messaging, discovery, propagation, pages, fleet, tunnels
        ├── styrene-lxmf   — LXMF delivery and propagation
        ├── styrene-mesh   — Styrene service wire envelope
        └── styrene-rns    — identity, destinations, links, resources, transports
```

Important workspace components:

| Crate | Responsibility |
|---|---|
| [`styrene-rns`](crates/libs/styrene-rns/) | RNS identity, destinations, packets, links, resources, ratchets, and transport interfaces |
| [`styrene-lxmf`](crates/libs/styrene-lxmf/) | LXMF messaging, delivery, propagation, stamps, and peer lifecycle |
| [`styrene-mesh`](crates/libs/styrene-mesh/) | Styrene service protocol envelope and compatibility fixtures |
| [`styrene-ipc`](crates/libs/styrene-ipc/) | Typed daemon/application boundary |
| [`styrene-identity`](crates/libs/styrene-identity/) | Scoped identity and key derivation facilities |
| [`styrene-tunnel`](crates/libs/styrene-tunnel/) | Peer tunnel and post-quantum session foundations |
| [`styrened`](crates/apps/styrened/) | Runtime and service composition |
| [`styrene`](crates/apps/styrene/) | Product facade and CLI |
| [`styrene-tui`](crates/apps/styrene-tui/) | Ratatui operator interface |

## Compatibility and upstreams

RNS and LXMF interoperability is validated against committed cross-language fixtures. Upstream RNS/LXMF projects are reviewed and changes are manually adopted rather than merged across the heavily restructured tree.

Rolling compatibility among Styrene Rust nodes is the maintained Styrene application contract. Upstream protocol compatibility remains covered where the RNS/LXMF specifications require it.

See:

- [UPSTREAM.md](UPSTREAM.md) — lineage and review process
- [CHANGELOG.md](CHANGELOG.md) — release history
- [SECURITY.md](SECURITY.md) — vulnerability reporting and security posture
- [CONTRIBUTING.md](CONTRIBUTING.md) — development and review workflow

## Platform support

| Target | Status |
|---|---|
| macOS Apple Silicon | Primary development and release target |
| macOS x86-64 | Release target |
| Linux x86-64 | Release target |
| iOS / Android | Rust runtime here; Dioxus product and packaging in `styrene-ui` |
| Windows | Not currently a release target |

## License

[MIT](LICENSE). Fork lineage and third-party attribution are documented in [UPSTREAM.md](UPSTREAM.md).
