+++
id = "77bf5c4d-9bd9-4c19-954a-185ffbf8eed0"
kind = "design_node"

[data]
title = "Nym Mixnet as Styrene Signaling Substrate"
status = "exploring"
issue_type = "architecture"
priority = 2
dependencies = []
open_questions = [
  "[assumption] Nym bandwidth credential acquisition can be automated headlessly on daemon nodes (no interactive wallet flow) at acceptable operational cost.",
  "[assumption] Mixnet latency and throughput are sufficient for RNS announce propagation and link establishment without protocol-level timeout changes (per-iface tuning at most).",
  "[assumption] Bumping the workspace lockfile tokio from 1.44.2 to >=1.47 (required by nym-sdk) does not raise the effective toolchain floor for default-member crates.",
  "Endpoint extension for tunnel payloads: kind discriminator enum vs optional nym_recipient field — which preserves msgpack wire-compat with Python styrened?",
  "Should tunnel control (0xD8-0xDE) over Nym ride LXMF-over-NymIface (no new message types) or a direct Nym Stream side-channel?",
  "Per-iface announce/path-request timing: does core_transport need per-interface timeout overrides for high-latency bearers, and does that generalize to LoRa too?",
  "How does a peer discover another peer's Nym recipient address — announce app-data extension, LXMF field, or out-of-band exchange?",
  "What is the reconnect/failover story when the entry gateway drops — does InterfaceDriver's lifecycle handle it or does NymIface need internal retry?",
  "Stream idle timeout (default 30 min) will reap quiet iface streams — keepalive frames, reopen-on-EOF, or builder-configured longer timeout?",
  "Dialer topology: which side calls open_stream vs listener().accept() — config-driven peer list (tcp_client-style) or accept-any (tcp_server-style), or both kinds?",
]
+++

# Nym Mixnet as Styrene Signaling Substrate

## Overview

