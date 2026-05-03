//! Error types for secret resolution and store operations.

/// Error resolving a secret.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The requested secret was not found in any configured source.
    #[error(
        "secret '{key}' not found — set environment variable STYRENE_SECRET_{env_key}, \
         or store it with: styrene-secrets set {key}"
    )]
    NotFound { key: String, env_key: String },

    /// The store backend returned an error.
    #[cfg(feature = "file-store")]
    #[error("store error resolving '{key}': {source}")]
    Store { key: String, source: StoreError },
}

/// Error from the encrypted secret store.
#[cfg(feature = "file-store")]
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Database error.
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),

    /// Encryption or decryption failed.
    #[error("{0}")]
    Crypto(String),

    /// Wrong passphrase (HMAC verification failed).
    #[error("wrong passphrase or corrupted store")]
    BadPassphrase,

    /// I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
