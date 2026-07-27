+++
title = "Asynchronous Tunnel Operations"
tags = ["architecture","tunnel","async","ipc"]
+++

+++
id = "94107e8c-223b-4224-9589-2b8fb094951d"
kind = "design_node"

[data]
title = "Asynchronous Tunnel Operations"
status = "decided"
issue_type = "architecture"
priority = 1
dependencies = []
open_questions = []
+++

## Overview

# Asynchronous Tunnel Operations

# Asynchronous Tunnel Operations

## Context

Tunnel establishment currently holds an IPC request open while Reticulum path discovery, link activation, and LXMF delivery execute. This couples a local control-plane deadline to an asynchronous mesh data plane. T26 demonstrates the failure: the command reaches the daemon but the client times out before negotiation produces an observable result.

## Decision

Tunnel establish, rekey, and teardown are asynchronous daemon-owned operations.

IPC acknowledges validated intent after an operation has been registered and queued. It does not wait for mesh delivery or remote negotiation. The daemon remains the owner if the submitting client disconnects.

Events announce transitions. Queryable operation state is authoritative for reconnect and reconciliation.

## Operation model

Each attempt has a stable operation ID and peer identity. Only one nonterminal operation of a given kind may exist for a peer.

Initial states:

- `queued`
- `sending_offer`
- `offer_sent`
- `negotiated`
- `configuring`
- `established`
- `degraded`
- `rejected`
- `failed`
- `timed_out`
- `cancelled`

`negotiated` and `established` are distinct. Successful control-plane negotiation does not imply that WireGuard configuration succeeded.

The first implementation may keep operation state in memory. Daemon-restart durability is deferred; client-disconnect durability is required.

## Command queue

`TunnelService` owns a bounded Tokio MPSC queue and worker. IPC performs capability and hash validation, applies idempotency rules, creates the operation record, enqueues a command, and immediately returns an acknowledgement.

Queue saturation is a typed `busy` error. Repeated establish requests return the existing nonterminal operation. An established peer returns its existing established state. A failed terminal attempt permits a new operation.

## IPC contract

Establish returns an additive operation projection:

```json
{
  "accepted": true,
  "operation_id": "<id>",
  "peer_hash": "<identity hash>",
  "nonce": "<offer nonce>",
  "state": "queued"
}
```

Peer status returns active tunnel state plus the latest operation. A later operation-specific query may be added without changing command semantics.

## Typed errors

Errors are divided by boundary.

### Submission errors

Returned synchronously through IPC:

- `InvalidPeerHash`
- `Unauthorized`
- `AlreadyEstablished`
- `ConflictingOperation`
- `QueueFull`
- `ServiceUnavailable`

### Execution failures

Stored on the operation and emitted as state transitions:

- `IdentityNotFound`
- `NoRoute`
- `LinkActivationTimeout`
- `DeliveryFailed`
- `OfferRejected`
- `AcceptanceTimeout`
- `ProtocolDecode`
- `BackendUnavailable`
- `BackendConfigurationFailed`
- `Internal`

Every execution failure has a stable machine code, an operator-readable message, and optional source detail. Internal detail is logged but IPC must not expose secrets or uncontrolled stack traces.

## Tracing

Every operation owns a tracing span with:

- `operation_id`
- `operation_kind`
- `peer_identity`
- `offer_nonce`
- `state`
- `delivery_destination` when derived

Each transition emits one structured event containing old state, new state, elapsed time, and error code when present. Network delivery logs include the operation ID and message ID so IPC intent, LXMF delivery, and remote handling can be correlated.

Required transitions include:

```text
accepted → queued → sending_offer → offer_sent
                                 ↘ failed(code, detail)
offer_sent → negotiated → configuring → established
           ↘ rejected / timed_out
                         configuring → degraded / failed
```

Do not use polling internally. Tests and disconnected clients may use bounded status queries for reconciliation; live clients should consume events.

## Event contract

Extend tunnel state events with:

- operation ID
- operation kind
- previous state
- current state
- peer hash
- backend
- timestamp
- optional typed error code and safe message

Broadcast events are advisory. The operation registry is authoritative.

## Lifecycle and cancellation

Operations survive IPC disconnect. Daemon shutdown cancels workers and marks unfinished operations cancelled when practical. Pending offers expire after a bounded acceptance deadline. Late accepts for expired or unknown nonces are ignored and traced.

Teardown acknowledges local intent immediately; local cleanup and remote notification continue asynchronously. Rekey is modeled as its own operation, not synchronous teardown followed by establish inside IPC.

## Testing

- A blocking transport must not delay IPC acknowledgement.
- Queue saturation returns `QueueFull`.
- Duplicate establish is idempotent.
- Delivery failure becomes terminal `failed` state with a typed code.
- Every transition emits a correlated event.
- Unknown/stale nonces cannot establish a tunnel.
- T26 asserts immediate acceptance, then observes operation transitions and remote negotiation separately.

## Assumptions resolved

- Client-disconnect durability is required; daemon-restart durability is deferred.
- In-memory state is sufficient for the first slice.
- One nonterminal operation per peer and kind is allowed.
- Negotiation and backend establishment are separate states.
- IPC response and event changes are additive during migration.
- Establish is the first vertical slice; teardown and rekey adopt the same model next.

## Open Questions
