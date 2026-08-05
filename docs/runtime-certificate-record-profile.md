---
id: runtime-certificate-record-profile
title: "Runtime Certificate Record Profile"
status: resolved
parent: identity-record-profile
tags: [identity, certificate, schema, cbor]
open_questions: []
dependencies:
  - canonical-encoding-profile
  - algorithm-and-key-version-registry
  - custody-and-attestation
  - certificate-renewal-and-revocation
related:
  - lifecycle-transition-record-profile
---

# Runtime Certificate Record Profile

## Overview

Freeze the portable profile-v1 runtime certificate: exact deterministic-CBOR fields, identifiers, bounds, signing domain, validity rules, custody result, predecessor binding, and signature container.

## Decisions

### Signed object shape and domain

**Status:** accepted

A runtime certificate is a two-element deterministic-CBOR array:

```text
[
  protected : bstr,  // exact canonical RuntimeCertificateClaims map bytes
  signature : bstr   // canonical signature bytes for issuer_suite
]
```

The signature covers the common signing frame constructed from `protected`:

```text
u16be(30) || "styrene-runtime-certificate-v1" ||
u16be(1) || u32be(len(protected)) || protected
```

The registered domain is exactly `styrene-runtime-certificate-v1` (30 ASCII bytes). The record profile version is `1`. `protected` is retained byte-for-byte; a verifier decodes, validates, re-encodes, and compares it before signature verification. The signature is outside the protected map, so certificate digest identity includes it while signing input does not recurse.

The certificate digest is SHA-256 over the complete canonical outer array. A certificate reference is `(issuer_key_id, certificate_id, certificate_digest)`. Equality of certificate content uses `certificate_digest`; reuse of `(issuer_key_id, certificate_id)` with different content is a permanent integrity conflict.

`MAX_RUNTIME_CERTIFICATE_BYTES` is 16,384 bytes for the complete outer array. The limit is enforced before allocation or nested parsing.

### RuntimeCertificateClaims field table

**Status:** accepted

The protected value is a closed CBOR map containing every key `0..24` exactly once in increasing order. Nullable values are explicit CBOR `null`; fields are never omitted.

| Key | Name | CBOR type | Required semantics and limit |
|---:|---|---|---|
| 0 | `profile_version` | uint | exactly `1` |
| 1 | `certificate_id` | bstr | exactly 16 random bytes |
| 2 | `issuer_key_id` | bstr | exactly 32 bytes |
| 3 | `issuer_key_version` | uint | `u64` |
| 4 | `issuer_suite` | uint | registry suite `0..2` |
| 5 | `subject_kind` | uint | enum below |
| 6 | `subject_id` | tstr | canonical ASCII, 1..255 bytes |
| 7 | `runtime_id` | bstr | exactly 16 random bytes |
| 8 | `runtime_key_id` | bstr | exactly 32 bytes |
| 9 | `runtime_key_version` | uint | `u64` |
| 10 | `runtime_suite` | uint | registry suite `0..2` |
| 11 | `runtime_public_key` | bstr or map | canonical suite-specific key form |
| 12 | `host_subject_binding` | bstr | exactly 32 opaque bytes |
| 13 | `not_before_ms` | uint | supported `u64` epoch milliseconds |
| 14 | `not_after_ms` | uint | supported `u64` epoch milliseconds |
| 15 | `renew_after_ms` | uint | supported `u64` epoch milliseconds |
| 16 | `custody_class` | uint | enum `0..2`; value `3` rejected in v1 |
| 17 | `evidence_profile_id` | tstr or null | canonical ASCII, 1..127 bytes when present |
| 18 | `evidence_digest` | bstr or null | exactly 32 bytes when present |
| 19 | `attestation_verifier_id` | bstr or null | exactly 32 bytes when present |
| 20 | `attested_at_ms` | uint or null | supported epoch when present |
| 21 | `degraded` | bool | explicit, never inferred by absence |
| 22 | `predecessor_certificate_digest` | bstr or null | exactly 32 bytes when present |
| 23 | `issuance_revision` | uint | revision in runtime-certificate lifecycle domain |
| 24 | `extensions_digest` | bstr or null | exactly 32 bytes; protected separately versioned extension object |

No display text exists in this record. `subject_id` and `evidence_profile_id` are security identifiers: they use printable ASCII bytes `0x21..0x7e`, must already be in their profile-defined canonical spelling, and are never Unicode-normalized or case-folded.

