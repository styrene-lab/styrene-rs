//! HKDF key derivation hierarchy — derives protocol-specific keys from root secret.
//!
//! Uses a two-tier structure:
//! - **Flat purposes** (fixed info strings) for protocol keys
//! - **Two-level HKDF** for parameterized families (SSH user keys, agent keys, etc.)
//!
//! ```text
//! root_secret (32 bytes)
//!   HKDF-Extract(salt="styrene-identity-v1", IKM=root_secret) = PRK
//!
//!     ── THE IDENTITY ──
//!     → Expand(PRK, "styrene-signing-v1")             → Ed25519 seed (THE identity key)
//!
//!     ── ENCRYPTION ──
//!     → Expand(PRK, "styrene-rns-encryption-v1")      → RNS X25519
//!     → Expand(PRK, "styrene-age-v1")                 → age X25519
//!     → Expand(PRK, "styrene-wireguard-v1")           → WireGuard Curve25519
//!
//!     ── DEVICE ──
//!     → Expand(PRK, "styrene-ssh-host-v1")            → SSH host Ed25519
//!
//!     ── OVERLAY TRANSPORTS ──
//!     → Expand(PRK, "styrene-yggdrasil-v1")           → Yggdrasil Ed25519
//!     → Expand(PRK, "styrene-i2p-signing-v1")         → I2P destination Ed25519
//!     → Expand(PRK, "styrene-i2p-encryption-v1")      → I2P destination X25519
//!     → Expand(PRK, "styrene-tor-v1")                 → Tor onion v3 Ed25519
//!
//!     ── PARAMETERIZED FAMILIES ──
//!     → Expand(PRK, "styrene-ssh-user-master-v1")     → SSH user master
//!         → Expand(master_PRK, label)                 → per-host SSH key
//!     → Expand(PRK, "styrene-agent-master-v1")        → agent master
//!         → Expand(master_PRK, agent_name)            → per-agent signing key
//!     → Expand(PRK, "styrene-i2p-service-master-v1")  → I2P service master
//!         → Expand(master_PRK, service_name)          → per-service destination keys
//!     → Expand(PRK, "styrene-onion-master-v1")        → Tor service master
//!         → Expand(master_PRK, service_name)          → per-service onion keys
//! ```

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

/// Error from parameterized key derivation (agent keys, SSH user keys, etc.).
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
const HKDF_SALT: &[u8] = b"styrene-identity-v1";
/// Level-2 salt for the agent key derivation tree.
const HKDF_SALT_AGENT: &[u8] = b"styrene-identity-agent-v1";
/// Level-2 salt for the SSH user key derivation tree.
const HKDF_SALT_SSH_USER: &[u8] = b"styrene-identity-ssh-user-v1";
/// Level-2 salt for the I2P per-service derivation tree.
const HKDF_SALT_I2P_SERVICE: &[u8] = b"styrene-identity-i2p-service-v1";
/// Level-2 salt for the Tor per-service derivation tree.
const HKDF_SALT_ONION_SERVICE: &[u8] = b"styrene-identity-onion-service-v1";

/// Key derivation purpose — maps to HKDF info strings for flat (non-parameterized) keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyPurpose {
    // ── The Identity ──
    /// THE identity signing key (Ed25519).
    /// Used for: mesh signing, git commits, personal attribution.
    /// Identity hash = SHA-256(pubkey) truncated to 16 bytes.
    /// This IS you.
    Signing,

    // ── Encryption (different curves) ──
    /// RNS X25519 encryption key.
    RnsEncryption,
    /// age X25519 encryption key.
    Age,
    /// WireGuard Curve25519 key.
    WireGuard,

    // ── Device ──
    /// SSH host Ed25519 key (identifies the machine, not the person).
    SshHost,

    // ── Overlay transports ──
    /// Yggdrasil Ed25519 key (IPv6 overlay network identity).
    Yggdrasil,
    /// I2P destination Ed25519 signing key.
    I2pSigning,
    /// I2P destination X25519 encryption key.
    I2pEncryption,
    /// Tor onion v3 service Ed25519 key.
    Tor,

    // ── Legacy aliases ──
    // These derive the SAME bytes as their canonical counterparts.
    // Kept for backwards compatibility with existing code that
    // references the old purpose names.
    /// Legacy alias for `Signing`. Derives identical bytes.
    #[deprecated(note = "use KeyPurpose::Signing — RnsSigning and GitSigning are now unified")]
    RnsSigning,
    /// Legacy alias for `Signing`. Derives identical bytes.
    #[deprecated(note = "use KeyPurpose::Signing — RnsSigning and GitSigning are now unified")]
    GitSigning,
}

