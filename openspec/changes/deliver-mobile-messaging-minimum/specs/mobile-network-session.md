# Mobile Network Session - Delta Spec

## ADDED Requirements

### Requirement: Mobile startup restores one persistent network session

The mobile application must restore one persisted identity and one validated TCP
client endpoint, start exactly one embedded node, and expose backend-confirmed
session state without requiring an RNode.

#### Scenario: Valid configuration is restored on cold launch
Given the application has a persisted identity and valid TCP client endpoint
When the user cold-launches the application
Then exactly one embedded node starts with the persisted identity
And the TCP client attempts the persisted endpoint without a manual node-start action

#### Scenario: TCP is available without an RNode
Given the embedded node has no attached RNode bearer
When its configured TCP client becomes connected
Then the application reports the TCP transport as connected
And messaging and discovery operations that require only TCP remain enabled

#### Scenario: Configured endpoint is invalid or refused
Given the persisted TCP endpoint is malformed, refused, or unreachable
When the embedded node attempts to connect
Then the application presents a recoverable typed connection failure
And it does not report the transport as connected

### Requirement: Mobile reconnect is bounded and generation-safe

The mobile session must reconnect after transient TCP interruption without
creating another embedded node or accepting stale state from an earlier
connection generation.

#### Scenario: Public TCP connection is interrupted
Given the mobile session is connected through its configured TCP client
When the TCP connection closes unexpectedly
Then the session enters a visible reconnecting or degraded state
And it retries with bounded delay while preserving the identity and endpoint

#### Scenario: Old refresh completes after reconnect
Given a directory or status request belongs to an earlier connection generation
When that request completes after the session reconnects
Then its result cannot replace current session state
And the current generation remains available for new operations

#### Scenario: Mobile state subscriber falls behind
Given the mobile state subscriber uses bounded event buffering
When the backend reports that state invalidations were dropped
Then the session requeries one authoritative current-generation snapshot
And it does not merge retained events into that snapshot

### Requirement: Mobile discovery reflects canonical announce state

The mobile directory must derive peers from canonical backend announce
observations and identify freshness without fabricating remote reachability.

#### Scenario: Canonical delivery announce arrives
Given the mobile session is connected
When the backend accepts a valid `lxmf.delivery` announce
Then the directory upserts one peer by destination hash
And it exposes the decoded display name and observation age

#### Scenario: The same peer announces repeatedly
Given a peer already exists in the current directory
When another valid announce for the same destination arrives
Then the existing peer receives the newer observation
And the directory does not add a duplicate peer

#### Scenario: User requests a local announce
Given the embedded node is ready
When the user requests an announce
Then the application reports local dispatch acceptance or a typed failure
And it does not claim that a remote peer received the announce

### Requirement: Network bearer state remains explicit

The mobile application must present TCP, Bluetooth RNode, and Android USB as
independent bearer states and must not infer one bearer from another.

#### Scenario: TCP and RNode states differ
Given the TCP client is connected and the approved RNode is disconnected
When the Network view renders
Then it reports TCP as connected and the RNode as disconnected
And it does not degrade the TCP session because the RNode is unavailable

#### Scenario: Platform lacks verified RNode support
Given the platform has no accepted physical RNode evidence
When capability information renders
Then the application identifies RNode support as unverified or unavailable
And unaffected TCP messaging remains operational
