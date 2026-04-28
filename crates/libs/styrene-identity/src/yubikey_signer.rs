//! Tier A: YubiKey hardware-backed signer — FIDO2 hmac-secret extension.
//!
//! Derives the StyreneID root secret from a FIDO2 credential's hmac-secret
//! PRF. The YubiKey's internal secret never leaves the secure element —
//! only the derived 32-byte output is returned.
//!
//! ## Setup (one-time)
//!
//! ```ignore
//! use styrene_identity::yubikey_signer::YubiKeySigner;
//!
//! let cred_id = YubiKeySigner::setup_credential("styrene.mesh", Some("1234"))?;
//! println!("Save this credential ID in your config: {}", cred_id);
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use styrene_identity::yubikey_signer::YubiKeySigner;
//!
//! let signer = YubiKeySigner::new(
//!     "base64-credential-id",
//!     "styrene.mesh",
//!     false, // require_touch
//! );
//! // signer.root_secret() derives the 32-byte root via hmac-secret
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! ## Security Model
//!
//! The root secret is hardware-derived but extracted to process memory for
//! HKDF derivation. This is Tier A for the root extraction step, but derived
//! protocol keys (SSH, age, etc.) are software-computed from that root. The
//! YubiKey must be present for each `root_secret()` call — there is no
//! caching across calls.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand_core::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

/// Dedicated StyreneID salt for FIDO2 hmac-secret.
///
/// Distinct from the RNS-specific salts used in styrened's Python implementation
/// (`styrene-encryption-v1`, `styrene-signing-v1`). This ensures the StyreneID
/// root secret is cryptographically independent from legacy direct-derivation keys.
fn styrene_identity_salt() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"styrene-identity-root-v1");
    let result = hasher.finalize();
    let mut salt = [0u8; 32];
    salt.copy_from_slice(&result);
    salt
}

/// Provides the YubiKey PIN securely.
///
/// Implementations should obtain the PIN from a platform keychain,
/// interactive prompt, or secure IPC — never from environment variables
/// which are visible to co-tenant processes and child processes.
pub trait PinProvider: Send + Sync {
    /// Get the PIN string, or None for touch-only / no-PIN operation.
    fn get_pin(&self) -> Result<Option<String>, SignerError>;
}

/// PIN provider that always returns None (touch-only / no PIN).
pub struct NoPinProvider;

impl PinProvider for NoPinProvider {
    fn get_pin(&self) -> Result<Option<String>, SignerError> {
        Ok(None)
    }
}

/// Tier A signer using YubiKey FIDO2 hmac-secret extension.
pub struct YubiKeySigner {
    /// Base64-encoded FIDO2 credential ID (from setup).
    credential_id_b64: String,
    /// Relying party ID for FIDO2 operations.
    rp_id: String,
    /// Whether to require physical touch for each derivation.
    require_touch: bool,
    /// Human-readable label.
    label: String,
    /// PIN provider — called on each root_secret() invocation.
    pin_provider: Box<dyn PinProvider>,
}

impl YubiKeySigner {
    /// Create a signer for an existing FIDO2 credential.
    ///
    /// The `credential_id_b64` is the base64 string returned by
    /// [`setup_credential`](Self::setup_credential).
    pub fn new(
        credential_id_b64: &str,
        rp_id: &str,
        require_touch: bool,
        pin_provider: Box<dyn PinProvider>,
    ) -> Self {
        Self {
            credential_id_b64: credential_id_b64.to_string(),
            rp_id: rp_id.to_string(),
            require_touch,
            label: format!("yubikey:{rp_id}"),
            pin_provider,
        }
    }

    /// One-time setup: create a FIDO2 resident credential with hmac-secret.
    ///
    /// Returns the base64-encoded credential ID that must be stored in config.
    /// Requires the YubiKey to be connected and a PIN to be set.
    pub fn setup_credential(rp_id: &str, pin: Option<&str>) -> Result<String, SignerError> {
        use ctap_hid_fido2::fidokey::make_credential::make_credential_params::{
            CredentialSupportedKeyType, Extension as Mext,
        };
        use ctap_hid_fido2::{FidoKeyHidFactory, LibCfg};

        let cfg = LibCfg::init();
        let device = FidoKeyHidFactory::create(&cfg)
            .map_err(|e| SignerError::Unavailable(format!("no YubiKey detected: {e}")))?;

        // Random challenge per FIDO2 spec, even though we only care about the credential.
        let mut challenge = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut challenge);
        let mut builder = ctap_hid_fido2::fidokey::make_credential::make_credential_params::MakeCredentialArgsBuilder::new(rp_id, &challenge)
            .key_type(CredentialSupportedKeyType::Ed25519)
            .extensions(&[Mext::HmacSecret(Some(true))])
            .resident_key();

        if let Some(pin) = pin {
            builder = builder.pin(pin);
        }

