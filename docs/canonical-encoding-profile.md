---
id: canonical-encoding-profile
title: "Canonical Encoding Profile"
status: exploring
parent: identity-record-profile
tags: [cbor, canonicalization, crypto]
open_questions:
  - "What exact field numbers and maximum encoded sizes apply to every profile-v1 signed record?"
  - "[assumption] Cross-language NFC libraries produce acceptable byte-identical display-field normalization for supported implementations."
dependencies: []
related: []
---

# Canonical Encoding Profile

## Overview

Freeze deterministic CBOR, identifier canonicalization, optional-field representation, domain framing, unknown-field policy, profile transitions, and golden/rejection vectors.

## Decisions

### Closed deterministic-CBOR profile

**Status:** accepted

**Rationale:** Signed profile-v1 records use closed schemas, increasing integer keys, shortest encodings, definite lengths, explicit nulls, canonical security identifiers, decode/re-encode byte comparison, and immutable golden/rejection vectors.

## Open Questions

- None for the profile-v1 encoding contract. Concrete per-record schemas still assign their own field numbers and record-specific limits under this profile.
