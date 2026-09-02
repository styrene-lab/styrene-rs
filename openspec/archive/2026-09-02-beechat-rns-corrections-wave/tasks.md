# Beechat RNS Corrections Wave Tasks

## 1. Authority And Admission Dependencies
<!-- specs: rns-wire-corrections, interface-failure-policy, embedded-time -->

- [x] 1.1 Depend on `reticulum-1-5-parity-wave` tasks 1.1-1.2 and consume its Reticulum 1.5.1 fixture manifest/corpus at `149e4151095adf098b8f53eab0c03b37169e8559` without creating another top-level pin; Task 1.3 verifies this consumption and must not be a prerequisite
- [x] 1.2 Depend on its tasks 2.1-2.3 for raw Type-1/Type-2 frame admission and received hops, and identify the current-branch insertion point before queueing
- [x] 1.3 Coordinate with its tasks 4.1-4.3 so bounded ingress consumes this change's Type-2 next-hop result without duplicating correction ownership

## 2. LinkRTT Wire Precision
<!-- specs: rns-wire-corrections -->

- [x] 2.1 Add failing link tests using the shared 1.5.1 fixture authority, proving Rust cannot decode canonical MessagePack `f64`, emits the wrong width, accepts an invalid numeric value, or ignores trailing bytes
- [x] 2.2 Change the current-branch LinkRTT codec to encode `f64`, accept only finite non-negative values with complete payload consumption, and preserve internal `Duration` and lifecycle semantics
- [x] 2.3 Run focused fixture/link tests and the existing bidirectional pinned-Python link gate, retaining the shared authority revision and exact assertions (local pinned-Python runs on 2026-09-02: `direct` interop-direct-115-0 and `direct_resource` interop-direct_resource-2834-0 passed against Reticulum `b48b96e6`, LXMF `795fdaa2`)

## 3. Type-2 Admission And Forwarding
<!-- specs: rns-wire-corrections -->

- [x] 3.1 Add failing prequeue tests for mismatched and matching transport IDs across Data, LinkRequest, Proof, and Link packets, plus a deterministic shared-medium LinkRequest loop topology
- [x] 3.2 Enforce the solely owned canonical non-announce next-hop decision at the current-branch Reticulum admission boundary before queues, caches, or mutable routing state, and remove any surviving generic ingress rebroadcast path
- [x] 3.3 Run focused ingress, routing, link-proof, and three-node shared-medium tests, verifying zero state/egress on overhearers and one forwarding action by the designated relay

## 4. Embedded Time Contract
<!-- specs: embedded-time -->

- [x] 4.1 Add failing no-default-feature compile checks and tests for unavailable time, initialized announce timestamps, advancing/refreshed time, and ratchet rotation using the same source
- [x] 4.2 Reconcile current-branch core destination time use with canonical Reticulum 1.5.1 non-panicking embedded-time behavior, removing unconditional `std` dependencies without adding Beechat's Embassy clock
- [x] 4.3 Run the no-default-feature check, focused timestamp/ratchet tests, and standard-feature regression tests

## 5. UDP Broadcast Capability
<!-- specs: interface-failure-policy -->

- [x] 5.1 Add a failing IPv4 UDP socket test proving a configured forwarding socket lacks `SO_BROADCAST`, with receive-only and IPv6 controls
- [x] 5.2 Enable broadcast on IPv4 UDP forwarding sockets before use through the current-branch socket construction path, without adding transport-wide broadcast policy
- [x] 5.3 Run focused UDP bind/send tests and transport interface tests on supported host platforms (loopback socket tests run locally on macOS with `--ignored` on 2026-09-02)

## 6. Disconnected TCP Queue Policy
<!-- specs: interface-failure-policy -->

- [x] 6.1 Add failing barrier-controlled tests for disconnect-before-enqueue, old-writer versus new-stream publication, reconnect versus drain, epoch overflow, and stale/fresh items crossing one connection boundary while a healthy interface remains active
- [x] 6.2 Add the atomic `(online, connection_epoch)` carrier contract, tag accepted items with the checked monotonic epoch, reject or discard epoch mismatches, and preserve truthful per-interface outcomes
- [x] 6.3 Run focused TCP epoch/reconnect/queue race tests and mixed-interface transport tests, verifying bounded dispatch latency, no stale replay, same-epoch fresh delivery, and healthy-interface progress (loopback TCP reconnect test run locally on macOS with `--ignored` on 2026-09-02)

## 7. Final Correction Evidence
<!-- specs: rns-wire-corrections, interface-failure-policy, embedded-time -->

- [x] 7.1 Run formatting, warning-denied Clippy, focused transport tests, ordinary offline workspace validation, and the pinned-Python interoperability gate (2026-09-02 on `354a91cd`: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --no-deps -- -D warnings`; `cargo test -p styrene-rns --features transport,serial` with the loopback UDP/TCP tests run under `--ignored` on macOS; `cargo clippy --lib --no-default-features --no-deps -p styrene-rns -- -D warnings` and `cargo test --no-default-features -p styrene-rns --test embedded_time`; `just test`, the offline guard, workspace policy, and fixture provenance; pinned-Python gates `direct` interop-direct-59883-0, `direct_resource` interop-direct_resource-62546-0, and `opportunistic` interop-opportunistic-64784-0 passed)
- [x] 7.2 Re-check every delta-spec scenario and confirm excluded Beechat behaviors were not reintroduced as tasks or duplicate implementations (diff of the wave against `01153736^` contains no Embassy clock, boot offset, local-destination admission exception, or transport-wide broadcast policy; every scenario in the three delta specs has a test named in its group)
- [x] 7.3 Record validation commands, package/features, exact reference revisions, and any unavailable platform evidence without advancing upstream tracking markers (fixture authority Reticulum 1.5.1 `149e4151095adf098b8f53eab0c03b37169e8559`; live gates Reticulum `b48b96e61676504e0a4e527b33b9a0b4495c6872`, LXMF `795fdaa2b0777c13033787d933d1afc94a2377cb`, NomadNet `ad10301569a39d4f43b3d21ae9fc392602c937ca`; loopback socket evidence exists for macOS only, the Linux and Windows loopback runs remain unavailable; no upstream tracking marker moved)
