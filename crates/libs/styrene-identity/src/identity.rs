//! Canonical identity hash and info — the unique fingerprint for a Styrene identity.
//!
//! The identity hash is `SHA-256(Ed25519 verifying key)` truncated to 16 bytes,
//! rendered as 32 lowercase hex characters. This is the short, human-readable
//! identifier that every consumer needs but previously had to hand-roll.
//!
//! All intermediate seed material is zeroized after extracting the public key.

use ed25519_dalek::Verifier;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

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
    let seed = Zeroizing::new(deriver.derive(KeyPurpose::Signing));
    let vk = ed25519_verifying_key(&seed);
    vk.to_bytes()
}

/// Bundled identity information — hash and public key together.
///
/// Prefer [`PublicIdentity`] for new code — it provides the same fields
/// plus `verify_hash()` and `verify()` methods.
#[deprecated(since = "0.3.0", note = "use PublicIdentity instead")]
#[derive(Debug, Clone)]
pub struct IdentityInfo {
    /// The 32-character hex identity hash.
    pub hash: String,
    /// The raw 32-byte Ed25519 verifying (public) key.
    pub pubkey: [u8; 32],
}

#[allow(deprecated)]
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

/// Self-contained signed attestation: hash + pubkey + signature.
///
/// Contains everything a verifier needs to check a signature without
/// holding the root secret or fetching the pubkey from a separate channel.
#[derive(Debug, Clone)]
pub struct SignedAttestation {
    /// The 32-character hex identity hash of the signer.
    pub hash: String,
    /// The raw 32-byte Ed25519 verifying (public) key.
    pub pubkey: [u8; 32],
    /// The 64-byte Ed25519 signature over the attested data.
    pub signature: [u8; 64],
}

