//! Styrene Identity — signing trait and HKDF key derivation hierarchy.
//!
//! Provides the [`IdentitySigner`] trait with four implementation tiers:
//! - **Tier A**: HardwareHsm (YubiKey PIV/FIDO2, non-exportable)
//! - **Tier B**: DeviceHsm (iOS Secure Enclave, Android Keystore)
//! - **Tier C**: CredentialManager (Bitwarden/1Password SSH key item)
//! - **Tier D**: EncryptedFile (argon2id + ChaCha20Poly1305, default)
//!
//! All tiers feed the same HKDF root-secret derivation hierarchy:
//! ```text
//! root_secret (32 bytes)
//!   → HKDF-SHA256("styrene-rns-encryption-v1")  → RNS X25519 encryption key
//!   → HKDF-SHA256("styrene-rns-signing-v1")     → RNS Ed25519 signing key
//!   → HKDF-SHA256("styrene-yggdrasil-v1")       → Yggdrasil Ed25519 key
//!   → HKDF-SHA256("styrene-wireguard-v1")       → WireGuard Curve25519 key
//! ```

pub mod derive;
pub mod signer;
#[cfg(feature = "file-signer")]
pub mod file_signer;

pub use derive::{DerivedKeys, KeyPurpose, derive_key, derive_keys};
pub use signer::{IdentitySigner, SignerError, SignerTier};
