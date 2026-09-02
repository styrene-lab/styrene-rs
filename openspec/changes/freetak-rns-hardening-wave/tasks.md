# FreeTAK RNS Hardening Wave Tasks

Every behavior group uses strict test-first ordering. Do not begin its implementation item until its failing regression has been observed, and do not begin its verification item until the minimal implementation passes that regression. A dependency named below is a stop gate: do not add that group's red test or implementation until the predecessor's verification item is complete and its contract is present in the reconciled tree.

## 1. Immutable provenance and dependency admission
<!-- specs: rns-security-hardening, rns-link-transport-hardening, rns-resource-hardening, rns-interface-policy -->

- [x] 1.1 Before any test or implementation work, record the immutable FreeTAKTeam repository URL and range, EPL-2.0/GPL-2.0-or-later evidence role, RNS authority revisions, observation date, applicable evidence commits, and an explicit `no source or fixture copied` admission
- [x] 1.2 Record completion evidence and reconciled-tree contract locations for every required Reticulum, Beechat, and mobile ownership gate in the design table; mark blocked groups as not startable
- [x] 1.3 Verify the admission record uses immutable revisions, changes no refs or tracking markers, and authorizes only independently written Styrene tests and implementation

## 2. Cached Fernet authentication reconciliation
<!-- specs: rns-security-hardening -->

- [x] 2.1 Record that archived `reticulum-1-5-parity-wave` tasks 10.1-10.3 already own and verify constant-time `CachedFernet` authentication; authorize no duplicate implementation or fixture corpus in this wave

## 3. Secure key and ratchet persistence
<!-- specs: rns-security-hardening -->

- [x] 3.1 Add and run failing Unix permission, predictable-temp-symlink, replacement-preservation, temp-cleanup, round-trip, and `StoredKey` debug-redaction tests for file keys, destination ratchets, and transport ratchets
- [x] 3.2 Add one minimal Styrene-owned private atomic-write primitive and route all raw key and ratchet persistence through it without changing persisted encodings
- [x] 3.3 Verify focused persistence tests on Unix and available cross-platform CI, then inspect failure logs and debug output for secret material

## 4. Key-manager fallback classification
<!-- specs: rns-security-hardening -->

- [x] 4.1 Add and run failing table-driven tests for get, put, and list behavior across not-found, backend-unavailable, invalid-argument, decode/integrity, and unclassified primary outcomes
- [x] 4.2 Implement the minimal availability classifier so only missing reads and classified availability failures use the secondary and non-availability writes never do
- [x] 4.3 Verify the focused fallback matrix and existing key-manager round trips, including proof that rejected primary writes leave the secondary unchanged

## 5. Poisoned receipt recovery
<!-- specs: rns-security-hardening -->

- [x] 5.1 Add and run failing tests that poison the receipt mutex and then exercise track, lookup, resolve, and prune while preserving pre-poison entries
- [x] 5.2 Add the minimal shared lock-recovery path that accepts the independent-entry map, clears poison, and resumes all receipt operations
- [x] 5.3 Verify focused poison tests, ordinary receipt correlation, one-time non-secret diagnostics, and direct post-recovery lock acquisition

## 6. Adversarial non-RTT Link state mutation
<!-- specs: rns-link-transport-hardening -->

Dependency gate: `beechat-rns-corrections-wave` group 2 must be verified complete before task 6.1 starts. RTT bytes, precision, parsing, validation, and Python RTT interoperability are excluded from this group.

- [ ] 6.1 Add and run failing adversarial tests for exact identify length and Link binding, every-byte identify corruption, repeated malformed controls, stale liveness under invalid identify/keepalive/close/channel traffic, corrupt channel ciphertext, and verified identity event ordering
- [ ] 6.2 Minimally reorder non-RTT validation before mutation/proof, enforce owned exact framing, and retain verified peer identity without replacing handshake identity
- [ ] 6.3 Verify the non-RTT adversarial matrix, valid controls after hostile traffic, channel proof behavior, teardown cleanup, existing Link interoperability, and no changes to the predecessor's LinkRTT contract

## 7. Bound-interface Link sends
<!-- specs: rns-link-transport-hardening -->

- [x] 7.1 Add and run failing in-memory-interface tests for data and channel sends to inbound, outbound, and all active Links on a non-broadcast transport, including inactive-Link exclusion
- [x] 7.2 Route each Link-context packet through that Link's bound interface with the smallest shared dispatch path and no destination-table lookup
- [x] 7.3 Verify all Link fan-out helpers enqueue on the expected interface, preserve Link destination/context, and do not hold the transport handler during interface dispatch

## 8. Resource retry and round accounting
<!-- specs: rns-resource-hardening -->

- [x] 8.1 Add and run failing deterministic tests proving active fragment progress does not consume retries, one request is emitted per drained round, clean rounds grow the bounded window, and timed-out rounds shrink it
- [x] 8.2 Implement only progress-neutral retry counters and bounded round-transition accounting without changing continuation, admission, MTU, or split behavior
- [x] 8.3 Verify virtual-clock loss, progress, timeout, window floor/ceiling, and terminal retry behavior with no wall-clock sleeps

## 9. Resource continuation and outstanding window
<!-- specs: rns-resource-hardening -->

- [x] 9.1 Add and run failing deterministic tests for outstanding-fragment deduplication, bounded window refill, active-window hashmap exhaustion, one outstanding continuation, lost-continuation expiry, and bounded re-request
- [x] 9.2 Implement the minimal outstanding-fragment and continuation-gate state on top of verified round accounting without changing the Reticulum-owned receive-part search offset
- [x] 9.3 Verify no in-flight fragment is requested twice, continuation loss cannot hang, requests remain bounded, and completion or one timeout failure releases owned request state

