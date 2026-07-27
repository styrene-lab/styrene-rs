# A2A Integration Architecture

Status: proposed
Owners: `styrene-a2a`, `styrene-services`, `styrene-ipc`, transport adapters

## 1. Decision summary

Styrene adopts the Linux Foundation A2A data model through the `a2a-lf` Rust SDK. Styrene does not define a second task, message, artifact, or agent-card protocol.

The integration has four boundaries:

1. **A2A domain facade (`styrene-a2a`)** — official A2A types, Styrene delegation extension, validation, and transport-neutral envelope.
2. **Agent service (`styrene-services`)** — durable task/event state, idempotency, graph reconciliation, and protocol handling.
3. **Daemon interface (`styrene-ipc`)** — typed local operations exposed to frontends; no transport implementation.
4. **Adapters** — local IPC, LXMF/RNS, MQTT, HTTP/JSON-RPC, or future bearers carry the same canonical agent envelope.

The existing local IPC frame remains unchanged:

```text
[LENGTH:4][TYPE:1][REQUEST_ID:16][MSGPACK PAYLOAD]
```

A2A is an application protocol carried inside existing transport boundaries, not a replacement for Styrene IPC, LXMF, MQTT, or RNS.

## 2. Existing architecture and constraints

### Consolidated extension points

- `styrene-services::protocol_registry` already routes inbound LXMF messages by a protocol string to pluggable `ProtocolHandler` implementations.
- `styrene-ipc` is the stable trait/DTO boundary between `styrened` and frontends. It contains no I/O or implementation logic.
- `styrene-ipc-server` preserves Python-compatible one-byte opcodes and MessagePack dictionaries. Unknown opcodes currently fail as `WireError::UnknownType`.
- `styrene-rns` already supports 16-byte request correlation and resource transfer.
- `styrene-a2a` already wraps `a2a-lf`, defines a delegation extension, and provides a CBOR `AgentEnvelope`.

### Compatibility constraints

- Existing opcode values MUST NOT change.
- Existing IPC payloads and Python compatibility MUST remain byte-compatible.
- Existing chat/default protocol behavior MUST remain the fallback for unrecognized LXMF protocol fields.
- A2A messages MUST NOT silently fall through to chat after being recognized as A2A but failing validation.
- Transport addresses, MQTT topics, and LXMF routing metadata MUST NOT enter the signed A2A/domain payload.
- Large artifacts MUST use content/resource references rather than exceed the IPC 4 MiB frame limit or bearer MTU.

## 3. Crate ownership

### `styrene-a2a`

Owns protocol semantics that are portable across daemon and applications:

- re-exported official A2A types;
- A2A protocol-version compatibility policy;
- `StyreneDelegationExtension` under URI `https://styrene.io/a2a/extensions/delegation/v1`;
- stable agent/runtime/root-operation identifiers;
- canonical `AgentEnvelope` CBOR encoding;
- receipt and snapshot DTOs;
- envelope/schema validation;
- authority attenuation validation;
- conversions between A2A JSON objects and envelope payload bytes.

It MUST NOT own:

- sockets, brokers, Reticulum destinations, or daemon storage;
- local IPC opcodes;
- process supervision;
- UI projection models.

### `styrene-services::agent`

Owns daemon-side behavior:

- `AgentService` trait and implementation;
- A2A task/event repository;
- append-only event sequence per `(runtime_id, task_id)`;
- command/message deduplication ledger;
- nested delegation graph index;
- snapshot generation and reconciliation;
- protocol-registry handler for `a2a` and `styrene.a2a.v1`;
- artifact references into `styrene-content`/resource transfer;
- policy checks before dispatching child work.

### `styrene-ipc`

Owns frontend-safe local contracts:

- request DTOs for submit/get/cancel/resubscribe/snapshot;
- task, event, and graph projection DTOs;
- `DaemonAgents` focused trait;
- `DaemonEvent::Agent` variant;
- `StubDaemon` not-implemented behavior.

The composite `Daemon` trait adds `DaemonAgents` only after all application implementations and tests are updated in the same change.

### `styrene-ipc-server`

Owns local frame mapping only:

- four generic request opcodes, not one opcode per A2A method;
- one pushed agent-event opcode;
- MessagePack conversion to/from `styrene-ipc` DTOs;
- subscription topic `Agents`;
- request-ID correlation using the existing 16-byte header.

