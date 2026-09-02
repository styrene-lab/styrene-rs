use alloc::vec;
use alloc::vec::Vec;

use rand_core::CryptoRngCore;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use crate::crypt::fernet::{
    FERNET_MAX_PADDING_SIZE, FERNET_OVERHEAD_SIZE, Fernet, PlainText, Token,
};
use crate::error::RnsError;
use crate::identity::{DerivedKey, PUBLIC_KEY_LENGTH, PrivateIdentity};

pub fn encrypt_for_public_key<R: CryptoRngCore + Copy>(
    public_key: &PublicKey,
    salt: &[u8],
    plaintext: &[u8],
    rng: R,
) -> Result<Vec<u8>, RnsError> {
    let secret = EphemeralSecret::random_from_rng(rng);
    let ephemeral_public = PublicKey::from(&secret);
    let shared = secret.diffie_hellman(public_key);
    let derived = DerivedKey::new(&shared, Some(salt));
    let key_bytes = derived.as_bytes();
    let split = key_bytes.len() / 2;

    let fernet = Fernet::new_from_slices(&key_bytes[..split], &key_bytes[split..], rng);
    let mut out =
        vec![
            0u8;
            PUBLIC_KEY_LENGTH + plaintext.len() + FERNET_OVERHEAD_SIZE + FERNET_MAX_PADDING_SIZE
        ];
    out[..PUBLIC_KEY_LENGTH].copy_from_slice(ephemeral_public.as_bytes());
    let token = fernet
        .encrypt(PlainText::from(plaintext), &mut out[PUBLIC_KEY_LENGTH..])
        .map_err(|_| RnsError::CryptoError)?;
    let total = PUBLIC_KEY_LENGTH + token.len();
    out.truncate(total);
    Ok(out)
}

pub fn decrypt_with_private_key(
    private_key: &StaticSecret,
    salt: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, RnsError> {
    if ciphertext.len() <= PUBLIC_KEY_LENGTH {
        return Err(RnsError::InvalidArgument);
    }
    let mut pub_bytes = [0u8; PUBLIC_KEY_LENGTH];
    pub_bytes.copy_from_slice(&ciphertext[..PUBLIC_KEY_LENGTH]);
    let ephemeral_public = PublicKey::from(pub_bytes);
    let shared = private_key.diffie_hellman(&ephemeral_public);
    let derived = DerivedKey::new(&shared, Some(salt));
    let key_bytes = derived.as_bytes();
    let split = key_bytes.len() / 2;

    let fernet =
        Fernet::new_from_slices(&key_bytes[..split], &key_bytes[split..], rand_core::OsRng);
    let token = Token::from(&ciphertext[PUBLIC_KEY_LENGTH..]);
    let verified = fernet.verify(token).map_err(|_| RnsError::CryptoError)?;
    let mut out = vec![0u8; ciphertext.len()];
    let plain = fernet.decrypt(verified, &mut out).map_err(|_| RnsError::CryptoError)?;
    Ok(plain.as_bytes().to_vec())
}

pub fn decrypt_with_identity(
    identity: &PrivateIdentity,
    salt: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, RnsError> {
    if ciphertext.len() <= PUBLIC_KEY_LENGTH {
        return Err(RnsError::InvalidArgument);
    }
    let mut pub_bytes = [0u8; PUBLIC_KEY_LENGTH];
    pub_bytes.copy_from_slice(&ciphertext[..PUBLIC_KEY_LENGTH]);
    let ephemeral_public = PublicKey::from(pub_bytes);
    let derived = identity.derive_key(&ephemeral_public, Some(salt));
    let key_bytes = derived.as_bytes();
    let split = key_bytes.len() / 2;

    let fernet =
        Fernet::new_from_slices(&key_bytes[..split], &key_bytes[split..], rand_core::OsRng);
    let token = Token::from(&ciphertext[PUBLIC_KEY_LENGTH..]);
    let verified = fernet.verify(token).map_err(|_| RnsError::CryptoError)?;
    let mut out = vec![0u8; ciphertext.len()];
    let plain = fernet.decrypt(verified, &mut out).map_err(|_| RnsError::CryptoError)?;
    Ok(plain.as_bytes().to_vec())
}

/// Current Unix time as fractional seconds from the system clock.
///
/// Transport-side bookkeeping that already requires `std` uses this; the
/// protocol core uses [`crate::time_source::unix_now`], which is explicit
/// about unavailable time on embedded targets.
#[cfg(feature = "std")]
pub fn now_secs() -> f64 {
    crate::time_source::unix_now_secs_f64().unwrap_or_default()
}
