# Mobile Application Parity - Delta Spec

## ADDED Requirements

### Requirement: Mobile workflow floors use a versioned reference corpus

The mobile product must maintain a versioned corpus of observed messaging
application workflows with enough provenance to reproduce or invalidate each
observation. Every reference must be classified as a protocol authority, an
observed RNS/LXMF application, a candidate RNS/LXMF application, or an
interaction-only reference.

#### Scenario: Reference observation is admitted
Given a messaging application provides evidence relevant to a mobile workflow
When its observation is added to the application-parity corpus
Then the record identifies application version and build, platform and OS, protocol versions when observable, provenance, observation date, and evidence artifacts
And the record identifies the workflows and evidence scope it can support

#### Scenario: Candidate application lacks retained evidence
Given an RNS-compatible application is named but has no pinned version, provenance, and executed observation
When the application-parity corpus is validated
Then the application remains a candidate with an unevidenced status
And it cannot become a designated workflow floor

#### Scenario: Interaction-only reference informs a workflow
Given an application is not proven to use compatible RNS or LXMF behavior
When its interaction pattern is recorded in the corpus
Then the record may inform information architecture or ergonomics
And it cannot satisfy a protocol, propagation, receipt, or bearer claim

### Requirement: Every required journey has an explicit parity decision

The application-parity corpus must map each required mobile journey to a
designated observed floor and record the observed facts, Styrene requirement,
intentional differences, exclusions, and current status. Status must be
`matched`, `intentionally_different`, `deferred`, `unsupported`, or
`unevidenced`.

#### Scenario: Required journey is defined
Given a P0 identity, connection, discovery, messaging, propagation, restart, or failure-recovery journey is in scope
When implementation readiness is evaluated
Then the parity matrix identifies its designated reference floor and observable Styrene outcome
And the row contains no unresolved or implicit difference

#### Scenario: Reference applications disagree
Given two admitted applications expose different behavior for the same journey
When the product floor is selected
Then the matrix names the selected behavior and rationale
And it does not silently combine unrelated behavior into a larger requirement

#### Scenario: Styrene intentionally differs
Given Styrene requires stronger typed evidence, security, accessibility, or failure disclosure than the observed application
When the parity row is approved
Then the row records the difference and its rationale
And the stronger behavior is tested instead of being reported as a parity failure

### Requirement: Packaged Dioxus applications replay accepted journeys

The iOS and Android Dioxus packages must execute every applicable required
journey against authoritative backend state. Component fixtures may prepare and
assert states, but they cannot satisfy packaged application or protocol gates.

#### Scenario: Required journey is accepted on both platforms
Given a parity row applies to iOS and Android
When the packaged Dioxus applications execute that journey
Then both platforms expose the required facts, actions, disabled reasons, and terminal backend outcome
And the evidence records exact UI revision, backend revision, artifact identity, platform, OS, reference row, and correlation

#### Scenario: Platform capability changes the journey
Given a parity row depends on a capability unavailable on one platform
When cross-platform acceptance is evaluated
Then the unavailable platform exposes the typed reason without fabricating success
And the parity ledger records the platform-specific outcome without weakening the other platform's gate

### Requirement: Product parity and protocol parity remain independent

Application observations, deterministic state fixtures, pinned protocol
interoperability, and packaged-target execution must be reported separately.

#### Scenario: Reference application journey is observed
Given an admitted application completes a messaging journey
When Styrene release evidence is evaluated
Then the observation establishes only the recorded product floor
And Styrene still requires its applicable Python interoperability and packaged-target evidence

#### Scenario: Python interoperability passes without product replay
Given pinned Python RNS or LXMF completes the corresponding protocol exchange
When the packaged Dioxus journey has not passed
Then protocol compatibility may be reported at its executed scope
And mobile product parity remains unevidenced

### Requirement: Release parity accounting is closed and non-vacuous

Every in-scope parity row must appear in release evidence. A required row may
support a workflow claim only when it is `matched` or an approved
`intentionally_different` result with all applicable evidence gates complete.

#### Scenario: Required row remains incomplete
Given a required parity row is deferred, unsupported, or unevidenced
When mobile capability claims are published
Then the affected workflow is excluded or identified as incomplete
And the row is not omitted from the release ledger

#### Scenario: Reference provenance becomes stale
Given an admitted observation cannot be reproduced or its provenance no longer resolves
When corpus validation runs
Then the observation cannot silently remain an accepted workflow floor
And affected parity rows return to an explicit unevidenced state

#### Scenario: One revision has conflicting version labels
Given two provenance records name the same upstream revision with different versions
When corpus validation runs
Then affected rows fail validation until the revision metadata is reconciled
And neither version label is selected implicitly
