# LXMF Propagation - Delta Spec

## ADDED Requirements

### Requirement: Standard LXMF propagation endpoints interoperate

Propagation nodes must expose the standard LXMF propagation destination, announcement metadata, request paths, transient IDs, and encrypted payload forms expected by Python LXMF.

#### Scenario: Python discovers a Rust propagation node
Given a Rust node is configured for LXMF propagation
When it announces its propagation destination
Then Python LXMF recognizes the node and its metadata
And the node can be selected without a Styrene-specific capability

#### Scenario: Active propagation discovery remains available
Given a Rust propagation node remains active after its startup announce
When its configured propagation announce interval elapses
Then it dispatches fresh active `lxmf.propagation` metadata
And discovery does not depend on restarting the node

#### Scenario: Operator requests an announce
Given an authorized operator requests a network announce on a propagation hub
When the local transport accepts the operation
Then both delivery and standard propagation announces are dispatched
And the operation does not claim remote reception

#### Scenario: Python offers messages to Rust
Given Python LXMF has propagation messages for a Rust node
When it performs the standard offer and transfer exchange
Then Rust validates and persists accepted transient messages
And duplicate offers do not duplicate stored payloads

### Requirement: Propagation retrieval is authenticated and bounded

Clients must identify over a link before retrieving messages, and limits, stamps, expiry, and authorization must be enforced without exposing payloads in inventory APIs.

#### Scenario: Identified client retrieves queued messages
Given a client has queued propagation messages
And the client identifies with the matching identity
When it requests messages from the propagation node
Then only authorized messages are transferred
And successful retrieval updates or removes queue state according to protocol semantics

#### Scenario: Unidentified client requests messages
Given a link has not identified an authorized identity
When it requests queued propagation messages
Then retrieval is denied
And queue contents and recipient metadata are not disclosed

### Requirement: Propagation synchronization and policy are authoritative

The daemon must report configured peers, selection, offers, fetches, downloads, sync progress, capacity, expiry, failures, and stamps from actual propagation state.

#### Scenario: Peer synchronization is active
Given a configured propagation peer is exchanging inventory
When propagation status is queried
Then the response reports the peer, stage, progress, timestamps, and correlation identifiers
And the operator view does not infer synchronization from queue size changes

#### Scenario: Capacity policy rejects a message
Given the propagation store has reached its configured capacity
When a new message cannot be admitted under policy
Then the message receives an explicit capacity failure
And existing queue state follows the configured eviction or rejection policy

### Requirement: Offline delivery survives restart

Propagation queue, transient identifiers, attempts, expiry, and synchronization checkpoints must persist across daemon restart.

#### Scenario: Recipient returns after restart
Given a message was queued while its recipient was offline
And the propagation node restarts
When the recipient reconnects and fetches messages
Then the original message is delivered once
And its persisted lifecycle reaches the correct terminal state
