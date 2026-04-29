//! Forge API types — the shared contract for forge backends.
//!
//! This crate defines the `ForgeClient` trait, typed `ForgeError`, and all
//! associated data types for interacting with git forges (Forgejo, GitHub,
//! GitLab). Minimal dependencies so it can be consumed by codex, omegon,
//! scribe, and other ecosystem crates without heavy transitive deps.
//!
//! Concrete client implementations (ForgejoForgeClient, GitHubForgeClient)
//! live in the `scribe` crate, not here.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Errors ──────────────────────────────────────────────────────────────────

/// Typed error for forge operations.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("rate limited — resets at {reset_at}")]
    RateLimited { reset_at: String },

    #[error("validation error: {0}")]
    Validation(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("forge API error ({status}): {message}")]
    Api { status: u16, message: String },
}

impl ForgeError {
    /// Construct from an HTTP status code and body.
    pub fn from_status(status: u16, body: String) -> Self {
        match status {
            401 => Self::Unauthorized(body),
            403 => Self::Forbidden(body),
            404 => Self::NotFound(body),
            422 => Self::Validation(body),
            429 => Self::RateLimited {
                reset_at: "unknown".into(),
            },
            _ => Self::Api {
                status,
                message: body,
            },
        }
    }
}

pub type ForgeResult<T> = Result<T, ForgeError>;

// ── Forge identity ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForgeKind {
    #[serde(rename = "forgejo")]
    Forgejo,
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "gitlab")]
    GitLab,
}

impl ForgeKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Forgejo => "Forgejo",
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
        }
    }
}

/// Connection to a forge instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEndpoint {
    pub id: String,
    pub kind: ForgeKind,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_secret: Option<String>,
}

// ── Forge data types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: IssueState,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub assignees: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIssue {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub assignees: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateIssue {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<IssueState>,
    pub labels: Option<Vec<String>>,
    pub milestone: Option<String>,
    pub assignees: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeRepo {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub default_branch: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub html_url: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRepo {
    pub name: String,
    pub description: String,
    pub private: bool,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeLabel {
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeMilestone {
    pub title: String,
    pub description: Option<String>,
    pub state: IssueState,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeWebhook {
    pub id: u64,
    pub url: String,
    pub events: Vec<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWebhook {
    pub url: String,
    pub events: Vec<String>,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListOpts {
    pub state: Option<IssueState>,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

// ── Forge client trait ──────────────────────────────────────────────────────

/// Pluggable forge API client.
#[async_trait]
pub trait ForgeClient: Send + Sync {
    fn kind(&self) -> ForgeKind;
    fn endpoint(&self) -> &ForgeEndpoint;

    async fn list_issues(&self, org: &str, repo: &str, opts: &ListOpts)
        -> ForgeResult<Vec<ForgeIssue>>;
    async fn get_issue(&self, org: &str, repo: &str, number: u64) -> ForgeResult<ForgeIssue>;
    async fn create_issue(&self, org: &str, repo: &str, issue: &CreateIssue)
        -> ForgeResult<ForgeIssue>;
    async fn update_issue(
        &self, org: &str, repo: &str, number: u64, update: &UpdateIssue,
    ) -> ForgeResult<ForgeIssue>;

    async fn list_labels(&self, org: &str, repo: &str) -> ForgeResult<Vec<ForgeLabel>>;
    async fn list_milestones(&self, org: &str, repo: &str) -> ForgeResult<Vec<ForgeMilestone>>;

    async fn list_repos(&self, org: &str) -> ForgeResult<Vec<ForgeRepo>>;
    async fn create_repo(&self, org: &str, repo: &CreateRepo) -> ForgeResult<ForgeRepo>;

    async fn create_webhook(
        &self, org: &str, repo: &str, hook: &CreateWebhook,
    ) -> ForgeResult<ForgeWebhook>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_kind_serde_roundtrip() {
        for kind in [ForgeKind::Forgejo, ForgeKind::GitHub, ForgeKind::GitLab] {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: ForgeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn issue_state_serde_roundtrip() {
        let json = serde_json::to_string(&IssueState::Open).unwrap();
        assert_eq!(json, "\"open\"");
        let parsed: IssueState = serde_json::from_str("\"closed\"").unwrap();
        assert_eq!(parsed, IssueState::Closed);
    }

    #[test]
    fn forge_endpoint_serde() {
        let endpoint = ForgeEndpoint {
            id: "local".into(),
            kind: ForgeKind::Forgejo,
            base_url: "http://localhost:3000".into(),
            token_secret: Some("FORGEJO_TOKEN".into()),
        };
        let json = serde_json::to_string(&endpoint).unwrap();
        let parsed: ForgeEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "local");
        assert_eq!(parsed.kind, ForgeKind::Forgejo);
    }

    #[test]
    fn forge_endpoint_optional_token() {
        let json = r#"{"id":"pub","kind":"github","base_url":"https://api.github.com"}"#;
        let parsed: ForgeEndpoint = serde_json::from_str(json).unwrap();
        assert!(parsed.token_secret.is_none());
    }

    #[test]
    fn forge_error_from_status() {
        assert!(matches!(ForgeError::from_status(401, "x".into()), ForgeError::Unauthorized(_)));
        assert!(matches!(ForgeError::from_status(403, "x".into()), ForgeError::Forbidden(_)));
        assert!(matches!(ForgeError::from_status(404, "x".into()), ForgeError::NotFound(_)));
        assert!(matches!(ForgeError::from_status(422, "x".into()), ForgeError::Validation(_)));
        assert!(matches!(ForgeError::from_status(429, "x".into()), ForgeError::RateLimited { .. }));
        assert!(matches!(ForgeError::from_status(500, "x".into()), ForgeError::Api { status: 500, .. }));
    }

    #[test]
    fn update_issue_default_is_empty() {
        let update = UpdateIssue::default();
        assert!(update.title.is_none());
        assert!(update.state.is_none());
        assert!(update.labels.is_none());
    }
}
