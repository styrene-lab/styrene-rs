# Live Echo Peer - Delta Spec

## ADDED Requirements

### Requirement: Inbound records keep their LXMF fields

An accepted inbound message's projection must persist the decoded LXMF fields,
with attachment bytes redacted, so clients can read correlation markers.

#### Scenario: Echo response received
Given an inbound message whose fields carry `styrene_echo.request_id`
When it is accepted and listed
Then the listed record's fields contain that request id

### Requirement: The link's representation is recorded, not rejected

When a link carries a payload as a packet or a resource contrary to the
planned representation, the send must complete and the outbound route must
record the representation the link used.

#### Scenario: Negotiated MTU carries a resource-sized body
Given a link whose confirmed MTU exceeds the default
When a body above the default packet MDU is sent
Then the link sends it as a packet
And the route's representation reads packet after completion

### Requirement: Live echo basics are provable on demand

With a peer address and destination configured, the e2e suite must prove
connection, announce, path resolution, packet and resource echo with delivered
receipts, and reconnection with the path moving to the live interface, and
must record staged evidence.

#### Scenario: Peer configured
Given `STYRENE_LIVE_PEER` and `STYRENE_LIVE_PEER_DESTINATION`
When the suite runs
Then every stage passes and the evidence file records each stage with its elapsed time

#### Scenario: Peer not configured
Given neither variable
When the suite runs
Then it is skipped without failing
