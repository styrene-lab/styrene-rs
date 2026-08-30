# Mobile Backend Messaging - Delta Spec

## ADDED Requirements

### Requirement: Legacy hub polling cannot delete unaccepted messages

If the legacy mobile hub-poll operation remains available, it must acknowledge
only messages that are durably accepted or verified as durable duplicates and
must expose partial outcomes.

#### Scenario: One fetched message fails storage
Given a hub poll returns one valid message whose local durable write fails
When the poll processes and acknowledges its results
Then the failed message identifier is not acknowledged to the hub
And the typed poll result reports the storage failure without counting a new message

#### Scenario: Poll contains accepted, duplicate, and rejected items
Given a hub poll returns an accepted message, a verified durable duplicate, and a rejected message
When the poll completes
Then only the accepted and verified duplicate identifiers are eligible for acknowledgement
And each item has one typed local and remote acknowledgement outcome

#### Scenario: Message preview contains multibyte text
Given an accepted message crosses the preview limit inside a multibyte Unicode scalar
When the poll constructs its bounded preview
Then preview construction returns valid UTF-8 without panic
And the preview does not exceed the documented limit

### Requirement: Canonical discovery can create a durable empty conversation

The messaging boundary must provide an explicit idempotent operation that starts
a conversation for a canonical LXMF delivery destination without requiring a
message or draft.

#### Scenario: Discovered peer has no conversation
Given a canonical discovered delivery destination has no conversation, message, or draft
When the caller starts a conversation for that destination
Then one durable conversation shell is created and returned
And its preview, timestamp, unread count, and connectivity evidence remain empty or zero

#### Scenario: Conversation is started twice
Given a durable conversation shell already exists for a destination
When the caller starts that conversation again
Then the operation returns the same canonical conversation identity
And no duplicate shell, message, draft, or unread state is created

### Requirement: Contact aliases resolve in conversation projections

Conversation summaries must resolve one deterministic display identity while
preserving contact and discovery as separate durable facts.

#### Scenario: Contact alias changes
Given a conversation peer has a canonical announce name and a local contact alias
When the contact alias is changed
Then subsequent conversation projections use the new non-empty alias
And a typed mutation invalidates affected conversation consumers

#### Scenario: Contact alias is absent
Given a conversation peer has no non-empty local alias
When its summary is projected
Then the current canonical announce name is used when available
And otherwise the projection uses a bounded public destination abbreviation without an online claim

### Requirement: Message attempts retain observed route and bearer evidence

A message attempt may reference immutable route and interface observations, but
method, bearer, path, and receipt must remain separate fields.

#### Scenario: Direct attempt uses a TCP interface
Given a direct message attempt has a correlated observed route over a TCP client interface
When the message projection is queried
Then the attempt references the observation generation, interface identity and kind, next hop, hops, and freshness
And Direct remains the delivery method rather than the bearer label

#### Scenario: Attempt has no correlated route observation
Given a message attempt was persisted without an attributable path or interface observation
When delivery details are queried
Then route and bearer evidence are explicitly unknown
And current interface state is not retroactively attached to the attempt
