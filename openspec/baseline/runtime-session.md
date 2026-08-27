# runtime-session - Baseline

### Requirement: Runtime profiles are explicit and isolated

The operator console must provide explicit Live, Embedded, and Fixture runtime profiles with visible identity, lifecycle, storage, network, and cleanup behavior.

#### Scenario: Live daemon is unavailable
Given the operator selected a Live profile
And the configured daemon socket is missing, stale, refused, incompatible, or unauthorized
When the console attempts to connect
Then it presents a recoverable connection failure
And does not start an embedded daemon

#### Scenario: Embedded runtime starts
Given the operator explicitly selected an Embedded profile
When the console starts the daemon
Then it displays the selected identity, storage, listeners, and persistence policy
And owns deterministic shutdown of the embedded runtime

#### Scenario: Fixture runtime starts
Given the operator selected a Fixture profile
When a fixture is loaded
Then every primary page can render from the fixture session
And no external network interface or daemon process is opened

### Requirement: Requests and events remain independently responsive

The console must correlate concurrent logical requests, fan out subscriptions independently, enforce deadlines, and reject stale responses from prior connection generations.

#### Scenario: Long request overlaps status refresh
Given a page or resource request is in flight
When the console requests current status
Then the status request does not wait for a global bridge mutex held by the long request
And each response is delivered to its matching correlation ID

#### Scenario: Connection changes during a request
Given a request belongs to an earlier connection generation
When the console reconnects before its response arrives
Then the old response cannot mutate current stores
And the request reaches an explicit disconnected or cancelled outcome

#### Scenario: Broker reaches capacity
Given the outbound or in-flight request limit is reached
When another request is submitted
Then the broker returns an overload outcome without unbounded allocation
And records a diagnostic activity event
