# Operation-Scoped Authorization Tasks

## 1. Principal and decision contracts
<!-- specs: authorization-policy -->

- [x] Add failing serialization, redaction, bounded-claim, stable-reason, and decision-shape tests
- [x] Implement normalized principal, operation, resource, context, decision, and audit types without HTTP dependencies
- [x] Add reusable `assert_allows`, `assert_denies`, and role-matrix test helpers

## 2. Grants and roles
<!-- specs: authorization-policy -->

- [x] Add failing exact, prefix, deny-precedence, unknown-operation, and malformed-pattern tests
- [x] Add failing role inheritance, unknown parent, cycle, compatibility bundle, and explicit-deny tests
- [x] Implement normalized grants and role bundles with atomic fail-closed policy loading

## 3. Resource and context constraints
<!-- specs: authorization-policy -->

- [x] Add failing resource, attribute, missing-field, type, and conflicting-constraint matrices
- [x] Implement bounded exact constraints and deny-first evaluation without service-specific catalogs
- [x] Add adversarial complexity and allocation limits for untrusted policy input

## 4. Trusted issuer adapter
<!-- specs: authorization-policy -->

- [x] Add failing tests for missing authentication, trusted and unknown issuers, missing subject, role mapping, configurable names, and header spoofing
- [x] Implement framework-neutral trusted issuer extraction that cannot authenticate a request by itself
- [x] Verify credentials and unrestricted claims never enter decisions, descriptors, errors, or audit output

## 5. Discovery and consumer compatibility
<!-- specs: authorization-policy -->

- [x] Add failing equivalence tests between effective-policy descriptors and enforcement decisions
- [x] Implement bounded policy discovery with requirements, scopes, and coarse or partial warnings
- [x] Prove existing Styrene coarse roles remain available as bundles without bypassing explicit denies
- [x] Add a consumer fixture that expresses issue #2 operation examples without a local operation-to-role table

## 6. Verification
<!-- specs: authorization-policy -->

- [x] Run focused default, no-default, config, and RBAC feature tests plus formatting and warning-denied Clippy
- [x] Validate OpenSpec, offline policy, public API boundaries, audit redaction, and malformed-input limits
