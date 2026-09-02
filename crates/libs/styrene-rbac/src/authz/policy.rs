//! Policy loading, validation, and evaluation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, RwLock};

use super::decision::{AuthorizationDecision, AuthorizationRequest, DecisionReason, Outcome};
use super::grant::{Constraint, Effect, Grant, GrantId, GrantParseError, RoleBundle, Selector};
use super::limits::Limits;
use super::operation::{Operation, OperationError, OperationPattern};
use super::principal::Principal;

/// Why a policy could not be built. Nothing is activated on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    Grant { index: usize, error: GrantParseError },
    InvalidOperation { operation: String, error: OperationError },
    UnknownRole { role: String, referenced_by: String },
    RoleCycle { path: Vec<String> },
    RoleDepth { role: String, max: usize },
    DuplicateRole { role: String },
    DuplicateGrantId { id: String },
    ConflictingConstraint { grant: String, first: String, second: String },
    TooManyGrants { count: usize, max: usize },
    TooManyRoles { count: usize, max: usize },
    TooManyConstraints { grant: String, count: usize, max: usize },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grant { index, error } => write!(f, "grant {index}: {error}"),
            Self::InvalidOperation { operation, error } => {
                write!(f, "operation {operation:?}: {error}")
            }
            Self::UnknownRole { role, referenced_by } => {
                write!(f, "role {role:?} referenced by {referenced_by} is not defined")
            }
            Self::RoleCycle { path } => write!(f, "role inheritance cycle: {}", path.join(" -> ")),
            Self::RoleDepth { role, max } => {
                write!(f, "role {role:?} inherits deeper than the limit of {max}")
            }
            Self::DuplicateRole { role } => write!(f, "role {role:?} is defined twice"),
            Self::DuplicateGrantId { id } => write!(f, "grant id {id:?} is used twice"),
            Self::ConflictingConstraint { grant, first, second } => {
                write!(f, "grant {grant} can never apply: {first} conflicts with {second}")
            }
            Self::TooManyGrants { count, max } => {
                write!(f, "{count} grants exceed the limit of {max}")
            }
            Self::TooManyRoles { count, max } => {
                write!(f, "{count} roles exceed the limit of {max}")
            }
            Self::TooManyConstraints { grant, count, max } => {
                write!(f, "grant {grant} has {count} constraints; the limit is {max}")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// Collects grants, role bundles, and declared operations, then validates
/// them all at once.
#[derive(Clone, Debug, Default)]
pub struct PolicyBuilder {
    limits: Limits,
    grants: Vec<Grant>,
    roles: Vec<RoleBundle>,
    declared: Vec<String>,
    grant_lines: Vec<String>,
}

impl PolicyBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    #[must_use]
    pub fn grant(mut self, grant: Grant) -> Self {
        self.grants.push(grant);
        self
    }

    /// Add a grant written in the text notation; it is parsed at build time.
    #[must_use]
    pub fn grant_line(mut self, line: impl Into<String>) -> Self {
        self.grant_lines.push(line.into());
        self
    }

    #[must_use]
    pub fn role(mut self, bundle: RoleBundle) -> Self {
        self.roles.push(bundle);
        self
    }

    #[must_use]
    pub fn roles<I: IntoIterator<Item = RoleBundle>>(mut self, bundles: I) -> Self {
        self.roles.extend(bundles);
        self
    }

    /// Declare an operation so discovery lists it and evaluation reports
    /// `capability_not_granted` rather than `misconfigured_operation`.
    #[must_use]
    pub fn declare_operation(mut self, operation: impl Into<String>) -> Self {
        self.declared.push(operation.into());
        self
    }

    /// Validate everything and produce an immutable policy.
    pub fn build(self) -> Result<OperationPolicy, PolicyError> {
        let limits = self.limits;
        let mut grants = self.grants;
        for (index, line) in self.grant_lines.iter().enumerate() {
            let default_id = format!("line-{}", index + 1);
            grants.push(
                Grant::parse(line, default_id, &limits)
                    .map_err(|error| PolicyError::Grant { index, error })?,
            );
        }
        if grants.len() > limits.max_grants {
            return Err(PolicyError::TooManyGrants { count: grants.len(), max: limits.max_grants });
        }
        if self.roles.len() > limits.max_roles {
            return Err(PolicyError::TooManyRoles {
                count: self.roles.len(),
                max: limits.max_roles,
            });
        }

        let mut roles: BTreeMap<String, RoleBundle> = BTreeMap::new();
        for bundle in self.roles {
            if roles.insert(bundle.name.clone(), bundle.clone()).is_some() {
                return Err(PolicyError::DuplicateRole { role: bundle.name });
            }
        }
        for bundle in roles.values() {
            for parent in &bundle.inherits {
                if !roles.contains_key(parent) {
                    return Err(PolicyError::UnknownRole {
                        role: parent.clone(),
                        referenced_by: format!("role {}", bundle.name),
                    });
                }
            }
        }
        let mut expansions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for name in roles.keys() {
            let mut path = Vec::new();
            let expanded = expand_role(name, &roles, &mut path, limits.max_role_depth)?;
            expansions.insert(name.clone(), expanded);
        }

        let mut ids = BTreeSet::new();
        for grant in &grants {
            if !ids.insert(grant.id.clone()) {
                return Err(PolicyError::DuplicateGrantId { id: grant.id.as_str().to_string() });
            }
            if grant.constraints.len() > limits.max_constraints_per_grant {
                return Err(PolicyError::TooManyConstraints {
                    grant: grant.id.as_str().to_string(),
                    count: grant.constraints.len(),
                    max: limits.max_constraints_per_grant,
                });
            }
            if let Some((first, second)) = grant.conflicting_constraints() {
                return Err(PolicyError::ConflictingConstraint {
                    grant: grant.id.as_str().to_string(),
                    first: first.to_string(),
                    second: second.to_string(),
                });
            }
            if let Selector::Role(role) = &grant.selector
                && !roles.contains_key(role)
            {
                return Err(PolicyError::UnknownRole {
                    role: role.clone(),
                    referenced_by: format!("grant {}", grant.id),
                });
            }
        }

        let mut declared = BTreeSet::new();
        for operation in &self.declared {
            let parsed = Operation::parse(operation, &limits).map_err(|error| {
                PolicyError::InvalidOperation { operation: operation.clone(), error }
            })?;
            declared.insert(parsed);
        }
        let exact_patterns = grants
            .iter()
            .map(|grant| &grant.pattern)
            .chain(roles.values().flat_map(|bundle| bundle.allow.iter().chain(bundle.deny.iter())));
        for pattern in exact_patterns {
            if let OperationPattern::Exact(operation) = pattern {
                declared.insert(operation.clone());
            }
        }

        Ok(OperationPolicy { limits, grants, roles, expansions, declared })
    }
}

