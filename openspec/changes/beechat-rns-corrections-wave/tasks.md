# Beechat RNS Corrections Wave Tasks

## 1. Authority And Admission Dependencies
<!-- specs: rns-wire-corrections, interface-failure-policy, embedded-time -->

- [ ] 1.1 Depend on `reticulum-1-5-parity-wave` tasks 1.1-1.3 and consume its Reticulum 1.5.1 fixture manifest/corpus at `149e4151095adf098b8f53eab0c03b37169e8559` without creating another top-level pin
- [ ] 1.2 Depend on its tasks 2.1-2.3 for raw Type-1/Type-2 frame admission and received hops, and identify the current-branch insertion point before queueing
- [ ] 1.3 Coordinate with its tasks 4.1-4.3 so bounded ingress consumes this change's Type-2 next-hop result without duplicating correction ownership

## 2. LinkRTT Wire Precision
<!-- specs: rns-wire-corrections -->

- [ ] 2.1 Add failing link tests using the shared 1.5.1 fixture authority, proving Rust cannot decode canonical MessagePack `f64`, emits the wrong width, accepts an invalid numeric value, or ignores trailing bytes
- [ ] 2.2 Change the current-branch LinkRTT codec to encode `f64`, accept only finite non-negative values with complete payload consumption, and preserve internal `Duration` and lifecycle semantics
- [ ] 2.3 Run focused fixture/link tests and the existing bidirectional pinned-Python link gate, retaining the shared authority revision and exact assertions

## 3. Type-2 Admission And Forwarding
<!-- specs: rns-wire-corrections -->

- [ ] 3.1 Add failing prequeue tests for mismatched and matching transport IDs across Data, LinkRequest, Proof, and Link packets, plus a deterministic shared-medium LinkRequest loop topology
- [ ] 3.2 Enforce the solely owned canonical non-announce next-hop decision at the current-branch Reticulum admission boundary before queues, caches, or mutable routing state, and remove any surviving generic ingress rebroadcast path
- [ ] 3.3 Run focused ingress, routing, link-proof, and three-node shared-medium tests, verifying zero state/egress on overhearers and one forwarding action by the designated relay

## 4. Embedded Time Contract
<!-- specs: embedded-time -->

- [ ] 4.1 Add failing no-default-feature compile checks and tests for unavailable time, initialized announce timestamps, advancing/refreshed time, and ratchet rotation using the same source
- [ ] 4.2 Reconcile current-branch core destination time use with canonical Reticulum 1.5.1 non-panicking embedded-time behavior, removing unconditional `std` dependencies without adding Beechat's Embassy clock
- [ ] 4.3 Run the no-default-feature check, focused timestamp/ratchet tests, and standard-feature regression tests

## 5. UDP Broadcast Capability
<!-- specs: interface-failure-policy -->

- [ ] 5.1 Add a failing IPv4 UDP socket test proving a configured forwarding socket lacks `SO_BROADCAST`, with receive-only and IPv6 controls
- [ ] 5.2 Enable broadcast on IPv4 UDP forwarding sockets before use through the current-branch socket construction path, without adding transport-wide broadcast policy
- [ ] 5.3 Run focused UDP bind/send tests and transport interface tests on supported host platforms

## 6. Disconnected TCP Queue Policy
<!-- specs: interface-failure-policy -->

- [ ] 6.1 Add failing barrier-controlled tests for disconnect-before-enqueue, old-writer versus new-stream publication, reconnect versus drain, epoch overflow, and stale/fresh items crossing one connection boundary while a healthy interface remains active
- [ ] 6.2 Add the atomic `(online, connection_epoch)` carrier contract, tag accepted items with the checked monotonic epoch, reject or discard epoch mismatches, and preserve truthful per-interface outcomes
- [ ] 6.3 Run focused TCP epoch/reconnect/queue race tests and mixed-interface transport tests, verifying bounded dispatch latency, no stale replay, same-epoch fresh delivery, and healthy-interface progress

## 7. Final Correction Evidence
<!-- specs: rns-wire-corrections, interface-failure-policy, embedded-time -->

- [ ] 7.1 Run formatting, warning-denied Clippy, focused transport tests, ordinary offline workspace validation, and the pinned-Python interoperability gate
- [ ] 7.2 Re-check every delta-spec scenario and confirm excluded Beechat behaviors were not reintroduced as tasks or duplicate implementations
- [ ] 7.3 Record validation commands, package/features, exact reference revisions, and any unavailable platform evidence without advancing upstream tracking markers
