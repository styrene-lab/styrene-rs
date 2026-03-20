//! AES-256-GCM session encryption and decryption.
//!
//! Nonces are deterministically derived from sequence numbers for replay
//! protection. The nonce spaces are domain-separated:
//!
//! - **Data nonces:** `[0x00; 4] || sequence.to_be_bytes()` — sequence 0..2^64
//! - **Confirm nonces:** `[0xFF; 4] || [0x00; 7] || role` — role is 0x01 (initiator)
//!   or 0x02 (responder)
//!
//! These spaces never overlap, preventing nonce reuse between handshake
//! confirmations and data frames.

use aes_gcm::aead::{Aead, KeyInit, Nonce};
use aes_gcm::Aes256Gcm;

use crate::error::TunnelError;

/// AES-256-GCM authentication tag overhead (16 bytes).
pub const AEAD_TAG_SIZE: usize = 16;
/// AES-256-GCM nonce size (12 bytes).
pub const NONCE_SIZE: usize = 12;

/// Role byte for initiator confirm nonces.
pub const CONFIRM_ROLE_INITIATOR: u8 = 0x01;
/// Role byte for responder confirm nonces.
pub const CONFIRM_ROLE_RESPONDER: u8 = 0x02;

/// AES-256-GCM cipher for session data encryption.
pub struct SessionCipher {
    cipher: Aes256Gcm,
}

impl SessionCipher {
    /// Create a new session cipher from a 32-byte key.
    pub fn new(key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256-GCM key is always 32 bytes");
        Self { cipher }
    }

