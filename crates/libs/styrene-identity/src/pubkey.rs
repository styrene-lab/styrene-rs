//! Public key derivation and signing from HKDF-derived seeds.
//!
//! Converts 32-byte seeds (from [`KeyDeriver`](crate::KeyDeriver)) into
//! typed Ed25519 and X25519 keys. All private key material is stack-allocated
//! and dropped at end of scope.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

/// Create an Ed25519 signing key from a 32-byte seed.
///
/// The seed is typically from `KeyDeriver::derive(KeyPurpose::RnsSigning)`,
/// `KeyDeriver::ssh_host_seed()`, or `KeyDeriver::derive_ssh_user_key(label)`.
pub fn ed25519_signing_key(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(seed)
}

/// Derive the Ed25519 verifying (public) key from a 32-byte seed.
pub fn ed25519_verifying_key(seed: &[u8; 32]) -> VerifyingKey {
    SigningKey::from_bytes(seed).verifying_key()
}

/// Derive the X25519 public key from a 32-byte private key.
///
/// Used for age encryption keys and RNS/WireGuard encryption.
/// Clamping is applied internally by x25519-dalek per RFC 7748.
pub fn x25519_public_key(secret: &[u8; 32]) -> X25519PublicKey {
    X25519PublicKey::from(&StaticSecret::from(*secret))
}

/// Sign data with an Ed25519 key derived from a seed.
///
/// The signing key exists only for the duration of the call.
pub fn sign_with_seed(seed: &[u8; 32], data: &[u8]) -> [u8; 64] {
    let sk = SigningKey::from_bytes(seed);
    sk.sign(data).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    #[test]
    fn ed25519_sign_and_verify() {
        let seed = [42u8; 32];
        let sk = ed25519_signing_key(&seed);
        let vk = ed25519_verifying_key(&seed);
        let data = b"hello styrene";

        let sig = sk.sign(data);
        assert!(vk.verify(data, &sig).is_ok());
    }

    #[test]
    fn ed25519_wrong_data_fails() {
        let seed = [42u8; 32];
        let vk = ed25519_verifying_key(&seed);
        let sig_bytes = sign_with_seed(&seed, b"correct");
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(vk.verify(b"wrong", &sig).is_err());
    }

    #[test]
    fn sign_with_seed_deterministic() {
        let seed = [42u8; 32];
        let s1 = sign_with_seed(&seed, b"data");
        let s2 = sign_with_seed(&seed, b"data");
        assert_eq!(s1, s2);
    }

    #[test]
    fn sign_with_seed_different_data() {
        let seed = [42u8; 32];
        let s1 = sign_with_seed(&seed, b"alpha");
        let s2 = sign_with_seed(&seed, b"beta");
        assert_ne!(s1, s2);
    }

    #[test]
    fn x25519_public_key_deterministic() {
        let secret = [42u8; 32];
        let pk1 = x25519_public_key(&secret);
        let pk2 = x25519_public_key(&secret);
        assert_eq!(pk1.as_bytes(), pk2.as_bytes());
    }

    #[test]
    fn x25519_different_secrets_different_pubkeys() {
        let pk1 = x25519_public_key(&[1u8; 32]);
        let pk2 = x25519_public_key(&[2u8; 32]);
        assert_ne!(pk1.as_bytes(), pk2.as_bytes());
    }

    #[test]
    fn x25519_public_key_non_zero() {
        let pk = x25519_public_key(&[42u8; 32]);
        assert_ne!(pk.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn verifying_key_matches_signing_key() {
        let seed = [99u8; 32];
        let sk = ed25519_signing_key(&seed);
        let vk = ed25519_verifying_key(&seed);
        assert_eq!(sk.verifying_key(), vk);
    }
}
