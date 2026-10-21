#![cfg(feature = "std")]

//! Legacy MessagePack ratchet encoding, loading, and private replacement.

use rand_core::OsRng;
use rns_core::RnsError;
use rns_core::destination::{DestinationAnnounce, DestinationName, SingleInputDestination};
use rns_core::identity::PrivateIdentity;
use rns_core::ratchets::encrypt_for_public_key;
use x25519_dalek::{PublicKey, StaticSecret};

// Encode the pre-change persistence format independently of serde: an array
// containing the signature bin and the signed ratchet-list bin. Two retained
// ratchets fit in fixarray/bin8, as does the single-ratchet historical input.
fn legacy_record(identity: &PrivateIdentity, secrets: &[[u8; 32]]) -> Vec<u8> {
    assert!(secrets.len() <= 2);
    let mut packed = vec![0x90 | secrets.len() as u8];
    for secret in secrets {
        packed.extend([0xc4, 32]);
        packed.extend(secret);
    }
    let signature = identity.sign(&packed).to_bytes();
    let mut record = vec![0x92, 0xc4, 64];
    record.extend(signature);
    record.extend([0xc4, packed.len() as u8]);
    record.extend(packed);
    record
}

#[test]
fn reads_legacy_ratchets_and_preserves_encoding_on_rotation() {
    let temp = tempfile::tempdir().expect("isolated persistence directory");
    let path = temp.path().join("ratchets");
    let identity = PrivateIdentity::new_from_name("synthetic legacy ratchet store");
    let old_secret = [0x37; 32];
    std::fs::write(&path, legacy_record(&identity, &[old_secret])).expect("legacy record");
    let salt = identity.as_identity().address_hash.as_slice().to_vec();
    let old_ciphertext = encrypt_for_public_key(
        &PublicKey::from(&StaticSecret::from(old_secret)),
        &salt,
        b"legacy retained key",
        OsRng,
    )
    .expect("legacy-ratchet ciphertext");
    let mut destination =
        SingleInputDestination::new(identity.clone(), DestinationName::new("test", "persistence"));
    destination.enable_ratchets(&path).expect("load legacy record");
    destination.enforce_ratchets(true);
    assert_eq!(
        destination.decrypt_with_ratchets(&old_ciphertext),
        Ok((b"legacy retained key".to_vec(), true))
    );

    let announce = destination.announce(OsRng, None).expect("rotate and persist");
    let current = DestinationAnnounce::validate(&announce)
        .expect("valid announce")
        .ratchet
        .expect("current ratchet");
    let encoded = std::fs::read(&path).expect("persisted bytes");
    // Header: array(2), bin8(64), signature, bin8(69), array(2), bin8(32).
    assert_eq!(&encoded[..3], &[0x92, 0xc4, 64]);
    assert_eq!(&encoded[67..72], &[0xc4, 69, 0x92, 0xc4, 32]);
    let new_secret: [u8; 32] = encoded[72..104].try_into().expect("new secret");
    assert_eq!(encoded, legacy_record(&identity, &[new_secret, old_secret]));
    assert_eq!(current, *PublicKey::from(&StaticSecret::from(new_secret)).as_bytes());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777, 0o600);
    }
    let mut restarted =
        SingleInputDestination::new(identity, DestinationName::new("test", "persistence"));
    restarted.enable_ratchets(&path).expect("read rotated record");
    restarted.enforce_ratchets(true);
    assert_eq!(
        restarted.decrypt_with_ratchets(&old_ciphertext),
        Ok((b"legacy retained key".to_vec(), true))
    );
}

#[test]
fn rejected_persisted_signature_does_not_replace_the_record() {
    let temp = tempfile::tempdir().expect("isolated persistence directory");
    let path = temp.path().join("ratchets");
    let identity = PrivateIdentity::new_from_name("synthetic rejected store");
    let mut encoded = legacy_record(&identity, &[[0x37; 32]]);
    encoded[3] ^= 1;
    std::fs::write(&path, &encoded).expect("invalid record");
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("test", "persistence"));
    assert_eq!(destination.enable_ratchets(&path), Err(RnsError::IncorrectSignature));
    assert_eq!(std::fs::read(&path).expect("unchanged record"), encoded);
}
