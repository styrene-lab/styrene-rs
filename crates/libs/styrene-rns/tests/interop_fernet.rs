#![cfg(feature = "interop-tests")]

mod common;

use rand_core::OsRng;
use rns_core::crypt::fernet::{CachedFernet, Fernet, Token};

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

#[test]
fn canonical_1_5_1_token_tags_are_verified_before_decryption() {
    let index = common::load_rns_index().expect("valid RNS fixture index");
    let valid = common::load_rns_vector_bytes(&index, "rns-1.5.1-token-valid")
        .expect("valid token fixture");
    let invalid = common::load_rns_vector_bytes(&index, "rns-1.5.1-token-invalid-tag")
        .expect("invalid token fixture");
    let truncated = common::load_rns_vector_bytes(&index, "rns-1.5.1-token-truncated-tag")
        .expect("truncated token fixture");
    let key = (0_u8..64).collect::<Vec<_>>();
    let (sign_key, enc_key) = key.split_at(32);
    let fernet = Fernet::new_from_slices(sign_key, enc_key, OsRng);
    let cached = CachedFernet::new_from_slices(sign_key, enc_key);

    for token in [&invalid, &truncated] {
        let output = [0xa5; 64];
        assert!(fernet.verify(Token::from(token.as_slice())).is_err());
        assert!(cached.verify(Token::from(token.as_slice())).is_err());
        assert_eq!(output, [0xa5; 64], "authentication failure must not expose plaintext");
    }

    let verified = fernet.verify(Token::from(valid.as_slice())).expect("canonical token HMAC");
    let mut output = [0; 64];
    assert_eq!(
        fernet.decrypt(verified, &mut output).expect("canonical token plaintext").as_bytes(),
        b"RNS 1.5.1 token"
    );
    assert!(cached.verify(Token::from(valid.as_slice())).is_ok());
}

#[test]
fn every_token_verifier_uses_the_constant_time_mac_api() {
    let source = include_str!("../src/crypt/fernet.rs");
    assert_eq!(source.matches("hmac.verify_slice(expected_tag)").count(), 2);
    assert!(!source.contains(".zip(actual_tag"));
    assert!(!source.contains("x.cmp(y)"));
}
