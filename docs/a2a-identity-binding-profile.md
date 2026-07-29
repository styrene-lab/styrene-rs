# A2A Identity-Binding Profile

Status: proposed
Owners: `styrene-a2a`, `styrene-identity`, Signum, `styrene-services`, transport adapters
Depends on: `docs/styrene-identity-signum-architecture.md`
Extends: `docs/a2a-integration-architecture.md`

## 1. Purpose

This profile authenticates a Styrene A2A envelope end to end without conflating authentication, transport security, or work authorization.

It defines:

- protected envelope fields;
- runtime-certificate reference and first-contact attachment;
- signer and verifier interfaces;
- trusted-time and lifecycle checks;
- transport channel evidence;
- structured verification output;
- the handoff to Meridian policy.

It does not define key custody, certificate issuance, Signum synchronization, or authorization-grant semantics. Those belong to the linked identity architecture and policy system.

## 2. Security claims

A successfully authenticated envelope proves:

> The protected envelope bytes were signed by a key whose valid runtime certificate binds that key to the claimed agent/workload, runtime incarnation, host/enrollment subject, and issuer chain under locally trusted Styrene records.

It does not independently prove:

- that the requested work is authorized;
- that the immediate transport peer authored the envelope;
- that globally current revocation state is available during a partition;
- that signer-controlled timestamps reflect external observation time;
- that a bearer receipt proves execution.

## 3. Required envelope fields

Profile v1 protects at least:

```text
profile_version
message_id
kind
source_agent_id
source_runtime_id
target_agent_id
target_runtime_id?
root_operation_id
task_id?
parent_task_id?
stream_id
sequence
created_at_ms
expires_at_ms?
content_type
payload_encoding
payload_schema
payload_digest
authorization_digest?
grant_reference?
signature_algorithm
signing_key_id
runtime_certificate_id
runtime_certificate_digest
attached_certificate_bundle_digest?
authorization_digest?
grant_reference?
attached_authorization_digest?
attachment_manifest_digest?
traceparent?
```

The protected attachment manifest contains type, canonical digest, and byte length for every inline certificate, grant, or evidence object. Exactly one authoritative digest identifies each semantic attachment. A reference and inline bytes may coexist only when both resolve to that digest; mismatch is a terminal integrity failure.

The signature bytes and attached object bytes are excluded from signing input. Their identifiers, digests, lengths, and criticality are protected. Transport route, address, topic, peer socket, and bearer receipt remain outside the signed domain payload. Omitted optional fields and explicit CBOR `null` are distinct: profile encoders MUST emit every schema field, using `null` only where the schema marks it optional; omission, default substitution, or alternate empty representations fail canonical byte comparison.

Unknown attachment types or unknown critical fields fail verification. Attachments have independent count, per-item, and aggregate byte limits within the envelope ceiling and are digested before expensive parsing. Parsing rejects duplicate manifest entries, duplicate semantic objects, trailing bytes, and ambiguous encodings.

The current draft must add explicit runtime-certificate reference/digest and attachment digest before profile-v1 field numbers freeze.

## 4. Canonical signing input

`styrene-a2a` owns deterministic construction of the protected signing input. It must:

- use explicit, profile-versioned field numbers;
- reject unsupported critical fields;
- include SHA-256 digests for A2A payload, embedded authorization, and optional attached certificate bundle;
- use a domain separator distinct from owner records, runtime certificates, RNS identities, and mesh certificates;
- produce byte-identical output across supported implementations;
- retain immutable golden vectors.

Recommended conceptual input:

```text
u16be(30) || "styrene-a2a-envelope-signing-v1" ||
u16be(1) || u32be(length(deterministic_cbor)) || deterministic_cbor(protected_fields)
```

The domain separator and framing are fixed by the profile and are not supplied by callers. The exact profile is frozen only after official SDK fixtures and complete signed-envelope vectors land. Profile v1 uses the identity architecture's deterministic-CBOR contract: closed schema, increasing integer keys, shortest encodings, definite lengths, no floats/tags/duplicate keys/trailing bytes, canonical ASCII protocol identifiers, and NFC-normalized human text. Verifiers decode, validate, re-encode, and byte-compare. Unknown fields fail profile-v1 verification rather than being silently ignored.