/// Sign arbitrary data with the identity's Ed25519 signing key.
///
/// Returns a [`SignedAttestation`] containing the identity hash, public key,
/// and signature — everything a verifier needs. All seed material is
/// zeroized after signing via RAII ([`Zeroizing`]).
///
/// This is the canonical way to attest data (profiles, audit records,
/// agent delegations) — consumers should prefer this over manual
/// `KeyDeriver` + `sign_with_seed` sequences.
pub fn identity_sign(root: &RootSecret, data: &[u8]) -> SignedAttestation {
    let deriver = KeyDeriver::new(root.as_bytes());
    let seed = Zeroizing::new(deriver.derive(KeyPurpose::Signing));
    let signature = sign_with_seed(&seed, data);
    let pubkey = identity_pubkey(root);
    let hash = identity_hash(root);
    SignedAttestation { hash, pubkey, signature }
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
        let pubkey: [u8; 32] =
            bytes.try_into().map_err(|_| "pubkey must be exactly 32 bytes".to_string())?;
        Ok(Self::from_pubkey(pubkey))
    }

    /// Verify that this pubkey matches a claimed identity hash.
    ///
    /// Uses constant-time comparison to prevent timing side-channels.
    /// Recomputes `SHA-256(pubkey)[..16]` and compares against `claimed_hash`.
    /// This is the binding check that prevents an attacker from substituting
    /// a different pubkey for a known identity hash.
    pub fn verify_hash(&self, claimed_hash: &str) -> bool {
        let ours = self.hash.as_bytes();
        let theirs = claimed_hash.as_bytes();
        // Length check first (not secret), then constant-time content compare
        ours.len() == theirs.len() && ours.ct_eq(theirs).into()
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

    // ── Hash tests ────────────────────────────────────────────────────────

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
        assert_eq!(h.len(), IDENTITY_HASH_BYTES * 2);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_roots_produce_different_hashes() {
        let h1 = identity_hash(&test_root(0x01));
        let h2 = identity_hash(&test_root(0x02));
        assert_ne!(h1, h2, "different roots must produce different hashes");
    }

    #[test]
    fn identity_hash_no_collisions_in_100_roots() {
        let mut hashes = std::collections::HashSet::new();
        for i in 0u32..100 {
            let mut r = [0u8; 32];
            r[..4].copy_from_slice(&i.to_le_bytes());
            let h = identity_hash(&RootSecret::new(r));
            assert!(hashes.insert(h), "hash collision at iteration {i}");
        }
    }

    #[test]
    fn test_vector_identity_hash() {
        let root = test_root(0x42);
        let h = identity_hash(&root);
        assert_eq!(h, "6279e31aff9bc151638ac305d88ab6bc");
    }

    // ── IdentityInfo (deprecated, backwards compat) ───────────────────────

    #[test]
    #[allow(deprecated)]
    fn identity_info_matches_individual_functions() {
        let root = test_root(0x42);
        let info = IdentityInfo::from_root(&root);
        assert_eq!(info.hash, identity_hash(&root));
        assert_eq!(info.pubkey, identity_pubkey(&root));
    }

    // ── Sign/verify ───────────────────────────────────────────────────────

    #[test]
    fn identity_sign_returns_self_contained_attestation() {
        let root = test_root(0x42);
        let data = b"test data for signing";
        let att = identity_sign(&root, data);

        assert_eq!(att.hash, identity_hash(&root));
        assert_eq!(att.pubkey, identity_pubkey(&root));
        assert!(identity_verify(&att.pubkey, data, &att.signature));
    }

    #[test]
    fn identity_sign_pubkey_can_verify_independently() {
        let root = test_root(0x42);
        let data = b"self-contained verification test";
        let att = identity_sign(&root, data);

        // Verifier has only the attestation — no root secret
        let pi = PublicIdentity::from_pubkey(att.pubkey);
        assert!(pi.verify_hash(&att.hash));
        assert!(pi.verify(data, &att.signature));
    }

    #[test]
    fn identity_verify_rejects_tampered_data() {
        let root = test_root(0x42);
        let att = identity_sign(&root, b"original");
        assert!(!identity_verify(&att.pubkey, b"tampered", &att.signature));
    }

    #[test]
    fn identity_verify_rejects_wrong_pubkey() {
        let root_a = test_root(0x01);
        let root_b = test_root(0x02);
        let data = b"signed by A";
        let att = identity_sign(&root_a, data);
        let pubkey_b = identity_pubkey(&root_b);
        assert!(!identity_verify(&pubkey_b, data, &att.signature));
    }

    #[test]
    fn identity_verify_rejects_zero_pubkey() {
        assert!(!identity_verify(&[0u8; 32], b"data", &[0u8; 64]));
    }

    #[test]
    fn identity_verify_rejects_all_ones_pubkey() {
        assert!(!identity_verify(&[0xFFu8; 32], b"data", &[0u8; 64]));
    }

    // ── PublicIdentity ────────────────────────────────────────────────────

    #[test]
    fn public_identity_from_pubkey() {
        let root = test_root(0x42);
        let pubkey = identity_pubkey(&root);
        let pi = PublicIdentity::from_pubkey(pubkey);
        assert_eq!(pi.hash, identity_hash(&root));
        assert_eq!(pi.pubkey, pubkey);
    }

    #[test]
    fn public_identity_from_hex_valid() {
        let root = test_root(0x42);
        let pubkey = identity_pubkey(&root);
        let hex_str = hex::encode(pubkey);
        let pi = PublicIdentity::from_hex(&hex_str).unwrap();
        assert_eq!(pi.hash, identity_hash(&root));
    }

    #[test]
    fn public_identity_from_hex_edge_cases() {
        // Not hex
        assert!(PublicIdentity::from_hex("not-hex").is_err());
        // Empty
        assert!(PublicIdentity::from_hex("").is_err());
        // Too short (4 bytes)
        assert!(PublicIdentity::from_hex("aabbccdd").is_err());
        // 31 bytes (62 hex chars)
        assert!(PublicIdentity::from_hex(&"aa".repeat(31)).is_err());
        // 33 bytes (66 hex chars)
        assert!(PublicIdentity::from_hex(&"aa".repeat(33)).is_err());
        // Odd length (63 hex chars)
        assert!(PublicIdentity::from_hex(&format!("{}a", "aa".repeat(31))).is_err());
        // Exactly 32 bytes (64 hex chars) — valid
        assert!(PublicIdentity::from_hex(&"aa".repeat(32)).is_ok());
    }

    #[test]
    fn public_identity_verify_hash_constant_time() {
        let root = test_root(0x42);
        let pi = PublicIdentity::from_pubkey(identity_pubkey(&root));
        let correct = identity_hash(&root);

        assert!(pi.verify_hash(&correct));
        assert!(!pi.verify_hash("00000000000000000000000000000000"));
        assert!(!pi.verify_hash("")); // different length
        assert!(!pi.verify_hash("too-short"));
        assert!(!pi.verify_hash(&format!("{correct}extra"))); // longer
    }

    #[test]
    fn public_identity_verify_signature() {
        let root = test_root(0x42);
        let data = b"signed data";
        let att = identity_sign(&root, data);
        let pi = PublicIdentity::from_pubkey(att.pubkey);
        assert!(pi.verify(data, &att.signature));
        assert!(!pi.verify(b"wrong data", &att.signature));
    }

    #[test]
    fn public_identity_cross_key_isolation() {
        let root_a = test_root(0x01);
        let root_b = test_root(0x02);
        let data = b"test";
        let att_a = identity_sign(&root_a, data);
        let pi_b = PublicIdentity::from_pubkey(identity_pubkey(&root_b));

        // Signature from A must NOT verify under B's pubkey
        assert!(!pi_b.verify(data, &att_a.signature));
        // And B's hash must not match A's
        assert!(!pi_b.verify_hash(&att_a.hash));
    }
}
