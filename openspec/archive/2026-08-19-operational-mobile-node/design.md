# Operational Mobile Node Design

## Boot Composition

`MobileNode::boot` continues to own platform paths, identity loading, SQLite, and optional hub TCP construction. Hub-backed construction additionally retains the delivery destination hash and passes normalized display-name application data to `TokioTransportAdapter`, whose existing announce implementation uses that data whenever callers announce without an explicit payload.

The final service composition is factored into a private constructor accepting an identity, `Arc<dyn MeshTransport>`, optional delivery hash, display name, and paths. Production boot and deterministic module tests share this constructor. It wires the signer, delivery hash, normalized identity display name, propagation hub, facade, and workers exactly once.

Offline boot keeps `NullTransport`, no delivery hash, and no display-name announce payload. Workers may subscribe to its closed/no-op channels and terminate naturally; the node remains useful for local conversation storage.

## Worker Ownership

The inbound worker currently starts separate packet and resource tasks but returns only the packet `JoinHandle`. Its return value becomes an `InboundWorkerHandle` retaining both tasks with `abort` and `is_finished` operations. Existing daemon call sites may continue ignoring the returned handle, preserving detached daemon behavior.

`MobileWorkers` retains the inbound handle plus announce and link handles. `MobileNode::drop` aborts all retained tasks. `MobileNode::shutdown(self)` first aborts workers and then awaits `MeshTransport::shutdown`. A boolean guard in the worker owner makes abort idempotent internally, while consuming `self` makes explicit node shutdown a one-shot API.

The underlying adapter's shutdown remains transport-defined; this change guarantees dispatch and worker cancellation rather than redesigning each RNS interface's shutdown semantics.

## Host API

`MobileNode::delivery_hash()` returns the optional configured LXMF destination. `MobileNode::is_connected()` delegates to the transport. Existing `daemon()` remains the high-level messaging/event boundary used by embedded applications.

The UniFFI wrapper already owns the node in `Mutex<Option<_>>`. Its shutdown method takes the node once and executes asynchronous shutdown on the retained Tokio runtime. Repeated FFI shutdown calls become harmless no-ops.

## Display Names

`normalize_display_name` is applied once during boot. The normalized value is passed to `IdentityService::set_identity` and encoded by `encode_delivery_display_name_app_data` for the adapter. Invalid values produce neither identity state nor announce data.

## Testing

Tests first use the private composition constructor with `MockTransport` to assert delivery metadata, display-name normalization, three retained workers, link event forwarding, and shutdown dispatch. A public boot test with temporary directories verifies the offline contract. Existing worker and daemon tests guard normal non-mobile composition.

The change does not require real sockets for deterministic tests. Real two-node delivery remains covered by the `styrene-e2e` package and downstream 4con integration tests.
