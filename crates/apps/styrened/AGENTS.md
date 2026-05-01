# styrened

Daemon binary for the Styrene mesh communications system. Implements the RNS/LXMF protocol stack in Rust, serving as the primary runtime for mesh networking, message delivery, and fleet management.

## What it does

styrened is a long-running daemon that:

- Maintains a Reticulum transport instance (TCP server/client interfaces)
- Sends and receives LXMF messages over the mesh
- Exposes two dispatch layers: a legacy JSON-RPC/MessagePack endpoint (port 4243) and a new Unix socket IPC server implementing the `styrene-ipc` Daemon trait
- Manages identity (X25519+Ed25519), peer discovery, announce processing, and delivery receipts
- Supports three node roles: `full_node` (routes packets), `hub` (propagation store), `propagation_client` (thin client)
- Serves NomadNet-compatible pages via the page service

## Architecture

Two-layer dispatch model, running in parallel during migration:

```
                  +------------------+
                  |   CLI / TUI      |
                  +--------+---------+
                           |
              Unix socket (daemon.sock)
                           |
                  +--------v---------+
                  |  DaemonFacade    |  <-- new: Daemon trait, RBAC, typed IPC
                  |  (styrene-ipc)   |
                  +--------+---------+
                           |
                  +--------v---------+
                  |   AppContext      |  <-- composition root, service graph
                  |   (12 services)  |
                  +--------+---------+
                           |
              +------------+-------------+
              |                          |
   +----------v---------+    +----------v---------+
   |  MeshTransport     |    |  MessagesStore     |
   |  (RNS transport)   |    |  (SQLite)          |
   +--------------------+    +--------------------+

   --- Legacy path (being replaced) ---

              TCP :4243 (JSON-RPC / MessagePack)
                           |
                  +--------v---------+
                  |   RpcDaemon      |  <-- legacy: god struct, manual dispatch
                  +------------------+
```

### Call direction

IPC -> DaemonFacade -> AuthService.check() -> Service -> storage/transport

Services never call DaemonFacade. Services access each other through AppContext accessors.

## Module map

### `src/bin/styrened/`

| File | Purpose |
|------|---------|
| `main.rs` | Entrypoint. Parses CLI args (clap), calls bootstrap, runs RPC loop. |
| `bootstrap.rs` | Service wiring. Constructs transport, identity, stores, AppContext, DaemonFacade, IPC server. 300+ lines of startup orchestration. |
| `rpc_loop.rs` | Legacy RPC event loop (TCP, optional TLS). |
| `bridge.rs` | TransportBridge -- connects RNS transport to RpcDaemon. |
| `inbound_worker.rs` | Legacy inbound message processing (feeds RpcDaemon). |
| `announce_worker.rs` | Legacy announce processing (feeds RpcDaemon). |
| `receipt_worker.rs` | Legacy receipt correlation (feeds RpcDaemon). |

### `src/` (library)

| File | Purpose |
|------|---------|
| `daemon_facade.rs` | IPC-facing Daemon trait impl. Auth + delegation to services. |
| `app_context.rs` | Composition root. Owns all services, wires dependencies. |
| `config.rs` | `DaemonConfig` model, TOML parsing, path defaults (XDG-compliant). |
| `identity_store.rs` | Load/create X25519+Ed25519 identity from disk. |
| `lxmf_bridge.rs` | Build LXMF wire messages for outbound delivery. |
| `inbound_delivery.rs` | Decode inbound LXMF payloads into MessageRecords. |
| `receipt_bridge.rs` | Receipt correlation helpers. |
| `rns_crypto.rs` | Cryptographic primitives (HKDF, HMAC, X25519). |
| `announce_names.rs` | Display name encoding/normalization for announces. |
| `lxmf_stamps.rs` | LXMF stamp validation. |
| `e2e_harness.rs` | Test utilities for end-to-end daemon testing. |

### `src/services/`

