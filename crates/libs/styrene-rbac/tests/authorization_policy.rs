//! Public-contract tests for operation-scoped authorization: the spec
//! scenarios, issue #2's operation examples without a local role table, and
//! the audit and limit guarantees.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use styrene_rbac::authz::testing::{
    assert_allows, assert_allows_resource, assert_denies, matrix_test_roles,
};
use styrene_rbac::authz::{
    AuthSource, AuthenticationState, AuthorizationRequest, DecisionReason, DescriptorWarning,
    GrantId, IssuerError, IssuerMapping, Limits, OperationPolicy, PolicyBuilder, PolicyError,
    PolicySlot, Principal, PrincipalExtractor, RequestContext, ResourceRef, RoleBundle,
    TrustedIssuerConfig,
};

fn limits() -> Limits {
    Limits::default()
}

fn principal(subject: &str, roles: &[&str]) -> Principal {
    Principal::new("omegon", subject, AuthSource::Bearer)
        .with_roles(roles.iter().copied())
        .normalize(&limits())
        .expect("principal")
}

/// Issue #2's catalog expressed as role bundles and grants, with no
/// operation-to-role table on the consumer side.
fn omegon_policy() -> OperationPolicy {
    let limits = limits();
    PolicyBuilder::new()
        .role(
            RoleBundle::new("monitor")
                .allow("omegon.native_session.read", &limits)
                .unwrap()
                .allow("omegon.surface.read", &limits)
                .unwrap()
                .allow("omegon.surface.stream", &limits)
                .unwrap(),
        )
        .role(
            RoleBundle::new("operator")
                .inherits("monitor")
                .allow("omegon.native_session.create", &limits)
                .unwrap()
                .allow("omegon.native_session.action", &limits)
                .unwrap()
                .allow("omegon.event.ingress", &limits)
                .unwrap(),
        )
        .role(RoleBundle::new("admin").inherits("operator").allow("omegon.*", &limits).unwrap())
        .declare_operation("omegon.ws.auth_login")
        .grant_line("allow user:alice omegon.surface.read session:default")
        .grant_line("allow group:operators omegon.native_session.*")
        .grant_line("deny user:bob omegon.event.ingress id=bob-no-ingress")
        .grant_line(
            "deny operator omegon.event.ingress where trigger_kind = \"shutdown\" id=no-shutdown",
        )
        .grant_line("allow role:admin omegon.ws.auth_login")
        .build()
        .expect("omegon policy")
}

#[test]
fn prefix_grant_allows_and_identifies_the_matched_grant() {
    let policy = omegon_policy();
    let carol = Principal::new("omegon", "carol", AuthSource::Session)
        .with_groups(["operators"])
        .normalize(&limits())
        .unwrap();
    let context = RequestContext::new();
    let decision = policy.evaluate(&AuthorizationRequest::new(
        Some(&carol),
        "omegon.native_session.read",
        &context,
    ));
    assert!(decision.allowed);
    assert_eq!(decision.reason, DecisionReason::Allowed);
    assert_eq!(decision.matched_grant, Some(GrantId::new("line-2")));
    assert_eq!(decision.required, vec!["admin", "monitor", "operator"]);
}

#[test]
fn explicit_deny_overrides_every_allow_including_inherited_ones() {
    let policy = omegon_policy();
    let bob = principal("bob", &["admin"]);
    let context = RequestContext::new();
    let decision =
        policy.evaluate(&AuthorizationRequest::new(Some(&bob), "omegon.event.ingress", &context));
    assert!(!decision.allowed);
    assert_eq!(decision.reason, DecisionReason::ExplicitDeny);
    assert_eq!(decision.matched_grant, Some(GrantId::new("bob-no-ingress")));
    // Admin's `omegon.*` still allows everything else.
    assert_allows(&policy, &bob, "omegon.native_session.create", &context);
}