It MUST NOT decode arbitrary A2A methods in `dispatch.rs`; it calls `DaemonAgents` operations.

## 4. Canonical model

A2A remains authoritative for:

- `AgentCard` and capabilities;
- `Message` and parts;
- `Task`, `TaskStatus`, and `TaskState`;
- artifacts;
- task status/artifact stream events.

Styrene extends A2A only where the standard lacks mesh orchestration provenance:

```text
root_operation_id
parent_task_id?
source { agent_id, runtime_id }
relationship
control_class
remaining_depth
grant_reference?
traceparent?
```

### Identity rules

- `agent_id`: stable logical agent identity, bound to a Styrene identity/signing key.
- `runtime_id`: random UUID per process incarnation.
- A2A `task.id`: execution identity.
- A2A `task.context_id`: conversation/workflow context where applicable.
- `root_operation_id`: all nested descendants of one operator objective.
- `parent_task_id`: immediate causal parent.
- envelope `message_id`: delivery/idempotency identity.

These identifiers MUST NOT substitute for one another.

### Delegation graph invariants

- A task has at most one immediate parent.
- A root task has no parent and creates a root operation ID.
- A child retains its parent's root operation ID.
- Remaining delegation depth strictly decreases.
- Effective authority is a subset of parent authority.
- Cycles are invalid.
- Unknown parents create explicit orphan records until reconciliation; ancestry is never fabricated.
- Runtime restart creates a new runtime ID without changing logical agent ID.

## 5. Agent envelope profile

The Styrene envelope is a bearer-neutral wrapper around serialized A2A messages/events:

```text
profile_version
message_id[16]
kind = command | event | result | receipt | snapshot
source_agent_id
source_runtime_id[16]
target_agent_id
sequence
created_at_ms
expires_at_ms?
payload_schema
a2a_payload
authorization?
traceparent?
```

### Required additions before stabilization

The current prototype needs:

- target runtime ID (optional, for incarnation pinning);
- root operation ID and task ID in the envelope index fields;
- parent task ID where present;
- content type/encoding (`application/a2a+json`, canonical CBOR profile);
- authorization grant digest/reference;
- signature/COSE protected bytes or a clearly defined external-signature input;
- size limits and canonical encoding rules;
- typed receipt and snapshot payloads.

Do not stabilize the current field numbering until these additions are reviewed.

## 6. Generic local IPC family

Reserve a coherent, currently unused opcode range rather than adding A2A methods to the fixed enum individually. Proposed allocation:

| Opcode | Name | Direction | Purpose |
|---|---|---|---|
| `0x76` | `CmdAgentSubmit` | request | Submit an A2A message/envelope |
| `0x77` | `QueryAgentTask` | request | Fetch one task and latest state |
| `0x78` | `CmdAgentCancel` | request | Request cancellation by task ID |
| `0x79` | `QueryAgentGraph` | request | Fetch root-operation graph snapshot |
| `0xC8` | `EventAgent` | push | Task/event/receipt/snapshot notification |

`0x7A..0x7E` remain reserved for future generic agent operations. `0x7F` remains untouched.

This is a proposal, not yet an allocation. Before implementation, update and test both Rust and any surviving Python opcode registries together.

### Why not four wire-specific envelope opcodes?

`AgentEnvelope`, receipt, and snapshot are protocol payload kinds. Local frontend operations are service intents. Keeping those layers separate lets frontends request a graph snapshot without constructing a mesh envelope and lets the daemon receive envelopes from any bearer through one service.

## 7. Service operations

Proposed `DaemonAgents` interface:

```rust
async fn agent_submit(&self, request: AgentSubmitRequest) -> Result<AgentSubmitResponse, IpcError>;
async fn agent_task(&self, request: AgentTaskRequest) -> Result<AgentTaskView, IpcError>;
async fn agent_cancel(&self, request: AgentCancelRequest) -> Result<AgentCancelResponse, IpcError>;
async fn agent_graph(&self, request: AgentGraphRequest) -> Result<AgentGraphSnapshot, IpcError>;
```

Submission response acknowledges daemon acceptance, not remote execution. Remote receipts and A2A task outcomes arrive as `DaemonEvent::Agent`.

