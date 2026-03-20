# Styrene-RS Parity Gaps & Architecture Decisions

> Generated 2026-02-28. Living document — update as gaps close.

## Current State Summary

| Metric | Value |
|--------|-------|
| Total Rust LOC | ~37K across 6 crates |
| Tests | 212 passing (unit + interop) |
| Fork date | 2026-02-24 (from FreeTAKTeam/LXMF-rs) |
| Python styrened LOC | ~35K (services, protocols, terminal, TUI, IPC, RPC, daemon) |

The Rust port has strong protocol-layer coverage (identity, crypto, packets, links, LXMF wire format) and an extensive RPC surface (60+ methods). What it lacks is the **service layer** — the application logic that makes styrened a mesh communications platform rather than a raw protocol daemon.

---

## Tier 1: Core Gaps (Blocks Real Deployment)

### 1.1 IFAC Multi-Hop Bug (Inherited)

**Status:** Open, critical  
**Files:** `crates/libs/styrene-rns/src/transport/core_transport/`  

Authenticated interfaces (IFAC) reject forwarded packets because the HMAC validation doesn't account for hop-modified headers. Single-hop works. Multi-hop networks — the actual deployment topology — do not.

**Fix scope:** Requires understanding how Python RNS strips/reattaches IFAC before forwarding. Likely 100-200 lines in `handler.rs` / `core.rs`.

### 1.2 Serial/KISS Interface

**Status:** Not started  
**Blocks:** Edge deployment on LoRa hardware (RNode, RP2040, ESP32)  
**Reference:** Python `RNS.Interfaces.SerialInterface`, `RNS.Interfaces.KISSInterface`  

The stated reason for the Rust port is constrained edge devices. Without serial transport, the binary can only communicate over TCP/UDP — no different from running Python with less ecosystem maturity.

**Implementation:** Add `serial.rs` to `crates/libs/styrene-rns/src/transport/iface/`. Requires a serial crate (`tokio-serial` or `serialport`). HDLC framing is already implemented and reusable. KISS framing is a separate encoder (FEND/FESC byte stuffing) — ~150 lines.

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

**Status:** `styrene-mesh` crate exists (670 LOC), no cross-language test vectors  

The wire protocol is the contract between Python and Rust nodes. There are interop fixtures for RNS primitives (identity, packets, HDLC, Fernet, announces) but **none for the Styrene envelope format**. Two implementations with no shared test vectors is a bug waiting to happen.

### 3.2 PQC Integration

**Status:** `ml-kem` crate in workspace deps, unused  
**Reference:** Python `crypto/pqc_crypto.py` (unlisted LOC), `services/pqc_session.py` (424 LOC)  

ML-KEM (Kyber) post-quantum key exchange. The dependency is declared but never imported. Python has a working PQC session layer with hybrid key exchange (X25519 + ML-KEM).

### 3.3 Ratchet Persistence

**Status:** In-memory only  
**Files:** `crates/libs/styrene-rns/src/transport/ratchet_store.rs`  

Ratchet state must survive daemon restarts or forward secrecy breaks on reboot. Python persists to disk. Rust keeps it in memory.

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

| # | Item | Type | Effort | Unblocks |
|---|------|------|--------|----------|
| 1 | S1: `Rc` → `Arc`, multi-thread runtime | Structural | Small (mechanical) | Everything — removes artificial constraint |
| 2 | S3: `ByteStream` trait | Structural | Medium | Serial, WASM, dedup |
| 3 | 1.1: IFAC fix | Bug | Medium | Real multi-hop mesh |
| 4 | 1.2: Serial/KISS interface | Feature | Medium | Edge hardware deployment |
| 5 | S2: `MeshTransport` trait | Structural | Medium | Testability, WASM, service architecture |
| 6 | S4: `include!()` → modules | Refactor | Small | Developer experience |
| 7 | S5: `AppContext` service registry | Structural | Large | All Tier 2 services |
| 8 | 1.3: Propagation backend | Feature | Large | Offline message delivery |
| 9 | 3.1: Wire interop vectors | Testing | Small | Confidence in cross-impl compat |
| 10 | 2.1–2.7: Service layer | Feature | Very large | Feature parity |
