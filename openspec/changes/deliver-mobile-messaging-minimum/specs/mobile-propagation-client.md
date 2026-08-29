# Mobile Propagation Client - Delta Spec

## ADDED Requirements

### Requirement: Mobile propagation-node selection is explicit and persistent

The mobile application must store the selected standard `lxmf.propagation`
destination independently from the TCP transport endpoint and must validate
announce metadata before reporting the node as ready.

#### Scenario: User selects an announced propagation node
Given the directory contains a valid active `lxmf.propagation` announce
When the user selects that destination as the propagation node
Then the application persists its destination hash and decoded policy metadata
And it reports the node as selected for the current identity

#### Scenario: Application reconnects through the same TCP endpoint
Given the application has a persisted propagation-node selection
When the TCP session reconnects
Then the application retains the selected propagation destination
And it does not confuse the TCP interface endpoint with the propagation destination

#### Scenario: Selected node has no valid current metadata
Given the persisted propagation destination has no valid active announce metadata
When propagation readiness is evaluated
Then the application reports the node as unavailable or stale
And it does not claim synchronization is ready

### Requirement: Propagated upload preserves one outbound lifecycle

The mobile client must upload through the selected standard propagation node
with backend-enforced stamp and transfer policy and must preserve one canonical
message across retries.

#### Scenario: Selected node accepts upload
Given a valid text message requests Propagated delivery
And the selected propagation node is ready
When the propagation upload completes
Then the message records the node, transient identifier, attempt, and upload outcome
And a repeated upload of the same transient payload does not create another canonical message

#### Scenario: Upload fails before acknowledgement
Given a propagated upload is in progress
When the link closes or its deadline expires before acknowledgement
Then the message records a retryable propagation failure
And the persisted canonical message remains available for retry

### Requirement: Mobile synchronization is identified, bounded, and idempotent

The mobile client must synchronize through an identified link, allow a manual
request, schedule bounded automatic requests, durably persist retrieved messages,
and acknowledge only durable results.

#### Scenario: User requests synchronization
Given a selected propagation node is ready
When the user requests synchronization
Then the client identifies with the current mobile identity and requests its inventory
And the application exposes one bounded in-flight synchronization with progress

#### Scenario: Automatic synchronization trigger occurs
Given automatic synchronization is enabled and no synchronization is in flight
When the session first connects, reconnects, or receives an allowed foreground opportunity
Then the client schedules one synchronization under its cooldown and deadline policy
And overlapping triggers do not create concurrent synchronizations

#### Scenario: Retrieved message is persisted
Given the propagation node returns a valid encrypted message for the mobile identity
When the client decrypts and validates that message
Then it persists the message before acknowledging its transient identifier
And the conversation presents the message once

#### Scenario: Synchronization repeats after acknowledgement
Given a prior synchronization persisted and acknowledged all returned messages
When the client synchronizes again without new messages
Then the result reports zero new messages
And no existing conversation message is duplicated

#### Scenario: Retrieved message cannot be persisted
Given the propagation node returns a message that cannot be validated or stored
When synchronization handles that message
Then the client records a typed failure without acknowledging that transient identifier
And other valid messages follow the documented partial-failure policy

### Requirement: Mobile propagation controls are client-only

The minimum mobile product must expose selected-node status, upload evidence,
last synchronization, progress, and failure without exposing propagation-host or
peer-administration controls.

#### Scenario: Mobile Propagation view renders
Given the backend reports propagation-client capability
When the user opens propagation details
Then the application shows selection, readiness, last sync, progress, and failure state
And it does not offer local hosting, peering, capacity administration, or expiry administration
