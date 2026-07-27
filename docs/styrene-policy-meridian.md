+++
title = "Styrene Policy and the Meridian"
tags = ["policy","identity","meridian","authorization","onboarding"]
+++

+++
id = "141c976e-35ce-460b-88f6-c589f81cf5ac"
kind = "design_node"

[data]
title = "Styrene Policy and the Meridian"
status = "decided"
issue_type = "architecture"
priority = 1
dependencies = []
open_questions = []
+++

## Overview

# Styrene Policy and the Meridian

## Status

Design baseline for replacing parallel RBAC/ABAC enforcement with one attribute-based policy model. Existing `styrene-rbac` APIs remain a migration projection until call sites move to the unified policy boundary.

## Decision

Styrene has one authorization engine: `styrene-policy`.

Roles are named, reusable bundles of attributes and capabilities. They are an administrative convenience and policy input, not a second authorization engine. `styrene-identity` authenticates principals and verifies claims; it does not authorize actions.

```text
wire / IPC credentials
        ↓
styrene-identity
  authenticated principal
  verified attestations and delegations
  provenance-bearing claims
        +
signed roster
  memberships
  named authority bundles
  explicit grants, restrictions, and expiry
        ↓
styrene-policy
  principal × action × resource × context
        ↓
Clear | Black | Indeterminate
  + reasons + obligations
```

Every protected operation receives one authoritative policy decision. Daemon call sites must not combine an independent RBAC answer with a policy answer.

## Clear World, Black World, and the Meridian

- **Clear** permits a specific interaction, potentially with mandatory obligations.
- **Black** denies a specific interaction.
- **Indeterminate** means required evidence is missing, stale, unsupported, or contradictory. Sensitive operations fail closed; the decision may request evidence refresh, approval, or operator confirmation.
- **The Meridian** is the context-dependent decision boundary between Clear and Black.

Clear and Black are dispositions of an interaction, not intrinsic labels on principals, devices, networks, or locations. The same identity may be Clear for discovery and Black for configuration. The operative setup term is **Draw Your Meridian**.

## Identity boundary

`styrene-identity` owns:

- canonical identity and public-key representation;
- signature and attestation verification;
- delegated identity verification;
- authentication freshness and anti-replay evidence;
- signer and key-custody properties where they can be attested.

It emits an authenticated principal and verified claims with provenance. Policy must not trust caller-supplied identity hashes, booleans such as `can_sign`, or unverified role claims.

A policy request binds at least:

- authenticated principal;
- action;
- resource;
- nonce;
- issued-at and expiry;
- policy-relevant context.

Derived keys are not assumed linkable merely because Styrene can derive them from one root. A verified attestation or protocol proof must establish the relationship.

## Attribute model

### Stable dimensions

```text
principal.kind
  human | device | service | delegated_agent

membership.state
  unknown | pending | enrolled | suspended | revoked

authority.bundle
  owner | maintainer | member | observer

device.purpose
  console | hub | relay | field_node | appliance

custody.relationship
  owned | managed | shared | external

resource.domain
  <locally defined stable identifier>
```

Each security-relevant claim carries provenance:

```text
issuer
issued_at
expires_at
verification_method
evidence_reference
```

Named roles and purposes use stable, namespaced identifiers rather than closed protocol enums, for example:

```text
styrene.role.owner
styrene.device.hub
lab.example.role.radio_operator
```

Closed enums are reserved for protocol invariants such as decision disposition and membership lifecycle.

### Semantics to keep separate

- Human authority and device purpose are orthogonal.
- `Blocked` is an explicit negative policy rule, not a role.
- `None` is absence of authority, not a role.
- Declared location is not evidence of trust.
- Network locality is observed context, not authorization.
- Stated purpose (`personal_lab`, `field_exercise`) selects setup defaults and audit labels; it never grants authority.
- Runtime capability describes what a node can do, not what it may do.

## Bundles and policy composition

Authority bundles have no ordinal hierarchy and no implicit cumulative inheritance. A principal may hold multiple bundles in different resource domains. Grants are scoped by action, resource, issuer, and time.

Composition rules:

1. verified revocation and explicit deny dominate grants;
2. unsupported or stale required evidence yields `Indeterminate`;
3. obligations are unioned and deduplicated;
4. conflicting verified claims yield `Indeterminate`, not arbitrary ordering;
5. evaluation is deterministic for the same policy version and evidence set;
6. important decisions record policy version and input-evidence references.

Mandatory obligations may include approval, audit, fresh authentication, signature, capability refresh, and post-action verification. An allow decision is not executable until its obligations are satisfied and time-sensitive evidence is revalidated.

## Small-lab first use

First setup asks for a consistent set of inputs:

- **Who:** owner, additional people, devices, and enrollment state.
- **What:** lab name and this device's purpose.
- **Where:** administrative/resource domains and expected placement; never a trust shortcut.
- **How:** intended and observed communication paths.
- **Why:** operational intent used for defaults and audit context only.

### Primary scenarios

#### Personal Lab

One owner, a console, hub, and field node. Discovery and messaging are generally Clear; administration and identity control remain Black except to the owner.

Happy path: establish the owner identity, enroll a second device, exchange a message, observe one expected denial, then explicitly grant a bounded authority.

#### Household / Small Team

A maintainer, members, shared infrastructure, and a guest or pending identity. Demonstrates invitation, membership, custodianship, and limited delegation.

Happy path: issue a signed invitation, enroll a member, assign a bundle in the lab domain, and verify that shared-service use does not imply infrastructure administration.

#### Test Range / Field Exercise

A controller, team leads, participants, relays, observers, and quarantined discoveries. Demonstrates resource-domain scoping and approval-gated crossings without claiming a real emergency-service deployment.

Happy path: approve a discovered node into one team domain and verify it cannot operate on another team's resources.

### Advanced starting postures

- **Clear World:** interoperability by default for baseline discovery and messaging; privileged actions remain explicit.
- **Black World:** rostered participation by default; unknown identities are quarantined and disclosure minimized.
- **Start Empty:** draw each initial rule explicitly.

Presets are versioned initialization templates. Updating a preset never silently mutates an existing lab.

## Growth path

The initial model expands without replacement:

1. **Personal lab:** one owner, several devices, one local Meridian.
2. **Shared lab:** multiple people, invitations, separate ownership and custodianship.
3. **Multiple domains:** remote resources, delegated administration, per-domain disclosure.
4. **Federation:** independently administered claim issuers, local mapping of foreign claims, policy-version negotiation, revocation, and cross-Meridian workflows.

Foreign role names are claims from another issuer, not locally authoritative roles. Local policy maps them to local attributes or leaves them indeterminate.

Constrained nodes receive compact, signed policy projections with expiry and scope rather than the complete attribute graph.

## Migration

1. Land `styrene-policy` as the sole decision model.
2. Treat `styrene-rbac` roles and capabilities as a compatibility resolver that produces effective principal attributes.
3. Replace `identity_hash: Option<String>` and `can_sign: bool` with typed authenticated-principal and evidence structures.
4. Add `Indeterminate` and deterministic composition semantics.
5. Move daemon action families from `has_capability()` to one `authorize()` call incrementally.
6. Retain legacy role names only as migration projections and serialized compatibility inputs.
7. Remove independent RBAC enforcement after all protected call sites use policy decisions.

## Non-goals

- A universal ontology of people, devices, or locations.
- Treating Clear/Black as stable perimeter membership.
- Inferring authorization from network placement.
- Making setup purpose an authority claim.
- Shipping a general-purpose policy language before the typed native model is proven.

## Open Questions
