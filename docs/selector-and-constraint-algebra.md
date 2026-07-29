---
id: selector-and-constraint-algebra
title: "Selector and Constraint Algebra"
status: exploring
parent: authorization-grants
tags: [authorization, selectors, algebra]
open_questions:
  - "What is the exact finite action enum, resource-selector grammar, and constraint partial-order registry for profile v1?"
dependencies: []
related: []
---

# Selector and Constraint Algebra

## Overview

Define the finite action set, typed resource-selector grammar, constraint partial orders, deterministic normalization/subset algorithm, and complexity ceilings.

## Decisions

### Typed finite attenuation algebra

**Status:** accepted

**Rationale:** Profile v1 uses exact typed resources, namespace segments, bounded unions/intersections, and closed constraint orders; regex, globs, negation, arbitrary code, and heuristic subset solvers are forbidden.

## Open Questions

- What is the exact finite action enum, resource-selector grammar, and constraint partial-order registry for profile v1?
