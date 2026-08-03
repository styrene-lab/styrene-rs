# Styrene Identity and Signum Architecture

Status: proposed
Owners: `styrene-identity`, Signum, `styrened`, Auspex

## 1. Decision summary

Styrene uses portable, owner-signed identity records as authority. Signum is the optional identity lifecycle, verification, distribution, and operator-management service. It is not a mandatory online authority and no database becomes global truth merely by storing a record.

The default installation runs Signum as a separate process supervised by `styrened`. Auspex and `styrened` use the same versioned, capability-scoped Signum API. Advanced deployments may operate Signum independently or replicate signed records among Signum nodes.

This architecture is broader than A2A. The linked A2A profile is `docs/a2a-identity-binding-profile.md`.

## 2. Trust planes

The following planes remain semantically separate:

| Plane | Purpose | Proof does not imply |
|---|---|---|
| RNS identity | Destination/link authentication and delivery | Styrene principal or agent authority |
| Mesh certificates | Channel security, bearer admission, TLS/mTLS | A2A authorship or work authorization |
| Styrene principal identity | Owner, host, service, agent, and workload ownership | Permission for a specific operation |
| Runtime identity | One process/workload incarnation signing domain messages | Work authorization |
| Meridian policy | Permission for an authenticated interaction | Cryptographic authenticity |

Signum presents a unified lifecycle surface through adapters but does not replace RNS or mesh issuance semantics. Cross-plane correlation requires explicit signed binding records. Derived keys are not assumed linkable merely because Styrene can derive them from one root.

## 3. Identity hierarchy

```text
Owner identity
  ├── AgentIdentityRecord
  │     └── agent authority key
  │           └── RuntimeCertificate
  ├── HostIdentityRecord
  ├── DurableWorkloadIdentityRecord
  │     └── workload/orchestrator issuer
  │           └── RuntimeCertificate
  └── lifecycle, recovery, and binding records
```

### 3.1 Agent identity

An owner may create many agents. Agent identity is stable and independent of runtime and authority-key rotation.

- Generate an immutable random UUID at agent creation.
- Derive a compact identifier from a domain separator, owner identity ID, and UUID using a collision-resistant hash.
- Publish `styrene:agent:<derived-agent-id>`.
- Do not embed the owner ID in the URI; ownership is proved ad hoc by `AgentIdentityRecord`.
- Keep display name mutable and outside identity derivation.
- The identifier conveys no authority by itself.

### 3.2 Agent authority

The owner authorizes an agent authority key. The agent authority issues runtime certificates. An `AgentIdentityRecord` is immutable and content-addressed; mutable display metadata, issuer membership, and lifecycle state are represented by successor records in the owner domain revision chain. Every authority record has a stable key ID, public key, permitted certificate profiles, not-before/not-after interval, and explicit lifecycle state.

Routine rotation supports an overlap window:

- old and new authority keys may issue during the declared transition;
- after cutoff, the old key cannot issue new certificates;
- certificates issued before routine retirement remain valid until expiry;
- authority compromise immediately invalidates every certificate it issued.

Rotation and revocation records MUST identify the exact predecessor key and record digest, use a strictly increasing owner-domain revision, and be signed by the current owner or configured recovery threshold. An authority key cannot sign its own promotion, extend its own validity, clear its own suspension, or reconcile a fork in its lifecycle.

### 3.3 Runtime identity

Every process incarnation receives:

- a fresh random `runtime_id`;
- an independently generated ephemeral signing key;
- a runtime certificate;
- key version `1` initially.

Restart never reuses a runtime ID, private key, or certificate. Certificate renewal retains the runtime ID, generates a new key, and increments a strictly monotonic key version. Gaps are allowed; reuse or rollback is rejected. Renewal overlap equals `acceptable_clock_drift`, default five minutes.

Default runtime certificate lifetime is 24 hours and configurable. Commands cannot be newly signed after expiry. Existing tasks do not stop merely because the certificate or accepting command envelope expires.

Runtime key URI:

