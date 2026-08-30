# Mobile Backend Runtime - Delta Spec

## ADDED Requirements

### Requirement: Composed offline runtime is distinct from stopped transport

The mobile backend must expose runtime readiness independently from interface
connectivity and shutdown state.

#### Scenario: Node boots without an operational interface
Given identity, storage, services, and retained workers compose successfully
When the node has no listening, connected, or active interface
Then the snapshot reports the runtime ready offline and transport not connected
And it does not report shutdown, failure, or a fabricated bearer

#### Scenario: Node shuts down
Given a composed mobile node is ready offline or connected
When explicit shutdown completes
Then the snapshot or terminal outcome reports the runtime stopped
And retained workers and listeners are no longer active

### Requirement: Mobile observations carry current generation and typed failure

Capability, interface, session, boot, and storage observations must identify the
generation that produced them and use typed bounded failure reasons.

#### Scenario: Interface reconnects
Given an interface observation belongs to an earlier session generation
When a reconnect creates a newer generation
Then current interface and capability snapshots carry the newer generation
And the older observation cannot be mistaken for current state

#### Scenario: Boot fails after partial composition
Given a configured fault occurs during a named boot stage
When mobile boot returns failure
Then the failure identifies the bounded stage, code, and retryability
And all resources created by the failed attempt are released

### Requirement: Forced termination has explicit recovery evidence

The backend must recover committed identity, messages, contacts, drafts, unread
state, conversation shells, propagation selection, and attempt history after
process termination without claiming that incomplete network work resumed.

#### Scenario: Process terminates after durable commit
Given an isolated mobile process commits representative P0 state
When the runner terminates that process and boots a replacement on the same storage
Then every committed record is restored exactly once with stable identity
And storage status reports a successful open and recovery outcome

#### Scenario: Process terminates during incomplete work
Given an isolated mobile process has an uncommitted or in-flight operation
When the process terminates before its documented commit point
Then restart reports the operation's persisted terminal or interrupted state explicitly
And it does not fabricate completion, duplicate a canonical record, or resume unsupported in-memory correlation
