---
id: algorithm-and-key-version-registry
title: "Algorithm and Key-Version Registry"
status: exploring
parent: identity-record-profile
tags: [crypto, keys, algorithms]
open_questions:
  - "Does profile v1 permit only Ed25519/SHA-256, or reserve and implement additional algorithms immediately?"
  - "How are concurrent issuance and rollover ordered without allowing key-version reuse or downgrade?"
dependencies:
  - canonical-encoding-profile
related: []
---

# Algorithm and Key-Version Registry

## Overview

Define signature/digest algorithm identifiers and parameters, key IDs and versions, rollover, concurrent issuance, deprecation, downgrade prevention, and historical verification.

## Open Questions

- None for profile-v1 primitive algorithm and key-version rules. Algorithm additions and provider-specific attestation suites require a new registry revision.
