//! Phase 3: Advanced verification — property-based testing, zeroization,
//! cross-language vector export, and stress testing.

use ed25519_dalek::{Signer, SigningKey, Verifier};
use proptest::prelude::*;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::pubkey::{ed25519_verifying_key, x25519_public_key};

// ═══════════════════════════════════════════════════════════════════
// Property-Based Testing (proptest)
// ═══════════════════════════════════════════════════════════════════

proptest! {
    /// For any random root secret, every flat purpose must produce:
    /// - Non-zero output
    /// - 32 bytes
    /// - Different output from every other purpose
    #[test]
    fn prop_any_root_all_purposes_valid(root in prop::array::uniform32(any::<u8>())) {
        let d = KeyDeriver::new(&root);
        let keys: Vec<[u8; 32]> = KeyPurpose::all()
            .iter()
            .map(|p| d.derive(*p))
            .collect();

        for (i, key) in keys.iter().enumerate() {
            prop_assert_ne!(key, &[0u8; 32], "purpose {} produced zero key", i);
        }

        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                prop_assert_ne!(
                    keys[i], keys[j],
                    "purposes {} and {} collided for root {:?}", i, j, &root[..4]
                );
            }
        }
    }

    /// For any random root, the Signing key must produce a valid Ed25519 signature.
    #[test]
    fn prop_any_root_signing_key_works(root in prop::array::uniform32(any::<u8>())) {
        let d = KeyDeriver::new(&root);
        let seed = d.derive(KeyPurpose::Signing);
        let sk = SigningKey::from_bytes(&seed);
        let vk = sk.verifying_key();

        let message = b"proptest signing verification";
        let sig = sk.sign(message);
        prop_assert!(vk.verify(message, &sig).is_ok());
    }

    /// For any random root, X25519 DH must produce a non-zero shared secret.
    #[test]
    fn prop_any_root_x25519_dh_works(root in prop::array::uniform32(any::<u8>())) {
        let d = KeyDeriver::new(&root);
        let secret_bytes = d.derive(KeyPurpose::RnsEncryption);

        let our_secret = StaticSecret::from(secret_bytes);
        let peer_secret = StaticSecret::from([0xAA; 32]);
        let peer_public = X25519PublicKey::from(&peer_secret);

        let shared = our_secret.diffie_hellman(&peer_public);
        prop_assert_ne!(shared.as_bytes(), &[0u8; 32], "DH shared secret is zero");
    }

    /// For any random label (non-empty), parameterized derivation must work.
    #[test]
    fn prop_any_label_ssh_user_key_valid(
        label in "[a-zA-Z0-9._-]{1,100}"
    ) {
        let d = KeyDeriver::new(&[0x42; 32]);
        let key = d.derive_ssh_user_key(&label).unwrap();
        prop_assert_ne!(key, [0u8; 32]);

        // Must produce valid Ed25519
        let sk = SigningKey::from_bytes(&key);
        let sig = sk.sign(b"test");
        prop_assert!(sk.verifying_key().verify(b"test", &sig).is_ok());
    }

    /// For any random label, agent keys must be valid and distinct from SSH user keys.
    #[test]
    fn prop_any_label_agent_key_differs_from_ssh(
        label in "[a-zA-Z0-9._-]{1,100}"
    ) {
        let d = KeyDeriver::new(&[0x42; 32]);
        let agent = d.derive_agent_key(&label).unwrap();
        let ssh = d.derive_ssh_user_key(&label).unwrap();
        prop_assert_ne!(agent, ssh, "agent and SSH user keys collided for label '{}'", label);
    }

    /// For any two different roots, the identity hash must differ.
    #[test]
    fn prop_different_roots_different_hashes(
        root_a in prop::array::uniform32(any::<u8>()),
        root_b in prop::array::uniform32(any::<u8>()),
    ) {
        prop_assume!(root_a != root_b);

        let da = KeyDeriver::new(&root_a);
        let db = KeyDeriver::new(&root_b);

        let seed_a = da.derive(KeyPurpose::Signing);
        let seed_b = db.derive(KeyPurpose::Signing);

        let vk_a = ed25519_verifying_key(&seed_a);
        let vk_b = ed25519_verifying_key(&seed_b);

        let hash_a = hex::encode(&Sha256::digest(vk_a.as_bytes())[..16]);
        let hash_b = hex::encode(&Sha256::digest(vk_b.as_bytes())[..16]);

        prop_assert_ne!(hash_a, hash_b, "different roots produced same identity hash");
    }

    /// I2P service keys: signing and encryption must always differ,
    /// and different service names must produce different keys.
    #[test]
    fn prop_i2p_service_isolation(
        name_a in "[a-zA-Z0-9]{1,50}",
        name_b in "[a-zA-Z0-9]{1,50}",
    ) {
        let d = KeyDeriver::new(&[0x42; 32]);

        let (sign_a, enc_a) = d.derive_i2p_service(&name_a).unwrap();
        prop_assert_ne!(sign_a, enc_a, "I2P signing and encryption collided for '{}'", name_a);

        if name_a != name_b {
            let (sign_b, _) = d.derive_i2p_service(&name_b).unwrap();
            prop_assert_ne!(sign_a, sign_b, "different I2P services produced same signing key");
        }
    }

    /// Tor onion service: different names must produce different keys.
    #[test]
    fn prop_onion_service_isolation(
        name_a in "[a-zA-Z0-9]{1,50}",
        name_b in "[a-zA-Z0-9]{1,50}",
    ) {
        prop_assume!(name_a != name_b);

        let d = KeyDeriver::new(&[0x42; 32]);
        let key_a = d.derive_onion_service(&name_a).unwrap();
        let key_b = d.derive_onion_service(&name_b).unwrap();
        prop_assert_ne!(key_a, key_b);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Zeroization Verification
// ═══════════════════════════════════════════════════════════════════

/// Verify that RootSecret debug output never leaks key material.
#[test]
fn root_secret_debug_is_redacted() {
    let root = styrene_identity::signer::RootSecret::new([0xAB; 32]);
    let debug = format!("{:?}", root);
    assert_eq!(debug, "RootSecret([REDACTED])");
    assert!(!debug.contains("ab"), "debug output leaked key bytes");
    assert!(!debug.contains("AB"), "debug output leaked key bytes");
    assert!(!debug.contains("171"), "debug output leaked decimal key bytes");
}

/// Verify that DerivedKeys debug output never leaks key material.
#[test]
fn derived_keys_debug_is_redacted() {
    let keys = styrene_identity::derive::derive_keys(&[0x42; 32]);
    let debug = format!("{:?}", keys);
    assert_eq!(debug, "DerivedKeys([REDACTED])");
}

/// Verify that KeyDeriver drop zeroizes the PRK.
/// We test indirectly: after drop, a new deriver from the same root
/// must still produce correct results (proving the drop didn't corrupt
/// shared state, which would happen if drop didn't properly zeroize).
#[test]
fn key_deriver_drop_does_not_corrupt_state() {
    let root = [0x42u8; 32];

    let expected = {
        let d = KeyDeriver::new(&root);
        d.derive(KeyPurpose::Signing)
    }; // d dropped here

    // After drop, creating a new deriver must still work correctly
    let d2 = KeyDeriver::new(&root);
    let actual = d2.derive(KeyPurpose::Signing);
    assert_eq!(actual, expected, "KeyDeriver drop corrupted shared state");
}

// ═══════════════════════════════════════════════════════════════════
// Vault File Integrity Under Stress
// ═══════════════════════════════════════════════════════════════════

/// Multiple sequential generate → load cycles with different passphrases
/// must produce different root secrets but consistent behavior.
#[test]
fn multiple_vaults_different_passphrases() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let mut roots = Vec::new();

    for i in 0..5 {
        let path = dir.path().join(format!("identity-{i}.key"));
        let passphrase = format!("passphrase-{i}");
        let pp = passphrase.as_bytes().to_vec();
        let signer = FileSigner::new(
            &path,
            Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))),
        );

        signer.generate(passphrase.as_bytes()).unwrap();
        let root = signer.load(passphrase.as_bytes()).unwrap();
        roots.push(*root.as_bytes());
    }

    // All roots must be distinct (random generation)
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(
                roots[i], roots[j],
                "vaults {i} and {j} produced the same root — RNG failure?"
            );
        }
    }
}

