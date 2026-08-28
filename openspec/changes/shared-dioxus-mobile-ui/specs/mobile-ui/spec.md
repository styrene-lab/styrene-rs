# Mobile UI - Delta Spec

## ADDED Requirements

### Requirement: Mobile product workflows use shared Dioxus source

iOS and Android MUST render Messages, People, Network, and More from the same
maintained Dioxus components and presentation reducers.

#### Scenario: Shared workflow changes
Given a supported mobile workflow changes without a platform-service change
When iOS and Android build the same `styrene-ui` revision
Then both platforms render the changed workflow from the same Rust source
And no platform-specific product screen change is required

#### Scenario: Mobile navigation is restored
Given the user restarts the application with valid retained navigation state
When the shared application shell starts
Then it restores only supported mobile route and selection state
And does not expose desktop-only Lab or Admin controls

#### Scenario: Accepted application journey is implemented
Given the versioned mobile application-parity corpus defines an accepted workflow floor
When iOS and Android execute that workflow from the same Dioxus revision
Then both expose the required facts, actions, disabled reasons, and backend-confirmed outcome
And any intentional Styrene difference remains explicit in the parity record

### Requirement: Shared UI consumes authoritative backend state

The mobile Dioxus application MUST use the shared typed frontend session and
MUST NOT infer protocol, route, bearer, or delivery outcomes from display text.

#### Scenario: Message lifecycle changes
Given the backend reports a typed message lifecycle event
When the shared message reducer applies the event
Then iOS and Android expose the same lifecycle state and correlation evidence
And neither platform fabricates a delivery transition

#### Scenario: Capability is unavailable
Given the active session reports an operation unsupported
When the related mobile workflow renders
Then both platforms omit or disable the action with the typed reason
And the UI does not emulate the operation

### Requirement: Mobile product source remains Rust-owned

Maintained mobile product and platform-service code MUST be Rust. Generated
platform packaging output MUST remain outside source control and MUST NOT own
product navigation or daemon domain state.

#### Scenario: Platform permission is requested
Given a shared Dioxus action requires an OS permission
When the platform-service interface receives the request
Then the Rust platform service presents the platform permission flow
And returns a typed result to shared Rust state

#### Scenario: Platform adapter is unavailable
Given a build lacks a requested Rust platform capability
When shared UI evaluates the capability
Then the platform service returns an explicit unavailable result
And the UI remains operational for unaffected workflows

### Requirement: Mobile RNode behavior survives renderer migration

The Dioxus mobile application MUST preserve approved-peripheral reconnect,
Bluetooth-first bearer selection, KISS packet-channel attachment, bounded
retention, and explicit Android USB fallback.

#### Scenario: Approved RNode becomes reachable
Given the application has restored an approved Bluetooth peripheral
When that peripheral becomes reachable
Then the Rust platform service reconnects without a manual node-start action
And the shared Network state reports backend-confirmed bearer status

#### Scenario: Android user selects USB fallback
Given Android Bluetooth is unavailable and a compatible USB RNode is attached
When the user explicitly selects USB fallback
Then the platform adapter requests permission and activates USB if approved
And the UI does not report USB active before the adapter confirms it

### Requirement: Native-language hosts are absent

The maintained source tree MUST NOT contain Swift or Kotlin mobile hosts,
platform adapters, product state, or product screens.

#### Scenario: Mobile source is inspected
Given the Dioxus mobile application is prepared for release
When maintained product and platform-service source is inspected
Then the implementation is Rust-owned
And no Swift or Kotlin host is required to build or validate it

#### Scenario: Platform tooling generates scaffolding
Given Dioxus or platform tooling generates native packaging files
When source-control status is inspected
Then those files remain untracked build output
And maintained source does not depend on editing them
