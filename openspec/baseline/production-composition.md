# production-composition - Baseline

### Requirement: Production capability claims match active composition

Each production runtime must prove that advertised Reticulum, LXMF, propagation, receipt, and NomadNet capabilities have their required active components.

#### Scenario: Production daemon starts
Given a production daemon configuration enables messaging, propagation, and pages
When startup completes
Then the required destinations, handlers, receipt workers, retry schedulers, and event bridges are registered
And startup reports their actual capabilities

#### Scenario: Test node has an extra handler
Given a test node registers a handler that a production runtime does not expose
When evidence from that test is classified
Then the evidence is marked internal or test-only
And it cannot satisfy the production capability claim

#### Scenario: Interoperability runner starts a Rust node
Given a live parity scenario requires a production claim
When the external runner starts the Rust endpoint
Then it launches a shipped artifact through its public command-line interface
And it does not call internal runtime composition functions

### Requirement: Capabilities are negotiated from active services

IPC capabilities must be derived from successfully initialized services and current authorization rather than hard-coded by clients.

#### Scenario: Optional service fails to initialize
Given a configured optional protocol service cannot start
When the daemon publishes capabilities
Then the failed service capability is absent or degraded with a reason
And operator controls cannot authorize work using stale capability state

#### Scenario: Client reconnects to another daemon
Given a client reconnects with a new connection generation
When capability negotiation completes
Then controls use only capabilities from the new generation
And in-flight work from the previous generation cannot mutate current state

### Requirement: Protocol state emits correlated observations

Requests, links, resources, messages, propagation jobs, and page requests must emit typed observations carrying source, time, generation, correlation, and terminal outcome.

#### Scenario: Page request spans protocol layers
Given a page request triggers path discovery, link creation, and a resource response
When events are consumed by an operator client
Then each event shares the browse operation correlation
And the client can construct stages without inferring completion from unrelated snapshots