fn expand_role(
    name: &str,
    roles: &BTreeMap<String, RoleBundle>,
    path: &mut Vec<String>,
    max_depth: usize,
) -> Result<Vec<String>, PolicyError> {
    if path.iter().any(|seen| seen == name) {
        let mut cycle = path.clone();
        cycle.push(name.to_string());
        return Err(PolicyError::RoleCycle { path: cycle });
    }
    path.push(name.to_string());
    if path.len() > max_depth {
        return Err(PolicyError::RoleDepth { role: path[0].clone(), max: max_depth });
    }
    let mut expanded = vec![name.to_string()];
    if let Some(bundle) = roles.get(name) {
        for parent in &bundle.inherits {
            for role in expand_role(parent, roles, path, max_depth)? {
                if !expanded.contains(&role) {
                    expanded.push(role);
                }
            }
        }
    }
    path.pop();
    Ok(expanded)
}

/// A validated, immutable policy.
#[derive(Clone, Debug)]
pub struct OperationPolicy {
    limits: Limits,
    grants: Vec<Grant>,
    roles: BTreeMap<String, RoleBundle>,
    /// Each role with every role it inherits, transitively, itself first.
    expansions: BTreeMap<String, Vec<String>>,
    declared: BTreeSet<Operation>,
}

/// One rule that matched during evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Match {
    effect: Effect,
    id: GrantId,
    specificity: usize,
}