    /// Encrypt plaintext with a sequence-derived nonce.
    ///
    /// Returns `nonce || ciphertext || tag`.
    pub fn encrypt(&self, sequence: u64, plaintext: &[u8]) -> Result<Vec<u8>, TunnelError> {
        let nonce = Self::nonce_from_sequence(sequence);
        let nonce_ref = Nonce::<Aes256Gcm>::from_slice(&nonce);

        let ciphertext = self
            .cipher
            .encrypt(nonce_ref, plaintext)
            .map_err(|_| TunnelError::Crypto("AES-256-GCM encryption failed".into()))?;

        // Prepend nonce for transmission
        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext that was encrypted with [`encrypt`].
    ///
    /// Input format: `nonce (12) || ciphertext || tag (16)`.
    pub fn decrypt(&self, sequence: u64, data: &[u8]) -> Result<Vec<u8>, TunnelError> {
        if data.len() < NONCE_SIZE + AEAD_TAG_SIZE {
            return Err(TunnelError::DecryptionFailed(format!(
                "ciphertext too short: {} bytes (minimum {})",
                data.len(),
                NONCE_SIZE + AEAD_TAG_SIZE
            )));
        }

        let expected_nonce = Self::nonce_from_sequence(sequence);
        let received_nonce = &data[..NONCE_SIZE];

        // Verify nonce matches expected sequence
        if received_nonce != expected_nonce {
            return Err(TunnelError::DecryptionFailed(
                "nonce does not match expected sequence".into(),
            ));
        }

        let nonce_ref = Nonce::<Aes256Gcm>::from_slice(received_nonce);
        let ciphertext = &data[NONCE_SIZE..];

        self.cipher
            .decrypt(nonce_ref, ciphertext)
            .map_err(|_| TunnelError::DecryptionFailed("AES-256-GCM authentication failed".into()))
    }

    /// Encrypt a confirmation tag (for handshake messages).
    ///
    /// Uses a role-specific nonce in the `[0xFF; 4]` prefix space, which
    /// never overlaps with data nonces (`[0x00; 4]` prefix).
    ///
    /// # Arguments
    /// * `tag` — 32-byte confirmation tag from KDF
    /// * `role` — [`CONFIRM_ROLE_INITIATOR`] or [`CONFIRM_ROLE_RESPONDER`]
    pub fn encrypt_confirm(&self, tag: &[u8; 32], role: u8) -> Result<Vec<u8>, TunnelError> {
        let nonce = Self::nonce_for_confirm(role);
        let nonce_ref = Nonce::<Aes256Gcm>::from_slice(&nonce);

        let ciphertext = self
            .cipher
            .encrypt(nonce_ref, tag.as_slice())
            .map_err(|_| TunnelError::Crypto("confirm tag encryption failed".into()))?;

        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt and verify a confirmation tag.
    ///
    /// # Arguments
    /// * `data` — `nonce (12) || ciphertext || tag (16)`
    /// * `role` — expected role byte (must match what was used to encrypt)
    pub fn decrypt_confirm(&self, data: &[u8], role: u8) -> Result<[u8; 32], TunnelError> {
        if data.len() < NONCE_SIZE + AEAD_TAG_SIZE {
            return Err(TunnelError::DecryptionFailed("confirm ciphertext too short".into()));
        }

        let expected_nonce = Self::nonce_for_confirm(role);
        let received_nonce = &data[..NONCE_SIZE];

        // Verify the nonce matches the expected role
        if received_nonce != expected_nonce.as_slice() {
            return Err(TunnelError::HandshakeFailed(
                "confirm nonce does not match expected role".into(),
            ));
        }

        let nonce_ref = Nonce::<Aes256Gcm>::from_slice(received_nonce);
        let ciphertext = &data[NONCE_SIZE..];

        let plaintext = self.cipher.decrypt(nonce_ref, ciphertext).map_err(|_| {
            TunnelError::HandshakeFailed("confirm tag authentication failed".into())
        })?;

        if plaintext.len() != 32 {
            return Err(TunnelError::HandshakeFailed(format!(
                "confirm tag must be 32 bytes, got {}",
                plaintext.len()
            )));
        }

        let mut tag = [0u8; 32];
        tag.copy_from_slice(&plaintext);
        Ok(tag)
    }

    /// Derive a 12-byte nonce from a sequence number (data plane).
    ///
    /// Format: `[0x00; 4] || sequence.to_be_bytes()`.
    fn nonce_from_sequence(sequence: u64) -> [u8; NONCE_SIZE] {
        let mut nonce = [0u8; NONCE_SIZE];
        // First 4 bytes are 0x00 (data prefix)
        nonce[4..].copy_from_slice(&sequence.to_be_bytes());
        nonce
    }

    /// Derive a 12-byte nonce for confirmation (handshake plane).
    ///
    /// Format: `[0xFF; 4] || [0x00; 7] || role`.
    /// Role is [`CONFIRM_ROLE_INITIATOR`] (0x01) or [`CONFIRM_ROLE_RESPONDER`] (0x02).
    fn nonce_for_confirm(role: u8) -> [u8; NONCE_SIZE] {
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[0] = 0xFF;
        nonce[1] = 0xFF;
        nonce[2] = 0xFF;
        nonce[3] = 0xFF;
        nonce[11] = role;
        nonce
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);

        let plaintext = b"hello, post-quantum world!";
        let ciphertext = cipher.encrypt(0, plaintext).expect("encrypt");
        let decrypted = cipher.decrypt(0, &ciphertext).expect("decrypt");

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn different_sequences_produce_different_ciphertext() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);
        let plaintext = b"same data";

        let ct1 = cipher.encrypt(0, plaintext).expect("encrypt 0");
        let ct2 = cipher.encrypt(1, plaintext).expect("encrypt 1");

        assert_ne!(ct1, ct2);
    }

