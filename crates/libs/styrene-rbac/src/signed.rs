//! Hub-signed roster entries — portable, cryptographically-verified role bindings.
//!
//! A `SignedRosterEntry` is a `RosterEntry` signed by a trusted hub's Ed25519 key.
//! Nodes verify the signature against a list of trusted hub public keys before
//! accepting the entry into their local RBAC policy.
//!
//! # Signing format
//!
//! Canonical bytes: `"styrene-roster-v1\n" + JSON(entry_fields) + "\nissued_at:" + issued_at`
//!
//! The signature covers the identity_hash, role, label, grants, and issued_at —
//! binding the role assignment to a specific identity and point in time.

#[cfg(feature = "config")]
use serde::{Deserialize, Serialize};

use crate::policy::RosterEntry;

/// Version prefix for canonical signing format.
const CANONICAL_VERSION: &str = "styrene-roster-v1";

/// A roster entry signed by a trusted hub's Ed25519 key.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
pub struct SignedRosterEntry {
    /// The roster entry being attested.
    pub entry: RosterEntry,
    /// Identity hash of the signing hub.
    pub hub_hash: String,
    /// Hub's Ed25519 public key (64 hex chars = 32 bytes).
    pub hub_pubkey: String,
    /// Ed25519 signature over canonical entry bytes (128 hex chars = 64 bytes).
    pub signature: String,
    /// When this entry was issued (Unix timestamp).
    pub issued_at: i64,
    /// When this entry expires (Unix timestamp, 0 = no expiry).
    #[cfg_attr(feature = "config", serde(default))]
    pub expires_at: i64,
}

/// A hub whose Ed25519 public key is trusted to sign roster entries.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
pub struct TrustedHub {
    /// Identity hash of the hub (32 hex chars).
    pub hub_hash: String,
    /// Ed25519 public key (64 hex chars = 32 bytes).
    pub hub_pubkey: String,
    /// Human-readable label (e.g., "signum.styrene.io").
    #[cfg_attr(feature = "config", serde(default))]
    pub label: String,
}

impl SignedRosterEntry {
    /// Build the canonical byte representation for signing/verification.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let entry_json = format!(
            r#"{{"identity_hash":"{}","role":"{}","label":"{}","grants":[{}]}}"#,
            self.entry.identity_hash.to_ascii_lowercase(),
            self.entry.role.as_str(),
            self.entry.label,
            self.entry.grants().iter().map(|g| format!(r#""{g}""#)).collect::<Vec<_>>().join(","),
        );
        format!("{CANONICAL_VERSION}\n{entry_json}\nissued_at:{}", self.issued_at).into_bytes()
    }

    /// Check whether the entry has expired.
    pub fn is_expired(&self, now_unix: i64) -> bool {
        self.expires_at > 0 && now_unix > self.expires_at
    }

    /// Verify the Ed25519 signature against the embedded hub public key.
    #[cfg(feature = "signing")]
    pub fn verify(&self) -> bool {
        let Some(pubkey_bytes) = hex_to_32_bytes(&self.hub_pubkey) else {
            return false;
        };
        let Some(sig_bytes) = hex_to_64_bytes(&self.signature) else {
            return false;
        };
        let Ok(verifying_key) = ed25519_dalek::VerifyingKey::from_bytes(&pubkey_bytes) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        let canonical = self.canonical_bytes();
        use ed25519_dalek::Verifier;
        verifying_key.verify(&canonical, &sig).is_ok()
    }

    /// Sign a roster entry with a hub's Ed25519 signing key.
    #[cfg(feature = "signing")]
    pub fn sign(
        entry: RosterEntry,
        signing_key: &ed25519_dalek::SigningKey,
        issued_at: i64,
        expires_at: i64,
    ) -> Self {
        use ed25519_dalek::Signer;
        use sha2::{Digest, Sha256};

        let hub_pubkey = hex::encode(signing_key.verifying_key().as_bytes());
        let hub_hash = {
            let digest = Sha256::digest(signing_key.verifying_key().as_bytes());
            hex::encode(&digest[..16])
        };
        let mut signed =
            Self { entry, hub_hash, hub_pubkey, signature: String::new(), issued_at, expires_at };
        let canonical = signed.canonical_bytes();
        let sig = signing_key.sign(&canonical);
        signed.signature = hex::encode(sig.to_bytes());
        signed
    }
}

#[cfg(feature = "signing")]
fn hex_to_32_bytes(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str).ok()?;
    bytes.try_into().ok()
}

#[cfg(feature = "signing")]
fn hex_to_64_bytes(hex_str: &str) -> Option<[u8; 64]> {
    let bytes = hex::decode(hex_str).ok()?;
    bytes.try_into().ok()
}

impl TrustedHub {
    /// Check whether a SignedRosterEntry was signed by this hub.
    pub fn matches(&self, entry: &SignedRosterEntry) -> bool {
        self.hub_hash == entry.hub_hash && self.hub_pubkey == entry.hub_pubkey
    }
}

#[cfg(all(test, feature = "signing"))]
mod tests {
    use super::*;
    use crate::{Capability, Role};

    fn test_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[0x42; 32])
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Operator)
            .with_label("alice");
        let signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        assert!(signed.verify());
        assert!(!signed.is_expired(2000));
    }

    #[test]
    fn verify_rejects_tampered_role() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Operator);
        let mut signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        signed.entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Admin);
        assert!(!signed.verify());
    }

    #[test]
    fn verify_rejects_tampered_identity() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Operator);
        let mut signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        signed.entry = RosterEntry::new("bbbb2222cccc3333dddd4444eeee5555", Role::Operator);
        assert!(!signed.verify());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Operator);
        let mut signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        let other_key = ed25519_dalek::SigningKey::from_bytes(&[0x99; 32]);
        signed.hub_pubkey = hex::encode(other_key.verifying_key().as_bytes());
        assert!(!signed.verify());
    }

    #[test]
    fn expiry_check() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Peer);
        let signed = SignedRosterEntry::sign(entry, &key, 1000, 2000);
        assert!(!signed.is_expired(1500));
        assert!(signed.is_expired(2001));
    }

    #[test]
    fn no_expiry_when_zero() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Peer);
        let signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        assert!(!signed.is_expired(999_999_999));
    }

    #[test]
    fn grants_included_in_canonical() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Operator)
            .with_grants(vec![Capability::VPN_HANDSHAKE.to_string()]);
        let signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        assert!(signed.verify());
        let canonical = String::from_utf8(signed.canonical_bytes()).unwrap();
        assert!(canonical.contains("vpn.handshake"));
    }

    #[test]
    fn trusted_hub_matches() {
        let key = test_signing_key();
        let entry = RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", Role::Peer);
        let signed = SignedRosterEntry::sign(entry, &key, 1000, 0);
        let hub = TrustedHub {
            hub_hash: signed.hub_hash.clone(),
            hub_pubkey: signed.hub_pubkey.clone(),
            label: "test-hub".into(),
        };
        assert!(hub.matches(&signed));
        let wrong_hub = TrustedHub {
            hub_hash: "wrong".into(),
            hub_pubkey: signed.hub_pubkey.clone(),
            label: "wrong".into(),
        };
        assert!(!wrong_hub.matches(&signed));
    }
}
