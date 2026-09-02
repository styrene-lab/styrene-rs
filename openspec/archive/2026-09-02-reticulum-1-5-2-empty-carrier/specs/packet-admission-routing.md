# packet-admission-routing Delta

## ADDED Requirements

### Requirement: Empty carrier units are ignored before packet admission

Interfaces must treat a zero-byte carrier unit as no input. They must not deserialize it as an RNS packet, record a protocol violation, update byte counters, or mutate transport state.

#### Scenario: Empty UDP datagram arrives

Given an active UDP interface receives a zero-byte datagram
When carrier admission processes the datagram
Then no packet is emitted
And no malformed-frame or IFAC violation is recorded
And a subsequent valid datagram remains deliverable

#### Scenario: Empty HDLC frame is decoded

Given an active stream interface decodes an HDLC frame with no payload
When carrier admission processes the frame
Then no packet is emitted
And no malformed-frame or IFAC violation is recorded
And a subsequent non-empty malformed frame remains fail closed
