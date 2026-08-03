---
id: custody-and-attestation
title: "Custody and Attestation"
status: resolved
parent: runtime-identity-issuance
tags: [custody, attestation, tpm]
open_questions: []
dependencies: []
related: []
---

# Custody and Attestation

## Overview

Define software/hardware custody evidence, proof of possession, nonce freshness, verifier anchors, host/workload binding, and degradation semantics.

## Decisions

### Assurance is an evidence result, not a caller claim

**Status:** accepted

Profile v1 defines four closed custody-assurance classes. Runtime requests may describe a provider, but only the issuing authority assigns the class after verifying evidence and local policy.

| ID | Class | Minimum meaning |
|---:|---|---|
| `0` | `software_ephemeral` | Runtime-local non-persisted key; proof of possession only |
| `1` | `os_protected` | Non-exportability asserted by an approved OS/platform keystore adapter; no portable hardware attestation |
| `2` | `hardware_nonexportable` | Fresh vendor-neutral or vendor-specific evidence proves the certified key is resident in an approved hardware boundary and non-exportable |
| `3` | `hardware_measured` | Reserved in profile v1; must not be issued because measured-boot/PCR policy is not yet standardized |

Class ordering is useful for minimum-policy checks but does not erase evidence details. A verifier reports the class, evidence profile, verifier authority, verification time, and any local policy qualification. Unknown classes fail closed. Class `1` is not promoted to class `2` merely because an OS reports that hardware may be involved.

### Profile-v1 hardware claim is deliberately narrow

**Status:** accepted

`hardware_nonexportable` proves all of the following at issuance:

1. the attested signing key matches the exact canonical public key and signature suite placed in the runtime certificate;
2. a proof operation was performed by that private key or by a credential cryptographically bound to it;
3. the key resides in the identified TPM, HSM, secure element, or TEE provider boundary;
4. provider attributes prohibit private-key export or duplication under the verified provider policy;
5. the evidence is bound to the authority's fresh challenge and the requested certificate context; and
6. the evidence chain terminates at a verifier trust anchor allowed by the issuer's local policy.

It does **not** prove trusted boot state, application identity, patch level, physical tamper resistance beyond the provider's certification, absence of firmware compromise, or that arbitrary runtime code is trustworthy. PCR, event-log, image, enclave-measurement, and reference-value appraisal are deferred to the future `hardware_measured` class.

The authority persists an evidence-verification result and digest; portable verifiers trust the signed certificate's assurance assertion according to their trust in the issuer. They do not need vendor attestation roots or the original evidence merely to verify an envelope.

### Challenge and certificate-context binding

**Status:** accepted

Before accepting custody evidence, the authority creates a CSPRNG challenge with:

- a 32-byte random nonce;
- a random 128-bit `challenge_id`;
- issuer identity;
- enrollment subject identity;
- requested runtime ID;
- requested canonical public-key digest;
- requested suite ID;
- issue time and expiry; and
- a single-use state bound to the authority's enrollment-nonce lifecycle domain.

The attested `qualifying_data` is:

```text
SHA-256("styrene-custody-challenge-v1\0" ||
       challenge_id || nonce || issuer_id || subject_id || runtime_id ||
       public_key_digest || suite_id || expires_at_ms)
```

Every variable-width identifier uses its canonical length-prefixed representation. The provider quote/report and the runtime-key proof of possession must bind this digest. If a provider cannot directly bind an externally supplied digest to the certified key, its evidence profile must define a verified chain from the quoting/attestation key to the runtime key plus a runtime-key signature over the digest.

Challenge creation, successful consumption, and certificate issuance participate in the atomic lifecycle transaction. A challenge is consumed on the first terminal verification attempt, whether verification succeeds or fails. It cannot be replayed to certify another key or retried with modified evidence.

### Freshness and trusted time

**Status:** accepted

The default challenge lifetime is five minutes. Issuers may configure a shorter lifetime but not one longer than ten minutes in profile v1. Creation and verification time come from the issuer's `TrustedClock`; caller timestamps are ignored.

Evidence must be generated after challenge creation and accepted before challenge expiry. Where provider evidence contains a monotonic clock, reset count, restart count, or security-version counter, the verifier records and checks it against previously observed state for that attestation identity. Counter rollback, unexpected reset, duplicated attestation identity, or inability to advance the issuer's rollback checkpoint yields `Indeterminate` or failure according to operation policy; it never silently establishes hardware assurance.

The attestation establishes custody only at certificate issuance. Profile v1 does not claim continuous attestation. Therefore a `hardware_nonexportable` runtime certificate has a maximum lifetime of 24 hours and must obtain fresh custody evidence whenever a new key is certified. Renewal with the same key may use fresh proof and evidence under a new challenge, but no attestation result is reused across certificates.

### TPM 2.0 minimum evidence profile

**Status:** accepted

A TPM 2.0 claim for class `2` minimally supplies or references:

- TPM `Quote` or `Certify` attestation structure with a valid magic value and expected attestation type;
- `extraData` equal to the custody challenge digest;
- the attested runtime-key `TPMT_PUBLIC` or its canonical digest;
- object attributes proving `fixedTPM`, `fixedParent`, `sensitiveDataOrigin`, and absence of an export-capable duplication policy;
- the attestation-key public area and signature;
- an endorsement/attestation credential chain or a locally enrolled attestation-key binding to an approved trust anchor;
- TPM manufacturer, model/family, firmware version, reset/restart counters, and clock-safety state when available; and
- a proof-of-possession signature by the runtime key over the custody challenge digest unless the verified `Certify` structure directly establishes possession and binding under the issuer's evidence profile.

