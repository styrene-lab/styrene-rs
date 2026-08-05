---
id: lifecycle-transition-record-profile
title: "Lifecycle Transition Record Profile"
status: resolved
parent: identity-record-profile
tags: [identity, lifecycle, schema, cbor]
open_questions: []
dependencies:
  - canonical-encoding-profile
  - lifecycle-domain-graph
  - certificate-renewal-and-revocation
related:
  - runtime-certificate-record-profile
---

# Lifecycle Transition Record Profile

## Overview

Freeze the common profile-v1 signed lifecycle transition: exact deterministic-CBOR fields, typed domain keys, revisions and predecessors, transition payloads, compromise scope, reconciliation, and validation rules.

## Decisions

### Signed object shape and domain

**Status:** accepted

A lifecycle transition is a two-element deterministic-CBOR array:

```text
[
  protected : bstr,  // exact canonical LifecycleTransitionClaims map bytes
  signature : bstr   // canonical signature bytes for issuer_suite
]
```

The signature covers:

```text
u16be(31) || "styrene-lifecycle-transition-v1" ||
u16be(1) || u32be(len(protected)) || protected
```

The registered domain is exactly `styrene-lifecycle-transition-v1` (31 ASCII bytes), and the record profile version is `1`. The transition digest is SHA-256 over the complete canonical outer array. `MAX_LIFECYCLE_TRANSITION_BYTES` is 65,535 bytes, enforced before allocation or parsing.

The protected map is closed. The transition does not embed the signed object it affects; it identifies targets and replacement/recovery objects by canonical digest.

### LifecycleTransitionClaims field table

**Status:** accepted

Every key `0..17` appears exactly once in increasing order. Nullable fields use explicit CBOR `null`.

| Key | Name | CBOR type | Required semantics and limit |
|---:|---|---|---|
| 0 | `profile_version` | uint | exactly `1` |
| 1 | `transition_id` | bstr | exactly 16 random bytes |
| 2 | `domain_key` | array | one canonical typed domain key |
| 3 | `revision` | uint | `u64`; zero permitted only for a domain's defined bootstrap transition |
| 4 | `previous_transition_digest` | bstr or null | null iff bootstrap revision `0`; otherwise exactly 32 bytes |
| 5 | `transition_kind` | uint | closed enum below |
| 6 | `target_kind` | uint | closed enum below |
| 7 | `target_id` | bstr | exactly 32 bytes |
| 8 | `effective_at_ms` | uint | issuer trusted-time observation |
| 9 | `reason_code` | uint | closed reason registry below |
| 10 | `issuer_key_id` | bstr | exactly 32 bytes |
| 11 | `issuer_key_version` | uint | `u64` |
| 12 | `issuer_suite` | uint | registry suite `0..2` |
| 13 | `replacement_digest` | bstr or null | exactly 32 bytes when present |
| 14 | `compromise_not_before_ms` | uint or null | only for compromise revocation; null means unknown start there |
| 15 | `descendant_scope` | uint | closed enum below |
| 16 | `reconciliation` | map or null | only for `reconcile` |
| 17 | `extensions_digest` | bstr or null | exactly 32 bytes when present |

`target_id` is a digest-like canonical identifier selected by `target_kind`; it is never a display label. `effective_at_ms` orders policy effect only within the authoritative signed transition chain; it never resolves forks or replaces `revision` and predecessor validation.

### Typed lifecycle domain keys

**Status:** accepted

`domain_key` is a closed CBOR array whose first element is a domain discriminant and whose remaining elements are fixed by that discriminant. Every text element is canonical printable ASCII `0x21..0x7e`, 1..255 bytes. Every `*_id` byte string below is exactly 32 bytes unless stated otherwise.

