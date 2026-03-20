//! ML-KEM-768 key encapsulation mechanism (FIPS 203).

use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::{Ciphertext, EncodedSizeUser, KemCore, MlKem768};
use rand_core::CryptoRngCore;
use zeroize::Zeroize;

use crate::error::TunnelError;

/// ML-KEM-768 encapsulation key size in bytes.
pub const MLKEM_ENCAPSULATION_KEY_SIZE: usize = 1184;
/// ML-KEM-768 ciphertext size in bytes.
pub const MLKEM_CIPHERTEXT_SIZE: usize = 1088;
/// ML-KEM-768 shared secret size in bytes.
pub const MLKEM_SHARED_SECRET_SIZE: usize = 32;

/// An ML-KEM-768 keypair (decapsulation key + encapsulation key).
pub struct MlKemKeyPair {
    dk: <MlKem768 as KemCore>::DecapsulationKey,
    ek: <MlKem768 as KemCore>::EncapsulationKey,
}

impl MlKemKeyPair {
    /// Generate a fresh ML-KEM-768 keypair.
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let (dk, ek) = MlKem768::generate(rng);
        Self { dk, ek }
    }

    /// Serialize the encapsulation key (public key) for transmission.
    pub fn encapsulation_key_bytes(&self) -> Vec<u8> {
        self.ek.as_bytes().to_vec()
    }

    /// Decapsulate a ciphertext to recover the shared secret.
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<MlKemSharedSecret, TunnelError> {
        if ciphertext.len() != MLKEM_CIPHERTEXT_SIZE {
            return Err(TunnelError::InvalidKeyMaterial(format!(
                "ML-KEM ciphertext must be {} bytes, got {}",
                MLKEM_CIPHERTEXT_SIZE,
                ciphertext.len()
            )));
        }

        let ct = Ciphertext::<MlKem768>::try_from(ciphertext).map_err(|_| {
            TunnelError::InvalidKeyMaterial("ML-KEM ciphertext conversion failed".into())
        })?;
        let ss = self
            .dk
            .decapsulate(&ct)
            .map_err(|_| TunnelError::Crypto("ML-KEM decapsulation failed".into()))?;
        let mut bytes = [0u8; MLKEM_SHARED_SECRET_SIZE];
        bytes.copy_from_slice(ss.as_slice());
        Ok(MlKemSharedSecret(bytes))
    }
}

/// Result of ML-KEM encapsulation: ciphertext + shared secret.
pub struct MlKemEncapsulated {
    /// Ciphertext to send to the keyholder.
    pub ciphertext: Vec<u8>,
    /// Shared secret known only to both parties.
    pub shared_secret: MlKemSharedSecret,
}

impl MlKemEncapsulated {
    /// Encapsulate against a peer's encapsulation key.
    pub fn encapsulate(ek_bytes: &[u8], rng: &mut impl CryptoRngCore) -> Result<Self, TunnelError> {
        if ek_bytes.len() != MLKEM_ENCAPSULATION_KEY_SIZE {
            return Err(TunnelError::InvalidKeyMaterial(format!(
                "ML-KEM encapsulation key must be {} bytes, got {}",
                MLKEM_ENCAPSULATION_KEY_SIZE,
                ek_bytes.len()
            )));
        }

        let ek_encoded =
            <ml_kem::Encoded<<MlKem768 as KemCore>::EncapsulationKey>>::try_from(ek_bytes)
                .map_err(|_| {
                    TunnelError::InvalidKeyMaterial(
                        "ML-KEM encapsulation key conversion failed".into(),
                    )
                })?;
        let ek = <MlKem768 as KemCore>::EncapsulationKey::from_bytes(&ek_encoded);
        let (ct, ss) = ek
            .encapsulate(rng)
            .map_err(|_| TunnelError::Crypto("ML-KEM encapsulation failed".into()))?;

        let ct_bytes = ct.as_slice().to_vec();
        let mut ss_bytes = [0u8; MLKEM_SHARED_SECRET_SIZE];
        ss_bytes.copy_from_slice(ss.as_slice());

        Ok(Self { ciphertext: ct_bytes, shared_secret: MlKemSharedSecret(ss_bytes) })
    }
}

/// A 32-byte ML-KEM shared secret, zeroized on drop.
pub struct MlKemSharedSecret([u8; MLKEM_SHARED_SECRET_SIZE]);

impl MlKemSharedSecret {
    pub fn as_bytes(&self) -> &[u8; MLKEM_SHARED_SECRET_SIZE] {
        &self.0
    }
}

impl Drop for MlKemSharedSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    #[test]
    fn kem_roundtrip() {
        let kp = MlKemKeyPair::generate(&mut OsRng);
        let ek_bytes = kp.encapsulation_key_bytes();
        assert_eq!(ek_bytes.len(), MLKEM_ENCAPSULATION_KEY_SIZE);

        let encapsulated =
            MlKemEncapsulated::encapsulate(&ek_bytes, &mut OsRng).expect("encapsulate");
        assert_eq!(encapsulated.ciphertext.len(), MLKEM_CIPHERTEXT_SIZE);

        let decapsulated = kp.decapsulate(&encapsulated.ciphertext).expect("decapsulate");
        assert_eq!(encapsulated.shared_secret.as_bytes(), decapsulated.as_bytes());
    }

    #[test]
    fn rejects_wrong_ek_size() {
        let result = MlKemEncapsulated::encapsulate(&[0u8; 100], &mut OsRng);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_ct_size() {
        let kp = MlKemKeyPair::generate(&mut OsRng);
        let result = kp.decapsulate(&[0u8; 100]);
        assert!(result.is_err());
    }
}
