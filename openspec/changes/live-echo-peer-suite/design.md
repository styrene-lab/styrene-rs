# Design

## Suite

`crates/tests/styrene-e2e/tests/live_echo_peer.rs` builds a `TestNode` with one
TCP client interface aimed at `STYRENE_LIVE_PEER` and targets the delivery
destination in `STYRENE_LIVE_PEER_DESTINATION`. The test is `#[ignore]` and
returns early when either variable is absent, so it never runs in ordinary
validation. Stages, each recorded with elapsed milliseconds:

1. interface connected
2. probe announced
3. peer resolved: the probe repeats a path request every four seconds until a
   path and identity exist, because the peer is not required to announce on
   its own
4. packet-sized echo: sent id, echoed id, round trip, delivered receipt
5. resource-sized echo: the same with a body above the default packet MDU
6. interface dropped, client reattached, and the path observed to move to the
   reattached interface after a fresh announce and path request
7. echo after reconnect

The echo is matched by the `styrene_echo.request_id` field of the inbound
message equalling the sent id, with the body and the auto-reply title checked.
Evidence is written to `STYRENE_LIVE_EVIDENCE` or
`target/live-peer/evidence.json` under the crate directory.

## Corrections

**Inbound fields.** Both canonical inbound inserts wrote NULL into the
projection's `fields` column while the canonical row kept the wire bytes.
Clients reading the projection never saw the fields. Both inserts now persist
the projection's fields, which the decoder already produces with attachment
bytes redacted.

**Representation reconciliation.** Messaging chose packet or resource from the
fixed default MDU, and the adapter returned "selected link representation
changed" when the link chose otherwise. A link that negotiated a larger MTU
carries a resource-sized body in one packet, which is correct. The adapter now
records the link's choice through the dispatch gate, and messaging updates the
outbound route's representation when it differs from the plan.

## Peer requirements found

The peer must run a daemon that honours link MTU signalling. A pre-parity
build echoed the requested MTU without clamping it, so the requester believed
the link carried 2 KB packets that the peer then dropped. The Raspberry Pi echo
peer was moved to a current build during this change.

## Follow-ups

- A bare `announce(None)` from the transport adapter goes out with empty app
  data and every receiver rejects it as non-canonical before the worker's
  canonical announce follows. The bare announce should carry the node's app
  data.
- The inbound announce worker logs the rejection at error level for that
  bare announce.