```text
styrene:key:<agent-id>:runtime:<runtime-id>:<key-version>
```

The URI conveys no authority; the certificate binds it to a key and issuer.

## 4. Workloads and orchestration

Containers, VMs, and subprocesses default to short-lived runtime identities. Durable workload identity is explicitly supported for long-running logical services.

- A durable workload remains stable across restart, rescheduling, image update, and host migration.
- Every replica/incarnation has a unique runtime ID, key, and certificate.
- Migration creates a new runtime identity bound to the destination host.
- A workload record declares permitted instance issuers: workload authority, scoped orchestrator, or both.
- Orchestrator issuers are limited to explicit workload IDs or namespaces.
- Authority keys must never be embedded in images.
- Replicas normally share one durable workload identity.

A workload may cap simultaneously active runtime certificates. Partition-tolerant issuers receive signed quotas:

- default local quota: 10 active replicas per authorized issuer;
- default quota lease: 24 hours;
- renewal overlap does not consume another replica slot;
- exceeding quota fails closed;
- reconciliation exposes conflicting or over-issued state.

## 5. Key custody and attestation

Hardware-backed, non-exportable keys are the preferred posture. Software keys are an explicit graceful-degradation path, never a silent equivalent.

Supported custody modes include:

- transient TPM/HSM key;
- platform or OS-protected signer;
- runtime-local memory key (default software fallback);
- daemon-memory key where explicitly configured.

Runtime keys are independently generated from the OS CSPRNG and are never derived from owner, authority, or shared runtime seeds. Runtime-local software keys are zeroized on exit and must not be serialized, logged, or included in crash dumps. Memory locking is hardening, not equivalence to a TPM.

When required hardware is unavailable, the runtime starts degraded and may still sign using a software-backed certificate. Receivers enforce minimum custody policy. Recovery to hardware generates a new key, runtime identity, and certificate rather than relabeling a software key.

### 5.1 Attestation

The authority verifies custody evidence before recording a hardware claim:

1. runtime submits its public key and requests certification;
2. authority returns a random, short-lived, single-use nonce;
3. TPM quote or certify evidence and runtime-key proof bind the challenge digest, requested runtime identity, and runtime public key;
4. authority verifies and consumes the challenge;
5. authority issues the certificate.

Profile v1 classifies verified hardware custody as key residency and non-exportability only. TPM evidence has a normative quote/certify minimum; TEE, HSM, secure-element, and OS-backed claims require registered evidence profiles with equivalent challenge/key binding and explicit assurance classes. PCR/measured-boot policy is reserved for a future profile with deployment-specific baseline, update, and recovery rules. Failed or indeterminate attestation follows explicit policy and defaults, when permitted, to a fresh software key and degraded certificate rather than relabeling the failed key.

### 5.2 Host binding

Runtime certificates bind to a host identity, but envelopes do not expose the stable host ID. Hardware-backed host identity survives OS reinstall. Software-backed fallback requires owner authorization and explicit recovery after reinstall. VMs, containers, subprocesses, and workloads use an enrollment subject abstraction rather than assuming every subject is a physical host.

## 6. Certificate profile and distribution

Minimum runtime certificate fields:

```text
profile_version
certificate_id / serial
agent_id or durable_workload_id
runtime_id
runtime_public_key
signing_key_id and key_version
issuer_key_id
host/subject identity reference
not_before / not_after
custody mode and assurance
attestation evidence digest/reference?
issuer signature
```

Certificate identity and digest rules:

- certificate and record digests use SHA-256 over canonical profile bytes;
- `certificate_id` is a random 128-bit serial generated by the issuer and encoded canonically;
- digest equality, not certificate ID equality, establishes content identity;
- `(issuer_key_id, certificate_id)` MUST be unique; reuse with different bytes is a permanent integrity conflict;
- string identifiers are normalized and validated before signing; verification compares canonical bytes, not presentation aliases;
- profile v1 is closed: unknown fields fail verification; extension requires a new profile version or a protected, separately versioned extension object declared by the base schema.

### 6.1 Canonical encoding

