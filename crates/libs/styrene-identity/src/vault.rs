//! Identity vault — safe lifecycle management for Styrene identities.
//!
//! Wraps [`FileSigner`] with guardrails:
//! - `init()` refuses to overwrite an existing identity
//! - `backup()` exports an encrypted copy before destructive operations
//! - Clear error messages guide operators through each failure mode
//! - Agent name and SSH label validation at config time (not derivation time)
//!
//! # Usage
//!
//! ```ignore
//! use styrene_identity::vault::IdentityVault;
//!
//! let vault = IdentityVault::new("/etc/styrene/identity.key", provider);
//!
//! // First-time setup — refuses if file already exists
//! vault.init(b"passphrase")?;
//!
//! // Backup before any risky operation
//! vault.backup("/etc/styrene/identity.key.bak")?;
//!
//! // Derive keys safely
//! let root = vault.unlock().await?;
//! ```

use std::path::{Path, PathBuf};

use crate::file_signer::{FileSigner, PassphraseProvider};
use crate::signer::{IdentitySigner, RootSecret, SignerError};

/// Safe lifecycle wrapper around a file-based identity.
pub struct IdentityVault {
    signer: FileSigner,
    path: PathBuf,
}

/// Errors specific to vault lifecycle operations.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// Attempted to initialize over an existing identity file.
    #[error(
        "identity file already exists at '{path}' — \
         refusing to overwrite. Back up the existing identity first, \
         or use a different path."
    )]
    AlreadyExists { path: String },

    /// Identity file does not exist (need to call init() first).
    #[error(
        "no identity file at '{path}' — \
         run identity initialization first to create one."
    )]
    NotInitialized { path: String },

    /// Backup destination already exists.
    #[error(
        "backup destination already exists at '{path}' — \
         choose a different backup path to avoid overwriting."
    )]
    BackupExists { path: String },

    /// Underlying signer error.
    #[error("{0}")]
    Signer(#[from] SignerError),

    /// I/O error during backup.
    #[error("backup failed: {0}")]
    Io(#[from] std::io::Error),
}

impl IdentityVault {
    /// Create a vault for the given identity file path.
    pub fn new(path: impl Into<PathBuf>, provider: Box<dyn PassphraseProvider>) -> Self {
        let path = path.into();
        let signer = FileSigner::new(&path, provider);
        Self { signer, path }
    }

    /// Create a vault using the default identity path (`~/.config/styrene/identity.key`).
    pub fn with_default_path(provider: Box<dyn PassphraseProvider>) -> Self {
        Self::new(FileSigner::default_path(), provider)
    }

    /// Whether an identity file exists at this vault's path.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Path to the identity file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Initialize a new identity. **Refuses to overwrite** an existing file.
    ///
    /// This is the only way to create a new identity through the vault.
    /// Uses `O_EXCL` (kernel-level atomic check) — no TOCTOU race.
    /// If a file already exists, returns `VaultError::AlreadyExists` with
    /// instructions to back up first.
    pub fn init(&self, passphrase: &[u8]) -> Result<(), VaultError> {
        match self.signer.generate(passphrase) {
            Ok(()) => Ok(()),
            Err(SignerError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(VaultError::AlreadyExists { path: self.path.display().to_string() })
            }
            Err(e) => Err(VaultError::Signer(e)),
        }
    }

    /// Create an encrypted backup copy of the identity file.
    ///
    /// The backup is a byte-for-byte copy of the encrypted file (not a
    /// plaintext export). The backup destination must not already exist.
    pub fn backup(&self, dest: impl AsRef<Path>) -> Result<(), VaultError> {
        let dest = dest.as_ref();

        if !self.path.exists() {
            return Err(VaultError::NotInitialized { path: self.path.display().to_string() });
        }
        if dest.exists() {
            return Err(VaultError::BackupExists { path: dest.display().to_string() });
        }

        // Read and validate source file size.
        let data = std::fs::read(&self.path)?;
        if data.len() != crate::file_signer::FILE_LEN {
            return Err(VaultError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "identity file is {} bytes, expected {} — file may be corrupted",
                    data.len(),
                    crate::file_signer::FILE_LEN
                ),
            )));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f =
                std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dest)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(dest, &data)?;
        }

        Ok(())
    }

    /// Unlock the identity — decrypt and return the root secret.
    ///
    /// The passphrase is obtained from the provider given at construction.
    pub async fn unlock(&self) -> Result<RootSecret, VaultError> {
        if !self.path.exists() {
            return Err(VaultError::NotInitialized { path: self.path.display().to_string() });
        }
        Ok(self.signer.root_secret().await?)
    }

    /// Access the underlying signer (for use with `KeyDeriver`, SSH agent, etc.).
    pub fn signer(&self) -> &FileSigner {
        &self.signer
    }
}

/// Validate a list of agent names at config time.
///
/// Returns a list of invalid names. Call this when loading agent config,
/// before any derivation happens — catches empty names early with a
/// clear error instead of a runtime panic.
pub fn validate_agent_names(names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|n| crate::derive::validate_label(n).is_err())
        .map(|n| n.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_signer::ClosurePassphraseProvider;

    fn test_vault(dir: &std::path::Path) -> IdentityVault {
        let path = dir.join("identity.key");
        IdentityVault::new(
            path,
            Box::new(ClosurePassphraseProvider::new(|| Ok(b"test-passphrase".to_vec()))),
        )
    }

    #[test]
    fn init_creates_identity() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        assert!(!vault.exists());
        vault.init(b"test-passphrase").unwrap();
        assert!(vault.exists());
    }

    #[test]
    fn init_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        vault.init(b"test-passphrase").unwrap();
        let err = vault.init(b"test-passphrase").unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "error should mention existing file: {err}"
        );
    }

    #[test]
    fn backup_creates_copy() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        vault.init(b"test-passphrase").unwrap();
        let backup_path = dir.path().join("identity.key.bak");
        vault.backup(&backup_path).unwrap();

        assert!(backup_path.exists());
        assert_eq!(std::fs::read(vault.path()).unwrap(), std::fs::read(&backup_path).unwrap());
    }

    #[test]
    fn backup_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        vault.init(b"test-passphrase").unwrap();
        let backup_path = dir.path().join("identity.key.bak");
        vault.backup(&backup_path).unwrap();

        let err = vault.backup(&backup_path).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn backup_requires_existing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        let err = vault.backup(dir.path().join("backup.key")).unwrap_err();
        assert!(err.to_string().contains("no identity file"));
    }

    #[tokio::test]
    async fn unlock_returns_root_secret() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        vault.init(b"test-passphrase").unwrap();
        let root = vault.unlock().await.unwrap();
        assert_ne!(root.as_bytes(), &[0u8; 32]);
    }

    #[tokio::test]
    async fn unlock_requires_existing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        let err = vault.unlock().await.unwrap_err();
        assert!(err.to_string().contains("no identity file"));
    }

    #[test]
    fn validate_agent_names_catches_empty() {
        let invalid = validate_agent_names(&["omegon-primary", "", "cleave-0"]);
        assert_eq!(invalid, vec![""]);
    }

    #[test]
    fn validate_agent_names_all_valid() {
        let invalid = validate_agent_names(&["omegon-primary", "cleave-0", "auspex"]);
        assert!(invalid.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn backup_has_restricted_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());

        vault.init(b"test-passphrase").unwrap();
        let backup_path = dir.path().join("identity.key.bak");
        vault.backup(&backup_path).unwrap();

        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&backup_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}