#[test]
fn unconfigured_operations_are_misconfigured_and_declared_ones_are_not_granted() {
    let policy = omegon_policy();
    let monitor = principal("mona", &["monitor"]);
    let context = RequestContext::new();
    // `styrene.vault.rotate` matches no grant, bundle, or declaration.
    assert_denies(
        &policy,
        Some(&monitor),
        "styrene.vault.rotate",
        &context,
        DecisionReason::MisconfiguredOperation,
    );
    // `omegon.vault.rotate` is configured for admin through `omegon.*`.
    assert_denies(
        &policy,
        Some(&monitor),
        "omegon.vault.rotate",
        &context,
        DecisionReason::CapabilityNotGranted,
    );
    assert_denies(
        &policy,
        Some(&monitor),
        "omegon.ws.auth_login",
        &context,
        DecisionReason::CapabilityNotGranted,
    );
    assert_denies(
        &policy,
        Some(&monitor),
        "not an operation",
        &context,
        DecisionReason::MisconfiguredOperation,
    );
    let decision = policy.evaluate(&AuthorizationRequest::new(
        Some(&monitor),
        "omegon.ws.auth_login",
        &context,
    ));
    assert_eq!(decision.required, vec!["admin"]);
}

#[test]
fn resource_constraints_scope_grants_to_exact_resources() {
    let policy = omegon_policy();
    let alice = Principal::new("omegon", "alice", AuthSource::Bearer).normalize(&limits()).unwrap();
    let context = RequestContext::new();
    let default = ResourceRef::new("session", "default");
    assert_allows_resource(&policy, &alice, "omegon.surface.read", &default, &context);
    let other = ResourceRef::new("session", "other");
    let decision = policy.evaluate(
        &AuthorizationRequest::new(Some(&alice), "omegon.surface.read", &context)
            .with_resource(&other),
    );
    assert_eq!(decision.reason, DecisionReason::CapabilityNotGranted);
    // A missing resource never satisfies a resource constraint.
    assert_denies(
        &policy,
        Some(&alice),
        "omegon.surface.read",
        &context,
        DecisionReason::CapabilityNotGranted,
    );
}

#[test]
fn context_denies_match_only_their_attribute() {
    let policy = omegon_policy();
    let operator = principal("otto", &["operator"]);
    let shutdown = RequestContext::new()
        .with_attribute("trigger_kind", "shutdown")
        .with_attribute("surface", "main");
    let decision = policy.evaluate(&AuthorizationRequest::new(
        Some(&operator),
        "omegon.event.ingress",
        &shutdown,
    ));
    assert_eq!(decision.reason, DecisionReason::ExplicitDeny);
    assert_eq!(decision.matched_grant, Some(GrantId::new("no-shutdown")));
    let manual = RequestContext::new()
        .with_attribute("trigger_kind", "manual")
        .with_attribute("surface", "main");
    assert_allows(&policy, &operator, "omegon.event.ingress", &manual);
    assert_allows(&policy, &operator, "omegon.event.ingress", &RequestContext::new());
}

#[test]
fn roles_inherit_and_decisions_identify_the_effective_source() {
    let policy = omegon_policy();
    let operator = principal("otto", &["operator"]);
    let context = RequestContext::new();
    let decision = policy.evaluate(&AuthorizationRequest::new(
        Some(&operator),
        "omegon.surface.read",
        &context,
    ));
    assert!(decision.allowed);
    assert_eq!(
        decision.matched_grant.as_ref().map(GrantId::as_str),
        Some("role:monitor:allow:omegon.surface.read")
    );
    matrix_test_roles(&policy, "omegon.surface.read", &["monitor", "operator", "admin"]);
    matrix_test_roles(&policy, "omegon.native_session.create", &["operator", "admin"]);
    matrix_test_roles(&policy, "omegon.ws.auth_login", &["admin"]);
}

#[test]
fn invalid_role_inheritance_fails_loading_and_keeps_the_previous_policy() {
    let limits = limits();
    let slot = PolicySlot::with_policy(omegon_policy());
    let cycle = PolicyBuilder::new()
        .role(RoleBundle::new("a").inherits("b"))
        .role(RoleBundle::new("b").inherits("a"));
    assert!(matches!(slot.load(cycle), Err(PolicyError::RoleCycle { .. })));
    let unknown = PolicyBuilder::new().role(RoleBundle::new("a").inherits("ghost"));
    assert!(matches!(slot.load(unknown), Err(PolicyError::UnknownRole { .. })));
    let unknown_grant = PolicyBuilder::new().grant_line("allow role:ghost x.y");
    assert!(matches!(slot.load(unknown_grant), Err(PolicyError::UnknownRole { .. })));
    let conflicting = PolicyBuilder::new().grant_line("allow any x.y session:a session:b");
    assert!(matches!(slot.load(conflicting), Err(PolicyError::ConflictingConstraint { .. })));
    let malformed = PolicyBuilder::new().grant_line("allow any x.*.y");
    assert!(matches!(slot.load(malformed), Err(PolicyError::Grant { .. })));
    let duplicate =
        PolicyBuilder::new().grant_line("allow any x.y id=same").grant_line("deny any x.z id=same");
    assert!(matches!(slot.load(duplicate), Err(PolicyError::DuplicateGrantId { .. })));
    let deep = PolicyBuilder::new()
        .with_limits(Limits { max_role_depth: 2, ..limits })
        .role(RoleBundle::new("r0").inherits("r1"))
        .role(RoleBundle::new("r1").inherits("r2"))
        .role(RoleBundle::new("r2"));
    assert!(matches!(slot.load(deep), Err(PolicyError::RoleDepth { .. })));

    // The previous policy still answers.
    let operator = principal("otto", &["operator"]);
    let decision = slot.evaluate(&AuthorizationRequest::new(
        Some(&operator),
        "omegon.surface.read",
        &RequestContext::new(),
    ));
    assert!(decision.allowed);
}