impl OperationPolicy {
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Declared operations: explicit declarations plus every exact pattern
    /// in a grant or bundle, sorted.
    #[must_use]
    pub fn declared_operations(&self) -> Vec<Operation> {
        self.declared.iter().cloned().collect()
    }

    #[must_use]
    pub fn role_names(&self) -> Vec<String> {
        self.roles.keys().cloned().collect()
    }

    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// The role itself plus everything it inherits, or `None` when the
    /// policy does not define it.
    #[must_use]
    pub fn effective_roles(&self, role: &str) -> Option<&[String]> {
        self.expansions.get(role).map(Vec::as_slice)
    }

    /// The transitive role set for a principal, or the first unknown role.
    fn principal_roles(&self, principal: &Principal) -> Result<Vec<String>, String> {
        let mut effective = Vec::new();
        for role in principal.roles() {
            let Some(expanded) = self.expansions.get(role) else {
                return Err(role.clone());
            };
            for role in expanded {
                if !effective.contains(role) {
                    effective.push(role.clone());
                }
            }
        }
        Ok(effective)
    }

    /// Role bundles whose unconstrained grants allow `operation`, sorted.
    /// These are the decision's `required` list.
    fn roles_allowing(&self, operation: &Operation) -> Vec<String> {
        let mut required: Vec<String> = self
            .roles
            .iter()
            .filter(|(name, _)| {
                let Some(expanded) = self.expansions.get(*name) else { return false };
                let denied = expanded
                    .iter()
                    .filter_map(|role| self.roles.get(role))
                    .any(|bundle| bundle.deny.iter().any(|pattern| pattern.matches(operation)));
                let allowed = expanded
                    .iter()
                    .filter_map(|role| self.roles.get(role))
                    .any(|bundle| bundle.allow.iter().any(|pattern| pattern.matches(operation)));
                allowed && !denied
            })
            .map(|(name, _)| name.clone())
            .collect();
        required.sort();
        required
    }

    fn is_declared(&self, operation: &Operation) -> bool {
        self.declared.contains(operation)
            || self.grants.iter().any(|grant| grant.pattern.matches(operation))
            || self.roles.values().any(|bundle| {
                bundle
                    .allow
                    .iter()
                    .chain(bundle.deny.iter())
                    .any(|pattern| pattern.matches(operation))
            })
    }

    /// Evaluate one request. Never panics; every failure is a denial with a
    /// stable reason.
    #[must_use]
    pub fn evaluate(&self, request: &AuthorizationRequest<'_>) -> AuthorizationDecision {
        AuthorizationDecision::build(self.outcome(request), request)
    }

