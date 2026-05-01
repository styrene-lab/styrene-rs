# Changelog

All notable changes to styrene-rs will be documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-04-19

First internal release. Rust is now the canonical Styrene distribution for new deployments.

### Protocol Layer (styrene-rns)

#### Added
- **IFAC authentication** — full wrap/unwrap at the interface boundary with multi-hop
  correctness. Cross-language interop verified byte-identical with Python RNS.
  Wired through TCP client, TCP server (propagated to accepted clients), and serial.
  UDP rejects IFAC-flagged packets.
- **Serial/KISS interface** — `SerialInterface::new()` (raw HDLC) and
  `SerialInterface::new_kiss()` (KISS+HDLC for TNC/RNode). Async reconnection,
  `KissReader`/`KissWriter` adapters with correct partial-write buffering.
- **KISS framing codec** — FEND/FESC byte-stuffing encoder and stateful stream decoder.
- **Ratchet persistence** — disk-backed store with in-memory cache, atomic writes,
  30-day expiry. Forward secrecy survives daemon restarts.
- **Channel packet semantics** — ported from upstream PRs #109, #123, #111.
- **Resource lifecycle** — startup/retry/timeout/cleanup ported from upstream PRs #112, #124.
- **Link handshake parity** — proof race fix, interface binding, RTT-based watchdog
  from upstream PR #107.
- **Announce correctness** — proof gates, hash validation, throttling, ingress
  rate limiting from upstream PRs #106, #115, #117, #121, #122, #125.
- **Path tag preservation** and duplicate suppression from upstream PR #115.

#### Fixed
- IFAC multi-hop bug (inherited from fork) — was critical, now resolved.
- HMAC timing oracle — constant-time comparison.
- `Identity.encrypt()` double-ephemeral key reuse.
- Announce validation accepting hash mismatch (upstream #106).
- Packet receipts satisfied by forged proofs (upstream #106).
- Known-destination pubkey stability check (upstream #106).
- Ratchet-bearing announce parsing too permissive (upstream #106).
- Transported link-request proofs skipping validation (upstream #106).

### LXMF Layer (styrene-lxmf)

#### Added
- Propagation link lifecycle (upstream PR #118).
- Propagation stamp cost retention (upstream PR #119).
- Peer lifecycle state machine (upstream PR #120).
- Propagation stamp validation (upstream PR #129).
- Propagation transient-id canonicalization (upstream PRs #130, #131).
- Stamp/ticket wire protocol options (upstream PR #126).

### Wire Protocol (styrene-mesh)

#### Added
- Wire format v2 with 29-byte header, 60+ message types including PQC and
  content distribution.
- Cross-language test vectors (13 V2 + 2 V1 binary fixtures) verified
  roundtrip byte-identical with Python.

### Daemon (styrened)

#### Added
- `AppContext` service architecture — owns transport, messaging, discovery,
  protocol registry, events, propagation, fleet services.
- `DaemonFacade` implementing the `Daemon` IPC trait.
- `MeshTransport` trait with `TokioTransportAdapter` and `NullTransport`.
- IPC server over Unix socket (styrene-ipc-server) with 60+ message types.
- Propagation service with SQLite store and expiry cleanup.
- Discovery service with `NodeStore` (SQLite-backed peer registry).
- Protocol registry with pluggable handler dispatch.
- Config service with TOML loading and role-based startup.
- Inbound, announce, and link worker pipelines.
- RPC response handler for fleet coordination.

### TUI (styrene-tui)

#### Added
- Production ratatui-based TUI with daemon RPC bridge.
- Topology sidebar, signal panel, structured mesh data model.
- Live link telemetry subscription (SubLinks).

### Infrastructure

#### Added
- Weekly upstream sync workflow with automated PR generation.
- Nightly validation + security audit (cargo-deny, cargo-audit).
- Interop CI with Python fixture generation.
- IFAC cross-language test vectors.
- `just` recipes for build, test, lint, interop, upstream review.

### Documentation

#### Changed
- styrene-rs is now **canonical** — README, CLAUDE.md, UPSTREAM.md updated.
- Python styrened positioned as legacy/supported, not primary.
- PARITY_GAPS.md updated: IFAC resolved, serial implemented, ratchet
  persistence resolved, upstream fixes closed, wire interop resolved.
- `docs/incremental-rust-migration.md` marked superseded.

---

## Next: Operator Verification Iteration

The following work is planned for the 0.2.0 cycle:

- Propagation backend (store-and-forward for offline recipients)
- Configuration system (per-interface IFAC keys, hot-reload)
- Hardware validation on RNode and RP2040 devices
- Cross-compilation CI for aarch64/armv7
- Pre-built release binaries
- Operator migration guide (Python to Rust)
