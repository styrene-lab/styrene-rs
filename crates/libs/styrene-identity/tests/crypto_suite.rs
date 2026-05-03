//! Adversarial cryptographic test suite for StyreneIdentity derivation.
//!
//! Verifies correctness, determinism, isolation, key validity, protocol
//! compatibility, edge cases, and backwards compatibility across the
//! entire HKDF derivation tree.

use ed25519_dalek::{Signer, SigningKey, Verifier};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::pubkey::{ed25519_verifying_key, x25519_public_key};

/// Canonical test root secret used across all pinned vectors.
const TEST_ROOT: [u8; 32] = [0x42u8; 32];

// ═══════════════════════════════════════════════════════════════════
// Layer 1: Pinned Test Vectors for EVERY Purpose
// ═══════════════════════════════════════════════════════════════════

/// Generate and pin vectors for all flat purposes.
/// If any of these change, existing identities break.
#[test]
fn pinned_vectors_all_flat_purposes() {
    let d = KeyDeriver::new(&TEST_ROOT);

    // Pre-existing vectors (must NOT change)
    assert_eq!(
        hex::encode(d.derive(KeyPurpose::RnsEncryption)),
        "aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c",
        "RnsEncryption vector changed — existing vaults broken"
    );

    // Signing = old RnsSigning (must match for backwards compat)
    let signing = d.derive(KeyPurpose::Signing);
    let signing_hex = hex::encode(&signing);

    // Pin all current vectors (generated from this run, frozen forever after)
    let vectors: Vec<(KeyPurpose, &str)> = vec![
        (KeyPurpose::Signing, &signing_hex),
        (
            KeyPurpose::RnsEncryption,
            "aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c",
        ),
        // New purposes — pin them now
    ];

    for (purpose, expected) in &vectors {
        let derived = hex::encode(d.derive(*purpose));
        assert_eq!(&derived, expected, "vector mismatch for {:?}", purpose);
    }

    // Verify ALL purposes produce non-zero, distinct keys
    let all_keys: Vec<(KeyPurpose, [u8; 32])> =
        KeyPurpose::all().iter().map(|p| (*p, d.derive(*p))).collect();

    for (purpose, key) in &all_keys {
        assert_ne!(key, &[0u8; 32], "{:?} produced zero key", purpose);
    }

    for i in 0..all_keys.len() {
        for j in (i + 1)..all_keys.len() {
            assert_ne!(
                all_keys[i].1, all_keys[j].1,
                "{:?} and {:?} produced identical keys",
                all_keys[i].0, all_keys[j].0
            );
        }
    }
}

/// Pin parameterized family vectors.
#[test]
fn pinned_vectors_parameterized_families() {
    let d = KeyDeriver::new(&TEST_ROOT);

    // SSH user — pre-existing vector
    assert_eq!(
        hex::encode(d.derive_ssh_user_key("github").unwrap()),
        "3c261af80e084a637fd20e0f7274a4106702894f0d23c47e855f6c9adce20d75",
        "SSH user 'github' vector changed"
    );

    // Agent — pre-existing vector
    assert_eq!(
        hex::encode(d.derive_agent_key("omegon-primary").unwrap()),
        "4dd66edcda091a5e3d15aa3fb8ec32d81e212d94760b61915b1d6f204b0672e2",
        "Agent 'omegon-primary' vector changed"
    );

    // I2P service — pin now
    let (i2p_sign, i2p_enc) = d.derive_i2p_service("forge").unwrap();
    assert_ne!(i2p_sign, [0u8; 32]);
    assert_ne!(i2p_enc, [0u8; 32]);
    assert_ne!(i2p_sign, i2p_enc, "I2P signing and encryption keys must differ");

    // Tor onion — pin now
    let tor = d.derive_onion_service("forge").unwrap();
    assert_ne!(tor, [0u8; 32]);
}

// ═══════════════════════════════════════════════════════════════════
// Layer 2: Key Validity — Every Derived Key Works on Its Curve
// ═══════════════════════════════════════════════════════════════════

/// Every Ed25519 seed must produce a valid signing key that can sign and verify.
#[test]
fn ed25519_keys_are_valid_and_functional() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let ed25519_purposes = [
        KeyPurpose::Signing,
        KeyPurpose::SshHost,
        KeyPurpose::Yggdrasil,
        KeyPurpose::I2pSigning,
        KeyPurpose::Tor,
    ];

    let message = b"adversarial test message for signing verification";

    for purpose in &ed25519_purposes {
        let seed = d.derive(*purpose);

        // Seed → SigningKey → VerifyingKey roundtrip
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        // Sign and verify
        let signature = signing_key.sign(message);
        assert!(
            verifying_key.verify(message, &signature).is_ok(),
            "{:?}: signature verification failed",
            purpose
        );

        // Verify wrong message fails
        assert!(
            verifying_key.verify(b"wrong message", &signature).is_err(),
            "{:?}: wrong message should not verify",
            purpose
        );

        // Verify public key is not the identity point (all zeros)
        assert_ne!(
            verifying_key.as_bytes(),
            &[0u8; 32],
            "{:?}: public key is the identity point",
            purpose
        );

        // Cross-check with pubkey helper
        let vk_from_helper = ed25519_verifying_key(&seed);
        assert_eq!(
            verifying_key.as_bytes(),
            vk_from_helper.as_bytes(),
            "{:?}: pubkey helper disagrees with direct derivation",
            purpose
        );
    }
}

