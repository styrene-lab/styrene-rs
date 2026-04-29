//! Deterministic key hierarchy for Styrene mesh nodes.
//!
//! One 32-byte root secret derives all protocol-specific keys — RNS,
//! Yggdrasil, WireGuard, SSH, age, git signing, and per-agent delegation
//! — via HKDF-SHA256 with domain separation.
//!
//! # Usage
//!
//! ```rust
//! use styrene_identity::derive::{KeyDeriver, KeyPurpose};
//!
//! let root_secret = [0x42u8; 32]; // in practice, from a signer
//! let deriver = KeyDeriver::new(&root_secret);
//!
//! // Flat-purpose keys (7 protocols)
//! let git_seed = deriver.derive(KeyPurpose::GitSigning);
//! let age_key  = deriver.derive(KeyPurpose::Age);
//!
//! // Parameterized keys (two-level HKDF, structurally collision-free)
//! let github_ssh = deriver.derive_ssh_user_key("github").unwrap();
//! let agent_key  = deriver.derive_agent_key("omegon-primary").unwrap();
//! ```
//!
//! # Signer tiers
//!
//! The [`IdentitySigner`] trait abstracts over four storage backends.
//! All tiers produce the same root secret — they are different access
//! paths to the same identity.
//!
//! | Tier | Backend | Feature |
//! |------|---------|---------|
//! | A | YubiKey FIDO2 hmac-secret | `yubikey` |
//! | B | Platform secure element | — (planned) |
//! | C | Credential manager (Bitwarden, Keychain) | — (planned) |
//! | D | Encrypted file (argon2id + ChaCha20Poly1305) | `file-signer` (default) |
//!
//! [`SignerChain`] tries signers in tier order (A→D), using the first available.
//!
//! # Feature flags
//!
//! | Feature | Default | Enables |
//! |---------|---------|---------|
//! | `file-signer` | **yes** | `FileSigner`, `IdentityVault` |
//! | `signing` | via file-signer | `pubkey` module (ed25519, x25519) |
//! | `yubikey` | no | `YubiKeySigner` (FIDO2 hmac-secret) |
//! | `ssh-agent` | no | `StyreneAgent` (SSH agent protocol) |
//!
//! # Derivation hierarchy
//!
//! ```text
//! root_secret (32 bytes)
//!   HKDF-Extract(salt="styrene-identity-v1", IKM=root_secret) = PRK
//!   │
//!   ├─ Expand("styrene-rns-encryption-v1")  → RNS X25519
//!   ├─ Expand("styrene-rns-signing-v1")     → RNS Ed25519 (canonical identity)
//!   ├─ Expand("styrene-yggdrasil-v1")       → Yggdrasil Ed25519
//!   ├─ Expand("styrene-wireguard-v1")       → WireGuard Curve25519
//!   ├─ Expand("styrene-ssh-host-v1")        → SSH host Ed25519
//!   ├─ Expand("styrene-age-v1")             → age X25519
//!   ├─ Expand("styrene-git-signing-v1")     → git signing Ed25519
//!   │
//!   ├─ SSH user keys (two-level, salt="styrene-identity-ssh-user-v1")
//!   │   └─ Expand(label) → per-host SSH Ed25519
//!   │
//!   └─ Agent keys (two-level, salt="styrene-identity-agent-v1")
//!       └─ Expand(name) → per-agent signing Ed25519
//! ```
//!
//! # Linkability warning
//!
//! **All keys derived from one root are cryptographically linked.** This is
//! by design for attribution and recovery, but it means derived keys cannot
//! provide anonymity or unlinkability. If you need an identity that cannot be
//! traced to your primary identity, use [`ephemeral()`](signer::RootSecret::ephemeral) or a
//! separate identity file. See `docs/unlinkability.md` for the full model.
//!
//! ```rust
//! use styrene_identity::signer::RootSecret;
//!
//! // Anonymous: independent CSPRNG root, no link to any persistent identity
//! let anon = RootSecret::ephemeral();
//! ```
//!
//! # Security
//!
//! - All secret material is zeroized on drop ([`RootSecret`], [`KeyDeriver`], [`DerivedKeys`])
//! - Passphrases and PINs are provided via traits, never environment variables
//! - File creation uses `O_EXCL` (no TOCTOU race)
//! - argon2id params exceed OWASP minimums (m=64MiB, t=3, p=1)
//!
//! [`IdentitySigner`]: signer::IdentitySigner
//! [`SignerChain`]: signer::SignerChain
//! [`RootSecret`]: signer::RootSecret
//! [`KeyDeriver`]: derive::KeyDeriver
//! [`DerivedKeys`]: derive::DerivedKeys

pub mod derive;
pub mod discover;
#[cfg(feature = "file-signer")]
pub mod file_signer;
#[cfg(feature = "signing")]
pub mod format;
#[cfg(feature = "signing")]
pub mod identity;
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
pub use discover::{discover, DiscoveredIdentity};
#[cfg(feature = "signing")]
pub use identity::{identity_hash, identity_pubkey, IdentityInfo, IDENTITY_HASH_BYTES};
pub use signer::{IdentitySigner, SignerChain, SignerError, SignerTier};
