//! Assertion helpers for consumers testing their own operation catalogs.

use super::decision::{AuthorizationRequest, DecisionReason, RequestContext, ResourceRef};
use super::policy::OperationPolicy;
use super::principal::{AuthSource, Principal};

/// Panic unless the policy allows `operation` for `principal`.
pub fn assert_allows(
    policy: &OperationPolicy,
    principal: &Principal,
    operation: &str,
    context: &RequestContext,
) {
    let decision = policy.evaluate(&AuthorizationRequest::new(Some(principal), operation, context));
    assert!(
        decision.allowed,
        "expected {operation} to be allowed for {}, got {} (matched {:?})",
        principal.subject(),
        decision.reason,
        decision.matched_grant
    );
}

/// Panic unless the policy allows `operation` on `resource` for `principal`.
pub fn assert_allows_resource(
    policy: &OperationPolicy,
    principal: &Principal,
    operation: &str,
    resource: &ResourceRef,
    context: &RequestContext,
) {
    let decision = policy.evaluate(
        &AuthorizationRequest::new(Some(principal), operation, context).with_resource(resource),
    );
    assert!(
        decision.allowed,
        "expected {operation} on {resource} to be allowed for {}, got {}",
        principal.subject(),
        decision.reason
    );
}

/// Panic unless the policy denies `operation` with exactly `reason`.
pub fn assert_denies(
    policy: &OperationPolicy,
    principal: Option<&Principal>,
    operation: &str,
    context: &RequestContext,
    reason: DecisionReason,
) {
    let decision = policy.evaluate(&AuthorizationRequest::new(principal, operation, context));
    assert!(
        !decision.allowed,
        "expected {operation} to be denied with {reason}, but it was allowed"
    );
    assert_eq!(
        decision.reason, reason,
        "expected {operation} to be denied with {reason}, got {}",
        decision.reason
    );
}

/// A principal holding exactly one role, for matrix tests.
#[must_use]
pub fn principal_with_role(role: &str) -> Principal {
    Principal::new("test", format!("role-{role}"), AuthSource::Local)
        .with_roles([role])
        .normalize(policy_limits())
        .unwrap_or_else(|error| panic!("role principal {role}: {error}"))
}

fn policy_limits() -> &'static super::limits::Limits {
    static LIMITS: super::limits::Limits = super::limits::Limits {
        max_grants: 4_096,
        max_roles: 256,
        max_role_depth: 16,
        max_operation_len: 256,
        max_constraints_per_grant: 16,
        max_claims: 32,
        max_claim_len: 512,
        max_attributes: 64,
        max_identifier_len: 256,
    };
    &LIMITS
}

/// Assert that exactly `expected_roles` (of every role the policy defines)
/// allow `operation` without constraints.
pub fn matrix_test_roles(policy: &OperationPolicy, operation: &str, expected_roles: &[&str]) {
    let context = RequestContext::new();
    let mut allowed: Vec<String> = policy
        .role_names()
        .into_iter()
        .filter(|role| {
            let principal = principal_with_role(role);
            policy
                .evaluate(&AuthorizationRequest::new(Some(&principal), operation, &context))
                .allowed
        })
        .collect();
    allowed.sort();
    let mut expected: Vec<String> =
        expected_roles.iter().map(|role| role.to_ascii_lowercase()).collect();
    expected.sort();
    assert_eq!(allowed, expected, "roles allowing {operation}");
}