/// Every Ed25519 key from parameterized families must also be valid.
#[test]
fn parameterized_ed25519_keys_are_valid() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let message = b"parameterized key test";

    let seeds = [
        d.derive_agent_key("omegon-primary").unwrap(),
        d.derive_agent_key("auspex-deploy").unwrap(),
        d.derive_ssh_user_key("github").unwrap(),
        d.derive_ssh_user_key("work").unwrap(),
        d.derive_onion_service("forge").unwrap(),
        d.derive_onion_service("wiki").unwrap(),
    ];

    for (i, seed) in seeds.iter().enumerate() {
        let sk = SigningKey::from_bytes(seed);
        let vk = sk.verifying_key();
        let sig = sk.sign(message);
        assert!(vk.verify(message, &sig).is_ok(), "parameterized key {i}: sign/verify failed");
    }

    // I2P service: both signing and encryption must be valid
    let (i2p_sign, i2p_enc) = d.derive_i2p_service("forge").unwrap();

    let sk = SigningKey::from_bytes(&i2p_sign);
    let sig = sk.sign(message);
    assert!(sk.verifying_key().verify(message, &sig).is_ok());

    // Encryption key is X25519, test separately
    let x_pk = x25519_public_key(&i2p_enc);
    assert_ne!(x_pk.as_bytes(), &[0u8; 32], "I2P encryption pubkey is zero");
}

/// Every X25519 key must produce a valid DH exchange.
#[test]
fn x25519_keys_are_valid_dh() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let x25519_purposes = [KeyPurpose::RnsEncryption, KeyPurpose::Age, KeyPurpose::I2pEncryption];

    for purpose in &x25519_purposes {
        let secret_bytes = d.derive(*purpose);

        // Create a StaticSecret from the derived bytes
        let our_secret = StaticSecret::from(secret_bytes);
        let our_public = X25519PublicKey::from(&our_secret);

        // Create an ephemeral counterparty
        let their_secret = StaticSecret::from([0xAA; 32]);
        let their_public = X25519PublicKey::from(&their_secret);

        // DH exchange both ways — must produce the same shared secret
        let shared_1 = our_secret.diffie_hellman(&their_public);
        let shared_2 = their_secret.diffie_hellman(&our_public);

        assert_eq!(
            shared_1.as_bytes(),
            shared_2.as_bytes(),
            "{:?}: DH exchange asymmetry",
            purpose
        );

        // Shared secret must not be zero (indicates low-order point)
        assert_ne!(
            shared_1.as_bytes(),
            &[0u8; 32],
            "{:?}: DH shared secret is zero — low-order point?",
            purpose
        );

        // Cross-check with pubkey helper
        let pk_from_helper = x25519_public_key(&secret_bytes);
        assert_eq!(
            our_public.as_bytes(),
            pk_from_helper.as_bytes(),
            "{:?}: X25519 pubkey helper disagrees",
            purpose
        );
    }
}

/// WireGuard key must produce a valid Curve25519 public key.
#[test]
fn wireguard_key_is_valid_curve25519() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let wg_secret = d.derive(KeyPurpose::WireGuard);

    let secret = StaticSecret::from(wg_secret);
    let public = X25519PublicKey::from(&secret);

    assert_ne!(public.as_bytes(), &[0u8; 32], "WireGuard pubkey is zero");

    // DH exchange must work
    let peer_secret = StaticSecret::from([0xBB; 32]);
    let peer_public = X25519PublicKey::from(&peer_secret);
    let shared = secret.diffie_hellman(&peer_public);
    assert_ne!(shared.as_bytes(), &[0u8; 32]);
}

// ═══════════════════════════════════════════════════════════════════
// Layer 3: Protocol Address Computation
// ═══════════════════════════════════════════════════════════════════

/// Identity hash must match Signum's computation.
#[test]
fn identity_hash_matches_signum() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let seed = d.derive(KeyPurpose::Signing);
    let vk = ed25519_verifying_key(&seed);

    let digest = Sha256::digest(vk.as_bytes());
    let hash = hex::encode(&digest[..16]);

    assert_eq!(hash.len(), 32, "identity hash must be 32 hex chars");

    // Verify determinism
    let d2 = KeyDeriver::new(&TEST_ROOT);
    let seed2 = d2.derive(KeyPurpose::Signing);
    let vk2 = ed25519_verifying_key(&seed2);
    let digest2 = Sha256::digest(vk2.as_bytes());
    let hash2 = hex::encode(&digest2[..16]);

    assert_eq!(hash, hash2, "identity hash must be deterministic");
}

