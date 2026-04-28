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
//!   HKDF-Extract(salt="styrene-identity-v1", IKM=root_secret) = PRK
//!     → Expand(PRK, "styrene-rns-encryption-v1")     → RNS X25519
//!     → Expand(PRK, "styrene-rns-signing-v1")         → RNS Ed25519 seed
//!     → Expand(PRK, "styrene-yggdrasil-v1")           → Yggdrasil Ed25519
//!     → Expand(PRK, "styrene-wireguard-v1")           → WireGuard Curve25519
//!     → Expand(PRK, "styrene-ssh-host-v1")            → SSH host Ed25519
//!     → Expand(PRK, "styrene-age-v1")                 → age X25519
//!     → Expand(PRK, "styrene-git-signing-v1")         → git commit signing Ed25519
//!     → Expand(PRK, "styrene-ssh-user-master-v1")     → SSH user master
//!         → Expand(master_PRK, label)                 → per-label SSH key
//!     → Expand(PRK, "styrene-agent-master-v1")        → agent master
//!         → Expand(master_PRK, agent_name)            → per-agent signing key
//! ```

pub mod derive;
#[cfg(feature = "file-signer")]
pub mod file_signer;
#[cfg(feature = "signing")]
pub mod pubkey;
pub mod signer;
#[cfg(feature = "ssh-agent")]
pub mod ssh_agent;
#[cfg(feature = "file-signer")]
pub mod vault;
#[cfg(feature = "yubikey")]
pub mod yubikey_signer;

pub use derive::{
    derive_key, derive_keys, validate_label, DeriveError, DerivedKeys, KeyDeriver, KeyPurpose,
};
pub use signer::{IdentitySigner, SignerChain, SignerError, SignerTier};
