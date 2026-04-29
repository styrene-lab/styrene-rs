//! Canonical identity hash and info — the unique fingerprint for a Styrene identity.
//!
//! The identity hash is `SHA-256(Ed25519 verifying key)` truncated to 16 bytes,
//! rendered as 32 lowercase hex characters. This is the short, human-readable
//! identifier that every consumer needs but previously had to hand-roll.
//!
//! All intermediate seed material is zeroized after extracting the public key.

use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::derive::{KeyDeriver, KeyPurpose};
use crate::pubkey::ed25519_verifying_key;
use crate::signer::RootSecret;

/// Number of bytes kept from the SHA-256 digest (16 bytes = 32 hex chars).
pub const IDENTITY_HASH_BYTES: usize = 16;

/// Compute the canonical identity hash from a root secret.
///
/// Returns a 32-character lowercase hex string:
/// `hex(SHA-256(Ed25519_verifying_key(Signing_seed))[..16])`
///
/// The derived seed is zeroized after the public key is extracted.
pub fn identity_hash(root: &RootSecret) -> String {
    let pubkey = identity_pubkey(root);
    let digest = Sha256::digest(pubkey);
    hex::encode(&digest[..IDENTITY_HASH_BYTES])
}

/// Extract the raw Ed25519 signing public key bytes from a root secret.
///
/// The derived seed is zeroized after the public key is extracted.
pub fn identity_pubkey(root: &RootSecret) -> [u8; 32] {
    let deriver = KeyDeriver::new(root.as_bytes());
    let mut seed = deriver.derive(KeyPurpose::Signing);
    let vk = ed25519_verifying_key(&seed);
    seed.zeroize();
    vk.to_bytes()
}

/// Bundled identity information — hash and public key together.
#[derive(Debug, Clone)]
pub struct IdentityInfo {
    /// The 32-character hex identity hash.
    pub hash: String,
    /// The raw 32-byte Ed25519 verifying (public) key.
    pub pubkey: [u8; 32],
}

impl IdentityInfo {
    /// Construct from a root secret, computing both hash and pubkey.
    pub fn from_root(root: &RootSecret) -> Self {
        let pubkey = identity_pubkey(root);
        let digest = Sha256::digest(pubkey);
        let hash = hex::encode(&digest[..IDENTITY_HASH_BYTES]);
        Self { hash, pubkey }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(fill: u8) -> RootSecret {
        RootSecret::new([fill; 32])
    }

    #[test]
    fn identity_hash_deterministic() {
        let root = test_root(0x42);
        let h1 = identity_hash(&root);
        let h2 = identity_hash(&root);
        assert_eq!(h1, h2, "same root must produce the same hash");
    }

    #[test]
    fn identity_hash_is_32_hex_chars() {
        let h = identity_hash(&test_root(0x42));
        assert_eq!(h.len(), 32, "identity hash must be 32 hex chars");
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit()),
            "identity hash must be valid hex"
        );
    }

    #[test]
    fn different_roots_produce_different_hashes() {
        let h1 = identity_hash(&test_root(0x01));
        let h2 = identity_hash(&test_root(0x02));
        assert_ne!(h1, h2, "different roots must produce different hashes");
    }

    #[test]
    fn identity_info_matches_individual_functions() {
        let root = test_root(0x42);
        let info = IdentityInfo::from_root(&root);
        assert_eq!(info.hash, identity_hash(&root));
        assert_eq!(info.pubkey, identity_pubkey(&root));
    }

    #[test]
    fn test_vector_identity_hash() {
        // Pinned test vector: root = [0x42; 32]
        let root = test_root(0x42);
        let h = identity_hash(&root);
        assert_eq!(
            h, "6279e31aff9bc151638ac305d88ab6bc",
            "pinned identity hash for root=[0x42; 32]"
        );
    }
}