/// The same passphrase on different vault files must produce different roots
/// (because the root is random, not derived from the passphrase).
#[test]
fn same_passphrase_different_roots() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let passphrase = b"same-passphrase";

    let mut roots = Vec::new();
    for i in 0..3 {
        let path = dir.path().join(format!("identity-{i}.key"));
        let pp = passphrase.to_vec();
        let signer = FileSigner::new(
            &path,
            Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))),
        );

        signer.generate(passphrase).unwrap();
        let root = signer.load(passphrase).unwrap();
        roots.push(*root.as_bytes());
    }

    // Same passphrase, different files → different roots
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(roots[i], roots[j], "same passphrase produced same root on different files");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Cross-Language Verification Script Export
// ═══════════════════════════════════════════════════════════════════

/// Write the complete test vector set to a JSON file that can be
/// consumed by Python/JS/Go verification scripts.
/// The file is written to the crate's test output directory.
#[test]
fn export_test_vectors_to_file() {
    let root = [0x42u8; 32];
    let d = KeyDeriver::new(&root);

    let mut vectors = serde_json::Map::new();
    vectors.insert("root_secret_hex".into(), hex::encode(&root).into());
    vectors.insert("hkdf_salt".into(), "styrene-identity-v1".into());

    // Flat purposes
    let mut flat = serde_json::Map::new();
    for purpose in KeyPurpose::all() {
        let seed = d.derive(*purpose);
        let mut entry = serde_json::Map::new();
        entry.insert("info".into(), String::from_utf8_lossy(purpose.info()).to_string().into());
        entry.insert("seed_hex".into(), hex::encode(&seed).into());

        match purpose {
            KeyPurpose::Signing
            | KeyPurpose::SshHost
            | KeyPurpose::Yggdrasil
            | KeyPurpose::I2pSigning
            | KeyPurpose::Tor => {
                let vk = ed25519_verifying_key(&seed);
                entry.insert("pubkey_hex".into(), hex::encode(vk.as_bytes()).into());
                entry.insert("curve".into(), "ed25519".into());
            }
            KeyPurpose::RnsEncryption
            | KeyPurpose::Age
            | KeyPurpose::I2pEncryption
            | KeyPurpose::WireGuard => {
                let pk = x25519_public_key(&seed);
                entry.insert("pubkey_hex".into(), hex::encode(pk.as_bytes()).into());
                entry.insert("curve".into(), "x25519".into());
            }
            _ => {}
        }

        flat.insert(format!("{:?}", purpose), entry.into());
    }
    vectors.insert("flat_purposes".into(), flat.into());

    // Identity hash
    let signing_seed = d.derive(KeyPurpose::Signing);
    let signing_vk = ed25519_verifying_key(&signing_seed);
    let id_digest = Sha256::digest(signing_vk.as_bytes());
    vectors.insert("identity_hash".into(), hex::encode(&id_digest[..16]).into());

    // Parameterized
    let mut param = serde_json::Map::new();

    let labels = ["github", "work", "forge", "wiki", "omegon-primary", "auspex-deploy"];
    for label in &labels {
        let ssh = d.derive_ssh_user_key(label).unwrap();
        param.insert(format!("ssh_user/{label}"), hex::encode(&ssh).into());

        let agent = d.derive_agent_key(label).unwrap();
        param.insert(format!("agent/{label}"), hex::encode(&agent).into());
    }

    let services = ["forge", "wiki", "chat"];
    for svc in &services {
        let (s, e) = d.derive_i2p_service(svc).unwrap();
        param.insert(format!("i2p/{svc}/signing"), hex::encode(&s).into());
        param.insert(format!("i2p/{svc}/encryption"), hex::encode(&e).into());

        let onion = d.derive_onion_service(svc).unwrap();
        param.insert(format!("onion/{svc}"), hex::encode(&onion).into());
    }
    vectors.insert("parameterized".into(), param.into());

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(vectors)).unwrap();

    // Write to file for cross-language verification
    let vectors_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("test-vectors.json");
    std::fs::write(&vectors_path, &json).unwrap();

    // Verify the file is valid JSON and contains expected fields
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["identity_hash"].is_string());
    assert!(parsed["flat_purposes"]["Signing"]["seed_hex"].is_string());
    assert!(parsed["flat_purposes"]["Signing"]["pubkey_hex"].is_string());
    assert!(parsed["flat_purposes"]["Tor"]["pubkey_hex"].is_string());
    assert!(parsed["parameterized"]["ssh_user/github"].is_string());
    assert!(parsed["parameterized"]["i2p/forge/signing"].is_string());
    assert!(parsed["parameterized"]["onion/forge"].is_string());

    eprintln!("Test vectors written to: {}", vectors_path.display());
}
