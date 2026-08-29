# Deliver Mobile Messaging Minimum Tasks

## 1. Shared Contract And Corpus
<!-- specs: mobile-application-parity, mobile-network-session, mobile-messaging, mobile-propagation-client, mobile-release-evidence -->

- [x] 1.1 Define shared fixture records for session generations, bearers, peers, conversations, message evidence, propagation selection, synchronization, and typed failures
- [x] 1.2 Add failing Rust serialization and reducer-contract tests for every fixture state before changing production state
- [x] 1.3 Add failing Dioxus component tests for both mobile target classes using the same fixture identifiers and required accessibility identifiers
- [x] 1.4 Remove live-path preview substitution and retain preview records only in explicitly marked fixture sessions
- [ ] 1.5 Recover and admit the versioned RNS-compatible application corpus with exact application, build, platform, protocol-version, provenance, observation-date, and artifact records
- [ ] 1.6 Classify each corpus entry as protocol authority, observed RNS/LXMF application, or interaction-only reference and reject evidence-scope promotion
- [ ] 1.7 Build the P0 workflow parity matrix for identity, TCP setup, discovery, conversations, drafts, direct send, receipts, retry, restart, propagation, and degraded states
- [ ] 1.8 Record the designated floor, observable Styrene outcome, intentional differences, exclusions, and status for every parity row
- [ ] 1.9 Add validation that rejects missing P0 rows, unresolved differences, invalid status, stale provenance, and application observations used as protocol evidence
- [ ] 1.10 Copy the versioned application corpus into `styrene-ui` with exact backend revision provenance and add journey fixtures before implementing unmatched UI workflows
- [ ] 1.11 Seed the recovered inventory with Skywave `1.0` build `5` and the pinned Python RNS, LXMF, and NomadNet references
- [ ] 1.12 Record Sideband, Columba, and MeshChat as unevidenced candidates and Meshtastic and MeshCore as interaction-only references
- [ ] 1.13 Resolve the `795fdaa2b0777c13033787d933d1afc94a2377cb` LXMF `1.1.0` versus `1.1.1` provenance conflict before admitting dependent rows
- [ ] 1.14 Reject deleted Styrene native hosts and RNode firmware evidence as substitutes for external application-parity rows

## 2. Persistent Network Session
<!-- specs: mobile-network-session -->

- [x] 2.1 Add failing `MobileNode` tests for persisted identity, hostname and IPv4 TCP clients, refused endpoints, bounded reconnect, and one-node ownership
- [x] 2.2 Extend the in-process mobile session with typed connection phase, endpoint, generation, failure, and independent bearer observations
- [x] 2.3 Add failing Rust store and Dioxus component tests for cold restoration, reconnect, stale completion rejection, and TCP operation without an RNode
- [x] 2.4 Implement persisted endpoint editing, automatic boot, reconnect presentation, and recoverable failure in shared Rust and Dioxus code
- [x] 2.5 Add deterministic local TCP integration tests proving connection, interruption, reconnect, identity continuity, and clean shutdown
- [x] 2.6 Expose bounded current-generation mobile state invalidations with explicit lag recovery

## 3. Canonical Discovery
<!-- specs: mobile-network-session -->

- [x] 3.1 Add failing Rust tests for canonical delivery-announce decoding, destination-keyed upsert, freshness, and generation-scoped snapshots and events
- [x] 3.2 Expose typed peer aspect, destination, name, observed time, age, and source through the mobile boundary
- [x] 3.3 Add failing Rust reducer and Dioxus component tests for repeated announces, stale generation events, empty live directories, and local announce outcomes
- [x] 3.4 Implement event-driven People and Network updates without duplicate peers or remote-reception claims

## 4. Durable Text Messaging
<!-- specs: mobile-messaging -->

