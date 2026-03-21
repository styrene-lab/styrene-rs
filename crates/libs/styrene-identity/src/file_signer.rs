//! Tier D: EncryptedFile signer — argon2id + ChaCha20Poly1305.
//!
//! Default signer for desktop/server deployments. Stores the root secret
//! in an encrypted file at `~/.styrene/identity.key` (or configurable path).
//!
//! File format:
//! ```text
//! [salt:32][nonce:12][ciphertext:32+16]
//! ```
//! - salt: random 32 bytes for argon2id
//! - nonce: random 12 bytes for ChaCha20Poly1305
//! - ciphertext: encrypted 32-byte root secret + 16-byte auth tag

use std::path::{Path, PathBuf};

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroize;

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SECRET_LEN: usize = 32;
const TAG_LEN: usize = 16;
const FILE_LEN: usize = SALT_LEN + NONCE_LEN + SECRET_LEN + TAG_LEN;

/// Tier D file-based identity signer.
pub struct FileSigner {
    path: PathBuf,
    label: String,
}

impl FileSigner {
    /// Create a file signer for the given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = format!("file:{}", path.display());
        Self { path, label }
    }

    /// Default identity file path: `~/.config/styrene/identity.key`.
    ///
    /// Uses $HOME directly (consistent with styrened-rs config module).
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        home.join(".config").join("styrene").join("identity.key")
    }

    /// Create with the default path.
    pub fn with_default_path() -> Self {
        Self::new(Self::default_path())
    }

    /// Generate a new random root secret and save it encrypted with the passphrase.
    pub fn generate(&self, passphrase: &[u8]) -> Result<(), SignerError> {
        let mut root_secret = [0u8; SECRET_LEN];
        OsRng.fill_bytes(&mut root_secret);

        let result = self.save(&root_secret, passphrase);
        root_secret.zeroize();
        result
    }

    /// Save a root secret encrypted with the given passphrase.
    fn save(&self, root_secret: &[u8; SECRET_LEN], passphrase: &[u8]) -> Result<(), SignerError> {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        // Derive encryption key from passphrase via argon2id
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(passphrase, &salt, &mut key)
            .map_err(|e| SignerError::SigningFailed(format!("argon2id: {e}")))?;

        // Encrypt root secret
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| SignerError::SigningFailed(format!("cipher init: {e}")))?;
        key.zeroize();

        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, root_secret.as_ref())
            .map_err(|e| SignerError::SigningFailed(format!("encrypt: {e}")))?;

        // Write file
        let mut file_data = Vec::with_capacity(FILE_LEN);
        file_data.extend_from_slice(&salt);
        file_data.extend_from_slice(&nonce_bytes);
        file_data.extend_from_slice(&ciphertext);

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, &file_data)?;

        // Set permissions to owner-only on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Load and decrypt the root secret.
    fn load(&self, passphrase: &[u8]) -> Result<RootSecret, SignerError> {
        let file_data = std::fs::read(&self.path)?;
        if file_data.len() != FILE_LEN {
            return Err(SignerError::DecryptionFailed(format!(
                "invalid file size: {} (expected {FILE_LEN})",
                file_data.len()
            )));
        }

        let salt = &file_data[..SALT_LEN];
        let nonce_bytes = &file_data[SALT_LEN..SALT_LEN + NONCE_LEN];
        let ciphertext = &file_data[SALT_LEN + NONCE_LEN..];

        // Derive key from passphrase
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(passphrase, salt, &mut key)
            .map_err(|e| SignerError::DecryptionFailed(format!("argon2id: {e}")))?;

        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| SignerError::DecryptionFailed(format!("cipher init: {e}")))?;
        key.zeroize();

        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| SignerError::DecryptionFailed("wrong passphrase or corrupted file".into()))?;

        let mut secret = [0u8; SECRET_LEN];
        secret.copy_from_slice(&plaintext);

        Ok(RootSecret::new(secret))
    }

    /// Check if the identity file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Path to the identity file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait::async_trait]
impl IdentitySigner for FileSigner {
    fn tier(&self) -> SignerTier {
        SignerTier::EncryptedFile
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn is_available(&self) -> bool {
        self.path.exists()
    }

    async fn sign(&self, _data: &[u8]) -> Result<Vec<u8>, SignerError> {
        // Ed25519 signing requires ed25519-dalek — not yet wired.
        // When added, this will: derive signing key via HKDF, then sign.
        Err(SignerError::Unavailable(
            "Ed25519 signing not yet implemented (needs ed25519-dalek)".into(),
        ))
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        let passphrase = std::env::var("STYRENE_PASSPHRASE")
            .map(|s| s.into_bytes())
            .map_err(|_| {
                SignerError::AuthRequired(
                    "STYRENE_PASSPHRASE environment variable not set — \
                     required to decrypt identity file"
                        .into(),
                )
            })?;
        self.load(&passphrase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_signer() -> (FileSigner, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let signer = FileSigner::new(tmp.path());
        (signer, tmp)
    }

    #[test]
    fn generate_and_load_roundtrip() {
        let (signer, _tmp) = temp_signer();
        let passphrase = b"test-passphrase-123";

        signer.generate(passphrase).unwrap();
        assert!(signer.exists());

        let secret = signer.load(passphrase).unwrap();
        assert_ne!(secret.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let (signer, _tmp) = temp_signer();
        signer.generate(b"correct").unwrap();

        let result = signer.load(b"wrong");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("wrong passphrase"),
        );
    }

    #[test]
    fn deterministic_key_from_same_file() {
        let (signer, _tmp) = temp_signer();
        let passphrase = b"deterministic";

        signer.generate(passphrase).unwrap();

        let s1 = signer.load(passphrase).unwrap();
        let s2 = signer.load(passphrase).unwrap();
        assert_eq!(s1.as_bytes(), s2.as_bytes());
    }

    #[test]
    fn derived_keys_from_file_signer() {
        let (signer, _tmp) = temp_signer();
        let passphrase = b"derive-test";

        signer.generate(passphrase).unwrap();
        let secret = signer.load(passphrase).unwrap();
        let keys = crate::derive::derive_keys(secret.as_bytes());

        // All keys should be non-zero and distinct
        assert_ne!(keys.rns_encryption, [0u8; 32]);
        assert_ne!(keys.rns_signing, [0u8; 32]);
        assert_ne!(keys.rns_encryption, keys.rns_signing);
    }

    #[test]
    fn tier_is_encrypted_file() {
        let (signer, _tmp) = temp_signer();
        assert_eq!(signer.tier(), SignerTier::EncryptedFile);
    }

    #[test]
    fn not_available_before_generate() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().join("nonexistent.key");
        let signer = FileSigner::new(path);
        assert!(!signer.is_available());
    }
}
