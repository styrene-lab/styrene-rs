# Live echo peer suite

`styrene-e2e` carries an opt-in suite that drives a real node against a
deployed `styrened` echo peer. It proves the low-level message operations over
a real network: link establishment, packet and resource delivery, receipts,
and reconnection.

## Running it

```sh
STYRENE_LIVE_PEER=<host:port> \
STYRENE_LIVE_PEER_DESTINATION=<32 hex chars of the peer's lxmf.delivery hash> \
just test-live-peer
```

The peer must run `auto_reply.mode = "echo"` and a daemon that honours link
MTU signalling. It does not need to announce on its own: the probe announces
itself and repeats a path request until the peer answers. Without the two
variables the suite is skipped.

Evidence is written to `STYRENE_LIVE_EVIDENCE`, or to
`crates/tests/styrene-e2e/target/live-peer/evidence.json`, as one JSON document
with the probe identity, the peer, and every stage with its elapsed time and
message ids.

## Stages

1. `interface.connected`
2. `probe.announced`
3. `peer.resolved`
4. `packet.sent`, `packet.echoed`, `packet.receipt`
5. `resource.sent`, `resource.echoed`, `resource.receipt`
6. `interface.dropped`, `interface.reconnected`, `path.after_drop`, `path.moved`
7. `after-reconnect.sent`, `after-reconnect.echoed`, `after-reconnect.receipt`

An echo is matched by the inbound message's `styrene_echo.request_id` field
equalling the sent message id, and its body and `[auto-reply]` title are
checked.

## Reading the peer

The peer's journal tags each message with `[messaging-flow]` stages
(`durable_insert`, `path_request_completed`, `identity_resolved`,
`link_send_started`, `link_delivery_completed`) and `[worker] auto-reply sent`,
keyed by the same message ids the evidence records. Its RPC on the loopback
port answers `status`, `list_interfaces`, `list_peers`, and `list_messages`
over `POST /rpc` as a MessagePack frame with a four-byte big-endian length
prefix; `GET /healthz` and `GET /api/status` are plain JSON.