| Service | Domain | Package |
|---------|--------|---------|
| `identity` | Identity hash, display name, announce | E |
| `config` | Config load/save/reload, interface enumeration | E |
| `status` | Uptime, interface count, propagation state | E |
| `auth` | RBAC -- caller identity to capability check | E |
| `auto_reply` | Auto-reply mode, message, cooldown | E |
| `fleet` | Remote device status, exec, reboot, inbox | E |
| `messaging` | Conversations, contacts, send/receive, receipts | F |
| `discovery` | Announce ingestion, peer resolution, node store | F |
| `protocol` | Pluggable per-type message handlers (Styrene wire) | G |
| `events` | Pub/sub for messages, devices, links | H |
| `tunnel` | VPN tunnel management (stub) | H |
| `propagation` | Store-and-forward for offline peers | -- |
| `pages` | NomadNet-compatible page serving | -- |

### `src/rpc/`

Legacy JSON-RPC dispatch. MessagePack codec, HTTP transport, event sink. Being superseded by DaemonFacade.

### `src/transport/`

| File | Purpose |
|------|---------|
| `mesh_transport.rs` | `MeshTransport` trait -- abstraction over RNS transport. |
| `adapter.rs` | `TokioTransportAdapter` -- real RNS transport wrapped for async. |
| `null_transport.rs` | `NullTransport` -- no-op impl for tests and no-transport mode. |
| `mock_transport.rs` | Mock for unit tests. |
| `test_bridge.rs` | Test bridge utilities. |

### `src/workers/`

Tokio tasks bridging transport events to the service layer:

| Worker | Feeds |
|--------|-------|
| `inbound` | MessagingService, ProtocolService, PropagationService |
| `announce` | DiscoveryService |
| `link` | EventService |
| `rpc_response` | FleetService |

### `src/storage/`

`MessagesStore` -- SQLite-backed message persistence. Shared between legacy RpcDaemon and new service layer (separate connection handles to the same on-disk database).

## Config model

TOML file at `~/.config/styrene/config.toml` (falls back to `config.yaml` for Python migration).

```toml
role = "full_node"  # or "hub", "propagation_client"

[[interfaces]]
type = "tcp_client"
enabled = true
host = "10.0.0.1"
port = 4242
name = "hub"

[[interfaces]]
type = "tcp_server"
enabled = true
host = "0.0.0.0"
port = 4242
```

Env overrides: `STYRENE_CONFIG_DIR`, `STYRENE_DATA_DIR`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `LXMF_DISPLAY_NAME`.

## Binary targets

| Binary | Description |
|--------|-------------|
| `styrened` | Main daemon. `--rpc`, `--db`, `--config`, `--identity`, `--transport`, `--socket`, `--announce-interval-secs`, TLS flags. |

## Build and test

```bash
cargo check -p styrened
cargo test -p styrened
cargo test -p styrened --no-run   # compile tests only
cargo build -p styrened --release
```

Integration tests in `tests/`:

- `daemon_facade_contract` -- DaemonFacade trait compliance
- `app_context_construction` -- service graph wiring
- `config` -- TOML parsing roundtrips
- `direct_link_delivery` / `direct_link_inbound` -- transport pipeline
- `python_compat_matrix` -- wire format compatibility with Python styrened
- `transport_contract` / `transport_null` -- MeshTransport trait contracts
- `worker_inbound` -- inbound worker decode + persist

## Known issues and TODOs

- **Dual dispatch**: RpcDaemon (legacy) and DaemonFacade (new) run side-by-side. Both connect to the same SQLite database via separate handles. RpcDaemon will be removed once all clients migrate to IPC.
- **ConversationStore**: Uses in-memory SQLite. Needs file-backed storage wired through bootstrap for persistence across restarts.
- **Tunnel service**: Stub only -- all methods return NotImplemented.
- **Terminal (remote shell)**: Stub only.
- **Self-update**: Stub only.
- **Remote page browsing**: Local page serving works; remote fetch over RNS links is not yet implemented.
- **Attachment storage**: LXMF messages carry content inline; separate attachment blob storage is not implemented.
- **CBOR migration**: Wire protocol will move from MessagePack to CBOR (RFC 8949) for deterministic encoding and COSE signing. Mechanical change but requires Python sync.
- **Config file naming**: Default path now prefers `config.toml` but falls back to `config.yaml` for compatibility with Python daemon installs.

## Current status

Active development. The service architecture (AppContext + DaemonFacade) is the target design. Legacy RpcDaemon is feature-complete but frozen -- new functionality goes through the service layer. The IPC server is operational and serves the TUI. Transport, messaging, discovery, and fleet management work end-to-end over real RNS mesh networks.
