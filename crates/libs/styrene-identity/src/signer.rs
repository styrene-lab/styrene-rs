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

/// Ordered chain of signers — tries each in tier order (A→B→C→D) until
/// one succeeds. This is the automatic fallback mechanism described in the spec.
///
/// ```text
/// SignerChain [YubiKeySigner, FileSigner]
///   1. Try YubiKeySigner.is_available() → false (no YubiKey plugged in)
///   2. Try FileSigner.is_available() → true
///   3. Use FileSigner
/// ```
pub struct SignerChain {
    signers: Vec<Box<dyn IdentitySigner>>,
}

impl SignerChain {
    /// Create a signer chain from a list of signers. They will be tried in
    /// the given order — callers should sort by tier (highest security first).
    pub fn new(signers: Vec<Box<dyn IdentitySigner>>) -> Self {
        Self { signers }
    }

    /// Create a signer chain sorted by tier (A before D).
    pub fn new_sorted(mut signers: Vec<Box<dyn IdentitySigner>>) -> Self {
        signers.sort_by_key(|s| s.tier());
        Self { signers }
    }

    /// Find the first available signer, or None.
    pub fn available(&self) -> Option<&dyn IdentitySigner> {
        self.signers.iter().find(|s| s.is_available()).map(|s| s.as_ref())
    }

    /// List all signers with their availability status.
    pub fn status(&self) -> Vec<(&str, SignerTier, bool)> {
        self.signers
            .iter()
            .map(|s| (s.label(), s.tier(), s.is_available()))
            .collect()
    }
}

#[async_trait::async_trait]
impl IdentitySigner for SignerChain {
    fn tier(&self) -> SignerTier {
        self.available().map(|s| s.tier()).unwrap_or(SignerTier::EncryptedFile)
    }

    fn label(&self) -> &str {
        self.available().map(|s| s.label()).unwrap_or("(no signer available)")
    }

    fn is_available(&self) -> bool {
        self.signers.iter().any(|s| s.is_available())
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        for signer in &self.signers {
            if signer.is_available() {
                return signer.root_secret().await;
            }
        }
        Err(SignerError::Unavailable("no signer available in chain".into()))
    }

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        for signer in &self.signers {
            if signer.is_available() {
                return signer.sign(data).await;
            }
        }
        Err(SignerError::Unavailable("no signer available in chain".into()))
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

    // ── SignerChain tests ───────────────────────────────────────────────────

    /// A mock signer for testing the chain.
    struct MockSigner {
        tier: SignerTier,
        name: &'static str,
        available: bool,
    }

    #[async_trait::async_trait]
    impl IdentitySigner for MockSigner {
        fn tier(&self) -> SignerTier {
            self.tier
        }
        fn label(&self) -> &str {
            self.name
        }
        fn is_available(&self) -> bool {
            self.available
        }
        async fn root_secret(&self) -> Result<RootSecret, SignerError> {
            if self.available {
                Ok(RootSecret::new([self.tier as u8; 32]))
            } else {
                Err(SignerError::Unavailable(self.name.into()))
            }
        }
        async fn sign(&self, _data: &[u8]) -> Result<Vec<u8>, SignerError> {
            if self.available {
                Ok(vec![self.tier as u8; 64])
            } else {
                Err(SignerError::Unavailable(self.name.into()))
            }
        }
    }

    #[test]
    fn chain_selects_first_available() {
        let chain = SignerChain::new(vec![
            Box::new(MockSigner {
                tier: SignerTier::HardwareHsm,
                name: "yubikey",
                available: false,
            }),
            Box::new(MockSigner {
                tier: SignerTier::EncryptedFile,
                name: "file",
                available: true,
            }),
        ]);
        assert!(chain.is_available());
        assert_eq!(chain.label(), "file");
        assert_eq!(chain.tier(), SignerTier::EncryptedFile);
    }

    #[test]
    fn chain_prefers_higher_tier() {
        let chain = SignerChain::new(vec![
            Box::new(MockSigner {
                tier: SignerTier::HardwareHsm,
                name: "yubikey",
                available: true,
            }),
            Box::new(MockSigner {
                tier: SignerTier::EncryptedFile,
                name: "file",
                available: true,
            }),
        ]);
        assert_eq!(chain.label(), "yubikey");
        assert_eq!(chain.tier(), SignerTier::HardwareHsm);
    }

    #[test]
    fn chain_empty_is_unavailable() {
        let chain = SignerChain::new(vec![]);
        assert!(!chain.is_available());
    }

    #[test]
    fn chain_all_unavailable() {
        let chain = SignerChain::new(vec![
            Box::new(MockSigner {
                tier: SignerTier::HardwareHsm,
                name: "yubikey",
                available: false,
            }),
            Box::new(MockSigner {
                tier: SignerTier::EncryptedFile,
                name: "file",
                available: false,
            }),
        ]);
        assert!(!chain.is_available());
        assert_eq!(chain.label(), "(no signer available)");
    }

    #[tokio::test]
    async fn chain_sign_uses_first_available() {
        let chain = SignerChain::new(vec![
            Box::new(MockSigner {
                tier: SignerTier::HardwareHsm,
                name: "yubikey",
                available: false,
            }),
            Box::new(MockSigner {
                tier: SignerTier::EncryptedFile,
                name: "file",
                available: true,
            }),
        ]);
        let sig = chain.sign(b"test").await.unwrap();
        assert_eq!(sig[0], SignerTier::EncryptedFile as u8);
    }

    #[tokio::test]
    async fn chain_sign_fails_when_none_available() {
        let chain = SignerChain::new(vec![Box::new(MockSigner {
            tier: SignerTier::HardwareHsm,
            name: "yubikey",
            available: false,
        })]);
        assert!(chain.sign(b"test").await.is_err());
    }

    #[test]
    fn chain_status_reports_all() {
        let chain = SignerChain::new(vec![
            Box::new(MockSigner {
                tier: SignerTier::HardwareHsm,
                name: "yubikey",
                available: false,
            }),
            Box::new(MockSigner {
                tier: SignerTier::EncryptedFile,
                name: "file",
                available: true,
            }),
        ]);
        let status = chain.status();
        assert_eq!(status.len(), 2);
        assert_eq!(status[0], ("yubikey", SignerTier::HardwareHsm, false));
        assert_eq!(status[1], ("file", SignerTier::EncryptedFile, true));
    }
}
