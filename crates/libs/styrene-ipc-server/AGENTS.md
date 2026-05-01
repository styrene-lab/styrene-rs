# styrene-ipc-server

Unix socket IPC server exposing `Arc<dyn Daemon>` over a framed msgpack wire protocol. Wire-compatible with the Python `styrened.ipc` protocol, enabling the Python TUI to connect to the Rust daemon as a drop-in replacement.

## Wire Format (IPC)

This is a different protocol from `styrene-mesh` (which is for mesh/LXMF). This one is for local daemon-to-frontend IPC.

```
[LENGTH:4][TYPE:1][REQUEST_ID:16][PAYLOAD:N]

LENGTH:     u32 big-endian, total bytes following (TYPE + REQUEST_ID + PAYLOAD)
TYPE:       u8 MessageType discriminant
REQUEST_ID: 16 bytes correlation token
PAYLOAD:    msgpack-encoded dict (HashMap<String, rmpv::Value>)
```

Max payload: 4 MB.

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Crate root, re-exports IpcServer, IpcServerConfig, default_socket_path |
| `src/server.rs` | `IpcServer` lifecycle (new, start, stop), Unix socket bind, accept loop, `default_socket_path()` |
| `src/wire.rs` | Frame encode/decode, `MessageType` enum (~70 variants), async read/write helpers |
| `src/connection.rs` | Per-client connection handler, subscription state (SubTopic), reader/writer task split |
| `src/dispatch.rs` | Maps `MessageType` to `Daemon` trait method calls, payload extraction/construction |

## Key Types

- **`IpcServer`** -- owns the Unix listener, daemon ref, and event broadcast. `start()` spawns accept loop, `stop()` cleans up socket file.
- **`IpcServerConfig`** -- `socket_path: PathBuf`, `event_capacity: usize` (default 256).
- **`MessageType`** -- `#[repr(u8)]` enum. Ranges: Keepalive (0x01/0x80), Query (0x10-0x1F), Command (0x20-0x2F), Subscription (0x30-0x3F), Extended (0x40-0x4F), Terminal (0x50-0x5F), Datalink (0x60-0x6F), Boundary (0x70-0x7F), Response (0x80-0x8F), Event (0xC0-0xFF). Methods: `is_request()`, `is_response()`, `is_event()`.
- **`Frame`** -- decoded frame: msg_type, request_id, payload dict.
- **`SubTopic`** -- Devices, Messages, Activity, Links.

## Socket Path Resolution

`default_socket_path()` checks in order:
1. `STYRENED_SOCKET` env var
2. `$XDG_RUNTIME_DIR/styrened/control.sock`
3. `~/.local/run/styrened/control.sock`

Socket permissions set to 0o600 (owner-only).

## Connection Architecture

Each client gets:
- **Reader task** (inline) -- reads frames, dispatches via `dispatch::dispatch()`, sends response bytes to writer channel
- **Writer task** (spawned) -- writes response frames and pushes subscription events from broadcast channel

Subscriptions are per-connection (`HashSet<SubTopic>` behind `Arc<Mutex<>>`). Events with zero request_id are pushed only to subscribed clients.

## Dispatch Coverage

`dispatch.rs` handles most MessageType variants. Not yet implemented:
- Page browser (delegated to Python TUI)
- Remote terminal (P3)
- Datalink management (P3)
- Adapter provisioning
- Attachment storage

Stubs return sensible defaults (empty arrays, false flags) rather than errors where possible.

## Test Commands

```bash
cargo test -p styrene-ipc-server
```

## Gotchas

- Python clients may send empty bytes `b""` instead of `b"\x80"` (msgpack empty map) as payload. The parser handles both.
- The `dispatch` function does manual msgpack dict construction with `rmpv::Value` -- no serde derives. This is intentional for wire compat with Python's dict-based protocol.
- `validate_hash()` in dispatch.rs requires 16-64 hex chars. Shorter hashes will be rejected.
- The server removes stale socket files on start and on Drop.
- `DaemonEvent` matching in `connection.rs` uses a wildcard arm for forward compat with new event variants.

## Status

Functional. Core query/command dispatch works. The Python TUI can connect and operate. Terminal sessions, datalink, and page browser are stubbed. Event push for Device, Message, Terminal, Tunnel, and Link events is wired.
