# reticulum-operations - Baseline

### Requirement: Runtime observations are authoritative

The daemon must report actual identity, destination, initialization, interface, path, link, and transport observations with source, observation time, connection generation, and freshness.

#### Scenario: Runtime interface is active
Given a configured interface has started and accepted traffic
When an operator requests runtime status
Then the response identifies the actual interface, endpoint, mode, state, peers, and counters
And it does not synthesize counters by dividing aggregate totals

#### Scenario: Observation becomes stale
Given an interface, route, or link observation exceeds its freshness threshold
When it is displayed or queried
Then it is marked stale with its last observation time
And cached presence alone does not make it current

### Requirement: Native request paths interoperate

Destinations must support registered request paths with access policy, link identity, correlated client requests, packet or resource responses, progress, timeout, and maximum-response enforcement.

#### Scenario: Request receives a packet response
Given a link is established to a destination with an allowed request path
When a client submits a request within packet limits
Then the matching response completes the request receipt
And callbacks or events identify the path, request ID, response, and RTT

#### Scenario: Request receives a resource response
Given a response exceeds the link packet capacity
When the request handler returns the response
Then the response transfers as a resource with progress and integrity verification
And the request completes only after resource proof succeeds

#### Scenario: Request is denied
Given the requester does not satisfy the request path access policy
When the requester submits the request
Then the server records a correlated authorization outcome without invoking the handler
And the remote receipt reaches its bounded timeout without response data

### Requirement: Path discovery and routing recover correctly

Path requests, route records, forwarding, expiry, loss, and rediscovery must preserve Reticulum semantics across multiple interfaces and hops.

#### Scenario: Three-node route is discovered
Given node A reaches node C only through transport node B
When A requests a path to C
Then A records a route through B with the correct hop count and interface
And A can deliver a proved packet to C

#### Scenario: Route is lost and rediscovered
Given A has an active route to C through B
And the route becomes unavailable
When A retries delivery after route expiry
Then stale route state is not used as current
And a new path request can establish a valid replacement route

### Requirement: Link lifecycle is race free and observable

Outbound link state must exist before a proof can arrive, remain bound to the proving interface, and expose establishment, identification, RTT, activity, teardown, and timeout outcomes.

#### Scenario: Fast proof arrives
Given a peer can answer a link request immediately
When the outbound link request is sent
Then the pending link is registered before the proof can be processed
And the link reaches active state without losing the proof

#### Scenario: Proof arrives on another interface
Given a pending link is bound to one proving interface
When a matching proof arrives on another interface
Then the proof is rejected for that link
And the original pending state remains uncorrupted

### Requirement: Reliable channels and resources complete under loss

Channel retries and resource state machines must be integrated into the transport loop and terminate successfully or explicitly under bounded loss.

#### Scenario: Channel proof is lost
Given a channel message is awaiting proof
When its retry deadline expires
Then the transport loop schedules the protocol retry
And the message eventually succeeds or reaches a bounded terminal failure

#### Scenario: Resource transfer is cancelled
Given a resource transfer is active
When either endpoint cancels it
Then both endpoints release transfer state
And progress terminates with a cancellation outcome rather than success