Pair the [Nym mixnet](https://nym.com/docs) with the Styrene stack as a **metadata-private signaling and rendezvous substrate**. Nym fills the one gap the current architecture does not cover: traffic analysis by a global passive adversary. PQC tunnels (docs/pqc-tunnel-architecture.md) protect content against harvest-now-decrypt-later, but a WireGuard tunnel between two peers — and RNS over plain TCP internet interfaces — still reveals **who talks to whom, when, and how much**. Nym's Sphinx packet format (uniform size, layered encryption, per-packet randomized delays, cover traffic; Loopix lineage) is purpose-built for exactly that residual threat.

Nym is **not** a media plane. Per-packet mixing delay is the anonymity mechanism; latency is intrinsic. This is consistent with the standing decision that live voice runs over established Styrene tunnels (docs/styrene-voice-overlay-addressing.md). Nym sits on the control-plane side, next to RNS/LXMF.

The developer surface is Rust-native and tokio-based (`nym-sdk`), matching this workspace's runtime.

## Placement in the Two-Plane Model

| Plane | Today | With Nym |
|---|---|---|
| Control / signaling | RNS links + LXMF over TCP/UDP/LoRa | + RNS over Nym mixnet (`NymIface`) — metadata-private internet bearer |
| Tunnel bootstrap | TunnelOffer/Accept over LXMF (0xD8–0xDE) | + rendezvous over mixnet; peer relationship hidden from backhaul observers |
| Data / media | PQC WireGuard/strongSwan tunnels | unchanged — Nym explicitly excluded |

**Design rule carried over:** Sphinx key exchange is X25519 — not post-quantum. A Nym channel is, like an RNS link, a *signaling channel with a shelf life*. Bootstrap PQC tunnels through it; never send long-lived secrets over it. Note that `TunnelOffer.psk` already transits LXMF today, so a Nym rendezvous path is the same trust class with strictly better metadata properties.

## Integration Point 1 — `NymIface` (RNS bearer)

**Where:** `crates/libs/styrene-rns/src/transport/iface/` (new `nym.rs` alongside `tcp_client.rs`, `udp.rs`, `serial.rs`).

**Mechanism:** `stream_iface.rs` provides `run_hdlc_rx_loop` / `run_hdlc_tx_loop`, generic over tokio `AsyncRead + AsyncWrite` — documented as the extension point for new stream transports. The nym-sdk **Stream module** provides multiplexed `AsyncRead + AsyncWrite` byte streams between two Nym clients (E2E, both sides controlled — not proxy/exit mode). Integration is:

1. Construct a `MixnetClient` and open a stream to the peer's Nym recipient address.
2. Split into read/write halves; hand them to the existing HDLC loops.
3. IFAC authentication and packet serialization come free from the shared pipeline.

Announces, path requests, links, and LXMF then flow over the mixnet like any other bearer — RNS bearer-agnosticism means this is additive, not architectural surgery.

**Config:** `styrened` `InterfaceConfig.kind` is an open string enum (`"tcp_client"`, `"tcp_server"`); add `kind = "nym"` with a `recipient` field (peer Nym address) and optional gateway selection.

**Dependency isolation:** nym-sdk is a heavy dependency (WebSocket gateway client, Sphinx crypto, cover-traffic scheduler). **Decided:** separate `styrene-nym` crate, excluded from default-members — see Decisions. A feature flag on `styrene-rns` cannot scope nym-sdk's 1.87 MSRV and would drag its ungated dependency tree into the core crate's manifest.

## Integration Point 2 — Tunnel Rendezvous over Mixnet

**Where:** `crates/libs/styrene-mesh/src/tunnel_payloads.rs` + `crates/libs/styrene-tunnel/src/orchestrator/`.

**Today:** `TunnelOffer` / `TunnelAccept` carry `endpoint: String` (WireGuard IP:port) and transit LXMF. The discovery-to-tunnel pipeline (docs/pqc-tunnel-architecture.md) leaks the peer relationship to anyone watching the backhaul during bootstrap, and the resulting WG tunnel endpoints are directly observable.

**Proposal:**
- Extend the endpoint representation with a kind discriminator (direct IP:port vs Nym recipient address vs future Tor/I2P/Yggdrasil), or add an optional `nym_recipient` field — wire-compat with Python `styrened` must be checked either way.
- Allow the tunnel control channel (offer/accept/rekey/keepalive/teardown, 0xD8–0xDE) to run over the mixnet — either via `NymIface` (LXMF-over-Nym, no new message types) or via a direct Nym Stream side-channel.
- This subsumes the open question in the voice design node about retaining Tor/I2P/Yggdrasil endpoint advertisements as tunnel rendezvous helpers: Nym is the strongest candidate for that role (decentralized, incentivized, GPA-resistant, Rust-native SDK).

**Residual exposure (named):** once the WG tunnel is up, the tunnel itself reveals the peer pair to a backhaul observer. Mixnet rendezvous protects *bootstrap* metadata, not steady-state data-plane metadata. Hiding the data plane would mean carrying it over Nym, which conflicts with the latency/bandwidth needs of the media plane — explicitly out of scope.

## Integration Point 3 (deferred) — smolmix exit proxy

`smolmix` provides `TcpStream`/`UdpSocket` via a userspace smoltcp stack, exiting to clearnet through an IPR exit gateway — the structural sibling of `styrene-i2p` (local proxy over mesh to hub's i2pd). A `styrene-nym` proxy app could give mesh nodes anonymous clearnet egress. Deferred: it serves a different goal (client anonymity to third-party services) than the mesh-internal pairing, and smolmix's API is still churning (pin minor versions per upstream guidance).

## Costs

- **Not post-quantum.** Sphinx is X25519-based. Shelf-life discipline applies (same as RNS links).
- **Internet + gateway dependency.** Client connects via WebSocket to an entry gateway. Internet-backhaul substrate only; irrelevant to LoRa/off-grid bearers. Semi-trusted infrastructure RNS itself doesn't require.
- **Economic coupling.** Bandwidth credentials (NYM token / zk-nym) required to send. Operational and sovereignty tradeoff for free-standing deployments; quantify the free-tier / credential-acquisition story before committing.
- **Latency.** Mixing delays make Nym-borne signaling slower than direct TCP. Announce propagation and path-request timing assumptions in `core_transport` (announce retry limits, path request timeouts) may need per-iface tuning.
- **Dependency weight.** nym-sdk pulls a large ungated tree (no cargo features yet); isolated via separate non-default-member crate (see Decisions).
- **API stability.** nym-sdk / smolmix warn of breaking changes between minor releases; pin versions.

## Research Findings (2026-07-12)

### nym-sdk publication and API surface

- Published on crates.io: **nym-sdk 1.21.2** (2026-06-24), Apache-2.0, MSRV **1.87.0**. The Stream module ships in the published crate at `nym_sdk::mixnet::stream`.
- `MixnetStream` implements tokio `AsyncRead + AsyncWrite` (verified in source, `sdk/rust/nym-sdk/src/mixnet/stream/mixnet_stream.rs`). Fields are `PollSender<InputMessage>`, `mpsc::UnboundedReceiver<Vec<u8>>`, `BytesMut`, `Box<Recipient>`, plain scalars — all `Send + Unpin`, no `PhantomPinned`, so the struct auto-derives `Send + Unpin`. `tokio::io::split()` halves therefore satisfy `run_hdlc_rx_loop`/`run_hdlc_tx_loop` bounds (`AsyncRead/AsyncWrite + Unpin + Send`) **without adapter shims**. Final confirmation is a one-line static assertion in the spike.
- API pattern: dialer calls `client.open_stream(recipient, surbs)`; acceptor calls `client.listener()` then `accept()`. Streams multiplex over one `MixnetClient`. Inbound replies travel via SURBs — the acceptor never learns the dialer's Nym address (nice metadata property; complicates symmetric peering).
- EOF semantics match the HDLC rx loop's exit condition (read returns 0 bytes when the stream channel closes).

### API caveats that shape NymIface

- **Streams and messages are mutually exclusive per client** — calling `open_stream()`/`listener()` permanently disables `send_plain_message` on that `MixnetClient`. NymIface must own a dedicated stream-mode client.
- **`poll_flush` is a no-op; `poll_write` succeeds whenever the input channel is ready** — backpressure is channel-level only, and there is no wire-level close (drop deregisters locally). Loss/congestion behavior differs from TCP; RNS's own reliability layer (links, receipts) sits above, which is the right division.
- **Default stream idle timeout is 30 minutes** — a quiet iface stream gets reaped by the router's cleanup task and subsequent reads return EOF. Needs keepalive traffic, reopen-on-EOF, or `MixnetClientBuilder::with_stream_idle_timeout`.
- **No cargo feature gates in nym-sdk yet** (explicit in crate docs): importing it pulls the full tree — `nym-client-core`, `nym-sphinx`, `nym-validator-client` (Cosmos chain client), credential/SURB fs storage, `bip39`, `clap`, `tracing-subscriber`, plus two proxy binaries. This is a heavyweight dependency by construction, not by option.

### Dependency and toolchain collisions with this workspace

| Axis | styrene-rs | nym-sdk 1.21.2 | Consequence |
|---|---|---|---|
| MSRV | workspace `rust-version = 1.75` | `1.87.0` | Cannot be a dependency of any default-member crate |
| tokio | 1.44.2 | ^1.47 | Lockfile bumps to ≥1.47 workspace-wide (tokio's own MSRV stays low; verify) |
| thiserror | 1 | ^2.0 | Duplicate major versions coexist; tree bloat only |
| rand_core | 0.6.4 | rand ^0.8 (rand_core 0.6) | Compatible |

## Decisions

1. **Separate crate `crates/libs/styrene-nym`, not a feature flag on styrene-rns.** Three independent lines of evidence converge:
   - **MSRV:** nym-sdk requires 1.87; workspace pins 1.75. `rust-version` is crate-wide metadata — a feature flag cannot scope it. A separate crate declares its own `rust-version = "1.87"`.
   - **The repo's own extension architecture:** `iface/driver.rs` states drivers "should implement these traits in external crates and integrate through the public interface manager API" — `InterfaceDriver`/`InterfaceDriverFactory` exist precisely for this.
   - **Dependency weight is not optional:** nym-sdk has no feature gates; a styrene-rns feature would drag the full Cosmos-client tree into the core protocol crate's manifest. The `no_std`-capable core stays clean only with a hard crate boundary.

   Follow the `styrene-dx` precedent: workspace member, **excluded from default-members** ("build explicitly with `-p styrene-nym`"), so the 1.75 toolchain keeps building the default workspace.

2. **NymIface composes public APIs, not new traits:** construct `MixnetClient` → `open_stream`/`listener().accept()` → `tokio::io::split` → hand halves to `run_hdlc_rx_loop`/`run_hdlc_tx_loop`, registering via the public `InterfaceChannel` machinery. IFAC and packet serde inherited.

## Suggested First Slice

A spike proving the bearer path end-to-end, smallest possible scope:

1. New crate `crates/libs/styrene-nym` (workspace member, excluded from default-members, `rust-version = "1.87"`) wrapping nym-sdk Stream into the `InterfaceDriver` shape.
2. Wire `kind = "nym"` into `styrened` config → iface spawn.
3. E2E test in `crates/tests/styrene-e2e`: two nodes exchange an LXMF message where the only shared bearer is the mixnet.
4. Measure: announce propagation latency, link establishment RTT, LXMF delivery time over mixnet vs TCP baseline.

Tunnel rendezvous (Integration Point 2) builds on the proven bearer and is a separate change with wire-protocol compatibility review.

