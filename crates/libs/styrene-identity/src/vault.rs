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

use subtle::ConstantTimeEq;

use crate::file_signer::{FileSigner, PassphraseProvider};
use crate::signer::{IdentitySigner, RootSecret, SignerError};

/// Version of the opaque backup container contract.
const BACKUP_CONTRACT_VERSION: u8 = 1;

/// Existing encrypted identity format carried by a backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncryptedIdentityBackupFormat {
    /// Headerless encrypted identity files accepted for migration compatibility.
    LegacyV0,
    /// Current `STID` version 1 encrypted identity file.
    StidV1,
}

/// Non-secret metadata for an opaque encrypted identity backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncryptedIdentityBackupMetadata {
    pub contract_version: u8,
    pub format: EncryptedIdentityBackupFormat,
    pub encrypted_size: u64,
}

/// An encrypted identity file whose plaintext remains opaque to presentation code.
#[derive(Clone, Eq, PartialEq)]
pub struct EncryptedIdentityBackup {
    metadata: EncryptedIdentityBackupMetadata,
    encrypted_bytes: Vec<u8>,
}

impl EncryptedIdentityBackup {
    /// Protect a root secret as a portable Argon2id-encrypted backup artifact.
    pub fn protect_root_secret(
        root_secret: &RootSecret,
        passphrase: &[u8],
    ) -> Result<Self, VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::ProtectionRequired);
        }
        let encrypted_bytes = FileSigner::encrypt_root_secret(root_secret.as_bytes(), passphrase)?;
        Self::from_encrypted_bytes(encrypted_bytes)
    }

    /// Parse structurally valid encrypted identity bytes without decrypting them.
    /// Authentication is performed by [`IdentityVault::restore_encrypted_backup`].
    pub fn from_encrypted_bytes(encrypted_bytes: Vec<u8>) -> Result<Self, VaultError> {
        let format = encrypted_backup_format(&encrypted_bytes)?;
        let encrypted_size =
            encrypted_bytes.len().try_into().map_err(|_| VaultError::InvalidBackup)?;
        Ok(Self {
            metadata: EncryptedIdentityBackupMetadata {
                contract_version: BACKUP_CONTRACT_VERSION,
                format,
                encrypted_size,
            },
            encrypted_bytes,
        })
    }

    /// Safe metadata that describes, but cannot decrypt, this backup.
    pub fn metadata(&self) -> EncryptedIdentityBackupMetadata {
        self.metadata
    }

    /// Opaque authenticated-encryption bytes for storage or transfer.
    pub fn encrypted_bytes(&self) -> &[u8] {
        &self.encrypted_bytes
    }

    /// Authenticate and decrypt this artifact using its portable protection input.
    pub fn decrypt_root_secret(&self, passphrase: &[u8]) -> Result<RootSecret, VaultError> {
        if passphrase.is_empty() {
            return Err(VaultError::ProtectionRequired);
        }
        FileSigner::decrypt(self.encrypted_bytes(), passphrase)
            .map_err(|_| VaultError::BackupAuthenticationFailed)
    }
}

impl std::fmt::Debug for EncryptedIdentityBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedIdentityBackup")
            .field("metadata", &self.metadata)
            .field("encrypted_bytes", &"[REDACTED]")
            .finish()
    }
}

