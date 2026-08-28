# Deliver Mobile Messaging Minimum Tasks

## 1. Shared Contract And Corpus
<!-- specs: mobile-network-session, mobile-messaging, mobile-propagation-client, mobile-release-evidence -->

- [x] 1.1 Define shared fixture records for session generations, bearers, peers, conversations, message evidence, propagation selection, synchronization, and typed failures
- [x] 1.2 Add failing Rust serialization and reducer-contract tests for every fixture state before changing production state
- [x] 1.3 Add failing Dioxus component tests for both mobile target classes using the same fixture identifiers and required accessibility identifiers
- [x] 1.4 Remove live-path preview substitution and retain preview records only in explicitly marked fixture sessions

## 2. Persistent Network Session
<!-- specs: mobile-network-session -->

- [x] 2.1 Add failing `MobileNode` tests for persisted identity, hostname and IPv4 TCP clients, refused endpoints, bounded reconnect, and one-node ownership
- [x] 2.2 Extend the in-process mobile session with typed connection phase, endpoint, generation, failure, and independent bearer observations
- [x] 2.3 Add failing Rust store and Dioxus component tests for cold restoration, reconnect, stale completion rejection, and TCP operation without an RNode
- [x] 2.4 Implement persisted endpoint editing, automatic boot, reconnect presentation, and recoverable failure in shared Rust and Dioxus code
- [x] 2.5 Add deterministic local TCP integration tests proving connection, interruption, reconnect, identity continuity, and clean shutdown

## 3. Canonical Discovery
<!-- specs: mobile-network-session -->

- [ ] 3.1 Add failing Rust tests for canonical delivery-announce decoding, destination-keyed upsert, freshness, and generation-scoped snapshots and events
- [ ] 3.2 Expose typed peer aspect, destination, name, observed time, age, and source through the mobile boundary
- [ ] 3.3 Add failing Rust reducer and Dioxus component tests for repeated announces, stale generation events, empty live directories, and local announce outcomes
- [ ] 3.4 Implement event-driven People and Network updates without duplicate peers or remote-reception claims

## 4. Durable Text Messaging
<!-- specs: mobile-messaging -->

- [ ] 4.1 Add failing embedded-runtime tests from mobile send request through canonical persistence, explicit method selection, attempt correlation, inbound persistence, unread state, and restart restoration
- [ ] 4.2 Extend the typed Rust mobile session for drafts, requested and actual method, attempt, propagation upload, receipt evidence, retry, and typed failure
- [ ] 4.3 Add failing Rust reducer and Dioxus component tests for draft revision races, empty live state, queued versus delivered rendering, inbound unread behavior, duplicate evidence, retry, and restart restoration
- [ ] 4.4 Implement shared Dioxus conversation, composer, history, retry, and delivery-detail components using backend-owned state
- [ ] 4.5 Add a deterministic two-identity text round trip proving one canonical outbound record, one inbound record, and exact correlation

## 5. Standard Propagation Client
<!-- specs: mobile-propagation-client -->

- [ ] 5.1 Add failing Rust mobile tests for selected-node persistence, active-metadata validation, explicit propagated upload, identified inventory, durable-before-ack retrieval, duplicate suppression, and partial failure
- [ ] 5.2 Expose standard propagation discovery, selection, readiness, upload, manual sync, automatic-sync policy, progress, and terminal observations through the typed Rust mobile session
- [ ] 5.3 Add failing single-flight scheduler tests for initial connection, reconnect, foreground opportunity, cooldown, deadline, cancellation, and shutdown
- [ ] 5.4 Add failing Rust reducer and Dioxus component tests for selection persistence, stale node metadata, manual sync, automatic sync disclosure, upload versus delivery, progress, repeat sync, and recoverable failure
- [ ] 5.5 Implement client-only propagation controls without hosting, peering, capacity, or expiry administration
- [ ] 5.6 Add local Python/Rust tests for upload, byte-identical replay, offline retrieval, acknowledgement, repeat sync, and daemon restart persistence

## 6. Platform And RNode Coexistence
<!-- specs: mobile-network-session, mobile-release-evidence -->

- [ ] 6.1 Add failing Rust platform-service and Dioxus tests proving TCP remains operational when Bluetooth or USB is unavailable, denied, interrupted, or unverified
- [ ] 6.2 Implement Rust-owned platform services and present TCP, Bluetooth RNode, and Android USB as independent backend-confirmed bearer states
- [ ] 6.3 Complete applicable `stabilize-mobile-platform-hosts` channel-detachment, approved-device, fragmentation, serialization, and retention gates before enabling an RNode support claim
- [ ] 6.4 Preserve explicit Android USB fallback and prevent it from preempting an approved Bluetooth bearer

## 7. Cross-Platform Acceptance
<!-- specs: mobile-release-evidence -->

- [ ] 7.1 Run the shared state corpus through Rust reducer and Dioxus component tests for iOS and Android target classes
- [ ] 7.2 Run iOS Simulator and Android emulator cold-launch, endpoint failure, reconnect, discovery, conversation, retry, and propagation-state scenarios
- [ ] 7.3 Run public-Brutus discovery, direct text, propagated upload, offline retrieval, acknowledgement, repeat sync, and hub-restart scenarios with bounded evidence
- [ ] 7.4 Run applicable physical iOS and Android TCP lifecycle scenarios and retain explicit gaps for unavailable hardware or background execution
- [ ] 7.5 Run physical RNode scenarios only for platform claims that include RNode support
- [ ] 7.6 Record exact application, backend, hub, platform, OS, endpoint class, bearer, correlation, deadline, and outcome for every live gate

## 8. Release Verification
<!-- specs: mobile-network-session, mobile-messaging, mobile-propagation-client, mobile-release-evidence -->

- [ ] 8.1 Run Rust formatting, warning-denied Clippy, focused mobile and propagation tests, fixture validation, and migration or restart checks
- [ ] 8.2 Build, install, cold-launch, and inspect fatal logs for the Dioxus iOS and Android release candidates
- [ ] 8.3 Verify existing identity, messages, contacts, drafts, endpoint, and new propagation selection survive upgrade
- [ ] 8.4 Publish capability claims from passing evidence and list RNode, attachment, Paper, NomadNet, propagation-host, capacity, and expiry exclusions explicitly