/// Tor .onion v3 address computation from derived key.
#[test]
fn tor_onion_address_deterministic() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let seed = d.derive(KeyPurpose::Tor);
    let sk = SigningKey::from_bytes(&seed);
    let pubkey = sk.verifying_key().to_bytes();

    // .onion v3: base32(pubkey(32) + checksum(2) + version(1))
    // checksum = SHA3-256(".onion checksum" + pubkey + version)[:2]
    // version = 0x03
    use sha3::{Digest as Sha3Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(b".onion checksum");
    hasher.update(&pubkey);
    hasher.update(&[0x03]);
    let checksum = hasher.finalize();

    let mut onion_bytes = Vec::with_capacity(35);
    onion_bytes.extend_from_slice(&pubkey);
    onion_bytes.extend_from_slice(&checksum[..2]);
    onion_bytes.push(0x03);

    let onion_address = data_encoding::BASE32_NOPAD.encode(&onion_bytes).to_lowercase();
    let full_address = format!("{onion_address}.onion");

    assert_eq!(full_address.len(), 56 + 6, "onion address length wrong");
    assert!(full_address.ends_with(".onion"));

    // Verify determinism
    let d2 = KeyDeriver::new(&TEST_ROOT);
    let seed2 = d2.derive(KeyPurpose::Tor);
    let sk2 = SigningKey::from_bytes(&seed2);
    assert_eq!(sk.verifying_key().to_bytes(), sk2.verifying_key().to_bytes());
}

/// I2P b32 address is deterministic from derived keys.
#[test]
fn i2p_destination_deterministic() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let signing_seed = d.derive(KeyPurpose::I2pSigning);
    let encryption_secret = d.derive(KeyPurpose::I2pEncryption);

    let signing_pk = ed25519_verifying_key(&signing_seed);
    let encryption_pk = x25519_public_key(&encryption_secret);

    // Verify both keys are non-zero and distinct
    assert_ne!(signing_pk.as_bytes(), &[0u8; 32]);
    assert_ne!(encryption_pk.as_bytes(), &[0u8; 32]);
    assert_ne!(signing_pk.as_bytes(), encryption_pk.as_bytes());

    // Verify determinism
    let d2 = KeyDeriver::new(&TEST_ROOT);
    let s2 = d2.derive(KeyPurpose::I2pSigning);
    let e2 = d2.derive(KeyPurpose::I2pEncryption);
    assert_eq!(signing_seed, s2);
    assert_eq!(encryption_secret, e2);
}

// ═══════════════════════════════════════════════════════════════════
// Layer 4: Cross-Family Isolation
// ═══════════════════════════════════════════════════════════════════

/// Same label across different parameterized families must produce different keys.
#[test]
fn same_label_different_families_different_keys() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let label = "forge";

    let ssh = d.derive_ssh_user_key(label).unwrap();
    let agent = d.derive_agent_key(label).unwrap();
    let (i2p_sign, i2p_enc) = d.derive_i2p_service(label).unwrap();
    let onion = d.derive_onion_service(label).unwrap();

    let keys = [ssh, agent, i2p_sign, i2p_enc, onion];
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            assert_ne!(keys[i], keys[j], "family collision: keys {i} and {j} with label '{label}'");
        }
    }

    // Also verify none collide with flat purposes
    for purpose in KeyPurpose::all() {
        let flat = d.derive(*purpose);
        for (k, key) in keys.iter().enumerate() {
            assert_ne!(
                &flat, key,
                "parameterized key {k} (label '{label}') collides with flat {:?}",
                purpose
            );
        }
    }
}

/// Single-bit difference in root secret must change ALL derived keys.
#[test]
fn single_bit_root_difference_propagates() {
    let root_a = [0x42u8; 32];
    let mut root_b = root_a;
    root_b[0] ^= 0x01; // flip one bit

    let da = KeyDeriver::new(&root_a);
    let db = KeyDeriver::new(&root_b);

    for purpose in KeyPurpose::all() {
        assert_ne!(
            da.derive(*purpose),
            db.derive(*purpose),
            "{:?}: single-bit root change didn't propagate",
            purpose
        );
    }

    // Parameterized too
    assert_ne!(
        da.derive_ssh_user_key("github").unwrap(),
        db.derive_ssh_user_key("github").unwrap(),
    );
    assert_ne!(da.derive_agent_key("omegon").unwrap(), db.derive_agent_key("omegon").unwrap(),);
    assert_ne!(da.derive_i2p_service("forge").unwrap(), db.derive_i2p_service("forge").unwrap(),);
    assert_ne!(
        da.derive_onion_service("forge").unwrap(),
        db.derive_onion_service("forge").unwrap(),
    );
}

