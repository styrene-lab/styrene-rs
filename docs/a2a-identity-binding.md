---
id: a2a-identity-binding
title: "A2A Identity Binding"
status: exploring
parent: identity-trust-system
tags: [a2a, identity, envelope]
open_questions: []
dependencies: []
related: []
---

# A2A Identity Binding

## Overview

Define end-to-end authentication of A2A envelopes using runtime identity evidence while keeping work authorization and transport evidence separate.

## Decisions

### Authentication does not authorize work

**Status:** accepted

**Rationale:** Envelope verification returns structured identity and lifecycle evidence. Meridian separately evaluates grants, requested action, resource, and risk.