impl KeyPurpose {
    /// HKDF info string for this purpose.
    pub fn info(&self) -> &'static [u8] {
        match self {
            // THE identity key — uses the RnsSigning info string for backwards
            // compatibility. Existing vaults produce the same derived bytes.
            Self::Signing => b"styrene-rns-signing-v1",

            Self::RnsEncryption => b"styrene-rns-encryption-v1",
            Self::Age => b"styrene-age-v1",
            Self::WireGuard => b"styrene-wireguard-v1",
            Self::SshHost => b"styrene-ssh-host-v1",
            Self::Yggdrasil => b"styrene-yggdrasil-v1",
            Self::I2pSigning => b"styrene-i2p-signing-v1",
            Self::I2pEncryption => b"styrene-i2p-encryption-v1",
            Self::Tor => b"styrene-tor-v1",

            // Legacy aliases — derive the same bytes as Signing
            #[allow(deprecated)]
            Self::RnsSigning => b"styrene-rns-signing-v1",
            #[allow(deprecated)]
            Self::GitSigning => b"styrene-rns-signing-v1",
        }
    }

    /// All current (non-deprecated) purposes.
    pub fn all() -> &'static [KeyPurpose] {
        &[
            Self::Signing,
            Self::RnsEncryption,
            Self::Age,
            Self::WireGuard,
            Self::SshHost,
            Self::Yggdrasil,
            Self::I2pSigning,
            Self::I2pEncryption,
            Self::Tor,
        ]
    }
}