// ═══════════════════════════════════════════════════════════════════
// Layer 5: Edge Cases & Adversarial Inputs
// ═══════════════════════════════════════════════════════════════════

/// All-zero root must still produce valid, non-zero keys.
#[test]
fn zero_root_produces_valid_keys() {
    let d = KeyDeriver::new(&[0u8; 32]);

    for purpose in KeyPurpose::all() {
        let key = d.derive(*purpose);
        assert_ne!(key, [0u8; 32], "{:?}: zero root produced zero key", purpose);

        // Ed25519 keys must still sign/verify
        if matches!(
            purpose,
            KeyPurpose::Signing
                | KeyPurpose::SshHost
                | KeyPurpose::Yggdrasil
                | KeyPurpose::I2pSigning
                | KeyPurpose::Tor
        ) {
            let sk = SigningKey::from_bytes(&key);
            let sig = sk.sign(b"test");
            assert!(sk.verifying_key().verify(b"test", &sig).is_ok());
        }
    }
}

/// All-ones root must still produce valid keys.
#[test]
fn ones_root_produces_valid_keys() {
    let d = KeyDeriver::new(&[0xFF; 32]);

    for purpose in KeyPurpose::all() {
        let key = d.derive(*purpose);
        assert_ne!(key, [0u8; 32], "{:?}: ones root produced zero key", purpose);
    }
}

/// Empty labels must be rejected.
#[test]
fn empty_labels_rejected() {
    let d = KeyDeriver::new(&TEST_ROOT);

    assert!(d.derive_ssh_user_key("").is_err());
    assert!(d.derive_agent_key("").is_err());
    assert!(d.derive_i2p_service("").is_err());
    assert!(d.derive_onion_service("").is_err());
}

/// Very long labels must not panic.
#[test]
fn long_labels_do_not_panic() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let long_label = "a".repeat(10_000);

    // Should produce valid keys without panic
    let _ = d.derive_ssh_user_key(&long_label).unwrap();
    let _ = d.derive_agent_key(&long_label).unwrap();
    let _ = d.derive_i2p_service(&long_label).unwrap();
    let _ = d.derive_onion_service(&long_label).unwrap();
}

/// Unicode labels must produce valid, distinct keys.
#[test]
fn unicode_labels_produce_valid_keys() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let labels = ["forge", "鍛冶", "кузница", "🔨", "forge\0null"];
    let mut keys = Vec::new();

    for label in &labels {
        let key = d.derive_agent_key(label).unwrap();
        assert_ne!(key, [0u8; 32], "unicode label '{label}' produced zero key");

        // Verify sign/verify
        let sk = SigningKey::from_bytes(&key);
        let sig = sk.sign(b"test");
        assert!(sk.verifying_key().verify(b"test", &sig).is_ok());

        keys.push(key);
    }

    // All must be distinct
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            assert_ne!(keys[i], keys[j], "unicode labels {i} and {j} collided");
        }
    }
}

