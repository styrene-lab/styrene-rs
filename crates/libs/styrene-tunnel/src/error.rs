//! Error types for tunnel operations.

/// Errors from tunnel and PQC session operations.
#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("PQC crypto error: {0}")]
    Crypto(String),

    #[error("invalid session state: expected {expected}, got {actual}")]
    InvalidState { expected: &'static str, actual: String },

    #[error("session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("replay detected: sequence {sequence} already seen")]
    ReplayDetected { sequence: u64 },

    #[error("ratchet step out of order: expected > {expected}, got {actual}")]
    RatchetOutOfOrder { expected: u64, actual: u64 },

    #[error("session expired")]
    SessionExpired,

    #[error("key material invalid: {0}")]
    InvalidKeyMaterial(String),

    #[error("tunnel backend error: {0}")]
    Backend(String),

    #[error("VICI protocol error: {0}")]
    #[cfg(feature = "strongswan")]
    Vici(String),

    #[error("WireGuard error: {0}")]
    #[cfg(feature = "wireguard")]
    WireGuard(String),

    #[error("no suitable tunnel backend for peer capabilities: 0x{capabilities:02x}")]
    NoSuitableBackend { capabilities: u8 },

    #[error("tunnel not established")]
    NotEstablished,

    #[error("msgpack error: {0}")]
    Msgpack(String),
}

impl From<rmp_serde::decode::Error> for TunnelError {
    fn from(e: rmp_serde::decode::Error) -> Self {
        Self::Msgpack(e.to_string())
    }
}

impl From<rmp_serde::encode::Error> for TunnelError {
    fn from(e: rmp_serde::encode::Error) -> Self {
        Self::Msgpack(e.to_string())
    }
}
