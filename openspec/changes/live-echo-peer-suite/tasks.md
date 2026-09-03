# Tasks

## 1. Suite

- [x] 1.1 Add the opt-in `live_echo_peer` e2e test with environment-gated target, staged evidence, and repeated path requests
- [x] 1.2 Add the `test-live-peer` recipe and the runbook in `docs/live-echo-peer.md`

## 2. Corrections

- [x] 2.1 Persist projected LXMF fields in both canonical inbound inserts, with storage and decode round-trip tests
- [x] 2.2 Record the link's actual representation instead of failing the send, and update the outbound route, with a storage test

## 3. Evidence

- [x] 3.1 Run the suite against the Raspberry Pi echo peer on a current daemon build. Every stage passed. Packet and resource round trips were near 230 ms. The path moved to the reattached interface in about four seconds.
- [x] 3.2 Run fmt, clippy, the daemon unit tests, the offline validation guard, the workspace policy, and fixture provenance