- [x] 4.1 Add failing embedded-runtime tests from mobile send request through canonical persistence, explicit method selection, attempt correlation, inbound persistence, unread state, and restart restoration
- [x] 4.2 Extend the typed Rust mobile session for drafts, requested and actual method, attempt, propagation upload, receipt evidence, retry, and typed failure
- [x] 4.3 Add failing Rust reducer and Dioxus component tests for draft revision races, empty live state, queued versus delivered rendering, inbound unread behavior, duplicate evidence, retry, and restart restoration
- [x] 4.4 Implement shared Dioxus conversation, composer, history, retry, and delivery-detail components using backend-owned state
- [x] 4.5 Add a deterministic two-identity text round trip proving one canonical outbound record, one inbound record, and exact correlation

## 5. Standard Propagation Client
<!-- specs: mobile-propagation-client -->

- [x] 5.1 Add failing Rust mobile tests for selected-node persistence, active-metadata validation, explicit propagated upload, identified inventory, durable-before-ack retrieval, duplicate suppression, and partial failure
- [x] 5.2 Expose standard propagation discovery, selection, readiness, upload, manual sync, automatic-sync policy, progress, and terminal observations through the typed Rust mobile session
- [x] 5.3 Add failing single-flight scheduler tests for initial connection, reconnect, foreground opportunity, cooldown, deadline, cancellation, and shutdown
- [x] 5.4 Add failing Rust reducer and Dioxus component tests for selection persistence, stale node metadata, manual sync, automatic sync disclosure, upload versus delivery, progress, repeat sync, and recoverable failure
- [x] 5.5 Implement client-only propagation controls without hosting, peering, capacity, or expiry administration
- [x] 5.6 Add local Python/Rust tests for upload, byte-identical replay, offline retrieval, acknowledgement, repeat sync, and daemon restart persistence

## 6. Platform And RNode Coexistence
<!-- specs: mobile-network-session, mobile-release-evidence -->

- [x] 6.1 Add failing Rust platform-service and Dioxus tests proving TCP remains operational when Bluetooth or USB is unavailable, denied, interrupted, or unverified
- [x] 6.2 Implement Rust-owned platform services and present TCP, Bluetooth RNode, and Android USB as independent backend-confirmed bearer states
- [ ] 6.3 Complete Dioxus channel-detachment, approved-device, queue-bound, fragmentation, serialization, retention, and physical acceptance gates before enabling an RNode support claim
- [x] 6.4 Preserve explicit Android USB fallback and prevent it from preempting an approved Bluetooth bearer

## 7. Cross-Platform Acceptance
<!-- specs: mobile-application-parity, mobile-release-evidence -->

- [x] 7.1 Run the shared state corpus through Rust reducer and Dioxus component tests for iOS and Android target classes
- [ ] 7.2 Run iOS Simulator and Android emulator cold-launch, endpoint failure, reconnect, discovery, conversation, retry, and propagation-state scenarios
- [ ] 7.3 Run public-Brutus discovery, direct text, propagated upload, offline retrieval, acknowledgement, repeat sync, and hub-restart scenarios with bounded evidence
- [ ] 7.4 Run applicable physical iOS and Android TCP lifecycle scenarios and retain explicit gaps for unavailable hardware or background execution
- [ ] 7.5 Run complete physical RNode scenarios for each claimed platform, including NUS properties, MTU or write limit, fragmented traffic, bidirectional correlation, interruption, retained replay, and reconnect
- [ ] 7.6 Record exact application, backend, hub, platform, OS, endpoint class, bearer, correlation, deadline, and outcome for every live gate
- [ ] 7.7 Verify the release candidate contains no maintained Swift or Kotlin host or adapter before publishing the product capability
- [ ] 7.8 Replay every applicable P0 application-parity journey through the packaged Dioxus iOS and Android applications
- [ ] 7.9 Record matched, intentionally different, deferred, unsupported, and unevidenced outcomes without substituting component or Python evidence for package execution

## 8. Release Verification
<!-- specs: mobile-application-parity, mobile-network-session, mobile-messaging, mobile-propagation-client, mobile-release-evidence -->