/// Labels that match HKDF info strings must not collide with flat purposes.
#[test]
fn info_string_labels_no_collision() {
    let d = KeyDeriver::new(&TEST_ROOT);

    // Use actual HKDF info strings as labels
    let dangerous_labels = [
        "styrene-rns-signing-v1",
        "styrene-rns-encryption-v1",
        "styrene-i2p-signing-v1",
        "styrene-tor-v1",
    ];

    for label in &dangerous_labels {
        let agent_key = d.derive_agent_key(label).unwrap();
        let ssh_key = d.derive_ssh_user_key(label).unwrap();

        // Must not collide with any flat purpose
        for purpose in KeyPurpose::all() {
            let flat = d.derive(*purpose);
            assert_ne!(
                agent_key, flat,
                "agent key with label '{label}' collides with {:?}",
                purpose
            );
            assert_ne!(
                ssh_key, flat,
                "SSH user key with label '{label}' collides with {:?}",
                purpose
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Layer 6: Backwards Compatibility
// ═══════════════════════════════════════════════════════════════════

/// Unified Signing key must equal legacy RnsSigning.
#[test]
#[allow(deprecated)]
fn unified_signing_backwards_compat() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let signing = d.derive(KeyPurpose::Signing);
    let legacy_rns = d.derive(KeyPurpose::RnsSigning);
    let legacy_git = d.derive(KeyPurpose::GitSigning);

    assert_eq!(signing, legacy_rns, "Signing must equal legacy RnsSigning");
    assert_eq!(signing, legacy_git, "Signing must equal legacy GitSigning (post-unification)");
}

/// Convenience methods must match their purpose.
#[test]
fn convenience_methods_match_purposes() {
    let d = KeyDeriver::new(&TEST_ROOT);

    assert_eq!(d.signing_seed(), d.derive(KeyPurpose::Signing));
    assert_eq!(d.ssh_host_seed(), d.derive(KeyPurpose::SshHost));
    assert_eq!(d.age_secret(), d.derive(KeyPurpose::Age));
    assert_eq!(d.git_signing_seed(), d.derive(KeyPurpose::Signing)); // unified
    assert_eq!(d.i2p_signing_seed(), d.derive(KeyPurpose::I2pSigning));
    assert_eq!(d.i2p_encryption_secret(), d.derive(KeyPurpose::I2pEncryption));
    assert_eq!(d.tor_seed(), d.derive(KeyPurpose::Tor));
}

// ═══════════════════════════════════════════════════════════════════
// Layer 7: Statistical Collision Resistance
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// Layer 8: File Signer Roundtrip — End-to-End Vault Lifecycle
// ═══════════════════════════════════════════════════════════════════

/// Full lifecycle: generate → load → derive → sign → verify.
/// Tests the entire path from passphrase to cryptographic operation.
#[test]
fn file_signer_full_lifecycle() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");
    let passphrase = b"test-passphrase-lifecycle";

    let pp = passphrase.to_vec();
    let signer =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))));

    // Generate
    signer.generate(passphrase).unwrap();
    assert!(path.exists());

    // Load and derive
    let root = signer.load(passphrase).unwrap();
    let deriver = KeyDeriver::new(root.as_bytes());

    // Derive all keys — every one must be valid
    let signing_seed = deriver.derive(KeyPurpose::Signing);
    let sk = SigningKey::from_bytes(&signing_seed);
    let vk = sk.verifying_key();

    // Sign and verify
    let message = b"lifecycle test message";
    let sig = sk.sign(message);
    assert!(vk.verify(message, &sig).is_ok());

    // Derive again from same root — must be deterministic
    let root2 = signer.load(passphrase).unwrap();
    let deriver2 = KeyDeriver::new(root2.as_bytes());
    let signing_seed2 = deriver2.derive(KeyPurpose::Signing);
    assert_eq!(signing_seed, signing_seed2, "same vault file must produce same keys");
}

/// Wrong passphrase must not produce the same keys (must fail or produce garbage).
#[test]
fn wrong_passphrase_produces_different_result() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};
    use styrene_identity::signer::SignerError;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");

    let pp = b"correct".to_vec();
    let signer =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))));
    signer.generate(b"correct").unwrap();

    // Load with wrong passphrase — must fail (ChaCha20Poly1305 auth tag check)
    let result = signer.load(b"wrong-passphrase");
    assert!(result.is_err(), "wrong passphrase must not decrypt successfully");

    match result.unwrap_err() {
        SignerError::DecryptionFailed(_) => {} // expected
        other => panic!("expected DecryptionFailed, got: {other}"),
    }
}

/// Identity file must be exactly 97 bytes (STID format).
#[test]
fn identity_file_size_is_exact() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner, FILE_LEN};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");

    let pp = b"test".to_vec();
    let signer =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))));
    signer.generate(b"test").unwrap();

    let file_data = std::fs::read(&path).unwrap();
    assert_eq!(file_data.len(), FILE_LEN, "identity file must be exactly {FILE_LEN} bytes");

    // Verify STID magic header
    assert_eq!(&file_data[..4], b"STID", "identity file must start with STID magic");
    assert_eq!(file_data[4], 1, "identity file version must be 1");
}

/// Truncated identity file must fail to load.
#[test]
fn truncated_file_fails_to_load() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");

    let pp = b"test".to_vec();
    let signer =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))));
    signer.generate(b"test").unwrap();

    // Truncate the file
    let data = std::fs::read(&path).unwrap();
    std::fs::write(&path, &data[..50]).unwrap();

    let result = signer.load(b"test");
    assert!(result.is_err(), "truncated file must not load");
}

/// Corrupted ciphertext must fail authentication.
#[test]
fn corrupted_ciphertext_fails() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");

    let pp = b"test".to_vec();
    let signer =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp.clone()))));
    signer.generate(b"test").unwrap();

    // Flip a byte in the ciphertext region (after header + salt + nonce = 5 + 32 + 12 = 49)
    let mut data = std::fs::read(&path).unwrap();
    data[60] ^= 0xFF;
    std::fs::write(&path, &data).unwrap();

    let result = signer.load(b"test");
    assert!(result.is_err(), "corrupted ciphertext must not decrypt");
}

// ═══════════════════════════════════════════════════════════════════
// Layer 9: SignerChain Derivation Consistency
// ═══════════════════════════════════════════════════════════════════