All signed profile-v1 records use deterministic CBOR with a closed schema:

- integer map keys only, strictly increasing, with exactly one occurrence each;
- shortest integer and length encodings;
- definite-length maps, arrays, byte strings, and text strings only;
- no floats, tags, indefinite-length items, duplicate keys, or trailing bytes;
- protocol identifiers are ASCII and validate in canonical form; display labels are never security identifiers;
- arbitrary human display text must already be UTF-8 NFC and is rejected rather than silently normalized before signing;
- hashes cover the exact canonical bytes produced by the owning profile encoder;
- verifiers decode, validate, re-encode, and byte-compare before accepting a signature;
- text normalization is allowed only for explicitly human-display fields. Security-significant names, resource selectors, host/workload identifiers, trust domains, key IDs, certificate IDs, and protocol identifiers are restricted to profile-defined canonical ASCII/byte forms and are never Unicode-normalized or case-folded.

Every signed record type has a unique length-prefixed domain separator framing:

```text
u16be(domain_separator_length) || domain_separator_ascii ||
u16be(profile_version) || u32be(canonical_cbor_length) || canonical_cbor
```

The separator is a centrally registered ASCII constant and cannot be selected by API callers. Immutable golden vectors cover minimum, maximum, and rejection cases. Profile version selects both schema and canonicalization rules; old bytes are never reinterpreted under new rules.

Maximum verification chain depth is four records. Expected hierarchy is owner, optional owner continuity, agent/workload authority, runtime.

Size limits:

- core runtime certificate: 16 KiB maximum;
- attached profile-v1 bootstrap bundle: 65,535 bytes maximum;
- adapters enforce independently lower inline limits;
- above the adapter threshold, use digest plus content/resource reference;
- large TPM evidence and future PCR logs remain external;
- identity material never enters transport headers or routing metadata.

Certificates are referenced by default and may be attached on first contact. Attachments are digest-bound. A validated certificate is cached by ID and digest until expiry or revocation. Conflicting content for one certificate ID is an integrity violation. Referenced retrieval is bounded by bytes, time, redirects, parse depth, and chain depth.

## 7. Time, expiry, and lifecycle

One configurable `acceptable_clock_drift`, default five minutes, governs backward-clock tolerance, future envelope skew, renewal overlap, and certificate-boundary comparisons where appropriate. Durations and timestamps are unsigned millisecond values with checked arithmetic; overflow, underflow, `not_after <= not_before`, and values outside the implementation's supported epoch range fail validation.

`processing_time` comes from a verifier-owned `TrustedClock`, never from the envelope, transport adapter, or caller-provided API field. Tests may inject a clock implementation; production API clients cannot select processing time or drift. Policy may tighten drift per operation but cannot exceed the locally configured ceiling.

Persist the last trusted wall-clock observation in a crash-safe monotonic checkpoint bound to the local trust-store generation. Significant rollback blocks new executable work while preserving permitted degraded work. If rollback detection state is missing, corrupt, cloned, or cannot be durably advanced, time-sensitive verification is `Indeterminate` rather than silently resetting its baseline.

Backup restore, database migration, or machine cloning cannot copy this checkpoint as ordinary data. A restore enters `rollback_recovery_required` and may verify timeless historical signatures but cannot issue certificates, sign executable commands, consume enrollment/recovery nonces, or accept new executable work. Recovery requires either: (a) unsealing a migration token bound to destination machine identity, source generation, target generation, backup digest, and expiry; or (b) a canonical recovery-threshold proposal authorizing a new checkpoint baseline. Completion rotates local store identity, advances generation, records source and destination checkpoint digests, consumes the token/proposal atomically, and revokes further use of the source clone for mutable authority. Concurrent live clones are a fork and remain `Indeterminate` until reconciled.

A signer-controlled creation timestamp is never proof that a key was trusted. Newly received executable objects require current trust. Persisted acceptance records retain local observation time and evidence references.

Lifecycle records distinguish:

- routine retirement;
- administrative revocation;
- compromise revocation;
- emergency suspension.

