# Operation-Scoped Authorization Tasks

## 1. Principal and decision contracts
<!-- specs: authorization-policy -->

- [ ] Add failing serialization, redaction, bounded-claim, stable-reason, and decision-shape tests
- [ ] Implement normalized principal, operation, resource, context, decision, and audit types without HTTP dependencies
- [ ] Add reusable `assert_allows`, `assert_denies`, and role-matrix test helpers

## 2. Grants and roles
<!-- specs: authorization-policy -->

- [ ] Add failing exact, prefix, deny-precedence, unknown-operation, and malformed-pattern tests
- [ ] Add failing role inheritance, unknown parent, cycle, compatibility bundle, and explicit-deny tests
- [ ] Implement normalized grants and role bundles with atomic fail-closed policy loading

## 3. Resource and context constraints
<!-- specs: authorization-policy -->

- [ ] Add failing resource, attribute, missing-field, type, and conflicting-constraint matrices
- [ ] Implement bounded exact constraints and deny-first evaluation without service-specific catalogs
- [ ] Add adversarial complexity and allocation limits for untrusted policy input

## 4. Trusted issuer adapter
<!-- specs: authorization-policy -->

- [ ] Add failing tests for missing authentication, trusted and unknown issuers, missing subject, role mapping, configurable names, and header spoofing
- [ ] Implement framework-neutral trusted issuer extraction that cannot authenticate a request by itself
- [ ] Verify credentials and unrestricted claims never enter decisions, descriptors, errors, or audit output

## 5. Discovery and consumer compatibility
<!-- specs: authorization-policy -->

- [ ] Add failing equivalence tests between effective-policy descriptors and enforcement decisions
- [ ] Implement bounded policy discovery with requirements, scopes, and coarse or partial warnings
- [ ] Prove existing Styrene coarse roles remain available as bundles without bypassing explicit denies
- [ ] Add a consumer fixture that expresses issue #2 operation examples without a local operation-to-role table

## 6. Verification
<!-- specs: authorization-policy -->

- [ ] Run focused default, no-default, config, and RBAC feature tests plus formatting and warning-denied Clippy
- [ ] Validate OpenSpec, offline policy, public API boundaries, audit redaction, and malformed-input limits
