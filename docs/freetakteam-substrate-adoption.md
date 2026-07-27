# FreeTAKTeam Substrate Adoption

## Purpose

Bring Styrene's RNS/LXMF substrate up to the current FreeTAKTeam implementation while preserving Styrene's independent product, service, IPC, and UI architecture.

FreeTAKTeam is the behavioral reference implementation for this effort. We port protocol semantics, state transitions, limits, and test scenarios; we do not merge its repository structure or rename Styrene around its current crate graph.

## Pinned reference

- Repository: `FreeTAKTeam/LXMF-rs`
- Reference release: `v0.9.0`
- Release commit: `ce6f7402c2c4ed645956f35062115412bcb87002`
- Tracking baseline before this assessment: `3a2d46bb`
- Current branch head observed during assessment: `0859680c`
- Scale since the old baseline: 677 commits

The release tag is the acceptance baseline. Post-release `main` commits are assessed separately and must not silently enter a release-parity slice.

## Adoption rule

For every slice:

1. Cite the upstream commits and scenarios.
2. Preserve upstream behavior and ordering unless a documented Styrene invariant conflicts.
3. Translate integration points into Styrene's existing crates rather than importing FreeTAKTeam's daemon/product topology.
4. Add focused tests before or with the port.
5. Run the relevant Styrene unit, E2E, and Python-interop gates.
6. Record intentional divergence explicitly.

## Ranked slices

### S0 — Import the evidence framework

**Source:** FreeTAKTeam parity matrices, software parity ledger, pinned Python-reference inventories, and interop manifests.

**Styrene target:** a machine-readable compatibility ledger covering RNS transport, LXMF delivery, propagation, interfaces, and external evidence.

**Why first:** prevents feature-count archaeology and gives every subsequent port a precise acceptance target.

### S1 — Idempotent inbound delivery and drop observability

**Primary upstream commits:**

- `d6c56c25` — Emit direct duplicate inbound drop events
- `f199a21c` — Emit direct duplicate delivery drop events
- `06e873e4` — Expose propagated destination mismatch drop event
- `ff4d4f3a` — Enforce direct delivery resource limit
- `9fd9232a` — Emit paper duplicate drop events

**Styrene surfaces:**

- `crates/apps/styrened/src/storage/messages.rs`
- `crates/apps/styrened/src/services/messaging.rs`
- `crates/apps/styrened/src/workers/inbound.rs`
- `crates/apps/styrened/src/services/events.rs`
- propagation and mobile import paths
- E2E delivery tests

**Acceptance:** duplicate packet/resource/import delivery stores and dispatches once, never auto-replies twice, and emits a structured drop outcome; malformed, oversized, and destination-mismatched payloads are distinguishable.

### S2 — Direct backchannel and delivery lifecycle

**Primary upstream commits:**

- `f0c8ed0d` — Add LXMF direct backchannel reuse
- `08e2f2dc` — Fix bidirectional LXMF direct backchannels
- `0132647f` — Model receipt delivery states
- `eb0b9d1c` — Cover opportunistic packet receipt metadata
- `fd896ac1` — Expose LXMF delivery trace envelope

**Acceptance:** established links are reused in both directions; receipt states and delivery traces are stable and observable; retries do not create duplicate side effects.

### S3 — Link liveness, channel, and resource parity

**Primary upstream commits:**

- `b27fb7da` — Implement Reticulum link liveness parity
- `204b3d21` — Auto-validate `LinkIdentify`
- `8c1002d1` — Close RNS channel parity
- `0169edac` — LXMF resource repro and routed transfer fixes

**Acceptance:** link teardown/retry, identify validation, channel sequencing, resource transfer, and Python channel/resource interop match the pinned reference scenarios.

### S4 — Path and routing policy

**Representative upstream work:** path-response ordering, request deduplication/scoping, random-blob freshness, link-request MTU signaling, path persistence/flush, blackholed identities, and next-hop metadata.

**Named commits include:**

- `aa0839fd` — Flush Reticulum path table on shutdown
- `ac7e88ab` — Add Reticulum blackholed identity RPCs

**Acceptance:** port the scenario set from FreeTAKTeam's `RNS-TRANSPORT-POLICY` ledger row, not just individual patches.

### S5 — Propagation lifecycle

**Source:** the large post-baseline propagation sequence: peer state, offer/import validation, transfer limits, backoff, stamp policy, duplicate accounting, queue lifecycle, selected-node behavior, ACLs, pruning, and observability.

**Acceptance:** treat as a subsystem migration with storage schema and lifecycle tests. Do not cherry-pick the hundreds of accounting commits individually.

### S6 — Runtime and interfaces

Order by Styrene deployment value:

1. TCP/Local/UDP lifecycle and hot apply
2. AutoInterface correctness (`bd380f62` and related scenarios)
3. I2P
4. KISS/RNode/serial
5. Meshtastic
6. BLE/mobile

Hardware claims remain separate from software parity.

## Explicit non-goals

- No bulk merge of `freetakteam/main`.
- No wholesale import of `reticulumd`, `rns-rpc`, ZeroMQ SDK, or FreeTAKTeam release tooling.
- No claim of v0.9.0 parity until every adopted/unsupported row has evidence.
- No conflation of software tests with hardware or external-client certification.
- No replacement of Styrene IPC, identity, service, TUI, or runtime-profile architecture merely to reduce diff size.

## Immediate implementation target

Start with S1. It is bounded, security/correctness relevant, and a prerequisite for safely adopting link reuse, receipts, and propagation behavior.
