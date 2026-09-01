//! Tier B: Keychain signer — device-protected root secret on macOS/iOS.
//!
//! Stores the 32-byte root secret in the system Keychain with
//! `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. The device passcode gates
//! access after a restart, while later app launches and background reconnects do
//! not require repeated Face ID or Touch ID prompts. The item cannot migrate to
//! another device.
//!
//! HKDF derivation happens in software (same as Tier D) after the OS
//! releases the root secret after the device's first unlock.
//!
//! # Feature
//!
//! Requires the `keychain` feature flag. Only available on macOS and iOS.

use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::item::{ItemClass, ItemSearchOptions};
use security_framework::passwords::{
    PasswordOptions, delete_generic_password, generic_password, set_generic_password_options,
};

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

/// Default Keychain service identifier.
pub const SERVICE: &str = "io.styrene.identity";
/// Default Keychain account name.
pub const ACCOUNT: &str = "root-secret-v2";
/// Account used by the former per-access biometric policy.
pub const LEGACY_BIOMETRIC_ACCOUNT: &str = "root-secret";

/// Tier B signer — reads a device-bound root secret from the macOS/iOS Keychain.
pub struct KeychainSigner {
    service: String,
    account: String,
}

impl Default for KeychainSigner {
    fn default() -> Self {
        Self { service: SERVICE.into(), account: ACCOUNT.into() }
    }
}

impl KeychainSigner {
    /// Create a signer with custom service/account identifiers.
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self { service: service.into(), account: account.into() }
    }

    /// Check if a protected identity exists without reading its secret.
    pub fn exists(&self) -> bool {
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&self.service)
            .account(&self.account)
            .load_attributes(true)
            .search()
            .is_ok_and(|items| !items.is_empty())
    }

    /// Generate a new random root secret and store it in the Keychain.
    pub fn create(&self) -> Result<(), SignerError> {
        self.create_root_secret().map(drop)
    }

    /// Generate, persist, and return a new root secret without reading it back.
    pub fn create_root_secret(&self) -> Result<RootSecret, SignerError> {
        if self.exists() {
            return Err(SignerError::Unavailable(
                "Identity already exists in Keychain. Delete it first.".into(),
            ));
        }

        let mut secret = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *secret);
        let root = RootSecret::new(*secret);
        self.create_from_root_secret(&root)?;
        Ok(root)
    }

    /// Persist existing root material under this signer's account.
    ///
    /// This is used to migrate the former biometric item to the first-unlock
    /// policy without deleting the only durable copy first.
    pub fn create_from_root_secret(&self, root: &RootSecret) -> Result<(), SignerError> {
        if self.exists() {
            return Err(SignerError::Unavailable(
                "Identity already exists in Keychain. Delete it first.".into(),
            ));
        }

        let mut opts = PasswordOptions::new_generic_password(&self.service, &self.account);
        let access_control = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
            0,
        )
        .map_err(|e| SignerError::SigningFailed(format!("Keychain access policy failed: {e}")))?;
        opts.set_access_control(access_control);

        set_generic_password_options(root.as_bytes(), opts)
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
        "Keychain (after first unlock)"
    }

    fn is_available(&self) -> bool {
        self.exists()
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        let data =
            generic_password(PasswordOptions::new_generic_password(&self.service, &self.account))
                .map_err(|e| {
                let code = e.code();
                if code == -25293 || code == -128 {
                    SignerError::AuthRequired("Device authentication cancelled".into())
                } else if code == -25308 {
                    SignerError::AuthRequired(
                        "Unlock the device once after restart to access the identity".into(),
                    )
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
        assert_eq!(signer.account, "root-secret-v2");
        assert_eq!(LEGACY_BIOMETRIC_ACCOUNT, "root-secret");
    }

    #[test]
    fn custom_signer() {
        let signer = KeychainSigner::new("custom.service", "custom-key");
        assert_eq!(signer.service, "custom.service");
        assert_eq!(signer.account, "custom-key");
    }

    // Note: full integration tests require a physical device Keychain.
    // Run manually: cargo test -p styrene-identity --features keychain -- keychain
}