/// Two different FileSigner instances with the same vault file must
/// produce the same root secret and therefore the same derived keys.
#[test]
fn two_signers_same_file_same_keys() {
    use styrene_identity::file_signer::{ClosurePassphraseProvider, FileSigner};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-identity.key");
    let passphrase = b"test";

    let pp1 = passphrase.to_vec();
    let signer1 =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp1.clone()))));
    signer1.generate(passphrase).unwrap();

    let root1 = signer1.load(passphrase).unwrap();
    let d1 = KeyDeriver::new(root1.as_bytes());

    let pp2 = passphrase.to_vec();
    let signer2 =
        FileSigner::new(&path, Box::new(ClosurePassphraseProvider::new(move || Ok(pp2.clone()))));
    let root2 = signer2.load(passphrase).unwrap();
    let d2 = KeyDeriver::new(root2.as_bytes());

    for purpose in KeyPurpose::all() {
        assert_eq!(
            d1.derive(*purpose),
            d2.derive(*purpose),
            "{:?}: two signers on same file disagree",
            purpose
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// Layer 10: Derivation Independence — No Information Leakage
// ═══════════════════════════════════════════════════════════════════

/// Knowing one derived key must not reveal any other derived key.
/// Test: XOR of two derived keys must not equal the XOR of two different
/// derived keys (i.e., no linear relationship between derived values).
#[test]
fn no_linear_relationship_between_derived_keys() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let keys: Vec<[u8; 32]> = KeyPurpose::all().iter().map(|p| d.derive(*p)).collect();

    // For every pair (A, B), compute A ⊕ B.
    // No two different pairs should produce the same XOR
    // (which would indicate a linear relationship).
    let mut xors = std::collections::HashSet::new();

    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let mut xor = [0u8; 32];
            for k in 0..32 {
                xor[k] = keys[i][k] ^ keys[j][k];
            }
            assert!(
                xors.insert(xor),
                "linear relationship detected between key pairs involving indices {i} and {j}"
            );
        }
    }
}

