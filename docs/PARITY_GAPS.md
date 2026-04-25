# Styrene-RS Parity Gaps & Architecture Decisions

> Generated 2026-02-28. Living document — update as gaps close.

## Current State Summary

| Metric | Value |
|--------|-------|
| Total Rust LOC | ~37K across 6 crates |
| Tests | 212 passing (unit + interop) |
| Fork date | 2026-02-24 (from FreeTAKTeam/LXMF-rs) |
| Python styrened LOC | ~35K (services, protocols, terminal, TUI, IPC, RPC, daemon) |
| Upstream drift | 89 commits reviewed 2026-03-23 — adoption queue in `docs/upstream-sync-log.md` |

The Rust port has strong protocol-layer coverage (identity, crypto, packets, links, LXMF wire format) and an extensive RPC surface (60+ methods). What it lacks is the **service layer** — the application logic that makes styrened a mesh communications platform rather than a raw protocol daemon.

**✅ Upstream parity porting complete (2026-03-23):** FreeTAKTeam upstream assembled a 41-issue
Rust/Python compatibility list and merged fixes for 15 of them. All 28 priority adoption commits
have been ported in session 2026-03-23. See `docs/upstream-sync-log.md` and `docs/COMPAT_ISSUES.md`.

---

## Tier 1: Core Gaps (Blocks Real Deployment)

### 1.0 Upstream Protocol Correctness Backlog

**Status:** ✅ Closed (2026-03-23) — all 28 priority adoption commits ported  
**Source:** FreeTAKTeam upstream PRs #106–#131 — see `docs/upstream-sync-log.md` for full triage  

Upstream assembled and addressed a 41-issue Rust/Python compatibility list. The following gaps
were ported from the upstream repo into styrene-rs:

| Issue | Description | Upstream PR | styrene-rs crate |
|-------|-------------|-------------|-----------------|
| 1 | Announce validation accepts hash mismatch | #106 | `styrene-rns` |
| 2 | Packet receipts satisfied by forged proofs | #106 | `styrene-rns` |
| 5 | Link activation has proof race | #107 | `styrene-rns` |
| 6 | Resource startup reports success prematurely | #112 | `styrene-rns` |
| 7 | Outbound resources lack retry/timeout/cleanup | #112 | `styrene-rns` |
| 8 | Failed inbound resources stuck forever | #112 | `styrene-rns` |
| 9 | Duplicate resource adverts reset receive progress | #112 | `styrene-rns` |
| 11 | Known-destination pubkey stability check missing | #106 | `styrene-rns` |
| 12 | Ratchet-bearing announce parsing too permissive | #106 | `styrene-rns` |
| 13 | Transported link-request proofs skip Python gates | #106 | `styrene-rns` |
| 14 | Link interface binding not enforced | #107 | `styrene-rns` |
| 15 | Channel packet semantics not implemented | #109+#123+#111 | `styrene-rns` |
| 16 | Link proof behavior differs from Python | #107 | `styrene-rns` |
| 17 | Link watchdog timing fixed-interval not RTT | #107 | `styrene-rns` |
| 19 | Inbound worker assumes every resource is LXMF | #112 | `styrene-rns` |

Additionally, these upstream fixes address issues that appear in our **Tier 2/3** gaps:

| Issue | Description | Upstream PR | styrene-rs crate |
|-------|-------------|-------------|-----------------|
| 20, 21, 22 | Path tag not preserved, duplicate suppression unbounded | #115 | `styrene-rns` |
| 23, 24, 25 | Announce throttling/queueing/ingress not interface-aware | #117, #121 | `styrene-rns` |
| 27, 28 | Announce retry timing and rate limiting wrong | #122, #125 | `styrene-rns` |
| 30, 31 | Stamp/ticket options ignored in send path | #126 | `styrened-rs` |
| 33, 34 | Propagation stamp validation and cost retention | #129, #119 | `styrene-lxmf` |
| 36 | Propagation transient-id lifecycle incomplete | #130, #131 | `styrene-lxmf` |

