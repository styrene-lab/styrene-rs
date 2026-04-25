//! HKDF key derivation hierarchy — derives protocol-specific keys from root secret.
//!
//! Uses a two-tier structure:
//! - **Flat purposes** (fixed info strings) for protocol keys
//! - **Two-level HKDF** for parameterized families (SSH user keys, agent keys)
//!
//! ```text
//! root_secret (32 bytes)
//!   HKDF-Extract(salt="styrene-identity-v1", IKM=root_secret) = PRK
//!     → Expand(PRK, "styrene-rns-encryption-v1")     → RNS X25519 (32 bytes)
//!     → Expand(PRK, "styrene-rns-signing-v1")         → RNS Ed25519 seed (32 bytes)
//!     → Expand(PRK, "styrene-yggdrasil-v1")           → Yggdrasil Ed25519 (32 bytes)
//!     → Expand(PRK, "styrene-wireguard-v1")           → WireGuard Curve25519 (32 bytes)
//!     → Expand(PRK, "styrene-ssh-host-v1")            → SSH host Ed25519 (32 bytes)
//!     → Expand(PRK, "styrene-age-v1")                 → age X25519 (32 bytes)
//!     → Expand(PRK, "styrene-git-signing-v1")         → git commit signing Ed25519 (32 bytes)
//!     → Expand(PRK, "styrene-ssh-user-master-v1")     → SSH user master (32 bytes)
//!         → Expand(master_PRK, label)                 → per-label SSH key (32 bytes)
//!     → Expand(PRK, "styrene-agent-master-v1")        → agent master (32 bytes)
//!         → Expand(master_PRK, agent_name)            → per-agent signing key (32 bytes)
//! ```

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

/// Error from parameterized key derivation (agent keys, SSH user keys).
#[derive(Debug, thiserror::Error)]
pub enum DeriveError {
    /// The name/label parameter was empty.
    #[error("key derivation label must not be empty")]
    EmptyLabel,
}

/// Validate an agent name or SSH user key label before derivation.
///
/// Returns `Ok(())` if the label is valid, `Err(DeriveError)` with a
/// descriptive error otherwise. Use this at config-load time to catch
/// invalid labels before they reach `derive_agent_key()` or
/// `derive_ssh_user_key()`.
pub fn validate_label(label: &str) -> Result<(), DeriveError> {
    if label.is_empty() {
        return Err(DeriveError::EmptyLabel);
    }
    Ok(())
}

/// Fixed domain-separation salt for HKDF-Extract.
///
/// Provides source-independent extraction per RFC 5869 §3.1, ensuring
/// Styrene Identity derivations cannot collide with any other HKDF usage
/// in the system (e.g. RNS DH-derived keys) even if the same IKM were
/// accidentally reused.
/// Fixed domain-separation salt for the root-level HKDF-Extract.
const HKDF_SALT: &[u8] = b"styrene-identity-v1";
/// Level-2 salt for the agent key derivation tree.
const HKDF_SALT_AGENT: &[u8] = b"styrene-identity-agent-v1";
/// Level-2 salt for the SSH user key derivation tree.
const HKDF_SALT_SSH_USER: &[u8] = b"styrene-identity-ssh-user-v1";

/// Key derivation purpose — maps to HKDF info strings for flat (non-parameterized) keys.
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
    /// SSH host Ed25519 key.
    SshHost,
    /// age X25519 encryption key.
    Age,
    /// Git commit signing Ed25519 key (user's personal signing key).
    GitSigning,
}

impl KeyPurpose {
    /// HKDF info string for this purpose.
    pub fn info(&self) -> &'static [u8] {
        match self {
            Self::RnsEncryption => b"styrene-rns-encryption-v1",
            Self::RnsSigning => b"styrene-rns-signing-v1",
            Self::Yggdrasil => b"styrene-yggdrasil-v1",
            Self::WireGuard => b"styrene-wireguard-v1",
            Self::SshHost => b"styrene-ssh-host-v1",
            Self::Age => b"styrene-age-v1",
            Self::GitSigning => b"styrene-git-signing-v1",
        }
    }

    /// All defined purposes.
    pub fn all() -> &'static [KeyPurpose] {
        &[
            Self::RnsEncryption,
            Self::RnsSigning,
            Self::Yggdrasil,
            Self::WireGuard,
            Self::SshHost,
            Self::Age,
            Self::GitSigning,
        ]
    }
}

