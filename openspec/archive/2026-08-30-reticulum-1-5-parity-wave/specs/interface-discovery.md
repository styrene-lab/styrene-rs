# Interface Discovery - Delta Spec

## ADDED Requirements

### Requirement: Discovery identifies the transport implementation

Canonical interface-discovery metadata must include transport implementation name and version in addition to interface type, transport identity, transport capability, and sanitized name.

#### Scenario: Discovery metadata is encoded
Given a discoverable interface has valid runtime metadata
When its discovery record is encoded
Then canonical keys `0xFD` and `0xFC` contain the implementation name and version
And decoding preserves those values as observed remote metadata

### Requirement: Discovery may publish an operator LXMF address

A discovery record may carry an operator LXMF address at canonical key `0xF0`, but only as exactly 16 bytes; omission remains compatible.

#### Scenario: Operator address is present
Given a discoverable interface has a configured 16-byte operator LXMF address
When discovery metadata round-trips
Then the decoded observation contains that exact address
And it is not inferred from the transport identity

#### Scenario: Operator address is malformed
Given discovery metadata contains a non-byte or non-16-byte operator address
When the metadata is decoded
Then the record is rejected without partial persistence
And no automatic connection is attempted

### Requirement: Discovery metadata is observational in this wave

Decoded reachability and operator metadata must not automatically create an interface or contact without a separately implemented and authorized workflow.

#### Scenario: Reachable endpoint is discovered
Given a valid record advertises a reachable endpoint
When the record is accepted
Then it is exposed as remote discovery metadata
And transport configuration remains unchanged
