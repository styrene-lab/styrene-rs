//! Layer 0 — Identity generation and key derivation.
//!
//! No network. Validates that identity creation works correctly
//! and that the transport identity bridge preserves key material.

use rand_core::OsRng;
use rns_core::identity::PrivateIdentity;

#[test]
fn deterministic_identity_from_name() {
    let a = PrivateIdentity::new_from_name("alice");
    let b = PrivateIdentity::new_from_name("alice");
    assert_eq!(
        a.address_hash().as_slice(),
        b.address_hash().as_slice(),
        "same name must produce same identity hash"
    );
}

#[test]
fn different_names_produce_different_identities() {
    let alice = PrivateIdentity::new_from_name("alice");
    let bob = PrivateIdentity::new_from_name("bob");
    assert_ne!(
        alice.address_hash().as_slice(),
        bob.address_hash().as_slice(),
        "different names must produce different identity hashes"
    );
}

#[test]
fn random_identity_is_unique() {
    let a = PrivateIdentity::new_from_rand(OsRng);
    let b = PrivateIdentity::new_from_rand(OsRng);
    assert_ne!(
        a.address_hash().as_slice(),
        b.address_hash().as_slice(),
        "two random identities must differ"
    );
}

#[test]
fn identity_hash_is_16_bytes() {
    let id = PrivateIdentity::new_from_name("test");
    assert_eq!(id.address_hash().as_slice().len(), 16);
    // Hex encoding should be 32 chars
    let hex = hex::encode(id.address_hash().as_slice());
    assert_eq!(hex.len(), 32);
}

#[test]
fn transport_bridge_preserves_address_hash() {
    let core_id = PrivateIdentity::new_from_name("bridge-test");
    let transport_id =
        rns_core::transport::identity_bridge::to_transport_private_identity(&core_id);

    // The transport identity's address hash should match the core identity's
    assert_eq!(
        core_id.address_hash().as_slice(),
        transport_id.address_hash().as_slice(),
        "transport bridge must preserve address hash"
    );
}

#[test]
fn private_key_bytes_roundtrip() {
    let original = PrivateIdentity::new_from_name("roundtrip");
    let bytes = original.to_private_key_bytes();
    assert_eq!(bytes.len(), 64, "private key bytes must be 64 bytes (32 enc + 32 sign)");

    let restored = PrivateIdentity::from_private_key_bytes(&bytes).expect("roundtrip from bytes");
    assert_eq!(
        original.address_hash().as_slice(),
        restored.address_hash().as_slice(),
        "roundtripped identity must have same hash"
    );
}
