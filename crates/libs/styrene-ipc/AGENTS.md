# styrene-ipc

Interface boundary traits for the styrene daemon. Defines the IPC contract between `styrened` and its frontends (TUI, GUI, web bridge). Pure trait + type crate -- no implementation logic, no I/O.

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Crate root, re-exports |
| `src/error.rs` | `IpcError` enum (NotImplemented, Unavailable, Timeout, InvalidRequest, NotFound, Conflict, Internal, Transport) |
| `src/types.rs` | All boundary DTOs: DeviceInfo, IdentityInfo, MessageInfo, ConversationInfo, ContactInfo, SendChatRequest, DaemonEvent, TunnelInfo, PageContent, etc. |
| `src/traits/mod.rs` | Composite `Daemon` trait + blanket impl |
| `src/traits/messaging.rs` | `DaemonMessaging` -- chat, conversations, contacts |
| `src/traits/identity.rs` | `DaemonIdentity` -- local node identity, announce |
| `src/traits/status.rs` | `DaemonStatus` -- health, config, devices, interfaces, blocking |
| `src/traits/fleet.rs` | `DaemonFleet` -- remote device ops, exec, reboot, terminal sessions |
| `src/traits/events.rs` | `DaemonEvents` -- event subscriptions via broadcast channels |
| `src/traits/tunnel.rs` | `DaemonTunnel` -- VPN tunnel management, SA listing |
| `src/traits/pages.rs` | `DaemonPages` -- NomadNet page browsing |
| `src/stub.rs` | `StubDaemon` -- returns `IpcError::NotImplemented` for every method |

## Key Types and Traits

### Trait Hierarchy

Seven focused `#[async_trait]` traits compose into one:

```
Daemon = DaemonMessaging + DaemonIdentity + DaemonStatus
       + DaemonFleet + DaemonEvents + DaemonTunnel + DaemonPages
```

`Daemon` is auto-implemented via blanket impl. Primary handle type: `Arc<dyn Daemon>`.

### IpcError

- `NotImplemented { method }` -- stub-first development pattern
- `is_retryable()` returns true for Unavailable, Timeout, Transport
- All variants are `Clone + Serialize + Deserialize + PartialEq`

### StubDaemon

Zero-dependency starting point. Every method returns `Err(IpcError::NotImplemented { .. })`. Used to wire `styrened` before real implementations exist. Replace methods one at a time.

### Type Aliases

- `MessageId = String` (hex-encoded)
- `PeerHash = String` (hex-encoded destination hash)
- `SessionId = String` (terminal session)

### Event System

`DaemonEvent` enum: Message, Device, TerminalOutput, TerminalStateChange, TunnelStateChange, Link. Delivered via `tokio::sync::broadcast`.

## Test Commands

```bash
cargo test -p styrene-ipc
```

## Gotchas

- All DTO structs are `#[non_exhaustive]` -- always construct via `Default::default()` + field assignment, not struct literals.
- `DaemonEvent` and `MessageEventKind` are also `#[non_exhaustive]` -- match arms need wildcard.
- No feature flags -- everything is always compiled.
- Types mirror what the Python TUI consumes. Field names must match the Python side for serialization compat.

## Status

Stable. Full trait surface covering messaging, identity, status, fleet, events, tunnels, and pages. All types are defined. `StubDaemon` covers 100% of the trait surface.
