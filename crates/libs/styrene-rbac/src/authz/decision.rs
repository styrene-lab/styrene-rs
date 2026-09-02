//! Requests, decisions, and audit projections.

use std::collections::BTreeMap;
use std::fmt;

use super::grant::GrantId;
use super::principal::{Principal, PrincipalSummary};

/// The stable, machine-readable outcome class of a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "config", serde(rename_all = "snake_case"))]
pub enum DecisionReason {
    Allowed,
    MissingAuthentication,
    UnknownRole,
    MissingClaim,
    UntrustedIssuer,
    CapabilityNotGranted,
    ExplicitDeny,
    PolicyUnavailable,
    MisconfiguredOperation,
}

impl DecisionReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::MissingAuthentication => "missing_authentication",
            Self::UnknownRole => "unknown_role",
            Self::MissingClaim => "missing_claim",
            Self::UntrustedIssuer => "untrusted_issuer",
            Self::CapabilityNotGranted => "capability_not_granted",
            Self::ExplicitDeny => "explicit_deny",
            Self::PolicyUnavailable => "policy_unavailable",
            Self::MisconfiguredOperation => "misconfigured_operation",
        }
    }

    /// Every reason, in a stable order, for exhaustive consumer tables.
    pub const ALL: [Self; 9] = [
        Self::Allowed,
        Self::MissingAuthentication,
        Self::UnknownRole,
        Self::MissingClaim,
        Self::UntrustedIssuer,
        Self::CapabilityNotGranted,
        Self::ExplicitDeny,
        Self::PolicyUnavailable,
        Self::MisconfiguredOperation,
    ];
}

impl fmt::Display for DecisionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stable resource reference: a kind such as `session` and an identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct ResourceRef {
    pub kind: String,
    pub id: String,
}

impl ResourceRef {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self { kind: kind.into(), id: id.into() }
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.id)
    }
}

/// Bounded request attributes and correlation fields. Attributes such as
/// `trigger_kind` feed context constraints; the rest only reach the audit
/// projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestContext {
    attributes: BTreeMap<String, String>,
    pub request_id: Option<String>,
    pub correlation_id: Option<String>,
    pub route: Option<String>,
    pub method: Option<String>,
    /// Unix seconds supplied by the caller; the evaluator reads no clock.
    pub timestamp: Option<u64>,
}

impl RequestContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn with_route(mut self, method: impl Into<String>, route: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self.route = Some(route.into());
        self
    }

    #[must_use]
    pub fn with_timestamp(mut self, unix_seconds: u64) -> Self {
        self.timestamp = Some(unix_seconds);
        self
    }

    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    #[must_use]
    pub fn attribute_count(&self) -> usize {
        self.attributes.len()
    }
}

/// One authorization question.
#[derive(Clone, Debug)]
pub struct AuthorizationRequest<'a> {
    /// `None` means the caller never authenticated anyone.
    pub principal: Option<&'a Principal>,
    pub operation: &'a str,
    pub resource: Option<&'a ResourceRef>,
    pub context: &'a RequestContext,
}

impl<'a> AuthorizationRequest<'a> {
    pub fn new(
        principal: Option<&'a Principal>,
        operation: &'a str,
        context: &'a RequestContext,
    ) -> Self {
        Self { principal, operation, resource: None, context }
    }

    #[must_use]
    pub fn with_resource(mut self, resource: &'a ResourceRef) -> Self {
        self.resource = Some(resource);
        self
    }
}

/// The audit-ready projection of a decision. Contains no credentials,
/// bearer values, display names, or claim values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct AuditFields {
    pub timestamp: Option<u64>,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub operation: String,
    pub resource: Option<String>,
    pub route: Option<String>,
    pub method: Option<String>,
    pub allowed: bool,
    pub reason: String,
    pub matched_grant: Option<String>,
    pub request_id: Option<String>,
    pub correlation_id: Option<String>,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
    /// The `trigger_kind` or `action_kind` attribute, when the request set one.
    pub trigger_kind: Option<String>,
}

/// One structured, explainable decision.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub reason: DecisionReason,
    /// The normalized operation, or the raw text when it could not normalize.
    pub operation: String,
    /// Role bundles that would allow the operation without constraints.
    pub required: Vec<String>,
    pub matched_grant: Option<GrantId>,
    pub principal: Option<PrincipalSummary>,
    pub resource: Option<ResourceRef>,
    pub audit: AuditFields,
}

/// The evaluator's verdict before it is joined with the request.
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub allowed: bool,
    pub reason: DecisionReason,
    pub operation: String,
    pub required: Vec<String>,
    pub matched_grant: Option<GrantId>,
}

impl Outcome {
    pub(crate) fn deny(reason: DecisionReason, operation: String) -> Self {
        Self { allowed: false, reason, operation, required: Vec::new(), matched_grant: None }
    }
}

impl AuthorizationDecision {
    pub(crate) fn build(outcome: Outcome, request: &AuthorizationRequest<'_>) -> Self {
        let Outcome { allowed, reason, operation, required, matched_grant } = outcome;
        let principal = request.principal;
        let resource = request.resource;
        let context = request.context;
        let summary = principal.map(Principal::summary);
        let audit = AuditFields {
            timestamp: context.timestamp,
            subject: summary.as_ref().map(|s| s.subject.clone()),
            issuer: summary.as_ref().map(|s| s.issuer.clone()),
            roles: summary.as_ref().map(|s| s.roles.clone()).unwrap_or_default(),
            groups: summary.as_ref().map(|s| s.groups.clone()).unwrap_or_default(),
            operation: operation.clone(),
            resource: resource.map(ToString::to_string),
            route: context.route.clone(),
            method: context.method.clone(),
            allowed,
            reason: reason.as_str().to_string(),
            matched_grant: matched_grant.as_ref().map(|grant| grant.as_str().to_string()),
            request_id: context.request_id.clone(),
            correlation_id: context.correlation_id.clone(),
            client_id: summary.as_ref().and_then(|s| s.client_id.clone()),
            session_id: summary.as_ref().and_then(|s| s.session_id.clone()),
            trigger_kind: context
                .attribute("trigger_kind")
                .or_else(|| context.attribute("action_kind"))
                .map(ToOwned::to_owned),
        };
        Self {
            allowed,
            reason,
            operation,
            required,
            matched_grant,
            principal: summary,
            resource: resource.cloned(),
            audit,
        }
    }
}
