//! PQC session state machine.
//!
//! Implements the 3-message handshake protocol:
//! 1. Initiator → Responder: PqcInitiate (X25519 public + ML-KEM ek)
//! 2. Responder → Initiator: PqcRespond (X25519 public + ML-KEM ct + encrypted confirm)
//! 3. Initiator → Responder: PqcConfirm (encrypted confirm)
//!
//! After handshake completes, both sides have a shared session key derived
//! from both X25519 and ML-KEM shared secrets via HKDF-SHA256.
//!
//! ## Security properties
//!
//! - **Role-bound confirmations:** Initiator and responder encrypt distinct
//!   confirm tags with distinct nonces, preventing reflection attacks.
//! - **Domain-separated nonces:** Confirm nonces (`[0xFF;4]` prefix) never
//!   overlap with data nonces (`[0x00;4]` prefix), preventing nonce reuse.
//! - **Sliding window replay:** 64-packet window per RFC 4303 §3.4.3,
//!   tolerating out-of-order delivery common in mesh networks.
//! - **Authenticated close:** Close messages from established sessions are
//!   encrypted, preventing unauthenticated session teardown.

use rand_core::{OsRng, RngCore};
use zeroize::Zeroize;

use crate::crypto::aead::{SessionCipher, CONFIRM_ROLE_INITIATOR, CONFIRM_ROLE_RESPONDER};
use crate::crypto::kdf::{HybridKdf, SESSION_KEY_SIZE};
use crate::crypto::kem::{MlKemEncapsulated, MlKemKeyPair};
use crate::error::TunnelError;
use styrene_mesh::pqc::*;

/// Domain separation string for HKDF.
const KDF_INFO: &[u8] = b"styrene-pqc-v1";

/// Current PQC protocol version.
pub const PQC_VERSION: u8 = 1;

/// Maximum sequence number before mandatory rekey.
const MAX_SEQUENCE: u64 = u64::MAX - 1;

/// Size of the sliding replay window (bits).
const REPLAY_WINDOW_SIZE: u64 = 64;

/// Magic prefix for authenticated close payloads inside PqcData frames.
/// 8 bytes that are extremely unlikely to appear in normal data.
const CLOSE_MAGIC: &[u8; 8] = b"\xFFSTYCLOS";