/// Result of a non-destructive identity restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityRestoreOutcome {
    /// No identity existed and the encrypted backup was installed exclusively.
    Restored,
    /// The active custody already contains the backed-up identity; nothing changed.
    AlreadyPresent,
}

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

    /// Backup bytes do not identify a supported encrypted identity format.
    #[error("invalid or unsupported encrypted identity backup")]
    InvalidBackup,

    /// Portable backup protection input was empty.
    #[error("encrypted identity backup protection is required")]
    ProtectionRequired,

    /// Backup authentication failed. Wrong protection and corruption are intentionally
    /// indistinguishable because authenticated encryption cannot safely classify them.
    #[error("encrypted identity backup authentication failed")]
    BackupAuthenticationFailed,

    /// Existing custody could not be authenticated and must not be replaced.
    #[error("existing identity custody is unavailable")]
    CustodyUnavailable,

    /// Restore would replace a different identity.
    #[error("identity restore conflicts with the existing identity")]
    IdentityConflict,

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

        // Read and validate the supported encrypted source format.
        let data = std::fs::read(&self.path)?;
        encrypted_backup_format(&data)?;
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

    /// Export the exact encrypted identity bytes after authenticating them with the
    /// configured protection provider. No plaintext or protection input is returned.
    pub fn export_encrypted_backup(&self) -> Result<EncryptedIdentityBackup, VaultError> {
        if !self.path.exists() {
            return Err(VaultError::NotInitialized { path: self.path.display().to_string() });
        }
        let encrypted_bytes = std::fs::read(&self.path)?;
        let backup = EncryptedIdentityBackup::from_encrypted_bytes(encrypted_bytes)?;
        self.signer
            .load_encrypted_bytes(backup.encrypted_bytes())
            .map_err(map_backup_authentication_error)?;
        Ok(backup)
    }

    /// Inspect non-secret backup metadata without exporting artifact bytes.
    pub fn encrypted_backup_metadata(&self) -> Result<EncryptedIdentityBackupMetadata, VaultError> {
        if !self.path.exists() {
            return Err(VaultError::NotInitialized { path: self.path.display().to_string() });
        }
        let encrypted_bytes = std::fs::read(&self.path)?;
        Ok(EncryptedIdentityBackup::from_encrypted_bytes(encrypted_bytes)?.metadata())
    }

    /// Authenticate and restore a backup without replacing existing custody.
    ///
    /// If custody already holds the same identity, this is an idempotent no-op. A
    /// different or inaccessible existing identity is left byte-for-byte unchanged.
    pub fn restore_encrypted_backup(
        &self,
        backup: &EncryptedIdentityBackup,
    ) -> Result<IdentityRestoreOutcome, VaultError> {
        let backup_secret = self
            .signer
            .load_encrypted_bytes(backup.encrypted_bytes())
            .map_err(map_backup_authentication_error)?;

        if self.path.exists() {
            let existing_bytes = std::fs::read(&self.path)?;
            let existing_secret = self
                .signer
                .load_encrypted_bytes(&existing_bytes)
                .map_err(|_| VaultError::CustodyUnavailable)?;
            if bool::from(existing_secret.as_bytes().ct_eq(backup_secret.as_bytes())) {
                return Ok(IdentityRestoreOutcome::AlreadyPresent);
            }
            return Err(VaultError::IdentityConflict);
        }

        write_exclusive(&self.path, backup.encrypted_bytes()).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                VaultError::IdentityConflict
            } else {
                VaultError::Io(error)
            }
        })?;
        Ok(IdentityRestoreOutcome::Restored)
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

fn encrypted_backup_format(data: &[u8]) -> Result<EncryptedIdentityBackupFormat, VaultError> {
    if data.len() == crate::file_signer::FILE_LEN && data.starts_with(b"STID\x01") {
        Ok(EncryptedIdentityBackupFormat::StidV1)
    } else if data.len() == crate::file_signer::LEGACY_FILE_LEN {
        Ok(EncryptedIdentityBackupFormat::LegacyV0)
    } else {
        Err(VaultError::InvalidBackup)
    }
}

fn map_backup_authentication_error(error: SignerError) -> VaultError {
    match error {
        SignerError::Unavailable(_)
        | SignerError::AuthRequired(_)
        | SignerError::KeyNotFound(_) => VaultError::CustodyUnavailable,
        _ => VaultError::BackupAuthenticationFailed,
    }
}

