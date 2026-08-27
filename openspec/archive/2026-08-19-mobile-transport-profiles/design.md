# Mobile Transport Profiles Design

## Configuration Model

`MobileInterfaceConfig` is a Rust enum with `TcpServer { bind_address }` and `TcpClient { remote_address }` variants. `MobileConfig` gains an ordered `interfaces` vector. The existing `hub_address`, when present, is normalized into an additional TCP client profile before validation because current UniFFI hosts construct it directly.

Addresses are trimmed and parsed as `SocketAddr`. Hostnames are intentionally excluded from this direct-local slice so validation and duplicate detection are deterministic before tasks start. Empty, malformed, and duplicate `(kind, address)` profiles fail boot. A legacy hub client and explicit client targeting the same address are duplicates.

## Transport Construction

Any nonempty validated profile set creates one RNS `Transport`, one `lxmf.delivery` destination, and one `TokioTransportAdapter`. Every profile is spawned through that transport's shared `InterfaceManager`:

- TCP server uses `TcpServer::new`; its watch receiver reports the actual bound socket.
- TCP client uses `TcpClient::new`.

Server profiles are started before clients. Boot waits up to five seconds for every server to report a bound address. Timeout or closed binding state aborts startup with context. Actual addresses are retained in configuration order and returned by `MobileNode::tcp_listen_addresses`.

No configured profiles continues to select `NullTransport` and has no delivery destination. `hub_delivery_hash` remains propagation configuration only and does not create a transport by itself.

## Interface Shutdown

`InterfaceManager` gains an additive `shutdown` operation that cancels its shared token and each local interface stop token. `TokioTransportAdapter::shutdown` invokes it before emitting `Disconnected`. This is used by full and mobile adapters and makes the managed `MobileNode::shutdown` from the preceding change actually stop TCP server/client loops.

The method is additive inside currently active RNS transport work; implementation must preserve all surrounding changes and tests.

## UniFFI

The binding exports a `MobileTcpInterface` enum with server and client variants and adds `interfaces` to its `MobileConfig` record. Conversion is mechanical into Rust `MobileInterfaceConfig`. `tcp_listen_addresses()` returns strings suitable for display or invitation metadata.

## Direct Two-Node Test

The integration test boots a server node with `127.0.0.1:0`, reads its actual address, then boots a client node targeting that address. Neither sets `hub_address` or `hub_delivery_hash`.

After a bounded settle period both nodes announce. The test waits for discovery, sends LXMF in both directions by delivery hash, and polls persisted inbound messages for expected source identity and content. All waits have explicit deadlines.

## Tradeoffs

- TCP listening requires one phone to be reachable on the local network, but removes centralized infrastructure and exercises production RNS links now.
- Restricting profiles to numeric socket addresses avoids hidden DNS and address-family behavior in the first mobile transport API.
- UDP is deferred until its IFAC gap is resolved; adding unauthenticated UDP merely for discovery would weaken the transport story.
