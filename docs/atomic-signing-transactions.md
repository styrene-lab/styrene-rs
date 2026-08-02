---
id: atomic-signing-transactions
title: "Atomic Signing Transactions"
status: resolved
parent: signum-service-boundary
tags: [signing, transactions, toctou]
open_questions: []
dependencies:
  - canonical-encoding-profile
  - algorithm-and-key-version-registry
  - lifecycle-domain-graph
  - trusted-time-and-rollback
related: []
---

# Atomic Signing Transactions

## Overview

Define the typed signing transaction from authorization through lifecycle checks, canonicalization, hardware signing, durable idempotency/audit commit, and response.

## Decisions

### Typed signing requests only

**Status:** accepted

**Rationale:** Signum re-derives domain-separated canonical input from typed requests. Raw arbitrary-byte signing is never exposed over RPC.

### Reserve before invoking a non-rollbackable signer

**Status:** accepted

Each request carries a caller-generated `request_id`. Signum derives and persists the canonical request digest, authorization decision, selected signing key ID and version, trusted-time reading, required lifecycle-domain keys, and their expected heads in a durable `prepared` operation before invoking the signer. A repeated `request_id` with a different request digest is rejected; an exact repeat reads or resumes the original operation.

Preparation obtains a transaction-level write reservation for every domain the operation can advance. The reservation is represented durably by the operation record and expected-head compare-and-swap predicates rather than by a process-local mutex. Domains are ordered by their canonical encoded domain key before reservation to prevent deadlock. At minimum, an operation reserves:

1. the selected key-version domain, including its enabled/revoked state and monotonic use or sequence state;
2. the subject lifecycle domain whose new signed record will become authoritative;
3. every additional lifecycle domain whose head the typed operation advances; and
4. the trusted-time or rollback-checkpoint domain when the operation advances that checkpoint.

Read-only dependencies are captured by digest or version in the prepared operation and revalidated immediately before signing. If policy requires a read dependency to remain unchanged through commit, it is promoted to a reserved CAS domain. The implementation must not hold a database transaction or process-local lock across an HSM/TPM/network call.

After preparation, no competing operation may commit an advancement of a reserved domain unless it first causes this operation to reach a terminal state. This fencing requirement is what makes the pre-sign expected heads valid after the hardware signature; attempting an optimistic CAS only after signing is insufficient because a lost CAS would strand an unaccounted valid signature.

### A produced signature is durable operation state, not a failed request

**Status:** accepted

The signer result is durably recorded as `signed_uncommitted` before response delivery. Finalization atomically writes the signed record, advances all reserved heads, appends the audit event, and changes the operation to `committed`. The response is returned only from a durable `committed` operation.

If signature production succeeds but recording `signed_uncommitted` cannot be confirmed, Signum returns `outcome_unknown(request_id)` and fences the selected key version from further signing. It must not report ordinary failure or automatically invoke the signer again. Recovery reconciles the operation using a provider operation handle or deterministic signature verification when the provider exposes sufficient evidence; otherwise an operator must resolve the fenced key under an audited recovery procedure.

If `signed_uncommitted` is durable but final commit fails, Signum returns `pending_commit(request_id)`. Recovery retries finalization without signing again. Exact retries return the persisted pending status or the committed result. A prepared operation for which the signer is proven not to have run may be aborted and its reservations released; a signed operation may never be aborted merely to release reservations.

### One local durability authority owns operation, audit, and head commit

**Status:** accepted

Profile v1 requires the operation journal, authoritative lifecycle heads, signed-record store, and audit ledger involved in one signing operation to share one transactional durability authority. Finalization is a single serializable local transaction. Replication occurs from committed records afterward and is not part of the signing commit path.

This deliberately excludes distributed two-phase commit in profile v1. Deployments that split those stores cannot claim atomic-signing conformance until a later profile defines equivalent consensus and recovery semantics.

## Transaction state machine

The durable states are:

- `prepared`: request and reservations are durable; signing may not yet have occurred;
- `signing`: optional provider invocation marker with provider operation handle;
- `signed_uncommitted`: signature and complete signed-record bytes are durable;
- `committed`: signed record, head advancements, audit event, and response are durable;
- `aborted`: terminal only when policy denial, validation failure, stale heads, or positive evidence establishes that no signature was produced;
- `outcome_unknown`: signature production cannot be ruled out; affected key version remains fenced pending recovery.

Only `committed` returns a successful signing response. State transitions are monotonic and audited. Crash recovery scans all nonterminal operations before permitting use of their reserved domains or key versions.

## Required failure properties

- Authorization, trusted time, lifecycle state, key selection, and canonicalization occur before signer invocation and are bound into the prepared request digest.
- No raw signature is exposed separately from its typed signed record.
- Cancellation and client disconnect do not cancel a prepared or later operation; the client resolves it by `request_id`.
- Timeouts after signer invocation produce `pending_commit` or `outcome_unknown`, never a retryable generic error.
- Audit append failure prevents `committed`, but does not erase or classify a produced signature as failed.
- Recovery is idempotent and never signs a second time for the same `request_id`.

## Assessment

This design resolves the two original blocking questions. It also closes the lifecycle-domain graph's local atomic-commit question for signing operations: ordered durable reservations plus one serializable finalization transaction are the normative profile-v1 mechanism.

The remaining dependencies are schema-level rather than transaction-model unknowns: the canonical encoding profile must assign operation and record fields and limits, while the algorithm/key registry must define the exact key-version domain and provider reconciliation capabilities.

## Open Questions

None for the profile-v1 transaction model. Provider-specific reconciliation and distributed durability authorities require later profiles.