fn write_exclusive(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    use std::io::Write;
    file.write_all(data)?;
    file.sync_all()
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
        test_vault_with_passphrase(dir, b"test-passphrase")
    }

    fn test_vault_with_passphrase(
        dir: &std::path::Path,
        passphrase: &'static [u8],
    ) -> IdentityVault {
        let path = dir.join("identity.key");
        IdentityVault::new(
            path,
            Box::new(ClosurePassphraseProvider::new(move || Ok(passphrase.to_vec()))),
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

    #[test]
    fn authenticated_export_and_non_destructive_restore_roundtrip() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = test_vault(source_dir.path());
        let target = test_vault(target_dir.path());
        source.init(b"test-passphrase").unwrap();

        let backup = source.export_encrypted_backup().unwrap();
        assert_eq!(source.encrypted_backup_metadata().unwrap(), backup.metadata());
        assert_eq!(backup.metadata().contract_version, 1);
        assert_eq!(backup.metadata().format, EncryptedIdentityBackupFormat::StidV1);
        assert_eq!(
            target.restore_encrypted_backup(&backup).unwrap(),
            IdentityRestoreOutcome::Restored
        );
        assert_eq!(
            target.restore_encrypted_backup(&backup).unwrap(),
            IdentityRestoreOutcome::AlreadyPresent
        );
        assert_eq!(std::fs::read(source.path()).unwrap(), std::fs::read(target.path()).unwrap());
    }

    #[tokio::test]
    async fn legacy_encrypted_backup_remains_compatible() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = test_vault(source_dir.path());
        let target = test_vault(target_dir.path());
        source.init(b"test-passphrase").unwrap();
        let versioned = std::fs::read(source.path()).unwrap();
        std::fs::write(source.path(), &versioned[5..]).unwrap();

        let backup = source.export_encrypted_backup().unwrap();
        assert_eq!(backup.metadata().format, EncryptedIdentityBackupFormat::LegacyV0);
        assert_eq!(
            target.restore_encrypted_backup(&backup).unwrap(),
            IdentityRestoreOutcome::Restored
        );
        assert!(target.unlock().await.is_ok());
    }

    #[test]
    fn wrong_protection_fails_restore_without_creating_custody() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let creator = test_vault_with_passphrase(source_dir.path(), b"correct-protection");
        creator.init(b"correct-protection").unwrap();
        let backup = creator.export_encrypted_backup().unwrap();
        let wrong = test_vault_with_passphrase(target_dir.path(), b"wrong-protection");

        let error = wrong.restore_encrypted_backup(&backup).unwrap_err();
        assert!(matches!(error, VaultError::BackupAuthenticationFailed));
        assert_eq!(error.to_string(), "encrypted identity backup authentication failed");
        assert!(!error.to_string().contains("wrong-protection"));
        assert!(!wrong.exists());
    }

    #[test]
    fn corrupted_backup_does_not_create_custody() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = test_vault(source_dir.path());
        let target = test_vault(target_dir.path());
        source.init(b"test-passphrase").unwrap();
        let backup = source.export_encrypted_backup().unwrap();
        let mut corrupted = backup.encrypted_bytes().to_vec();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        let corrupted = EncryptedIdentityBackup::from_encrypted_bytes(corrupted).unwrap();

        let error = target.restore_encrypted_backup(&corrupted).unwrap_err();
        assert!(matches!(error, VaultError::BackupAuthenticationFailed));
        assert_eq!(error.to_string(), "encrypted identity backup authentication failed");
        assert!(!target.exists());
    }

    #[test]
    fn unavailable_custody_cannot_export() {
        let dir = tempfile::tempdir().unwrap();
        let creator = test_vault(dir.path());
        creator.init(b"test-passphrase").unwrap();
        let unavailable = IdentityVault::new(
            creator.path(),
            Box::new(ClosurePassphraseProvider::new(|| {
                Err(SignerError::Unavailable("host custody locked".into()))
            })),
        );

        assert!(matches!(
            unavailable.export_encrypted_backup(),
            Err(VaultError::CustodyUnavailable)
        ));
    }

    #[test]
    fn conflicting_identity_is_left_unchanged() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = test_vault(source_dir.path());
        let target = test_vault(target_dir.path());
        source.init(b"test-passphrase").unwrap();
        target.init(b"test-passphrase").unwrap();
        let target_before = std::fs::read(target.path()).unwrap();
        let backup = source.export_encrypted_backup().unwrap();

        assert!(matches!(
            target.restore_encrypted_backup(&backup),
            Err(VaultError::IdentityConflict)
        ));
        assert_eq!(std::fs::read(target.path()).unwrap(), target_before);
    }

    #[test]
    fn backup_debug_redacts_encrypted_payload() {
        let dir = tempfile::tempdir().unwrap();
        let vault = test_vault(dir.path());
        vault.init(b"test-passphrase").unwrap();
        let backup = vault.export_encrypted_backup().unwrap();
        let debug = format!("{backup:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(&hex::encode(backup.encrypted_bytes())));
        assert!(!debug.contains("test-passphrase"));
    }

    #[tokio::test]
    async fn portable_backup_reencrypts_root_for_user_passphrase() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = test_vault(source_dir.path());
        source.init(b"test-passphrase").unwrap();
        let root = source.unlock().await.unwrap();

        let backup =
            EncryptedIdentityBackup::protect_root_secret(&root, b"portable-passphrase").unwrap();
        assert_eq!(backup.metadata().format, EncryptedIdentityBackupFormat::StidV1);
        assert_eq!(
            backup.decrypt_root_secret(b"portable-passphrase").unwrap().as_bytes(),
            root.as_bytes()
        );
        assert!(matches!(
            backup.decrypt_root_secret(b"wrong-passphrase"),
            Err(VaultError::BackupAuthenticationFailed)
        ));
    }
}
