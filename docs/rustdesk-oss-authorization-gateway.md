---
id: rustdesk-oss-authorization-gateway
title: "RustDesk OSS Authorization Gateway"
status: seed
parent: authorization-grants
tags: [future-work, remote-access, rustdesk, oidc, cedar, grants]
open_questions:
  - "Can a stock RustDesk client or local helper populate the existing rendezvous token field, or is a managed client patch unavoidable?"
  - "Does an hbbs-only enforcement point remain sufficient while stock hbbr is WireGuard-only, including every direct and relay fallback path?"
  - "Should Styrene define a generic short-lived connection-grant profile reusable beyond RustDesk, or remain only an optional device-identity adapter?"
  - "What active-session termination semantics are required after user or device revocation?"
  - "Which desktop platforms must the first managed-client pilot support?"
dependencies:
  - authorization-grants
  - api-authentication-and-capabilities
  - trusted-time-and-rollback
related:
  - styrene-policy-meridian
---

# RustDesk OSS Authorization Gateway

## Overview

Preserve the RustDesk user experience while replacing license-gated identity and authorization features with a fully open-source control plane. The candidate work stream is a narrow fork of the OSS RustDesk rendezvous server (`hbbs`) deployed alongside stock OSS `hbbr`, with Cognito/OIDC authentication, embedded Cedar policy evaluation, short-lived signed connection grants, and durable audit records.

This is a **future possible work stream**, not an implementation commitment. The immediate decision gate is a bounded protocol spike proving that the existing RustDesk `token` field can carry an enforceable grant without changing the protobuf schema or broadly forking the clients.

## Motivation

Stock RustDesk OSS uses a server-wide `licence_key` at the rendezvous and relay boundaries. It does not perform user-to-device authorization. OIDC login in an auxiliary API console does not gate `hbbs` connection establishment, so it cannot satisfy unattended-access requirements that include centralized user/device authorization, revocation, and auditable access decisions.

Source assessment against current upstream snapshots found:

- `PunchHoleRequest` already carries both `token` and `licence_key`.
- Rendezvous-side `RequestRelay` also carries `token`.
- OSS `hbbs` ignores the token and checks only the shared key.
- OSS `hbbr` pairs streams using a shared key and caller-supplied relay UUID, without knowing the initiating OIDC principal.

The smallest credible enforcement point is therefore inside `hbbs`, before it forwards punch-hole or relay requests.

## Candidate boundary

```text
Cognito / external OIDC
          │ Authorization Code + PKCE
          ▼
RustDesk auth broker
  - validate immutable (issuer, subject)
  - bind a managed source device
  - load target device attributes
  - evaluate embedded Cedar policy
  - mint a 60–120 second signed grant
  - persist the authorization decision
          │ existing RustDesk token field
          ▼
Forked hbbs
  - verify signature, audience, target, source, action, time
  - consume a single-use grant ID
  - deny before rendezvous forwarding
  - emit admission/denial evidence
          │ authorized relay UUID
          ▼
Stock hbbr, reachable only through WireGuard
          │
          ▼
Stock target agent retains local consent/password controls
```

### Keep the fork narrow

The `hbbs` delta should be limited to:

- a grant-verifier interface at every rendezvous admission path;
- deny-by-default handling for missing, malformed, expired, replayed, or mismatched grants;
- target/source/action binding;
- correlated authorization events;
- focused compatibility and upstream-rebase tests.

Do not add an identity provider, general administration suite, or policy language to `hbbs` itself.

### Keep Styrene optional at the boundary

The recommended authorization stack is simpler than making this depend on current Styrene RBAC internals:

- Cognito for authentication;
- Cedar OSS embedded in the Rust broker for principal/action/resource/context decisions;
- PostgreSQL for device inventory, revocation state, and queryable audit records;
- a purpose-built Ed25519 or PASETO v4.public connection grant;
- WireGuard containment for the initial stock `hbbr` deployment.

Styrene may provide device identity, key derivation, WireGuard enrollment, and later a reusable connection-grant profile through adapters. It should not initially supply the policy engine or audit store. Current `styrene-rbac` is global-capability oriented rather than resource scoped, and its `SignedRosterEntry` canonical signature does not cover `expires_at`; that type must not be reused as a session grant.

A generic adapter boundary should accept an opaque stable source-device ID, status, and optional public key so that RustDesk authorization does not become inseparable from Styrene.

## Grant requirements

Every field affecting authorization must be signed:

- format and algorithm version;
- issuer and `hbbs` audience;
- random single-use grant ID;
- immutable OIDC issuer and subject;
- source device identity;
- target RustDesk device ID;
- permitted connection mode/action;
- policy version/hash;
- issued-at, not-before, and expiration.

