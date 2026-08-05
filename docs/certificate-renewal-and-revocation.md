---
id: certificate-renewal-and-revocation
title: "Certificate Renewal and Revocation"
status: resolved
parent: runtime-identity-issuance
tags: [certificate, renewal, revocation]
open_questions: []
dependencies: []
related: []
---

# Certificate Renewal and Revocation

## Overview

Define runtime certificate renewal, overlap, key reuse/versioning, retirement, suspension, revocation, authority compromise blast radius, and cache invalidation.

## Decisions

### Renewal always advances the runtime key version

**Status:** accepted

Every runtime-certificate renewal allocates a new monotonically increasing key version under the existing key-family lifecycle transaction. A certificate ID is never renewed in place, and a key version is never reused for a second certificate.

The default and required behavior by custody class is:

| Custody class | Renewal key behavior |
|---|---|
| `software_ephemeral` | Generate a fresh runtime-local key; reuse is forbidden |
| `os_protected` | Generate a fresh keystore key object and obtain fresh OS evidence |
| `hardware_nonexportable` | Generate a fresh non-exportable hardware key and obtain fresh challenge-bound evidence |
| `hardware_measured` | Not issuable in profile v1 |

This rule intentionally prefers simple compromise boundaries and uniform verification over provider-specific key reuse. Hardware with slow generation may begin renewal early, but cannot certify the old key under a new version. Providers unable to create a fresh key at least once per 24-hour certificate lifetime cannot issue profile-v1 runtime certificates; deployments may use a longer-lived authority key to issue short-lived runtime keys, but may not weaken the runtime rule silently.

A renewal retains the logical agent or durable-workload subject and may retain the runtime ID only while the same runtime incarnation remains alive. Restart, migration, custody-class upgrade or downgrade, provider replacement, host-subject change, or recovery from failed attestation creates a new runtime ID as well as a new key.

### Renewal is a typed atomic transaction

**Status:** accepted

A renewal request carries a unique `request_id` and identifies the current certificate digest, runtime ID, and requested issuance policy. Signum reserves:

- the runtime-certificate lifecycle domain for the authority and subject;
- the runtime key-family lifecycle domain;
- the revocation/suspension domain needed to prove the predecessor remains eligible; and
- the enrollment/attestation challenge domain when custody evidence is required.

The transaction verifies authorization, current certificate state, trusted time, quota, provider policy, and fresh proof of possession before committing. It atomically records the new key version, certificate, predecessor relation, activation state, audit event, and any consumed challenge. A retry with the same request ID returns the original result and never creates another key.

Failure after provider interaction follows the atomic-signing uncertain-outcome rules. Allocated versions are not reused. A failed or unknown hardware-generation outcome is tombstoned and may fence the affected provider object until reconciled.

### Bounded renewal window and overlap

**Status:** accepted

The default runtime-certificate lifetime is 24 hours. The issuer defines a `renew_after_ms` no earlier than 50 percent and no later than 80 percent of that lifetime. Clients add stable per-runtime jitter within the permitted window to prevent synchronized renewal storms.

The normal overlap between predecessor and successor equals the configured `acceptable_clock_drift`, default five minutes and bounded by the local ceiling. During overlap:

- the new version is preferred immediately after activation;
- transactions prepared before activation may finish under the predecessor;
- receivers may verify objects signed by either version when their claimed creation time falls within that version's certificate validity;
- new signing requests select the predecessor only when explicitly tied to a transaction prepared before activation; and
- overlap does not consume an additional replica quota slot.

Overlap never extends either certificate's `not_before` or `not_after`. If renewal completes after expiry, there is no retroactive overlap and no authority to sign during the gap. An expired runtime must stop creating new signed work until a valid successor is active.

### Certificate and key lifecycle states are explicit

**Status:** accepted

Profile v1 distinguishes:

- `issued`: certificate exists but is not yet preferred for signing;
- `active`: eligible according to certificate validity and policy;
- `retired`: routine terminal state; no new signatures, historical verification retained;
- `suspended`: immediate reversible block on new signing and newly accepted executable work;
- `revoked_administrative`: permanent invalidation from the effective revision forward;
- `revoked_compromise`: permanent compromise invalidation with the applicable retrospective policy; and
- `expired`: derived from trusted time, not a signed lifecycle transition.

Activation, retirement, suspension, reinstatement, and revocation are signed lifecycle records. State changes are monotonic except that suspension may be lifted by a fresh authorized reinstatement record. Reinstatement never clears a permanent revocation and cannot restore an expired certificate.

Routine renewal activates the successor and schedules or commits predecessor retirement. It does not revoke the predecessor. Key deletion is a provider cleanup action performed only after durable retirement/revocation state and required audit retention; deletion does not replace lifecycle records.

### Revocation dominates validity and overlap

**Status:** accepted