#[test]
fn missing_authentication_and_unavailable_policy_fail_closed_with_distinct_reasons() {
    let policy = omegon_policy();
    let context = RequestContext::new().with_request_id("req-1").with_route("POST", "/api/session");
    assert_denies(
        &policy,
        None,
        "omegon.surface.read",
        &context,
        DecisionReason::MissingAuthentication,
    );
    let decision =
        policy.evaluate(&AuthorizationRequest::new(None, "omegon.surface.read", &context));
    assert!(decision.principal.is_none());
    assert_eq!(decision.audit.request_id.as_deref(), Some("req-1"));
    assert_eq!(decision.audit.route.as_deref(), Some("/api/session"));

    let slot = PolicySlot::new();
    let operator = principal("otto", &["operator"]);
    let decision =
        slot.evaluate(&AuthorizationRequest::new(Some(&operator), "omegon.surface.read", &context));
    assert_eq!(decision.reason, DecisionReason::PolicyUnavailable);
    assert_ne!(decision.reason, DecisionReason::CapabilityNotGranted);
    slot.load(
        PolicyBuilder::new()
            .role(RoleBundle::new("operator").allow("omegon.surface.read", &limits()).unwrap()),
    )
    .expect("valid policy loads");
    assert!(
        slot.evaluate(&AuthorizationRequest::new(Some(&operator), "omegon.surface.read", &context))
            .allowed
    );
    slot.clear();
    assert_eq!(
        slot.evaluate(&AuthorizationRequest::new(Some(&operator), "omegon.surface.read", &context))
            .reason,
        DecisionReason::PolicyUnavailable
    );
}

#[test]
fn unknown_principal_roles_deny_with_a_stable_reason() {
    let policy = omegon_policy();
    let stranger = principal("stan", &["superuser"]);
    assert_denies(
        &policy,
        Some(&stranger),
        "omegon.surface.read",
        &RequestContext::new(),
        DecisionReason::UnknownRole,
    );
}

#[test]
fn audit_projection_carries_correlation_but_never_claims_or_display_names() {
    let policy = omegon_policy();
    let alice = Principal::new("omegon", "alice", AuthSource::Bearer)
        .with_display_name("Alice Example")
        .with_session_id("sess-9")
        .with_client_id("client-3")
        .with_claim("token", "secret-bearer-value")
        .with_claim("workspace", "blue")
        .normalize(&limits())
        .unwrap();
    let context = RequestContext::new()
        .with_attribute("trigger_kind", "manual")
        .with_correlation_id("corr-7")
        .with_route("GET", "/surfaces/main")
        .with_timestamp(1_700_000_000);
    let resource = ResourceRef::new("session", "default");
    let decision = policy.evaluate(
        &AuthorizationRequest::new(Some(&alice), "omegon.surface.read", &context)
            .with_resource(&resource),
    );
    assert!(decision.allowed);
    let audit = &decision.audit;
    assert_eq!(audit.subject.as_deref(), Some("alice"));
    assert_eq!(audit.issuer.as_deref(), Some("omegon"));
    assert_eq!(audit.session_id.as_deref(), Some("sess-9"));
    assert_eq!(audit.client_id.as_deref(), Some("client-3"));
    assert_eq!(audit.correlation_id.as_deref(), Some("corr-7"));
    assert_eq!(audit.trigger_kind.as_deref(), Some("manual"));
    assert_eq!(audit.resource.as_deref(), Some("session:default"));
    assert_eq!(audit.timestamp, Some(1_700_000_000));
    assert_eq!(audit.reason, "allowed");
    let rendered = format!("{decision:?}{alice:?}");
    assert!(!rendered.contains("secret-bearer-value"));
    assert!(!rendered.contains("Alice Example"));
    assert!(!rendered.contains("blue"));
    assert!(rendered.contains("<2 redacted>"));
}