Profile v1 does not require PCR selection or event-log appraisal. A quote that includes PCRs may be accepted, but PCR values do not raise assurance above class `2` and are not interpreted unless a later measured profile is explicitly selected.

TPM algorithms must be compatible with the profile-v1 signature registry and local policy. RSA storage or attestation keys may certify an Ed25519/P-256/RSA runtime key only when the TPM/provider evidence supplies a sound cryptographic binding; matching labels or handles is insufficient.

### TEE, HSM, secure-element, and OS profiles

**Status:** accepted

Non-TPM hardware may attain class `2` only through a registered evidence profile that defines:

- exact evidence and endorsement-chain formats;
- challenge-binding location;
- runtime-key binding and proof of possession;
- non-exportability attributes and forbidden states;
- verifier trust anchors and revocation inputs;
- freshness counters or replay controls;
- canonical evidence-result fields and rejection vectors; and
- supported signature suites and provider transformations.

Examples include platform-specific TEE reports, PKCS#11/HSM attestation mechanisms, and secure-element certificate chains. Generic PKCS#11 attributes such as `CKA_EXTRACTABLE=false` without independently authenticated provider evidence do not establish class `2`; they may qualify for class `1` under local policy.

OS keystore assertions qualify for class `1` only through an approved adapter that verifies key identity, access-control flags, and proof of possession. Software adapters without trustworthy protection metadata remain class `0`.

### Evidence storage and certificate fields

**Status:** accepted

A runtime certificate carries only bounded, portable custody results:

- `custody_class`;
- `evidence_profile_id` or `null`;
- SHA-256 `evidence_digest` or `null`;
- `attestation_verifier_id` or `null`;
- `attested_at_ms` or `null`; and
- `host_subject_binding`.

All five attestation fields are non-null for class `2` and null for class `0`. Class `1` uses a registered OS evidence-profile ID and verification metadata but may reference only a local evidence receipt rather than portable vendor evidence.

Raw evidence is stored or externally referenced under issuer retention policy and is never embedded in the core runtime certificate. Its canonical evidence envelope is bounded to 65,535 bytes; larger vendor collateral and event logs use digest-bound references with independent retrieval limits. The evidence digest commits to the complete canonical evidence envelope, including collateral references and verifier inputs.

The host/subject binding is an opaque canonical identifier meaningful to the issuer's enrollment domain. It is not exposed in ordinary A2A envelopes. A certificate cannot be moved to a different runtime ID, subject, or public key because all are included in the challenge and signed certificate.

### Degradation never relabels a key

**Status:** accepted

If required hardware is absent, evidence is invalid or stale, trust anchors are unavailable, or verification is indeterminate, issuance follows explicit policy:

- if hardware class `2` is mandatory, issuance fails closed;
- otherwise, the runtime generates a fresh software key and receives a class `0` certificate marked degraded;
- a key that failed hardware attestation is never certified as software merely by changing its label;
- class `1` may be issued only when its independent OS evidence profile succeeds;
- recovery from class `0` or `1` to class `2` creates a fresh runtime key, runtime ID, and certificate; and
- authorization policy may restrict degraded identities by operation, resource, network, or duration.

Verification returns structured custody evidence and degradation status rather than a boolean. Authentication success does not imply that custody policy is satisfied.

### Verifier anchors and revocation

**Status:** accepted

Attestation roots are local issuer policy, not globally embedded profile constants. An evidence profile identifies the expected root namespace and validation procedure; the issuer configures allowed roots, intermediates, manufacturer allowlists, minimum firmware policy, and collateral freshness.

Root or collateral revocation discovered after issuance does not rewrite certificate bytes. It may suspend or revoke affected runtime certificates through lifecycle records. For newly received executable work, local policy may require current certificate-revocation state even when the original attestation was valid.

## Required conformance cases

Profile-v1 implementations must test:

- exact challenge digest construction and canonical identifier framing;
- successful proof of possession for every supported signature suite;
- wrong nonce, challenge ID, issuer, subject, runtime ID, key digest, suite, or expiry;
- challenge replay, duplicate terminal submission, expiration boundaries, and trusted-clock rollback;
- TPM wrong magic/type, malformed public areas, missing required object attributes, unsafe clock, counter rollback, invalid AK chain/signature, key mismatch, and unsupported algorithms;
- class inflation attempts from software or unauthenticated PKCS#11 attributes;
- evidence-envelope byte limits and digest mismatches;
- fail-closed mandatory hardware policy and fresh-key degraded fallback; and
- certificate verification without access to vendor evidence.

## Assessment

The profile-v1 minimum is key residency and non-exportability with fresh, challenge-bound proof. It intentionally does not claim measured boot. This supports TPM 2.0 and registered older HSM/secure-element paths while keeping portable certificate verification independent of vendor infrastructure.

No unstated assumption remains that every provider exposes the same quote format. TPM has a normative minimum; other hardware reaches class `2` only through registered evidence profiles with equivalent security properties.

## Open Questions

None for profile-v1 custody classes, minimum evidence, and freshness. Concrete non-TPM evidence-profile registrations are implementation artifacts and can be added without weakening these requirements.
