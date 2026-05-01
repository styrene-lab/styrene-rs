//! styrene-identity ↔ RNS identity integration tests.
//!
//! Tests the bridge between the styrene-identity key hierarchy (root secret →
//! HKDF → protocol keys) and the RNS transport identity system (PrivateIdentity).
//! Validates determinism, hash consistency, sign/verify across the boundary,
//! and vault lifecycle for daemon integration.

use rns_core::identity::PrivateIdentity;
use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::signer::RootSecret;

/// Derive an RNS PrivateIdentity from a styrene-identity root secret.
///
/// This is the bridge function the daemon will use when migrating from
/// raw identity files to encrypted vault storage.
fn private_identity_from_root(root: &RootSecret) -> PrivateIdentity {
    let deriver = KeyDeriver::new(root.as_bytes());
    let signing_seed = deriver.derive(KeyPurpose::Signing);
    let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);

    let sign_key = ed25519_dalek::SigningKey::from_bytes(&signing_seed);
    let static_secret = x25519_dalek::StaticSecret::from(encryption_seed);

    PrivateIdentity::new(static_secret, sign_key)
}

// ── Deterministic Derivation ───────────────────────────────────────────

#[test]
fn same_root_produces_same_identity() {
    let root = RootSecret::new([0x42u8; 32]);
    let id1 = private_identity_from_root(&root);
    let id2 = private_identity_from_root(&root);

    assert_eq!(
        id1.address_hash().as_slice(),
        id2.address_hash().as_slice(),
        "same root must produce same RNS identity hash"
    );
}

#[test]
fn different_roots_produce_different_identities() {
    let id1 = private_identity_from_root(&RootSecret::new([0x01u8; 32]));
    let id2 = private_identity_from_root(&RootSecret::new([0x02u8; 32]));

    assert_ne!(
        id1.address_hash().as_slice(),
        id2.address_hash().as_slice(),
        "different roots must produce different identities"
    );
}

#[test]
fn ephemeral_root_produces_unique_identity() {
    let id1 = private_identity_from_root(&RootSecret::ephemeral());
    let id2 = private_identity_from_root(&RootSecret::ephemeral());

    assert_ne!(
        id1.address_hash().as_slice(),
        id2.address_hash().as_slice(),
        "ephemeral roots should produce different identities"
    );
}

// ── Hash Consistency ───────────────────────────────────────────────────

#[test]
fn rns_hash_differs_from_styrene_identity_hash() {
    // The RNS address hash includes both X25519 and Ed25519 keys:
    //   SHA256(X25519_pubkey || Ed25519_verifying_key)[:16]
    //
    // The styrene-identity hash is signing-key only:
    //   SHA256(Ed25519_verifying_key)[:16]
    //
    // These are intentionally different: the styrene-identity hash is
    // the "person" identifier, while the RNS hash is the "node" identifier.
    let root = RootSecret::new([0x42u8; 32]);
    let rns_id = private_identity_from_root(&root);
    let rns_hash = hex::encode(rns_id.address_hash().as_slice());
    let styrene_hash = styrene_identity::identity_hash(&root);

    assert_ne!(
        rns_hash, styrene_hash,
        "RNS hash (pubkey||verifying) should differ from styrene-identity hash (verifying only)"
    );

    // Both should be 32 hex chars
    assert_eq!(rns_hash.len(), 32);
    assert_eq!(styrene_hash.len(), 32);
}

#[test]
fn rns_identity_hash_is_deterministic() {
    let root = RootSecret::new([0x42u8; 32]);
    let h1 = hex::encode(private_identity_from_root(&root).address_hash().as_slice());
    let h2 = hex::encode(private_identity_from_root(&root).address_hash().as_slice());
    assert_eq!(h1, h2);
}

// ── Sign/Verify Across Boundary ────────────────────────────────────────

#[test]
fn sign_with_rns_verify_with_rns() {
    let root = RootSecret::new([0x42u8; 32]);
    let id = private_identity_from_root(&root);
    let data = b"test message for signing";

    let signature = id.sign(data);

    assert!(
        id.as_identity().verify(data, &signature).is_ok(),
        "RNS identity sign/verify should work"
    );
}

#[test]
fn sign_with_styrene_verify_with_rns() {
    // Sign data using styrene-identity's identity_sign,
    // then verify using the corresponding RNS Identity's public key.
    let root = RootSecret::new([0x42u8; 32]);
    let attestation = styrene_identity::identity_sign(&root, b"cross-boundary");

    // The RNS identity derived from the same root should have the same
    // Ed25519 verifying key
    let rns_id = private_identity_from_root(&root);
    let rns_verifying = rns_id.as_identity().verifying_key_bytes();

    assert_eq!(
        attestation.pubkey,
        *rns_verifying,
        "styrene-identity pubkey should match RNS verifying key"
    );

    // Verify the signature using RNS identity
    let sig = ed25519_dalek::Signature::from_bytes(&attestation.signature);
    assert!(
        rns_id.as_identity().verify(b"cross-boundary", &sig).is_ok(),
        "signature from styrene-identity should verify via RNS identity"
    );
}