Compromise of an authority immediately invalidates all certificates it issued. A proven isolated runtime-key compromise revokes only that runtime. If isolation cannot be established—or host, signer service, entropy, authority, or shared custody may be affected—revoke all affected agent runtimes and rotate authority as needed. Uncertainty defaults to the broader blast radius.

Emergency automation may suspend broadly through a narrow capability. Suspension blocks new executable work immediately, never auto-expires, and requires fresh operator authorization to lift or convert to permanent revocation.

## 8. Signed record graph and synchronization

Signed records, not a mutable central database, are authoritative. Signum stores, indexes, verifies, and distributes replicas.

Each authority/domain has a signed monotonic revision chain:

```text
domain_id
authority_key_id
revision
record_kind
record_digest
previous_record_digest
issued_at
```

`knowledge_as_of` is local synchronization metadata and MUST NOT be signed into, or ordered as part of, an authority's canonical revision record. Each replica records `observed_at`, source, and synchronization cursor separately. This prevents a distributor from rewriting the apparent freshness of authoritative records.

Revision chains are per explicitly named lifecycle domain, not merely per key. Profile v1 recognizes only this inventory:

| Domain | Typed canonical key | Superior reconciler |
|---|---|---|
| owner state | trust domain | recovery threshold |
| agent/workload authority | trust domain + authority ID | owner or recovery threshold |
| runtime-certificate lifecycle | authority ID + subject ID | issuing authority |
| revocation/suspension | issuing authority ID | owner; recovery threshold after compromise |
| enrollment nonce consumption | enrollment issuer ID | same issuer; no unconsume operation |
| replica quota leases | issuer ID + replica ID | issuing authority |
| recovery policy | trust domain + recovery epoch | prior threshold or owner where policy permits |
| client/API grants | granting authority + client ID | granting authority |
| audit purge authorization | trust domain | owner plus configured recovery threshold |

A record type absent from this inventory cannot mutate trusted state in profile v1. Domain keys are typed tuples, never concatenated strings. Cross-domain transactions list all expected heads and commit atomically; partial advancement is invalid. A revision is accepted only when its predecessor is present or supplied in the same bounded bundle. Gaps remain pending and cannot advance trusted head state.

Revocation dominates authorization. Same-revision or same-predecessor divergent successors yield `Indeterminate`; sensitive operations fail closed. A higher numeric revision does not automatically resolve a fork. No timestamp, arrival order, replica priority, or digest-order last-writer-wins rule resolves trust conflicts. Only a canonical reconciliation record signed by a superior owner authority or the predeclared recovery threshold may name the winning branch and all rejected branch digests. Conflicting records remain in history.

A local bootstrap transaction creates revocation baseline revision `0`. During Signum outage, `styrened` may use cached evidence according to per-operation freshness policy. Sensitive execution defaults to one-hour maximum cached revocation age, configurable by risk class. Never-synchronized state fails closed for sensitive work.

## 9. Signum deployment and API

Signum is a separate process supervised by `styrened` by default. `styrened` deploys it, waits for bounded readiness, reports status to Auspex, and starts degraded if Signum is unavailable. Externally managed Signum is supported.

API transports:

- Unix-domain socket or Windows named pipe with OS peer credentials by default;
- HTTPS/mTLS for containers, remote hosts, or split deployments;
- one versioned API and authorization model across transports;
- loopback location alone never grants trust.

Every API request is authenticated as a stable client identity and authorized against an explicit capability grant. OS peer credentials are mapped through local configuration; they are not themselves Styrene principals. Remote certificates bind to the same client identity model. High-impact requests carry a unique operation ID and are replay-protected in durable storage. Mutating APIs are idempotent by operation ID and return the original committed result on retry. Capability grants are deny-by-default, resource-scoped, time-bounded where practical, and cannot be delegated unless explicitly marked delegable.

Verification endpoints accept object bytes/references and policy context, but production callers cannot supply authoritative clock time, revocation freshness, custody assurance, or a precomputed verification disposition. Those facts come from Signum's trusted clock and local verified store.