/// Cached HKDF pseudo-random key with zeroize-on-drop.
///
/// Runs HKDF-Extract once at construction with the fixed domain-separation
/// salt, stores the 32-byte PRK, and reconstructs the HKDF expander on
/// each derive call. The PRK is root-equivalent key material and is
/// zeroized when the `KeyDeriver` is dropped.
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

    /// Derive all flat-purpose keys.
    pub fn derive_all(&self) -> DerivedKeys {
        DerivedKeys {
            signing: self.derive(KeyPurpose::Signing),
            rns_encryption: self.derive(KeyPurpose::RnsEncryption),
            age: self.derive(KeyPurpose::Age),
            wireguard: self.derive(KeyPurpose::WireGuard),
            ssh_host: self.derive(KeyPurpose::SshHost),
            yggdrasil: self.derive(KeyPurpose::Yggdrasil),
            i2p_signing: self.derive(KeyPurpose::I2pSigning),
            i2p_encryption: self.derive(KeyPurpose::I2pEncryption),
            tor: self.derive(KeyPurpose::Tor),
        }
    }

    // ── Convenience methods ──

    /// Derive THE identity Ed25519 seed (32 bytes).
    /// Used for mesh signing, git commit signing, personal attribution.
    pub fn signing_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::Signing)
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
    /// This is now the same key as `signing_seed()` — the unified identity key.
    pub fn git_signing_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::Signing)
    }

    /// Derive I2P destination signing key (Ed25519 seed, 32 bytes).
    pub fn i2p_signing_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::I2pSigning)
    }

    /// Derive I2P destination encryption key (X25519, 32 bytes).
    pub fn i2p_encryption_secret(&self) -> [u8; 32] {
        self.derive(KeyPurpose::I2pEncryption)
    }

    /// Derive Tor onion v3 service key (Ed25519 seed, 32 bytes).
    pub fn tor_seed(&self) -> [u8; 32] {
        self.derive(KeyPurpose::Tor)
    }

    // ── Parameterized families (two-level HKDF) ──

    /// Derive a per-agent Ed25519 signing seed via two-level HKDF.
    pub fn derive_agent_key(&self, agent_name: &str) -> Result<[u8; 32], DeriveError> {
        self.derive_parameterized(b"styrene-agent-master-v1", HKDF_SALT_AGENT, agent_name)
    }

    /// Derive a per-label SSH user Ed25519 seed via two-level HKDF.
    pub fn derive_ssh_user_key(&self, label: &str) -> Result<[u8; 32], DeriveError> {
        self.derive_parameterized(b"styrene-ssh-user-master-v1", HKDF_SALT_SSH_USER, label)
    }

    /// Derive a per-service I2P destination key pair via two-level HKDF.
    /// Returns (signing_seed, encryption_secret) — both 32 bytes.
    pub fn derive_i2p_service(
        &self,
        service_name: &str,
    ) -> Result<([u8; 32], [u8; 32]), DeriveError> {
        if service_name.is_empty() {
            return Err(DeriveError::EmptyLabel);
        }

        // Signing key
        let signing = self.derive_parameterized(
            b"styrene-i2p-service-master-v1",
            HKDF_SALT_I2P_SERVICE,
            &format!("{service_name}/signing"),
        )?;

        // Encryption key (same master, different label suffix)
        let encryption = self.derive_parameterized(
            b"styrene-i2p-service-master-v1",
            HKDF_SALT_I2P_SERVICE,
            &format!("{service_name}/encryption"),
        )?;

        Ok((signing, encryption))
    }

    /// Derive a per-service Tor onion v3 key via two-level HKDF.
    pub fn derive_onion_service(&self, service_name: &str) -> Result<[u8; 32], DeriveError> {
        self.derive_parameterized(b"styrene-onion-master-v1", HKDF_SALT_ONION_SERVICE, service_name)
    }

    /// Generic two-level HKDF derivation for parameterized families.
    fn derive_parameterized(
        &self,
        master_info: &[u8],
        level2_salt: &[u8],
        label: &str,
    ) -> Result<[u8; 32], DeriveError> {
        if label.is_empty() {
            return Err(DeriveError::EmptyLabel);
        }

        let mut master = [0u8; 32];
        self.expander().expand(master_info, &mut master).expect("HKDF expand should not fail");

        let hk2 = Hkdf::<Sha256>::new(Some(level2_salt), &master);
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

/// All flat-purpose derived keys from a root secret.
///
/// Debug output is redacted — key material is never printed.
/// Parameterized keys (SSH user, agent, I2P service, onion service) are
/// derived separately via their respective methods.
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct DerivedKeys {
    /// THE identity Ed25519 signing key seed (mesh + git + attribution).
    pub signing: [u8; 32],
    /// RNS X25519 encryption key (32 bytes).
    pub rns_encryption: [u8; 32],
    /// age X25519 private key (32 bytes).
    pub age: [u8; 32],
    /// WireGuard Curve25519 private key (32 bytes).
    pub wireguard: [u8; 32],
    /// SSH host Ed25519 seed (32 bytes).
    pub ssh_host: [u8; 32],
    /// Yggdrasil Ed25519 key (32 bytes).
    pub yggdrasil: [u8; 32],
    /// I2P destination Ed25519 signing key seed (32 bytes).
    pub i2p_signing: [u8; 32],
    /// I2P destination X25519 encryption key (32 bytes).
    pub i2p_encryption: [u8; 32],
    /// Tor onion v3 Ed25519 key seed (32 bytes).
    pub tor: [u8; 32],
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
#[allow(deprecated)]
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
    fn derive_keys_produces_all() {
        let root = [99u8; 32];
        let keys = derive_keys(&root);
        assert_ne!(keys.signing, [0u8; 32]);
        assert_ne!(keys.rns_encryption, [0u8; 32]);
        assert_ne!(keys.yggdrasil, [0u8; 32]);
        assert_ne!(keys.wireguard, [0u8; 32]);
        assert_ne!(keys.ssh_host, [0u8; 32]);
        assert_ne!(keys.age, [0u8; 32]);
        assert_ne!(keys.i2p_signing, [0u8; 32]);
        assert_ne!(keys.i2p_encryption, [0u8; 32]);
        assert_ne!(keys.tor, [0u8; 32]);
        assert_ne!(keys.signing, keys.rns_encryption);
    }

    #[test]
    fn all_purposes_covered() {
        assert_eq!(KeyPurpose::all().len(), 9);
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
        assert_eq!(all.signing, deriver.derive(KeyPurpose::Signing));
        assert_eq!(all.rns_encryption, deriver.derive(KeyPurpose::RnsEncryption));
        assert_eq!(all.yggdrasil, deriver.derive(KeyPurpose::Yggdrasil));
        assert_eq!(all.wireguard, deriver.derive(KeyPurpose::WireGuard));
        assert_eq!(all.ssh_host, deriver.derive(KeyPurpose::SshHost));
        assert_eq!(all.age, deriver.derive(KeyPurpose::Age));
        assert_eq!(all.i2p_signing, deriver.derive(KeyPurpose::I2pSigning));
        assert_eq!(all.i2p_encryption, deriver.derive(KeyPurpose::I2pEncryption));
        assert_eq!(all.tor, deriver.derive(KeyPurpose::Tor));
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

    // --- Unified signing key tests ---

    #[test]
    fn signing_equals_legacy_rns_signing() {
        let d = KeyDeriver::new(&[42u8; 32]);
        assert_eq!(
            d.derive(KeyPurpose::Signing),
            d.derive(KeyPurpose::RnsSigning),
            "Signing must produce same bytes as legacy RnsSigning"
        );
    }

    #[test]
    fn signing_equals_legacy_git_signing() {
        let d = KeyDeriver::new(&[42u8; 32]);
        // GitSigning now maps to the same info string as Signing/RnsSigning
        assert_eq!(
            d.derive(KeyPurpose::Signing),
            d.derive(KeyPurpose::GitSigning),
            "Signing must produce same bytes as legacy GitSigning (unified)"
        );
    }

    #[test]
    fn git_signing_seed_equals_signing_seed() {
        let d = KeyDeriver::new(&[42u8; 32]);
        assert_eq!(d.git_signing_seed(), d.signing_seed());
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

    // --- Agent key tests ---

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

    // --- I2P service key tests ---

    #[test]
    fn i2p_service_deterministic() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let (s1, e1) = d.derive_i2p_service("forge").unwrap();
        let (s2, e2) = d.derive_i2p_service("forge").unwrap();
        assert_eq!(s1, s2);
        assert_eq!(e1, e2);
    }

    #[test]
    fn i2p_service_signing_differs_from_encryption() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let (signing, encryption) = d.derive_i2p_service("forge").unwrap();
        assert_ne!(signing, encryption);
    }

    #[test]
    fn i2p_service_different_names() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let (s1, _) = d.derive_i2p_service("forge").unwrap();
        let (s2, _) = d.derive_i2p_service("wiki").unwrap();
        assert_ne!(s1, s2);
    }

    #[test]
    fn i2p_service_no_collision_with_flat_i2p() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let (per_service, _) = d.derive_i2p_service("forge").unwrap();
        let flat = d.derive(KeyPurpose::I2pSigning);
        assert_ne!(per_service, flat, "per-service I2P key should differ from flat I2P key");
    }

    #[test]
    fn i2p_service_empty_name_rejected() {
        let d = KeyDeriver::new(&[42u8; 32]);
        assert!(d.derive_i2p_service("").is_err());
    }

    // --- Tor onion service key tests ---

    #[test]
    fn onion_service_deterministic() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let k1 = d.derive_onion_service("forge").unwrap();
        let k2 = d.derive_onion_service("forge").unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn onion_service_different_names() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let k1 = d.derive_onion_service("forge").unwrap();
        let k2 = d.derive_onion_service("wiki").unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn onion_service_no_collision_with_flat_tor() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let per_service = d.derive_onion_service("forge").unwrap();
        let flat = d.derive(KeyPurpose::Tor);
        assert_ne!(per_service, flat);
    }

    // --- Pinned test vectors (backwards compatibility) ---

    #[test]
    fn test_vector_flat_purposes() {
        let d = KeyDeriver::new(&[0x42u8; 32]);

        // RnsEncryption vector unchanged
        assert_eq!(
            hex::encode(d.derive(KeyPurpose::RnsEncryption)),
            "aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c"
        );

        // Signing uses the RnsSigning info string — must match the original RnsSigning vector
        let signing_hex = hex::encode(d.derive(KeyPurpose::Signing));
        let legacy_rns_hex = hex::encode(d.derive(KeyPurpose::RnsSigning));
        assert_eq!(signing_hex, legacy_rns_hex);
    }

    #[test]
    fn test_vector_git_signing_is_now_signing() {
        let d = KeyDeriver::new(&[0x42u8; 32]);
        // GitSigning NOW produces the same as Signing (was different before unification)
        // Old GitSigning vector: 6eb3d3ef12a2447f... — this is NO LONGER produced.
        // New GitSigning = Signing = RnsSigning.
        assert_eq!(
            hex::encode(d.derive(KeyPurpose::Signing)),
            hex::encode(d.derive(KeyPurpose::GitSigning)),
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

    // --- Overlay transport key isolation ---

    #[test]
    fn overlay_keys_all_distinct() {
        let d = KeyDeriver::new(&[42u8; 32]);
        let signing = d.signing_seed();
        let yggdrasil = d.derive(KeyPurpose::Yggdrasil);
        let i2p_sig = d.i2p_signing_seed();
        let i2p_enc = d.i2p_encryption_secret();
        let tor = d.tor_seed();

        let keys = [signing, yggdrasil, i2p_sig, i2p_enc, tor];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "overlay keys {i} and {j} must differ");
            }
        }
    }
}
