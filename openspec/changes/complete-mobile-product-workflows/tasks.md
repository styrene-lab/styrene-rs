# Complete Mobile Product Workflows Tasks

## 1. Contract Freeze And Corpus Reconciliation
<!-- specs: mobile-product-projection, mobile-product-verification -->

- [x] 1.1 `[styrene-rs]` Add failing serialization and projection tests for distinct runtime phases, canonical message and attempt identifiers, direction, persisted time, requested and actual method, retry eligibility, typed failure, route, bearer, propagation upload, and receipt evidence
- [x] 1.2 `[styrene-rs]` Reconcile stale integration-corpus missing-capability text against completed backend P0 contracts without changing packaged or application-parity status
- [x] 1.3 `[styrene-rs]` Record Reticulum `1.4.2` as a capture-scoped Skywave build 9 observation while retaining unresolved LXMF revision, distribution provenance, and candidate status
- [x] 1.4 `[cross-repo]` Define the additive handoff fixture and immutable revision metadata consumed by `styrene-ui`, including mutation tests that reject dropped or synthesized authoritative fields

## 2. Authoritative Runtime And Delivery Projection
<!-- specs: mobile-product-projection -->

- [x] 2.1 `[styrene-rs]` Expose any missing additive DTO fields needed to preserve runtime phases and independent message, attempt, route, bearer, receipt, failure, and retry evidence through the public mobile/session boundary
- [x] 2.2 `[styrene-ui]` Preserve backend runtime state and every connection phase independently in renderer-neutral state, live projection, and rendered status
- [x] 2.3 `[styrene-ui]` Replace lossy `project_message` mapping with exhaustive typed projection; remove text-based retry inference and preserve unknown evidence explicitly
- [x] 2.4 `[styrene-ui]` Render message direction, canonical chronology, retry eligibility, delivery method, upload, route, bearer, receipt, and typed failure without conflating transport acceptance with delivery
- [x] 2.5 `[cross-repo]` Run equivalent backend projection, UI reducer, restart, stale-generation, duplicate-event, and non-retryable-failure tests against the declared revision pair

## 3. Discovery And New Message
<!-- specs: mobile-product-projection, mobile-platform-workflows -->

- [x] 3.1 `[styrene-ui]` Replace the discovered-peer dead end with an action that invokes the backend idempotent conversation-shell operation and opens the resulting conversation
- [x] 3.2 `[styrene-ui]` Add a first-class New Message entry with peer search and bounded direct LXMF destination entry using the same backend validation path
- [x] 3.3 `[styrene-ui]` Define bounded generation-tagged Rust platform-service contracts and deterministic mocks for clipboard read and QR scanning, including denied, restricted, unavailable, oversized, malformed, cancelled, and successful outcomes
- [x] 3.4 `[styrene-ui]` Implement iOS and Android clipboard and QR adapters behind the typed destination-ingress contracts without moving destination validation into platform code
- [x] 3.5 `[styrene-ui]` Gate Direct and Propagated methods independently from backend capability and current selected-node readiness; preserve destination, draft revision, and method when readiness changes
- [x] 3.6 `[cross-repo]` Verify discovered, direct-entry, paste, and scan paths create one canonical conversation and never create state from invalid input

## 4. Status, People, And Network Truth
<!-- specs: mobile-product-projection -->

- [x] 4.1 `[styrene-rs]` Confirm which runtime, bearer, peer, unread, route, and propagation summary facts are authoritative; add only missing bounded aggregate fields and retain unknown values
- [x] 4.2 `[styrene-ui]` Add a concise operational summary within the existing Messages, People, Network, and More architecture without fabricating node, relay, path, mail, or reachability state
- [x] 4.3 `[styrene-ui]` Present peer aspect, source, observation age, announce count, and bounded destination details while keeping freshness distinct from reachability
- [x] 4.4 `[styrene-ui]` Add empty, ready, reconnecting, degraded, failed, stale-peer, unknown-route, and mixed-bearer fixture and component coverage

## 5. Identity And Encrypted Recovery
<!-- specs: mobile-platform-workflows -->

- [x] 5.1 `[styrene-ui]` Implement display-name edit and correct public LXMF destination labeling using existing typed metadata and durable edit operations
- [x] 5.2 `[styrene-ui]` Implement Copy and Show QR through Rust platform services and ensure clipboard, QR, accessibility, fixtures, and logs contain only explicitly public identity material
- [x] 5.3 `[styrene-rs]` Define and test opaque encrypted identity backup metadata, authenticated export, non-destructive restore, compatibility, wrong-protection, corruption, custody-unavailable, and identity-conflict outcomes
- [x] 5.4 `[styrene-rs]` Expose typed backup and restore operations that never return private key bytes or protection input through presentation DTOs, diagnostics, or generic debug output
- [x] 5.5 `[styrene-ui]` Implement encrypted Backup and preboot Restore surfaces with typed progress, explicit create-or-restore choice, confirmation, cancellation, and failure states
- [x] 5.6 `[styrene-ui]` Implement Rust file/share adapters for opaque backup artifacts without retaining protection input or private material
- [ ] 5.7 `[cross-repo]` Run migration, restart, wrong-backup, interrupted-export, interrupted-restore, custody continuity, and unchanged-identity tests before enabling recovery claims

