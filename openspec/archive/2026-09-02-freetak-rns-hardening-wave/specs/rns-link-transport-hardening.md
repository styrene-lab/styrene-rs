# RNS Link and Transport Hardening - Delta Spec

## ADDED Requirements

### Requirement: Link control traffic mutates state only after validation

Link identify, keepalive, close, and channel controls must satisfy exact framing, bounds, Link binding, authentication, and decryption before they update liveness, reactivate a Link, emit semantic state, or receive a proof. LinkRTT bytes, precision, parsing, and validation are outside this requirement and are supplied by the verified Beechat LinkRTT work.

#### Scenario: Hostile control traffic cannot revive a stale Link
Given an active Link became stale without subsequent valid inbound traffic
When it receives malformed identify, suffixed keepalive, invalid close, or corrupt channel ciphertext
Then its liveness and stale state remain unchanged
And no peer identity, channel delivery, or packet proof is emitted

#### Scenario: Valid identify follows repeated invalid identifies
Given a Link received a bounded sequence of malformed identify controls
When it receives an exactly framed identity proof bound to that Link
Then the verified identity is retained before the semantic event is published
And the Link's authenticated handshake identity remains unchanged

### Requirement: Established Link sends use each Link's bound interface

Data and channel helpers for established inbound and outbound Links must dispatch directly through each active Link's bound interface rather than looking up the ephemeral Link ID in the destination path table.

#### Scenario: Non-broadcast client sends on an established Link
Given a non-broadcast transport has an active Link bound to a registered interface
When a Link data or channel fan-out helper sends a payload
Then the packet is enqueued directly on that registered interface
And inactive Links receive no packet

### Requirement: Transport workers fail as one supervised runtime

Long-lived packet, Link, interface cleanup, cache cleanup, announce, and resource workers must be retained by a named supervisor that cancels and drains siblings when any worker panics or returns before shutdown.

#### Scenario: Worker exits before shutdown
Given the transport runtime is not shutting down and its named packet worker exits
When the supervisor observes the task completion
Then it records an attributable worker failure
And it cancels and drains every remaining supervised worker

#### Scenario: Workers observe normal shutdown
Given all transport workers are healthy
When the runtime cancellation token is cancelled
Then the supervisor drains every worker within the shutdown bound
And normal cancellation is not reported as an unexpected worker failure
