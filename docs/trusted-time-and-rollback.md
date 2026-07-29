---
id: trusted-time-and-rollback
title: "Trusted Time and Rollback"
status: exploring
parent: runtime-identity-issuance
tags: [time, rollback, security]
open_questions:
  - "How are migration tokens sealed and transferred for TPM, HSM, and software-only deployments?"
dependencies:
  - lifecycle-domain-graph
related: []
---

# Trusted Time and Rollback

## Overview

Define TrustedClock ownership, drift, monotonic checkpoints, rollback detection, supported epochs, backup/clone behavior, and degraded verification.

## Decisions

### Verifier-owned trusted time

**Status:** accepted

**Rationale:** Production request DTOs cannot supply authoritative processing time or drift. Verifiers obtain time from injected TrustedClock and local policy.

## Open Questions

- How are migration tokens sealed and transferred for TPM, HSM, and software-only deployments?
