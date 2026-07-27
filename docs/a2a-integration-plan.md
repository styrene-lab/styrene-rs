# A2A Integration Implementation Plan

This plan implements `docs/a2a-integration-architecture.md`. It is ordered to keep every merged step compiling and to preserve old IPC clients.

## Phase 0 — lifecycle and compatibility fixtures

- [ ] Create an OpenSpec change for A2A integration, binding this architecture document.
- [ ] Capture current Rust IPC opcode/classification fixtures and Python compatibility fixtures.
- [ ] Add a workspace validation target covering `styrene-a2a`, `styrene-ipc`, `styrene-ipc-server`, `styrene-services`, and `styrened`.
- [ ] Document supported A2A protocol/SDK versions and update policy.

Exit: baseline tests prove no pre-A2A wire changes.

## Phase 1 — stabilize `styrene-a2a`

- [ ] Replace prototype string IDs with validated, documented identity formats.
- [ ] Add target runtime, root operation, task, and parent-task index fields to `AgentEnvelope`.
- [ ] Define payload content type and canonical JSON/CBOR conversion rules.
- [ ] Define canonical signing input and COSE/external signature representation using `styrene-identity`.
- [ ] Add typed acceptance receipt, protocol error, snapshot request, and graph snapshot DTOs.
- [ ] Add explicit size, expiry, sequence, cycle, and schema validation errors.
- [ ] Represent authority grants directly or through a cryptographically bound digest/reference.
- [ ] Add Agent Card extension construction and required/optional negotiation helpers.
- [ ] Add official-SDK round-trip fixtures for message, task, status event, artifact event, and Agent Card.

Exit: envelope profile v1 is frozen; no transport dependency in `styrene-a2a`.

## Phase 2 — service boundary

- [ ] Add `styrene-services::agent` module.
- [ ] Define repository traits for tasks, events, deduplication, delegation edges, and sequence watermarks.
- [ ] Implement in-memory repositories for conformance tests.
- [ ] Implement graph validation: one parent, no cycles, root preservation, depth/authority attenuation.
- [ ] Implement submit/get/cancel/graph/snapshot service operations.
- [ ] Implement idempotency ledger with bounded retention and conflict detection.
- [ ] Implement snapshot reconciliation and orphan-parent handling.
- [ ] Implement artifact reference threshold and content-store abstraction.
- [ ] Add `ProtocolHandler` for `a2a` and `styrene.a2a.v1`; recognized invalid traffic must return an A2A error and never default to chat.

Exit: transport-independent service passes nested delegation, replay, cancellation, and reconciliation tests.

## Phase 3 — local daemon interface

- [ ] Add `styrene-a2a` dependency to `styrene-ipc` only if DTO re-export is intentional; otherwise define stable projection DTOs and conversions in `styrened`.
- [ ] Add `DaemonAgents` trait with submit/task/cancel/graph operations.
- [ ] Add request/response/event DTOs with `#[non_exhaustive]` and defaults consistent with existing IPC conventions.
- [ ] Add `DaemonEvent::Agent` and update all exhaustive matches.
- [ ] Implement `StubDaemon` methods as `IpcError::NotImplemented`.
- [ ] Add trait to composite `Daemon` only after all implementations compile.
- [ ] Wire `styrened` facade to `styrene-services::agent`.

Exit: daemon API is usable in-process with no opcode allocation yet.

## Phase 4 — local IPC mapping

- [ ] Confirm proposed opcode availability against Rust and Python registries.
- [ ] Allocate generic request opcodes (`submit`, `task`, `cancel`, `graph`) and one agent event opcode.
- [ ] Extend `from_byte`, `is_request`, `is_event`, exhaustive classification, and discriminant stability tests.
- [ ] Add MessagePack mapping in `dispatch.rs`; avoid protocol-method-specific dispatch arms.
- [ ] Add `SubTopic::Agents` and event push conversion.
- [ ] Add server integration tests for each operation and pushed events.
- [ ] Add old-client/new-server and new-client/old-server fixtures.
- [ ] Update Python constants/client only if Python frontend compatibility remains supported.

Exit: local IPC compatibility suite is green and existing byte fixtures are unchanged.

## Phase 5 — LXMF/RNS adapter

- [ ] Register A2A protocol handler during daemon composition.
- [ ] Map small envelopes to LXMF fields/body without converting binary payloads through lossy UTF-8 strings.
- [ ] Add resource-transfer/content-reference path for large payloads and artifacts.
- [ ] Map RNS delivery receipts to bearer-level evidence only.
- [ ] Enforce expiry before service acceptance.
- [ ] Test offline propagation, duplicate delivery, delayed expiry, corrupt signature, unknown required extension, and large artifact retrieval.

Exit: two Styrene daemons exchange A2A tasks over direct and propagated RNS paths.

## Phase 6 — MQTT 5 adapter

- [ ] Define adapter configuration, topic templates, and principal-to-agent ACL mapping.
- [ ] Map message ID to Correlation Data, expiry to Message Expiry, and response routing to Response Topic.
- [ ] Use QoS 1 by default; retain snapshots/state only, never commands.
- [ ] Reuse the service deduplication ledger.
- [ ] Test reconnect persistent sessions, duplicate QoS delivery, ACL denial, expiry, retained-state recovery, and cross-adapter task reconciliation.

Exit: MQTT and LXMF adapters produce identical service-level outcomes for shared fixtures.

## Phase 7 — observability and frontend projection

- [ ] Emit OpenTelemetry spans using `traceparent`, root operation, task, agent, and runtime attributes.
- [ ] Add daemon status capability advertisement.
- [ ] Add task/graph projection to TUI/Dioxus only after backend conformance gates pass.
- [ ] Display owned/attached/observed/delegated control class and runtime incarnation.
- [ ] Display descendant cancellation as independently acknowledged/confirmed nodes.
- [ ] Ensure large artifacts are lazy-loaded rather than placed in event payloads.

Exit: UI is a projection of the canonical graph and does not invent lifecycle state.

## Cross-cutting test matrix

Each phase adds tests for:

- fresh command and duplicate replay;
- conflicting message-ID replay;
- nested delegate and cleave child;
- depth exhaustion and authority escalation rejection;
- runtime restart and snapshot reconciliation;
- orphan child followed by parent reconciliation;
- cancellation with partially unreachable descendants;
- malformed/expired/oversized envelope;
- unknown optional versus required extension;
- old/new local IPC peers;
- direct, propagated, and brokered delivery;
- artifact above inline threshold;
- signature identity mismatch;
- sequence gap and duplicate event.

## Explicit deferrals

- Exactly-once execution across daemon process crashes.
- Cross-organization global agent naming authority.
- Durable distributed transactions spanning nested agents.
- UI implementation before service and adapter conformance.
- A separate Styrene task/message protocol beyond A2A extensions.
