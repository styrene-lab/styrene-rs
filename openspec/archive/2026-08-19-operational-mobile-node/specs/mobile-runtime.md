# Mobile Runtime Delta

## ADDED Requirements

### Requirement: Transport-backed mobile boot exposes its delivery destination

When mobile boot creates an LXMF delivery destination, the same destination hash must be stored in the identity service and exposed to an embedding host.

#### Scenario: Hub-configured boot reports delivery hash
Given a mobile configuration contains a hub TCP address
When the embedded node finishes booting
Then its delivery hash is a nonzero 16-byte lowercase hexadecimal value
And its identity service reports the same value
And outbound callers can use the value in invitations and replies

#### Scenario: Offline boot does not invent a destination
Given a mobile configuration has no transport hub or interface
When the embedded node finishes booting
Then it remains available for local data access
And it reports no routable delivery destination
And it reports that transport is disconnected

### Requirement: Mobile announces carry the configured display name

A valid configured display name must be normalized through the shared announce-name rules, stored in identity state, and encoded as LXMF delivery announce application data.

#### Scenario: Valid display name is announced
Given the host configures a display name with surrounding whitespace
When the mobile transport and identity service are created
Then identity state contains the normalized name
And outgoing delivery announces contain the normalized name in standard application data

#### Scenario: Invalid display name is omitted
Given the host configures an empty or control-character display name
When the mobile node boots
Then identity state has no display name
And delivery announces omit display-name application data

### Requirement: Embedded nodes run operational service workers

Mobile boot must subscribe the normal inbound, announce, and link workers before returning the node to the host.

#### Scenario: Inbound LXMF reaches messaging events
Given a booted mobile node with an operational transport
When the transport emits an inbound LXMF message for its delivery destination
Then the inbound worker validates and persists the message
And the event service emits the corresponding message event

#### Scenario: Peer announce reaches discovery
Given a booted mobile node with an operational transport
When the transport emits a valid peer announce
Then the announce worker updates discovery state
And device subscribers receive the peer event

#### Scenario: Link lifecycle reaches subscribers
Given a booted mobile node with an operational transport
When the transport emits a link lifecycle event
Then link subscribers receive the corresponding typed link event

### Requirement: Mobile node lifecycle is explicitly managed

The embedded node must own the worker tasks it starts and provide a shutdown operation that stops workers and asks the transport to shut down.

#### Scenario: Explicit shutdown stops runtime work
Given a booted mobile node
When the host invokes shutdown
Then all retained worker tasks are aborted
And transport shutdown is invoked exactly once
And the operation returns after shutdown dispatch completes

#### Scenario: Drop does not leave retained workers running
Given a booted mobile node whose host does not call explicit shutdown
When the node is dropped
Then every worker handle retained by the node is aborted