    fn outcome(&self, request: &AuthorizationRequest<'_>) -> Outcome {
        let context = request.context;
        let Some(principal) = request.principal else {
            return Outcome::deny(
                DecisionReason::MissingAuthentication,
                request.operation.to_string(),
            );
        };
        let Ok(operation) = Operation::parse(request.operation, &self.limits) else {
            return Outcome::deny(
                DecisionReason::MisconfiguredOperation,
                request.operation.to_string(),
            );
        };
        if context.attribute_count() > self.limits.max_attributes
            || principal.claim_count() > self.limits.max_claims
        {
            return Outcome::deny(DecisionReason::MissingClaim, operation.to_string());
        }
        let Ok(effective_roles) = self.principal_roles(principal) else {
            return Outcome::deny(DecisionReason::UnknownRole, operation.to_string());
        };
        let required = self.roles_allowing(&operation);

        let mut matches: Vec<Match> = Vec::new();
        for grant in &self.grants {
            if grant.pattern.matches(&operation)
                && grant.applies(principal, &effective_roles, request.resource, context)
            {
                matches.push(Match {
                    effect: grant.effect,
                    id: grant.id.clone(),
                    specificity: grant.pattern.specificity(),
                });
            }
        }
        for role in &effective_roles {
            let Some(bundle) = self.roles.get(role) else { continue };
            for pattern in &bundle.deny {
                if pattern.matches(&operation) {
                    matches.push(Match {
                        effect: Effect::Deny,
                        id: GrantId::new(format!("role:{role}:deny:{pattern}")),
                        specificity: pattern.specificity(),
                    });
                }
            }
            for pattern in &bundle.allow {
                if pattern.matches(&operation) {
                    matches.push(Match {
                        effect: Effect::Allow,
                        id: GrantId::new(format!("role:{role}:allow:{pattern}")),
                        specificity: pattern.specificity(),
                    });
                }
            }
        }

        let best = |effect: Effect| {
            matches
                .iter()
                .filter(|m| m.effect == effect)
                .max_by_key(|m| (m.specificity, std::cmp::Reverse(m.id.clone())))
                .map(|m| m.id.clone())
        };
        if let Some(denied_by) = best(Effect::Deny) {
            return Outcome {
                allowed: false,
                reason: DecisionReason::ExplicitDeny,
                operation: operation.to_string(),
                required,
                matched_grant: Some(denied_by),
            };
        }
        if let Some(allowed_by) = best(Effect::Allow) {
            return Outcome {
                allowed: true,
                reason: DecisionReason::Allowed,
                operation: operation.to_string(),
                required,
                matched_grant: Some(allowed_by),
            };
        }
        if self.is_declared(&operation) {
            Outcome {
                allowed: false,
                reason: DecisionReason::CapabilityNotGranted,
                operation: operation.to_string(),
                required,
                matched_grant: None,
            }
        } else {
            Outcome::deny(DecisionReason::MisconfiguredOperation, operation.to_string())
        }
    }

    /// Constraint texts on grants that could allow `operation` for the
    /// principal, for discovery scopes.
    pub(crate) fn scopes_for(&self, principal: &Principal, operation: &Operation) -> Vec<String> {
        let effective = self.principal_roles(principal).unwrap_or_default();
        let mut scopes: Vec<String> = self
            .grants
            .iter()
            .filter(|grant| {
                grant.effect == Effect::Allow
                    && grant.pattern.matches(operation)
                    && grant.selector.matches(principal, &effective)
                    && !grant.constraints.is_empty()
            })
            .flat_map(|grant| grant.constraints.iter().map(Constraint::to_string))
            .collect();
        scopes.sort();
        scopes.dedup();
        scopes
    }
}

/// The currently active policy. Loading a new policy that fails validation
/// leaves the previous policy authoritative; with no valid policy every
/// request is denied with `policy_unavailable`.
#[derive(Debug, Default)]
pub struct PolicySlot {
    current: RwLock<Option<Arc<OperationPolicy>>>,
}

impl PolicySlot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_policy(policy: OperationPolicy) -> Self {
        Self { current: RwLock::new(Some(Arc::new(policy))) }
    }

    /// Build and activate atomically. On error nothing changes.
    pub fn load(&self, builder: PolicyBuilder) -> Result<(), PolicyError> {
        let policy = builder.build()?;
        *self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Arc::new(policy));
        Ok(())
    }

    /// Remove the active policy; every request now fails closed.
    pub fn clear(&self) {
        *self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    #[must_use]
    pub fn current(&self) -> Option<Arc<OperationPolicy>> {
        self.current.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    #[must_use]
    pub fn evaluate(&self, request: &AuthorizationRequest<'_>) -> AuthorizationDecision {
        match self.current() {
            Some(policy) => policy.evaluate(request),
            None => AuthorizationDecision::build(
                Outcome::deny(DecisionReason::PolicyUnavailable, request.operation.to_string()),
                request,
            ),
        }
    }
}
