# Mobile Product Verification - Delta Spec

## ADDED Requirements

### Requirement: Product completion uses synchronized cross-repository evidence

Every completed workflow must be tested against an immutable `styrene-rs` and
`styrene-ui` revision pair with fixture, packaged, and physical evidence kept at
their distinct boundaries.

#### Scenario: Backend handoff is consumed by the UI
Given `styrene-rs` publishes the reviewed additive mobile contract
When `styrene-ui` adopts that contract
Then its dependency and fixture metadata identify the immutable backend revision
And projection tests fail if an authoritative field is dropped or synthesized

#### Scenario: Packaged workflow is accepted
Given component and reducer tests pass for a mobile workflow
When the workflow is executed in a packaged iOS or Android application
Then the retained result identifies both revisions, artifact, platform, OS, scenario, and outcome
And component evidence is not substituted for packaged execution

### Requirement: Accessibility claims require shipping-WebView evidence

Supported mobile workflows must expose accurate semantics and adaptive behavior,
and screen-reader claims must be based on the packaged shipping WebView.

#### Scenario: Compose workflow is inspected
Given New Message contains entry methods, validation, and disabled actions
When its rendered document and packaged accessibility tree are inspected
Then controls expose aligned labels, roles, states, errors, and disabled reasons
And focus reaches recovery guidance without a keyboard trap

#### Scenario: Physical screen reader is evaluated
Given automated semantic checks pass
When VoiceOver or TalkBack evidence is assessed for release
Then the claim remains pending until the workflow is exercised in the packaged WebView
And retained evidence names the screen reader, platform, OS, and revision pair

### Requirement: Evidence ledgers preserve scope decisions and unresolved gaps

Corpus updates must distinguish implemented behavior, packaged acceptance,
reference observations, intentional differences, and excluded product scope.

#### Scenario: Skywave build 9 metadata is reconciled
Given reviewed capture reports Skywave build 9 and Reticulum 1.4.2
When the application-parity candidate record is updated
Then the observed protocol label is scoped to that reviewed capture
And absent LXMF revision and distribution provenance remain unresolved

#### Scenario: Excluded reference feature is reviewed
Given a reference application exposes Calls, Map, location sharing, or group selection
When Styrene parity status is reconciled
Then the feature is recorded as outside this P0 product scope
And it is not silently reported as matched, defective, or protocol evidence
