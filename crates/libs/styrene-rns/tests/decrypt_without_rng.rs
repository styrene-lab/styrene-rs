//! Failure classification and verify-before-decrypt checks, with entropy traps.

#[path = "support/rng.rs"]
mod rng;

use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, RngCore};
use rng::RecordingRng;
use rns_core::RnsError;
use rns_core::crypt::fernet::{CachedFernet, Fernet, Token};
use rns_core::identity::{
    DERIVED_KEY_LENGTH, DecryptIdentity, DerivedKey, EmptyIdentity, PrivateIdentity,
};
use rns_core::ratchets::{decrypt_with_identity, decrypt_with_private_key, encrypt_for_public_key};
use rns_core::{group_decrypt, group_encrypt_with_rng};
use sha2::Sha256;
use std::cell::Cell;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Clone, Copy)]
struct EntropyTrap<'a>(&'a Cell<usize>);

impl RngCore for EntropyTrap<'_> {
    fn next_u32(&mut self) -> u32 {
        self.fill_bytes(&mut []);
        unreachable!()
    }
    fn next_u64(&mut self) -> u64 {
        self.fill_bytes(&mut []);
        unreachable!()
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        self.0.set(self.0.get() + 1);
        panic!("decryption requested entropy");
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for EntropyTrap<'_> {}

fn retag(token: &mut [u8], sign_key: &[u8]) {
    let tag_start = token.len() - 32;
    let mut mac = Hmac::<Sha256>::new_from_slice(sign_key).expect("HMAC key");
    mac.update(&token[..tag_start]);
    token[tag_start..].copy_from_slice(&mac.finalize().into_bytes());
}

/// This is deliberately an authenticated invalid padding token. Altering the
/// last IV byte changes the last byte of its single plaintext block to zero.
fn invalid_padding(token: &[u8], sign_key: &[u8], plaintext_len: usize) -> Vec<u8> {
    let mut invalid = token.to_vec();
    invalid[15] ^= (16 - plaintext_len) as u8;
    retag(&mut invalid, sign_key);
    invalid
}

#[test]
fn identity_and_fernet_decrypt_outcomes_never_request_entropy() {
    let identity = PrivateIdentity::new_from_name("decrypt contract");
    let key = DerivedKey::new_empty();
    let mut rng = RecordingRng::default();
    let mut buffer = [0; 128];
    let plaintext = b"authenticated";
    let token =
        identity.encrypt_with_rng(&mut rng, plaintext, &key, &mut buffer).expect("token").to_vec();
    let requests = Cell::new(0);
    let trap = EntropyTrap(&requests);
    let fernet = Fernet::new_from_slices(&[0; 32], &[0; 32], trap);
    let cached = CachedFernet::new_from_slices(&[0; 32], &[0; 32]);

    let mut tampered = token.clone();
    tampered[16] ^= 1;
    let mut bad_tag = token.clone();
    *bad_tag.last_mut().expect("tag") ^= 1;
    let failures = [
        (Vec::new(), RnsError::InvalidArgument),
        (vec![0; 32], RnsError::InvalidArgument),
        (vec![0; 48], RnsError::InvalidArgument),
        (token[..token.len() - 1].to_vec(), RnsError::IncorrectSignature),
        (tampered, RnsError::IncorrectSignature),
        (bad_tag, RnsError::IncorrectSignature),
    ];
    for (data, expected) in failures {
        let mut output = [0xa5; 128];
        assert_eq!(identity.decrypt(trap, &data, &key, &mut output), Err(expected));
        assert_eq!(output, [0xa5; 128], "authentication failure exposes no plaintext");
        assert_eq!(identity.decrypt_with_derived_key(&data, &key, &mut output), Err(expected));
        assert_eq!(output, [0xa5; 128]);
        assert_eq!(fernet.verify(Token::from(data.as_slice())).err(), Some(expected));
        assert_eq!(cached.verify(Token::from(data.as_slice())).err(), Some(expected));
    }

    let wrong_key = identity.as_identity().derive_key(&mut rng, None);
    let mut output = [0xa5; 128];
    assert_eq!(
        identity.decrypt(trap, &token, &wrong_key, &mut output),
        Err(RnsError::IncorrectSignature)
    );
    assert_eq!(
        identity.decrypt_with_derived_key(&token, &wrong_key, &mut output),
        Err(RnsError::IncorrectSignature)
    );
    assert_eq!(output, [0xa5; 128]);

    assert_eq!(identity.decrypt(trap, &token, &key, &mut output), Ok(plaintext.as_slice()));
    assert_eq!(
        identity.decrypt_with_derived_key(&token, &key, &mut output),
        Ok(plaintext.as_slice())
    );
    let verified = fernet.verify(Token::from(token.as_slice())).expect("valid tag");
    assert_eq!(fernet.decrypt(verified, &mut output).expect("plaintext").as_bytes(), plaintext);
    let verified = cached.verify(Token::from(token.as_slice())).expect("valid cached tag");
    assert_eq!(cached.decrypt(verified, &mut output).expect("plaintext").as_bytes(), plaintext);

    let mut nonaligned = token.clone();
    nonaligned.remove(16);
    retag(&mut nonaligned, &key.as_bytes()[..DERIVED_KEY_LENGTH / 2]);
    let bad_padding =
        invalid_padding(&token, &key.as_bytes()[..DERIVED_KEY_LENGTH / 2], plaintext.len());
    for data in [bad_padding, nonaligned] {
        // Authenticated decryption errors can modify output. Assert the error,
        // not a stronger untouched-buffer promise than the legacy API provides.
        assert_eq!(identity.decrypt(trap, &data, &key, &mut output), Err(RnsError::CryptoError));
        assert_eq!(
            identity.decrypt_with_derived_key(&data, &key, &mut output),
            Err(RnsError::CryptoError)
        );
        let verified =
            fernet.verify(Token::from(data.as_slice())).expect("authenticated invalid ciphertext");
        assert_eq!(fernet.decrypt(verified, &mut output).err(), Some(RnsError::CryptoError));
        let verified =
            cached.verify(Token::from(data.as_slice())).expect("authenticated invalid ciphertext");
        assert_eq!(cached.decrypt(verified, &mut output).err(), Some(RnsError::CryptoError));
    }
    assert_eq!(identity.decrypt(trap, &token, &key, &mut output[..1]), Err(RnsError::CryptoError));
    assert_eq!(
        identity.decrypt_with_derived_key(&token, &key, &mut output[..1]),
        Err(RnsError::CryptoError)
    );
    let verified = fernet.verify(Token::from(token.as_slice())).expect("tag");
    assert_eq!(fernet.decrypt(verified, &mut output[..1]).err(), Some(RnsError::CryptoError));
    let verified = cached.verify(Token::from(token.as_slice())).expect("tag");
    assert_eq!(cached.decrypt(verified, &mut output[..1]).err(), Some(RnsError::CryptoError));
    assert_eq!(requests.get(), 0);
}

#[test]
fn empty_identity_decryption_ignores_legacy_rng_for_every_outcome() {
    let requests = Cell::new(0);
    let trap = EntropyTrap(&requests);
    let key = DerivedKey::new_empty();
    let mut out = [0xa5; 5];
    assert_eq!(EmptyIdentity.decrypt(trap, b"plain", &key, &mut out), Ok(b"plain".as_slice()));
    assert_eq!(EmptyIdentity.decrypt(trap, &[], &key, &mut out), Ok([].as_slice()));
    out.fill(0xa5);
    assert_eq!(
        EmptyIdentity.decrypt(trap, b"too long", &key, &mut out),
        Err(RnsError::OutOfMemory)
    );
    assert_eq!(out, [0xa5; 5]);
    assert_eq!(requests.get(), 0);
}

#[test]
fn group_and_ratchet_decrypt_preserve_error_mapping_without_a_provider() {
    let mut rng = RecordingRng::default();
    let plaintext = b"authenticated";
    let group = group_encrypt_with_rng(&[0; 16], plaintext, &mut rng).expect("group token");
    let before = rng.lengths();
    assert_eq!(group_decrypt(&[0; 16], &group), Ok(plaintext.to_vec()));
    assert_eq!(group_decrypt(&[1; 16], &group), Err(RnsError::IncorrectSignature));
    assert_eq!(group_decrypt(&[0; 16], &[]), Err(RnsError::InvalidArgument));
    assert_eq!(group_decrypt(&[0; 16], &group[..48]), Err(RnsError::InvalidArgument));
    assert_eq!(
        group_decrypt(&[0; 16], &group[..group.len() - 1]),
        Err(RnsError::IncorrectSignature)
    );
    let mut tampered = group.clone();
    tampered[16] ^= 1;
    assert_eq!(group_decrypt(&[0; 16], &tampered), Err(RnsError::IncorrectSignature));
    let bad_padding = invalid_padding(&group, &[0; DERIVED_KEY_LENGTH / 2], plaintext.len());
    assert_eq!(group_decrypt(&[0; 16], &bad_padding), Err(RnsError::CryptoError));
    assert_eq!(rng.lengths(), before);

    let identity = PrivateIdentity::new_from_name("ratchet decrypt contract");
    let private = StaticSecret::from(
        <[u8; 32]>::try_from(&identity.to_private_key_bytes()[..32]).expect("private key"),
    );
    let ciphertext =
        encrypt_for_public_key(&identity.as_identity().public_key, b"salt", plaintext, &mut rng)
            .expect("ratchet ciphertext");
    let before = rng.lengths();
    let ephemeral =
        PublicKey::from(<[u8; 32]>::try_from(&ciphertext[..32]).expect("ephemeral public"));
    let key = identity.derive_key(&ephemeral, Some(b"salt"));
    let mut padding = ciphertext[..32].to_vec();
    padding.extend(invalid_padding(
        &ciphertext[32..],
        &key.as_bytes()[..DERIVED_KEY_LENGTH / 2],
        plaintext.len(),
    ));
    let mut tampered = ciphertext.clone();
    tampered[48] ^= 1;
    for (data, expected) in [
        (ciphertext.clone(), Ok(plaintext.to_vec())),
        (vec![], Err(RnsError::InvalidArgument)),
        (vec![0; 32], Err(RnsError::InvalidArgument)),
        (vec![0; 33], Err(RnsError::CryptoError)),
        (ciphertext[..ciphertext.len() - 1].to_vec(), Err(RnsError::CryptoError)),
        (tampered, Err(RnsError::CryptoError)),
        (padding, Err(RnsError::CryptoError)),
    ] {
        assert_eq!(decrypt_with_private_key(&private, b"salt", &data), expected);
        assert_eq!(decrypt_with_identity(&identity, b"salt", &data), expected);
    }
    assert_eq!(
        decrypt_with_identity(&identity, b"wrong salt", &ciphertext),
        Err(RnsError::CryptoError)
    );
    assert_eq!(
        decrypt_with_private_key(&StaticSecret::from([1; 32]), b"salt", &ciphertext),
        Err(RnsError::CryptoError)
    );
    assert_eq!(
        decrypt_with_identity(
            &PrivateIdentity::new_from_name("wrong identity"),
            b"salt",
            &ciphertext
        ),
        Err(RnsError::CryptoError)
    );
    assert_eq!(rng.lengths(), before);
}