        let args = builder.build();
        let attestation = device
            .make_credential_with_args(&args)
            .map_err(|e| SignerError::SigningFailed(format!("credential creation failed: {e}")))?;

        let cred_id = &attestation.credential_descriptor.id;
        if cred_id.is_empty() {
            return Err(SignerError::KeyNotFound(
                "no credential ID in attestation response".into(),
            ));
        }

        Ok(BASE64.encode(cred_id))
    }

    /// Derive the 32-byte root secret from the YubiKey via hmac-secret.
    fn derive_root(&self, pin: Option<&str>) -> Result<RootSecret, SignerError> {
        use ctap_hid_fido2::fidokey::get_assertion::get_assertion_params::Extension as Gext;
        use ctap_hid_fido2::{FidoKeyHidFactory, LibCfg};

        let cfg = LibCfg::init();
        let device = FidoKeyHidFactory::create(&cfg)
            .map_err(|e| SignerError::Unavailable(format!("no YubiKey detected: {e}")))?;

        let credential_id = BASE64
            .decode(&self.credential_id_b64)
            .map_err(|e| SignerError::KeyNotFound(format!("invalid credential ID base64: {e}")))?;

        let salt = styrene_identity_salt();
        // Random challenge per FIDO2 spec — prevents assertion replay at CTAP layer.
        let mut challenge = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut challenge);

        let mut builder = ctap_hid_fido2::fidokey::get_assertion::GetAssertionArgsBuilder::new(
            &self.rp_id,
            &challenge,
        )
        .credential_id(&credential_id)
        .extensions(&[Gext::HmacSecret(Some(salt))]);

        if let Some(pin) = pin {
            builder = builder.pin(pin);
        } else if !self.require_touch {
            builder = builder.without_pin_and_uv();
        }

        let args = builder.build();
        let assertions = device.get_assertion_with_args(&args).map_err(|e| {
            SignerError::SigningFailed(format!("hmac-secret assertion failed: {e}"))
        })?;

        let assertion = assertions
            .first()
            .ok_or_else(|| SignerError::KeyNotFound("no assertion returned".into()))?;

        // Extract hmac-secret output from extensions
        for ext in &assertion.extensions {
            if let Gext::HmacSecret(Some(output)) = ext {
                return Ok(RootSecret::new(*output));
            }
        }

        Err(SignerError::KeyNotFound(
            "YubiKey did not return hmac-secret output — \
             ensure the credential was created with hmac-secret enabled"
                .into(),
        ))
    }
}

#[async_trait::async_trait]
impl IdentitySigner for YubiKeySigner {
    fn tier(&self) -> SignerTier {
        SignerTier::HardwareHsm
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn is_available(&self) -> bool {
        // Check if any FIDO device is connected
        !ctap_hid_fido2::get_fidokey_devices().is_empty()
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        let pin = self.pin_provider.get_pin()?;
        self.derive_root(pin.as_deref())
    }

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        let root = self.root_secret().await?;
        let deriver = crate::derive::KeyDeriver::new(root.as_bytes());
        let mut seed = deriver.derive(crate::derive::KeyPurpose::Signing);
        let sig = crate::pubkey::sign_with_seed(&seed, data);
        seed.zeroize();
        Ok(sig.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styrene_identity_salt_is_deterministic() {
        let s1 = styrene_identity_salt();
        let s2 = styrene_identity_salt();
        assert_eq!(s1, s2);
    }

    #[test]
    fn styrene_identity_salt_is_not_zero() {
        let salt = styrene_identity_salt();
        assert_ne!(salt, [0u8; 32]);
    }

    #[test]
    fn styrene_identity_salt_differs_from_rns_salts() {
        let identity_salt = styrene_identity_salt();

        // The RNS Python salts from styrened's yubikey.py
        let mut hasher = Sha256::new();
        hasher.update(b"styrene-encryption-v1");
        let rns_encrypt: [u8; 32] = hasher.finalize().into();

        let mut hasher = Sha256::new();
        hasher.update(b"styrene-signing-v1");
        let rns_sign: [u8; 32] = hasher.finalize().into();

        assert_ne!(identity_salt, rns_encrypt);
        assert_ne!(identity_salt, rns_sign);
    }

    #[test]
    fn signer_tier_is_hardware_hsm() {
        let signer = YubiKeySigner::new("dGVzdA==", "styrene.mesh", false, Box::new(NoPinProvider));
        assert_eq!(signer.tier(), SignerTier::HardwareHsm);
    }

    #[test]
    fn signer_label_includes_rp_id() {
        let signer = YubiKeySigner::new("dGVzdA==", "styrene.mesh", false, Box::new(NoPinProvider));
        assert_eq!(signer.label(), "yubikey:styrene.mesh");
    }

    // NOTE: Hardware-dependent tests (setup_credential, derive_root, sign)
    // require a physical YubiKey and are not run in CI. They should be
    // tested manually with: cargo test -p styrene-identity --features yubikey -- --ignored
}
