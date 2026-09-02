# Operation-Scoped Authorization Design

## Reassessment

Issue #2 defines the consumer gap. The unmerged
`origin/feature/styrene-policy-crate` commit
`781fd11c03c349109d05ef7a9bf80ab895a89093` provides only basic principal,
action, resource, context, and decision primitives. It does not implement trusted
issuers, deny precedence, wildcard grants, constraints, policy discovery, stable
reason classes, or the requested audit schema.

The prototype is not implementation authority. New work begins with failing
public-contract tests on current `main`.

## Crate Boundary

Keep `styrene-rbac` as the compatibility source for hierarchical roles and mesh
roster grants. Add one reusable policy layer for operation and resource decisions.
The policy evaluator is pure and performs no network or filesystem I/O.

Authentication and transport adapters construct a validated principal only after
their own authentication succeeds. Trusted proxy-header extraction is an optional
adapter with configurable names. It does not make the core evaluator depend on an
HTTP framework.

## Evaluation Order

1. Reject missing authentication and invalid or untrusted issuer state.
2. Expand explicit role bundles with cycle and unknown-role validation.
3. Select exact and prefix grants applicable to the principal and operation.
4. Apply resource and context constraints.
5. Give every matching explicit deny precedence over allows.
6. Return one structured decision and bounded audit projection.

Unknown operations and unavailable or invalid policy fail closed. Wildcards are
suffix-prefix matches such as `omegon.native_session.*`. Arbitrary glob syntax is
not accepted.

## Decisions And Discovery

The decision includes allowed state, stable reason, operation, matched grant,
principal summary, optional resource, requirements, and audit fields. It does not
include credentials, bearer values, unrestricted claims, or private policy input.

Policy discovery evaluates the same normalized grants and constraints as
enforcement. It can report allowed, denied, coarse fallback, partial, or warning
state, but it cannot grant an operation independently.

## Compatibility

Existing `Peer`, `Monitor`, `Operator`, and `Admin` behavior remains available as
data-backed role bundles. Compatibility does not bypass explicit denies or trusted
principal requirements. Consumers migrate operation catalogs independently.
