# Live Echo Peer Suite

## Intent

Exercise the low-level message operations against a deployed `styrened` echo
peer over a real network, so that link establishment, packet and resource
delivery, receipts, and reconnection are proven against a running node rather
than only against loopback peers.

## Scope

An opt-in e2e suite in `styrene-e2e` that drives an in-process node against an
echo peer named by environment variables, records staged JSON evidence, and is
skipped unless the peer is configured. The first run exposed two daemon
defects, which this change corrects. Inbound LXMF fields were dropped by the
projection inserts. A link whose negotiated MTU differed from the planned
representation failed the send instead of recording what the link did.

It excludes the peer's own deployment, RNode radio evidence, and the mobile
application's own live tests, which build on this suite.

## Success criteria

- With the peer configured, the suite connects, announces, and resolves the peer by path request.
- It echoes a packet-sized and a resource-sized message with delivered receipts.
- It drops and reattaches the client interface, sees the path move to the live interface, and echoes again.
- Every stage lands in an evidence file with elapsed time and the correlating message ids.
- Inbound message records carry their decoded LXMF fields, so an echo response's request id is visible to clients.
- A send whose link chose a different representation than planned completes and the route records the representation the link used.
- Without the environment variables the suite is skipped, and ordinary validation is unchanged.
