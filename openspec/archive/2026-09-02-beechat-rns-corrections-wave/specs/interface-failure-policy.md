# Interface Failure Policy - Delta Spec

## ADDED Requirements

### Requirement: IPv4 UDP forwarding supports broadcast targets

An IPv4 UDP interface with a configured forwarding target must enable operating-system broadcast
permission before its first send. Receive-only and IPv6 sockets must not acquire unrelated behavior.

#### Scenario: IPv4 broadcast target is configured
Given an IPv4 UDP interface has a forwarding target that may be a broadcast address
When the interface binds its transmit socket
Then the socket has `SO_BROADCAST` enabled before transmission
And an allowed datagram can be submitted without a permission-denied broadcast failure

#### Scenario: UDP interface is receive only
Given a UDP interface has no forwarding target
When the interface binds its receive socket
Then no transmit capability is inferred from broadcast enablement
And receive behavior remains unchanged

### Requirement: Disconnected TCP traffic is bounded and non-replayable

A disconnected TCP client interface must not retain transport traffic for later replay or block
dispatch to healthy interfaces. Each established stream has a checked, monotonically increasing
`u64` connection epoch; carrier admission and queued traffic bind atomically to that epoch.
Dispatch outcomes must distinguish failed offline or stale-epoch egress from sent egress.

#### Scenario: Broadcast includes a disconnected client
Given one TCP client is disconnected and another eligible interface is healthy
When transport dispatches a broadcast packet
Then dispatch completes within the bounded interface enqueue deadline
And the healthy interface receives the packet while the disconnected interface is reported failed

#### Scenario: TCP client reconnects
Given packets were pending or submitted while a TCP client was disconnected
When the client reconnects
Then none of the pre-reconnect packets are transmitted from the interface backlog
And only packets submitted after the carrier is online can be sent

#### Scenario: Dispatch races disconnect
Given dispatch observes an online TCP client at connection epoch E
When the client disconnects before that item is committed to or consumed from the transmit queue
Then the item is rejected or discarded without transmission
And a later connection epoch cannot consume it

#### Scenario: Old writer races reconnect
Given a writer and queued item belong to connection epoch E
When a new stream is published online at connection epoch E plus one
Then the old writer cannot transmit the epoch-E item on the new stream
And a fresh item tagged E plus one remains eligible for the new stream

#### Scenario: Reconnect races stale queue drain
Given disconnect has published the TCP client offline for connection epoch E
When stale-queue drain overlaps publication of an online stream at epoch E plus one
Then the drain cannot remove a fresh item tagged E plus one
And the new stream cannot consume any stale item tagged E

#### Scenario: Connection epoch cannot wrap
Given the TCP client's connection epoch cannot be incremented without unsigned overflow
When another stream would be published online
Then the interface fails closed instead of reusing an earlier epoch
And no queued item is made eligible by epoch aliasing

#### Scenario: Direct send targets a disconnected client
Given a direct packet targets a disconnected TCP client interface
When transport dispatches the packet
Then dispatch reports no successful egress for that interface
And the interface queue remains bounded without waiting for reconnect
