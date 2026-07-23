---
id: cross-network-delivery-reliability
title: "Cross-Network Delivery Reliability Matrix"
status: exploring
tags: [mesh, delivery, lxmf, routing, docker, k3s, network-isolation, fault-injection]
open_questions:
  - "[assumption] A proxy-mediated TCP impairment model is representative enough for the first latency, loss, partition, and recovery gates before bearer-specific testing."
  - "What measured recovery-time bound should the 30-second partition and hub-restart scenarios enforce after an instrumented recovery run?"
dependencies: []
related:
  - constrained-communicator-constraints
---

# Cross-Network Delivery Reliability Matrix

## Overview

Make Styrene cross-network LXMF delivery deterministic and diagnosable across isolated Docker and Kubernetes network zones. Establish a focused baseline matrix, observable route-readiness gates, per-hop correlation evidence, impairment profiles, partition/recovery behavior, and explicit release thresholds. Diagnose T12 at the first missing transport stage rather than treating a final timeout as sufficient evidence.

## Decisions

### Use layered deterministic test tiers

**Status:** accepted

**Rationale:** Start with a focused two-zone Docker topology, then introduce proxy-mediated impairments, then apply the same scenario contract to policy-enforced K3s namespaces. This separates transport defects from orchestration and CNI behavior.

### Require observable route readiness before delivery

**Status:** accepted

**Rationale:** Peer discovery and fixed sleeps are insufficient. A delivery test begins only after the sender reports a usable path to the destination, or records path discovery as the explicit stage under test.

### Correlate delivery across every transport stage

**Status:** accepted

**Rationale:** Each message carries a unique run/scenario correlation. Evidence records CLI acceptance, destination route, selected interface, link establishment, hub ingress/forwarding, receiver ingress/dispatch, durable insertion, and receipt or named terminal failure.

### Adopt quantitative cross-network release gate

**Status:** accepted

**Rationale:** Require 100 consecutive messages in both directions without duplicates, explicit ordering semantics, bounded recovery after 30-second partition and hub restart, non-silent handling during partitions, successful operation under 5% loss and 500 ms latency, and last-successful-stage evidence for every failure.

### Define baseline ordering as deterministic receiver insertion order

**Status:** accepted

**Rationale:** The baseline gate claims deterministic newest-first receiver insertion order for messages returned by the daemon. When timestamps tie, SQLite `rowid DESC` is the stable secondary key. This is verified with 100 equal-timestamp records and by each live 100-message directional batch. Ordering across reconnects and impairment remains a separate scenario to verify before extending the claim.

## Progress Milestone — 2026-07-23

The deterministic baseline is implemented and verified on Brutus at exact commit `00930fad8c3b1015de28206e8740a0ff15d854d1`.

### Measured evidence

- Clean run: `target/mesh-scenarios/verify-batch-00930fad-20260723T122712Z`.
- Run result: passed; operator exit `0`; resilience exit `0`.
- Broad matrix: 35 passed, 0 failed.
- Alpha → Gamma: 100 sends accepted; 100 durable inserts; 0 missing; 0 duplicates; 0 ordering failures; 3 seconds elapsed.
- Gamma → Alpha: 100 sends accepted; 100 durable inserts; 0 missing; 0 duplicates; 0 ordering failures; 3 seconds elapsed.
- Route evidence: both directions reported `found=true`, two hops, and a selected interface before sending.
- Recovery evidence: T24 passed after hub recreation under the existing 60-second scenario deadline.

### Defects closed on the path to this gate

- TCP clients now reconnect when either stream half terminates (`a861e7bc`).
- Half-open TCP clients are detected (`6f0e6a8f`).
- Keepalive detection is nominally bounded to 25 seconds (`55c0617a`).
- Routed control packets no longer enter generic rebroadcast loops (`527b63ff`).
- Equal-timestamp message ordering is deterministic (`00930fad`).

### Scope boundary

This milestone closes the healthy-network deterministic baseline and proves restart recovery within the current 60-second deadline. It does **not** yet prove the remaining release-gate clauses for a controlled 30-second partition, 5% packet loss, 500 ms latency, non-silent partition handling, or an observed recovery duration. Those are the next impairment slice, not evidence already established by this run.

## Open Questions

- [assumption] A proxy-mediated TCP impairment model is representative enough for the first latency, loss, partition, and recovery gates before bearer-specific testing.
- What measured recovery-time bound should the 30-second partition and hub-restart scenarios enforce after an instrumented recovery run?

## Implementation Notes

### File Scope

- `tests/mesh/run_cross_network.sh` — focused entry point; defaults the quantitative batch to 100 messages per direction.
- `tests/mesh/scenarios/08_cross_network.sh` — route readiness, correlation, durable-delivery, duplicate, and ordering assertions.
- `tests/mesh/harness.sh` — bounded observable readiness and message polling helpers.
- `tests/mesh/run_k3s_scenarios.sh` — clean Brutus orchestration and artifact collection.
- `crates/apps/styrened/src/storage/messages.rs` — deterministic newest-first ordering with `rowid` tie-break.
- `crates/libs/styrene-rns/src/transport` — reconnect, keepalive, and directed-control forwarding repairs.
- `tests/mesh/compose.yaml` — planned proxy-mediated impairment topology; not implemented in this milestone.

### Constraints

- No fixed timing sleeps for route readiness; poll observable conditions with bounded deadlines.
- Operator/controller alone owns container or Kubernetes lifecycle and fault injection.
- Every scenario uses a unique correlation identifier and emits a deterministic artifact bundle.
- Do not grant NET_ADMIN to Styrene daemon workloads for the first proxy-mediated impairment slice.
- Preserve existing broad mesh scenarios; introduce a focused runner rather than overloading run_tests.sh.
