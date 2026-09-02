# packet-admission-routing - Baseline

### Requirement: Packet admission is fail closed

The transport must reject structurally incomplete, zero-data, excessive-hop, or over-interface-MTU frames before truncation, queue insertion, deduplication, routing, or application delivery.

#### Scenario: Oversized frame arrives
Given an interface has a finite hardware MTU and receives a frame larger than that MTU plus allowed IFAC overhead
When the frame reaches packet admission
Then the frame is rejected without constructing a truncated packet
And no route, link, receipt, resource, or application state is changed

#### Scenario: Packet data is empty
Given a frame contains the complete header and destination fields but no data bytes
When the frame reaches packet admission
Then the frame is rejected as malformed
And the interface records one protocol violation

### Requirement: Received hops have one canonical meaning

Physical ingress must increment the wire hop count exactly once before transport behavior observes it, while explicitly local ingress must not add a network hop. Outbound and wire admission must enforce the canonical maximum without wrapping.

#### Scenario: Physical packet is received
Given a valid physical-interface frame carries wire hop count 7
When the transport admits and delivers the packet
Then routing and application observations report 8 received hops
And no later transport stage increments it again

#### Scenario: Maximum wire hop is exceeded
Given a frame carries a wire hop count of 128 or greater
When packet admission processes it
Then the frame is rejected before hop arithmetic
And the hop count cannot wrap or saturate into an accepted route

### Requirement: Path requests batch by destination

Valid concurrent path requests for one destination must share one bounded in-flight discovery operation while retaining every eligible requesting interface and preserving tag replay protection.

#### Scenario: Different requesters ask for one destination
Given two eligible interfaces send valid requests with different tags for the same unknown destination
When the first request has already started recursive discovery
Then only one recursive discovery request is emitted for that destination
And a matching announce is returned once to each eligible requester

#### Scenario: Batched request is ingress limited
Given discovery is already in flight for a destination
And another request for it is classified as ingress limited
When the request is admitted
Then it cannot add an unrestricted waiter or trigger another recursive request
And ingress and egress limits remain enforceable

### Requirement: Path discovery deadlines reflect the active medium

Discovery and link-proof deadlines must use positive runtime bitrates from active interfaces. For lowest online bitrate `b`, medium path grace is `2 * (500 * 8 / max(b, 5)) + 6` seconds and discovery uses the greater of configured path timeout and that grace. Extra link-proof grace is `(500 * 8) / outbound-interface bitrate` when positive and zero otherwise.

#### Scenario: A slow interface is online
Given the slowest online interface has a known positive bitrate
When recursive path discovery starts
Then its deadline is the greater of configured path timeout and `2 * (500 * 8 / max(bitrate, 5)) + 6` seconds
And an offline slower interface does not extend the deadline

#### Scenario: No bitrate is known
Given no online interface reports a positive bitrate
When a path or link-proof deadline is calculated
Then medium path grace and extra link-proof grace are zero and the finite configured base deadlines remain in force
And calculation does not panic, overflow, or divide by zero

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
