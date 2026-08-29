# Reticulum 1.5 Parity Wave Tasks

## 1. Shared Versioned RNS Fixture Authority
<!-- specs: parity-evidence -->

- [x] 1.1 Add failing schema/provenance tests requiring `tests/interop/fixtures/rns/index-v2.json`, schema version 2, separate `rns-1.4.2` and `rns-1.5.1` authority records, typed vector expectations, source symbols, and artifact SHA-256 values while asserting every existing 1.4.2 ID, path, byte sequence, and checksum is unchanged
- [x] 1.2 Minimally add the shared additive v2 index and bounded 1.5.1 vectors, retain `provenance-v1.toml` readability, and expose one fixture-loader contract for `beechat-rns-corrections-wave`, `freetak-rns-hardening-wave`, and `leviculum-rns-corpus-wave` consumers without vendoring Python or mutable revisions
- [x] 1.3 Verify both authority revisions and all checksums offline, prove old 1.4.2 fixture consumers still pass, and prove the three consumer waves can reference the v2 authority IDs without defining another canonical RNS schema

## 2. Packet Admission And Received Hops
<!-- specs: packet-admission-routing -->

- [x] 2.1 Add failing canonical/adversarial tests in `styrene-rns` for short Type 1/Type 2 frames, incomplete IDs, zero data, oversized payload/frame, wire hops 127/128/255, outbound hop limits, and physical versus local ingress
- [x] 2.2 Minimally make `Packet::from_bytes` and interface admission reject rather than truncate, increment physical ingress exactly once with checked semantics, and carry canonical post-ingress hops through routing and `ReceivedData`
- [x] 2.3 Run focused packet, interface, three-node route, and canonical fixture tests and retain an acceptance matrix for every malformed vector and hop boundary

## 3. Violations And Policy Drops
<!-- specs: packet-admission-routing, link-resource-security -->

- [x] 3.1 Add failing adversarial tests distinguishing malformed frames, IFAC failures, invalid announces, pre-validation link traffic, excessive path-request tags, and valid blackholed announces
- [x] 3.2 Minimally add typed per-interface violation/filter counters and preserve blackholed traffic as a policy drop rather than a protocol-invalid event
- [x] 3.3 Verify each rejection is side-effect free except for its exact counter and that malformed input cannot stop the ingress worker

## 4. Strict-Priority Bounded Ingress
<!-- specs: bounded-ingress -->

- [x] 4.1 Add failing deterministic tests for FIFO order within each class, strict `data > announce > path request > ingress limited` selection, indefinite lower-class starvation under sustained higher-class load, immediate lower-class progress after higher classes empty, capacities `1024/128/128/8`, and per-class overflow drops
- [x] 4.2 Minimally replace the single transport FIFO with configurable bounded class queues that restart selection at data after every dequeue and expose one lock-consistent unsigned-integer snapshot of `capacity`, `depth`, and monotonic `dropped` per class
- [x] 4.3 Verify sustained concurrent producers cannot exceed class capacities, no fairness/aging silently changes canonical starvation, FIFO resumes correctly after starvation, and all occupancy/drop assertions run offline

## 5. Path-Request Batching And Canonical Limits
<!-- specs: packet-admission-routing, bounded-ingress -->

- [x] 5.1 Add failing differential/adversarial boundary tests for tag entries 16,000 and the crossing entry, duplicates in current/previous generations, generation aging, one 120-second in-flight gate per destination, duplicate waiters from one interface, waiter count equal to registered-interface count, detached waiters, pending discovery entries 32/33, ingress limiting, and a late egress-state change
- [x] 5.2 Minimally separate two-generation tag replay state from destination-keyed in-flight discovery, rotate tags by count rather than time, deduplicate waiters by registered interface, expire gates after 120 seconds, refuse the 33rd pending discovery enqueue observably, answer each eligible interface once, and enforce pre-send plus late egress checks
- [x] 5.3 Verify one recursive request per destination, exact cleanup on answer/timeout/local resolution/detach, duplicate rejection across both tag generations, acceptance only after the containing generation ages out, and no invented fixed total in-flight or waiter limit beyond canonical lifetime and registered-interface bounds

## 6. Online Bitrate Metadata And Deadlines
<!-- specs: packet-admission-routing -->

