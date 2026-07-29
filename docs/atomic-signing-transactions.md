---
id: atomic-signing-transactions
title: "Atomic Signing Transactions"
status: exploring
parent: signum-service-boundary
tags: [signing, transactions, toctou]
open_questions:
  - "What lifecycle heads must be locked or compare-and-swapped after a non-rollbackable hardware signature operation?"
  - "What result is persisted and returned if signing succeeds but durable operation/audit commit fails?"
dependencies:
  - canonical-encoding-profile
  - algorithm-and-key-version-registry
  - lifecycle-domain-graph
  - trusted-time-and-rollback
related: []
---

# Atomic Signing Transactions

## Overview

Define the typed signing transaction from authorization through lifecycle checks, canonicalization, hardware signing, durable idempotency/audit commit, and response.

## Decisions

### Typed signing requests only

**Status:** accepted

**Rationale:** Signum re-derives domain-separated canonical input from typed requests. Raw arbitrary-byte signing is never exposed over RPC.

## Open Questions

- What lifecycle heads must be locked or compare-and-swapped after a non-rollbackable hardware signature operation?
- What result is persisted and returned if signing succeeds but durable operation/audit commit fails?
