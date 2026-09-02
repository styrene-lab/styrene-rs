# Mobile Backend Observability - Delta Spec

## ADDED Requirements

### Requirement: Mobile diagnostics are bounded and payload-free

The backend must expose a versioned bounded chronological diagnostic snapshot
whose fields are allowlisted for mobile support use.

#### Scenario: Diagnostic capacity is exceeded
Given more diagnostic events are recorded than the configured mobile capacity
When the diagnostic snapshot is queried
Then it returns the newest events in stable sequence order and reports truncation
And retained event and byte counts remain within documented bounds

#### Scenario: Sensitive values enter diagnostic sources
Given operations contain message payloads, titles, canonical wire, attachment bytes, identity keys, credentials, tokens, passphrases, and private paths
When mobile diagnostics and export are produced
Then none of those values or reversible encodings appear in the result
And safe source, stage, severity, generation, time, and correlation remain available

### Requirement: Diagnostic export is explicit and reproducible

The backend export operation must serialize a stable snapshot through an
explicit redaction pass and return metadata that identifies schema, backend
revision, sequence range, bounds, and truncation.

#### Scenario: Same diagnostic snapshot is exported twice
Given no diagnostic event changes between two exports
When the backend exports the same snapshot twice
Then the serialized content has the same canonical digest
And neither export depends on generic Debug formatting

#### Scenario: Host requests a shareable export
Given the backend produces a validated redacted export artifact
When the host requests its bytes and metadata
Then the backend returns bounded content without choosing a platform share destination
And platform sharing remains a frontend-owned operation

### Requirement: Capability availability is generation-scoped

Active, degraded, unauthorized, unavailable, and unverified capabilities must
carry the current mobile generation and a typed reason where the state is not
active.

#### Scenario: Session generation changes
Given capability state was queried for an earlier generation
When reconnect or replacement creates a newer mobile generation
Then the next capability snapshot carries the newer generation
And a consumer can reject the stale capability result without comparing labels

#### Scenario: Operation is not available
Given a runtime component, authorization, platform capability, or evidence gate prevents an operation
When capability state is queried
Then the operation has a typed unavailable, unauthorized, degraded, or unverified reason
And it is not omitted or reported active
