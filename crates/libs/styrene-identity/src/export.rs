//! Batch public key export — all derived public keys in one call.
//!
//! Every consumer that displays identity information (CLI show, TUI panel,
//! Codex context) needs the same set of formatted public keys. This module
//! provides a single `AllPublicKeys::from_root()` that derives everything
//! in one pass with proper zeroization.

use zeroize::Zeroizing;

use crate::derive::{KeyDeriver, KeyPurpose};
use crate::format;
use crate::identity::{identity_hash, identity_pubkey};
use crate::signer::RootSecret;

/// All public keys derived from one identity, formatted for display and export.
///
/// Constructed via `from_root()` which derives every key purpose once,
/// extracts the public component, and zeroizes all seed material.
///
/// ```ignore
/// let keys = AllPublicKeys::from_root(&root);
/// println!("identity: {}", keys.identity_hash);
/// println!("ssh:      {}", keys.ssh_host_pubkey);
/// println!("wg:       {}", keys.wireguard_pubkey);
/// println!("age:      {}", keys.age_recipient);
/// ```
#[derive(Debug, Clone)]
pub struct AllPublicKeys {
    /// The 32-character hex identity hash.
    pub identity_hash: String,
    /// Raw 32-byte Ed25519 signing verifying key.
    pub signing_pubkey: [u8; 32],
    /// Hex-encoded signing pubkey (for embedding in signed profiles).
    pub signing_pubkey_hex: String,
    /// OpenSSH `ssh-ed25519 AAAA... styrene` format.
    pub ssh_host_pubkey: String,
    /// `SHA256:...` fingerprint matching `ssh-keygen -l` output.
    pub ssh_host_fingerprint: String,
    /// Base64-encoded WireGuard public key (X25519).
    pub wireguard_pubkey: String,
    /// age recipient string: `age1...` (empty if `age-format` feature disabled).
    pub age_recipient: String,
}

impl AllPublicKeys {
    /// Derive all public keys from a root secret.
    ///
    /// All intermediate seed material is zeroized after public key extraction.
    pub fn from_root(root: &RootSecret) -> Self {
        let identity_hash = identity_hash(root);
        let signing_pubkey = identity_pubkey(root);
        let signing_pubkey_hex = hex::encode(signing_pubkey);

        let deriver = KeyDeriver::new(root.as_bytes());

        // SSH host key — Zeroizing wrapper guarantees cleanup even on panic
        let (ssh_host_pubkey, ssh_host_fingerprint) = {
            let seed = Zeroizing::new(deriver.derive(KeyPurpose::SshHost));
            (format::ssh_pubkey(&seed, "styrene"), format::ssh_pubkey_fingerprint(&seed))
        };

        // WireGuard
        let wireguard_pubkey = {
            let secret = Zeroizing::new(deriver.derive(KeyPurpose::WireGuard));
            format::wireguard_pubkey(&secret)
        };

        // age recipient (feature-gated)
        let age_recipient = {
            #[cfg(feature = "age-format")]
            {
                let secret = Zeroizing::new(deriver.derive(KeyPurpose::Age));
                format::age_recipient(&secret)
            }
            #[cfg(not(feature = "age-format"))]
            {
                String::new()
            }
        };

        Self {
            identity_hash,
            signing_pubkey,
            signing_pubkey_hex,
            ssh_host_pubkey,
            ssh_host_fingerprint,
            wireguard_pubkey,
            age_recipient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(fill: u8) -> RootSecret {
        RootSecret::new([fill; 32])
    }

    #[test]
    fn from_root_produces_all_fields() {
        let keys = AllPublicKeys::from_root(&test_root(0x42));

        assert_eq!(keys.identity_hash.len(), 32);
        assert!(keys.identity_hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(keys.signing_pubkey_hex.len(), 64);
        assert!(keys.ssh_host_pubkey.starts_with("ssh-ed25519 "));
        assert!(keys.ssh_host_fingerprint.starts_with("SHA256:"));
        assert_eq!(keys.wireguard_pubkey.len(), 44); // base64 of 32 bytes
    }

    #[test]
    fn from_root_deterministic() {
        let a = AllPublicKeys::from_root(&test_root(0x42));
        let b = AllPublicKeys::from_root(&test_root(0x42));
        assert_eq!(a.identity_hash, b.identity_hash);
        assert_eq!(a.signing_pubkey, b.signing_pubkey);
        assert_eq!(a.ssh_host_pubkey, b.ssh_host_pubkey);
        assert_eq!(a.wireguard_pubkey, b.wireguard_pubkey);
    }

    #[test]
    fn different_roots_different_keys() {
        let a = AllPublicKeys::from_root(&test_root(0x01));
        let b = AllPublicKeys::from_root(&test_root(0x02));
        assert_ne!(a.identity_hash, b.identity_hash);
        assert_ne!(a.signing_pubkey, b.signing_pubkey);
        assert_ne!(a.wireguard_pubkey, b.wireguard_pubkey);
    }

    #[test]
    fn identity_hash_matches_standalone() {
        let root = test_root(0x42);
        let keys = AllPublicKeys::from_root(&root);
        assert_eq!(keys.identity_hash, identity_hash(&root));
    }
}
