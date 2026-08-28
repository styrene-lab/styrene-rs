# Mobile UI - Delta Spec

## ADDED Requirements

### Requirement: Mobile product workflows use shared Dioxus source

iOS and Android MUST render Messages, People, Network, and More from the same
maintained Dioxus components and presentation reducers.

#### Scenario: Shared workflow changes
Given a supported mobile workflow changes without a platform-service change
When iOS and Android build the same `styrene-ui` revision
Then both platforms render the changed workflow from the same Rust source
And no equivalent SwiftUI or Compose screen change is required

#### Scenario: Mobile navigation is restored
Given the user restarts the application with valid retained navigation state
When the shared application shell starts
Then it restores only supported mobile route and selection state
And does not expose desktop-only Lab or Admin controls

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

### Requirement: Native code is limited to platform services

Swift and Kotlin may implement OS lifecycle, Bluetooth, Android USB, secure
storage, notifications, permissions, and packaging behind typed platform-service
interfaces. Native code MUST NOT own product navigation or daemon domain state.

#### Scenario: Platform permission is requested
Given a shared Dioxus action requires an OS permission
When the platform-service interface receives the request
Then the native adapter presents the platform permission flow
And returns a typed result to shared Rust state

#### Scenario: Platform adapter is unavailable
Given a build lacks a requested platform capability
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
Then the native adapter reconnects without a manual node-start action
And the shared Network state reports backend-confirmed bearer status

#### Scenario: Android user selects USB fallback
Given Android Bluetooth is unavailable and a compatible USB RNode is attached
When the user explicitly selects USB fallback
Then the platform adapter requests permission and activates USB if approved
And the UI does not report USB active before the adapter confirms it

### Requirement: Native screens retire only after parity evidence

Duplicate SwiftUI and Compose product screens MUST remain available as reference
targets until the corresponding Dioxus workflows pass declared automated and
physical-device gates.

#### Scenario: A shared workflow lacks physical evidence
Given a migrated hardware workflow has no required physical-device result
When retirement readiness is evaluated
Then the corresponding native reference path remains available
And the migration record identifies the missing evidence

#### Scenario: A workflow reaches parity
Given a shared workflow passes state corpus, accessibility, simulator or emulator, and required physical checks
When its parity review is accepted
Then the duplicate native product screen can be removed
And its native platform-service adapter remains if the shared UI still requires it
