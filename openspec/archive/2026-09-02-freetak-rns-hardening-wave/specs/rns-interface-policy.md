# RNS Interface Policy - Delta Spec

## ADDED Requirements

### Requirement: Passive nodes do not accumulate undrainable announce work

An accepted announce may enter the retransmission queue only when transport forwarding is enabled or the local/shared-instance exception applies, and never for a path response; other accepted announces needed for path persistence must remain in a bounded cache.

#### Scenario: Passive node receives distinct announces
Given transport forwarding is disabled and ingress is not a local/shared-instance exception
When the node accepts distinct ordinary announces
Then its retransmission queue does not grow
And the newest accepted packets remain available through the bounded persistence cache

#### Scenario: Transport node receives an ordinary announce
Given transport forwarding is enabled and the announce is not rate-blocked or a path response
When the node accepts the announce
Then it enters the bounded retransmission lifecycle
And existing retransmission behavior remains enabled

### Requirement: Internal-interface announce policy follows RNS 1.5

Per-interface `announces_from_internal` and `announces_to_internal` settings must apply the covered announce decision rows introduced in Reticulum 1.5.0 and retained by the pinned 1.5.1 authority, with permissive `announces_from_internal` default, no implicit `announces_to_internal` override, and consistent startup, child-interface, and hot-apply propagation.

#### Scenario: Outgoing interface rejects announces learned internally
Given a non-local announce has an internal-mode next hop
And the candidate outgoing interface explicitly disables announces from internal
When announce broadcast policy is evaluated
Then the announce is not sent on that interface
And an absent setting would preserve the permissive default

#### Scenario: Boundary announce crosses to an internal interface only by override
Given a non-local announce has a boundary-mode next hop
When an internal outgoing interface evaluates the announce
Then it blocks the announce unless the next hop explicitly permits announces to internal
And local-destination announces remain allowed

### Requirement: The RNode protocol engine uses a low-level bearer-neutral byte attempt

`styrene-rns` must define only a one-attempt ordered-byte open, read, write, and idempotent close trait consumed by one shared RNode protocol engine. Existing mobile plans own every platform implementation, reconnect policy, permission flow, application lifecycle, mobile bridge, and physical acceptance gate.

#### Scenario: Cancellation interrupts bearer I/O
Given a fake ordered-byte attempt is blocked in open, read, or write
When that single low-level operation is cancelled
Then the blocked operation may be abandoned and close remains callable and idempotent
And the protocol engine does not create a reconnect loop or platform lifecycle

#### Scenario: Different bearers carry the same protocol bytes
Given two fake attempt backends report the same ordered input bytes and safe write capacity
When each backend drives the shared RNode protocol engine after startup validation
Then both produce equivalent Reticulum packets and KISS output bytes
And no application payload is written before startup policy permits it