#[test]
fn stable_reasons_and_bounded_claims() {
    let reasons: Vec<&str> = DecisionReason::ALL.iter().map(|reason| reason.as_str()).collect();
    assert_eq!(
        reasons,
        [
            "allowed",
            "missing_authentication",
            "unknown_role",
            "missing_claim",
            "untrusted_issuer",
            "capability_not_granted",
            "explicit_deny",
            "policy_unavailable",
            "misconfigured_operation",
        ]
    );
    let mut too_many = Principal::new("omegon", "alice", AuthSource::Bearer);
    for index in 0..=Limits::default().max_claims {
        too_many = too_many.with_claim(format!("claim{index}"), "v");
    }
    assert!(too_many.normalize(&limits()).is_err());
    let long = "x".repeat(Limits::default().max_identifier_len + 1);
    assert!(Principal::new("omegon", long, AuthSource::Bearer).normalize(&limits()).is_err());
    assert!(Principal::new("omegon", "  ", AuthSource::Bearer).normalize(&limits()).is_err());
    let mut wide = RequestContext::new();
    for index in 0..=Limits::default().max_attributes {
        wide = wide.with_attribute(format!("a{index}"), "v");
    }
    let policy = omegon_policy();
    let operator = principal("otto", &["operator"]);
    assert_denies(
        &policy,
        Some(&operator),
        "omegon.surface.read",
        &wide,
        DecisionReason::MissingClaim,
    );
}

#[test]
fn adversarial_policy_sizes_are_rejected_before_activation() {
    let limits = Limits { max_grants: 2, max_roles: 1, ..Limits::default() };
    let too_many_grants = PolicyBuilder::new()
        .with_limits(limits)
        .grant_line("allow any a.b")
        .grant_line("allow any a.c")
        .grant_line("allow any a.d");
    assert!(matches!(too_many_grants.build(), Err(PolicyError::TooManyGrants { .. })));
    let too_many_roles = PolicyBuilder::new()
        .with_limits(limits)
        .role(RoleBundle::new("a"))
        .role(RoleBundle::new("b"));
    assert!(matches!(too_many_roles.build(), Err(PolicyError::TooManyRoles { .. })));
    let mut wide = String::from("allow any a.b");
    for index in 0..=Limits::default().max_constraints_per_grant {
        wide.push_str(&format!(" k{index}:v"));
    }
    assert!(matches!(
        PolicyBuilder::new().grant_line(wide).build(),
        Err(PolicyError::TooManyConstraints { .. })
    ));
}

fn extractor() -> PrincipalExtractor {
    let mapping = IssuerMapping {
        roles: BTreeMap::from([
            ("operator".to_string(), "operator".to_string()),
            ("viewer".to_string(), "monitor".to_string()),
        ]),
        retained_claims: vec!["Workspace".into()],
    };
    PrincipalExtractor::new(
        TrustedIssuerConfig::new("Omegon-Principal-").trust("omegon-proxy", mapping),
        limits(),
    )
}

