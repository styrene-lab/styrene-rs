#![cfg(feature = "interop-tests")]

mod common;

use rns_core::transport::iface::ifac::{ifac_unwrap, ifac_wrap, IfacConfig};

#[derive(serde::Deserialize)]
struct IfacVector {
    description: String,
    sign_key_hex: String,
    ifac_key_hex: String,
    ifac_size: usize,
    inner_hex: String,
    wrapped_hex: String,
}

fn make_identity_from_seed(seed: &[u8]) -> rns_core::identity::PrivateIdentity {
    use ed25519_dalek::SigningKey;
    use x25519_dalek::StaticSecret;

    // IFAC test-only: both keys derived from the same seed to match the Python
    // fixture generator. Real PrivateIdentity uses independent randomness for
    // Ed25519 and X25519 (see PrivateIdentity::new_from_rand). This is safe
    // here because IFAC only uses the Ed25519 signing key.
    let sign_key = SigningKey::from_bytes(seed.try_into().expect("32-byte seed"));
    let x25519_key = StaticSecret::from(<[u8; 32]>::try_from(seed).expect("32-byte seed"));
    rns_core::identity::PrivateIdentity::new(x25519_key, sign_key)
}

#[test]
fn ifac_decode_python_fixtures() {
    let vectors: Vec<IfacVector> = common::load_fixture("ifac_vectors.json");
    assert!(!vectors.is_empty(), "no IFAC vectors loaded");

    for (i, v) in vectors.iter().enumerate() {
        let sign_key_bytes = common::hex_decode(&v.sign_key_hex);
        let ifac_key = common::hex_decode(&v.ifac_key_hex);
        let inner = common::hex_decode(&v.inner_hex);
        let wrapped = common::hex_decode(&v.wrapped_hex);

        let identity = make_identity_from_seed(&sign_key_bytes);
        let config = IfacConfig::new(ifac_key.clone(), identity, v.ifac_size);

        // Verify Rust can unwrap what Python wrapped
        let unwrapped = ifac_unwrap(&wrapped, &config);
        assert!(unwrapped.is_some(), "vector {i} ({}) — unwrap failed", v.description);
        assert_eq!(
            unwrapped.unwrap(),
            inner,
            "vector {i} ({}) — unwrapped bytes differ from inner",
            v.description
        );
    }
}

#[test]
fn ifac_encode_matches_python() {
    let vectors: Vec<IfacVector> = common::load_fixture("ifac_vectors.json");

    for (i, v) in vectors.iter().enumerate() {
        let sign_key_bytes = common::hex_decode(&v.sign_key_hex);
        let ifac_key = common::hex_decode(&v.ifac_key_hex);
        let inner = common::hex_decode(&v.inner_hex);
        let expected_wrapped = common::hex_decode(&v.wrapped_hex);

        let identity = make_identity_from_seed(&sign_key_bytes);
        let config = IfacConfig::new(ifac_key, identity, v.ifac_size);

        // Verify Rust produces byte-identical output to Python
        let rust_wrapped = ifac_wrap(&inner, &config);
        assert_eq!(
            rust_wrapped, expected_wrapped,
            "vector {i} ({}) — Rust wrap output differs from Python",
            v.description
        );
    }
}

#[test]
fn ifac_wrong_key_rejects() {
    let vectors: Vec<IfacVector> = common::load_fixture("ifac_vectors.json");
    assert!(vectors.len() >= 2, "need at least 2 vectors for cross-key test");

    // Try unwrapping vector 0's wrapped data with vector 4's key (different key pair)
    let v0 = &vectors[0];
    let v4 = &vectors[4]; // "different key pair" vector

    let wrong_key_bytes = common::hex_decode(&v4.sign_key_hex);
    let wrong_ifac_key = common::hex_decode(&v4.ifac_key_hex);
    let wrapped = common::hex_decode(&v0.wrapped_hex);

    let wrong_identity = make_identity_from_seed(&wrong_key_bytes);
    let config = IfacConfig::new(wrong_ifac_key, wrong_identity, v0.ifac_size);

    let result = ifac_unwrap(&wrapped, &config);
    assert!(result.is_none(), "wrong key should reject IFAC packet");
}