A suspension or revocation takes effect according to its authoritative lifecycle revision and immediately overrides certificate validity, activation, and renewal overlap for new security decisions. A verifier never falls back to an older certificate because the preferred version is revoked.

Administrative revocation blocks newly accepted work at and after its effective trusted time/revision but does not by itself assert that earlier signatures were forged. Compromise revocation carries:

- affected key versions or authority scope;
- `compromise_not_before_ms`, or `unknown`;
- reason code;
- issuer or superior recovery authority;
- affected descendant policy; and
- replacement/recovery reference when available.

For compromise with a known lower bound, signatures at or after that bound are not accepted as authentic for new decisions. With an unknown compromise start, all signatures under the affected key are `Indeterminate` unless a persisted local acceptance receipt proves they were verified and accepted before compromise discovery under then-current policy. Cryptographic verification remains available for audit but does not imply authorization acceptance.

Emergency suspension never auto-expires. It requires a fresh authorized record to reinstate or convert to permanent revocation.

### Blast radius follows verified custody boundaries

**Status:** accepted

A proven isolated runtime-key compromise revokes that key version and all certificates containing it. The runtime receives a new runtime ID, key family or next version according to recovery policy, and certificate only after fresh authorization and custody evidence.

If isolation is not established, revocation expands conservatively:

- compromise of a host/provider boundary affects every runtime key whose custody evidence depends on that boundary during the affected interval;
- compromise of an issuing authority affects every unexpired descendant certificate it issued and blocks new issuance;
- compromise of shared entropy, signer service, attestation verifier, or authority custody affects all keys for which independence cannot be proven; and
- owner/recovery-authority compromise follows the superior lifecycle and recovery policy rather than being repaired by a runtime renewal.

Authority rollover does not automatically rehabilitate descendants issued during a suspected compromise interval. A superior reconciliation or recovery record must identify trusted branches and replacement authority state.

### Cache invalidation is push-accelerated and pull-bounded

**Status:** accepted

Revocation and suspension records are authoritative signed objects. Signum distributes them through replication and optional push notifications, but correctness never depends solely on push delivery.

Caches are indexed by certificate digest, certificate ID plus issuer, runtime key ID/version, issuer authority, host/provider binding where policy permits, and lifecycle-domain head. Applying a lifecycle update invalidates all affected positive-verification entries and descendant chain results atomically.

For newly received executable work:

- sensitive execution defaults to a maximum cached revocation age of one hour;
- local policy may require fresher state or online confirmation;
- never-synchronized revocation state fails closed;
- a cache older than policy yields `Indeterminate`, not valid;
- negative structural/cryptographic results may be cached by object digest;
- transient retrieval failures are not cached as permanent revocation; and
- persisted acceptance receipts are immutable historical evidence, not reusable proof for a different message or task.

Certificate-reference retrieval validates both certificate ID and digest. Conflicting bytes for one `(issuer_key_id, certificate_id)` are a permanent integrity conflict and invalidate the cache entry rather than selecting by arrival time.

### Renewal and revocation races

**Status:** accepted

Renewal, suspension, and revocation serialize through their shared lifecycle domains. A renewal prepared against expected heads cannot commit after a competing suspension or revocation advances those heads. If hardware signing or key generation has already occurred, the prepared renewal resolves through `signed_uncommitted`, tombstone, or `outcome_unknown`; it never overrides the newer revocation.

Revoking a predecessor during overlap does not revoke the successor unless the revocation scope, compromise interval, or shared-custody blast-radius rule includes it. Conversely, issuance of a successor does not erase evidence that the predecessor or its provider was compromised.

## Required conformance cases

Profile-v1 implementations must test:

- fresh key and strict version advancement for every custody class and signature suite;
- duplicate renewal request IDs and concurrent renewal allocation;
- provider failure before and after durable version allocation;
- normal overlap boundaries, clock drift, expiry gaps, and prepared-before-activation signing;
- retirement versus administrative revocation versus compromise revocation;
- suspension, failed auto-expiry attempts, authorized reinstatement, and revocation dominance;
- known and unknown compromise intervals;
- isolated runtime, shared provider, issuer-authority, and indeterminate compromise blast radii;
- renewal racing suspension or revocation;
- cache invalidation by certificate, key version, issuer, and lifecycle head;
- stale and never-synchronized revocation state; and
- historical cryptographic verification separated from current authorization acceptance.

## Assessment

Routine key reuse would weaken compromise boundaries, create custody-specific semantics, and complicate the already-established monotonic key-version model. Profile v1 therefore uses one uniform rule: every renewal means a fresh key and a fresh key version.

The remaining work is schema and implementation detail, not policy. Runtime-certificate and lifecycle-transition record profiles must encode the predecessor, custody evidence result, state transition, compromise scope, and effective-time fields defined here.

## Open Questions

None for profile-v1 runtime-certificate renewal, overlap, suspension, revocation, or cache behavior.
