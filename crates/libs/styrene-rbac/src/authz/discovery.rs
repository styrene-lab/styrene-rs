//! Effective-policy discovery for clients, evaluated with enforcement semantics.

use super::decision::{AuthorizationRequest, DecisionReason, RequestContext};
use super::policy::OperationPolicy;
use super::principal::{Principal, PrincipalSummary};

/// Warnings about the shape of the policy a descriptor came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "config", serde(rename_all = "snake_case"))]
pub enum DescriptorWarning {
    /// Every operation is reachable only through role bundles.
    CoarseRoles,
    /// The principal holds a role the policy does not define; results are
    /// denials, not a description of intent.
    UnknownRole,
    /// At least one operation depends on constraints the descriptor's
    /// context did not supply, so its allowed state is partial.
    Partial,
    /// The policy declares no operations.
    Empty,
}

/// One declared operation as the principal would experience it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct OperationDescriptor {
    pub operation: String,
    pub allowed: bool,
    pub reason: DecisionReason,
    /// Role bundles that allow the operation without constraints.
    pub requirements: Vec<String>,
    /// Constraints on grants that could allow the operation for this principal.
    pub scopes: Vec<String>,
}

/// A client-facing projection. It describes; it never grants.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct PolicyDescriptor {
    pub principal: PrincipalSummary,
    pub operations: Vec<OperationDescriptor>,
    pub warnings: Vec<DescriptorWarning>,
}

impl OperationPolicy {
    /// Describe every declared operation for `principal` under `context` by
    /// running the same evaluation enforcement uses.
    #[must_use]
    pub fn describe(&self, principal: &Principal, context: &RequestContext) -> PolicyDescriptor {
        let mut operations = Vec::new();
        let mut warnings = Vec::new();
        let mut unknown_role = false;
        let mut partial = false;
        for operation in self.declared_operations() {
            let request = AuthorizationRequest::new(Some(principal), operation.as_str(), context);
            let decision = self.evaluate(&request);
            let scopes = self.scopes_for(principal, &operation);
            if decision.reason == DecisionReason::UnknownRole {
                unknown_role = true;
            }
            if !decision.allowed && !scopes.is_empty() {
                partial = true;
            }
            operations.push(OperationDescriptor {
                operation: operation.to_string(),
                allowed: decision.allowed,
                reason: decision.reason,
                requirements: decision.required,
                scopes,
            });
        }
        if operations.is_empty() {
            warnings.push(DescriptorWarning::Empty);
        }
        if self.grants().is_empty() && !operations.is_empty() {
            warnings.push(DescriptorWarning::CoarseRoles);
        }
        if unknown_role {
            warnings.push(DescriptorWarning::UnknownRole);
        }
        if partial {
            warnings.push(DescriptorWarning::Partial);
        }
        PolicyDescriptor { principal: principal.summary(), operations, warnings }
    }
}