### Identifier and public-key derivation

**Status:** accepted

`issuer_key_id` and `runtime_key_id` are derived rather than arbitrary labels:

```text
SHA-256("styrene-key-id-v1\0" || suite_id_u16be || canonical_public_key)
```

For Ed25519 and P-256, `canonical_public_key` is the fixed key byte string defined by the suite registry. For RSA-PSS it is the canonical CBOR map `{0: modulus_bstr, 1: 65537}` with keys in increasing order, no leading zero in the unsigned modulus, and a permitted modulus size.

A verifier recomputes `runtime_key_id` from fields `10` and `11`. It obtains the issuer public key from the verified issuer chain, checks that fields `2..4` identify that exact version and suite, and recomputes `issuer_key_id`. Mismatch is structural failure before signature acceptance.

`subject_kind` is closed:

| ID | Meaning |
|---:|---|
| 0 | logical agent |
| 1 | durable workload |

The subject ID namespace is selected by `subject_kind`; presentation aliases are forbidden. The 32-byte `host_subject_binding` is `SHA-256` over the issuer-owned canonical enrollment-subject record, not a stable host name exposed to peers.

### Time and renewal invariants

**Status:** accepted

The following are structural invariants:

- `not_before_ms < renew_after_ms < not_after_ms`;
- `not_after_ms - not_before_ms <= 86,400,000` (24 hours);
- `renew_after_ms` lies between 50% and 80% of the certificate lifetime, using checked integer arithmetic;
- all values are within the implementation's supported epoch range;
- certificate validity is evaluated with verifier-owned trusted time and configured clock drift; and
- expiry is derived and never encoded as lifecycle state.

A first certificate has null `predecessor_certificate_digest`. Renewal must name the complete prior certificate digest, use a greater runtime key version, and carry the issuance revision committed for this certificate. A verifier cannot infer valid renewal merely from adjacent key-version integers; it validates the lifecycle chain.

### Custody-field matrix

**Status:** accepted

The custody fields have closed combinations:

| Class | Evidence fields | `degraded` |
|---|---|---|
| `software_ephemeral` (`0`) | fields `17..20` all null | must be `true` only when issuance policy fell back from requested stronger custody; otherwise `false` |
| `os_protected` (`1`) | fields `17..20` all non-null and identify an approved OS evidence receipt | may be true only when local issuance policy records reduced assurance |
| `hardware_nonexportable` (`2`) | fields `17..20` all non-null and identify fresh verified evidence | must be `false` |
| `hardware_measured` (`3`) | invalid in profile v1 | invalid |

`attested_at_ms` must satisfy `not_before_ms - acceptable_clock_drift <= attested_at_ms <= not_before_ms`, with checked arithmetic. For class `2`, the evidence must be from the challenge consumed by the issuance transaction and must bind fields `5..12`. Raw evidence is not embedded.

### Signature and issuer validation

**Status:** accepted

`issuer_suite` selects the exact canonical signature encoding. The signature length and scalar/parameter rules are enforced before provider invocation. Verification then requires:

1. canonical outer array and protected map;
2. all structural, identifier, time, custody, and size rules;
3. an issuer key/version record whose exact key ID, version, and suite match fields `2..4`;
4. issuer authorization for the subject and runtime-certificate lifecycle domain at `issuance_revision`;
5. certificate signature validity over the registered frame;
6. no dominating suspension, revocation, compromise, or fork; and
7. lifecycle freshness required by the consuming operation.

A cryptographically valid certificate is not by itself current authorization. Verification returns structured certificate, custody, lifecycle-head, freshness, and degradation evidence.

## Canonical fixture requirements

The profile fixture set must contain:

- minimum valid software, OS-protected, TPM P-256, Ed25519, and RSA-PSS certificates;
- renewal certificates with predecessor linkage and all time-window boundaries;
- exact protected CBOR, signing frame, signature, outer CBOR, certificate digest, and derived key IDs;
- maximum-size extension reference case;
- rejection vectors for every omitted, nullability, type, key-order, identifier, public-key, signature, time, custody-matrix, predecessor, digest, and size violation; and
- conflicting `(issuer_key_id, certificate_id)` content.

## Open Questions

None for the profile-v1 runtime-certificate schema.
