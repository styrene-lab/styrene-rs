# Mobile Transports Delta

## ADDED Requirements

### Requirement: Mobile hosts configure direct TCP interfaces

`MobileConfig` must accept explicit TCP server and client profiles in addition to the existing optional hub client address.

#### Scenario: Mobile node listens on an ephemeral port
Given a mobile configuration contains a TCP server profile bound to `127.0.0.1:0`
When the node finishes booting
Then it reports one TCP listener with the actual bound address
And the reported port is nonzero
And the node exposes a routable LXMF delivery destination

#### Scenario: Mobile node connects to a peer listener
Given one mobile node reports a bound TCP server address
And a second mobile configuration contains that address as a TCP client profile
When the second node boots
Then its Reticulum transport connects to the first node
And neither node requires a propagation hub configuration

#### Scenario: Multiple interfaces share one node identity
Given a mobile configuration contains multiple distinct TCP profiles
When the node boots
Then every profile is attached to the same Reticulum transport
And all profiles advertise the same LXMF delivery destination

### Requirement: Interface configuration is validated before startup

Invalid or ambiguous mobile interface profiles must fail boot without partially creating a running node.

#### Scenario: Empty address is rejected
Given a TCP server or client profile has an empty or whitespace-only address
When mobile boot validates configuration
Then boot returns a validation error
And no transport workers are started

#### Scenario: Duplicate profile is rejected
Given the same normalized TCP profile appears more than once
When mobile boot validates configuration
Then boot returns a duplicate-profile error
And no transport workers are started

#### Scenario: Legacy hub duplicates explicit client
Given `hub_address` and an explicit TCP client identify the same normalized address
When mobile boot validates configuration
Then boot treats them as a duplicate rather than starting two client loops

### Requirement: Bound listeners are available to embedding hosts

The embedded Rust and UniFFI APIs must expose actual TCP listener addresses after successful boot.

#### Scenario: Host reads bound listener addresses
Given one or more TCP server profiles boot successfully
When the host requests listener addresses
Then it receives each actual socket address in configuration order
And client-only profiles do not appear in that list

#### Scenario: Client-only node has no listener addresses
Given a mobile node has only TCP client profiles
When the host requests listener addresses
Then it receives an empty list

### Requirement: Direct mobile peers exchange LXMF

Two mobile nodes connected by configured TCP interfaces must use the normal announce and LXMF service pipeline.

#### Scenario: Peers discover each other without a hub
Given one mobile node listens and another connects directly
When both nodes announce their delivery destinations
Then each node's discovery service records the other peer
And both delivery identities become resolvable

#### Scenario: Direct peers exchange messages bidirectionally
Given two directly connected mobile nodes have discovered one another
When each sends an LXMF message to the other's delivery destination
Then each receiver persists the expected content
And each inbound record identifies the sending identity

#### Scenario: Direct interfaces stop with the node
Given two directly connected mobile nodes
When either node performs explicit shutdown
Then its configured interface tasks stop
And its retained service workers stop
