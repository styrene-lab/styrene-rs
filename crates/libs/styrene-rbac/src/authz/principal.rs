//! Authenticated principals and their audit-safe summaries.

use std::collections::BTreeMap;
use std::fmt;

use super::limits::Limits;

/// How the principal was authenticated before it reached the policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "config", serde(rename_all = "snake_case"))]
pub enum AuthSource {
    Bearer,
    Session,
    MutualTls,
    /// A Reticulum identity verified by the mesh transport.
    Mesh,
    /// A local operator on the same host (Unix socket, terminal).
    Local,
}

impl AuthSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::Session => "session",
            Self::MutualTls => "mutual_tls",
            Self::Mesh => "mesh",
            Self::Local => "local",
        }
    }
}

/// Why a principal could not be normalized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrincipalError {
    MissingSubject,
    MissingIssuer,
    IdentifierTooLong { field: &'static str, len: usize, max: usize },
    TooManyClaims { count: usize, max: usize },
    ClaimTooLong { name: String, len: usize, max: usize },
}

impl fmt::Display for PrincipalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubject => f.write_str("principal has no subject"),
            Self::MissingIssuer => f.write_str("principal has no issuer"),
            Self::IdentifierTooLong { field, len, max } => {
                write!(f, "principal {field} has {len} bytes; the limit is {max}")
            }
            Self::TooManyClaims { count, max } => {
                write!(f, "principal carries {count} claims; the limit is {max}")
            }
            Self::ClaimTooLong { name, len, max } => {
                write!(f, "claim {name} has {len} bytes; the limit is {max}")
            }
        }
    }
}

impl std::error::Error for PrincipalError {}

/// An authenticated identity. Construct one only after authentication
/// succeeded; the policy never authenticates anything itself.
#[derive(Clone, PartialEq, Eq)]
pub struct Principal {
    subject: String,
    issuer: String,
    display_name: Option<String>,
    roles: Vec<String>,
    groups: Vec<String>,
    session_id: Option<String>,
    client_id: Option<String>,
    auth_source: AuthSource,
    claims: BTreeMap<String, String>,
}

impl Principal {
    /// Start a principal. Roles, groups, and claims are added with the
    /// builder methods and bounded by [`Principal::normalize`].
    pub fn new(
        issuer: impl Into<String>,
        subject: impl Into<String>,
        auth_source: AuthSource,
    ) -> Self {
        Self {
            subject: subject.into(),
            issuer: issuer.into(),
            display_name: None,
            roles: Vec::new(),
            groups: Vec::new(),
            session_id: None,
            client_id: None,
            auth_source,
            claims: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn with_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.roles.extend(roles.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_groups<I, S>(mut self, groups: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.groups.extend(groups.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    #[must_use]
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Attach a bounded claim. Claims never appear in summaries, decisions,
    /// or audit output; grants may constrain on them by name.
    #[must_use]
    pub fn with_claim(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.claims.insert(name.into(), value.into());
        self
    }

    /// Validate identifiers and bounds, lowercase roles and groups, and
    /// drop duplicates. Returns the normalized principal.
    pub fn normalize(mut self, limits: &Limits) -> Result<Self, PrincipalError> {
        self.subject = self.subject.trim().to_string();
        self.issuer = self.issuer.trim().to_string();
        if self.subject.is_empty() {
            return Err(PrincipalError::MissingSubject);
        }
        if self.issuer.is_empty() {
            return Err(PrincipalError::MissingIssuer);
        }
        let check = |field: &'static str, value: &str| {
            if value.len() > limits.max_identifier_len {
                Err(PrincipalError::IdentifierTooLong {
                    field,
                    len: value.len(),
                    max: limits.max_identifier_len,
                })
            } else {
                Ok(())
            }
        };
        check("subject", &self.subject)?;
        check("issuer", &self.issuer)?;
        if let Some(name) = &self.display_name {
            check("display_name", name)?;
        }
        if let Some(session) = &self.session_id {
            check("session_id", session)?;
        }
        if let Some(client) = &self.client_id {
            check("client_id", client)?;
        }
        for role in &self.roles {
            check("role", role)?;
        }
        for group in &self.groups {
            check("group", group)?;
        }
        if self.claims.len() > limits.max_claims {
            return Err(PrincipalError::TooManyClaims {
                count: self.claims.len(),
                max: limits.max_claims,
            });
        }
        for (name, value) in &self.claims {
            if name.len() > limits.max_claim_len || value.len() > limits.max_claim_len {
                return Err(PrincipalError::ClaimTooLong {
                    name: name.clone(),
                    len: name.len().max(value.len()),
                    max: limits.max_claim_len,
                });
            }
        }
        self.roles = dedup_lower(&self.roles);
        self.groups = dedup_lower(&self.groups);
        Ok(self)
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    #[must_use]
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    #[must_use]
    pub fn auth_source(&self) -> AuthSource {
        self.auth_source
    }

    /// A claim value by name, for constraint evaluation only.
    #[must_use]
    pub fn claim(&self, name: &str) -> Option<&str> {
        self.claims.get(name).map(String::as_str)
    }

    #[must_use]
    pub fn claim_count(&self) -> usize {
        self.claims.len()
    }

    /// The audit-safe projection: no display name, no claims.
    #[must_use]
    pub fn summary(&self) -> PrincipalSummary {
        PrincipalSummary {
            subject: self.subject.clone(),
            issuer: self.issuer.clone(),
            roles: self.roles.clone(),
            groups: self.groups.clone(),
            session_id: self.session_id.clone(),
            client_id: self.client_id.clone(),
            auth_source: self.auth_source,
        }
    }
}

/// Debug output never prints claim values.
impl fmt::Debug for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Principal")
            .field("subject", &self.subject)
            .field("issuer", &self.issuer)
            .field("roles", &self.roles)
            .field("groups", &self.groups)
            .field("session_id", &self.session_id)
            .field("client_id", &self.client_id)
            .field("auth_source", &self.auth_source)
            .field("claims", &format_args!("<{} redacted>", self.claims.len()))
            .finish()
    }
}

fn dedup_lower(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        let lowered = value.trim().to_ascii_lowercase();
        if !lowered.is_empty() && !out.contains(&lowered) {
            out.push(lowered);
        }
    }
    out
}

/// What a decision and an audit record say about who asked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct PrincipalSummary {
    pub subject: String,
    pub issuer: String,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub session_id: Option<String>,
    pub client_id: Option<String>,
    pub auth_source: AuthSource,
}