#[test]
fn trusted_issuer_extraction_requires_authentication_and_a_trusted_issuer() {
    let extractor = extractor();
    let headers = [
        ("omegon-principal-issuer", "omegon-proxy"),
        ("Omegon-Principal-Subject", "alice"),
        ("Omegon-Principal-Role", "Viewer"),
        ("Omegon-Principal-Display-Name", "Alice"),
        ("Omegon-Principal-Session-Id", "sess-1"),
        ("Omegon-Principal-Client-Id", "web"),
        ("Omegon-Principal-Workspace", "blue"),
        ("Omegon-Principal-Token", "should-not-be-retained"),
        ("Authorization", "Bearer secret"),
    ];
    let principal = extractor
        .extract(AuthenticationState::Authenticated(AuthSource::Bearer), headers)
        .expect("principal");
    assert_eq!(principal.subject(), "alice");
    assert_eq!(principal.issuer(), "omegon-proxy");
    assert_eq!(principal.roles(), ["monitor"]);
    assert_eq!(principal.session_id(), Some("sess-1"));
    assert_eq!(principal.claim("workspace"), Some("blue"));
    assert_eq!(principal.claim("token"), None);
    assert_eq!(principal.claim_count(), 1);

    let error = extractor.extract(AuthenticationState::Unauthenticated, headers).unwrap_err();
    assert_eq!(error, IssuerError::MissingAuthentication);
    assert_eq!(error.reason(), DecisionReason::MissingAuthentication);

    let untrusted = [
        ("Omegon-Principal-Issuer", "evil"),
        ("Omegon-Principal-Subject", "alice"),
        ("Omegon-Principal-Role", "viewer"),
    ];
    let error = extractor
        .extract(AuthenticationState::Authenticated(AuthSource::Bearer), untrusted)
        .unwrap_err();
    assert!(matches!(error, IssuerError::UntrustedIssuer { .. }));
    assert_eq!(error.reason(), DecisionReason::UntrustedIssuer);

    let no_subject =
        [("Omegon-Principal-Issuer", "omegon-proxy"), ("Omegon-Principal-Role", "viewer")];
    assert_eq!(
        extractor
            .extract(AuthenticationState::Authenticated(AuthSource::Bearer), no_subject)
            .unwrap_err(),
        IssuerError::MissingSubject
    );
    let unmapped = [
        ("Omegon-Principal-Issuer", "omegon-proxy"),
        ("Omegon-Principal-Subject", "a"),
        ("Omegon-Principal-Role", "root"),
    ];
    assert!(matches!(
        extractor
            .extract(AuthenticationState::Authenticated(AuthSource::Bearer), unmapped)
            .unwrap_err(),
        IssuerError::UnmappedRole { .. }
    ));
    let no_issuer = [("Omegon-Principal-Subject", "a"), ("Omegon-Principal-Role", "viewer")];
    assert_eq!(
        extractor
            .extract(AuthenticationState::Authenticated(AuthSource::Bearer), no_issuer)
            .unwrap_err(),
        IssuerError::MissingIssuer
    );
}

#[test]
fn spoofed_and_conflicting_headers_never_escalate() {
    let extractor = extractor();
    let spoofed = [
        ("Omegon-Principal-Issuer", "omegon-proxy"),
        ("Omegon-Principal-Subject", "alice"),
        ("Omegon-Principal-Role", "viewer"),
        ("Omegon-Principal-Role", "operator"),
    ];
    assert!(matches!(
        extractor
            .extract(AuthenticationState::Authenticated(AuthSource::Bearer), spoofed)
            .unwrap_err(),
        IssuerError::ConflictingHeader { .. }
    ));
    // A different prefix is ignored entirely, so it cannot supply identity.
    let other_prefix = [
        ("X-Principal-Issuer", "omegon-proxy"),
        ("X-Principal-Subject", "alice"),
        ("X-Principal-Role", "viewer"),
    ];
    assert_eq!(
        extractor
            .extract(AuthenticationState::Authenticated(AuthSource::Bearer), other_prefix)
            .unwrap_err(),
        IssuerError::MissingIssuer
    );
    let renamed = PrincipalExtractor::new(
        TrustedIssuerConfig::new("X-Principal-")
            .trust("omegon-proxy", extractor.config().issuers["omegon-proxy"].clone()),
        limits(),
    );
    assert_eq!(
        renamed
            .extract(AuthenticationState::Authenticated(AuthSource::Session), other_prefix)
            .unwrap()
            .roles(),
        ["monitor"]
    );
}

