//! Android Keystore-backed identity root secret.
//!
//! A non-exportable AES-GCM key in `AndroidKeyStore` wraps the random root
//! secret. Only the authenticated ciphertext and IV are persisted in the
//! application's private `SharedPreferences`.

use android_keyring::credential::AndroidBuilder;
use keyring::credential::CredentialBuilderApi;
use keyring::{Entry, Error as KeyringError};
use rand_core::{OsRng, RngCore};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::signer::{IdentitySigner, RootSecret, SignerError, SignerTier};

pub const SERVICE: &str = "io.styrene.identity";
pub const ACCOUNT: &str = "root-secret";

pub struct AndroidKeystoreSigner {
    entry: Entry,
}

impl AndroidKeystoreSigner {
    pub fn new(service: &str, account: &str) -> Result<Self, SignerError> {
        let builder = AndroidBuilder::from_ndk_context()
            .map_err(|error| SignerError::Unavailable(format!("Android context: {error}")))?;
        let credential = builder.build(None, service, account).map_err(map_storage_error)?;
        Ok(Self { entry: Entry::new_with_credential(credential) })
    }

    pub fn load_or_create_root_secret(&self) -> Result<RootSecret, SignerError> {
        match self.entry.get_secret() {
            Ok(secret) => decode_root_secret(secret),
            Err(KeyringError::NoEntry) => {
                let mut secret = Zeroizing::new([0_u8; 32]);
                OsRng.fill_bytes(&mut *secret);
                self.entry.set_secret(secret.as_slice()).map_err(map_storage_error)?;
                let persisted = self.entry.get_secret().map_err(map_storage_error)?;
                if !bool::from(secret.as_slice().ct_eq(&persisted)) {
                    return Err(SignerError::DecryptionFailed(
                        "Android Keystore root secret verification failed".into(),
                    ));
                }
                Ok(RootSecret::new(*secret))
            }
            Err(error) => Err(map_storage_error(error)),
        }
    }

    /// Load existing custody without creating a replacement when it is absent.
    pub fn load_root_secret(&self) -> Result<RootSecret, SignerError> {
        self.entry.get_secret().map_err(map_storage_error).and_then(decode_root_secret)
    }

    /// Whether this application's protected Android custody already exists.
    pub fn exists(&self) -> Result<bool, SignerError> {
        match self.entry.get_secret() {
            Ok(_) => Ok(true),
            Err(KeyringError::NoEntry) => Ok(false),
            Err(error) => Err(map_storage_error(error)),
        }
    }

    /// Install a recovered root secret without replacing existing custody.
    pub fn restore_root_secret(&self, secret: &RootSecret) -> Result<(), SignerError> {
        if self.exists()? {
            return Err(SignerError::Unavailable(
                "Identity already exists in Android Keystore".into(),
            ));
        }
        self.entry.set_secret(secret.as_bytes()).map_err(map_storage_error)?;
        let persisted = self.entry.get_secret().map_err(map_storage_error)?;
        if !bool::from(secret.as_bytes().ct_eq(&persisted)) {
            return Err(SignerError::DecryptionFailed(
                "Android Keystore root secret verification failed".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl IdentitySigner for AndroidKeystoreSigner {
    fn tier(&self) -> SignerTier {
        SignerTier::DeviceHsm
    }

    fn label(&self) -> &str {
        "Android Keystore"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn root_secret(&self) -> Result<RootSecret, SignerError> {
        self.load_or_create_root_secret()
    }

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        let root = self.root_secret().await?;
        let deriver = crate::derive::KeyDeriver::new(root.as_bytes());
        let seed = Zeroizing::new(deriver.derive(crate::derive::KeyPurpose::Signing));
        Ok(crate::pubkey::sign_with_seed(&seed, data).to_vec())
    }
}

fn decode_root_secret(secret: Vec<u8>) -> Result<RootSecret, SignerError> {
    let secret = Zeroizing::new(secret);
    let bytes: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
        SignerError::DecryptionFailed(format!(
            "invalid Android Keystore root secret length: {}",
            secret.len()
        ))
    })?;
    Ok(RootSecret::new(bytes))
}

fn map_storage_error(error: KeyringError) -> SignerError {
    match error {
        KeyringError::NoEntry => SignerError::KeyNotFound("no Android identity".into()),
        KeyringError::NoStorageAccess(error) => {
            SignerError::Unavailable(format!("Android Keystore access: {error}"))
        }
        error => SignerError::DecryptionFailed(format!("Android Keystore: {error}")),
    }
}