- [ ] 6.1 Add failing virtual-clock differential tests for runtime online positive bitrate metadata, exclusion of offline/missing/zero bitrates, slowest-online selection, path-discovery deadlines, and outbound-interface link-proof deadlines
- [ ] 6.2 Minimally add the online positive bitrate runtime contract and derive every bitrate-based discovery/proof deadline from it with a finite configured fallback; do not add hardware MTU, MTU signaling, clamping, proof encoding/validation, or payload-limit behavior in this task
- [ ] 6.3 Verify exact deadline boundaries and metadata transitions with no division-by-zero, overflow, wall-clock sleep, stale-interface influence, or dependency on Task 7 hardware MTU state

## 7. Link MTU Signaling, Proofs, And Payloads
<!-- specs: link-resource-security -->

- [ ] 7.1 After Task 6, add failing canonical link request/proof fixtures for discovery enabled/disabled, unsupported hops, mixed hardware MTUs, route-minimum clamping, authenticated mode/MTU proofs, and negotiated packet/channel/resource payload limits
- [ ] 7.2 Minimally consume Task 6 bitrate metadata while adding optional positive hardware MTU metadata, MTU policy, request signaling, route clamping, proof construction/validation, and confirmed-MTU payload derivation; do not redefine bitrate metadata or deadline formulas
- [ ] 7.3 Verify packet/proof bytes against 1.5.1 fixtures and run focused link, channel, resource, and mixed-interface route tests at threshold MTUs while proving Task 6 bitrate/deadline tests remain unchanged

## 8. Link And Resource Regressions
<!-- specs: link-resource-security -->

- [ ] 8.1 Add failing adversarial tests for closing a link with multiple inbound/outbound resources, out-of-order parts at a requested-window boundary, cancellation races, receive-handler failure, and terminal-state idempotence
- [ ] 8.2 Minimally snapshot correlated transfers before cancellation, align part lookup to `consecutive_completed_height + 1`, and guarantee receive failure releases guards while preserving one terminal outcome
- [ ] 8.3 Verify every transfer terminates exactly once, no part is skipped or misindexed, no watchdog/scheduler remains blocked, and existing broad resource tests still pass

## 9. Receipt Callback Reentrancy
<!-- specs: link-resource-security -->

- [ ] 9.1 Add failing reentrant receipt tests in which a validated proof callback synchronously sends another packet, plus duplicate-proof and concurrent-expiry adversarial cases
- [ ] 9.2 Minimally select receipt candidates and mutate collection state under synchronization, then release synchronization before proof validation side effects and callback invocation
- [ ] 9.3 Verify callback send completes within a deterministic timeout, the original receipt transitions exactly once, duplicate/expiry races remain terminally idempotent, and no receipt collection lock is held during callbacks

## 10. Canonical Token Authentication
<!-- specs: link-resource-security -->

- [ ] 10.1 Add failing 1.5.1 valid, invalid, and truncated token-tag fixtures plus a regression guard that detects ordinary equality comparison in every token HMAC verification implementation path
- [ ] 10.2 Minimally retain or restore constant-time HMAC comparison before decryption without changing canonical token bytes or accepted key modes
- [ ] 10.3 Verify canonical crypto vectors, no plaintext on authentication failure, all feature-specific token implementations, and warning-denied focused tests

## 11. Interface Discovery Metadata
<!-- specs: interface-discovery -->

- [ ] 11.1 Add failing 1.5.1 MessagePack fixtures for required implementation/version fields, a valid 16-byte operator LXMF address, absent optional address, wrong types/lengths, and sanitized names
- [ ] 11.2 Minimally implement the `rnstransport.discovery.interface` metadata codec and truthful runtime observations without automatic connection or a second discovery protocol
- [ ] 11.3 Verify byte/semantic fixture agreement, fail-closed decode without partial persistence, and compatibility with discovery records that omit the new optional operator field

## 12. Offline And Live Gates
<!-- specs: parity-evidence, packet-admission-routing, bounded-ingress, link-resource-security, interface-discovery -->

- [ ] 12.1 Add a failing validation-policy test that detects network access, Python launch, hardware assumptions, mutable revisions, competing RNS fixture schemas, missing evidence checksums, or unsupported claim promotion in normal tests
- [ ] 12.2 Minimally register focused offline suites, then package live routed/MTU/discovery scenarios for handoff to the existing interop runner owners in `reticulum-lxmf-nomadnet-parity` tasks 4.7, 5.7, 8.8, and 12.6 without registering or enabling those live gates here
- [ ] 12.3 Run OpenSpec validation, formatting, warning-denied Clippy, focused unit/component/fixture/adversarial tests offline, then retain bounded live evidence only in the dedicated revision-pinned gate
