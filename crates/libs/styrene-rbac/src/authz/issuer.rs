//! Framework-neutral trusted issuer extraction for proxy-provided identity.
//!
//! The extractor never authenticates a request. It only turns identity
//! headers into a [`Principal`] after the caller proved authentication
//! succeeded and the named issuer is configured as trusted.

use std::collections::BTreeMap;
use std::fmt;

use super::decision::DecisionReason;
use super::limits::Limits;
use super::principal::{AuthSource, Principal, PrincipalError};

/// What the transport established before consulting identity headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticationState {
    Unauthenticated,
    Authenticated(AuthSource),
}

/// How one trusted issuer's headers map onto policy roles and claims.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct IssuerMapping {
    /// Header role value, lowercased, to policy role name. Unmapped roles
    /// are rejected; nothing is inferred.
    #[cfg_attr(feature = "config", serde(default))]
    pub roles: BTreeMap<String, String>,
    /// Header suffixes whose values are kept as bounded claims, such as
    /// `Workspace` for `<prefix>Workspace`.
    #[cfg_attr(feature = "config", serde(default))]
    pub retained_claims: Vec<String>,
}

/// Trusted issuers and the configurable header prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "config", derive(serde::Serialize, serde::Deserialize))]
pub struct TrustedIssuerConfig {
    /// Header name prefix, matched case-insensitively.
    pub header_prefix: String,
    pub issuers: BTreeMap<String, IssuerMapping>,
}

impl Default for TrustedIssuerConfig {
    fn default() -> Self {
        Self { header_prefix: "Styrene-Principal-".into(), issuers: BTreeMap::new() }
    }
}

impl TrustedIssuerConfig {
    #[must_use]
    pub fn new(header_prefix: impl Into<String>) -> Self {
        Self { header_prefix: header_prefix.into(), issuers: BTreeMap::new() }
    }

    #[must_use]
    pub fn trust(mut self, issuer: impl Into<String>, mapping: IssuerMapping) -> Self {
        self.issuers.insert(issuer.into(), mapping);
        self
    }
}

/// Why identity headers did not become a principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssuerError {
    MissingAuthentication,
    MissingIssuer,
    UntrustedIssuer {
        issuer: String,
    },
    /// The same header appeared with different values.
    ConflictingHeader {
        header: String,
    },
    MissingSubject,
    MissingRole,
    UnmappedRole {
        role: String,
    },
    Principal(PrincipalError),
}

impl IssuerError {
    /// The decision reason a consumer should report for this failure.
    #[must_use]
    pub const fn reason(&self) -> DecisionReason {
        match self {
            Self::MissingAuthentication => DecisionReason::MissingAuthentication,
            Self::MissingIssuer | Self::UntrustedIssuer { .. } | Self::ConflictingHeader { .. } => {
                DecisionReason::UntrustedIssuer
            }
            Self::MissingSubject | Self::Principal(_) => DecisionReason::MissingClaim,
            Self::MissingRole | Self::UnmappedRole { .. } => DecisionReason::UnknownRole,
        }
    }
}

impl fmt::Display for IssuerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAuthentication => f.write_str("request is not authenticated"),
            Self::MissingIssuer => f.write_str("identity headers name no issuer"),
            Self::UntrustedIssuer { issuer } => write!(f, "issuer {issuer:?} is not trusted"),
            Self::ConflictingHeader { header } => {
                write!(f, "header {header} appears with conflicting values")
            }
            Self::MissingSubject => f.write_str("identity headers name no subject"),
            Self::MissingRole => f.write_str("identity headers name no role"),
            Self::UnmappedRole { role } => {
                write!(f, "role {role:?} has no mapping for this issuer")
            }
            Self::Principal(error) => write!(f, "principal is invalid: {error}"),
        }
    }
}

impl std::error::Error for IssuerError {}

/// Builds principals from identity headers under a trusted issuer policy.
#[derive(Clone, Debug)]
pub struct PrincipalExtractor {
    config: TrustedIssuerConfig,
    limits: Limits,
}

impl PrincipalExtractor {
    #[must_use]
    pub fn new(config: TrustedIssuerConfig, limits: Limits) -> Self {
        Self { config, limits }
    }

    #[must_use]
    pub fn config(&self) -> &TrustedIssuerConfig {
        &self.config
    }

    /// Extract a principal from `(name, value)` headers. Names match the
    /// configured prefix case-insensitively; anything else is ignored.
    /// Repeated headers must agree. Only configured claims are retained.
    pub fn extract<'h, I>(
        &self,
        authentication: AuthenticationState,
        headers: I,
    ) -> Result<Principal, IssuerError>
    where
        I: IntoIterator<Item = (&'h str, &'h str)>,
    {
        let source = match authentication {
            AuthenticationState::Unauthenticated => return Err(IssuerError::MissingAuthentication),
            AuthenticationState::Authenticated(source) => source,
        };
        let prefix = self.config.header_prefix.to_ascii_lowercase();
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in headers {
            let lowered = name.to_ascii_lowercase();
            let Some(suffix) = lowered.strip_prefix(prefix.as_str()) else { continue };
            let suffix = suffix.to_ascii_lowercase();
            match fields.get(&suffix) {
                Some(existing) if existing != value => {
                    return Err(IssuerError::ConflictingHeader {
                        header: format!("{}{suffix}", self.config.header_prefix),
                    });
                }
                Some(_) => {}
                None => {
                    fields.insert(suffix, value.to_string());
                }
            }
        }
        let issuer = fields
            .get("issuer")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or(IssuerError::MissingIssuer)?;
        let mapping = self
            .config
            .issuers
            .get(issuer)
            .ok_or_else(|| IssuerError::UntrustedIssuer { issuer: issuer.to_string() })?;
        let subject = fields
            .get("subject")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or(IssuerError::MissingSubject)?;
        let role_header = fields
            .get("role")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or(IssuerError::MissingRole)?;
        let mut roles = Vec::new();
        for role in role_header.split(',').map(str::trim).filter(|r| !r.is_empty()) {
            let mapped = mapping
                .roles
                .get(&role.to_ascii_lowercase())
                .ok_or_else(|| IssuerError::UnmappedRole { role: role.to_string() })?;
            roles.push(mapped.clone());
        }
        let mut principal = Principal::new(issuer, subject, source).with_roles(roles);
        if let Some(name) = fields.get("display-name") {
            principal = principal.with_display_name(name.clone());
        }
        if let Some(session) = fields.get("session-id") {
            principal = principal.with_session_id(session.clone());
        }
        if let Some(client) = fields.get("client-id") {
            principal = principal.with_client_id(client.clone());
        }
        if let Some(groups) = fields.get("groups") {
            principal = principal.with_groups(
                groups.split(',').map(str::trim).filter(|g| !g.is_empty()).map(str::to_owned),
            );
        }
        for claim in &mapping.retained_claims {
            if let Some(value) = fields.get(&claim.to_ascii_lowercase()) {
                principal = principal.with_claim(claim.to_ascii_lowercase(), value.clone());
            }
        }
        principal.normalize(&self.limits).map_err(IssuerError::Principal)
    }
}
