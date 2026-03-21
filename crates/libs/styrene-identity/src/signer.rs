//! IdentitySigner trait — abstract signing interface across hardware tiers.

use zeroize::Zeroize;

/// Signer implementation tier — indicates trust level and key storage model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SignerTier {
    /// Hardware HSM — non-exportable keys (YubiKey PIV/FIDO2).
    HardwareHsm,
    /// Device HSM — platform secure element (iOS Secure Enclave, Android StrongBox).
    DeviceHsm,
    /// Credential manager — software key store (Bitwarden, 1Password SSH items).
    CredentialManager,
    /// Encrypted file — argon2id + ChaCha20Poly1305 on disk (default).
    EncryptedFile,
}

/// Errors from signer operations.
#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("signer not available: {0}")]
    Unavailable(String),

    #[error("authentication required: {0}")]
    AuthRequired(String),

    #[error("key not found: {0}")]
    KeyNotFound(String),

    #[error("signing failed: {0}")]
    SigningFailed(String),

    #[error("decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Abstract identity signer — implementations wrap hardware or software key stores.
///
/// The signer provides the 32-byte root secret that feeds the HKDF derivation
/// hierarchy. Higher-tier signers (A, B) never expose the raw secret — they
/// perform derivation internally. Lower-tier signers (C, D) yield the secret
/// for the caller to derive keys.
///
/// All implementations must be `Send + Sync` for use in async daemon context.
#[async_trait::async_trait]
pub trait IdentitySigner: Send + Sync {
    /// Which tier this signer implements.
    fn tier(&self) -> SignerTier;

    /// Human-readable label (e.g., "YubiKey 5C #12345", "Keychain", "~/.styrene/identity").
    fn label(&self) -> &str;

    /// Whether the signer is currently unlocked and ready to sign.
    fn is_available(&self) -> bool;

    /// Get the 32-byte root secret for HKDF derivation.
    ///
    /// For Tier A/B, this may require user interaction (NFC tap, biometric).
    /// For Tier C/D, this reads from the key store.
    ///
    /// The returned secret is zeroized on drop.
    async fn root_secret(&self) -> Result<RootSecret, SignerError>;

    /// Sign arbitrary data with the identity's Ed25519 key.
    ///
    /// Implementations must derive or use the signing key appropriate to their tier.
    /// Hardware signers (Tier A/B) use on-device signing.
    /// Software signers (Tier C/D) derive via HKDF then sign with ed25519-dalek.
    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError>;
}

/// A 32-byte root secret that zeroizes on drop.
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct RootSecret {
    bytes: [u8; 32],
}

impl RootSecret {
    /// Create from raw bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Access the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for RootSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RootSecret([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_secret_zeroizes_debug() {
        let secret = RootSecret::new([42u8; 32]);
        let debug = format!("{:?}", secret);
        assert_eq!(debug, "RootSecret([REDACTED])");
        assert_eq!(secret.as_bytes(), &[42u8; 32]);
    }

    #[test]
    fn signer_tier_ordering() {
        assert!(SignerTier::HardwareHsm < SignerTier::DeviceHsm);
        assert!(SignerTier::DeviceHsm < SignerTier::CredentialManager);
        assert!(SignerTier::CredentialManager < SignerTier::EncryptedFile);
    }
}
