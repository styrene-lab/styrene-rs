# Mobile Messaging - Delta Spec

## ADDED Requirements

### Requirement: Mobile text conversations are durable

The mobile application must use backend-owned conversation, message, contact,
draft, and unread state for text messaging and must not substitute preview data
after a live session becomes ready.

#### Scenario: Live session has no conversations
Given the live backend reports an empty conversation list
When the Messages route finishes loading
Then the application shows a live empty state
And it does not display sample conversations as live records

#### Scenario: User composes a text message
Given the user has selected a valid LXMF delivery destination
And entered non-empty text
When the user submits the message
Then the backend persists one canonical message and returns its identifier
And the draft clears only after that acceptance while newer draft edits remain intact

#### Scenario: Application restarts with conversation history
Given the backend persisted messages, unread state, contacts, and a draft
When the application restarts with the same identity and storage
Then the conversation and message history are restored
And the unread, contact, and draft state matches the persisted records

#### Scenario: Inbound message arrives
Given the live session has a conversation with a remote destination
When the backend persists a valid inbound LXMF text message
Then the conversation shows that message once
And its unread state changes according to whether the conversation is active

### Requirement: Mobile message lifecycle uses authoritative evidence

The mobile application must preserve the backend's requested method, actual
method, attempt, correlation, and terminal evidence. It must not treat transport
acceptance or propagation upload as recipient delivery.

#### Scenario: Transport accepts an outbound packet
Given an outbound message has no authenticated delivery receipt
When the transport accepts its packet
Then the application reports a queued or sent state
And it does not report the message as delivered

#### Scenario: Propagation node accepts an outbound message
Given an outbound message uses the Propagated method
When the selected propagation node acknowledges upload
Then the application reports propagation upload separately from recipient delivery
And it retains the message correlation and selected propagation node

#### Scenario: Authenticated delivery evidence arrives
Given a tracked outbound message is awaiting delivery evidence
When the backend accepts evidence that exactly correlates to that message
Then the application reports the backend's delivered state
And duplicate or unrelated evidence cannot create another transition

#### Scenario: User retries a failed message
Given an outbound text message has a retryable terminal failure
When the user requests retry
Then the backend creates or resumes the documented correlated attempt
And the conversation does not create a duplicate canonical message

### Requirement: Delivery method selection is explicit

The mobile composer must submit an explicit supported LXMF delivery method and
must preserve a draft when the selected method is unavailable.

#### Scenario: User selects Direct delivery
Given the backend advertises Direct delivery as available
When the user submits a text message with Direct selected
Then the message records Direct as its requested method
And the application does not silently replace it with Propagated

#### Scenario: User selects Propagated without a node
Given no standard propagation node is selected and ready
When the user submits a text message with Propagated selected
Then the application rejects the submission with a recoverable reason
And it preserves the draft and selected destination
