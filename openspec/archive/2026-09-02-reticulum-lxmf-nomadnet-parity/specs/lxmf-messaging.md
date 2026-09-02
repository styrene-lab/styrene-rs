# LXMF Messaging - Delta Spec

## ADDED Requirements

### Requirement: Delivery methods are authoritative

The daemon must honor Direct, Opportunistic, Propagated, and Paper delivery requests according to LXMF constraints and report any protocol-required fallback.

#### Scenario: Opportunistic message fits a packet
Given an outbound message fits the opportunistic packet limit
When the caller selects Opportunistic delivery
Then the daemon sends the opportunistic LXMF representation
And the lifecycle records Opportunistic as the actual method

#### Scenario: Opportunistic message is oversized
Given an outbound message exceeds the opportunistic packet limit
When the caller selects Opportunistic delivery
Then the daemon applies the defined LXMF fallback or rejects the request explicitly
And the actual method and reason are recorded

#### Scenario: Propagated delivery is unavailable
Given no compatible propagation node is configured
When the caller selects Propagated delivery
Then the send fails or remains unavailable with a propagation-specific reason
And the daemon does not silently send it as Direct

### Requirement: Message lifecycle follows verified protocol outcomes

Outbound messages must transition through queued, sending, sent, delivered, failed, or cancelled states from correlated transport, resource, receipt, and propagation evidence.

#### Scenario: Packet-backed Direct message is delivered
Given a Direct outbound message was dispatched as a packet
And transport accepted the packet for sending
When an authenticated RNS delivery receipt exactly matching the tracked packet is received
Then exactly that message transitions from Sent to Delivered
And its receipt, method, timestamps, and correlation identifiers are retained

#### Scenario: Resource-backed Direct message is delivered
Given a Direct outbound message was dispatched as a resource
And transport accepted the resource for sending
When verified completion of the exact tracked outbound resource is observed
Then exactly that message transitions from Sent to Delivered
And packet receipt evidence cannot substitute for resource completion

#### Scenario: Delivery evidence is not correlated
Given an outbound message is Sent
When a forged, unknown, cross-message, or representation-mismatched receipt or resource event is observed
Then the message remains Sent
And the evidence is not retained for later delivery

#### Scenario: Resource-backed message fails
Given an LXMF message is transferring as a resource
When the resource reaches a terminal integrity or timeout failure
Then the message transitions to Failed
And the resource failure is not presented as successful LXMF delivery

#### Scenario: Failed message is retried
Given a message has a retryable terminal failure
When the operator requests Retry
Then the daemon creates or updates one correlated retry attempt
And retry count, actual method, and terminal result are authoritative

### Requirement: Inbound authenticity and content fidelity are preserved

Inbound processing must preserve LXMF timestamp and field semantics and expose signature and stamp state as verified, invalid, unknown, or not applicable.

#### Scenario: Sender identity is unknown
Given an inbound signed message cannot yet be associated with a sender identity
When it is stored and presented
Then its authenticity state is Unknown rather than Verified
And later identity resolution can trigger verification without replacing message content

#### Scenario: Binary-compatible fields are received
Given a valid LXMF message contains non-text field values or fractional timestamp data
When it is decoded and persisted
Then the protocol values remain representable without lossy empty-string conversion
And rendering concerns do not alter the canonical record

### Requirement: Conversation operations are complete and persistent

Conversation history, unread state, pagination, search, contacts, pinning, muting, deletion, and drafts must use persistent daemon state and deterministic ordering.

#### Scenario: Older history is loaded
Given a conversation contains more messages than one page
When the operator loads the next history cursor
Then older messages append without duplication or omission
And live events remain correctly ordered with the loaded history

#### Scenario: Conversation is opened
Given a conversation contains unread messages
When the operator opens and marks the conversation read
Then the daemon persists the read state
And unread summaries update without a client-only override

### Requirement: Attachments use verified resources

Attachments must support bounded upload, download, progress, cancellation, storage, integrity verification, and explicit unsupported behavior.

#### Scenario: Attachment download completes
Given a received message references an available attachment resource
When the operator downloads it
Then progress and size are reported from the resource transfer
And content is exposed only after integrity verification succeeds
