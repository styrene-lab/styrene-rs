# Link, Resource, And Security - Delta Spec

## ADDED Requirements

### Requirement: Link MTU discovery follows route capability

When enabled, link establishment must authenticate and negotiate the minimum supported MTU across participating interfaces; when disabled or unsupported, it must use the canonical base MTU without signaling an upgrade.

#### Scenario: Route MTUs differ
Given link MTU discovery is enabled across interfaces with different supported MTUs
When a link request and proof traverse the route
Then the signaled and confirmed MTU is clamped to the route minimum
And packet, channel, and resource payload limits use the confirmed MTU

#### Scenario: Discovery is disabled
Given link MTU discovery is disabled by configuration
When a link request is created
Then it omits MTU upgrade signaling
And the link uses the canonical base MTU

### Requirement: Link teardown terminates all resources

Closing a link must terminate every correlated incoming, pending outgoing, and active outgoing resource exactly once even when cancellation mutates resource collections.

#### Scenario: Multiple resources are active
Given a link owns multiple incoming and outgoing resources
When the link closes
Then every resource reaches one cancellation or failure outcome
And no resource or retry remains registered to the closed link

### Requirement: Resource windows accept the requested parts

Resource receive-part matching must begin after the consecutive completed height and cover the current requested window without an off-by-one omission.

#### Scenario: First missing part arrives out of order
Given a receiver has completed a consecutive prefix and requested the next window
When the first missing part in that window arrives after another part
Then it is matched to its canonical index and retained once
And completion advances only across truly consecutive received parts

### Requirement: Receipt callbacks are reentrant

Proof validation and receipt collection mutation must not hold collection synchronization while invoking an application callback.

#### Scenario: Receipt callback sends a packet
Given a valid proof concludes an outstanding receipt
When its delivery callback synchronously sends another packet
Then the send completes without deadlock
And the original receipt transitions exactly once

### Requirement: Token authentication is constant time

Token tags must be verified with constant-time comparison before decryption on every implementation path.

#### Scenario: Token tag is invalid
Given a token differs from a valid token only in its authentication tag
When token verification runs
Then verification fails before decryption through a constant-time comparison path
And no plaintext is returned

### Requirement: Policy rejection remains distinct

Valid traffic rejected by blackhole policy must remain distinguishable from malformed or cryptographically invalid traffic.

#### Scenario: Announced identity is blackholed
Given an announce is structurally valid and has a valid signature from a blackholed identity
When announce admission applies policy
Then the announce is dropped as blackholed
And it is not reported as a malformed packet or invalid signature