| ID | Domain | Canonical array |
|---:|---|---|
| 0 | owner state | `[0, trust_domain:tstr]` |
| 1 | agent/workload authority | `[1, trust_domain:tstr, authority_id:bstr]` |
| 2 | runtime-certificate lifecycle | `[2, authority_id:bstr, subject_kind:uint, subject_id:tstr]` |
| 3 | revocation/suspension | `[3, issuing_authority_id:bstr]` |
| 4 | enrollment nonce consumption | `[4, enrollment_issuer_id:bstr]` |
| 5 | replica quota leases | `[5, issuer_id:bstr, replica_id:bstr]` |
| 6 | recovery policy | `[6, trust_domain:tstr, recovery_epoch:uint]` |
| 7 | client/API grants | `[7, granting_authority_id:bstr, client_id:bstr]` |
| 8 | audit purge authorization | `[8, trust_domain:tstr]` |
| 9 | key-family lifecycle | `[9, key_id:bstr]` |

ID `9` makes explicit the key-family domain required by monotonic key-version allocation. A profile-v1 transition with an unknown discriminant or wrong array arity/type is invalid. Domain identity is the exact canonical array bytes; implementations must not concatenate strings.

`subject_kind` has the same registry as the runtime certificate (`0` agent, `1` durable workload). Domain keys contain no profile version because the outer signed record fixes interpretation; a future incompatible key shape requires a new lifecycle-transition profile.

### Transition and target registries

**Status:** accepted

`transition_kind` is closed:

| ID | Kind | Meaning |
|---:|---|---|
| 0 | `bootstrap` | establish a domain at revision `0` |
| 1 | `issue` | record issuance without activation |
| 2 | `activate` | make target preferred/current |
| 3 | `retire` | routine terminal signing state |
| 4 | `suspend` | immediate reversible block |
| 5 | `reinstate` | lift suspension under fresh authority |
| 6 | `revoke_administrative` | permanent forward invalidation |
| 7 | `revoke_compromise` | permanent compromise invalidation |
| 8 | `consume` | single-use nonce/token consumption |
| 9 | `tombstone` | preserve allocated but unusable identifier/version |
| 10 | `reconcile` | superior-authority fork resolution |
| 11 | `grant` | establish or replace authorization state |
| 12 | `purge_authorize` | authorize bounded audit purge |

`target_kind` is closed:

| ID | Target |
|---:|---|
| 0 | runtime certificate digest |
| 1 | key-version record digest |
| 2 | authority record digest |
| 3 | owner-state record digest |
| 4 | enrollment challenge digest |
| 5 | replica lease digest |
| 6 | recovery-policy record digest |
| 7 | API-grant record digest |
| 8 | audit-purge proposal digest |
| 9 | lifecycle-transition digest |

A domain admits only semantically applicable transition/target pairs. In particular, runtime-certificate lifecycle admits issue/activate/retire/suspend/reinstate/revocation targeting certificates; enrollment nonce admits consume; key-family lifecycle admits issue/activate/retire/suspend/revocation/tombstone targeting key-version records; and reconcile is signed in the domain whose fork it resolves while targeting a lifecycle transition.

### Reason and descendant-scope registries

**Status:** accepted

`reason_code` is closed:

| ID | Reason |
|---:|---|
| 0 | unspecified/non-adverse |
| 1 | routine renewal |
| 2 | operator request |
| 3 | policy violation |
| 4 | key compromise |
| 5 | provider/host compromise |
| 6 | authority compromise |
| 7 | attestation or collateral invalidation |
| 8 | lost custody |
| 9 | superseded |
| 10 | failed or uncertain provider operation |
| 11 | recovery action |

Reason `0` is permitted only for bootstrap, issue, activate, consume, grant, and purge authorization. Adverse suspension/revocation and tombstone transitions require a nonzero reason appropriate to their kind.

`descendant_scope` is closed:

| ID | Scope |
|---:|---|
| 0 | target only |
| 1 | named key version and certificates containing it |
| 2 | all descendants issued by the target authority |
| 3 | all records bound to the affected provider/host evidence boundary |
| 4 | entire trust domain pending superior recovery |

Non-revocation transitions must use `0`. Compromise revocation uses the narrowest scope justified by verified isolation; uncertainty expands scope conservatively under policy.

### Revision and predecessor invariants

**Status:** accepted