- [ ] 8.1 Run Rust formatting, warning-denied Clippy, focused mobile and propagation tests, fixture validation, and migration or restart checks
- [ ] 8.2 Build, install, cold-launch, and inspect fatal logs for the Dioxus iOS and Android release candidates
- [ ] 8.3 Verify existing identity, messages, contacts, drafts, endpoint, and new propagation selection survive upgrade
- [ ] 8.4 Publish capability claims from passing evidence and list RNode, attachment, Paper, NomadNet, propagation-host, capacity, and expiry exclusions explicitly
- [ ] 8.5 Publish the complete application-parity ledger with corpus version, row status, intentional-difference rationale, and evidence references

## 9. BLE Backend Ownership
<!-- specs: mobile-ble-rnode -->

- [x] 9.1 Add a failing `MobileNode` test proving a Bluetooth byte attempt requires approval and changes only the Bluetooth RNode bearer
- [x] 9.2 Add explicit BLE and Android USB attempt identity to the mobile byte-session API without changing KISS protocol behavior
- [x] 9.3 Add failing tests for one active bearer, stale attempt rejection, exact readback gating, and idempotent stop attribution
- [x] 9.4 Integrate the shared `RNodeEngine` attempt metadata so backend-owned fragmentation respects the safe platform write size
- [x] 9.5 Add failing retained-handoff tests proving a failed or cancelled platform write does not silently lose an outbound RNS packet

## 10. BLE Platform Contract
<!-- specs: mobile-ble-rnode -->

- [x] 10.1 Add failing pure Rust tests for permission, adapter availability, bounded discovery, explicit selection, approval, forget, and generation rejection
- [x] 10.2 Define plain Rust BLE candidate, approved-peripheral, NUS property, write-limit, event, and ordered-byte attempt contracts
- [x] 10.3 Add fake-attempt tests for arbitrary notification fragmentation, multiple KISS frames per notification, serialized writes, cancellation, disconnect, and reconnect
- [x] 10.4 Implement one cancellable session owner that pumps the platform attempt through the backend byte session without owning RNode protocol truth
- [ ] 10.5 Add Rust-owned Network controls for Scan, explicit peripheral selection, retry, and Forget with typed disabled and failure states

## 11. Native BLE Adapters
<!-- specs: mobile-ble-rnode -->

- [ ] 11.1 Add failing safe-boundary tests for CoreBluetooth state, NUS discovery, characteristic properties, notification delivery, response writes, write limits, and disconnect
- [ ] 11.2 Implement the iOS adapter inside the existing Rust Apple bridge without exposing Objective-C objects or adding maintained Swift
- [ ] 11.3 Add failing Android bridge tests for API-level permission, adapter state, scan identity, NUS discovery, response writes, MTU conversion, callback generation, and close
- [ ] 11.4 Implement the Android GATT adapter through tracked Rust and approved generated-host extension seams without adding maintained Kotlin product logic
- [ ] 11.5 Build warning-denied iOS and Android targets and verify generated native output remains untracked

## 12. BLE Physical Acceptance
<!-- specs: mobile-ble-rnode, mobile-release-evidence -->

- [ ] 12.1 Record board, firmware, NUS UUIDs and properties, mobile model, OS, application and backend revisions, radio profile, and test jurisdiction
- [ ] 12.2 Verify explicit first approval, stored-identifier reconnect, pairing-window expiry, Forget, denial, powered-off, interruption, and foreground recovery on each claimed platform
- [ ] 12.3 Verify observed safe write size, fragmented inbound traffic, serialized response writes, exact RNode configuration readback, and bounded queue behavior
- [ ] 12.4 Verify bidirectional packet and message correlation, packet counters, retained replay after interruption, and no duplicate delivery
- [ ] 12.5 Publish BLE support only for the exercised platform, board, firmware class, and scenarios. Retain every missing evidence item as an explicit exclusion
