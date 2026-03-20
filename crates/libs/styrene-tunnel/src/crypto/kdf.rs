//! Hybrid KDF combining X25519 and ML-KEM shared secrets.
//!
//! Derives session keys via HKDF-SHA256 over the concatenation of both
//! shared secrets, providing security even if one primitive is broken.
//!
//! Output is role-bound: initiator and responder get distinct confirmation
//! tags, preventing reflection attacks.

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::error::TunnelError;

/// Session key size (AES-256-GCM key).
pub const SESSION_KEY_SIZE: usize = 32;
/// Confirmation tag size.
pub const CONFIRM_TAG_SIZE: usize = 32;

/// Total HKDF output: session_key + initiator_confirm + responder_confirm.
const KDF_OUTPUT_SIZE: usize = SESSION_KEY_SIZE + CONFIRM_TAG_SIZE + CONFIRM_TAG_SIZE;

/// Output of the hybrid KDF: a session key and role-bound confirmation tags.
///
/// The initiator and responder tags are cryptographically distinct, preventing
/// reflection attacks where a captured responder confirmation could be replayed
/// as an initiator confirmation.
pub struct HybridKeyMaterial {
    /// AES-256-GCM session key (32 bytes).
    session_key: [u8; SESSION_KEY_SIZE],
    /// Confirmation tag for the initiator to prove key knowledge (32 bytes).
    initiator_confirm_tag: [u8; CONFIRM_TAG_SIZE],
    /// Confirmation tag for the responder to prove key knowledge (32 bytes).
    responder_confirm_tag: [u8; CONFIRM_TAG_SIZE],
}

impl HybridKeyMaterial {
    pub fn session_key(&self) -> &[u8; SESSION_KEY_SIZE] {
        &self.session_key
    }

    /// Tag that the initiator encrypts to prove key possession.
    pub fn initiator_confirm_tag(&self) -> &[u8; CONFIRM_TAG_SIZE] {
        &self.initiator_confirm_tag
    }

    /// Tag that the responder encrypts to prove key possession.
    pub fn responder_confirm_tag(&self) -> &[u8; CONFIRM_TAG_SIZE] {
        &self.responder_confirm_tag
    }
}

impl Drop for HybridKeyMaterial {
    fn drop(&mut self) {
        self.session_key.zeroize();
        self.initiator_confirm_tag.zeroize();
        self.responder_confirm_tag.zeroize();
    }
}

/// Hybrid KDF combining X25519 ECDH and ML-KEM-768 shared secrets.
pub struct HybridKdf;

impl HybridKdf {
    /// Derive session key material from X25519 and ML-KEM shared secrets.
    ///
    /// The IKM is `x25519_shared_secret || mlkem_shared_secret` (64 bytes).
    /// HKDF-SHA256 extracts and expands to produce:
    /// - 32-byte session key (shared, for data encryption)
    /// - 32-byte initiator confirmation tag (role-bound)
    /// - 32-byte responder confirmation tag (role-bound)
    ///
    /// # Arguments
    /// * `x25519_shared` - X25519 Diffie-Hellman shared secret (32 bytes)
    /// * `mlkem_shared` - ML-KEM-768 shared secret (32 bytes)
    /// * `session_id` - 16-byte session identifier, used as HKDF salt
    /// * `info` - Context info for domain separation (e.g., b"styrene-pqc-v1")
    pub fn derive(
        x25519_shared: &[u8; 32],
        mlkem_shared: &[u8; 32],
        session_id: &[u8],
        info: &[u8],
    ) -> Result<HybridKeyMaterial, TunnelError> {
        // Concatenate both shared secrets as IKM
        let mut ikm = [0u8; 64];
        ikm[..32].copy_from_slice(x25519_shared);
        ikm[32..].copy_from_slice(mlkem_shared);

        let hk = Hkdf::<Sha256>::new(Some(session_id), &ikm);

        // Derive 96 bytes: 32 session key + 32 initiator confirm + 32 responder confirm
        let mut okm = [0u8; KDF_OUTPUT_SIZE];
        hk.expand(info, &mut okm).map_err(|_| TunnelError::Crypto("HKDF expand failed".into()))?;

        let mut session_key = [0u8; SESSION_KEY_SIZE];
        let mut initiator_confirm_tag = [0u8; CONFIRM_TAG_SIZE];
        let mut responder_confirm_tag = [0u8; CONFIRM_TAG_SIZE];

        session_key.copy_from_slice(&okm[..SESSION_KEY_SIZE]);
        initiator_confirm_tag
            .copy_from_slice(&okm[SESSION_KEY_SIZE..SESSION_KEY_SIZE + CONFIRM_TAG_SIZE]);
        responder_confirm_tag.copy_from_slice(&okm[SESSION_KEY_SIZE + CONFIRM_TAG_SIZE..]);

        // Zeroize intermediates
        ikm.zeroize();
        okm.zeroize();

        Ok(HybridKeyMaterial { session_key, initiator_confirm_tag, responder_confirm_tag })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_kdf_produces_different_keys_for_different_sessions() {
        let x25519_shared = [0xAA; 32];
        let mlkem_shared = [0xBB; 32];

        let km1 = HybridKdf::derive(&x25519_shared, &mlkem_shared, &[1; 16], b"styrene-pqc-v1")
            .expect("derive 1");
        let km2 = HybridKdf::derive(&x25519_shared, &mlkem_shared, &[2; 16], b"styrene-pqc-v1")
            .expect("derive 2");

        assert_ne!(km1.session_key(), km2.session_key());
        assert_ne!(km1.initiator_confirm_tag(), km2.initiator_confirm_tag());
        assert_ne!(km1.responder_confirm_tag(), km2.responder_confirm_tag());
    }

    #[test]
    fn hybrid_kdf_deterministic() {
        let x25519_shared = [0xAA; 32];
        let mlkem_shared = [0xBB; 32];
        let sid = [0xCC; 16];

        let km1 = HybridKdf::derive(&x25519_shared, &mlkem_shared, &sid, b"styrene-pqc-v1")
            .expect("derive 1");
        let km2 = HybridKdf::derive(&x25519_shared, &mlkem_shared, &sid, b"styrene-pqc-v1")
            .expect("derive 2");

        assert_eq!(km1.session_key(), km2.session_key());
        assert_eq!(km1.initiator_confirm_tag(), km2.initiator_confirm_tag());
        assert_eq!(km1.responder_confirm_tag(), km2.responder_confirm_tag());
    }

    #[test]
    fn confirm_tags_are_role_bound() {
        let x25519_shared = [0xAA; 32];
        let mlkem_shared = [0xBB; 32];
        let sid = [0xCC; 16];

        let km = HybridKdf::derive(&x25519_shared, &mlkem_shared, &sid, b"styrene-pqc-v1")
            .expect("derive");

        // Initiator and responder confirm tags MUST differ
        assert_ne!(
            km.initiator_confirm_tag(),
            km.responder_confirm_tag(),
            "role-bound tags must be distinct to prevent reflection attacks"
        );
    }
}