Access is capability-scoped, for example:

```text
identity.verify
runtime.issue
runtime.renew
runtime.revoke
authority.rotate
trust.approve_anchor
attestation.verify
records.distribute
audit.read
```

Auspex calls Signum directly with its own scoped identity for identity management. High-impact actions additionally require fresh operator authorization. Automated emergency capability may suspend but cannot restore authority, issue replacement authority, or approve anchors.

When Signum is unavailable, `styrened` uses cached verification, remains observable, and permits policy-approved degraded work. Issuance, rotation, and lifecycle management are unavailable unless `styrened` was explicitly provisioned as a scoped local issuer.

## 10. Authorization grant attenuation

Profile v1 grants are typed claim sets, not opaque strings:

```text
grant_id and profile_version
issuer and subject principal
trust_domain
allowed_actions: finite enum set
resource_scope: typed resource selector AST
constraints: typed key/value predicates from a closed registry
not_before / not_after
max_delegation_depth
delegable
parent_grant_digest?
issuer_signature
```

Attenuation is structural and decidable. A child grant is valid only when:

- issuer is the parent subject and the parent is delegable;
- trust domain is unchanged;
- child actions are a subset of parent actions;
- child resource selector denotes a subset under the profile's typed selector algebra;
- each child constraint is equal or stricter according to its registry-defined partial order;
- validity is contained within the parent's interval;
- remaining delegation depth strictly decreases;
- the protected parent digest matches the exact parent grant;
- the complete ancestor chain is supplied or resolved, and every ancestor is valid and non-revoked at processing time.

Grant IDs are random serials only; digest identity is authoritative. Implementations index descendants by every ancestor grant digest so revocation of an ancestor invalidates the full descendant closure without requiring graph discovery at request time. Effective authorization is the intersection of all ancestor claims, never merely the leaf claims.

Profile v1 resource selectors support only exact resource IDs, typed namespace segments, and finite unions/intersections with bounded depth and cardinality. Selector and constraint values use canonical typed components, not strings requiring escaping. The subset algorithm has one normative implementation contract, deterministic normalization, and fixed complexity ceilings; implementations must not substitute heuristic solvers. No regex, glob, path-prefix string comparison, negation, arbitrary code, or externally defined comparator is permitted. If subset proof is unavailable, ambiguous, or exceeds complexity limits, attenuation fails. Unknown action, selector, or constraint types fail closed. Revocation targets grant digests and dominates descendant authorization.

## 11. First-run and delegated enrollment

Existing Styrene onboarding becomes a UI over a reusable, resumable Signum bootstrap state machine. It must stop treating a 64-byte RNS private identity as the Styrene owner identity.

Bootstrap stages separately establish:

- owner identity or delegated enrollment;
- host/enrollment-subject identity and custody;
- RNS setup through its native adapter;
- mesh certificate setup through its native adapter;
- signed cross-plane bindings;
- trust and revocation store.

Stages persist as pending, active, degraded, or retryable failure. The setup completion marker means minimum viable readiness, not that every optional plane succeeded.

Owner creation is optional only with a canonical `StyreneEnrollmentBundle`. Generic X.509, RNS, SSH, OAuth/OIDC, DID, JWT, transport credentials, or third-party attestations are ineligible as owner authority on their own. Parsers MUST reject ambiguous encodings, duplicate fields, unknown critical fields, trailing bytes, overlong strings, chain-order ambiguity, and identifiers that are not in canonical form.

An eligible delegated enrollment bundle MUST contain exactly one coherent enrollment transaction:

```text
profile_version and trust_domain
bundle_id and one-use nonce/challenge
created_at and expires_at
owner anchor fingerprint or reference
complete owner-to-enrollment authorization chain
enrollment-subject public key digest and requested subject kind
permitted host/workload/agent scope
permitted certificate profiles and custody minimum
maximum runtime/issuer quota and delegation depth
Signum API capability ceiling
recovery-policy reference
all record digests and canonical issuer signatures
```