/// Cached HKDF pseudo-random key with zeroize-on-drop.
///
/// Runs HKDF-Extract once at construction with the fixed domain-separation
/// salt, stores the 32-byte PRK, and reconstructs the HKDF expander on
/// each derive call. The PRK is root-equivalent key material and is
/// zeroized when the `KeyDeriver` is dropped.
///
/// For parameterized key families (SSH user keys), use
/// [`derive_ssh_user_key`](Self::derive_ssh_user_key) which performs a
/// second-level HKDF derivation to avoid info-string collision risks.
pub struct KeyDeriver {
    /// The pseudo-random key extracted from the root secret.
    /// Zeroized on drop — this is root-equivalent material.
    prk: [u8; 32],
}

impl Drop for KeyDeriver {
    fn drop(&mut self) {
        self.prk.zeroize();
    }
}

impl KeyDeriver {
    /// Create from a root secret. Runs HKDF-Extract once and stores the PRK.
    pub fn new(root_secret: &[u8; 32]) -> Self {
        // Extract PRK directly — no intermediate Hkdf struct is stored,
        // avoiding non-zeroizable copies of root-equivalent material.
        let (prk_hmac, _) = Hkdf::<Sha256>::extract(Some(HKDF_SALT), root_secret);
        let mut prk_bytes = [0u8; 32];
        prk_bytes.copy_from_slice(prk_hmac.as_slice());
        Self { prk: prk_bytes }
    }

    /// Reconstruct the HKDF expander from stored PRK bytes.
    fn expander(&self) -> Hkdf<Sha256> {
        Hkdf::<Sha256>::from_prk(&self.prk).expect("32-byte PRK is always valid for HKDF-SHA256")
    }

    /// Derive a 32-byte key for a flat (non-parameterized) purpose.
    pub fn derive(&self, purpose: KeyPurpose) -> [u8; 32] {
        let mut okm = [0u8; 32];
        self.expander()
            .expand(purpose.info(), &mut okm)
            .expect("HKDF-SHA256 expand to 32 bytes should never fail");
        okm
    }

    /// Derive all core protocol keys.
    pub fn derive_all(&self) -> DerivedKeys {
        DerivedKeys {
            rns_encryption: self.derive(KeyPurpose::RnsEncryption),
            rns_signing: self.derive(KeyPurpose::RnsSigning),
            yggdrasil: self.derive(KeyPurpose::Yggdrasil),
            wireguard: self.derive(KeyPurpose::WireGuard),
        }
    }

    /// Derive SSH host Ed25519 seed (32 bytes).
    pub fn ssh_host_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::SshHost)
    }

    /// Derive age X25519 private key (32 bytes).
    pub fn age_secret(&self) -> [u8; 32] {
        self.derive(KeyPurpose::Age)
    }

    /// Derive git commit signing Ed25519 seed (32 bytes).
    ///
    /// This is the user's personal git signing key. Agent-specific signing
    /// keys are derived separately via [`derive_agent_key`](Self::derive_agent_key).
    pub fn git_signing_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::GitSigning)
    }

    /// Derive a per-agent Ed25519 signing seed via two-level HKDF.
    ///
    /// Level 1: `Expand(PRK, "styrene-agent-master-v1")` → master key.
    /// Level 2: `Expand(master_PRK, agent_name)` → per-agent key.
    ///
    /// Agent names are freeform identifiers (e.g., `"omegon-primary"`,
    /// `"omegon-cleave-0"`, `"auspex-deploy"`). The two-level structure
    /// ensures no collision with flat purposes or SSH user keys.
    ///
    /// These keys can be used for git commit signing (`gpg.format = ssh`),
    /// allowing cryptographic distinction between user-authored and
    /// agent-authored commits while all keys trace back to the same root.
    pub fn derive_agent_key(&self, agent_name: &str) -> Result<[u8; 32], DeriveError> {
        if agent_name.is_empty() {
            return Err(DeriveError::EmptyLabel);
        }

        let mut master = [0u8; 32];
        self.expander()
            .expand(b"styrene-agent-master-v1", &mut master)
            .expect("HKDF expand should not fail");

        let hk2 = Hkdf::<Sha256>::new(Some(HKDF_SALT_AGENT), &master);
        master.zeroize();

        let mut okm = [0u8; 32];
        hk2.expand(agent_name.as_bytes(), &mut okm).expect("HKDF expand should not fail");
        Ok(okm)
    }

    /// Derive a per-label SSH user Ed25519 seed via two-level HKDF.
    ///
    /// Level 1: `Expand(PRK, "styrene-ssh-user-master-v1")` → master key.
    /// Level 2: `Expand(master_PRK, label)` → per-label key.
    ///
    /// This structure makes collisions with flat-namespace purposes
    /// structurally impossible — per-label keys live in a completely
    /// separate HKDF tree rooted at the master.
    pub fn derive_ssh_user_key(&self, label: &str) -> Result<[u8; 32], DeriveError> {
        if label.is_empty() {
            return Err(DeriveError::EmptyLabel);
        }

        // Level 1: derive SSH user master key
        let mut master = [0u8; 32];
        self.expander()
            .expand(b"styrene-ssh-user-master-v1", &mut master)
            .expect("HKDF expand should not fail");

        // Level 2: derive per-label key from master (distinct salt from agent tree)
        let hk2 = Hkdf::<Sha256>::new(Some(HKDF_SALT_SSH_USER), &master);
        master.zeroize();

        let mut okm = [0u8; 32];
        hk2.expand(label.as_bytes(), &mut okm).expect("HKDF expand should not fail");
        Ok(okm)
    }
}