A domain's bootstrap transition is revision `0`, has null predecessor, uses kind `bootstrap`, and is the only transition allowed to omit a predecessor. Every successor has:

```text
revision == predecessor.revision + 1
previous_transition_digest == SHA-256(predecessor_outer_bytes)
domain_key == predecessor.domain_key
```

Arithmetic is checked. Revision gaps remain pending. Two valid successors of one predecessor form a fork and make the domain `Indeterminate`; numeric revision, timestamp, arrival order, replica priority, or digest order never selects a winner.

The tuple `(issuer_key_id, transition_id)` is unique. Reuse with different bytes is a permanent integrity conflict. A transition signature is valid only if the issuer was authorized to advance the exact domain at the predecessor state.

### Transition-specific field matrix

**Status:** accepted

- `compromise_not_before_ms` is null for every kind except `revoke_compromise`. For compromise revocation, null means the compromise start is unknown; a present value must be `<= effective_at_ms`.
- `replacement_digest` is optional for activate, retire, suspension, both revocations, tombstone, and reconcile; it is null for bootstrap and consume. When used for activation/renewal it identifies the replacement signed object, not an unsigned locator.
- `reconciliation` is non-null only for `reconcile` and is required there.
- `descendant_scope` is nonzero only for compromise revocation.
- `extensions_digest` refers to a separately versioned protected extension object and does not permit unknown base fields.

Expiration is never a transition kind. It is derived from a certificate or grant's signed time bounds. Suspension cannot encode auto-expiry; reinstatement requires a new transition.

### Canonical reconciliation payload

**Status:** accepted

For `reconcile`, field `16` is the closed map:

| Key | Name | Type | Rule |
|---:|---|---|---|
| 0 | `fork_predecessor_digest` | bstr | exactly 32 bytes |
| 1 | `winning_head_digest` | bstr | exactly 32 bytes |
| 2 | `rejected_head_digests` | array of bstr | 1..64 unique 32-byte digests, sorted lexicographically |
| 3 | `superior_domain_key` | typed domain-key array | domain authorized by the lifecycle inventory |
| 4 | `superior_transition_digest` | bstr | exactly 32 bytes |

The winning digest must not occur in the rejected array. Every named head must descend from the named fork predecessor in the same target domain. The superior transition must explicitly authorize this reconciliation and itself be verified before the target domain advances. Rejected branches remain retained history.

### Signature and semantic validation

**Status:** accepted

Verification requires, in order:

1. outer size, canonical array, protected-map, and signature encoding validation;
2. exact domain-key, registry, field-matrix, and transition-specific validation;
3. issuer key ID/version/suite resolution and signature verification over the registered frame;
4. predecessor presence and exact digest/revision/domain match, except bootstrap;
5. issuer authority for the domain and transition kind;
6. target object existence or inclusion in the same bounded atomic bundle when the kind requires it;
7. cross-check of target identity, state, and replacement relation;
8. fork and superior-reconciliation evaluation; and
9. atomic advancement of every domain named by the encompassing operation.

A transition can be cryptographically valid yet not advance authoritative state because its predecessor is missing, its domain is forked, its issuer lacks authority, or a concurrent transaction won the expected-head reservation.

## Canonical fixture requirements

Fixtures must include:

- bootstrap and successor vectors for all ten domain-key shapes;
- every transition, target, reason, and descendant-scope discriminant;
- certificate renewal activation/retirement, suspension/reinstatement, known/unknown compromise, nonce consumption, failed-allocation tombstone, and fork reconciliation;
- exact protected CBOR, signing frame, signature, outer CBOR, transition digest, and predecessor digest;
- rejection vectors for wrong arity/type, unknown IDs, omitted nullable fields, noncanonical identifiers, gaps, overflow, wrong predecessor/domain, invalid field combinations, unauthorized issuer, duplicate IDs, fork winner-by-order attempts, and malformed reconciliation arrays; and
- multi-domain atomic bundles demonstrating all-or-nothing head advancement.

## Open Questions

None for the profile-v1 lifecycle-transition schema and typed domain keys.