The bundle MUST NOT contain an owner private key, reusable bearer secret, wildcard trust-domain adoption, unrestricted anchor approval, authority-rotation capability, recovery-policy mutation capability, or capability broader than its issuer possesses. Scope attenuation is checked structurally at every delegation hop; string-prefix matching is forbidden for namespace authorization.

A delegated bundle binds to a one-use challenge completed by a newly generated enrollment-subject key. Enrollment atomically validates the chain and domain, verifies proof of possession, checks expiry and policy attenuation, consumes the nonce, and persists the adopted records plus a durable consumption tombstone. A bundle is either committed in full or not adopted; partial trust-anchor, capability, or identity writes are rolled back. Replaying a consumed or expired bundle is a typed security failure.

Offline enrollment is supported:

- file or QR carries the complete chain for pristine nodes;
- manual self-contained word phrase carries compact one-use authorization and requires a preinstalled owner anchor;
- no short online retrieval code in profile v1;
- encodings are checksummed and authenticated;
- duplicate offline use discovered later is a security conflict.

A pristine node never trusts a bundled owner anchor merely because the bundle is well encoded. Trust requires explicit fingerprint confirmation through an independent route. This approval is automatable over SSH with pinned host key, local console, provisioning image, configuration management, TPM fleet provisioning, or removable media. SSH is delivery evidence, not Styrene owner authority. Approval binds to the enrollment subject key or to a narrowly scoped issuer authorized to certify such subjects.

## 11. Recovery

Operational recovery uses independent recovery authorities, not owner-secret reconstruction.

- recommended preset: 2-of-3;
- generic M-of-N support;
- majority-of-odd presets such as 2-of-3, 3-of-5, and 4-of-7;
- authorities sign the exact same canonical proposal;
- proposal records include policy version, authority set, threshold, and digest;
- default proposal lifetime: 24 hours;
- proposal is consumed only on successful atomic commit;
- uncertain outcome becomes `Indeterminate` pending reconciliation.

Recovery policy has a monotonic `recovery_epoch`. A proposal binds to exactly one epoch and its complete authority-set and threshold digest. Epoch change immediately prevents new approvals under prior epochs but does not silently invalidate an already complete approval set: a fully approved old-epoch proposal may execute only if the new policy's transition record explicitly names its proposal digest during a bounded grace period. That transition record itself must be authorized under the old epoch's threshold (or an already valid owner path allowed by the old policy); the new authority set cannot grandfather proposals unilaterally. Otherwise the proposal is cancelled. Approvals from different epochs or authority sets never combine. Removing or compromising an authority requires a new epoch; no authority identifier is reused within a trust domain.

Shared custody among recovery authorities warns but is not universally rejected. High-assurance policy may require distinct attested devices. Recovery policy changes require the current threshold or owner authorization. Shamir secret sharing is deferred as a separate offline disaster-recovery mechanism, never the routine authorization mechanism.

Recovery proposal fields also include a globally unique proposal ID, target trust domain, exact operation type, target record digests, expected current trusted-head digests/revisions, resulting authority or policy digest, and a one-use execution nonce. Approvals bind to the complete proposal digest and cannot be transplanted across domains or operation types. Signum durably records proposal state and approval-set digests before execution; execution uses compare-and-swap against the expected heads and writes a consumption tombstone atomically with the recovered state. Expiry, cancellation, fork, threshold-set change, or partial commit yields a typed failure or `Indeterminate` requiring reconciliation—never automatic retry under newly observed state.

## 11.1 Abuse resistance

Rate-limit enrollment attempts, typed signing requests, certificate issuance, recovery proposals, approval submissions, and verification misses by authenticated principal, source, and affected trust domain. Persist counters and tombstones required to prevent restart from resetting one-use or high-impact limits. Emit audit records for throttling and lockout without logging bearer credentials, enrollment bundles, private key material, raw authorization grants, or complete attestation evidence.

