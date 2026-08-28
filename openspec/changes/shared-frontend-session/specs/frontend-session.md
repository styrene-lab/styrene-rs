# Frontend Session - Delta Spec

## ADDED Requirements

### Requirement: Frontends use one typed daemon client

Interactive frontends must invoke daemon commands, queries, and subscriptions
through a reusable typed client instead of parsing raw wire maps independently.

#### Scenario: Two frontends query the same daemon
Given Ratatui and Dioxus support the same daemon operation
When each frontend submits that operation through the shared client
Then each receives the same typed result contract
And neither frontend parses the operation's raw wire payload

#### Scenario: Daemon reports an unsupported operation
Given a daemon capability marks an operation unsupported
When a frontend evaluates the related action
Then the shared client preserves the typed unsupported outcome
And the frontend does not substitute a different protocol operation

### Requirement: Session profiles preserve explicit lifecycle ownership

The frontend session boundary must expose explicit Live, Embedded, and Fixture
profiles without implicit fallback between them.

#### Scenario: Live connection fails
Given a frontend selected a Live session
When the configured daemon endpoint cannot be opened or negotiated
Then the session returns a recoverable typed connection failure
And does not start an Embedded runtime

#### Scenario: Embedded session closes
Given a frontend started an Embedded session
When the frontend closes that session
Then the session shuts down its owned daemon and interfaces
And releases its declared temporary resources

#### Scenario: Fixture session starts
Given a frontend selected a Fixture session
When the fixture is loaded
Then the session exposes the same frontend operation surface supported by that fixture
And opens no daemon process or external network interface

### Requirement: Requests and events are bounded and generation-safe

The shared client must correlate concurrent requests, fan out subscriptions,
enforce bounds and deadlines, and reject state from prior connection generations.

#### Scenario: Long request overlaps a status query
Given one client request remains in flight
When the frontend submits a status query
Then the status query does not wait for a global request mutex
And each response reaches its matching correlation identifier

#### Scenario: Connection changes before a response
Given an in-flight request belongs to an earlier connection generation
When the client reconnects before the response arrives
Then the old response cannot update the current session
And the request receives an explicit disconnected or cancelled outcome

#### Scenario: Request capacity is exhausted
Given the outbound or in-flight request limit is reached
When another request is submitted
Then the client returns a typed overload outcome
And does not allocate an unbounded request queue

### Requirement: Embedded and remote sessions preserve daemon semantics

An operation must retain its daemon-defined capability, lifecycle, correlation,
and evidence semantics across remote IPC and in-process Embedded sessions.

#### Scenario: Message operation uses two session types
Given a message operation is supported by both Live and Embedded sessions
When each session completes that operation
Then both return the same typed lifecycle and correlation fields
And neither session fabricates presentation-only success
