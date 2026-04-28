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

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroize;

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

/// File format magic bytes: "STID" (Styrene Identity).
const MAGIC: &[u8; 4] = b"STID";
/// File format version. Increment when the format changes.
const FORMAT_VERSION: u8 = 1;
/// Header: magic(4) + version(1) = 5 bytes.
const HEADER_LEN: usize = MAGIC.len() + 1;

const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SECRET_LEN: usize = 32;
const TAG_LEN: usize = 16;
/// Expected file size: header(5) + salt(32) + nonce(12) + ciphertext(32+16) = 97 bytes.
pub const FILE_LEN: usize = HEADER_LEN + SALT_LEN + NONCE_LEN + SECRET_LEN + TAG_LEN;
/// Legacy file size (v0 format without header).
const LEGACY_FILE_LEN: usize = SALT_LEN + NONCE_LEN + SECRET_LEN + TAG_LEN;

/// Hardened Argon2id parameters.
/// m=65536 KiB (64 MiB), t=3 iterations, p=1 parallelism.
/// Exceeds OWASP minimum recommendation (m=47104, t=1, p=1).
fn argon2_params() -> Argon2<'static> {
    let params = Params::new(65536, 3, 1, Some(32)).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Tier D file-based identity signer.
///
/// Requires a passphrase to decrypt the identity file. The passphrase is
/// provided via a [`PassphraseProvider`] — never read from environment
/// variables, which are visible to co-tenant processes.
pub struct FileSigner {
    path: PathBuf,
    label: String,
    passphrase_provider: Box<dyn PassphraseProvider>,
}

/// Provides the passphrase for decrypting the identity file.
///
/// Implementations should obtain the passphrase securely — e.g., from a
/// platform keychain, interactive prompt, or Unix domain socket.
/// Environment variables are explicitly not supported as they leak to
/// child processes and `/proc/<pid>/environ`.
pub trait PassphraseProvider: Send + Sync {
    /// Get the passphrase bytes. Called on each `root_secret()` invocation.
    fn get_passphrase(&self) -> Result<Vec<u8>, SignerError>;
}

/// Passphrase provider that reads from a closure (for testing or integration).
pub struct ClosurePassphraseProvider<F: Fn() -> Result<Vec<u8>, SignerError> + Send + Sync> {
    f: F,
}

impl<F: Fn() -> Result<Vec<u8>, SignerError> + Send + Sync> ClosurePassphraseProvider<F> {
    /// Create a new closure-based passphrase provider.
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<F: Fn() -> Result<Vec<u8>, SignerError> + Send + Sync> PassphraseProvider
    for ClosurePassphraseProvider<F>
{
    fn get_passphrase(&self) -> Result<Vec<u8>, SignerError> {
        (self.f)()
    }
}

impl FileSigner {
    /// Create a file signer for the given path with a passphrase provider.
    pub fn new(path: impl Into<PathBuf>, provider: Box<dyn PassphraseProvider>) -> Self {
        let path = path.into();
        let label = format!("file:{}", path.display());
        Self { path, label, passphrase_provider: provider }
    }

    /// Create a file signer with a static passphrase (for testing only).
    #[cfg(test)]
    pub fn with_static_passphrase(path: impl Into<PathBuf>, passphrase: &'static [u8]) -> Self {
        Self::new(path, Box::new(ClosurePassphraseProvider { f: move || Ok(passphrase.to_vec()) }))
    }

    /// Default identity file path: `~/.config/styrene/identity.key`.
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        home.join(".config").join("styrene").join("identity.key")
    }

    /// Generate a new random root secret and save it encrypted with the passphrase.
    ///
    /// Uses `O_EXCL` semantics — **refuses to overwrite** an existing file.
    /// This is atomic at the kernel level (no TOCTOU race). To replace an
    /// existing identity, delete the file first (after backing up).
    pub fn generate(&self, passphrase: &[u8]) -> Result<(), SignerError> {
        let mut root_secret = [0u8; SECRET_LEN];
        OsRng.fill_bytes(&mut root_secret);

        let result = self.save_exclusive(&root_secret, passphrase);
        root_secret.zeroize();
        result
    }