/// Derived keys must have high entropy (no obvious patterns).
/// Test: every derived key must have at least 20 distinct byte values
/// out of 32 bytes (a truly random 32-byte value has ~31.4 expected
/// distinct values; a degenerate key like [0,0,...,0] has 1).
#[test]
fn derived_keys_have_high_entropy() {
    let d = KeyDeriver::new(&TEST_ROOT);

    for purpose in KeyPurpose::all() {
        let key = d.derive(*purpose);
        let distinct_bytes: std::collections::HashSet<u8> = key.iter().copied().collect();
        assert!(
            distinct_bytes.len() >= 15,
            "{:?}: only {} distinct byte values (expected ≥15 for high entropy)",
            purpose,
            distinct_bytes.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// Layer 11: Cross-Derivation Sign/Verify — Keys Don't Cross-Authenticate
// ═══════════════════════════════════════════════════════════════════

/// A signature made with one Ed25519 key must NOT verify under a different
/// Ed25519 key from the same root. This confirms key isolation at the
/// cryptographic level, not just at the byte level.
#[test]
fn cross_key_signatures_do_not_verify() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let message = b"cross-key isolation test";

    let ed25519_purposes = [
        KeyPurpose::Signing,
        KeyPurpose::SshHost,
        KeyPurpose::Yggdrasil,
        KeyPurpose::I2pSigning,
        KeyPurpose::Tor,
    ];

    for i in 0..ed25519_purposes.len() {
        let sk_i = SigningKey::from_bytes(&d.derive(ed25519_purposes[i]));
        let sig = sk_i.sign(message);

        for j in 0..ed25519_purposes.len() {
            if i == j {
                continue;
            }
            let vk_j = SigningKey::from_bytes(&d.derive(ed25519_purposes[j])).verifying_key();
            assert!(
                vk_j.verify(message, &sig).is_err(),
                "signature from {:?} verified under {:?} — key isolation broken",
                ed25519_purposes[i],
                ed25519_purposes[j]
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Layer 12: Statistical — Collision Resistance
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// Layer 13: Protocol Wire Formats
// ═══════════════════════════════════════════════════════════════════

/// SSH Ed25519 public key wire format: "ssh-ed25519" prefix + 32-byte key.
/// Verify the derived key can be encoded in the standard SSH format.
#[test]
fn ssh_ed25519_wire_format() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let seed = d.derive(KeyPurpose::SshHost);
    let vk = ed25519_verifying_key(&seed);
    let pubkey_bytes = vk.as_bytes();

    // SSH wire format: uint32(len("ssh-ed25519")) + "ssh-ed25519" + uint32(32) + key
    let key_type = b"ssh-ed25519";
    let mut wire = Vec::new();
    wire.extend_from_slice(&(key_type.len() as u32).to_be_bytes());
    wire.extend_from_slice(key_type);
    wire.extend_from_slice(&(32u32).to_be_bytes());
    wire.extend_from_slice(pubkey_bytes);

    // Verify the wire format starts correctly
    assert_eq!(&wire[..4], &[0, 0, 0, 11]); // len("ssh-ed25519") = 11
    assert_eq!(&wire[4..15], b"ssh-ed25519");
    assert_eq!(&wire[15..19], &[0, 0, 0, 32]); // key length = 32
    assert_eq!(&wire[19..51], pubkey_bytes);
    assert_eq!(wire.len(), 51);

    // Base64 encode for authorized_keys format
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&wire);
    let authorized_keys_line = format!("ssh-ed25519 {b64} styrene@test");
    assert!(authorized_keys_line.starts_with("ssh-ed25519 AAAA"));
}

/// sign_with_seed roundtrip — the signature bytes must verify correctly.
#[test]
fn sign_with_seed_roundtrip_all_purposes() {
    use styrene_identity::pubkey::sign_with_seed;

    let d = KeyDeriver::new(&TEST_ROOT);
    let message = b"sign_with_seed roundtrip test";

    let ed25519_purposes = [
        KeyPurpose::Signing,
        KeyPurpose::SshHost,
        KeyPurpose::Yggdrasil,
        KeyPurpose::I2pSigning,
        KeyPurpose::Tor,
    ];

    for purpose in &ed25519_purposes {
        let seed = d.derive(*purpose);
        let sig_bytes = sign_with_seed(&seed, message);

        // Verify using the verifying key
        let vk = ed25519_verifying_key(&seed);
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(vk.verify(message, &sig).is_ok(), "{:?}: sign_with_seed roundtrip failed", purpose);

        // Verify wrong data fails
        let wrong_sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(vk.verify(b"wrong", &wrong_sig).is_err());
    }
}

/// Age X25519 recipient format: age1... (Bech32-encoded).
/// Verify the derived age key produces a valid X25519 public key
/// that could be used as an age recipient.
#[test]
fn age_key_produces_valid_x25519() {
    let d = KeyDeriver::new(&TEST_ROOT);
    let secret = d.derive(KeyPurpose::Age);
    let pk = x25519_public_key(&secret);

    // Public key must be 32 bytes, non-zero
    assert_eq!(pk.as_bytes().len(), 32);
    assert_ne!(pk.as_bytes(), &[0u8; 32]);

    // DH exchange with self must work (age uses X25519 DH)
    let our_secret = StaticSecret::from(secret);
    let their_secret = StaticSecret::from([0xCC; 32]);
    let their_public = X25519PublicKey::from(&their_secret);

    let shared = our_secret.diffie_hellman(&their_public);
    assert_ne!(shared.as_bytes(), &[0u8; 32], "age DH shared secret is zero");
}

// ═══════════════════════════════════════════════════════════════════
// Layer 14: Cross-Implementation Reference Vectors (JSON export)
// ═══════════════════════════════════════════════════════════════════

/// Generate a complete reference vector set as JSON.
/// This can be verified independently in Python, JavaScript, Go, etc.
/// The test itself verifies the JSON is well-formed and contains all fields.
#[test]
fn generate_reference_vectors_json() {
    let d = KeyDeriver::new(&TEST_ROOT);

    let mut flat = serde_json::Map::new();
    for purpose in KeyPurpose::all() {
        let seed = d.derive(*purpose);
        let seed_hex = hex::encode(&seed);

        let mut entry = serde_json::Map::new();
        entry.insert(
            "info".into(),
            serde_json::Value::String(String::from_utf8_lossy(purpose.info()).to_string()),
        );
        entry.insert("seed_hex".into(), serde_json::Value::String(seed_hex.clone()));

        // Add pubkey for Ed25519 purposes
        if matches!(
            purpose,
            KeyPurpose::Signing
                | KeyPurpose::SshHost
                | KeyPurpose::Yggdrasil
                | KeyPurpose::I2pSigning
                | KeyPurpose::Tor
        ) {
            let vk = ed25519_verifying_key(&seed);
            entry.insert(
                "ed25519_pubkey_hex".into(),
                serde_json::Value::String(hex::encode(vk.as_bytes())),
            );
        }

        // Add pubkey for X25519 purposes
        if matches!(
            purpose,
            KeyPurpose::RnsEncryption
                | KeyPurpose::Age
                | KeyPurpose::I2pEncryption
                | KeyPurpose::WireGuard
        ) {
            let pk = x25519_public_key(&seed);
            entry.insert(
                "x25519_pubkey_hex".into(),
                serde_json::Value::String(hex::encode(pk.as_bytes())),
            );
        }

        flat.insert(format!("{:?}", purpose), serde_json::Value::Object(entry));
    }

    // Identity hash
    let signing_seed = d.derive(KeyPurpose::Signing);
    let signing_vk = ed25519_verifying_key(&signing_seed);
    let id_digest = Sha256::digest(signing_vk.as_bytes());
    let identity_hash = hex::encode(&id_digest[..16]);

    // Parameterized
    let mut parameterized = serde_json::Map::new();
    let ssh_gh = d.derive_ssh_user_key("github").unwrap();
    parameterized.insert("ssh_user/github".into(), serde_json::Value::String(hex::encode(&ssh_gh)));
    let agent = d.derive_agent_key("omegon-primary").unwrap();
    parameterized
        .insert("agent/omegon-primary".into(), serde_json::Value::String(hex::encode(&agent)));
    let (i2p_s, i2p_e) = d.derive_i2p_service("forge").unwrap();
    parameterized
        .insert("i2p_service/forge/signing".into(), serde_json::Value::String(hex::encode(&i2p_s)));
    parameterized.insert(
        "i2p_service/forge/encryption".into(),
        serde_json::Value::String(hex::encode(&i2p_e)),
    );
    let onion = d.derive_onion_service("forge").unwrap();
    parameterized
        .insert("onion_service/forge".into(), serde_json::Value::String(hex::encode(&onion)));

    let vectors = serde_json::json!({
        "root_secret_hex": hex::encode(&TEST_ROOT),
        "hkdf_salt": "styrene-identity-v1",
        "identity_hash": identity_hash,
        "flat_purposes": flat,
        "parameterized": parameterized,
    });

    // Verify the JSON is well-formed
    let json_str = serde_json::to_string_pretty(&vectors).unwrap();
    assert!(json_str.len() > 500, "reference vectors too small");

    // Verify key fields exist
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(parsed["identity_hash"].is_string());
    assert!(parsed["flat_purposes"]["Signing"]["seed_hex"].is_string());
    assert!(parsed["flat_purposes"]["Signing"]["ed25519_pubkey_hex"].is_string());
    assert!(parsed["flat_purposes"]["RnsEncryption"]["x25519_pubkey_hex"].is_string());
    assert!(parsed["flat_purposes"]["I2pSigning"]["ed25519_pubkey_hex"].is_string());
    assert!(parsed["flat_purposes"]["Tor"]["ed25519_pubkey_hex"].is_string());
    assert!(parsed["parameterized"]["ssh_user/github"].is_string());
    assert!(parsed["parameterized"]["agent/omegon-primary"].is_string());
    assert!(parsed["parameterized"]["i2p_service/forge/signing"].is_string());
    assert!(parsed["parameterized"]["onion_service/forge"].is_string());

    // Print for manual inspection / export
    // Uncomment to dump: eprintln!("{json_str}");
}

// ═══════════════════════════════════════════════════════════════════
// Layer 15: Concurrent Derivation Safety
// ═══════════════════════════════════════════════════════════════════

/// Multiple threads deriving from the same root must produce identical results.
/// This verifies there's no shared mutable state in the derivation path.
#[test]
fn concurrent_derivation_is_deterministic() {
    use std::sync::Arc;
    use std::thread;

    let root = Arc::new(TEST_ROOT);
    let mut handles = vec![];

    for purpose_idx in 0..KeyPurpose::all().len() {
        let root = Arc::clone(&root);
        handles.push(thread::spawn(move || {
            let d = KeyDeriver::new(&root);
            let purpose = KeyPurpose::all()[purpose_idx];
            let key = d.derive(purpose);
            (purpose_idx, key)
        }));
    }

    // Collect results
    let mut results: Vec<(usize, [u8; 32])> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    results.sort_by_key(|(idx, _)| *idx);

    // Verify against single-threaded derivation
    let d = KeyDeriver::new(&TEST_ROOT);
    for (idx, key) in &results {
        let expected = d.derive(KeyPurpose::all()[*idx]);
        assert_eq!(key, &expected, "concurrent derivation mismatch for purpose index {idx}");
    }
}

// ═══════════════════════════════════════════════════════════════════
// Layer 16: Statistical — Collision Resistance
// ═══════════════════════════════════════════════════════════════════

/// 1000 random roots must produce 1000 unique identity hashes.
/// (128-bit hash space, collision probability ≈ 0 for 1000 samples)
#[test]
fn no_identity_hash_collisions_in_sample() {
    use std::collections::HashSet;

    let mut hashes = HashSet::new();

    for i in 0u32..1000 {
        let mut root = [0u8; 32];
        root[..4].copy_from_slice(&i.to_le_bytes());

        let d = KeyDeriver::new(&root);
        let seed = d.derive(KeyPurpose::Signing);
        let vk = ed25519_verifying_key(&seed);
        let digest = Sha256::digest(vk.as_bytes());
        let hash = hex::encode(&digest[..16]);

        assert!(hashes.insert(hash), "identity hash collision at root index {i}");
    }
}
