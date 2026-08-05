---
id: identity-record-profile
title: "Identity Record Profile"
status: resolved
parent: identity-trust-system
tags: [identity, records, crypto]
open_questions: []
dependencies:
  - canonical-encoding-profile
  - algorithm-and-key-version-registry
  - lifecycle-domain-graph
  - runtime-certificate-record-profile
  - lifecycle-transition-record-profile
related: []
---

# Identity Record Profile

## Overview

## Decisions

### First implementation tranche is schema-frozen

**Status:** accepted

The common canonical encoding, algorithm/key registry, and lifecycle graph are resolved. The runtime-certificate and lifecycle-transition profiles now freeze the first concrete portable record schemas needed for runtime issuance, renewal, suspension, revocation, and A2A identity verification.

Owner, authority, recovery-policy, and API-grant records remain separate concrete profiles. Their absence does not permit implementations to improvise fields: until frozen, they are represented only through already-defined digest references and verified issuer-chain interfaces.

## Open Questions

None for the first runtime-identity record tranche. Additional concrete record families are follow-on profiles.
