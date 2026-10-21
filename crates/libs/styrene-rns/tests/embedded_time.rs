#![cfg(not(feature = "std"))]

//! The embedded wall-clock contract: without `std`, timestamp-dependent
//! operations fail with a typed error until the embedding supplies Unix time,
//! then announces carry the supplied time and ratchet rotation follows it.

#[path = "support/rng.rs"]
mod rng;

use rng::RecordingRng;
use rns_core::RnsError;
use rns_core::destination::{DestinationAnnounce, DestinationName, SingleInputDestination};
use rns_core::identity::PrivateIdentity;
use rns_core::packet::Packet;
use rns_core::time_source::{advance_unix_time, clear_unix_time, set_unix_time, unix_now};
use std::sync::Mutex;

/// The clock is process-global, so tests that touch it run one at a time.
static CLOCK: Mutex<()> = Mutex::new(());

fn announce_timestamp(announce: &Packet) -> u64 {
    let info = DestinationAnnounce::validate(announce).expect("valid announce");
    let mut bytes = [0u8; 8];
    bytes[3..8].copy_from_slice(&info.random_blob[5..10]);
    u64::from_be_bytes(bytes)
}

fn destination(name: &str, rng: &mut RecordingRng) -> SingleInputDestination {
    SingleInputDestination::new(
        PrivateIdentity::new_from_rand(rng),
        DestinationName::new("embedded", name),
    )
}

#[test]
fn time_is_unavailable_until_the_embedding_supplies_it() {
    let _guard = CLOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_unix_time();
    assert_eq!(unix_now(), Err(RnsError::TimeUnavailable));
    assert_eq!(advance_unix_time(5), None, "nothing to advance before initialization");

    let mut rng = RecordingRng::default();
    let mut destination = destination("unavailable", &mut rng);
    destination.enable_ratchets_in_memory();
    assert_eq!(destination.announce(&mut rng, None).map(|_| ()), Err(RnsError::TimeUnavailable));
    assert_eq!(rng.lengths(), [32, 32, 5], "failed announce does not rotate");
    assert_eq!(
        destination.path_response_with_tag(&mut rng, None, Some(&[1; 16])).map(|_| ()),
        Err(RnsError::TimeUnavailable)
    );
    assert_eq!(rng.lengths(), [32, 32, 5], "missing-time path response requests no entropy");

    set_unix_time(1_700_000_000);
    assert_eq!(unix_now(), Ok(1_700_000_000));
    let announce = destination.announce(&mut rng, None).expect("announce with supplied time");
    assert_eq!(announce_timestamp(&announce), 1_700_000_000, "five-byte timestamp suffix");

    clear_unix_time();
    assert_eq!(destination.announce(&mut rng, None).map(|_| ()), Err(RnsError::TimeUnavailable));
    assert_eq!(rng.lengths(), [32, 32, 5, 5, 32, 5]);
}

#[test]
fn supplied_time_advances_and_ratchets_rotate_on_the_same_clock() {
    let _guard = CLOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    set_unix_time(1_800_000_000);
    let mut rng = RecordingRng::default();
    let mut destination = destination("rotation", &mut rng);
    destination.enable_ratchets_in_memory();
    destination.set_ratchet_interval_secs(10).expect("interval");

    let first = destination.announce(&mut rng, None).expect("first announce");
    let first_info = DestinationAnnounce::validate(&first).expect("valid");
    let first_ratchet = first_info.ratchet.expect("ratchets are announced");
    assert_eq!(announce_timestamp(&first), 1_800_000_000);
    assert_eq!(rng.lengths(), [32, 32, 5, 32]);

    assert_eq!(advance_unix_time(4), Some(1_800_000_004));
    let second = destination.announce(&mut rng, None).expect("second announce");
    assert_eq!(announce_timestamp(&second), 1_800_000_004, "announces follow refreshed time");
    let second_info = DestinationAnnounce::validate(&second).expect("valid");
    assert_eq!(second_info.ratchet, Some(first_ratchet), "no rotation inside the interval");
    assert_eq!(rng.lengths(), [32, 32, 5, 32, 5]);

    assert_eq!(advance_unix_time(6), Some(1_800_000_010));
    let boundary = destination.announce(&mut rng, None).expect("interval boundary");
    assert_eq!(
        DestinationAnnounce::validate(&boundary).expect("valid").ratchet,
        Some(first_ratchet)
    );
    assert_eq!(rng.lengths(), [32, 32, 5, 32, 5, 5], "rotation requires strictly greater time");

    assert_eq!(advance_unix_time(1), Some(1_800_000_011));
    let third = destination.announce(&mut rng, None).expect("third announce");
    let third_info = DestinationAnnounce::validate(&third).expect("valid");
    assert_eq!(announce_timestamp(&third), 1_800_000_011);
    assert_ne!(third_info.ratchet, Some(first_ratchet), "rotation uses the supplied clock");
    assert_eq!(rng.lengths(), [32, 32, 5, 32, 5, 5, 5, 32]);

    set_unix_time(1_800_000_100);
    let refreshed = destination.announce(&mut rng, None).expect("refreshed announce");
    assert_eq!(announce_timestamp(&refreshed), 1_800_000_100);
    assert_eq!(rng.lengths(), [32, 32, 5, 32, 5, 5, 5, 32, 5, 32]);

    let salt = destination.identity.as_identity().address_hash.as_slice().to_vec();
    let mut ciphertext = rns_core::ratchets::encrypt_for_public_key(
        &x25519_dalek::PublicKey::from(first_ratchet),
        &salt,
        b"retained ratchet",
        &mut rng,
    )
    .expect("ratchet ciphertext");
    let before = rng.lengths();
    assert_eq!(
        destination.decrypt_with_ratchets(&ciphertext),
        Ok((b"retained ratchet".to_vec(), true))
    );
    destination.enforce_ratchets(true);
    *ciphertext.last_mut().expect("tag") ^= 1;
    assert_eq!(destination.decrypt_with_ratchets(&ciphertext), Err(RnsError::CryptoError));
    assert_eq!(destination.decrypt_with_ratchets(&[]), Err(RnsError::CryptoError));
    assert_eq!(rng.lengths(), before, "nested ratchet decryption requests no entropy");
    clear_unix_time();
}