#[test]
fn discovery_agrees_with_enforcement_and_reports_policy_shape() {
    let policy = omegon_policy();
    let alice = Principal::new("omegon", "alice", AuthSource::Bearer)
        .with_roles(["monitor"])
        .normalize(&limits())
        .unwrap();
    let context = RequestContext::new();
    let descriptor = policy.describe(&alice, &context);
    assert_eq!(descriptor.principal.subject, "alice");
    assert!(!descriptor.operations.is_empty());
    for entry in &descriptor.operations {
        let decision =
            policy.evaluate(&AuthorizationRequest::new(Some(&alice), &entry.operation, &context));
        assert_eq!(entry.allowed, decision.allowed, "{}", entry.operation);
        assert_eq!(entry.reason, decision.reason, "{}", entry.operation);
        assert_eq!(entry.requirements, decision.required, "{}", entry.operation);
    }
    let surface = descriptor
        .operations
        .iter()
        .find(|entry| entry.operation == "omegon.surface.read")
        .unwrap();
    assert!(surface.allowed);
    assert_eq!(surface.scopes, vec!["session:default"]);
    let login = descriptor
        .operations
        .iter()
        .find(|entry| entry.operation == "omegon.ws.auth_login")
        .unwrap();
    assert!(!login.allowed);
    assert_eq!(login.requirements, vec!["admin"]);
    assert!(!descriptor.warnings.contains(&DescriptorWarning::CoarseRoles));

    // Discovery cannot grant: a stale "allowed" descriptor does not survive a policy change.
    let slot = PolicySlot::with_policy(policy);
    slot.load(
        PolicyBuilder::new()
            .role(RoleBundle::new("monitor"))
            .declare_operation("omegon.surface.read"),
    )
    .expect("stricter policy loads");
    let decision =
        slot.evaluate(&AuthorizationRequest::new(Some(&alice), "omegon.surface.read", &context));
    assert_eq!(decision.reason, DecisionReason::CapabilityNotGranted);

    let coarse = PolicyBuilder::new()
        .roles(RoleBundle::styrene_compatibility(&limits()).unwrap())
        .build()
        .unwrap();
    let peer = Principal::new("mesh", "peer", AuthSource::Mesh)
        .with_roles(["peer"])
        .normalize(&limits())
        .unwrap();
    let descriptor = coarse.describe(&peer, &context);
    assert!(descriptor.warnings.contains(&DescriptorWarning::CoarseRoles));
    let stranger = principal("stan", &["superuser"]);
    assert!(
        coarse.describe(&stranger, &context).warnings.contains(&DescriptorWarning::UnknownRole)
    );
    let empty = PolicyBuilder::new().build().unwrap();
    assert!(empty.describe(&peer, &context).warnings.contains(&DescriptorWarning::Empty));
}

#[test]
fn styrene_coarse_roles_are_bundles_that_cannot_bypass_explicit_denies() {
    let limits = limits();
    let policy = PolicyBuilder::new()
        .roles(RoleBundle::styrene_compatibility(&limits).unwrap())
        .grant_line("deny user:rogue rpc.exec id=rogue-exec")
        .build()
        .unwrap();
    let context = RequestContext::new();
    matrix_test_roles(&policy, "chat.send", &["peer", "monitor", "operator", "admin"]);
    matrix_test_roles(&policy, "rpc.config_update", &["operator", "admin"]);
    matrix_test_roles(&policy, "rpc.exec", &["admin"]);
    matrix_test_roles(&policy, "vpn.handshake", &[]);
    let rogue = principal("rogue", &["admin"]);
    assert_denies(&policy, Some(&rogue), "rpc.exec", &context, DecisionReason::ExplicitDeny);
    assert_allows(&policy, &rogue, "rpc.reboot", &context);
    for role in ["peer", "monitor", "operator", "admin"] {
        let expected =
            styrene_rbac::capabilities_for_role(styrene_rbac::Role::from_name(role).unwrap());
        for capability in expected {
            assert_allows(&policy, &principal("x", &[role]), capability, &context);
        }
    }
}

#[cfg(feature = "config")]
#[test]
fn decisions_and_descriptors_serialize_with_stable_field_names() {
    let policy = omegon_policy();
    let operator = principal("otto", &["operator"]);
    let context = RequestContext::new().with_request_id("r1");
    let decision = policy.evaluate(&AuthorizationRequest::new(
        Some(&operator),
        "omegon.event.ingress",
        &context,
    ));
    let json = serde_json::to_value(&decision).unwrap();
    assert_eq!(json["allowed"], true);
    assert_eq!(json["reason"], "allowed");
    assert_eq!(json["audit"]["request_id"], "r1");
    assert_eq!(json["principal"]["auth_source"], "bearer");
    assert!(json["principal"].get("claims").is_none());
    let descriptor = policy.describe(&operator, &context);
    let json = serde_json::to_value(&descriptor).unwrap();
    assert!(
        json["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["operation"] == "omegon.surface.read")
    );
    let mapping: IssuerMapping = serde_json::from_str(r#"{"roles":{"viewer":"monitor"}}"#).unwrap();
    assert_eq!(mapping.roles["viewer"], "monitor");
    assert!(mapping.retained_claims.is_empty());
}