/// Derive a 32-byte key for a specific purpose from the root secret.
///
/// Convenience wrapper around [`KeyDeriver`]. For multiple derivations from
/// the same root, prefer constructing a `KeyDeriver` to avoid redundant
/// HKDF-Extract calls.
pub fn derive_key(root_secret: &[u8; 32], purpose: KeyPurpose) -> [u8; 32] {
    KeyDeriver::new(root_secret).derive(purpose)
}

/// All derived keys from a root secret.
///
/// Debug output is redacted — key material is never printed.
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

impl std::fmt::Debug for DerivedKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DerivedKeys([REDACTED])")
    }
}

/// Derive all core protocol keys from a root secret.
///
/// Convenience wrapper around [`KeyDeriver::derive_all`].
pub fn derive_keys(root_secret: &[u8; 32]) -> DerivedKeys {
    KeyDeriver::new(root_secret).derive_all()
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
        let keys: Vec<[u8; 32]> = KeyPurpose::all().iter().map(|p| derive_key(&root, *p)).collect();

        // Every pair must be distinct
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "collision between purposes {i} and {j}");
            }
        }
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
        assert_ne!(keys.rns_encryption, [0u8; 32]);
        assert_ne!(keys.rns_signing, [0u8; 32]);
        assert_ne!(keys.yggdrasil, [0u8; 32]);
        assert_ne!(keys.wireguard, [0u8; 32]);
        assert_ne!(keys.rns_encryption, keys.rns_signing);
    }

    #[test]
    fn all_purposes_covered() {
        assert_eq!(KeyPurpose::all().len(), 7);
    }

    #[test]
    fn key_deriver_matches_free_function() {
        let root = [42u8; 32];
        let deriver = KeyDeriver::new(&root);
        for purpose in KeyPurpose::all() {
            assert_eq!(deriver.derive(*purpose), derive_key(&root, *purpose));
        }
    }

    #[test]
    fn key_deriver_derive_all_matches_individual() {
        let root = [77u8; 32];
        let deriver = KeyDeriver::new(&root);
        let all = deriver.derive_all();
        assert_eq!(all.rns_encryption, deriver.derive(KeyPurpose::RnsEncryption));
        assert_eq!(all.rns_signing, deriver.derive(KeyPurpose::RnsSigning));
        assert_eq!(all.yggdrasil, deriver.derive(KeyPurpose::Yggdrasil));
        assert_eq!(all.wireguard, deriver.derive(KeyPurpose::WireGuard));
    }

    #[test]
    fn ssh_host_and_age_non_zero_and_distinct() {
        let root = [55u8; 32];
        let deriver = KeyDeriver::new(&root);
        let ssh = deriver.ssh_host_seed();
        let age = deriver.age_secret();
        assert_ne!(ssh, [0u8; 32]);
        assert_ne!(age, [0u8; 32]);
        assert_ne!(ssh, age);
    }

    // --- SSH user key (two-level HKDF) tests ---

    #[test]
    fn ssh_user_key_deterministic() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let k1 = d.derive_ssh_user_key("github").unwrap();
        let k2 = d.derive_ssh_user_key("github").unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn ssh_user_key_different_labels() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let github = d.derive_ssh_user_key("github").unwrap();
        let work = d.derive_ssh_user_key("work").unwrap();
        assert_ne!(github, work);
    }

    #[test]
    fn ssh_user_key_no_collision_with_flat_purposes() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let ssh_user = d.derive_ssh_user_key("github").unwrap();

        for purpose in KeyPurpose::all() {
            let flat = d.derive(*purpose);
            assert_ne!(ssh_user, flat, "SSH user key collides with {:?}", purpose);
        }
        assert_ne!(ssh_user, d.ssh_host_seed());
        assert_ne!(ssh_user, d.age_secret());
    }

    #[test]
    fn ssh_user_key_different_roots() {
        let k1 = KeyDeriver::new(&[1u8; 32]).derive_ssh_user_key("github").unwrap();
        let k2 = KeyDeriver::new(&[2u8; 32]).derive_ssh_user_key("github").unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn ssh_user_key_empty_label_rejected() {
        let d = KeyDeriver::new(&[42u8; 32]);
        assert!(d.derive_ssh_user_key("").is_err());
    }

    // --- Agent key (two-level HKDF) tests ---

    #[test]
    fn agent_key_deterministic() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let k1 = d.derive_agent_key("omegon-primary").unwrap();
        let k2 = d.derive_agent_key("omegon-primary").unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn agent_key_different_names() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let primary = d.derive_agent_key("omegon-primary").unwrap();
        let cleave = d.derive_agent_key("omegon-cleave-0").unwrap();
        assert_ne!(primary, cleave);
    }

    #[test]
    fn agent_key_no_collision_with_flat_or_ssh() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let agent = d.derive_agent_key("omegon-primary").unwrap();

        for purpose in KeyPurpose::all() {
            assert_ne!(agent, d.derive(*purpose), "agent key collides with {:?}", purpose);
        }
        assert_ne!(agent, d.derive_ssh_user_key("github").unwrap());
        assert_ne!(agent, d.git_signing_seed());
    }

    #[test]
    fn agent_key_differs_from_ssh_user_same_label() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let ssh = d.derive_ssh_user_key("github").unwrap();
        let agent = d.derive_agent_key("github").unwrap();
        assert_ne!(ssh, agent);
    }

    #[test]
    fn agent_key_empty_name_rejected() {
        let d = KeyDeriver::new(&[42u8; 32]);
        assert!(d.derive_agent_key("").is_err());
    }

    // --- Git signing tests ---

    #[test]
    fn git_signing_distinct_from_all() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let git = d.git_signing_seed();
        assert_ne!(git, [0u8; 32]);
        assert_ne!(git, d.ssh_host_seed());
        assert_ne!(git, d.age_secret());
        assert_ne!(git, d.derive_ssh_user_key("github").unwrap());
        assert_ne!(git, d.derive_agent_key("omegon-primary").unwrap());
    }

    // --- Pinned test vectors (Appendix C of spec) ---

    #[test]
    fn test_vector_flat_purposes() {
        let d = KeyDeriver::new(&[0x42u8; 32]);

        assert_eq!(
            hex::encode(d.derive(KeyPurpose::RnsEncryption)),
            "aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c"
        );
        assert_eq!(
            hex::encode(d.derive(KeyPurpose::GitSigning)),
            "6eb3d3ef12a2447f6de281d6f896eba20ad0b0add3bc6fce80499f36b7343842"
        );
    }

    #[test]
    fn test_vector_ssh_user_key() {
        let d = KeyDeriver::new(&[0x42u8; 32]);
        assert_eq!(
            hex::encode(d.derive_ssh_user_key("github").unwrap()),
            "3c261af80e084a637fd20e0f7274a4106702894f0d23c47e855f6c9adce20d75"
        );
    }

    #[test]
    fn test_vector_agent_key() {
        let d = KeyDeriver::new(&[0x42u8; 32]);
        assert_eq!(
            hex::encode(d.derive_agent_key("omegon-primary").unwrap()),
            "4dd66edcda091a5e3d15aa3fb8ec32d81e212d94760b61915b1d6f204b0672e2"
        );
    }

    #[test]
    fn salt_provides_domain_separation() {
        // Verify that the salted HKDF produces different output than
        // an unsalted one would. This is a one-time migration validation.
        let root = [42u8; 32];
        let salted = Hkdf::<Sha256>::new(Some(HKDF_SALT), &root);
        let unsalted = Hkdf::<Sha256>::new(None, &root);

        let mut s_out = [0u8; 32];
        let mut u_out = [0u8; 32];
        let info = KeyPurpose::RnsEncryption.info();
        salted.expand(info, &mut s_out).expect("expand");
        unsalted.expand(info, &mut u_out).expect("expand");

        assert_ne!(s_out, u_out, "salt must change derived output");
    }
}
