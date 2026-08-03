---
id: algorithm-and-key-version-registry
title: "Algorithm and Key-Version Registry"
status: resolved
parent: identity-record-profile
tags: [crypto, keys, algorithms]
open_questions: []
dependencies:
  - canonical-encoding-profile
related: []
---

# Algorithm and Key-Version Registry

## Overview

Define signature/digest algorithm identifiers and parameters, key IDs and versions, rollover, concurrent issuance, deprecation, downgrade prevention, and historical verification.

## Decisions

### Bounded multi-algorithm profile v1

**Status:** accepted

Profile v1 supports a closed set of signature suites selected to cover modern software implementations, TPM 2.0 and common secure elements, older PKCS#11/HSM deployments, and FIPS-oriented environments without permitting open-ended caller negotiation.

| Suite ID | Signature suite | Public key encoding | Signature encoding | Intended compatibility |
|---:|---|---|---|---|
| `0` | Ed25519 | 32-byte compressed Edwards point | exactly 64 bytes | preferred software and modern hardware path |
| `1` | ECDSA P-256 with SHA-256 | SEC 1 compressed point, exactly 33 bytes | fixed-width `r || s`, exactly 64 bytes | TPM 2.0, secure elements, cloud KMS, FIPS-oriented deployments |
| `2` | RSA-PSS with SHA-256, MGF1-SHA-256, salt length 32 | canonical unsigned big-endian modulus plus fixed exponent `65537` | exactly `ceil(modulus_bits / 8)` bytes | older HSM and PKCS#11 estates |

SHA-256 remains the sole profile-v1 object, record-reference, and key-identifier digest. The suite ID selects signature semantics only; it does not negotiate the object digest.

Ed25519 is the default for newly provisioned software custody. P-256 is preferred when hardware or compliance policy cannot support Ed25519. RSA-PSS is a compatibility suite, not a default for new deployments.

### Strict suite parameters

**Status:** accepted

Profile v1 permits no parameter negotiation inside a suite:

- Ed25519 uses the pure Ed25519 operation defined by RFC 8032. Ed25519ph and Ed25519ctx are different, unsupported algorithms.
- P-256 uses SHA-256 over the complete registered signing frame. Public points must be on-curve, non-identity, and canonically compressed. Verifiers parse fixed-width `r || s`, require `1 <= r,s < n`, and require low-S form (`s <= n/2`). Signers normalize to low-S before persistence. ASN.1 DER ECDSA signatures are rejected at profile boundaries.
- RSA-PSS uses SHA-256 for both message hashing and MGF1, a 32-byte salt, trailer field `0xbc`, exponent `65537`, and a modulus of 2048, 3072, or 4096 bits. PKCS#1 v1.5 signatures, alternate exponents, auto salt length, SHA-1, and moduli below 2048 bits are rejected.

Provider adapters may translate native provider encodings only before the canonical result is persisted. For example, a DER-encoded ECDSA provider result must be strictly parsed and converted to fixed-width low-S form; permissive parsing is forbidden.

### No caller-controlled algorithm negotiation

**Status:** accepted

The authoritative key-version record fixes one suite. A typed signing request identifies an allowed key or key family; it does not provide a preference list or request a weaker suite. Signum selects an eligible key version from verified lifecycle state and local policy, then binds the selected suite and key version into the prepared operation.

Verification never falls back from the declared suite to another suite. Unknown suite IDs, parameter mismatches, malformed keys, and malformed signatures are terminal structural failures.

Policy may impose a minimum suite per operation, trust domain, custody class, or deployment. Compatibility does not imply equal assurance: records carry custody and assurance claims separately, and RSA support must not be represented as hardware backing unless attestation proves it.

### Provider capability and uncertain-outcome requirements

**Status:** accepted

Every provider adapter declares supported suites, key sizes, deterministic-signature behavior, canonicalization behavior, attestation capability, and uncertain-outcome reconciliation capability.

Ed25519 is deterministic. ECDSA and RSA-PSS provider output may be nondeterministic. Therefore idempotency is defined by `request_id` and the persisted produced signature, never by expecting repeated provider calls to return the same bytes. After invocation begins, Signum must follow the atomic-signing `signed_uncommitted` or `outcome_unknown` recovery path and must never invoke the provider again merely because the result was lost.

A provider that cannot expose an operation handle or prove whether an interrupted invocation produced a signature is still usable, but an uncertain outcome fences that key version for operator recovery. Policy may reject such a provider for unattended or high-availability issuance.

