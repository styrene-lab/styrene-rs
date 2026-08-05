---
id: lifecycle-domain-graph
title: "Lifecycle Domain Graph"
status: resolved
parent: identity-record-profile
tags: [lifecycle, replication, transactions]
open_questions: []
dependencies:
  - canonical-encoding-profile
related: []
---

# Lifecycle Domain Graph

## Overview

Define authoritative revision domains, typed domain keys, predecessor/fork rules, reconciliation authority, atomic multi-domain updates, and crash recovery.

## Decisions

### Atomic signing journal closes local cross-domain recovery

**Status:** accepted

Profile v1 uses the durable operation states and ordered domain reservations defined by Atomic Signing Transactions. The operation journal, authoritative heads, signed-record store, and audit ledger share one serializable local durability authority. Finalization advances every expected head and records every signed object and audit event in one transaction; replication follows committed state.

Recovery scans nonterminal `prepared`, `signing`, `signed_uncommitted`, and `outcome_unknown` operations before releasing their domains. It never partially advances a multi-domain operation or invokes a signer twice for one request ID. Distributed stores require a later consensus profile and cannot claim profile-v1 atomic conformance.

## Open Questions

None for the profile-v1 lifecycle-domain graph and local durability model.