/// PQC session states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Initial state, no handshake started.
    Idle,
    /// Initiator has sent PqcInitiate, waiting for PqcRespond.
    Initiating,
    /// Responder has sent PqcRespond, waiting for PqcConfirm.
    Responding,
    /// Handshake complete, session keys established.
    Established,
    /// Rekey in progress.
    Rekeying,
    /// Session closed (terminal state).
    Closed,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Initiating => write!(f, "Initiating"),
            Self::Responding => write!(f, "Responding"),
            Self::Established => write!(f, "Established"),
            Self::Rekeying => write!(f, "Rekeying"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// A PQC session between two peers.
///
/// Manages the handshake state machine and provides encrypt/decrypt
/// operations once the session is established.
pub struct PqcSession {
    /// 16-byte session identifier.
    session_id: [u8; 16],
    /// Current state.
    state: SessionState,
    /// Our RNS identity hash (16 bytes).
    our_identity_hash: [u8; 16],
    /// Peer's RNS identity hash (16 bytes), set during handshake.
    peer_identity_hash: Option<[u8; 16]>,
    /// Session cipher for data encryption (set after handshake).
    cipher: Option<SessionCipher>,
    /// Session key bytes (for deriving tunnel PSK).
    session_key: Option<[u8; SESSION_KEY_SIZE]>,
    /// Initiator's confirmation tag (for handshake verification).
    initiator_confirm_tag: Option<[u8; 32]>,
    /// Responder's confirmation tag (for handshake verification).
    responder_confirm_tag: Option<[u8; 32]>,
    /// Our X25519 ephemeral public key (set during initiation).
    our_x25519_public: Option<[u8; 32]>,
    /// ML-KEM keypair (set during initiation, consumed during response).
    mlkem_keypair: Option<MlKemKeyPair>,
    /// Our X25519 ephemeral secret (consumed during key derivation).
    /// Stored as raw bytes since EphemeralSecret can't be cloned.
    our_x25519_secret_bytes: Option<[u8; 32]>,
    /// Send sequence counter.
    send_sequence: u64,
    /// Replay window: highest sequence number seen.
    replay_window_top: u64,
    /// Replay window: bitmask of seen sequences relative to
    /// `replay_window_top - REPLAY_WINDOW_SIZE + 1`.
    replay_window: u64,
    /// Current ratchet step (used during rekey operations).
    #[allow(dead_code)] // planned for rekey implementation
    ratchet_step: u64,
}

impl PqcSession {
    /// Create a new idle session.
    pub fn new(our_identity_hash: [u8; 16]) -> Self {
        Self {
            session_id: [0u8; 16],
            state: SessionState::Idle,
            our_identity_hash,
            peer_identity_hash: None,
            cipher: None,
            session_key: None,
            initiator_confirm_tag: None,
            responder_confirm_tag: None,
            our_x25519_public: None,
            mlkem_keypair: None,
            our_x25519_secret_bytes: None,
            send_sequence: 0,
            replay_window_top: 0,
            replay_window: 0,
            ratchet_step: 0,
        }
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &[u8; 16] {
        &self.session_id
    }

    /// Get the current session state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// Get the peer's identity hash, if known.
    pub fn peer_identity_hash(&self) -> Option<&[u8; 16]> {
        self.peer_identity_hash.as_ref()
    }

    /// Get the session key bytes (for deriving tunnel PSK).
    ///
    /// Only available after the session is established.
    pub fn session_key(&self) -> Option<&[u8; SESSION_KEY_SIZE]> {
        self.session_key.as_ref()
    }

    /// Check if the session is established and can encrypt/decrypt data.
    pub fn is_established(&self) -> bool {
        self.state == SessionState::Established
    }

    // ── Initiator side ──────────────────────────────────────────────────────

    /// Begin the handshake as the initiator.
    ///
    /// Returns the PqcInitiatePayload to send to the responder.
    pub fn initiate(&mut self) -> Result<PqcInitiatePayload, TunnelError> {
        if self.state != SessionState::Idle {
            return Err(TunnelError::InvalidState {
                expected: "Idle",
                actual: self.state.to_string(),
            });
        }

        // Generate session ID
        OsRng.fill_bytes(&mut self.session_id);

        // Generate X25519 ephemeral keypair from random bytes.
        // We use StaticSecret (not EphemeralSecret) because the secret must be
        // stored across the initiate() → process_respond() boundary.
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        self.our_x25519_secret_bytes = Some(secret_bytes);

        let static_secret = x25519_dalek::StaticSecret::from(secret_bytes);
        let public_key = x25519_dalek::PublicKey::from(&static_secret);
        self.our_x25519_public = Some(public_key.to_bytes());

        // Generate ML-KEM keypair
        let mlkem_kp = MlKemKeyPair::generate(&mut OsRng);
        let ek_bytes = mlkem_kp.encapsulation_key_bytes();
        self.mlkem_keypair = Some(mlkem_kp);

        self.state = SessionState::Initiating;

        Ok(PqcInitiatePayload {
            version: PQC_VERSION,
            session_id: self.session_id.to_vec(),
            x25519_public: self.our_x25519_public.expect("just set").to_vec(),
            mlkem_encapsulation_key: ek_bytes,
            identity_hash: self.our_identity_hash.to_vec(),
        })
    }

    /// Process the responder's PqcRespond message (initiator side).
    ///
    /// Verifies the responder's confirmation and returns PqcConfirmPayload.
    pub fn process_respond(
        &mut self,
        respond: &PqcRespondPayload,
    ) -> Result<PqcConfirmPayload, TunnelError> {
        if self.state != SessionState::Initiating {
            return Err(TunnelError::InvalidState {
                expected: "Initiating",
                actual: self.state.to_string(),
            });
        }

        // Verify session ID
        if respond.session_id.as_slice() != self.session_id.as_slice() {
            return Err(TunnelError::HandshakeFailed("session ID mismatch".into()));
        }

        // Store peer identity
        if respond.identity_hash.len() != 16 {
            return Err(TunnelError::InvalidKeyMaterial(
                "peer identity hash must be 16 bytes".into(),
            ));
        }
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(&respond.identity_hash);
        self.peer_identity_hash = Some(peer_hash);

        // Perform X25519 DH
        let secret_bytes = self
            .our_x25519_secret_bytes
            .take()
            .ok_or_else(|| TunnelError::Crypto("X25519 secret already consumed".into()))?;
        let our_secret = x25519_dalek::StaticSecret::from(secret_bytes);
        if respond.x25519_public.len() != 32 {
            return Err(TunnelError::InvalidKeyMaterial(
                "X25519 public key must be 32 bytes".into(),
            ));
        }
        let mut peer_x25519 = [0u8; 32];
        peer_x25519.copy_from_slice(&respond.x25519_public);
        let their_pk = x25519_dalek::PublicKey::from(peer_x25519);
        let x25519_shared = our_secret.diffie_hellman(&their_pk);

        // Decapsulate ML-KEM
        let mlkem_kp = self
            .mlkem_keypair
            .take()
            .ok_or_else(|| TunnelError::Crypto("ML-KEM keypair already consumed".into()))?;
        let mlkem_shared = mlkem_kp.decapsulate(&respond.mlkem_ciphertext)?;

        // Derive hybrid key material (role-bound confirm tags)
        let key_material = HybridKdf::derive(
            x25519_shared.as_bytes(),
            mlkem_shared.as_bytes(),
            &self.session_id,
            KDF_INFO,
        )?;

        // Verify responder's confirmation tag (role = responder)
        let cipher = SessionCipher::new(key_material.session_key());
        let received_tag =
            cipher.decrypt_confirm(&respond.encrypted_confirm, CONFIRM_ROLE_RESPONDER)?;
        if received_tag != *key_material.responder_confirm_tag() {
            return Err(TunnelError::HandshakeFailed("responder confirmation tag mismatch".into()));
        }

        // Create our confirmation (role = initiator, distinct from responder's)
        let our_confirm =
            cipher.encrypt_confirm(key_material.initiator_confirm_tag(), CONFIRM_ROLE_INITIATOR)?;

        // Store session state
        let mut sk = [0u8; SESSION_KEY_SIZE];
        sk.copy_from_slice(key_material.session_key());
        self.session_key = Some(sk);
        self.cipher = Some(cipher);
        self.state = SessionState::Established;

        Ok(PqcConfirmPayload {
            session_id: self.session_id.to_vec(),
            encrypted_confirm: our_confirm,
        })
    }

    // ── Responder side ──────────────────────────────────────────────────────

    /// Process an incoming PqcInitiate message (responder side).
    ///
    /// Returns the PqcRespondPayload to send back.
    pub fn process_initiate(
        &mut self,
        initiate: &PqcInitiatePayload,
    ) -> Result<PqcRespondPayload, TunnelError> {
        if self.state != SessionState::Idle {
            return Err(TunnelError::InvalidState {
                expected: "Idle",
                actual: self.state.to_string(),
            });
        }

        if initiate.version != PQC_VERSION {
            return Err(TunnelError::HandshakeFailed(format!(
                "unsupported PQC version: {}",
                initiate.version
            )));
        }

        // Adopt session ID from initiator
        if initiate.session_id.len() != 16 {
            return Err(TunnelError::InvalidKeyMaterial("session ID must be 16 bytes".into()));
        }
        self.session_id.copy_from_slice(&initiate.session_id);

        // Store peer identity
        if initiate.identity_hash.len() != 16 {
            return Err(TunnelError::InvalidKeyMaterial(
                "peer identity hash must be 16 bytes".into(),
            ));
        }
        let mut peer_hash = [0u8; 16];
        peer_hash.copy_from_slice(&initiate.identity_hash);
        self.peer_identity_hash = Some(peer_hash);

        // Generate our X25519 ephemeral keypair
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        let our_secret = x25519_dalek::StaticSecret::from(secret_bytes);
        let our_public = x25519_dalek::PublicKey::from(&our_secret);
        self.our_x25519_public = Some(our_public.to_bytes());

        // Perform X25519 DH with initiator's public key
        if initiate.x25519_public.len() != 32 {
            return Err(TunnelError::InvalidKeyMaterial(
                "X25519 public key must be 32 bytes".into(),
            ));
        }
        let mut peer_x25519 = [0u8; 32];
        peer_x25519.copy_from_slice(&initiate.x25519_public);
        let their_pk = x25519_dalek::PublicKey::from(peer_x25519);
        let x25519_shared = our_secret.diffie_hellman(&their_pk);

        // Zeroize the secret bytes now that DH is complete
        secret_bytes.zeroize();

        // Encapsulate against initiator's ML-KEM encapsulation key
        let encapsulated =
            MlKemEncapsulated::encapsulate(&initiate.mlkem_encapsulation_key, &mut OsRng)?;

        // Derive hybrid key material (role-bound confirm tags)
        let key_material = HybridKdf::derive(
            x25519_shared.as_bytes(),
            encapsulated.shared_secret.as_bytes(),
            &self.session_id,
            KDF_INFO,
        )?;

        // Encrypt our confirmation tag (role = responder)
        let cipher = SessionCipher::new(key_material.session_key());
        let encrypted_confirm =
            cipher.encrypt_confirm(key_material.responder_confirm_tag(), CONFIRM_ROLE_RESPONDER)?;

        // Store initiator's confirm tag for later verification in process_confirm()
        let mut ict = [0u8; 32];
        ict.copy_from_slice(key_material.initiator_confirm_tag());
        self.initiator_confirm_tag = Some(ict);
        let mut sk = [0u8; SESSION_KEY_SIZE];
        sk.copy_from_slice(key_material.session_key());
        self.session_key = Some(sk);
        self.cipher = Some(cipher);

        self.state = SessionState::Responding;

        Ok(PqcRespondPayload {
            session_id: self.session_id.to_vec(),
            x25519_public: our_public.to_bytes().to_vec(),
            mlkem_ciphertext: encapsulated.ciphertext,
            encrypted_confirm,
            identity_hash: self.our_identity_hash.to_vec(),
        })
    }

    /// Process the initiator's PqcConfirm message (responder side).
    ///
    /// Completes the handshake and transitions to Established.
    pub fn process_confirm(&mut self, confirm: &PqcConfirmPayload) -> Result<(), TunnelError> {
        if self.state != SessionState::Responding {
            return Err(TunnelError::InvalidState {
                expected: "Responding",
                actual: self.state.to_string(),
            });
        }

        // Verify session ID
        if confirm.session_id.as_slice() != self.session_id.as_slice() {
            return Err(TunnelError::HandshakeFailed("session ID mismatch".into()));
        }

        // Verify initiator's confirmation tag (role = initiator)
        let cipher = self
            .cipher
            .as_ref()
            .ok_or_else(|| TunnelError::Crypto("session cipher not initialized".into()))?;
        let received_tag =
            cipher.decrypt_confirm(&confirm.encrypted_confirm, CONFIRM_ROLE_INITIATOR)?;

        let expected_tag = self
            .initiator_confirm_tag
            .as_ref()
            .ok_or_else(|| TunnelError::Crypto("initiator confirm tag not stored".into()))?;

        if received_tag != *expected_tag {
            return Err(TunnelError::HandshakeFailed("initiator confirmation tag mismatch".into()));
        }

        self.initiator_confirm_tag = None; // No longer needed
        self.responder_confirm_tag = None;
        self.state = SessionState::Established;
        Ok(())
    }

    // ── Data encryption/decryption ──────────────────────────────────────────

    /// Encrypt application data for transmission.
    ///
    /// Returns a PqcDataPayload ready for serialization and sending.
    pub fn encrypt_data(&mut self, plaintext: &[u8]) -> Result<PqcDataPayload, TunnelError> {
        if self.state != SessionState::Established {
            return Err(TunnelError::InvalidState {
                expected: "Established",
                actual: self.state.to_string(),
            });
        }

        if self.send_sequence >= MAX_SEQUENCE {
            return Err(TunnelError::Crypto("sequence number exhausted, rekey required".into()));
        }

        let cipher = self
            .cipher
            .as_ref()
            .ok_or_else(|| TunnelError::Crypto("session cipher not initialized".into()))?;

        let sequence = self.send_sequence;
        let ciphertext = cipher.encrypt(sequence, plaintext)?;
        self.send_sequence += 1;

        Ok(PqcDataPayload { session_id: self.session_id.to_vec(), sequence, ciphertext })
    }

    /// Decrypt received application data.
    ///
    /// Uses a 64-packet sliding window for replay protection, tolerating
    /// out-of-order delivery common in mesh networks.
    pub fn decrypt_data(&mut self, data: &PqcDataPayload) -> Result<Vec<u8>, TunnelError> {
        if self.state != SessionState::Established {
            return Err(TunnelError::InvalidState {
                expected: "Established",
                actual: self.state.to_string(),
            });
        }

        // Verify session ID
        if data.session_id.as_slice() != self.session_id.as_slice() {
            return Err(TunnelError::SessionNotFound { session_id: hex::encode(&data.session_id) });
        }

        // Sliding window replay check (RFC 4303 §3.4.3)
        self.check_replay(data.sequence)?;

        let cipher = self
            .cipher
            .as_ref()
            .ok_or_else(|| TunnelError::Crypto("session cipher not initialized".into()))?;

        let plaintext = cipher.decrypt(data.sequence, &data.ciphertext)?;

        // Only mark as seen AFTER successful decryption (authentication)
        self.mark_seen(data.sequence);

        Ok(plaintext)
    }

    /// Check if a sequence number passes the replay window.
    ///
    /// Does NOT modify state — call [`mark_seen`] after successful decryption.
    fn check_replay(&self, sequence: u64) -> Result<(), TunnelError> {
        if self.replay_window_top == 0 && self.replay_window == 0 {
            // First packet ever — accept anything
            return Ok(());
        }

        if sequence > self.replay_window_top {
            // Ahead of window — always accept (window will slide on mark_seen)
            return Ok(());
        }

        // How far behind the top is this sequence?
        let delta = self.replay_window_top - sequence;

        if delta >= REPLAY_WINDOW_SIZE {
            // Too old — falls outside the window
            return Err(TunnelError::ReplayDetected { sequence });
        }

        // Check if this bit is already set in the window
        let bit = 1u64 << delta;
        if self.replay_window & bit != 0 {
            return Err(TunnelError::ReplayDetected { sequence });
        }

        Ok(())
    }

    /// Mark a sequence number as seen in the replay window.
    ///
    /// Call ONLY after successful decryption (GCM authentication passed).
    fn mark_seen(&mut self, sequence: u64) {
        if self.replay_window_top == 0 && self.replay_window == 0 {
            // First packet: initialize window
            self.replay_window_top = sequence;
            self.replay_window = 1; // bit 0 = the top sequence itself
            return;
        }

        if sequence > self.replay_window_top {
            // Slide the window forward
            let shift = sequence - self.replay_window_top;
            if shift >= REPLAY_WINDOW_SIZE {
                // Entire window is obsolete
                self.replay_window = 1;
            } else {
                self.replay_window <<= shift;
                self.replay_window |= 1; // Mark the new top
            }
            self.replay_window_top = sequence;
        } else {
            // Within window — set the appropriate bit
            let delta = self.replay_window_top - sequence;
            self.replay_window |= 1u64 << delta;
        }
    }

    // ── Session lifecycle ───────────────────────────────────────────────────

    /// Create a close message.
    ///
    /// When the session is established, the close payload is encrypted as a
    /// PqcData frame to prevent unauthenticated teardown. Returns either an
    /// authenticated close (PqcData containing the close payload) or an
    /// unauthenticated PqcClose for pre-established states.
    pub fn close(
        &mut self,
        reason: u8,
        message: Option<String>,
    ) -> Result<CloseAction, TunnelError> {
        if self.state == SessionState::Closed {
            return Err(TunnelError::InvalidState {
                expected: "any non-Closed",
                actual: self.state.to_string(),
            });
        }

        if self.state == SessionState::Established {
            // Authenticated close: encrypt the close reason inside a data frame
            let close_inner = encode_authenticated_close(reason, message.as_deref());
            let data_payload = self.encrypt_data(&close_inner)?;
            self.state = SessionState::Closed;
            Ok(CloseAction::Authenticated(data_payload))
        } else {
            // Pre-established: no shared key, send unauthenticated close
            self.state = SessionState::Closed;
            Ok(CloseAction::Unauthenticated(PqcClosePayload {
                session_id: self.session_id.to_vec(),
                reason,
                message,
            }))
        }
    }

    /// Process a received unauthenticated close message.
    ///
    /// Only accepted when the session is NOT established (no shared key).
    /// Established sessions must use authenticated close via data frames.
    pub fn process_close(&mut self, close: &PqcClosePayload) -> Result<(), TunnelError> {
        if close.session_id.as_slice() != self.session_id.as_slice() {
            return Err(TunnelError::SessionNotFound {
                session_id: hex::encode(&close.session_id),
            });
        }

        if self.state == SessionState::Established {
            return Err(TunnelError::HandshakeFailed(
                "established sessions require authenticated close".into(),
            ));
        }

        self.state = SessionState::Closed;
        Ok(())
    }

    /// Try to interpret a decrypted data frame as an authenticated close.
    ///
    /// After calling `decrypt_data()`, pass the plaintext here to check if
    /// it's a close message. Returns `Some((reason, message))` if so.
    pub fn try_authenticated_close(&mut self, plaintext: &[u8]) -> Option<(u8, Option<String>)> {
        let result = decode_authenticated_close(plaintext)?;
        self.state = SessionState::Closed;
        Some(result)
    }
}

/// Result of a close operation.
#[derive(Debug)]
pub enum CloseAction {
    /// Session was established — close payload is encrypted inside a PqcData frame.
    Authenticated(PqcDataPayload),
    /// Session was not established — close payload is sent as cleartext PqcClose.
    Unauthenticated(PqcClosePayload),
}

/// Encode an authenticated close payload for embedding in a PqcData frame.
///
/// Format: `CLOSE_MAGIC (8) || reason (1) || message_len (2 BE) || message (0..)`
fn encode_authenticated_close(reason: u8, message: Option<&str>) -> Vec<u8> {
    let msg_bytes = message.unwrap_or("").as_bytes();
    let msg_len = msg_bytes.len().min(u16::MAX as usize);

    let mut buf = Vec::with_capacity(8 + 1 + 2 + msg_len);
    buf.extend_from_slice(CLOSE_MAGIC);
    buf.push(reason);
    buf.extend_from_slice(&(msg_len as u16).to_be_bytes());
    buf.extend_from_slice(&msg_bytes[..msg_len]);
    buf
}

/// Try to decode an authenticated close payload from decrypted data.
fn decode_authenticated_close(data: &[u8]) -> Option<(u8, Option<String>)> {
    if data.len() < 8 + 1 + 2 {
        return None;
    }
    if &data[..8] != CLOSE_MAGIC.as_slice() {
        return None;
    }
    let reason = data[8];
    let msg_len = u16::from_be_bytes([data[9], data[10]]) as usize;
    let message = if msg_len > 0 && data.len() >= 11 + msg_len {
        String::from_utf8(data[11..11 + msg_len].to_vec()).ok()
    } else {
        None
    };
    Some((reason, message))
}

impl Drop for PqcSession {
    fn drop(&mut self) {
        if let Some(ref mut key) = self.session_key {
            key.zeroize();
        }
        if let Some(ref mut tag) = self.initiator_confirm_tag {
            tag.zeroize();
        }
        if let Some(ref mut tag) = self.responder_confirm_tag {
            tag.zeroize();
        }
        if let Some(ref mut bytes) = self.our_x25519_secret_bytes {
            bytes.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity_hash(val: u8) -> [u8; 16] {
        [val; 16]
    }

    #[test]
    fn full_handshake_roundtrip() {
        let mut initiator = PqcSession::new(test_identity_hash(0xAA));
        let mut responder = PqcSession::new(test_identity_hash(0xBB));

        // Step 1: Initiator creates PqcInitiate
        let initiate = initiator.initiate().expect("initiate");
        assert_eq!(initiator.state(), SessionState::Initiating);

        // Step 2: Responder processes PqcInitiate, returns PqcRespond
        let respond = responder.process_initiate(&initiate).expect("process_initiate");
        assert_eq!(responder.state(), SessionState::Responding);

        // Step 3: Initiator processes PqcRespond, returns PqcConfirm
        let confirm = initiator.process_respond(&respond).expect("process_respond");
        assert_eq!(initiator.state(), SessionState::Established);

        // Step 4: Responder processes PqcConfirm
        responder.process_confirm(&confirm).expect("process_confirm");
        assert_eq!(responder.state(), SessionState::Established);

        // Both sides should have the same session key
        assert_eq!(
            initiator.session_key().expect("initiator key"),
            responder.session_key().expect("responder key")
        );

        // Both sides should know each other's identity
        assert_eq!(initiator.peer_identity_hash().expect("peer hash"), &test_identity_hash(0xBB));
        assert_eq!(responder.peer_identity_hash().expect("peer hash"), &test_identity_hash(0xAA));
    }

    #[test]
    fn confirmation_is_not_reflectable() {
        let mut initiator = PqcSession::new(test_identity_hash(0xAA));
        let mut responder = PqcSession::new(test_identity_hash(0xBB));

        let initiate = initiator.initiate().expect("initiate");
        let respond = responder.process_initiate(&initiate).expect("process_initiate");
        let confirm = initiator.process_respond(&respond).expect("process_respond");

        // The initiator's encrypted_confirm (in PqcConfirm) must differ from
        // the responder's encrypted_confirm (in PqcRespond) — they use
        // different role nonces and different confirm tags.
        assert_ne!(
            respond.encrypted_confirm, confirm.encrypted_confirm,
            "responder and initiator confirmations must differ (anti-reflection)"
        );
    }

    #[test]
    fn reflected_confirm_is_rejected() {
        let mut initiator = PqcSession::new(test_identity_hash(0xAA));
        let mut responder = PqcSession::new(test_identity_hash(0xBB));

        let initiate = initiator.initiate().expect("initiate");
        let respond = responder.process_initiate(&initiate).expect("process_initiate");

        // Attacker tries to reflect the responder's encrypted_confirm as
        // the initiator's PqcConfirm message
        let forged_confirm = PqcConfirmPayload {
            session_id: respond.session_id.clone(),
            encrypted_confirm: respond.encrypted_confirm.clone(), // reflected!
        };

        let result = responder.process_confirm(&forged_confirm);
        assert!(result.is_err(), "reflected confirmation must be rejected");
    }

    #[test]
    fn data_encryption_roundtrip() {
        let (mut initiator, mut responder) = establish_session();

        let plaintext = b"hello from initiator";
        let data = initiator.encrypt_data(plaintext).expect("encrypt");
        let decrypted = responder.decrypt_data(&data).expect("decrypt");
        assert_eq!(&decrypted, plaintext);

        // And the other direction
        let plaintext2 = b"hello from responder";
        let data2 = responder.encrypt_data(plaintext2).expect("encrypt");
        let decrypted2 = initiator.decrypt_data(&data2).expect("decrypt");
        assert_eq!(&decrypted2, plaintext2);
    }

    #[test]
    fn replay_detection_exact_replay() {
        let (mut initiator, mut responder) = establish_session();

        let data = initiator.encrypt_data(b"first").expect("encrypt");
        responder.decrypt_data(&data).expect("decrypt first");

        // Replaying the exact same message should fail
        let result = responder.decrypt_data(&data);
        assert!(matches!(result, Err(TunnelError::ReplayDetected { .. })));
    }

    #[test]
    fn replay_window_accepts_out_of_order() {
        let (mut initiator, mut responder) = establish_session();

        // Send packets 0, 1, 2 but deliver 0, 2, 1 (out of order)
        let pkt0 = initiator.encrypt_data(b"pkt0").expect("encrypt 0");
        let pkt1 = initiator.encrypt_data(b"pkt1").expect("encrypt 1");
        let pkt2 = initiator.encrypt_data(b"pkt2").expect("encrypt 2");

        responder.decrypt_data(&pkt0).expect("pkt0 should pass");
        responder.decrypt_data(&pkt2).expect("pkt2 should pass (ahead of window)");
        responder.decrypt_data(&pkt1).expect("pkt1 should pass (within window)");
    }

    #[test]
    fn replay_window_rejects_too_old() {
        let (mut initiator, mut responder) = establish_session();

        // Encrypt 65 packets (0..64)
        let mut packets = Vec::new();
        for i in 0..65u64 {
            let data = format!("pkt{}", i);
            packets.push(initiator.encrypt_data(data.as_bytes()).expect("encrypt"));
        }

        // Deliver packet 0 first
        responder.decrypt_data(&packets[0]).expect("pkt0");

        // Deliver packet 64 — this slides the window so 0 is now outside
        responder.decrypt_data(&packets[64]).expect("pkt64");

        // Packet 0 is now too old (delta = 64 >= REPLAY_WINDOW_SIZE)
        // It was already accepted though, so try an undelivered old packet
        // Packet 1 should be outside the window now (delta = 63, but bit was never set)
        // Actually delta = 64 - 1 = 63 which is still inside window for a 64-bit window
        // Let's test packet that's definitely outside
        // With top=64, window_size=64, anything < 64-64+1 = 1 is outside
        // So packet 0 was already seen (bit set), and is at delta=64 which is >= 64
        let result = responder.decrypt_data(&packets[0]);
        assert!(result.is_err(), "packet far outside window should be rejected");
    }

    #[test]
    fn replay_window_rejects_duplicate_within_window() {
        let (mut initiator, mut responder) = establish_session();

        let pkt0 = initiator.encrypt_data(b"pkt0").expect("encrypt 0");
        let pkt1 = initiator.encrypt_data(b"pkt1").expect("encrypt 1");
        let pkt2 = initiator.encrypt_data(b"pkt2").expect("encrypt 2");

        responder.decrypt_data(&pkt0).expect("pkt0");
        responder.decrypt_data(&pkt2).expect("pkt2");
        responder.decrypt_data(&pkt1).expect("pkt1");

        // Now try to replay pkt1 — it's within the window but already seen
        let result = responder.decrypt_data(&pkt1);
        assert!(matches!(result, Err(TunnelError::ReplayDetected { .. })));
    }

    #[test]
    fn authenticated_close_from_established() {
        let (mut initiator, mut responder) = establish_session();

        // Close from established state should produce authenticated close
        let close_action =
            initiator.close(close_reason::NORMAL, Some("done".into())).expect("close");
        assert_eq!(initiator.state(), SessionState::Closed);

        match close_action {
            CloseAction::Authenticated(data_payload) => {
                // Responder decrypts the data frame
                let plaintext = responder.decrypt_data(&data_payload).expect("decrypt close");
                // Interpret as close
                let (reason, message) =
                    responder.try_authenticated_close(&plaintext).expect("should be close");
                assert_eq!(reason, close_reason::NORMAL);
                assert_eq!(message.as_deref(), Some("done"));
                assert_eq!(responder.state(), SessionState::Closed);
            }
            CloseAction::Unauthenticated(_) => {
                panic!("established session must produce authenticated close");
            }
        }
    }

    #[test]
    fn unauthenticated_close_rejected_when_established() {
        let (mut _initiator, mut responder) = establish_session();

        // An attacker sends an unauthenticated close to an established session
        let forged_close = PqcClosePayload {
            session_id: responder.session_id().to_vec(),
            reason: close_reason::ERROR,
            message: Some("forged".into()),
        };

        let result = responder.process_close(&forged_close);
        assert!(result.is_err(), "established session must reject unauthenticated close");
        // Session should still be established
        assert_eq!(responder.state(), SessionState::Established);
    }

    #[test]
    fn unauthenticated_close_accepted_pre_established() {
        let mut initiator = PqcSession::new(test_identity_hash(0xAA));
        let mut responder = PqcSession::new(test_identity_hash(0xBB));

        let initiate = initiator.initiate().expect("initiate");
        let _respond = responder.process_initiate(&initiate).expect("process_initiate");

        // Initiator is in Initiating state — unauthenticated close is OK
        let close_action = initiator.close(close_reason::TIMEOUT, None).expect("close");
        match close_action {
            CloseAction::Unauthenticated(close_payload) => {
                responder.process_close(&close_payload).expect("process close");
                assert_eq!(responder.state(), SessionState::Closed);
            }
            CloseAction::Authenticated(_) => {
                panic!("pre-established session should produce unauthenticated close");
            }
        }
    }

    #[test]
    fn close_rejects_already_closed() {
        let (mut initiator, mut _responder) = establish_session();

        initiator.close(close_reason::NORMAL, None).expect("first close");
        let result = initiator.close(close_reason::NORMAL, None);
        assert!(
            matches!(result, Err(TunnelError::InvalidState { .. })),
            "double close should fail"
        );
    }

    #[test]
    fn wrong_state_errors() {
        let mut session = PqcSession::new(test_identity_hash(0xAA));

        // Can't process respond when idle
        let fake_respond = PqcRespondPayload {
            session_id: vec![0; 16],
            x25519_public: vec![0; 32],
            mlkem_ciphertext: vec![0; 1088],
            encrypted_confirm: vec![0; 60],
            identity_hash: vec![0; 16],
        };
        assert!(matches!(
            session.process_respond(&fake_respond),
            Err(TunnelError::InvalidState { .. })
        ));

        // Can't encrypt data when idle
        assert!(matches!(session.encrypt_data(b"data"), Err(TunnelError::InvalidState { .. })));
    }

    #[test]
    fn normal_data_not_misinterpreted_as_close() {
        let (mut initiator, mut responder) = establish_session();

        let data = initiator.encrypt_data(b"normal data").expect("encrypt");
        let plaintext = responder.decrypt_data(&data).expect("decrypt");

        assert!(
            responder.try_authenticated_close(&plaintext).is_none(),
            "normal data must not be interpreted as close"
        );
        assert_eq!(responder.state(), SessionState::Established);
    }

    /// Helper to create an established session pair.
    fn establish_session() -> (PqcSession, PqcSession) {
        let mut initiator = PqcSession::new(test_identity_hash(0xAA));
        let mut responder = PqcSession::new(test_identity_hash(0xBB));

        let initiate = initiator.initiate().expect("initiate");
        let respond = responder.process_initiate(&initiate).expect("process_initiate");
        let confirm = initiator.process_respond(&respond).expect("process_respond");
        responder.process_confirm(&confirm).expect("process_confirm");

        (initiator, responder)
    }
}