    /// Encrypt and write an identity file. Uses `create_new(true)` (O_EXCL)
    /// to atomically refuse if the file already exists.
    fn save_exclusive(
        &self,
        root_secret: &[u8; SECRET_LEN],
        passphrase: &[u8],
    ) -> Result<(), SignerError> {
        let file_data = self.encrypt(root_secret, passphrase)?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true) // O_EXCL — atomic overwrite protection
                .mode(0o600)
                .open(&self.path)?;
            f.write_all(&file_data)?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            // create_new is available on all platforms via std
            let mut f =
                std::fs::OpenOptions::new().write(true).create_new(true).open(&self.path)?;
            use std::io::Write;
            f.write_all(&file_data)?;
            f.sync_all()?;
        }

        Ok(())
    }

    /// Encrypt a root secret into the wire format bytes.
    fn encrypt(
        &self,
        root_secret: &[u8; SECRET_LEN],
        passphrase: &[u8],
    ) -> Result<Vec<u8>, SignerError> {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        // Derive encryption key from passphrase via argon2id (hardened params)
        let mut key = [0u8; 32];
        argon2_params()
            .hash_password_into(passphrase, &salt, &mut key)
            .map_err(|e| SignerError::SigningFailed(format!("argon2id: {e}")))?;

        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| SignerError::SigningFailed(format!("cipher init: {e}")))?;
        key.zeroize();

        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, root_secret.as_ref())
            .map_err(|e| SignerError::SigningFailed(format!("encrypt: {e}")))?;

        let mut file_data = Vec::with_capacity(FILE_LEN);
        file_data.extend_from_slice(MAGIC);
        file_data.push(FORMAT_VERSION);
        file_data.extend_from_slice(&salt);
        file_data.extend_from_slice(&nonce_bytes);
        file_data.extend_from_slice(&ciphertext);
        Ok(file_data)
    }

    /// Load and decrypt the root secret.
    ///
    /// Accepts both the current versioned format (v1, 97 bytes with STID header)
    /// and the legacy headerless format (92 bytes) for backward compatibility.
    pub fn load(&self, passphrase: &[u8]) -> Result<RootSecret, SignerError> {
        let file_data = std::fs::read(&self.path)?;

        // Determine format: versioned (has STID header) or legacy (no header).
        let payload = if file_data.len() == FILE_LEN && file_data.starts_with(MAGIC) {
            let version = file_data[MAGIC.len()];
            if version != FORMAT_VERSION {
                return Err(SignerError::DecryptionFailed(format!(
                    "unsupported identity file version: {version} (expected {FORMAT_VERSION})"
                )));
            }
            &file_data[HEADER_LEN..]
        } else if file_data.len() == LEGACY_FILE_LEN {
            // Legacy v0 format without header — still supported for migration.
            &file_data[..]
        } else {
            return Err(SignerError::DecryptionFailed(format!(
                "invalid identity file: {} bytes (expected {FILE_LEN} or {LEGACY_FILE_LEN})",
                file_data.len()
            )));
        };

        let salt = &payload[..SALT_LEN];
        let nonce_bytes = &payload[SALT_LEN..SALT_LEN + NONCE_LEN];
        let ciphertext = &payload[SALT_LEN + NONCE_LEN..];

        // Derive key from passphrase (hardened params)
        let mut key = [0u8; 32];
        argon2_params()
            .hash_password_into(passphrase, salt, &mut key)
            .map_err(|e| SignerError::DecryptionFailed(format!("argon2id: {e}")))?;

        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| SignerError::DecryptionFailed(format!("cipher init: {e}")))?;
        key.zeroize();

        let nonce = Nonce::from_slice(nonce_bytes);
        let mut plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
            SignerError::DecryptionFailed("wrong passphrase or corrupted file".into())
        })?;

        let mut secret = [0u8; SECRET_LEN];
        secret.copy_from_slice(&plaintext);
        plaintext.zeroize();

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

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        let root = self.root_secret().await?;
        let deriver = crate::derive::KeyDeriver::new(root.as_bytes());
        let mut seed = deriver.derive(crate::derive::KeyPurpose::Signing);
        let sig = crate::pubkey::sign_with_seed(&seed, data);
        seed.zeroize();
        Ok(sig.to_vec())
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        // Wrap in Zeroizing to guarantee cleanup even if load() panics.
        let passphrase = zeroize::Zeroizing::new(self.passphrase_provider.get_passphrase()?);
        self.load(&passphrase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_signer() -> (FileSigner, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let signer = FileSigner::with_static_passphrase(path, b"test-passphrase");
        (signer, dir)
    }

    #[test]
    fn generate_and_load_roundtrip() {
        let (signer, _dir) = temp_signer();
        let passphrase = b"test-passphrase";

        signer.generate(passphrase).unwrap();
        assert!(signer.exists());

        let secret = signer.load(passphrase).unwrap();
        assert_ne!(secret.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn generate_refuses_overwrite() {
        let (signer, _dir) = temp_signer();
        signer.generate(b"test-passphrase").unwrap();
        let err = signer.generate(b"test-passphrase").unwrap_err();
        assert!(
            err.to_string().contains("exists") || err.to_string().contains("already"),
            "should refuse overwrite: {err}"
        );
    }

    #[test]
    fn wrong_passphrase_fails() {
        let (signer, _dir) = temp_signer();
        signer.generate(b"correct").unwrap();

        let result = signer.load(b"wrong");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong passphrase"),);
    }

    #[test]
    fn deterministic_key_from_same_file() {
        let (signer, _dir) = temp_signer();
        let passphrase = b"test-passphrase";

        signer.generate(passphrase).unwrap();

        let s1 = signer.load(passphrase).unwrap();
        let s2 = signer.load(passphrase).unwrap();
        assert_eq!(s1.as_bytes(), s2.as_bytes());
    }

    #[test]
    fn derived_keys_from_file_signer() {
        let (signer, _dir) = temp_signer();
        let passphrase = b"test-passphrase";

        signer.generate(passphrase).unwrap();
        let secret = signer.load(passphrase).unwrap();
        let keys = crate::derive::derive_keys(secret.as_bytes());

        assert_ne!(keys.rns_encryption, [0u8; 32]);
        assert_ne!(keys.signing, [0u8; 32]);
        assert_ne!(keys.rns_encryption, keys.signing);
    }

    #[test]
    fn tier_is_encrypted_file() {
        let (signer, _dir) = temp_signer();
        assert_eq!(signer.tier(), SignerTier::EncryptedFile);
    }

    #[tokio::test]
    async fn sign_produces_valid_ed25519() {
        let (signer, _dir) = temp_signer();
        let passphrase = b"test-passphrase";
        signer.generate(passphrase).unwrap();

        let data = b"hello styrene identity";
        let sig_bytes = signer.sign(data).await.unwrap();
        assert_eq!(sig_bytes.len(), 64);

        // Verify with the matching public key
        let root = signer.load(passphrase).unwrap();
        let deriver = crate::derive::KeyDeriver::new(root.as_bytes());
        let seed = deriver.derive(crate::derive::KeyPurpose::Signing);
        let vk = crate::pubkey::ed25519_verifying_key(&seed);

        let sig = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap());
        use ed25519_dalek::Verifier;
        assert!(vk.verify(data, &sig).is_ok());
    }

    #[test]
    fn not_available_before_generate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.key");
        let signer = FileSigner::with_static_passphrase(path, b"unused");
        assert!(!signer.is_available());
    }

    #[cfg(unix)]
    #[test]
    fn file_created_with_restricted_permissions() {
        let (signer, _dir) = temp_signer();
        signer.generate(b"test-passphrase").unwrap();

        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(signer.path()).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn root_secret_via_provider() {
        let (signer, _dir) = temp_signer();
        signer.generate(b"test-passphrase").unwrap();

        // The provider returns the passphrase; root_secret() should work.
        let root = signer.root_secret().await.unwrap();
        assert_ne!(root.as_bytes(), &[0u8; 32]);
    }
}
