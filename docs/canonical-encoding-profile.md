---
id: canonical-encoding-profile
title: "Canonical Encoding Profile"
status: resolved
parent: identity-record-profile
tags: [cbor, canonicalization, crypto]
open_questions: []
dependencies: []
related: []
---

# Canonical Encoding Profile

## Overview

Freeze deterministic CBOR, identifier canonicalization, optional-field representation, domain framing, unknown-field policy, profile transitions, and golden/rejection vectors.

## Decisions

### Closed deterministic-CBOR profile

**Status:** accepted

**Rationale:** Signed profile-v1 records use closed schemas, increasing integer keys, shortest encodings, definite lengths, explicit nulls, canonical security identifiers, decode/re-encode byte comparison, and immutable golden/rejection vectors.

### Encoding profile governs mechanics; record profiles own schemas

**Status:** accepted

The encoding profile does not allocate one global field-number table or one universal record-size limit. Each concrete signed-record profile must freeze its own complete field table, field types, required versus nullable status, semantic limits, canonical domain separator, and maximum canonical CBOR size before that record type can enter profile v1.

Within each concrete schema:

- map keys are unsigned integers allocated contiguously from `0` in semantic order;
- an allocated number is never reused with different meaning within the same record profile version;
- every allocated field appears exactly once, including nullable fields encoded as CBOR `null`;
- unknown, omitted, duplicated, reordered, or type-mismatched fields are rejected;
- enum discriminants are unsigned integers allocated contiguously from `0` and are closed in profile v1;
- a schema change that adds, removes, renumbers, retypes, or changes the canonical interpretation of a field requires a new record profile version.

This resolves the request for field numbers at the correct ownership boundary. Inventing numbers here for records whose schemas are not yet defined would create false stability. A record is not profile-v1-ready until its owning profile supplies the table and vectors.

### Bounded canonical inputs

**Status:** accepted

Every concrete record profile declares a `MAX_CANONICAL_CBOR_BYTES` no greater than 65,535 bytes. The common signing frame uses a `u32be` CBOR length for format stability, but profile-v1 identity records remain bounded to 65,535 bytes before framing. Smaller limits are mandatory where the use case permits them.

Profile-wide ceilings are:

| Object | Maximum canonical or aggregate size |
|---|---:|
| Core runtime certificate | 16,384 bytes |
| Any other single profile-v1 identity/lifecycle/grant record | 65,535 bytes |
| Attached bootstrap bundle, including framing and contained records | 65,535 bytes |
| Verification chain | 4 signed records |

External evidence, event logs, PCR logs, and large attestations are not embedded in core records. They are represented by a SHA-256 digest plus a typed reference and are fetched under independent byte, time, redirect, and parse-depth limits.

Decoders enforce the applicable byte ceiling before allocating or recursively parsing. They also enforce schema-specific limits on text length, array count, nesting, and referenced-object count. A small encoded object that exceeds a semantic count or nesting limit is still invalid.

### Unicode normalization is not a security dependency

**Status:** accepted

Profile v1 removes the assumption that independently chosen Unicode normalization libraries are trusted to produce security-significant bytes. Security identifiers and all fields used for lookup, authorization, lifecycle-domain keys, resource selection, protocol negotiation, key identity, certificate identity, or digest references use profile-defined canonical ASCII or fixed-width byte forms. They are validated, never case-folded, and never Unicode-normalized.

A field may permit human-display text only when its concrete schema marks it `display_text`. Such text:

1. must be valid UTF-8;
2. must already be NFC when presented to the canonical encoder;
3. is rejected rather than silently normalized when it is not NFC;
4. is excluded from identity, authorization, lookup, ordering, and domain-key semantics; and
5. has a schema-specific scalar and encoded-byte limit.

Supported implementations must pass shared positive and negative conformance vectors, including composed/decomposed pairs. Cross-language byte identity is established by vectors and rejection behavior, not assumed from library equivalence.

### Fixed signing frame and registered domains

**Status:** accepted

The signing input for every signed profile-v1 object is:

```text
u16be(domain_length) || domain_ascii ||
u16be(record_profile_version) ||
u32be(canonical_cbor_length) || canonical_cbor
```

`domain_ascii` is a nonempty, centrally registered ASCII constant owned by the concrete record profile. It is not accepted from API callers. Domain constants are unique across semantic record types and versions unless a later profile explicitly defines compatible verification semantics. The framed bytes—not bare CBOR—are passed to the signature algorithm.

For atomic signing, the prepared request digest is:

```text
SHA-256("styrene-signum-prepared-request-v1\0" || signing_input)
```

The ASCII prefix including its terminal zero byte is fixed. The typed request's authorization decision, selected key/version, expected lifecycle heads, and trusted-time observation are persisted alongside this digest; if any of those facts affect signed semantics, they must already be represented in the concrete signed record. A repeated `request_id` is equivalent only when both the prepared request digest and all persisted transaction predicates match.

### Decode, validate, re-encode, compare

**Status:** accepted

A verifier accepts bytes only after all of the following succeed in order:

1. enforce the outer byte limit;
2. decode exactly one value with no trailing bytes;
3. reject tags, floats, indefinite lengths, duplicate keys, unknown keys, non-shortest encodings, and invalid UTF-8;
4. validate the closed schema and all semantic limits;
5. re-encode with the owning canonical encoder;
6. compare the re-encoded bytes byte-for-byte with the received bytes;
7. construct the registered signing frame and verify the signature.

Implementations must not rely on a generic CBOR library's default map ordering or claim canonicality merely because decoding succeeded.

## Conformance artifacts

Before a concrete record profile is declared stable, its repository fixtures must include:

- a field-number/type/nullability table;
- the exact domain separator and profile version;
- minimum and maximum valid canonical objects;
- canonical CBOR and complete framed signing-input bytes;
- expected SHA-256 record and prepared-request digests where applicable;
- at least one valid signature vector;
- rejection vectors for omitted nullable fields, unknown and duplicate keys, key reordering, non-shortest integers/lengths, indefinite forms, tags, floats, trailing bytes, invalid UTF-8, non-NFC display text, invalid canonical identifiers, and every size/count boundary;
- cross-implementation fixture verification for every supported SDK language.

Golden vectors are immutable after release. Corrections require a new profile version and an explicit migration rule; tests must retain old vectors for historical verification.

## Assessment

The original field-number question mixed two layers. This common profile can freeze canonical mechanics and global ceilings, but only record owners can assign meaningful fields. The gating work therefore moves to the concrete record profiles rather than remaining an unresolved assumption here.

No unstated normalization assumption remains: Unicode is presentation-only, input must already be NFC, and conformance is proven through shared vectors. The design is ready to constrain implementation.

## Open Questions

None for the common profile-v1 encoding contract. Concrete record profiles remain individually blocked until they publish their schema tables, limits, domains, and vectors.
