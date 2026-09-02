//! Grants, selectors, constraints, and role bundles.

use std::fmt;

use super::decision::{RequestContext, ResourceRef};
use super::limits::Limits;
use super::operation::{OperationError, OperationPattern};
use super::principal::Principal;

/// Allow or deny. A matching deny always wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "config", serde(rename_all = "snake_case"))]
pub enum Effect {
    Allow,
    Deny,
}

/// Who a grant applies to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Selector {
    /// `user:<subject>`
    Subject(String),
    /// `group:<group>`
    Group(String),
    /// `role:<role>` or a bare role name in grant text.
    Role(String),
    /// `any`: every authenticated principal.
    Any,
}

impl Selector {
    /// Parse `user:alice`, `group:ops`, `role:operator`, `any`, or a bare
    /// role name.
    pub fn parse(raw: &str) -> Result<Self, GrantParseError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(GrantParseError::MissingSelector);
        }
        let lowered = raw.to_ascii_lowercase();
        if lowered == "any" {
            return Ok(Self::Any);
        }
        match lowered.split_once(':') {
            Some(("user", subject)) if !subject.is_empty() => {
                Ok(Self::Subject(subject.to_string()))
            }
            Some(("group", group)) if !group.is_empty() => Ok(Self::Group(group.to_string())),
            Some(("role", role)) if !role.is_empty() => Ok(Self::Role(role.to_string())),
            Some(_) => Err(GrantParseError::InvalidSelector { selector: raw.to_string() }),
            None => Ok(Self::Role(lowered)),
        }
    }

    pub(crate) fn matches(&self, principal: &Principal, effective_roles: &[String]) -> bool {
        match self {
            Self::Subject(subject) => principal.subject().eq_ignore_ascii_case(subject),
            Self::Group(group) => principal.groups().iter().any(|g| g == group),
            Self::Role(role) => effective_roles.iter().any(|r| r == role),
            Self::Any => true,
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Subject(subject) => write!(f, "user:{subject}"),
            Self::Group(group) => write!(f, "group:{group}"),
            Self::Role(role) => write!(f, "role:{role}"),
            Self::Any => f.write_str("any"),
        }
    }
}

/// An exact condition a grant requires before it applies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// The request names this resource kind with exactly this identifier.
    Resource { kind: String, id: String },
    /// The request context carries this attribute with exactly this value.
    Attribute { name: String, value: String },
    /// The principal carries this bounded claim with exactly this value.
    Claim { name: String, value: String },
}

impl Constraint {
    pub(crate) fn matches(
        &self,
        principal: &Principal,
        resource: Option<&ResourceRef>,
        context: &RequestContext,
    ) -> bool {
        match self {
            Self::Resource { kind, id } => {
                resource.is_some_and(|resource| &resource.kind == kind && &resource.id == id)
            }
            Self::Attribute { name, value } => context.attribute(name) == Some(value.as_str()),
            Self::Claim { name, value } => principal.claim(name) == Some(value.as_str()),
        }
    }

    /// Two constraints of the same kind and key with different values can
    /// never both hold.
    fn conflicts_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Resource { kind: a, id: x }, Self::Resource { kind: b, id: y }) => {
                a == b && x != y
            }
            (Self::Attribute { name: a, value: x }, Self::Attribute { name: b, value: y })
            | (Self::Claim { name: a, value: x }, Self::Claim { name: b, value: y }) => {
                a == b && x != y
            }
            _ => false,
        }
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource { kind, id } => write!(f, "{kind}:{id}"),
            Self::Attribute { name, value } => write!(f, "where {name} = \"{value}\""),
            Self::Claim { name, value } => write!(f, "where claim.{name} = \"{value}\""),
        }
    }
}

/// A stable identifier for a grant, used in decisions and audit output.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct GrantId(String);

impl GrantId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a grant line could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantParseError {
    Empty,
    InvalidEffect { effect: String },
    MissingSelector,
    InvalidSelector { selector: String },
    MissingOperation,
    InvalidPattern(OperationError),
    InvalidConstraint { text: String },
}

impl fmt::Display for GrantParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("grant is empty"),
            Self::InvalidEffect { effect } => {
                write!(f, "grant effect {effect:?} is not allow or deny")
            }
            Self::MissingSelector => f.write_str("grant has no principal selector"),
            Self::InvalidSelector { selector } => {
                write!(f, "grant selector {selector:?} is invalid")
            }
            Self::MissingOperation => f.write_str("grant has no operation"),
            Self::InvalidPattern(error) => write!(f, "grant operation is invalid: {error}"),
            Self::InvalidConstraint { text } => write!(f, "grant constraint {text:?} is invalid"),
        }
    }
}