Every mutating request carries an idempotency/message ID. Retryable `IpcError` values do not imply whether remote execution occurred; callers reconcile by task/message ID.

## 8. Transport adapters

### LXMF/RNS

- protocol registry keys: `a2a` and `styrene.a2a.v1`;
- body contains encoded `AgentEnvelope` or a resource reference;
- recognized invalid envelopes return a protocol error and never become chat;
- envelope expiry is enforced before execution;
- RNS delivery receipt is bearer evidence only;
- large artifacts use resource transfer/content references.

### MQTT 5

- adapter-owned topics; no topics in signed payload;
- QoS 1 default;
- Correlation Data maps envelope message ID;
- Message Expiry maps envelope expiry;
- Response Topic is routing metadata;
- retained messages are allowed only for latest state/snapshot, never commands;
- broker ACL identity maps to permitted target agents.

### HTTP/JSON-RPC

Use the official A2A SDK's standard HTTP/JSON-RPC shapes at the edge. Convert immediately to the internal service/envelope model. HTTP URLs remain in Agent Cards and adapter configuration, not mesh payloads.

## 9. Delivery, ordering, and reconciliation

Guarantee:

> At-least-once bearer delivery with idempotent application effects within ledger retention.

Do not claim global ordering or exactly-once execution.

- `message_id` deduplicates one command/event.
- `sequence` is monotonic per source runtime and task stream.
- gaps trigger snapshot reconciliation.
- receipts distinguish bearer acceptance, daemon acceptance/deduplication, and A2A outcome.
- snapshots include runtime incarnation, active/recent tasks, graph edges, effective grants, and sequence watermarks.
- process restart invalidates runtime-local sequence assumptions and requires a snapshot.

## 10. Security model

- Bind `agent_id` to `styrene-identity`; do not trust self-declared IDs alone.
- Verify authentication before deduplication disclosure.
- Sign canonical envelope bytes or protect them with COSE; transport TLS/broker auth is not sufficient for store-and-forward.
- Enforce target, expiry, size, schema, and authority before recording acceptance.
- Authority grants attenuate tools, path scope, network, token/time budgets, and delegation depth.
- A receipt proves acceptance by one runtime incarnation, not successful task completion.
- Cancellation is hop-by-hop; ancestor acknowledgement does not prove descendant termination.

## 11. Compatibility and rollout

### Capability discovery

Agent Cards advertise the Styrene delegation extension. Local daemon status advertises:

```text
agent_protocols: ["a2a/1.0"]
agent_extensions: ["https://styrene.io/a2a/extensions/delegation/v1"]
agent_ipc_version: 1
```

### Mixed-version behavior

- Old IPC clients never emit new opcodes and remain unaffected.
- New clients encountering an old daemon receive unknown-opcode/unsupported behavior and must hide agent controls.
- Old mesh peers ignore an unregistered `styrene.a2a.v1` protocol or process it through explicit unsupported-protocol handling; it must not render as chat.
- Unknown optional A2A extensions are preserved where possible and ignored according to A2A rules.
- Unknown required extensions reject the request before execution.

## 12. Rejected alternatives

### Invent a Styrene-native agent/task protocol
Rejected: duplicates A2A discovery, task state, streaming, messages, and artifacts.

### Put A2A methods directly into local IPC opcodes
Rejected: couples a stable Python-compatible daemon wire to protocol method churn.

### Treat LXMF or MQTT as the agent protocol
Rejected: bearer metadata and delivery semantics would leak into domain identity and signatures.

### Reuse chat DTOs for agents
Rejected: loses task lifecycle, artifacts, delegation provenance, and cancellation semantics.

### Encode complete artifacts in every event
Rejected: violates constrained-bearer and 4 MiB IPC boundaries.

## 13. Readiness gates

No production adapter ships until:

- envelope profile fields and canonical signature input are frozen;
- authority attenuation has positive and negative tests;
- duplicate command execution is prevented;
- reconnect snapshot reconciliation is tested;
- mixed-version IPC compatibility fixtures pass;
- invalid A2A traffic cannot fall through to chat;
- task cancellation reports each descendant independently;
- resource references cover oversized artifacts;
- Agent Card extension negotiation is tested against the official SDK.