The first implementation may use bearer grants because the current RustDesk protocol already has a token carrier. It must compensate with encrypted transport, a 60–120 second lifetime, target/source binding, and a replay cache. Proof-of-possession is a later hardening item and may require a client/server challenge.

## Authorization scope

An `hbbs` fork can reliably decide whether a connection starts. It cannot inspect the end-to-end encrypted peer session and therefore cannot centrally guarantee clipboard, file-transfer, tunnel, or view-only restrictions after rendezvous. Those controls remain target configuration or target-client responsibilities.

The initial honest policy surface is:

- `device.connect.view`
- `device.connect.control`
- `device.enroll`
- `device.retire`
- `address_book.read`
- `address_book.admin`

Do not claim centrally enforced feature-level authorization unless a later target-client work stream implements and tests it.

## Decisive spike

### Goal

Prove or reject the narrow-fork architecture before building the broker.

### Experiments

1. Patch every `hbbs` punch-hole and rendezvous relay path to reject a missing or invalid test token before forwarding.
2. Use the existing protobuf token field; do not change the schema in the first experiment.
3. Demonstrate allow, deny, wrong target, expiration, and replay behavior using a small HMAC fixture or minting CLI.
4. Exercise direct rendezvous and relay fallback against stock `hbbr`.
5. Determine, in order, whether the token can be supplied through:
   - stock-client configuration or command interface;
   - a local credential helper/proxy;
   - a minimal managed-client patch.
6. Rebase the narrow patch over two recent upstream server tags and record conflicts.

### Acceptance gate

Proceed to an end-to-end demonstration only if:

- all normal direct and relay rendezvous paths fail closed;
- no protobuf change is needed;
- stock `hbbr` can remain overlay-only for the pilot;
- the client integration is either stock/helper based or a small, isolated patch;
- the `hbbs` patch remains localized and mechanically rebaseable.

## Agent-assisted schedule

Observed Styrene/Omegon delivery pace materially reduces conventional estimates. Recent Styrene evidence included 90 commits and roughly 13.8k lines of churn over 14 days, with test-bearing changes across signed envelopes, MQTT, persistence, IPC, transport recovery, and CI. That supports aggressive implementation ranges, but it does not compress real cross-platform acceptance, external identity administration, security review, or incident ownership.

| Gate | Expected focused effort | Likely calendar |
|---|---:|---:|
| Decisive protocol spike | 1–3 working days | 1–3 days |
| Cognito-to-`hbbs` demonstration | 4–8 working days | 1–2 weeks |
| Windows-first internal pilot | 2–3 effective engineer-weeks | 7–12 working days after spike |
| Multi-desktop internal pilot | 3–5 effective engineer-weeks | 2–4 weeks |
| Production internal service | 7–12 effective engineer-weeks | 6–10 weeks |
| Expanded control product | 12–20 effective engineer-weeks | 3–5 months |

Expected steady-state ownership for a narrow fork is 0.05–0.12 FTE, with temporary increases during disruptive upstream or security releases. Broad GUI/client ownership is the dominant maintenance risk.

## Stop conditions

Stop this stream and adopt MeshCentral or a licensed integrated product if:

- unmodified upstream clients are mandatory and no helper path can supply grants;
- the required client patch spreads broadly through the GUI or platform layers;
- immediate termination of active P2P sessions is mandatory without target changes;
- clipboard/file/tunnel restrictions must be centrally guaranteed;
- `hbbr` must be publicly exposed before relay-side authorization exists;
- upstream rebase testing shows recurring protocol or packaging instability;
- no owner accepts ongoing security and fork maintenance.

## Recommendation

Approve only the **1–3 day decisive spike** when this work stream is scheduled. If the existing token carrier and narrow `hbbs` hook work as expected, authorize a Windows-first end-to-end demonstration. Defer broader Styrene integration until the protocol and client boundaries are measured.

For authorization, prefer embedded Cedar and a purpose-built signed grant over extending `styrene-rbac` specifically for RustDesk. Promote a grant format into Styrene only after it proves reusable across at least one additional connection-oriented system.

MeshCentral remains the lower-total-cost answer when the objective is simply fully open-source remote access with integrated OIDC, authorization, unattended access, and audit. This proposal is justified when RustDesk UX/performance or the reusable grant architecture is strategically valuable.

## Research provenance

The detailed scratch assessment is intentionally outside this repository at:

`/Users/wilson/workspace/styrene-labs/rustdesk-auth-spike/SPIKE.md`

Source snapshots used by that assessment:

- `rustdesk-server` `6e7de5b1d648e64e5d7930eea2239f58721420b9`
- `rustdesk-client` `12f2de5959fa1fcd36d5a5b0c2fa91657411cc7a`
- `styrene-rs` `f52eda386915015a9e5559c8390b812fdd144103`
