# Mobile Product Projection - Delta Spec

## ADDED Requirements

### Requirement: Conversation entry accepts canonical destinations

The mobile product must start one durable conversation from a discovered peer or
a manually supplied valid LXMF delivery destination through the backend-owned
conversation operation.

#### Scenario: User starts a conversation from People
Given People contains a canonical discovered delivery destination with no conversation
When the user requests a conversation with that peer
Then the backend persists one conversation shell for that destination
And Messages opens the shell without fabricating a message, route, or reachability state

#### Scenario: User enters a valid destination
Given the user supplies a valid LXMF delivery destination that is not in People
When the user confirms the destination
Then the backend validates it and persists one conversation shell
And repeated confirmation does not create another conversation

#### Scenario: User enters an invalid destination
Given the user supplies a malformed or unsupported destination
When the user confirms the destination
Then the application reports a bounded validation error
And no contact or conversation is persisted

### Requirement: Mobile projections preserve authoritative lifecycle evidence

The frontend projection must preserve distinct runtime phases and backend-owned
message, attempt, route, bearer, receipt, failure, and retry evidence without
reconstructing those facts from terminal labels.

#### Scenario: Runtime enters a recoverable degraded state
Given the embedded runtime remains composed but one current-generation operation is degraded
When the backend publishes the runtime snapshot
Then the mobile projection reports Degraded rather than Reconnecting or Failed
And it retains the typed reason and current generation

#### Scenario: Backend reports a terminal message failure
Given a canonical outbound message has a terminal typed failure
When the conversation projection renders that message
Then it reports the backend's retry eligibility and failure reason
And it does not offer Retry when the backend marks the failure non-retryable

#### Scenario: Delivery evidence arrives
Given a message attempt has route, bearer, propagation, and receipt observations
When the frontend applies the authoritative projection
Then each observation remains independently inspectable with its correlation
And no current bearer or upload state is presented as recipient delivery

### Requirement: Delivery method readiness is evaluated before submission

The composer must evaluate Direct and Propagated availability independently from
current backend capability and propagation-node readiness.

#### Scenario: No propagation node is ready
Given Direct is supported and no selected propagation node is ready
When the composer renders delivery methods
Then Direct remains available and Propagated is disabled with a recoverable reason
And the composer does not report an eligible Propagated draft as ready to send

#### Scenario: Propagation metadata becomes stale
Given Propagated was selected while its propagation node was ready
When the backend reports that the node metadata is stale
Then submission is disabled before dispatch
And the draft, destination, and selected method remain intact

### Requirement: Mobile status and history expose useful context

The mobile product must expose a concise operational summary and message history
using only available typed facts, including message direction and persisted time.

#### Scenario: Operational facts are available
Given the backend reports runtime, bearer, peer, unread, route, and propagation facts
When the user opens the mobile status summary
Then the application presents those facts without requiring navigation through every settings page
And unknown relay, path, or synchronization facts remain unknown rather than inferred

#### Scenario: Conversation has inbound and outbound messages
Given a conversation contains persisted inbound and outbound messages with timestamps
When the message history renders
Then each message exposes its direction and persisted time
And ordering follows backend-owned canonical chronology

#### Scenario: Peer details are inspected
Given a peer has destination, aspect, source, observation age, and announce count
When the user opens that peer's details
Then the available metadata is presented with bounded destination formatting
And freshness is not described as current reachability