## 10. Resource admission caps and effective-MTU sizing
<!-- specs: rns-resource-hardening -->

Dependency gate: `reticulum-1-5-parity-wave` group 7 must be verified complete before task 10.1 starts.

- [ ] 10.1 Add and run failing exact-limit and one-over-limit tests before allocation plus threshold tests proving advertisements, hashmap updates, and fragments consume the predecessor's effective Link/interface MTU
- [ ] 10.2 Implement only pre-allocation size/part caps and effective-MTU resource sizing without adding MTU signaling, negotiation, clamping, or proof policy
- [ ] 10.3 Verify no state/allocation/request for rejected advertisements, every owned resource packet fits the supplied effective MTU, and the predecessor's mixed-interface MTU suite remains green

## 11. Split resource construction, metadata, and terminal cleanup
<!-- specs: rns-resource-hardening -->

Dependency gate: `reticulum-1-5-parity-wave` group 8 must be verified complete before task 11.1 starts.

- [ ] 11.1 Add and run failing tests for first-segment-only eager preparation, outside-lock later construction, byte-exact multi-segment assembly, first-segment-only metadata stripping, segment cancellation, timeout, packet/segment build failure, and assembly mismatch
- [ ] 11.2 Implement only lazy split construction and original-hash split cleanup/events, consuming rather than duplicating generic Link-close cancellation and receive-part offset behavior
- [ ] 11.3 Verify byte-exact assembly, lock exclusion, state-count cleanup, and exactly one terminal split outcome while the predecessor's generic resource regression suite remains green

## 12. Transport worker supervision
<!-- specs: rns-link-transport-hardening -->

- [x] 12.1 Add and run failing tests for a named worker's silent early return, panic, sibling cancellation, attribution, and normal shutdown drain
- [x] 12.2 Retain all long-lived worker handles in the minimal supervisor and cancel/drain the worker set on unexpected completion
- [x] 12.3 Verify focused supervision tests and transport startup/shutdown tests, including no false failure on ordinary cancellation and no surviving sibling tasks

## 13. Passive-node announce bounds
<!-- specs: rns-interface-policy -->

Dependency gate: `reticulum-1-5-parity-wave` groups 2-6 must all be verified complete and their final admission/path architecture must be present before task 13.1 starts.

- [ ] 13.1 Add and run failing tier-size tests for passive, transport-enabled, shared-instance, path-response, rate-limited, cache-refresh, and path-persistence announce cases through the predecessor's final admission/path boundary
- [ ] 13.2 Add only the queue-admission guard and bounded-cache fallback after canonical admission while preserving the newest packet needed by path persistence
- [ ] 13.3 Verify queue/cache bounds, transport retransmission, shared-instance exceptions, path-response suppression, refresh behavior, persisted-path availability, and the predecessor's admission/path suites

## 14. Internal-interface announce policy
<!-- specs: rns-interface-policy -->

Dependency gate: `reticulum-1-5-parity-wave` groups 2-6 must all be verified complete and their final admission/path architecture must be present before task 14.1 starts.

- [ ] 14.1 Add and run a failing authority-derived decision table plus startup, child-inheritance, and hot-apply round-trip tests for both internal flags at the predecessor's final egress-policy boundary
- [ ] 14.2 Implement only `announces_from_internal` and `announces_to_internal` propagation and decisions with authority defaults and no parallel admission, queue, path, or deadline architecture
- [ ] 14.3 Verify the policy was introduced at Reticulum 1.5.0 `e32d4df754a7b87b1bf1bb0d08675d12ff505ae6`, remains authoritative at 1.5.1 `149e4151095adf098b8f53eab0c03b37169e8559`, and preserves predecessor admission/path tests

## 15. Low-level ordered-byte attempt and shared RNode engine
<!-- specs: rns-interface-policy -->

Ownership gate: this group is limited to `styrene-rns`. `shared-dioxus-mobile-ui` group 5 and `deliver-mobile-messaging-minimum` groups 6-7 retain all platform implementation and acceptance work.

- [x] 15.1 Add and run failing fake-attempt tests for ordered reads/writes, empty-read distinction, bearer metadata, negotiated and conservative write caps, startup write gating, cancellation during one open/read/write operation, shutdown-write failure, and idempotent close
- [x] 15.2 Add only the low-level attempt trait and shared RNode/KISS protocol engine; add no platform adapter, reconnect loop, permission flow, application lifecycle, mobile bridge, native handle wrapper, or physical-device path
- [x] 15.3 Verify focused fake-attempt protocol tests and existing serial/KISS behavior, then prove the diff contains no duplicate mobile bridge or platform-host implementation

## 16. Clean-room and aggregate verification
<!-- specs: rns-security-hardening, rns-link-transport-hardening, rns-resource-hardening, rns-interface-policy -->

- [ ] 16.1 Reconcile final implementation evidence with the initial immutable admission record and record any independently derived behavioral clarification without changing the admitted range
- [ ] 16.2 Run `cargo fmt --all -- --check`, `cargo test -p styrene-rns --all-features`, warning-denied all-target/all-feature Clippy for `styrene-rns`, and every required predecessor regression gate
- [ ] 16.3 Validate `freetak-rns-hardening-wave`, confirm existing OpenSpecs, refs, and tracking markers are unchanged, and audit that no evidence-repository source, fixtures, names, comments, or structure entered the diff
