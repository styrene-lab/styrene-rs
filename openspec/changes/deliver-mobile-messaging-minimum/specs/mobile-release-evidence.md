# Mobile Release Evidence - Delta Spec

## ADDED Requirements

### Requirement: Mobile minimum behavior has one cross-platform state corpus

The Dioxus iOS and Android targets must run the same named fixtures for session,
discovery, messaging, propagation, stale-generation, duplicate, and failure
states while preserving platform-specific presentation conventions.

#### Scenario: Shared state fixture is rendered
Given a named mobile-minimum fixture defines typed backend and platform state
When the Dioxus iOS and Android targets render that fixture
Then both expose the same required facts, actions, disabled reasons, and accessibility identifiers
And platform layout differences do not change the protocol outcome

#### Scenario: Fixture mode is active
Given the Dioxus mobile application renders deterministic fixture data
When a user views or acts on that data
Then the application marks the session as fixture or preview
And it opens no external network interface or message transmission

### Requirement: Release claims follow executed evidence

Mobile release evidence must identify revision, platform, OS, runtime profile,
network endpoint class, bearer, scenario, and outcome. A lower evidence class
must not imply physical or cross-platform behavior.

#### Scenario: Public TCP acceptance passes
Given a release candidate connects to the public Brutus endpoint
When discovery and messaging acceptance results are recorded
Then the evidence identifies the exact application and hub revisions
And the claim includes only the operations that reached their required terminal evidence

#### Scenario: Physical RNode evidence is absent
Given a platform passes TCP, simulator, or emulator scenarios without accepted physical RNode evidence
When release support is reported
Then TCP messaging support can be reported for that platform
And RNode support remains explicitly unverified

#### Scenario: Physical RNode support is claimed
Given a reference host has complete physical RNode acceptance evidence
When the Dioxus release candidate repeats the applicable scenario on that platform
Then the evidence records NUS properties, MTU or write limit, radio profile, jurisdiction, bidirectional correlation, packet counts, interruption, retained replay, reconnect, and terminal outcome
And the claim applies only to the exercised platform, bearer, RNode board, and firmware class

#### Scenario: Propagation acceptance is evaluated
Given the release candidate selects Brutus as its standard propagation node
When upload, restart persistence, retrieval, acknowledgement, and repeat sync complete
Then the evidence records exactly one delivered message and zero messages on repeat sync
And capacity and expiry are not claimed unless their separate gates pass
