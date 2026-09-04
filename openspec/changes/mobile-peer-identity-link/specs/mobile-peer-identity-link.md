# Mobile Peer Identity And Link - Delta Spec

## ADDED Requirements

### Requirement: A peer carries the identity that announced it

A `MobilePeer` must carry the hex address hash of the identity behind the
announced destination, so announces from one identity across several
destinations can be grouped. A peer record written before the identity was
projected must still project, with an empty identity hash.

#### Scenario: Announce is heard
Given a mobile node running its announce worker
When an announce for a delivery destination is received
Then the projected peer's identity hash is the announcing identity's address hash
And it differs from the peer's destination hash

#### Scenario: Legacy peer record
Given a peer persisted before the identity was recorded
When the peer snapshot is taken
Then the peer projects with an empty identity hash and no route

### Requirement: A peer describes the route its announce took

A `MobilePeer` must carry the hop count of the announce that produced it and
the kind of interface that announce arrived on, when the receiving interface
is still known to the transport.

#### Scenario: Announce over a TCP client interface
Given a transport whose interface set contains a TCP client interface
When an announce arrives on that interface after three hops
Then the projected peer reports three hops and the interface kind "tcp_client"

### Requirement: The link to a peer can be queried

`MobileNode::peer_link` must report whether a destination is reachable now,
and for a reachable destination the hops and interface kind of its path. A
destination with no path, and a destination hash that is not a valid address
hash, must both be reported as unreachable rather than as an error.

#### Scenario: Unknown destination
Given a mobile node with no path to a destination
When the link to that destination is queried
Then the link reports unreachable with no hops and no interface kind

#### Scenario: Malformed destination hash
Given a destination hash that is not valid hex of address-hash length
When the link to it is queried
Then the link reports unreachable rather than failing

#### Scenario: Known path
Given a transport with a path to a destination two hops away over a TCP client interface
When the link to that destination is queried
Then the link reports reachable, two hops, and the interface kind "tcp_client"
