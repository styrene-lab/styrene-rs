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

### Requirement: Mobile implementation follows versioned framework sources

The mobile application MUST resolve framework behavior from its exact Dioxus
crate and CLI version and MUST satisfy the WebView, interaction, accessibility,
and platform-service requirements in this change.

#### Scenario: Stable guidance differs from the pinned prerelease
Given stable Dioxus documentation differs from the pinned prerelease source
When a version-sensitive API or `Dioxus.toml` field is selected
Then the implementation follows the pinned source and schema
And a clean packaged build confirms that the selected configuration is accepted
And runtime evidence separately verifies the selected behavior

#### Scenario: Dioxus generates native scaffolding
Given the pinned Dioxus CLI generates native project and package files
When platform packaging needs configuration
Then maintained Rust code or supported Dioxus inputs provide the configuration
And generated native files remain disposable build output

### Requirement: Mobile layouts remain adaptive and unobscured

The shared application MUST adapt from compact single-pane navigation to wider
list-detail layouts from available window space and MUST keep task-critical
content clear of platform occlusion.

#### Scenario: Available window size changes
Given a selected conversation and retained route state
When the available window crosses a supported layout boundary
Then the application presents the appropriate pane arrangement
And preserves destination, selection, draft, and backend state

#### Scenario: Platform occlusion changes
Given system bars, a display cutout, a safe area, or the virtual keyboard changes
When the application lays out the active workflow
Then the focused control, its label, its error, and the primary action remain reachable
And native and CSS layout do not apply the same inset twice

### Requirement: Mobile interaction meets the accessibility baseline

The Dioxus document MUST meet WCAG 2.2 Level AA, use native HTML semantics when
available, and preserve applicable iOS and Android accessibility behavior.

#### Scenario: A mobile workflow is rendered
Given a supported workflow has interactive controls and asynchronous state
When its rendered structure is inspected
Then controls expose accurate names, roles, values, states, actions, and disabled reasons
And meaningful status changes are available without moving focus

#### Scenario: Accessibility preferences change
Given the user increases text to 200 percent or enables dark appearance, increased contrast, or reduced motion
When the active workflow rerenders
Then content reflows without loss of task-critical information or operation
And state remains perceivable without color or motion alone

#### Scenario: Touch and keyboard input are used
Given the application runs on a supported mobile platform
When the user operates it by touch or keyboard
Then ordinary controls meet the applicable platform target-size floor
And focus remains ordered, visible, unobscured, and free of keyboard traps

### Requirement: Platform behavior crosses typed Rust services

System navigation, lifecycle, permissions, notifications, secure storage, and
required WebView preference or geometry bridges MUST cross typed Rust platform
services without owning product state.

#### Scenario: Platform permission is denied
Given a user action requests a protected platform capability
When the operating system denies or restricts access
Then the platform service returns the typed outcome
And unrelated messaging workflows remain usable

#### Scenario: Mobile process does not receive final cleanup
Given iOS suspends the application or Android kills its process
When the application later starts or resumes
Then durable messages, outbox state, drafts, and retry correlation are restored
And correctness does not depend on a final lifecycle callback

### Requirement: Mobile UX claims use layered evidence

The project MUST keep state tests, rendered-document checks, browser tests,
packaged runs, and physical accessibility tests distinct and MUST NOT substitute
one evidence class for another.

#### Scenario: Automated accessibility checks pass
Given rendered markup passes the configured accessibility checks
When mobile accessibility evidence is evaluated
Then the result is limited to the checked document rules
And VoiceOver and TalkBack claims remain pending until exercised in the shipping WebViews

#### Scenario: Packaged workflow evidence is recorded
Given a corpus journey runs on a packaged iOS or Android application
When the result is retained
Then it identifies revisions, artifact, platform, OS, applicable WebView version, scenario, window, text, appearance, motion context, and outcome
And it links the applicable fixture or application-parity row
