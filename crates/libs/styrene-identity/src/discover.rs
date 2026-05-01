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
        self.tier == SignerTier::CredentialManager && self.label.starts_with("env:STYRENE_IDENTITY_HASH")
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
    #[cfg(feature = "keychain")]
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

    // 1. Default config path
    if let Some(home) = home_dir() {
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
    if let Ok(custom_path) = std::env::var("STYRENE_IDENTITY_PATH") {
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
    if let Ok(hash) = std::env::var("STYRENE_IDENTITY_HASH") {
        if !hash.is_empty() {
            return Some(DiscoveredIdentity {
                path: PathBuf::from(format!("hash:{hash}")),
                tier: SignerTier::CredentialManager,
                label: format!("env:STYRENE_IDENTITY_HASH={hash}"),
            });
        }
    }

    None
}

/// Resolve the user's home directory.
///
/// Prefers `$HOME` for testability, falls back to `dirs::home_dir` pattern.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Mutex to serialize tests that mutate environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Run a closure with specific env vars set, restoring originals afterward.
    /// Also sets HOME to a temporary directory to avoid finding the real identity.
    /// Serialized via `ENV_LOCK` to prevent parallel test interference.
    fn with_clean_env<F: FnOnce(&std::path::Path)>(f: F) {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let tmp = tempfile::tempdir().expect("tempdir");

        // Save originals
        let orig_home = std::env::var("HOME").ok();
        let orig_path = std::env::var("STYRENE_IDENTITY_PATH").ok();
        let orig_hash = std::env::var("STYRENE_IDENTITY_HASH").ok();

        // Clean environment
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("STYRENE_IDENTITY_PATH");
        std::env::remove_var("STYRENE_IDENTITY_HASH");

        f(tmp.path());

        // Restore originals
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match orig_path {
            Some(v) => std::env::set_var("STYRENE_IDENTITY_PATH", v),
            None => std::env::remove_var("STYRENE_IDENTITY_PATH"),
        }
        match orig_hash {
            Some(v) => std::env::set_var("STYRENE_IDENTITY_HASH", v),
            None => std::env::remove_var("STYRENE_IDENTITY_HASH"),
        }
    }

    #[test]
    fn discover_returns_none_when_nothing_configured() {
        with_clean_env(|_tmp| {
            assert!(discover().is_none(), "should return None with no identity");
        });
    }

    #[test]
    fn discover_finds_default_path() {
        with_clean_env(|tmp| {
            let key_dir = tmp.join(".config").join("styrene");
            fs::create_dir_all(&key_dir).unwrap();
            fs::write(key_dir.join("identity.key"), b"fake-key-data").unwrap();

            let result = discover().expect("should find default identity file");
            assert_eq!(result.tier, SignerTier::EncryptedFile);
            assert!(result.path.ends_with("identity.key"));
            assert!(!result.is_hash_only());
        });
    }

    #[test]
    fn discover_env_path_overrides_when_no_default() {
        with_clean_env(|tmp| {
            // No default file exists, but STYRENE_IDENTITY_PATH is set
            let custom_file = tmp.join("custom.key");
            fs::write(&custom_file, b"fake-key-data").unwrap();
            std::env::set_var("STYRENE_IDENTITY_PATH", &custom_file);

            let result = discover().expect("should find env var identity");
            assert_eq!(result.tier, SignerTier::EncryptedFile);
            assert_eq!(result.path, custom_file);
            assert!(!result.is_hash_only());
        });
    }

    #[test]
    fn discover_hash_only_from_env() {
        with_clean_env(|_tmp| {
            std::env::set_var("STYRENE_IDENTITY_HASH", "abcdef1234567890abcdef1234567890");

            let result = discover().expect("should find hash-only identity");
            assert!(result.is_hash_only(), "should be hash-only");
            assert_eq!(
                result.label,
                "env:STYRENE_IDENTITY_HASH=abcdef1234567890abcdef1234567890"
            );
        });
    }

    #[test]
    fn discover_default_path_takes_priority_over_env() {
        with_clean_env(|tmp| {
            // Set up both default file and env vars
            let key_dir = tmp.join(".config").join("styrene");
            fs::create_dir_all(&key_dir).unwrap();
            fs::write(key_dir.join("identity.key"), b"fake-key-data").unwrap();

            std::env::set_var("STYRENE_IDENTITY_HASH", "somehash");

            let result = discover().expect("should find identity");
            assert_eq!(result.tier, SignerTier::EncryptedFile);
            assert!(result.path.ends_with("identity.key"), "default path should win");
        });
    }
}
