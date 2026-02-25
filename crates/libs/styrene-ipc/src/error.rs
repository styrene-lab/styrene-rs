use serde::{Deserialize, Serialize};

/// Errors returned by daemon IPC operations.
///
/// `NotImplemented` is the critical variant for stub-first development — every
/// method starts as a stub returning this, then gets replaced with real logic.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum IpcError {
    #[error("not implemented: {method}")]
    NotImplemented { method: String },

    #[error("unavailable: {reason}")]
    Unavailable { reason: String },

    #[error("timeout: {operation}")]
    Timeout { operation: String },

    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("not found: {resource}")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("internal error: {message}")]
    Internal { message: String },

    #[error("transport error: {message}")]
    Transport { message: String },
}

impl IpcError {
    /// Returns `true` for transient errors that may succeed on retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable { .. } | Self::Timeout { .. } | Self::Transport { .. }
        )
    }

    /// Convenience constructor for `NotImplemented`.
    pub fn not_implemented(method: impl Into<String>) -> Self {
        Self::NotImplemented {
            method: method.into(),
        }
    }
}