    #[test]
    fn wrong_sequence_fails_decrypt() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);

        let ciphertext = cipher.encrypt(0, b"data").expect("encrypt");
        let result = cipher.decrypt(1, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn confirm_tag_roundtrip_initiator() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);

        let tag = [0xAB; 32];
        let encrypted =
            cipher.encrypt_confirm(&tag, CONFIRM_ROLE_INITIATOR).expect("encrypt confirm");
        let decrypted =
            cipher.decrypt_confirm(&encrypted, CONFIRM_ROLE_INITIATOR).expect("decrypt confirm");

        assert_eq!(tag, decrypted);
    }

    #[test]
    fn confirm_tag_roundtrip_responder() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);

        let tag = [0xCD; 32];
        let encrypted =
            cipher.encrypt_confirm(&tag, CONFIRM_ROLE_RESPONDER).expect("encrypt confirm");
        let decrypted =
            cipher.decrypt_confirm(&encrypted, CONFIRM_ROLE_RESPONDER).expect("decrypt confirm");

        assert_eq!(tag, decrypted);
    }

    #[test]
    fn confirm_roles_produce_different_ciphertext() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);
        let tag = [0xAB; 32];

        let ct_init =
            cipher.encrypt_confirm(&tag, CONFIRM_ROLE_INITIATOR).expect("encrypt initiator");
        let ct_resp =
            cipher.encrypt_confirm(&tag, CONFIRM_ROLE_RESPONDER).expect("encrypt responder");

        assert_ne!(
            ct_init, ct_resp,
            "same tag encrypted with different roles must produce different ciphertext"
        );
    }

    #[test]
    fn confirm_wrong_role_fails_decrypt() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);
        let tag = [0xAB; 32];

        let encrypted =
            cipher.encrypt_confirm(&tag, CONFIRM_ROLE_INITIATOR).expect("encrypt confirm");
        // Try to decrypt with wrong role
        let result = cipher.decrypt_confirm(&encrypted, CONFIRM_ROLE_RESPONDER);
        assert!(result.is_err(), "decrypting with wrong role must fail");
    }

    #[test]
    fn confirm_nonce_never_collides_with_data_nonce() {
        // Data nonces: [0x00; 4] || sequence
        let data_nonce_0 = SessionCipher::nonce_from_sequence(0);
        let data_nonce_max = SessionCipher::nonce_from_sequence(u64::MAX);

        // Confirm nonces: [0xFF; 4] || [0x00; 7] || role
        let confirm_init = SessionCipher::nonce_for_confirm(CONFIRM_ROLE_INITIATOR);
        let confirm_resp = SessionCipher::nonce_for_confirm(CONFIRM_ROLE_RESPONDER);

        // Data nonces always have first 4 bytes = 0x00
        assert_eq!(data_nonce_0[..4], [0x00; 4]);
        assert_eq!(data_nonce_max[..4], [0x00; 4]);

        // Confirm nonces always have first 4 bytes = 0xFF
        assert_eq!(confirm_init[..4], [0xFF; 4]);
        assert_eq!(confirm_resp[..4], [0xFF; 4]);

        // Therefore they never collide
        assert_ne!(data_nonce_0, confirm_init);
        assert_ne!(data_nonce_0, confirm_resp);
        assert_ne!(data_nonce_max, confirm_init);
        assert_ne!(data_nonce_max, confirm_resp);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [0x42u8; 32];
        let cipher = SessionCipher::new(&key);

        let mut ciphertext = cipher.encrypt(0, b"data").expect("encrypt");
        // Tamper with the ciphertext body (after nonce)
        if let Some(byte) = ciphertext.get_mut(NONCE_SIZE + 1) {
            *byte ^= 0xFF;
        }
        let result = cipher.decrypt(0, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn nonce_encoding() {
        let nonce = SessionCipher::nonce_from_sequence(0);
        assert_eq!(nonce, [0; 12]);

        let nonce = SessionCipher::nonce_from_sequence(1);
        assert_eq!(nonce, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

        let nonce = SessionCipher::nonce_from_sequence(u64::MAX);
        assert_eq!(nonce, [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn confirm_nonce_encoding() {
        let nonce = SessionCipher::nonce_for_confirm(CONFIRM_ROLE_INITIATOR);
        assert_eq!(nonce, [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0x01]);

        let nonce = SessionCipher::nonce_for_confirm(CONFIRM_ROLE_RESPONDER);
        assert_eq!(nonce, [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0x02]);
    }
}
