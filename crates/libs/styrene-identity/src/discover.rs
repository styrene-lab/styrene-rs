//! Identity auto-discovery — probes the machine for an existing Styrene identity.
//!
//! Checks well-known locations in priority order without requiring a passphrase.
//! Returns a [`DiscoveredIdentity`] describing the found identity file or env var,
//! or `None` if no identity is configured.

use std::path::PathBuf;

use crate::signer::SignerTier;

/// A discovered identity on the local machine.
#[derive(Debug, Clone)]
pub struct DiscoveredIdentity {
    /// Path to the identity file, or `None` for hash-only discovery.
    pub path: PathBuf,
    /// The signer tier that would be used.
    pub tier: SignerTier,
    /// Human-readable description of the discovery source.
    pub label: String,
}

impl DiscoveredIdentity {
    /// Whether this discovery is hash-only (from `STYRENE_IDENTITY_HASH` env var).
    ///
    /// Hash-only identities provide attribution but cannot sign — no key material
    /// is available on disk.
    pub fn is_hash_only(&self) -> bool {
        self.tier == SignerTier::CredentialManager
            && self.label.starts_with("env:STYRENE_IDENTITY_HASH")
    }
}

/// Probe the machine for an existing Styrene identity.
///
/// Discovery order:
///   0. macOS/iOS Keychain with biometric protection (Tier B)
///   1. `~/.config/styrene/identity.key` — default encrypted file location
///   2. `STYRENE_IDENTITY_PATH` env var — custom file path
///   3. `STYRENE_IDENTITY_HASH` env var — hash-only mode (CI attribution)
///
/// Returns `None` if no identity is found. Does not require a passphrase —
/// only checks file existence and env var presence.
pub fn discover() -> Option<DiscoveredIdentity> {
    // 0. Keychain with biometric protection (macOS/iOS)
    #[cfg(all(feature = "keychain", any(target_os = "macos", target_os = "ios")))]
    {
        let ks = crate::keychain_signer::KeychainSigner::default();
        if ks.exists() {
            return Some(DiscoveredIdentity {
                path: PathBuf::from("(Keychain)"),
                tier: SignerTier::DeviceHsm,
                label: "Keychain (biometric)".to_string(),
            });
        }
    }

    discover_from_sources(
        home_dir(),
        std::env::var("STYRENE_IDENTITY_PATH").ok(),
        std::env::var("STYRENE_IDENTITY_HASH").ok(),
    )
}

fn discover_from_sources(
    home: Option<PathBuf>,
    custom_path: Option<String>,
    identity_hash: Option<String>,
) -> Option<DiscoveredIdentity> {
    // 1. Default config path
    if let Some(home) = home {
        let default_path = home.join(".config").join("styrene").join("identity.key");
        if default_path.is_file() {
            return Some(DiscoveredIdentity {
                path: default_path,
                tier: SignerTier::EncryptedFile,
                label: "~/.config/styrene/identity.key".to_string(),
            });
        }
    }

    // 2. Custom file path from env var
    if let Some(custom_path) = custom_path {
        let path = PathBuf::from(&custom_path);
        if path.is_file() {
            return Some(DiscoveredIdentity {
                path,
                tier: SignerTier::EncryptedFile,
                label: format!("env:STYRENE_IDENTITY_PATH={custom_path}"),
            });
        }
    }

    // 3. Hash-only mode from env var (CI attribution)
    if let Some(hash) = identity_hash
        && !hash.is_empty()
    {
        return Some(DiscoveredIdentity {
            path: PathBuf::from(format!("hash:{hash}")),
            tier: SignerTier::CredentialManager,
            label: format!("env:STYRENE_IDENTITY_HASH={hash}"),
        });
    }

    None
}

/// Resolve the user's home directory.
///
/// Prefers `$HOME` for testability, falls back to `dirs::home_dir` pattern.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discover_returns_none_when_nothing_configured() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(
            discover_from_sources(Some(tmp.path().to_path_buf()), None, None).is_none(),
            "should return None with no identity"
        );
    }

    #[test]
    fn discover_finds_default_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_dir = tmp.path().join(".config").join("styrene");
        fs::create_dir_all(&key_dir).unwrap();
        fs::write(key_dir.join("identity.key"), b"fake-key-data").unwrap();

        let result = discover_from_sources(Some(tmp.path().to_path_buf()), None, None)
            .expect("should find default identity file");
        assert_eq!(result.tier, SignerTier::EncryptedFile);
        assert!(result.path.ends_with("identity.key"));
        assert!(!result.is_hash_only());
    }

    #[test]
    fn discover_env_path_overrides_when_no_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let custom_file = tmp.path().join("custom.key");
        fs::write(&custom_file, b"fake-key-data").unwrap();

        let result = discover_from_sources(
            Some(tmp.path().to_path_buf()),
            Some(custom_file.to_string_lossy().into_owned()),
            None,
        )
        .expect("should find env var identity");
        assert_eq!(result.tier, SignerTier::EncryptedFile);
        assert_eq!(result.path, custom_file);
        assert!(!result.is_hash_only());
    }

    #[test]
    fn discover_hash_only_from_env() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = discover_from_sources(
            Some(tmp.path().to_path_buf()),
            None,
            Some("abcdef1234567890abcdef1234567890".to_string()),
        )
        .expect("should find hash-only identity");
        assert!(result.is_hash_only(), "should be hash-only");
        assert_eq!(result.label, "env:STYRENE_IDENTITY_HASH=abcdef1234567890abcdef1234567890");
    }

    #[test]
    fn discover_default_path_takes_priority_over_env() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let key_dir = tmp.path().join(".config").join("styrene");
        fs::create_dir_all(&key_dir).unwrap();
        fs::write(key_dir.join("identity.key"), b"fake-key-data").unwrap();

        let result = discover_from_sources(
            Some(tmp.path().to_path_buf()),
            None,
            Some("somehash".to_string()),
        )
        .expect("should find identity");
        assert_eq!(result.tier, SignerTier::EncryptedFile);
        assert!(result.path.ends_with("identity.key"), "default path should win");
    }
}
