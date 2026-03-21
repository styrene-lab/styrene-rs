//! HKDF key derivation hierarchy — derives protocol-specific keys from root secret.
//!
//! ```text
//! root_secret (32 bytes)
//!   → HKDF-SHA256(info="styrene-rns-encryption-v1")  → 32 bytes (X25519)
//!   → HKDF-SHA256(info="styrene-rns-signing-v1")     → 32 bytes (Ed25519 seed)
//!   → HKDF-SHA256(info="styrene-yggdrasil-v1")       → 32 bytes (Ed25519)
//!   → HKDF-SHA256(info="styrene-wireguard-v1")       → 32 bytes (Curve25519)
//! ```

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

/// Key derivation purpose — maps to HKDF info strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyPurpose {
    /// RNS X25519 encryption key.
    RnsEncryption,
    /// RNS Ed25519 signing key.
    RnsSigning,
    /// Yggdrasil Ed25519 key.
    Yggdrasil,
    /// WireGuard Curve25519 key.
    WireGuard,
}

impl KeyPurpose {
    /// HKDF info string for this purpose.
    pub fn info(&self) -> &'static [u8] {
        match self {
            Self::RnsEncryption => b"styrene-rns-encryption-v1",
            Self::RnsSigning => b"styrene-rns-signing-v1",
            Self::Yggdrasil => b"styrene-yggdrasil-v1",
            Self::WireGuard => b"styrene-wireguard-v1",
        }
    }

    /// All defined purposes.
    pub fn all() -> &'static [KeyPurpose] {
        &[
            Self::RnsEncryption,
            Self::RnsSigning,
            Self::Yggdrasil,
            Self::WireGuard,
        ]
    }
}

/// Derive a 32-byte key for a specific purpose from the root secret.
///
/// Uses HKDF-SHA256 with no salt (the root secret is already high-entropy).
pub fn derive_key(root_secret: &[u8; 32], purpose: KeyPurpose) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, root_secret);
    let mut okm = [0u8; 32];
    hk.expand(purpose.info(), &mut okm)
        .expect("HKDF-SHA256 expand to 32 bytes should never fail");
    okm
}

/// All derived keys from a root secret.
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct DerivedKeys {
    /// RNS X25519 encryption key (32 bytes).
    pub rns_encryption: [u8; 32],
    /// RNS Ed25519 signing key seed (32 bytes).
    pub rns_signing: [u8; 32],
    /// Yggdrasil Ed25519 key (32 bytes).
    pub yggdrasil: [u8; 32],
    /// WireGuard Curve25519 private key (32 bytes).
    pub wireguard: [u8; 32],
}

/// Derive all protocol keys from a root secret.
pub fn derive_keys(root_secret: &[u8; 32]) -> DerivedKeys {
    DerivedKeys {
        rns_encryption: derive_key(root_secret, KeyPurpose::RnsEncryption),
        rns_signing: derive_key(root_secret, KeyPurpose::RnsSigning),
        yggdrasil: derive_key(root_secret, KeyPurpose::Yggdrasil),
        wireguard: derive_key(root_secret, KeyPurpose::WireGuard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_deterministic() {
        let root = [42u8; 32];
        let k1 = derive_key(&root, KeyPurpose::RnsEncryption);
        let k2 = derive_key(&root, KeyPurpose::RnsEncryption);
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_purposes_produce_different_keys() {
        let root = [42u8; 32];
        let enc = derive_key(&root, KeyPurpose::RnsEncryption);
        let sig = derive_key(&root, KeyPurpose::RnsSigning);
        let ygg = derive_key(&root, KeyPurpose::Yggdrasil);
        let wg = derive_key(&root, KeyPurpose::WireGuard);

        assert_ne!(enc, sig);
        assert_ne!(enc, ygg);
        assert_ne!(enc, wg);
        assert_ne!(sig, ygg);
        assert_ne!(sig, wg);
        assert_ne!(ygg, wg);
    }

    #[test]
    fn different_roots_produce_different_keys() {
        let k1 = derive_key(&[1u8; 32], KeyPurpose::RnsEncryption);
        let k2 = derive_key(&[2u8; 32], KeyPurpose::RnsEncryption);
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_keys_produces_all_four() {
        let root = [99u8; 32];
        let keys = derive_keys(&root);
        // All should be non-zero and distinct
        assert_ne!(keys.rns_encryption, [0u8; 32]);
        assert_ne!(keys.rns_signing, [0u8; 32]);
        assert_ne!(keys.yggdrasil, [0u8; 32]);
        assert_ne!(keys.wireguard, [0u8; 32]);
        assert_ne!(keys.rns_encryption, keys.rns_signing);
    }

    #[test]
    fn all_purposes_covered() {
        assert_eq!(KeyPurpose::all().len(), 4);
    }
}