## 5. Command validity

Executable command envelopes require `expires_at_ms`.

- Default maximum acceptance window: one hour.
- Window is configurable by receiver policy.
- Window is `expires_at_ms - created_at_ms`.
- Expiry cannot exceed the signing runtime certificate’s validity.
- Envelope expiry controls first acceptance/execution, not task lifetime.
- Once atomically accepted, a task may run beyond envelope and certificate expiry.
- Late retries reconcile by message/task ID rather than re-executing stale commands.

Non-command envelope kinds use separate retention and acceptance policy.

The signer-controlled `created_at_ms` is never sufficient proof that a key was valid. For newly received executable work, the key must be trusted at processing time, certificate validity must cover the claimed creation time, and timestamp skew must fall within `acceptable_clock_drift`. Processing time comes only from a verifier-owned `TrustedClock`; production callers and transport adapters cannot supply or override it. Timestamp and duration arithmetic is checked, and overflow, underflow, unsupported epoch values, or invalid intervals fail structurally.

## 6. Runtime certificate reference and bootstrap

Normal envelope:

```text
runtime_certificate_id
runtime_certificate_digest
attached_certificate_bundle = none
```

First-contact envelope may attach one bounded bootstrap bundle:

```text
runtime_certificate_id
runtime_certificate_digest
attached_certificate_bundle_digest
attached_certificate_bundle
```

Limits inherited from the identity architecture:

- core runtime certificate: 16 KiB;
- profile-v1 attached bootstrap bundle: 65,535 bytes;
- chain depth: four records;
- transport adapters may impose lower inline limits;
- larger bundles use content/resource references.

Verification order:

1. enforce envelope and attachment byte/count limits before allocation where possible;
2. validate protected structure, manifest uniqueness, lengths, and digests;
3. verify the envelope signature with the referenced key before parsing expensive signer-supplied certificate or authorization bodies when key material is already cached;
4. resolve cached certificate by ID and digest, or parse the attached/retrieved bundle as untrusted bounded input;
5. validate chain, lifecycle, custody, subject/host binding, and revocation evidence;
6. if key material came from the attached bundle, verify the envelope signature immediately after the leaf key is structurally resolved and before expensive attestation/network checks;
7. evaluate transport channel evidence;
8. pass authenticated evidence to policy;
9. atomically persist acceptance, evidence references, and deduplication state.

A same-ID/different-digest certificate is an integrity conflict. Fetching by reference uses bounded bytes, timeout, redirects, parse depth, and negative caching. Failure to obtain a required certificate returns a specific unavailable/indeterminate result; it never falls through to chat or unauthenticated execution.

## 7. Signing interface

`styrene-a2a` must not access root secrets or hardware providers. It consumes a purpose-specific signer:

```rust
#[async_trait]
pub trait EnvelopeSigner: Send + Sync {
    fn signing_key_id(&self) -> &SigningKeyId;
    fn runtime_certificate_ref(&self) -> RuntimeCertificateRef;
    async fn sign_a2a_envelope(
        &self,
        request: TypedEnvelopeSigningRequest<'_>,
    ) -> Result<SignedEnvelopeBinding, SigningError>;
}
```

The implementation is supplied through `styrene-identity`/Signum custody providers. The API must be domain-specific rather than generic `sign(bytes)` at application call sites. Hardware implementations sign without exporting private material. Signum, not the caller, constructs or re-derives the complete protected envelope input from typed fields, compares any caller-provided digest, and returns the signature together with the exact protected digest and certificate reference used. A raw `canonical_input: &[u8]` method may exist only as an internal crate-private custody adapter after typed authorization and cannot be exposed over Signum RPC.

Before signing a command, the signer checks:

- runtime certificate is currently usable;
- key version is active and not suspended/revoked;
- required explicit expiry is present and policy-bounded;
- custody mode satisfies local emission policy;
- trusted clock is not in rollback-degraded state.

A degraded software-backed runtime may sign when policy allows; the certificate reports actual custody.

## 8. Verification interfaces

Portable identity verification owns key/certificate semantics:

