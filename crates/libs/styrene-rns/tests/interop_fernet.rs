#![cfg(feature = "interop-tests")]

mod common;

use rand_core::OsRng;
use rns_core::crypt::fernet::{Fernet, Token};

#[derive(serde::Deserialize)]
struct FernetVector {
    description: String,
    sign_key_hex: String,
    enc_key_hex: String,
    plaintext_hex: String,
    token_hex: String,
}

#[test]
fn fernet_verify_python_tokens() {
    let vectors: Vec<FernetVector> = common::load_fixture("fernet_vectors.json");
    assert!(!vectors.is_empty(), "no fernet vectors loaded");

    for v in &vectors {
        let sign_key = common::hex_decode(&v.sign_key_hex);
        let enc_key = common::hex_decode(&v.enc_key_hex);
        let token_bytes = common::hex_decode(&v.token_hex);

        let fernet = Fernet::new_from_slices(&sign_key, &enc_key, OsRng);
        let token = Token::from(token_bytes.as_slice());

        fernet
            .verify(token)
            .unwrap_or_else(|e| panic!("{}: HMAC verification failed: {e:?}", v.description));
    }
}

#[test]
fn fernet_decrypt_python_tokens() {
    let vectors: Vec<FernetVector> = common::load_fixture("fernet_vectors.json");

    for v in &vectors {
        let sign_key = common::hex_decode(&v.sign_key_hex);
        let enc_key = common::hex_decode(&v.enc_key_hex);
        let expected_plaintext = common::hex_decode(&v.plaintext_hex);
        let token_bytes = common::hex_decode(&v.token_hex);

        let fernet = Fernet::new_from_slices(&sign_key, &enc_key, OsRng);
        let token = Token::from(token_bytes.as_slice());

        let verified = fernet
            .verify(token)
            .unwrap_or_else(|e| panic!("{}: verify failed: {e:?}", v.description));

        let mut out_buf = vec![0u8; token_bytes.len()];
        let plaintext = fernet
            .decrypt(verified, &mut out_buf)
            .unwrap_or_else(|e| panic!("{}: decrypt failed: {e:?}", v.description));

        assert_eq!(
            plaintext.as_bytes(),
            expected_plaintext.as_slice(),
            "{}: plaintext mismatch",
            v.description
        );
    }
}

#[test]
fn fernet_rejects_tampered_token() {
    let vectors: Vec<FernetVector> = common::load_fixture("fernet_vectors.json");

    for v in &vectors {
        let sign_key = common::hex_decode(&v.sign_key_hex);
        let enc_key = common::hex_decode(&v.enc_key_hex);
        let mut token_bytes = common::hex_decode(&v.token_hex);

        if token_bytes.is_empty() {
            continue;
        }

        // Flip a bit in the ciphertext portion (between IV and HMAC)
        let tamper_idx = 16.min(token_bytes.len() - 1);
        token_bytes[tamper_idx] ^= 0x01;

        let fernet = Fernet::new_from_slices(&sign_key, &enc_key, OsRng);
        let token = Token::from(token_bytes.as_slice());

        assert!(
            fernet.verify(token).is_err(),
            "{}: tampered token should fail HMAC verification",
            v.description
        );
    }
}