impl std::error::Error for GrantParseError {}

/// One allow or deny rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: GrantId,
    pub effect: Effect,
    pub selector: Selector,
    pub pattern: OperationPattern,
    pub constraints: Vec<Constraint>,
}

impl Grant {
    pub fn new(
        id: impl Into<String>,
        effect: Effect,
        selector: Selector,
        pattern: OperationPattern,
    ) -> Self {
        Self { id: GrantId::new(id), effect, selector, pattern, constraints: Vec::new() }
    }

    #[must_use]
    pub fn with_constraint(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Parse one grant line in the issue's notation:
    ///
    /// ```text
    /// allow user:alice omegon.surface.read session:default
    /// deny operator omegon.event.ingress where trigger_kind = "shutdown"
    /// ```
    ///
    /// The line's position or an explicit `id=<name>` token names the grant.
    pub fn parse(
        line: &str,
        default_id: impl Into<String>,
        limits: &Limits,
    ) -> Result<Self, GrantParseError> {
        let mut tokens = line.split_whitespace().peekable();
        let effect = match tokens.next() {
            None => return Err(GrantParseError::Empty),
            Some(word) => match word.to_ascii_lowercase().as_str() {
                "allow" => Effect::Allow,
                "deny" => Effect::Deny,
                other => return Err(GrantParseError::InvalidEffect { effect: other.to_string() }),
            },
        };
        let selector = Selector::parse(tokens.next().ok_or(GrantParseError::MissingSelector)?)?;
        let pattern_text = tokens.next().ok_or(GrantParseError::MissingOperation)?;
        let pattern = OperationPattern::parse(pattern_text, limits)
            .map_err(GrantParseError::InvalidPattern)?;
        let mut grant = Self::new(default_id, effect, selector, pattern);
        let rest: Vec<&str> = tokens.collect();
        let mut index = 0;
        while index < rest.len() {
            let token = rest[index];
            if let Some(id) = token.strip_prefix("id=") {
                if id.is_empty() {
                    return Err(GrantParseError::InvalidConstraint { text: token.to_string() });
                }
                grant.id = GrantId::new(id);
                index += 1;
            } else if token.eq_ignore_ascii_case("where") {
                // where <name> = "<value>"
                let name = rest.get(index + 1).copied();
                let equals = rest.get(index + 2).copied();
                let value = rest.get(index + 3).copied();
                match (name, equals, value) {
                    (Some(name), Some("="), Some(value)) => {
                        let value = value.trim_matches('"').to_string();
                        let constraint = match name.strip_prefix("claim.") {
                            Some(claim) => Constraint::Claim { name: claim.to_string(), value },
                            None => Constraint::Attribute { name: name.to_string(), value },
                        };
                        grant.constraints.push(constraint);
                        index += 4;
                    }
                    _ => {
                        return Err(GrantParseError::InvalidConstraint {
                            text: rest[index..].join(" "),
                        });
                    }
                }
            } else if let Some((kind, id)) = token.split_once(':') {
                if kind.is_empty() || id.is_empty() {
                    return Err(GrantParseError::InvalidConstraint { text: token.to_string() });
                }
                grant
                    .constraints
                    .push(Constraint::Resource { kind: kind.to_string(), id: id.to_string() });
                index += 1;
            } else {
                return Err(GrantParseError::InvalidConstraint { text: token.to_string() });
            }
        }
        Ok(grant)
    }

    /// The first pair of constraints that can never both hold, if any.
    pub(crate) fn conflicting_constraints(&self) -> Option<(&Constraint, &Constraint)> {
        for (index, first) in self.constraints.iter().enumerate() {
            for second in &self.constraints[index + 1..] {
                if first.conflicts_with(second) {
                    return Some((first, second));
                }
            }
        }
        None
    }

    pub(crate) fn applies(
        &self,
        principal: &Principal,
        effective_roles: &[String],
        resource: Option<&ResourceRef>,
        context: &RequestContext,
    ) -> bool {
        self.selector.matches(principal, effective_roles)
            && self
                .constraints
                .iter()
                .all(|constraint| constraint.matches(principal, resource, context))
    }
}

/// A named bundle of grants that other bundles can inherit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleBundle {
    pub name: String,
    pub inherits: Vec<String>,
    pub allow: Vec<OperationPattern>,
    pub deny: Vec<OperationPattern>,
}

