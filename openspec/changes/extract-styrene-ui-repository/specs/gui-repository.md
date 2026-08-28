# GUI Repository - Delta Spec

## ADDED Requirements

### Requirement: Dioxus application source has one repository authority

The maintained Dioxus application source must reside in the dedicated
`styrene-ui` repository. `styrene-rs` must not retain a second editable copy.

#### Scenario: Extraction is accepted
Given the extracted repository passes its required checks
When the repository transition is completed
Then `styrene-ui` is the authoritative Dioxus application source
And `styrene-rs` contains only migration records or links instead of a maintained copy

### Requirement: Extraction preserves provenance

The new repository must retain auditable provenance for the extracted
`styrene-dx` source and must record its source revision.

#### Scenario: An extracted file is inspected
Given a file originated in `crates/apps/styrene-dx`
When its history or migration record is inspected in `styrene-ui`
Then the record identifies the source repository and immutable revision
And the file's relevant history is available or mapped by the extraction record

### Requirement: GUI dependencies use immutable backend revisions

`styrene-ui` must consume application-facing `styrene-rs` crates through an
immutable revision or released version. It must not depend on a developer's
local checkout path.

#### Scenario: Clean GUI checkout builds
Given a clean checkout has the supported Rust and platform toolchains
When its dependency graph is resolved
Then all `styrene-rs` dependencies resolve from declared immutable sources
And no dependency requires an undeclared sibling checkout

### Requirement: Repository boundaries preserve backend authority

The GUI repository must not own RNS, LXMF, daemon, IPC wire, transport, or
interoperability-runner behavior.

#### Scenario: GUI invokes a backend operation
Given a Dioxus workflow needs a protocol or daemon operation
When the workflow submits the operation
Then it uses a typed `styrene-rs` client or session contract
And the Dioxus component does not implement the protocol operation

### Requirement: Desktop behavior survives extraction

The extracted desktop application must retain its explicit runtime profiles,
domain stores, operator routes, capability behavior, and bounded Lab boundary.

#### Scenario: Extracted desktop smoke suite runs
Given `styrene-ui` is checked out without a running production daemon
When its desktop smoke suite runs
Then Fixture, Live-failure, and Embedded scenarios reach their declared outcomes
And Fixture opens no external network interface
