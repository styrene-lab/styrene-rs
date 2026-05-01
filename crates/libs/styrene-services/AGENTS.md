# styrene-services

Domain service abstractions for the Styrene mesh daemon. Transport-independent, SQLite-backed services that compose into the daemon runtime. Each service owns a focused domain.

## Module map

| Module | Purpose | Status |
|--------|---------|--------|
| `conversations` | Message threading by peer, unread counts, pagination, pin/mute | Implemented |
| `node_store` | Persistent peer registry, announce ingestion, block/bookmark | Implemented |
| `protocol_registry` | Pluggable per-type inbound message handler dispatch | Implemented |
| `propagation` | Store-and-forward for offline peers | Planned |
| `file_transfer` | Chunked file delivery over links | Planned |
| `hub_connection` | Auto-connect to hub transport | Planned |

## Key types

### `ServiceError` (lib.rs)
Common error enum: `Storage(String)`, `NotFound(String)`, `InvalidArgument(String)`, `Database(rusqlite::Error)`.

### conversations
- **`ConversationStore`** -- SQLite-backed, `Mutex<Connection>`. Open with `::open(path)` or `::in_memory()` for tests.
- **`Conversation`** -- summary struct: peer_hash, display_name, message/unread counts, last_activity, pinned, muted.
- **`ConversationMessage`** -- individual message: id, peer_hash, outbound, content, timestamp, read, delivery_status.
- Key methods: `insert_message()`, `list_conversations()` (sorted by pinned then recency), `get_messages(peer, limit, before_timestamp)` (paginated), `mark_read()`, `set_pinned()`, `set_display_name()`, `delete_conversation()`.

### node_store
- **`NodeStore`** -- SQLite-backed peer registry. `::open(path)` or `::in_memory()`.
- **`Node`** -- identity_hash, display_name, name_source, first/last_seen, announce_count, signal_quality, device_type, blocked, bookmarked.
- Key methods: `accept_announce()` (upsert, increments announce_count), `get()`, `list(since_secs)`, `count()`, `set_blocked()`, `set_bookmarked()`, `remove()`.

### protocol_registry
- **`ProtocolHandler`** (async trait) -- implement `name()`, `protocol_types() -> Vec<String>`, `handle(&InboundMessage) -> HandleResult`.
- **`ProtocolRegistry`** -- register handlers, dispatch by LXMF `fields["protocol"]` string. Unmatched messages go to default handler.
- **`InboundMessage`** -- source_hash, protocol, content, fields (HashMap<String, serde_json::Value>), timestamp, message_id.
- **`HandleResult`** -- `Handled`, `Reply(String)`, `NotHandled`, `Error(String)`.

## Dependencies

- `rusqlite` 0.31 (bundled SQLite) -- conversations and node_store
- `async-trait` 0.1 -- ProtocolHandler trait
- `tokio` 1 (sync) -- Mutex for ProtocolRegistry
- `serde` + `serde_json` -- serialization for domain types

## Test commands

```bash
cargo test -p styrene-services              # all service tests
cargo test -p styrene-services conversations  # just conversation tests
cargo test -p styrene-services node_store     # just node store tests
cargo test -p styrene-services protocol_registry  # async dispatch tests (uses tokio)
```

## Gotchas

- `ConversationStore` and `NodeStore` use `std::sync::Mutex` (not tokio), so don't hold the lock across `.await` points.
- `ProtocolRegistry` uses `tokio::sync::Mutex` -- all its methods are async.
- `last_message_preview` truncates at 97 chars + "..." -- uses byte slicing on the String, could panic on multi-byte UTF-8 at the boundary.
- `list(since_secs)` computes cutoff from `SystemTime::now()` at call time -- not injectable for testing. Tests that use it need real-ish timestamps.
- SQLite tables are auto-created on first `open()`/`in_memory()` via internal `migrate()`.
- The crate is `std`-only (no `no_std` support). Depends on rusqlite with `bundled` feature, so it compiles its own SQLite.

## Status

Three services implemented with full test coverage. Three more planned (propagation, file_transfer, hub_connection). This crate is independent of the app/daemon layer by design -- it uses SQLite directly, not the daemon's store abstractions.