Also new: a **complete interop test harness** (PRs #116, #127) — `python_compat_matrix.rs` +
shell smoke script — is the scaffold for our own interop gate.

**All ported (2026-03-23).** Protocol correctness now matches upstream. Service-layer work is
next in priority order. Open issues (3, 4, 18, 20–43 minus those closed above) are tracked in
`docs/COMPAT_ISSUES.md`.

---

### 1.1 IFAC Multi-Hop Bug (Inherited)

**Status:** ✅ Resolved (2026-04-19)  
**Files:** `crates/libs/styrene-rns/src/transport/iface/ifac.rs` (276 LOC, 9 tests)  

Full IFAC wrap/unwrap algorithm implemented at the interface boundary (not transport layer).
Each forwarding hop re-applies IFAC for its outbound interface, so the token is always fresh.
Multi-hop tests pass (3-hop and 4-hop chains). Cross-language interop fixtures verify
byte-identical output with Python (`tests/interop/fixtures/ifac_vectors.json`).

**Config plumbing:** `InterfaceContext` now carries `Option<Arc<IfacConfig>>`, wired through
TCP client, TCP server (propagated to accepted clients), and serial interfaces.
UDP rejects IFAC-flagged packets but does not support IFAC wrap/unwrap (no HDLC pipeline).

### 1.2 Serial/KISS Interface

**Status:** ✅ Implemented (2026-04-19)  
**Files:** `crates/libs/styrene-rns/src/transport/iface/serial.rs`, `kiss.rs`  
**Feature gate:** `serial` (adds `tokio-serial`)  

Serial interface with async reconnection (`serial.rs`, ~200 LOC). KISS codec with full
FEND/FESC byte-stuffing (`kiss.rs`, 230 LOC, 8 tests). `KissReader`/`KissWriter` adapters
wrap the serial port for transparent KISS framing — the HDLC loops work unchanged.

- `SerialInterface::new(path, baud)` — raw HDLC (direct serial)
- `SerialInterface::new_kiss(path, baud)` — KISS+HDLC (TNC/RNode devices)
- IFAC support wired via `InterfaceContext.ifac`

**Remaining:** Hardware validation on RNode and RP2040 devices.

### 1.3 Propagation (Store-and-Forward)

**Status:** RPC stubs exist, no backend  
**Files:** `rpc/daemon/dispatch_legacy_propagation.rs`  

LXMF propagation nodes store messages for offline recipients and sync with peers. The Rust daemon has the RPC method routing but no storage backend, no sync protocol, no ingest/fetch cycle. Without this, messages to offline nodes are lost.

**Scope:** ~800-1200 lines. Needs a `PropagationStore` (SQLite table), ingest handler, fetch handler, peer sync state machine.

### 1.4 Configuration System

**Status:** Basic TOML parsing  
**Files:** `crates/apps/styrened-rs/src/config.rs`  
**Reference:** Python `services/config.py` (1,048 LOC)  

Missing: interface hot-reload, per-interface IFAC keys, auto-discovery configuration, hub connection settings, graceful degradation policies for constrained devices. The Python config drives which services start based on device capabilities — the Rust daemon starts everything or nothing.

---

## Tier 2: Service Layer Gaps

### 2.1 Conversation Service

**Reference:** Python `services/conversation_service.py` (1,316 LOC)  

Threading LXMF messages into conversations, read tracking, conversation-level queries, message search. The Rust daemon stores flat messages in SQLite — no conversation abstraction. Required for any UI (TUI or Dioxus) to show a chat interface.

### 2.2 Auto-Reply

**Reference:** Python `services/auto_reply.py` (634 LOC)  

Autonomous message handling for unattended edge nodes. Pattern-matched responses, status queries, command dispatch. Critical for headless edge deployments where no operator is present.

### 2.3 Node Store & Discovery

**Reference:** Python `services/node_store.py` (964 LOC), `services/reticulum.py` (1,293 LOC)  

Peer node registry with persistence, health tracking, identity-to-device mapping, announce-based discovery with callbacks. The Rust daemon has `peers` in memory (a `HashMap<String, PeerRecord>`) but no persistence or discovery loop.

### 2.4 Remote Terminal

**Reference:** Python `terminal/service.py` (2,121 LOC), `terminal/client.py` (848 LOC)  

SSH-like shell sessions over LXMF links. Flagship feature for remote edge management. No Rust equivalent exists. Depends on links + resources being stable (they are) but also needs PTY handling, session multiplexing, and the Styrene protocol framing.

### 2.5 Styrene Protocol & Protocol Registry

**Reference:** Python `protocols/registry.py`, `protocols/styrene.py` (658 LOC), `protocols/chat.py`, `protocols/read_receipt.py`  

Pluggable protocol dispatch over LXMF. The Python daemon registers protocol handlers by type code. Incoming LXMF messages are routed to the appropriate handler (chat, terminal, RPC, file transfer). The Rust daemon has no protocol dispatch — all inbound messages go through a single `inbound_worker` path.

### 2.6 File Transfer

**Reference:** Python `services/file_transfer.py` (637 LOC)  

Chunked file transfer over LXMF resources. Used for firmware updates to edge devices, log retrieval, config push. Builds on the resource layer which exists in Rust.

### 2.7 Hub Connection

**Reference:** Python `services/hub_connection.py` (342 LOC)  

Auto-connect to a hub transport node with reconnection, health monitoring, and graceful fallback. Edge devices use this to maintain mesh presence through a hub. The Rust daemon requires manual `--transport` flag with a static address.

---

## Tier 3: Protocol & Crypto Gaps

### 3.1 Styrene Wire Interop

**Status:** ✅ Resolved — cross-language test vectors in place  
**Files:** `crates/libs/styrene-mesh/tests/wire_interop.rs`, `tests/interop/fixtures/wire_manifest.json`  

Python fixture generator produces 13 V2 + 2 V1 binary fixtures. Rust tests verify decode
and roundtrip byte-identical re-encode. Coverage includes all message types (Ping, Chat,
Exec, Terminal, PQC, Error, large payloads, binary payloads). IFAC interop vectors added
separately (`tests/interop/fixtures/ifac_vectors.json`).

### 3.2 PQC Integration

**Status:** `ml-kem` crate in workspace deps, unused  
**Blocked by:** `styrene-entropy` crate (Gap 3.4) — ML-KEM key generation requires ~800+ bytes of quality entropy per keypair. Activating ml-kem without a verified entropy source silently degrades key quality.  
**Reference:** Python `crypto/pqc_crypto.py` (unlisted LOC), `services/pqc_session.py` (424 LOC)  

ML-KEM (Kyber) post-quantum key exchange. The dependency is declared but never imported. Python has a working PQC session layer with hybrid key exchange (X25519 + ML-KEM).

Do not activate this dependency until Gap 3.4 (`styrene-entropy`) is in place and wired into `AppContext`. ML-KEM key generation drawing from a weak entropy source produces correlated key material with no visible error.

### 3.4 Entropy Architecture

**Status:** Not started — design complete  
**Reference:** `docs/entropy-architecture.md`  

`styrene-entropy` — a new lib crate providing a source → pool → DRBG abstraction for all cryptographic key material generation in the daemon and Hub. Sources: hardware TRNG (nRF52840 coprocessor via UART), kernel (`/dev/random`), CPU jitter, mesh Hub pool (via LXMF RPC).

**This is a prerequisite for Gap 3.2 (PQC Integration).** Wire into `AppContext` (Gap S5) when the service registry lands.

Independent of all service layer work — can be built and tested standalone. See `docs/entropy-architecture.md` for full design including hardware coprocessor spec and DRBG policy.

---

### 3.3 Ratchet Persistence

**Status:** ✅ Resolved  
**Files:** `crates/libs/styrene-rns/src/transport/ratchet_store.rs` (160 LOC)  

Disk-backed persistence implemented: directory-based storage with MessagePack serialization.
In-memory cache + lazy disk load. Atomic writes (write-to-tmp + rename). 30-day expiry with
background cleanup. Ratchet state survives daemon restarts.

---

## Structural Decisions: Python → Rust

These are not bugs — they are architectural choices inherited from the upstream fork that need deliberate redesign, not 1:1 porting.

### S1. Kill `Rc<RpcDaemon>` — Go Multi-Threaded

**Current:** `Rc<RpcDaemon>` + `tokio::main(flavor = "current_thread")` + `LocalSet`  
**Problem:** Artificially constrains the daemon to a single OS thread. On a Pi 4B (4 cores) or any real server, this wastes 75% of available compute.  
**Root cause:** Upstream avoided `Send` bounds on state. `RpcDaemon` contains 40+ `Mutex<T>` fields — all `std::sync::Mutex`, which is `Send + Sync`. The `Rc` is the only thing preventing `Arc`.

**Fix:** Replace `Rc<RpcDaemon>` with `Arc<RpcDaemon>`, remove `LocalSet`, switch to `tokio::main(flavor = "multi_thread")`. This is mechanical — the `Mutex` fields are already thread-safe. The `Rc` in `test_bridge.rs` (`Rc<dyn Fn>`) needs a similar upgrade to `Arc<dyn Fn + Send + Sync>`.

**Graceful degradation for single-core:** Tokio's multi-thread runtime degrades naturally on single-core devices — it runs the work-stealing scheduler with one worker thread, which is functionally equivalent to `current_thread` but doesn't require `Rc`. No artificial constraint needed.

### S2. Extract `Transport` Trait

**Current:** `Transport` is a 500+ line concrete struct with direct `tokio::spawn`, `Arc<Mutex<InterfaceManager>>`, and hardcoded TCP/UDP.  
**Problem:** Can't mock for tests, can't compile to WASM, can't swap transport backends.

**Fix:** Define a `MeshTransport` trait:

```rust
#[async_trait]
pub trait MeshTransport: Send + Sync {
    async fn send_packet(&self, packet: Packet) -> SendPacketOutcome;
    async fn send_announce(&self, destination: &Destination, app_data: Option<&[u8]>);
    async fn request_path(&self, destination: &AddressHash, iface: Option<AddressHash>, tag: Option<TagBytes>);
    async fn link(&self, destination: DestinationDesc) -> Arc<Mutex<Link>>;
    fn subscribe_announces(&self) -> broadcast::Receiver<AnnounceEvent>;
    fn subscribe_inbound(&self) -> broadcast::Receiver<ReceivedData>;
    async fn destination_identity(&self, address: &AddressHash) -> Option<Identity>;
}
```

The existing `Transport` struct becomes `TokioTransport: MeshTransport`. A `WasmTransport` implements the same trait over WebSocket. A `MockTransport` provides test doubles.

### S3. Extract `ByteStream` Trait from Interface Layer

**Current:** Each interface (TCP client, TCP server, UDP) reimplements the full read→HDLC→deserialize→channel pipeline (~200 LOC each, mostly duplicated).  
**Problem:** Adding Serial/KISS or WebSocket means copying 200 more lines. WASM has no TCP.

**Fix:**

```rust
#[async_trait]
pub trait ByteStream: Send + 'static {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), TransportError>;
    async fn flush(&mut self) -> Result<(), TransportError>;
}
```

Then a single generic `fn run_framed_interface<S: ByteStream>()` handles HDLC encode/decode for all transports. Platform-specific code shrinks to constructing the stream.

### S4. Replace `include!()` with Module Hierarchy

**Current:** `rpc/daemon.rs` uses 16 `include!()` macros to stitch ~8K lines into one compilation unit.  
**Problem:** Defeats IDE navigation, rust-analyzer struggles, all code shares one `impl RpcDaemon` block, no visibility control.

**Fix:** Convert each `include!()` file to a proper submodule:
```
rpc/daemon/
  mod.rs          (re-exports + handle_rpc dispatch)
  negotiate.rs
  runtime.rs
  topics.rs
  attachments.rs
  markers.rs
  identity.rs
  outbound.rs
  events.rs
  metrics.rs
  legacy/
    messages.rs
    propagation.rs
    misc.rs
    clear.rs
```

Methods move to trait impls or free functions that take `&RpcDaemon`. This is a refactor-only change with no behavioral impact.

### S5. Service Architecture (Not a 1:1 Port)

Python styrened uses singleton services accessed via `get_lxmf_service()`, `get_node_store()` etc. — module-level globals with lazy initialization. This is a Python pattern that doesn't translate to Rust.

**Rust approach:** A `ServiceRegistry` or `AppContext` struct owns all services:

```rust
pub struct AppContext {
    pub transport: Arc<dyn MeshTransport>,
    pub messages: MessagesStore,
    pub conversations: ConversationService,
    pub node_store: NodeStore,
    pub auto_reply: Option<AutoReplyHandler>,
    pub protocols: ProtocolRegistry,
    pub config: DaemonConfig,
}
```

Services receive `Arc<AppContext>` and subscribe to transport events via channels. This replaces both the Python singleton pattern and the current `RpcDaemon` god-struct (40+ Mutex fields).

The `RpcDaemon` becomes a thin dispatch layer that delegates to services on the `AppContext`, rather than owning all state itself.

### S6. Graceful Degradation for Edge

Python styrened conditionally starts services based on `CoreConfig` and device capabilities. The Rust daemon must do the same — but the mechanism is Cargo features + runtime config, not Python's dynamic `try/except ImportError`.

**Pattern:**
- **Compile-time:** Feature flags for heavy optional deps (`serial`, `pqc`, `tui`, `web`). A Pi Zero 2W build can exclude TLS, PQC, web UI.
- **Runtime:** `DaemonConfig` lists enabled services. Services that fail to initialize log a warning and the daemon continues with reduced capability — never crash on a missing optional service.
- **Memory budget:** Configurable limits on message store size, announce cache, peer table. Constrained devices get smaller defaults.

This is orthogonal to thread count. A single-core Pi Zero 2W runs `tokio::main(flavor = "multi_thread")` with `worker_threads = 1` and gets identical behavior to `current_thread` — except the codebase doesn't need `Rc` and `LocalSet` scaffolding.

---

## Priority Order

| # | Item | Type | Effort | Status |
|---|------|------|--------|--------|
| ~~1~~ | ~~S1: `Rc` → `Arc`, multi-thread runtime~~ | ~~Structural~~ | ~~Small~~ | ✅ Done |
| ~~2~~ | ~~S3: `ByteStream` trait~~ | ~~Structural~~ | ~~Medium~~ | ✅ Done |
| ~~3~~ | ~~1.1: IFAC fix~~ | ~~Bug~~ | ~~Medium~~ | ✅ Done |
| ~~4~~ | ~~1.2: Serial/KISS interface~~ | ~~Feature~~ | ~~Medium~~ | ✅ Done |
| ~~5~~ | ~~S2: `MeshTransport` trait~~ | ~~Structural~~ | ~~Medium~~ | ✅ Done |
| ~~6~~ | ~~S4: `include!()` → modules~~ | ~~Refactor~~ | ~~Small~~ | ✅ Done |
| 7 | **3.4: `styrene-entropy` crate** | **Feature** | **Medium** | Scaffolded |
| ~~8~~ | ~~S5: `AppContext` service registry~~ | ~~Structural~~ | ~~Large~~ | ✅ Scaffolded + wired |
| 9 | 1.3: Propagation backend | Feature | Large | RPC stubs only |
| ~~10~~ | ~~3.1: Wire interop vectors~~ | ~~Testing~~ | ~~Small~~ | ✅ Done |
| 11 | 3.2: PQC integration | Feature | Medium | Blocked by 3.4 |
| 12 | 2.1–2.7: Service layer | Feature | Very large | Partially scaffolded |
