# Tasks

## 1. Peer projection

- [x] 1.1 Record the announcing identity, hop count, and interface kind with the persisted node record, migrating existing databases to nullable columns
- [x] 1.2 Resolve the announce's interface hash to its kind in the announce worker and thread the route through the discovery service
- [x] 1.3 Project the identity hash, hops, and interface kind onto `DeviceInfo` and onto `MobilePeer`, defaulting for records written before the change

## 2. Link query

- [x] 2.1 Add `MobilePeerLink` and `MobileNode::peer_link`, reading the transport path table and interface snapshots, with an invalid hash reported as unreachable

## 3. Validation

- [x] 3.1 Cover a received announce yielding a non-empty identity hash, hops, and interface kind in the mobile lib tests
- [x] 3.2 Cover `peer_link` for unknown, malformed, and known destinations
- [x] 3.3 Run format, clippy, `styrened --lib`, `styrene-e2e --no-run`, the offline validation test, and the workspace policy check