```rust
pub trait EnvelopeIdentityVerifier: Send + Sync {
    fn verify_runtime_identity(
        &self,
        request: RuntimeIdentityVerificationRequest<'_>,
    ) -> Result<RuntimeIdentityEvidence, IdentityVerificationError>;
}
```

A2A orchestration composes identity evidence with the envelope signature:

```rust
pub struct EnvelopeVerificationRequest<'a> {
    pub envelope: &'a AgentEnvelope,
    pub channel_evidence: Option<&'a ChannelEvidence>,
    pub operation_risk: OperationRisk,
}
```

The verifier obtains processing time from its injected `TrustedClock` and drift limits from local policy. Public request DTOs do not contain authoritative time, freshness, or custody fields. Tests inject clock and policy implementations at verifier construction rather than per request.

Signum or the local verification store resolves:

- signing key ID and strict key version;
- runtime certificate ID/digest;
- agent/workload identity and issuer chain;
- runtime ID and host/enrollment-subject binding;
- validity, retirement, suspension, revocation, and authority compromise;
- custody and attestation evidence;
- revocation freshness and revision-fork state.

The verifier must reject:

- absent or malformed signature;
- unknown signature algorithm;
- unknown/conflicting certificate ID;
- key/agent mismatch;
- key/runtime mismatch;
- target-runtime mismatch when the protected target runtime is present;
- key version rollback/reuse;
- invalid, expired, suspended, or revoked chain according to operation policy;
- authority compromise affecting the leaf certificate;
- tampered protected fields, payload, authorization, or bundle;
- stale/unknown revocation state where policy requires freshness;
- chain depth or size excess;
- `Indeterminate` revision forks for sensitive work.

Verification alone does not consume sequence or message IDs. The acceptance boundary atomically checks and persists `(source_agent_id, source_runtime_id, stream_id, sequence)` plus `message_id` in the same transaction as task mutation. The same message ID with byte-identical protected digest is idempotent; the same ID with different digest is an integrity conflict. A sequence below or equal to the committed contiguous watermark is accepted only as an exact known replay, never as new work. A sequence gap is recorded and reconciled according to envelope kind; executable commands cannot bypass unresolved predecessor requirements where service policy demands ordering. Runtime ID changes begin a new sequence namespace but do not erase retained replay tombstones before their configured horizon.

## 9. Transport channel evidence

Adapters report authenticated facts; senders and adapters cannot lower local policy.

```text
ChannelEvidence
  bearer_kind
  authenticated_peer_identity?
  peer_host_binding?
  directness = direct | relayed | store_and_forward | unknown
  bearer_receipt?
  evidence_time
```

Local policy decides whether direct channel binding is required.

- Direct mode may require the authenticated mesh/RNS peer to match the runtime certificate’s host binding.
- Relayed, brokered, or store-and-forward delivery expects the immediate peer to differ; end-to-end runtime signature remains authoritative.
- Channel assurance is reported separately from identity authentication.
- Transport addresses, topics, and routes never enter the signed envelope.
- A valid RNS or mesh signature does not substitute for runtime-certificate verification.

## 10. Structured verification report

Verification returns evidence, not a boolean:

```text
EnvelopeVerificationReport
  envelope_authentication
  authenticated_principal
  agent_or_workload_id
  runtime_id
  signing_key_id / version
  runtime_certificate_id / digest
  certificate_chain_status
  custody_assurance
  host_or_subject_binding
  lifecycle_status
  revocation_revision / knowledge_as_of / freshness
  synchronization_status
  channel_binding_assurance
  created_at_assessment
  processing_time
  warnings / degraded_reasons
  evidence_references / digests
```

Cryptographic validity and policy acceptability remain distinct. Examples:

- signature valid, custody software-backed;
- signature valid, revocation state stale;
- signature valid historically, authority now routinely retired;
- cryptographically valid but authority compromised;
- identity valid, direct channel binding absent;
- lifecycle state indeterminate due to revision fork.

## 11. Policy handoff and persistence

`styrene-services` passes the report, action, resource, grant evidence, and operation risk to Meridian policy. Envelope authentication does not evaluate work authority.

Acceptance transaction atomically persists:

- message/deduplication ID and protected envelope digest;
- task/event mutation;
- verification disposition;
- policy version and disposition;
- verification evidence references/digests;
- local first-accepted time;
- stream watermark and outbound receipt/event.

