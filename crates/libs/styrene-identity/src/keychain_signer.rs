//! Tier B: Keychain signer — biometric-protected root secret on macOS/iOS.
//!
//! Stores the 32-byte root secret in the system Keychain with
//! `kSecAccessControlBiometryCurrentSet`, requiring Face ID or Touch ID
//! to access. The Secure Enclave protects the Keychain entry — the root
//! secret never leaves the device.
//!
//! HKDF derivation happens in software (same as Tier D) after the OS
//! releases the root secret following biometric authentication.
//!
//! # Feature
//!
//! Requires the `keychain` feature flag. Only available on macOS and iOS.

use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use security_framework::passwords::{
    delete_generic_password, generic_password, set_generic_password_options, PasswordOptions,
};
use security_framework::passwords_options::AccessControlOptions;

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

/// Default Keychain service identifier.
pub const SERVICE: &str = "io.styrene.identity";
/// Default Keychain account name.
pub const ACCOUNT: &str = "root-secret";

/// Tier B signer — reads the root secret from the macOS/iOS Keychain
/// with biometric authentication (Face ID / Touch ID).
pub struct KeychainSigner {
    service: String,
    account: String,
}

impl Default for KeychainSigner {
    fn default() -> Self {
        Self {
            service: SERVICE.into(),
            account: ACCOUNT.into(),
        }
    }
}

impl KeychainSigner {
    /// Create a signer with custom service/account identifiers.
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    /// Check if a biometric-protected identity exists in the Keychain.
    ///
    /// Calls `generic_password()` which may trigger a biometric prompt if the
    /// item is accessible without explicit auth. In practice, biometric-protected
    /// items return `errSecInteractionNotAllowed` (-25308) or `errSecAuthFailed`
    /// (-25293) before prompting, which we interpret as "exists".
    pub fn exists(&self) -> bool {
        match generic_password(PasswordOptions::new_generic_password(&self.service, &self.account)) {
            Ok(_) => true,
            Err(e) => {
                let code = e.code();
                // errSecInteractionNotAllowed (-25308) = item exists but needs auth
                // errSecAuthFailed (-25293) = item exists but auth was cancelled
                code == -25308 || code == -25293
            }
        }
    }

    /// Generate a new random root secret and store it in the Keychain
    /// with biometric protection.
    ///
    /// The biometric prompt appears immediately to confirm storage.
    pub fn create(&self) -> Result<(), SignerError> {
        if self.exists() {
            return Err(SignerError::Unavailable(
                "Identity already exists in Keychain. Delete it first.".into(),
            ));
        }

        // Generate 32 random bytes
        let mut secret = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *secret);

        // Store with biometric protection
        let mut opts = PasswordOptions::new_generic_password(&self.service, &self.account);
        opts.set_access_control_options(
            AccessControlOptions::BIOMETRY_CURRENT_SET | AccessControlOptions::OR | AccessControlOptions::DEVICE_PASSCODE,
        );

        set_generic_password_options(&*secret, opts)
            .map_err(|e| SignerError::SigningFailed(format!("Keychain store failed: {e}")))?;

        Ok(())
    }

    /// Delete the identity from the Keychain.
    pub fn delete(&self) -> Result<(), SignerError> {
        delete_generic_password(&self.service, &self.account)
            .map_err(|e| SignerError::Unavailable(format!("Keychain delete failed: {e}")))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl IdentitySigner for KeychainSigner {
    fn tier(&self) -> SignerTier {
        SignerTier::DeviceHsm
    }

    fn label(&self) -> &str {
        "Keychain (biometric)"
    }

    fn is_available(&self) -> bool {
        self.exists()
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        // This triggers the biometric prompt on macOS/iOS
        let data = generic_password(
            PasswordOptions::new_generic_password(&self.service, &self.account),
        )
        .map_err(|e| {
            let code = e.code();
            if code == -25293 || code == -128 {
                // User cancelled biometric prompt
                SignerError::AuthRequired("Biometric authentication cancelled".into())
            } else if code == -25308 {
                // Interaction not allowed (e.g. device locked, no UI context)
                SignerError::AuthRequired("Biometric authentication required but not available in this context".into())
            } else if code == -25300 {
                // Item not found
                SignerError::KeyNotFound("No identity in Keychain".into())
            } else {
                SignerError::DecryptionFailed(format!("Keychain read failed: {e}"))
            }
        })?;

        if data.len() != 32 {
            return Err(SignerError::DecryptionFailed(format!(
                "Invalid root secret length: {} (expected 32)",
                data.len()
            )));
        }

        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&data);
        Ok(RootSecret::new(bytes))
    }

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        let root = self.root_secret().await?;
        let deriver = crate::derive::KeyDeriver::new(root.as_bytes());
        let seed = Zeroizing::new(deriver.derive(crate::derive::KeyPurpose::Signing));
        Ok(crate::pubkey::sign_with_seed(&seed, data).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_signer_has_correct_service() {
        let signer = KeychainSigner::default();
        assert_eq!(signer.service, "io.styrene.identity");
        assert_eq!(signer.account, "root-secret");
    }

    #[test]
    fn custom_signer() {
        let signer = KeychainSigner::new("custom.service", "custom-key");
        assert_eq!(signer.service, "custom.service");
        assert_eq!(signer.account, "custom-key");
    }

    // Note: full integration tests require a physical device with biometrics.
    // The Keychain biometric APIs don't work in the iOS simulator or in CI.
    // Run manually: cargo test -p styrene-identity --features keychain -- keychain
}
