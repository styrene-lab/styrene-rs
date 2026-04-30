//! Canonical identity hash and info — the unique fingerprint for a Styrene identity.
//!
//! The identity hash is `SHA-256(Ed25519 verifying key)` truncated to 16 bytes,
//! rendered as 32 lowercase hex characters. This is the short, human-readable
//! identifier that every consumer needs but previously had to hand-roll.
//!
//! All intermediate seed material is zeroized after extracting the public key.

use ed25519_dalek::Verifier;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::derive::{KeyDeriver, KeyPurpose};
use crate::pubkey::{ed25519_verifying_key, sign_with_seed};
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

// ── High-level sign/verify ────────────────────────────────────────────────

/// Sign arbitrary data with the identity's Ed25519 signing key.
///
/// Returns `(identity_hash, signature)`. All seed material is zeroized
/// after signing. This is the canonical way to attest data (profiles,
/// audit records, agent delegations) — consumers should prefer this
/// over manual `KeyDeriver` + `sign_with_seed` sequences.
pub fn identity_sign(root: &RootSecret, data: &[u8]) -> (String, [u8; 64]) {
    let deriver = KeyDeriver::new(root.as_bytes());
    let mut seed = deriver.derive(KeyPurpose::Signing);
    let signature = sign_with_seed(&seed, data);
    let hash = identity_hash(root);
    seed.zeroize();
    (hash, signature)
}

/// Verify a signature against an Ed25519 public key.
///
/// No root secret needed — works with just the 32-byte verifying key
/// (e.g. the `pubkey` field embedded in a signed profile).
pub fn identity_verify(pubkey: &[u8; 32], data: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    vk.verify(data, &sig).is_ok()
}

// ── Public identity (no secrets) ──────────────────────────────────────────

/// A public-only view of a Styrene identity. Contains no secrets.
///
/// Constructed from the 32-byte Ed25519 verifying key (pubkey) that is
/// embedded in signed profiles, announced on the mesh, or returned by
/// `identity_pubkey()`. Use this when you need to verify signatures or
/// check hash bindings without holding a root secret.
#[derive(Debug, Clone)]
pub struct PublicIdentity {
    /// The 32-character hex identity hash.
    pub hash: String,
    /// The raw 32-byte Ed25519 verifying (public) key.
    pub pubkey: [u8; 32],
}

impl PublicIdentity {
    /// Construct from raw Ed25519 verifying key bytes.
    pub fn from_pubkey(pubkey: [u8; 32]) -> Self {
        let digest = Sha256::digest(pubkey);
        let hash = hex::encode(&digest[..IDENTITY_HASH_BYTES]);
        Self { hash, pubkey }
    }

    /// Construct from a hex-encoded pubkey string.
    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {e}"))?;
        let pubkey: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "pubkey must be exactly 32 bytes".to_string())?;
        Ok(Self::from_pubkey(pubkey))
    }

    /// Verify that this pubkey matches a claimed identity hash.
    ///
    /// Recomputes `SHA-256(pubkey)[..16]` and compares against `claimed_hash`.
    /// This is the binding check that prevents an attacker from substituting
    /// a different pubkey for a known identity hash.
    pub fn verify_hash(&self, claimed_hash: &str) -> bool {
        self.hash == claimed_hash
    }

    /// Verify a signature over `data` using this identity's public key.
    pub fn verify(&self, data: &[u8], signature: &[u8; 64]) -> bool {
        identity_verify(&self.pubkey, data, signature)
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
    fn identity_sign_and_verify_roundtrip() {
        let root = test_root(0x42);
        let data = b"test data for signing";
        let (hash, sig) = identity_sign(&root, data);
        let pubkey = identity_pubkey(&root);

        assert_eq!(hash, identity_hash(&root));
        assert!(identity_verify(&pubkey, data, &sig));
    }

    #[test]
    fn identity_verify_rejects_tampered_data() {
        let root = test_root(0x42);
        let (_, sig) = identity_sign(&root, b"original");
        let pubkey = identity_pubkey(&root);
        assert!(!identity_verify(&pubkey, b"tampered", &sig));
    }

    #[test]
    fn identity_verify_rejects_wrong_pubkey() {
        let root_a = test_root(0x01);
        let root_b = test_root(0x02);
        let data = b"signed by A";
        let (_, sig) = identity_sign(&root_a, data);
        let pubkey_b = identity_pubkey(&root_b);
        assert!(!identity_verify(&pubkey_b, data, &sig));
    }

    #[test]
    fn public_identity_from_pubkey() {
        let root = test_root(0x42);
        let pubkey = identity_pubkey(&root);
        let pi = PublicIdentity::from_pubkey(pubkey);
        assert_eq!(pi.hash, identity_hash(&root));
        assert_eq!(pi.pubkey, pubkey);
    }

    #[test]
    fn public_identity_from_hex() {
        let root = test_root(0x42);
        let pubkey = identity_pubkey(&root);
        let hex_str = hex::encode(pubkey);
        let pi = PublicIdentity::from_hex(&hex_str).unwrap();
        assert_eq!(pi.hash, identity_hash(&root));
    }

    #[test]
    fn public_identity_from_hex_rejects_invalid() {
        assert!(PublicIdentity::from_hex("not-hex").is_err());
        assert!(PublicIdentity::from_hex("aabb").is_err()); // too short
    }

    #[test]
    fn public_identity_verify_hash() {
        let root = test_root(0x42);
        let pi = PublicIdentity::from_pubkey(identity_pubkey(&root));
        assert!(pi.verify_hash(&identity_hash(&root)));
        assert!(!pi.verify_hash("0000000000000000000000000000000"));
    }

    #[test]
    fn public_identity_verify_signature() {
        let root = test_root(0x42);
        let data = b"signed data";
        let (_, sig) = identity_sign(&root, data);
        let pi = PublicIdentity::from_pubkey(identity_pubkey(&root));
        assert!(pi.verify(data, &sig));
        assert!(!pi.verify(b"wrong data", &sig));
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