A recognized invalid A2A envelope produces a typed protocol error and never falls through to chat.

Persisted historical records retain the verification result observed at acceptance. Newly arriving backdated objects cannot rely on `created_at_ms` to bypass retirement or revocation. Historical trust may later acquire warnings after compromise, but audit history is not silently rewritten.

## 12. Failure classes

The profile should distinguish at least:

```text
MalformedEnvelope
UnsupportedProfile
MissingExpiry
AcceptanceWindowTooLong
TimestampOutsideDrift
ExpiredEnvelope
MissingSignature
UnsupportedSignatureAlgorithm
SignatureMismatch
PayloadDigestMismatch
AuthorizationDigestMismatch
CertificateUnavailable
CertificateDigestMismatch
CertificateChainTooDeep
CertificateTooLarge
UnknownSigningKey
AgentBindingMismatch
RuntimeBindingMismatch
HostBindingMismatch
KeyVersionRollback
CertificateNotYetValid
CertificateExpired
CertificateSuspended
CertificateRevoked
IssuerCompromised
RevocationFreshnessUnknown
IdentityRevisionFork
CustodyInsufficient
ChannelBindingInsufficient
PolicyDenied
PolicyIndeterminate
```

External protocol errors should avoid leaking local paths, private topology, or unnecessary trust-store details. Internal audit records retain precise evidence references.

## 13. Local-first implementation sequence

1. Define canonical identity record and runtime certificate types in `styrene-identity`.
2. Replace the hardware-incompatible mandatory `root_secret()` signer contract with non-exporting purpose-specific operations.
3. Add an in-memory local trust/revocation store and trusted-clock test double.
4. Extend `AgentEnvelope` with runtime-certificate ID/digest and attachment digest/reference.
5. Write golden signing vectors including those fields.
6. Implement software runtime signer and verifier tests.
7. Add key/agent/runtime mismatch, timestamp, retirement, suspension, compromise, and stale-revocation tests.
8. Add channel-evidence tests independent of transport implementations.
9. Integrate Signum local API and certificate cache.
10. Add TPM custody provider and nonce-based residency attestation.
11. Integrate `styrene-services` acceptance and Meridian policy.
12. Add transport-specific inline/reference behavior.

## 14. TDD acceptance scenarios

Before service integration, tests must cover:

- software-backed valid signature;
- non-exporting mock hardware signer;
- signature insertion does not alter canonical input;
- mutation of every protected index changes verification;
- mutation of payload, authorization, and attached certificate is detected;
- key ID resolves but belongs to another agent;
- key belongs to agent but another runtime;
- renewal increments version and accepts drift-sized overlap;
- key-version reuse/rollback fails;
- command without expiry fails;
- one-hour default acceptance window is independent of task lifetime;
- malicious backdating cannot revive retired/revoked keys;
- authority compromise invalidates all issued runtime certificates;
- isolated runtime compromise does not invalidate independent runtimes;
- uncertain custody compromise applies broad blast radius;
- certificate cache accepts same ID/digest and rejects same ID/different digest;
- profile and adapter attachment limits are enforced;
- direct channel match, direct mismatch, and relayed delivery remain distinct;
- never-synchronized revocation state fails sensitive work;
- revision fork returns `Indeterminate`;
- audit persistence references the exact evidence and policy versions.

## 15. Deferred profile work

- federation and foreign issuer mapping;
- PCR/measured-boot policy;
- remote first-contact retrieval protocols;
- official bearer-specific inline thresholds;
- authority-grant subset semantics;
- COSE adoption versus the external deterministic-CBOR signature profile;
- complete signed-envelope interoperability vectors.

## 16. Open assumptions

- [assumption] The A2A SDK permits retaining official JSON payloads while Styrene signs a deterministic external CBOR index.
- [assumption] A 16 KiB core certificate and 65,535-byte bootstrap bundle cover representative TPM/vendor chains.
- [assumption] Local trusted-clock rollback detection can be persisted safely enough for the first slice.
- [assumption] Mesh and RNS adapters can provide authenticated channel evidence without modifying their native identity semantics.
- [assumption] Signum’s future API can expose the structured verifier without requiring a network round trip for every envelope.