### Monotonic key-version allocation

**Status:** accepted

A key family is identified by its canonical `key_id` and has one authoritative lifecycle domain. `key_version` is an unsigned 64-bit integer allocated by Signum; callers and providers cannot choose it.

Version allocation follows the atomic-signing reservation protocol:

1. reserve the key-family lifecycle head;
2. read its durable `high_water_version`;
3. checked-add one and persist the new high-water value in the prepared operation before provider key generation or signing;
4. bind the intended suite, provider, custody class, and predecessor key version to that allocation;
5. generate or import the provider key;
6. commit an issuance record or a terminal tombstone for the allocated version.

Version `0` is the first allocatable version. The high-water mark never decreases. Once allocation is durable, a version is never reused—even if generation fails, a client disconnects, or provider outcome is uncertain. Gaps are valid and represented by tombstone records where provider interaction may have occurred.

Concurrent allocators serialize on the canonical key-family lifecycle domain. No process-local counter, wall clock, random selection, replica arrival order, or provider-native label is authoritative for version allocation.

### Explicit activation and rollover

**Status:** accepted

Issuance, activation, retirement, suspension, revocation, and compromise are separate signed lifecycle transitions. Creating a key version does not activate it. Activation atomically names:

- the newly active key version and suite;
- the previously active version, if any;
- an activation revision;
- the permitted overlap interval, if any; and
- operation-specific issuance policy during overlap.

At most one version is preferred for new signatures in a key family. A bounded overlap may allow an older version only for explicitly enumerated operations or queued transactions prepared before activation. New operations cannot select the old version merely because it remains cryptographically valid.

A suite transition is a normal rollover to a higher key version. It cannot rewrite the suite attached to an existing version. Retirement blocks new signing but preserves historical verification. Revocation and compromise apply according to their lifecycle semantics and are not aliases for retirement.

### Downgrade prevention and historical verification

**Status:** accepted

The verifier evaluates a signature against the exact suite and key version named by the signed record and proven through the authoritative lifecycle chain. It never substitutes another version or suite.

For newly accepted executable records, local policy and current authoritative lifecycle state must permit that suite and key version at processing time. Once a verifier has observed activation revision `N`, a record relying on an earlier active-key view cannot reset the family to an older version. Missing newer lifecycle state may yield `Indeterminate`; it never authorizes fallback.

Historical verification retains immutable public key material and suite parameters for every issued version, including retired and later-disallowed suites. Deprecating a suite has two independent policy dates:

- `sign_not_after`: no new signatures may be produced after this point;
- `accept_not_after`: newly encountered signatures under the suite are rejected or become indeterminate after this point according to record class.

Previously persisted acceptance receipts may continue to prove what was accepted under the policy then in force. Cryptographic verification remains available for audit even after authorization acceptance is disabled.

### Registry evolution

**Status:** accepted

Suite IDs are never reused. Profile v1 is closed to IDs `0`, `1`, and `2`; unknown IDs fail verification. Adding a suite requires a new registry revision with complete key/signature encodings, parameter constraints, provider behavior, security policy, and positive/rejection vectors. It does not silently alter profile-v1 semantics.

Post-quantum and hybrid suites are deferred. Their key and signature sizes, combination rules, provider support, and lifecycle transitions require an explicit later profile rather than speculative IDs in v1.

## Required conformance vectors

Each suite must have immutable vectors covering:

- canonical public-key and signature encodings;
- complete framed signing input and valid signature;
- malformed lengths and out-of-range values;
- wrong-suite and wrong-key verification;
- P-256 invalid points, DER input, zero/out-of-range scalars, and high-S rejection;
- RSA unsupported modulus sizes, alternate exponent, incorrect PSS salt length, MGF mismatch, PKCS#1 v1.5 input, and leading-byte edge cases;
- unknown suite IDs and policy-disabled suites;
- rollover overlap, skipped/tombstoned versions, concurrent allocation, downgrade attempts, retirement, and historical verification.

## Assessment

The expanded suite set provides practical coverage for older hardware without making algorithm choice caller-controlled. Its cost is three provider and conformance paths plus explicit handling for nondeterministic signatures. The strict parameters and fixed encodings keep that cost bounded.

The key-version question is mechanical and is closed by durable high-water allocation under the existing atomic transaction protocol. No operator-selected conflict rule remains.

## Open Questions

None for profile-v1 signature suites and key-version lifecycle rules. Provider-specific attestation evidence remains owned by the custody and attestation profile.
