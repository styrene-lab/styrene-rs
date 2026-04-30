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

    #[error("tunnel not found: {0}")]
    NotFound(String),

    #[error("tunnel configuration error: {0}")]
    Config(String),

    #[error("tunnel negotiation timed out")]
    Timeout,

    #[error("duplicate nonce: {0}")]
    NonceDuplicate(String),

    #[error("endpoint unreachable: {0}")]
    EndpointUnreachable(String),

    #[error("tunnel offer rejected: {0}")]
    Rejected(String),

    #[error("CBOR error: {0}")]
    Cbor(String),
}