#[test]
fn sign_with_rns_verify_with_styrene() {
    let root = RootSecret::new([0x42u8; 32]);
    let rns_id = private_identity_from_root(&root);
    let data = b"reverse-boundary";

    let signature = rns_id.sign(data);

    // Verify using styrene-identity's verify function
    let pubkey = styrene_identity::identity_pubkey(&root);
    assert!(
        styrene_identity::identity_verify(&pubkey, data, &signature.to_bytes()),
        "signature from RNS should verify via styrene-identity"
    );
}

// ── Transport Bridge ───────────────────────────────────────────────────

#[test]
fn derived_identity_works_with_transport_bridge() {
    let root = RootSecret::new([0x42u8; 32]);
    let rns_id = private_identity_from_root(&root);

    // The transport bridge should preserve the identity
    let transport_id =
        rns_core::transport::identity_bridge::to_transport_private_identity(&rns_id);

    assert_eq!(
        rns_id.address_hash().as_slice(),
        transport_id.address_hash().as_slice(),
        "transport bridge should preserve address hash"
    );
}

// ── Key Purpose Isolation ──────────────────────────────────────────────

#[test]
fn signing_and_encryption_keys_are_different() {
    let root = RootSecret::new([0x42u8; 32]);
    let deriver = KeyDeriver::new(root.as_bytes());

    let signing = deriver.derive(KeyPurpose::Signing);
    let encryption = deriver.derive(KeyPurpose::RnsEncryption);

    assert_ne!(signing, encryption, "signing and encryption seeds must differ");
}

#[test]
fn all_protocol_keys_are_unique() {
    let root = RootSecret::new([0x42u8; 32]);
    let deriver = KeyDeriver::new(root.as_bytes());

    let mut keys = std::collections::HashSet::new();
    for purpose in KeyPurpose::all() {
        let key = deriver.derive(*purpose);
        assert!(
            keys.insert(key),
            "key for {:?} should be unique",
            purpose
        );
    }
}

// ── Vault Lifecycle ────────────────────────────────────────────────────

#[tokio::test]
async fn vault_init_unlock_produces_same_identity() {
    use styrene_identity::IdentitySigner;
    use styrene_identity::file_signer::ClosurePassphraseProvider;
    use styrene_identity::vault::IdentityVault;

    let dir = tempfile::tempdir().expect("tempdir");
    let key_path = dir.path().join("test-identity.key");

    let passphrase = b"test-passphrase-123";

    // Init vault
    let vault = IdentityVault::new(
        key_path.clone(),
        Box::new(ClosurePassphraseProvider::new(|| Ok(passphrase.to_vec()))),
    );
    vault.init(passphrase).expect("init vault");

    // Derive identity from the vault's root secret
    let root1 = vault.signer().root_secret().await.expect("unlock 1");
    let id1 = private_identity_from_root(&root1);
    let hash1 = hex::encode(id1.address_hash().as_slice());

    // Create new vault instance pointing to same file
    let vault2 = IdentityVault::new(
        key_path,
        Box::new(ClosurePassphraseProvider::new(|| Ok(passphrase.to_vec()))),
    );
    let root2 = vault2.signer().root_secret().await.expect("unlock 2");
    let id2 = private_identity_from_root(&root2);
    let hash2 = hex::encode(id2.address_hash().as_slice());

    assert_eq!(
        hash1, hash2,
        "same vault file should produce same identity on re-unlock"
    );
}

#[tokio::test]
async fn vault_wrong_passphrase_fails() {
    use styrene_identity::IdentitySigner;
    use styrene_identity::file_signer::ClosurePassphraseProvider;
    use styrene_identity::vault::IdentityVault;

    let dir = tempfile::tempdir().expect("tempdir");
    let key_path = dir.path().join("test-wrong-pass.key");

    // Init with correct passphrase
    let vault = IdentityVault::new(
        key_path.clone(),
        Box::new(ClosurePassphraseProvider::new(|| Ok(b"correct".to_vec()))),
    );
    vault.init(b"correct").expect("init");

    // Try to unlock with wrong passphrase
    let vault2 = IdentityVault::new(
        key_path,
        Box::new(ClosurePassphraseProvider::new(|| Ok(b"wrong".to_vec()))),
    );
    let result = vault2.signer().root_secret().await;
    assert!(
        result.is_err(),
        "wrong passphrase should fail to unlock vault"
    );
}