Do not expose an unrestricted arbitrary-byte signing oracle. Signing APIs accept a typed profile plus canonical object or reference, enforce domain separation internally, re-derive the signing input, and authorize key/profile/resource use. Callers cannot request arbitrary bytes under owner, authority, recovery, or audit keys. High-impact keys may require local confirmation or quorum according to policy.

Remote endpoints use bounded request bodies, bounded chains and bundles, bounded concurrency, per-request deadlines, and response-size limits. Expensive attestation and chain validation occurs only after cheap framing, profile, identity, nonce, and quota checks. Unknown identifiers do not trigger unconstrained network fetches; discovery is separately rate-limited and returns `Indeterminate` on exhaustion.

## 12. Verification and policy handoff

`styrene-identity` verification returns structured evidence, not a boolean. Dimensions include:

- identity and chain validity;
- key and runtime binding;
- custody and host assurance;
- certificate lifetime;
- revocation status and freshness;
- synchronization/fork state;
- channel evidence supplied by adapters;
- degraded-mode reasons and warnings.

Meridian policy separately evaluates the requested action. Every accepted executable operation persists policy disposition/version and verification-evidence references/digests.

## 13. Audit and retention

Profile v1 uses a structured local JSON Lines operational audit log. It is append-only operational evidence, not yet tamper-proof.

Each entry contains schema version, event ID, timestamp, actor/client, action, target IDs, outcome, reason, and correlation ID. It never contains secrets, raw private keys, tokens, or full attestation evidence.

Defaults:

- rotate at 10 MiB;
- retain 10 rotated files;
- terminal-task evidence: 90 days;
- incident, suspension, revocation, and recovery evidence: one year;
- minimal tombstone after detailed deletion: one additional year.

All values are highly configurable globally and by domain/risk class. Evidence retention is never shorter than retained task references. Shortening policy affects new evidence only; early purge requires a separately authorized action. Increasing policy extends existing evidence if present. Ordinary purge requires fresh owner/operator approval. Security lifecycle evidence requires owner/operator plus recovery threshold. Purge tombstones cannot be removed early.

Future audit records become hash-chained, signed, optionally mesh-replicated, and externally anchorable. Stable event IDs and schema versions preserve migration from v1 logs.

## 14. Crate and component ownership

### `styrene-identity`

Owns canonical records, domain-separated signing inputs, cryptographic verification, certificate-chain validation, conflict rules, signer/custody abstractions, and attestation-verifier contracts.

The current `IdentitySigner::root_secret()` requirement is incompatible with non-exportable hardware. The revised API makes `sign`, `public_key`, `key_reference`, and challenge-based custody evidence primary. Secret export is optional and software-only. Custody should be structured properties rather than one ordinal tier.

### Signum

Owns lifecycle orchestration, durable signed-record graph, synchronization, certificate caches, revocation index, attestation workflow, bootstrap, audit, operator APIs, and adapters to RNS/mesh issuance.

### `styrened`

Owns local enforcement composition, cached evidence required for accepted work, trusted-clock integration, Signum supervision, degraded operation, and optional explicitly delegated local issuance.

### Auspex

Owns rich operator projections and workflows. It exposes separate identity, custody, freshness, channel-binding, recovery, and synchronization dimensions and may distill them into smaller health views.

## 15. Deferred work

- PCR/measured-boot policy and event-log handling;
- remote Signum federation protocols;
- hash-chained replicated audit;
- optional Shamir disaster-recovery packages;
- transport-specific inline thresholds based on field measurements;
- policy profiles for hardware-required workloads;
- owner-key continuity ceremony details.

## 16. Unstated assumptions to validate

- [assumption] Signum source can be brought under Styrene ownership or replaced without an incompatible external API commitment.
- [assumption] Supported TPM providers can create transient non-exportable signing keys and produce evidence binding a nonce and public key.
- [assumption] The existing onboarding can delegate bootstrap state to a separate process without losing clean-room installation support.
- [assumption] Five minutes is adequate default clock drift and renewal overlap; field data may change it.
- [assumption] The selected runtime certificate and bootstrap limits cover real TPM/vendor chains; fixtures must test representative providers.
