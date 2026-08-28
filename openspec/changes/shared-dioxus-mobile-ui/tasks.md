# Shared Dioxus Mobile UI Tasks

## 1. Shared Mobile Shell
<!-- specs: mobile-ui/spec -->

- [ ] 1.1 Define mobile route, adaptive layout, navigation restoration, and desktop-route exclusion tests
- [x] 1.2 Add iOS and Android Dioxus launchers that render the same fixture-only shell
- [ ] 1.3 Implement shared Messages, People, Network, and More navigation and persistent identity context
- [ ] 1.4 Add loading, empty, ready, degraded, error, and high-information fixtures for every mobile route
- [ ] 1.5 Verify one workflow source change appears on both platforms from the same Rust source

## 2. Backend Session And State
<!-- specs: mobile-ui/spec -->

- [ ] 2.1 Connect the shared Embedded session directly to the Rust Dioxus application
- [ ] 2.2 Move mobile snapshot, event, generation, capability, and lifecycle handling into shared reducers
- [ ] 2.3 Preserve typed correlation, observation provenance, delivery lifecycle, and disabled reasons
- [ ] 2.4 Add tests that reject stale generations, duplicate events, fabricated success, and fixture data in live stores
- [ ] 2.5 Verify embedded startup, partial-failure cleanup, restoration, and explicit shutdown on both platforms

## 3. Messaging And People
<!-- specs: mobile-ui/spec -->

- [ ] 3.1 Implement shared conversation list, thread, draft, composition, send, retry, and delivery-evidence views
- [ ] 3.2 Keep LXMF method, bearer, and evidence as independent typed fields
- [ ] 3.3 Implement shared saved-contact, discovered-peer, identity, route, and trust views
- [ ] 3.4 Add reducer and component tests for outbound, inbound, failed, receipt, resource, and restart states
- [ ] 3.5 Run cross-platform message roundtrip and durable correlation corpus scenarios

## 4. Platform-Service Boundary
<!-- specs: mobile-ui/spec -->

- [ ] 4.1 Define typed service traits for lifecycle, Bluetooth, Android USB, secure storage, notifications, permissions, and sharing
- [ ] 4.2 Implement iOS platform services in Rust without product navigation or daemon domain state
- [ ] 4.3 Implement Android platform services in Rust without product navigation or daemon domain state
- [ ] 4.4 Add deterministic mock adapters for unavailable, denied, interrupted, failed, and successful outcomes
- [ ] 4.5 Verify adapter callbacks are bounded, generation-aware, and safe after route or lifecycle changes

## 5. RNode And Lifecycle Integration
<!-- specs: mobile-ui/spec -->

- [ ] 5.1 Connect iOS Bluetooth discovery, approval, protected access, reconnect, and byte transport through Rust platform services
- [ ] 5.2 Connect Android Bluetooth discovery, approval, reconnect, and byte transport to the shared Rust host
- [ ] 5.3 Connect Android USB permission and byte transport only through an explicit fallback action
- [ ] 5.4 Preserve Rust KISS framing, serialized writes, radio configuration, bounded retention, and packet-channel attachment
- [ ] 5.5 Verify automatic node startup and approved-RNode reconnect on cold launch
- [ ] 5.6 Verify bearer interruption, app recreation, channel reattachment, forgetting approval, and shutdown

## 6. Remaining Mobile Workflows
<!-- specs: mobile-ui/spec -->

- [ ] 6.1 Implement shared identity, propagation, experimental pages, settings, diagnostics, and capability disclosures
- [ ] 6.2 Keep unavailable backend operations explicit and omit desktop-only Lab and Admin controls
- [ ] 6.3 Add secure storage, notification, background opportunity, sharing, and platform-settings flows through adapters
- [ ] 6.4 Verify sensitive fields are redacted before entering diagnostics, fixture exports, or UI logs

## 7. Accessibility And Cross-Platform Corpus
<!-- specs: mobile-ui/spec -->

- [ ] 7.1 Define stable shared accessibility labels, actions, values, disabled reasons, and focus order
- [ ] 7.2 Run the same state corpus on iOS Simulator and Android emulator
- [ ] 7.3 Verify dynamic text, contrast, reduced motion, keyboard or switch access, and screen-reader semantics where supported
- [ ] 7.4 Record exact GUI revision, backend revision, platform, OS, fixture, and artifact provenance
- [ ] 7.5 Preserve partial outcomes when physical hardware or platform services are unavailable
- [ ] 7.6 Replay every applicable accepted application-parity journey on both packaged targets and retain the backend corpus row in evidence

## 8. Physical Acceptance
<!-- specs: mobile-ui/spec -->

- [ ] 8.1 Verify each workflow against the application-parity corpus, shared state corpus, and backend contract
- [ ] 8.2 Run physical iOS Bluetooth, secure storage, notification, and lifecycle scenarios
- [ ] 8.3 Run physical Android Bluetooth, USB, secure storage, notification, and lifecycle scenarios
- [ ] 8.4 Approve or reject each workflow based on recorded Dioxus evidence
- [ ] 8.5 Verify no maintained Swift or Kotlin mobile host or adapter exists
- [ ] 8.6 Verify generated packaging output remains untracked and disposable

## 9. Release Verification
<!-- specs: mobile-ui/spec -->

- [ ] 9.1 Run Rust formatting, warning-denied Clippy, unit, reducer, component, and dependency-boundary checks
- [ ] 9.2 Build and test desktop, iOS, and Android packages from a clean `styrene-ui` checkout
- [ ] 9.3 Verify immutable `styrene-rs` dependency resolution and backend compatibility negotiation
- [ ] 9.4 Run available cross-platform messaging, RNode, restart, and evidence-retention scenarios
- [ ] 9.5 Publish release status, physical evidence gaps, and rollback instructions