## 6. Permissions And Propagation Lifecycle
<!-- specs: mobile-platform-workflows -->

- [x] 6.1 `[styrene-rs]` Remove or disable free-running mobile propagation polling so automatic synchronization starts only from connection, reconnection, foreground, or platform-granted background opportunities under single-flight cooldown and deadlines
- [x] 6.2 `[styrene-rs]` Expose trigger source, capability, readiness, last synchronization, progress, cooldown, and terminal outcome without claiming guaranteed background execution
- [x] 6.3 `[styrene-ui]` Present independent camera, Bluetooth, notification, and secure-storage states plus supported system-settings recovery; do not request location permission in the P0 product
- [x] 6.4 `[styrene-ui]` Replace unconditional automatic-sync copy with selected-node readiness, manual airtime disclosure, best-effort lifecycle disclosure, disabled Sync reason, and progress or failure
- [x] 6.5 `[styrene-ui]` Render the backend-owned last synchronization trigger source without inferring it from lifecycle or elapsed time
- [x] 6.6 `[cross-repo]` Test no-op wall-clock passage, initial connection, reconnect, foreground opportunity, denied background opportunity, cooldown, overlap, cancellation, and process restart

## 7. Adaptive UX And Accessibility
<!-- specs: mobile-platform-workflows, mobile-product-verification -->

- [x] 7.1 `[styrene-ui]` Add semantic headings, lists, forms, status regions, aligned visible and accessible names, error association, disabled reasons, and deterministic focus restoration to every changed workflow
- [ ] 7.2 `[styrene-ui]` Verify compact, medium, expanded, landscape, split-window, keyboard-open, 320 CSS-pixel, 200 percent text, platform accessibility text, dark, increased-contrast, and reduced-motion states
- [ ] 7.3 `[styrene-ui]` Verify ordinary controls meet iOS and Android target-size floors and that New Message, QR denial, backup/restore, retry, and propagation recovery have no gesture-only action or keyboard trap
- [ ] 7.4 `[cross-repo]` Retain packaged VoiceOver and TalkBack evidence separately from document, reducer, simulator, and semantic-snapshot checks

## 8. Packaged Acceptance And Claims
<!-- specs: mobile-product-projection, mobile-platform-workflows, mobile-product-verification -->

- [ ] 8.1 `[cross-repo]` Build and install clean iOS and Android candidates from one immutable backend and UI revision pair and verify no maintained Swift or Kotlin product implementation is required
- [ ] 8.2 `[cross-repo]` Execute discovery-to-conversation, direct entry, paste, QR denial and success, draft preservation, Direct send, receipt, retryable and terminal failure, restart restoration, and degraded runtime scenarios
- [ ] 8.3 `[cross-repo]` Execute propagation no-node, stale-node, manual sync, foreground automatic sync, overlap, cooldown, process restart, upload, receipt, and retrieved-message scenarios
- [ ] 8.4 `[cross-repo]` Execute identity rename, public copy and QR, encrypted backup, corrupt and wrong-protection restore, successful restore, custody continuity, and private-material redaction scenarios on applicable physical devices
- [ ] 8.5 `[cross-repo]` Reconcile backend, integration, application-parity, accessibility, and release ledgers from retained evidence while preserving Calls, Map, location sharing, groups, iCloud sync, propagation hosting, and guaranteed background execution as exclusions
- [ ] 8.6 `[cross-repo]` Run formatting, warning-denied Clippy, focused backend, reducer, component, migration, restart, corpus, OpenSpec, clean package, and fatal-log checks in both repositories

## 9. QR Ingress Handoff
<!-- specs: mobile-platform-workflows -->

- [x] 9.1 `[cross-repo]` Select the P0 QR architecture, enumerate rejected and deferred alternatives, define resource and privacy bounds, and publish the TDD corpus
- [x] 9.2 `[styrene-ui]` Add failing pure-Rust tests that generate JPEG and PNG QR images in memory and cover canonical, malformed, oversized, no-code, ambiguous, and resource-exhaustion outcomes
- [x] 9.3 `[styrene-ui]` Implement the bounded `quircs` decoder with `image` default formats disabled and no decoded payload in `Debug`, errors, or diagnostics
- [x] 9.4 `[styrene-ui]` Compose one generation-aware Dioxus file capture with the existing `QrDestinationScanner` contract while preserving current manual and pasted input
- [ ] 9.5 `[nucleus]` Build and install the Android package, then retain E87 evidence for camera grant, denial, cancellation, gallery selection, canonical success, malformed input, ambiguous input, rotation, process interruption, and stale completion
- [ ] 9.6 `[Chriss-MacBook-Pro]` Build and install the iOS package, then retain equivalent camera and image-picker evidence without using Android results as Apple evidence
- [x] 9.7 `[cross-repo]` Complete task 3.6 with manual, discovered, pasted, and scanned candidates; do not treat the implemented UI convergence as proof of one durable backend operation