impl RoleBundle {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into().to_ascii_lowercase(),
            inherits: Vec::new(),
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }

    #[must_use]
    pub fn inherits(mut self, parent: impl Into<String>) -> Self {
        self.inherits.push(parent.into().to_ascii_lowercase());
        self
    }

    pub fn allow(mut self, pattern: &str, limits: &Limits) -> Result<Self, OperationError> {
        self.allow.push(OperationPattern::parse(pattern, limits)?);
        Ok(self)
    }

    pub fn deny(mut self, pattern: &str, limits: &Limits) -> Result<Self, OperationError> {
        self.deny.push(OperationPattern::parse(pattern, limits)?);
        Ok(self)
    }

    /// The existing Styrene coarse roles as data-backed bundles: `peer`,
    /// `monitor` (inherits peer), `operator` (inherits monitor), and `admin`
    /// (inherits operator), each allowing exactly its capability list.
    pub fn styrene_compatibility(limits: &Limits) -> Result<Vec<Self>, OperationError> {
        use crate::Role;
        let tiers = [
            (Role::Peer, None),
            (Role::Monitor, Some(Role::Peer)),
            (Role::Operator, Some(Role::Monitor)),
            (Role::Admin, Some(Role::Operator)),
        ];
        let mut bundles = Vec::with_capacity(tiers.len());
        for (role, parent) in tiers {
            let mut bundle = Self::new(role.as_str());
            if let Some(parent) = parent {
                bundle = bundle.inherits(parent.as_str());
            }
            for capability in crate::capabilities_for_role(role) {
                bundle = bundle.allow(capability, limits)?;
            }
            bundles.push(bundle);
        }
        Ok(bundles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_lines_parse_selectors_patterns_and_constraints() {
        let limits = Limits::default();
        let grant =
            Grant::parse("allow user:alice omegon.surface.read session:default", "g1", &limits)
                .unwrap();
        assert_eq!(grant.effect, Effect::Allow);
        assert_eq!(grant.selector, Selector::Subject("alice".into()));
        assert_eq!(grant.pattern.to_string(), "omegon.surface.read");
        assert_eq!(
            grant.constraints,
            vec![Constraint::Resource { kind: "session".into(), id: "default".into() }]
        );

        let grant = Grant::parse(
            "deny operator omegon.event.ingress where trigger_kind = \"shutdown\" id=no-shutdown",
            "g2",
            &limits,
        )
        .unwrap();
        assert_eq!(grant.effect, Effect::Deny);
        assert_eq!(grant.selector, Selector::Role("operator".into()));
        assert_eq!(grant.id.as_str(), "no-shutdown");
        assert_eq!(
            grant.constraints,
            vec![Constraint::Attribute { name: "trigger_kind".into(), value: "shutdown".into() }]
        );

        let grant =
            Grant::parse("allow group:operators omegon.native_session.*", "g3", &limits).unwrap();
        assert_eq!(grant.selector, Selector::Group("operators".into()));
        assert!(!grant.pattern.is_exact());

        assert_eq!(Grant::parse("", "g", &limits), Err(GrantParseError::Empty));
        assert!(matches!(
            Grant::parse("permit any x.y", "g", &limits),
            Err(GrantParseError::InvalidEffect { .. })
        ));
        assert!(matches!(
            Grant::parse("allow any", "g", &limits),
            Err(GrantParseError::MissingOperation)
        ));
        assert!(matches!(
            Grant::parse("allow any a.*.b", "g", &limits),
            Err(GrantParseError::InvalidPattern(_))
        ));
        assert!(matches!(
            Grant::parse("allow any a.b where x", "g", &limits),
            Err(GrantParseError::InvalidConstraint { .. })
        ));
        assert!(matches!(
            Grant::parse("allow team:x a.b", "g", &limits),
            Err(GrantParseError::InvalidSelector { .. })
        ));
    }

    #[test]
    fn conflicting_constraints_are_detected() {
        let limits = Limits::default();
        let grant = Grant::parse("allow any a.b session:one session:two", "g", &limits).unwrap();
        assert!(grant.conflicting_constraints().is_some());
        let grant = Grant::parse("allow any a.b session:one surface:main", "g", &limits).unwrap();
        assert!(grant.conflicting_constraints().is_none());
    }
}
