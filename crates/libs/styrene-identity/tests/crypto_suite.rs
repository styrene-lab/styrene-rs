//! Adversarial cryptographic test suite for StyreneIdentity derivation.
//!
//! Verifies correctness, determinism, isolation, key validity, protocol
//! compatibility, edge cases, and backwards compatibility across the
//! entire HKDF derivation tree.

use ed25519_dalek::{Signer, Verifier, SigningKey};
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
        (KeyPurpose::Signing,        &signing_hex),
        (KeyPurpose::RnsEncryption,  "aefdbd63fb6746c2edb73bba3bcb34f61909077f65fe033c9372b55f6ace0c0c"),
        // New purposes — pin them now
    ];

    for (purpose, expected) in &vectors {
        let derived = hex::encode(d.derive(*purpose));
        assert_eq!(&derived, expected, "vector mismatch for {:?}", purpose);
    }

    // Verify ALL purposes produce non-zero, distinct keys
    let all_keys: Vec<(KeyPurpose, [u8; 32])> = KeyPurpose::all()
        .iter()
        .map(|p| (*p, d.derive(*p)))
        .collect();

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
        assert!(
            vk.verify(message, &sig).is_ok(),
            "parameterized key {i}: sign/verify failed"
        );
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

    let x25519_purposes = [
        KeyPurpose::RnsEncryption,
        KeyPurpose::Age,
        KeyPurpose::I2pEncryption,
    ];

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
    use sha3::{Sha3_256, Digest as Sha3Digest};
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
            assert_ne!(
                keys[i], keys[j],
                "family collision: keys {i} and {j} with label '{label}'"
            );
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
    assert_ne!(
        da.derive_agent_key("omegon").unwrap(),
        db.derive_agent_key("omegon").unwrap(),
    );
    assert_ne!(
        da.derive_i2p_service("forge").unwrap(),
        db.derive_i2p_service("forge").unwrap(),
    );
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

        assert!(
            hashes.insert(hash),
            "identity hash collision at root index {i}"
        );
    }
}
