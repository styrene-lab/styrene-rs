#![cfg(feature = "interop-tests")]

mod common;

use ed25519_dalek::Signature;
use rns_core::identity::{lxmf_verify, PrivateIdentity};

#[derive(serde::Deserialize)]
struct IdentityVector {
    description: String,
    private_key_hex: String,
    public_key_hex: String,
    verifying_key_hex: String,
    address_hash_hex: String,
    sign_data_hex: String,
    signature_hex: String,
}

#[test]
fn identity_key_derivation() {
    let vectors: Vec<IdentityVector> = common::load_fixture("identity_vectors.json");
    assert!(!vectors.is_empty(), "no identity vectors loaded");

    for v in &vectors {
        let priv_bytes = common::hex_decode(&v.private_key_hex);
        let identity = PrivateIdentity::from_private_key_bytes(&priv_bytes).expect(&v.description);

        let pub_identity = identity.as_identity();

        // Public key must match Python-derived value
        assert_eq!(
            hex::encode(pub_identity.public_key_bytes()),
            v.public_key_hex,
            "{}: public key mismatch",
            v.description
        );

        // Verifying key must match
        assert_eq!(
            hex::encode(pub_identity.verifying_key_bytes()),
            v.verifying_key_hex,
            "{}: verifying key mismatch",
            v.description
        );

        // Address hash must match
        assert_eq!(
            hex::encode(pub_identity.address_hash.as_slice()),
            v.address_hash_hex,
            "{}: address hash mismatch",
            v.description
        );
    }
}

#[test]
fn identity_sign_verify() {
    let vectors: Vec<IdentityVector> = common::load_fixture("identity_vectors.json");

    for v in &vectors {
        let priv_bytes = common::hex_decode(&v.private_key_hex);
        let identity = PrivateIdentity::from_private_key_bytes(&priv_bytes).expect(&v.description);

        let data = common::hex_decode(&v.sign_data_hex);

        // Rust signature must match Python signature (Ed25519 is deterministic)
        let rust_sig = identity.sign(&data);
        assert_eq!(
            hex::encode(rust_sig.to_bytes()),
            v.signature_hex,
            "{}: signature mismatch",
            v.description
        );

        // Verify Python-generated signature with Rust
        let py_sig_bytes = common::hex_decode(&v.signature_hex);
        let py_sig = Signature::from_slice(&py_sig_bytes).expect("valid signature bytes");
        identity
            .verify(&data, &py_sig)
            .unwrap_or_else(|_| panic!("{}: failed to verify Python signature", v.description));

        // Also verify via the standalone lxmf_verify helper
        assert!(
            lxmf_verify(identity.as_identity(), &data, &py_sig_bytes),
            "{}: lxmf_verify failed",
            v.description
        );
    }
}

#[test]
fn identity_roundtrip_private_key_bytes() {
    let vectors: Vec<IdentityVector> = common::load_fixture("identity_vectors.json");

    for v in &vectors {
        let priv_bytes = common::hex_decode(&v.private_key_hex);
        let identity = PrivateIdentity::from_private_key_bytes(&priv_bytes).expect(&v.description);

        let exported = identity.to_private_key_bytes();
        assert_eq!(
            hex::encode(exported),
            v.private_key_hex,
            "{}: private key roundtrip failed",
            v.description
        );
    }
}
