//! Protocol-specific key formatting.
//!
//! Converts raw 32-byte derived keys (from [`KeyDeriver`](crate::KeyDeriver))
//! into the string formats that protocols actually consume: OpenSSH
//! authorized_keys lines, WireGuard base64 keys, age Bech32 identities,
//! and git signing config snippets.

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::pubkey::{ed25519_verifying_key, x25519_public_key};

// ---------------------------------------------------------------------------
// SSH (Ed25519)
// ---------------------------------------------------------------------------

/// Build the SSH wire-format blob for an Ed25519 public key.
///
/// Wire format (RFC 4253): `[u32 len]["ssh-ed25519"][u32 len][32-byte pubkey]`
/// All lengths are big-endian.
fn ssh_wire_pubkey(seed: &[u8; 32]) -> Vec<u8> {
    let vk = ed25519_verifying_key(seed);
    let pubkey_bytes = vk.to_bytes();
    let key_type = b"ssh-ed25519";

    let mut wire = Vec::with_capacity(4 + key_type.len() + 4 + pubkey_bytes.len());
    wire.extend_from_slice(&(key_type.len() as u32).to_be_bytes());
    wire.extend_from_slice(key_type);
    wire.extend_from_slice(&(pubkey_bytes.len() as u32).to_be_bytes());
    wire.extend_from_slice(&pubkey_bytes);
    wire
}

/// Returns an OpenSSH authorized_keys line for the Ed25519 key derived from
/// `seed`.
///
/// Format: `ssh-ed25519 AAAA... comment\n`
pub fn ssh_pubkey(seed: &[u8; 32], comment: &str) -> String {
    let wire = ssh_wire_pubkey(seed);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&wire);
    format!("ssh-ed25519 {b64} {comment}\n")
}

/// Returns the `SHA256:...` fingerprint of the Ed25519 public key derived
/// from `seed`, matching the output of `ssh-keygen -l`.
///
/// The fingerprint is the base64 (no padding) of the SHA-256 hash of the
/// wire-format public key blob.
pub fn ssh_pubkey_fingerprint(seed: &[u8; 32]) -> String {
    let wire = ssh_wire_pubkey(seed);
    let hash = Sha256::digest(&wire);
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash);
    format!("SHA256:{b64}")
}

// ---------------------------------------------------------------------------
// WireGuard
// ---------------------------------------------------------------------------

/// Returns the base64-encoded WireGuard private key (standard format).
///
/// The caller owns the secret and is responsible for zeroizing it when done.
pub fn wireguard_privkey(secret: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(secret)
}

/// Returns the base64-encoded WireGuard public key (X25519) derived from
/// `secret`.
pub fn wireguard_pubkey(secret: &[u8; 32]) -> String {
    let pk = x25519_public_key(secret);
    base64::engine::general_purpose::STANDARD.encode(pk.as_bytes())
}

// ---------------------------------------------------------------------------
// age (Bech32) — gated on `age-format` feature
// ---------------------------------------------------------------------------

/// Returns the age secret identity string: `AGE-SECRET-KEY-1...`
///
/// Bech32 encoding with HRP `age-secret-key-`, uppercase per the age spec.
#[cfg(feature = "age-format")]
pub fn age_identity(secret: &[u8; 32]) -> String {
    bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("AGE-SECRET-KEY-").unwrap(), secret)
        .unwrap()
        .to_uppercase()
}

/// Returns the age recipient string: `age1...`
///
/// Bech32 encoding of the X25519 public key with HRP `age`, lowercase per
/// the age spec.
#[cfg(feature = "age-format")]
pub fn age_recipient(secret: &[u8; 32]) -> String {
    let pk = x25519_public_key(secret);
    bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("age").unwrap(), pk.as_bytes()).unwrap()
}

// ---------------------------------------------------------------------------
// Git config
// ---------------------------------------------------------------------------

/// Returns a git config snippet that configures SSH-based commit signing
/// using the Ed25519 key derived from `seed`.
///
/// ```text
/// [gpg]
///     format = ssh
/// [user]
///     signingkey = key::ssh-ed25519 AAAA...
/// [commit]
///     gpgsign = true
/// ```
pub fn git_signing_config(seed: &[u8; 32]) -> String {
    let wire = ssh_wire_pubkey(seed);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&wire);
    format!(
        "[gpg]\n    format = ssh\n[user]\n    signingkey = key::ssh-ed25519 {b64}\n[commit]\n    gpgsign = true\n"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SEED: [u8; 32] = [0x42u8; 32];

    // -- SSH --

    #[test]
    fn ssh_pubkey_format() {
        let pk = ssh_pubkey(&TEST_SEED, "test@example");
        assert!(pk.starts_with("ssh-ed25519 "));
        assert!(pk.ends_with("test@example\n"));
    }

    #[test]
    fn ssh_pubkey_deterministic() {
        let a = ssh_pubkey(&TEST_SEED, "c");
        let b = ssh_pubkey(&TEST_SEED, "c");
        assert_eq!(a, b);
    }

    #[test]
    fn ssh_fingerprint_format() {
        let fp = ssh_pubkey_fingerprint(&TEST_SEED);
        assert!(fp.starts_with("SHA256:"));
    }

    // -- WireGuard --

    #[test]
    fn wireguard_privkey_base64_length() {
        let pk = wireguard_privkey(&TEST_SEED);
        assert_eq!(pk.len(), 44); // 32 bytes -> 44 base64 chars with padding
        // verify it decodes back
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&pk)
            .unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn wireguard_pubkey_differs_from_privkey() {
        let priv_b64 = wireguard_privkey(&TEST_SEED);
        let pub_b64 = wireguard_pubkey(&TEST_SEED);
        assert_ne!(priv_b64, pub_b64);
    }

    // -- Git config --

    #[test]
    fn git_config_contents() {
        let config = git_signing_config(&TEST_SEED);
        assert!(config.contains("format = ssh"));
        assert!(config.contains("signingkey = key::ssh-ed25519"));
        assert!(config.contains("[gpg]"));
        assert!(config.contains("[commit]"));
        assert!(config.contains("gpgsign = true"));
    }

    // -- Pinned test vectors for seed [0x42; 32] --

    #[test]
    fn pinned_ssh_pubkey() {
        let pk = ssh_pubkey(&TEST_SEED, "test@example");
        assert_eq!(
            pk,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICFS+NGbeR0kRTJC4V8uq2y3z/p7al7TAJeWDgaYgdsS test@example\n"
        );
    }

    #[test]
    fn pinned_ssh_fingerprint() {
        let fp = ssh_pubkey_fingerprint(&TEST_SEED);
        assert_eq!(fp, "SHA256:ZsrOVCtcb1bouzun0GIHz5vL5oCjVhVIQ3jfIBIgZ8g");
    }

    #[test]
    fn pinned_wireguard_pubkey() {
        let pk = wireguard_pubkey(&TEST_SEED);
        assert_eq!(pk, "EyxEK+AQ+9V+cmAzKKp25x/MwVA6riGTJ9FNnJmT9HI=");
    }

    // -- age (only under age-format feature) --

    #[cfg(feature = "age-format")]
    mod age_tests {
        use super::*;

        #[test]
        fn age_identity_format() {
            let id = age_identity(&TEST_SEED);
            assert!(id.starts_with("AGE-SECRET-KEY-1"));
        }

        #[test]
        fn age_recipient_format() {
            let r = age_recipient(&TEST_SEED);
            assert!(r.starts_with("age1"));
        }
    }
}
