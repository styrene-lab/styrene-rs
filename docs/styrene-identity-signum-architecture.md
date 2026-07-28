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

The owner authorizes an agent authority key. The agent authority issues runtime certificates. Routine rotation supports an overlap window:

- old and new authority keys may issue during the declared transition;
- after cutoff, the old key cannot issue new certificates;
- certificates issued before routine retirement remain valid until expiry;
- authority compromise immediately invalidates every certificate it issued.

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
3. TPM quote binds nonce and runtime public key;
4. authority verifies and consumes the challenge;
5. authority issues the certificate.

Profile v1 proves key residency and non-exportability. PCR/measured-boot policy is reserved for a future profile with deployment-specific baseline, update, and recovery rules. Failed attestation follows policy and defaults to a fresh software key in degraded mode.

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

One configurable `acceptable_clock_drift`, default five minutes, governs backward-clock tolerance, future envelope skew, renewal overlap, and certificate-boundary comparisons where appropriate.

Persist the last trusted wall-clock observation. Significant rollback blocks new executable work while preserving permitted degraded work.

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
authority_id
revision
previous_record_digest
knowledge_as_of
```

Revocation dominates authorization. Same-revision conflicts yield `Indeterminate`; sensitive operations fail closed. No timestamp or digest-order last-writer-wins rule resolves trust conflicts. Only a superior owner authority or predeclared recovery authority may issue reconciliation. Conflicting records remain in history.

A local bootstrap transaction creates revocation baseline revision `0`. During Signum outage, `styrened` may use cached evidence according to per-operation freshness policy. Sensitive execution defaults to one-hour maximum cached revocation age, configurable by risk class. Never-synchronized state fails closed for sensitive work.

## 9. Signum deployment and API

Signum is a separate process supervised by `styrened` by default. `styrened` deploys it, waits for bounded readiness, reports status to Auspex, and starts degraded if Signum is unavailable. Externally managed Signum is supported.

API transports:

- Unix-domain socket or Windows named pipe with OS peer credentials by default;
- HTTPS/mTLS for containers, remote hosts, or split deployments;
- one versioned API and authorization model across transports;
- loopback location alone never grants trust.

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

## 10. First-run and delegated enrollment

Existing Styrene onboarding becomes a UI over a reusable, resumable Signum bootstrap state machine. It must stop treating a 64-byte RNS private identity as the Styrene owner identity.

Bootstrap stages separately establish:

- owner identity or delegated enrollment;
- host/enrollment-subject identity and custody;
- RNS setup through its native adapter;
- mesh certificate setup through its native adapter;
- signed cross-plane bindings;
- trust and revocation store.

Stages persist as pending, active, degraded, or retryable failure. The setup completion marker means minimum viable readiness, not that every optional plane succeeded.

Owner creation is optional only with a canonical `StyreneEnrollmentBundle`. Generic X.509, RNS, SSH, OAuth/OIDC, DID, JWT, or transport credentials are ineligible as owner authority on their own.

A delegated bundle binds to a one-use challenge completed by a newly generated enrollment-subject key. Enrollment atomically validates the chain and domain, verifies the challenge, consumes the nonce, and persists the adopted records.

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

Shared custody among recovery authorities warns but is not universally rejected. High-assurance policy may require distinct attested devices. Recovery policy changes require the current threshold or owner authorization. Shamir secret sharing is deferred as a separate offline disaster-recovery mechanism, never the routine authorization mechanism.

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
