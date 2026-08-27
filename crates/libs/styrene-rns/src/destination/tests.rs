use crate::ratchets::now_secs;
use core::num::Wrapping;
use rand_core::OsRng;
use rand_core::{CryptoRng, RngCore};
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

use crate::buffer::OutputBuffer;
use crate::error::RnsError;
use crate::hash::Hash;
use crate::identity::PrivateIdentity;
use crate::serde::Serialize;

use crate::hash::AddressHash;
use crate::packet::ContextFlag;

use super::DestinationAnnounce;
use super::DestinationName;
use super::SingleInputDestination;
use super::RATCHET_LENGTH;
use super::{
    request_path_hash, IngressRegistrationError, RequestAccess, RequestDispatchError,
    RequestHandler, RequestLinkContext, RequestRegistrationError,
};

#[derive(Clone, Copy)]
struct FixedRng {
    next: Wrapping<u8>,
}

impl FixedRng {
    fn new(seed: u8) -> Self {
        Self { next: Wrapping(seed) }
    }
}

impl RngCore for FixedRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for slot in dest.iter_mut() {
            *slot = self.next.0;
            self.next += Wrapping(1);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for FixedRng {}

fn decode_announce_random_blob(announce: &crate::packet::Packet) -> [u8; 10] {
    let payload = announce.data.as_slice();
    let start = 32 + 32 + 10;
    let end = start + 10;
    let mut blob = [0u8; 10];
    blob.copy_from_slice(&payload[start..end]);
    blob
}

#[test]
fn create_announce() {
    let identity = PrivateIdentity::new_from_rand(OsRng);

    let mut single_in_destination =
        SingleInputDestination::new(identity, DestinationName::new("test", "in"));

    let announce_packet =
        single_in_destination.announce(OsRng, None).expect("valid announce packet");

    println!("Announce packet {}", announce_packet);
}

#[test]
fn create_path_request_hash() {
    let name = DestinationName::new("rnstransport", "path.request");

    println!("PathRequest Name Hash {}", name.hash);
    println!("PathRequest Destination Hash {}", Hash::new_from_slice(name.as_name_hash_slice()));
}

#[test]
fn compare_announce() {
    let priv_key: [u8; 32] = [
        0xf0, 0xec, 0xbb, 0xa4, 0x9e, 0x78, 0x3d, 0xee, 0x14, 0xff, 0xc6, 0xc9, 0xf1, 0xe1, 0x25,
        0x1e, 0xfa, 0x7d, 0x76, 0x29, 0xe0, 0xfa, 0x32, 0x41, 0x3c, 0x5c, 0x59, 0xec, 0x2e, 0x0f,
        0x6d, 0x6c,
    ];

    let sign_priv_key: [u8; 32] = [
        0xf0, 0xec, 0xbb, 0xa4, 0x9e, 0x78, 0x3d, 0xee, 0x14, 0xff, 0xc6, 0xc9, 0xf1, 0xe1, 0x25,
        0x1e, 0xfa, 0x7d, 0x76, 0x29, 0xe0, 0xfa, 0x32, 0x41, 0x3c, 0x5c, 0x59, 0xec, 0x2e, 0x0f,
        0x6d, 0x6c,
    ];

    let priv_identity = PrivateIdentity::new(priv_key.into(), sign_priv_key.into());

    println!("identity hash {}", priv_identity.as_identity().address_hash);

    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );

    println!("destination name hash {}", destination.desc.name.hash);
    println!("destination hash {}", destination.desc.address_hash);

    let announce = destination.announce(OsRng, None).expect("valid announce packet");

    let mut output_data = [0u8; 4096];
    let mut buffer = OutputBuffer::new(&mut output_data);

    let _ = announce.serialize(&mut buffer).expect("correct data");

    println!("ANNOUNCE {}", buffer);
}

#[test]
fn check_announce() {
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);

    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );

    let announce = destination.announce(OsRng, None).expect("valid announce packet");

    DestinationAnnounce::validate(&announce).expect("valid announce");
}

#[test]
fn announce_signature_covers_app_data() {
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );

    let app_data = b"Rust announce app-data";
    let announce = destination.announce(OsRng, Some(app_data)).expect("valid announce packet");

    let mut tampered = announce;
    let payload = tampered.data.as_mut_slice();
    let app_data_offset = 32 + 32 + 10 + 10 + 64;
    assert!(payload.len() > app_data_offset, "announce must include app_data");
    payload[app_data_offset] ^= 0x01;

    match DestinationAnnounce::validate(&tampered) {
        Ok(_) => panic!("tampered app_data should fail signature verification"),
        Err(err) => assert!(matches!(err, RnsError::IncorrectSignature)),
    }
}

#[test]
fn announce_includes_ratchet_when_enabled() {
    let temp = TempDir::new().expect("temp dir");
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );
    let ratchet_path = temp
        .path()
        .join("ratchets")
        .join(format!("{}.ratchets", destination.desc.address_hash.to_hex_string()));
    destination.enable_ratchets(&ratchet_path).expect("enable ratchets");

    let announce = destination.announce(OsRng, None).expect("valid announce packet");
    let info = DestinationAnnounce::validate(&announce).expect("valid announce");
    assert!(info.ratchet.is_some());
}

#[test]
fn announce_without_ratchet_flag_ignores_ratchet_bytes() {
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );

    let app_data = vec![0u8; RATCHET_LENGTH];
    let announce = destination.announce(OsRng, Some(&app_data)).expect("valid announce packet");
    let info = DestinationAnnounce::validate(&announce).expect("valid announce");
    assert!(info.ratchet.is_none());
    assert_eq!(info.app_data, app_data.as_slice());
}

#[test]
fn announce_random_blob_matches_python_layout() {
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );
    let before = now_secs().floor() as u64;
    let announce = destination.announce(FixedRng::new(0x11), None).expect("valid announce");
    let after = now_secs().floor() as u64;

    let blob = decode_announce_random_blob(&announce);
    assert_eq!(&blob[..5], &[0x11, 0x12, 0x13, 0x14, 0x15]);

    let mut ts_bytes = [0u8; 8];
    ts_bytes[3..8].copy_from_slice(&blob[5..10]);
    let emitted = u64::from_be_bytes(ts_bytes);
    assert!(emitted >= before.saturating_sub(1));
    assert!(emitted <= after.saturating_add(1));
}

#[test]
fn announce_destination_hash_mismatch_is_rejected() {
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );

    let mut announce = destination.announce(OsRng, None).expect("valid announce packet");
    announce.destination = AddressHash::new_from_slice(&[0xAAu8; 16]);

    match DestinationAnnounce::validate(&announce) {
        Ok(_) => panic!("mismatched destination hash must fail validation"),
        Err(err) => assert!(matches!(err, RnsError::IncorrectHash)),
    }
}

#[test]
fn announce_with_ratchet_bytes_but_unset_flag_is_rejected() {
    let temp = TempDir::new().expect("temp dir");
    let priv_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination = SingleInputDestination::new(
        priv_identity,
        DestinationName::new("example_utilities", "announcesample.fruits"),
    );
    let ratchet_path = temp
        .path()
        .join("ratchets")
        .join(format!("{}.ratchets", destination.desc.address_hash.to_hex_string()));
    destination.enable_ratchets(&ratchet_path).expect("enable ratchets");

    let mut announce = destination.announce(OsRng, None).expect("valid announce packet");
    // Flip the ratchet context flag off after the announce was built with ratchets
    announce.header.context_flag = ContextFlag::Unset;

    if DestinationAnnounce::validate(&announce).is_ok() {
        panic!("ratchet bytes without ratchet flag must fail validation");
    }
}

fn request_link(destination: &SingleInputDestination) -> RequestLinkContext {
    RequestLinkContext {
        link_id: AddressHash::new_from_slice(b"request-test-link"),
        destination: destination.desc.address_hash,
    }
}

fn empty_request_handler() -> RequestHandler {
    Arc::new(|_: &[u8], _: Option<&crate::identity::Identity>, _: &RequestLinkContext, _| {
        Vec::new()
    })
}

fn echo_request_handler() -> RequestHandler {
    Arc::new(|data: &[u8], _: Option<&crate::identity::Identity>, _: &RequestLinkContext, _| {
        data.to_vec()
    })
}

#[test]
fn request_path_registration_retains_canonical_path_and_hash() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("nomadnetwork", "node"));
    let path_hash = destination
        .register_request_path(
            "/page/index.mu",
            RequestAccess::Public,
            64,
            256,
            empty_request_handler(),
        )
        .expect("valid request path registration");

    assert_eq!(
        path_hash,
        [
            0xfb, 0x40, 0xab, 0xf3, 0x59, 0xb3, 0xf2, 0x5f, 0xa0, 0x08, 0x61, 0x07, 0xc5, 0xee,
            0xe5, 0x16,
        ],
        "path hash must match the Reticulum truncated SHA-256 value",
    );
    assert_eq!(path_hash, request_path_hash("/page/index.mu"));
    let registered = destination.request_path(&path_hash).expect("registered path");
    assert_eq!(registered.path(), "/page/index.mu");
    assert_eq!(registered.path_hash(), path_hash);
    assert_eq!(registered.max_request_size(), 64);
    assert_eq!(registered.max_response_size(), 256);
}

#[test]
fn duplicate_request_path_and_invalid_registration_are_rejected() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("nomadnetwork", "node"));
    destination
        .register_request_path(
            "/page/index.mu",
            RequestAccess::Public,
            1,
            1,
            empty_request_handler(),
        )
        .expect("first path registration");
    assert_eq!(
        destination.register_request_path(
            "/page/index.mu",
            RequestAccess::Public,
            1,
            1,
            empty_request_handler(),
        ),
        Err(RequestRegistrationError::DuplicatePath)
    );
    assert_eq!(
        destination.register_request_path(
            "page/no-slash",
            RequestAccess::Public,
            1,
            1,
            empty_request_handler(),
        ),
        Err(RequestRegistrationError::InvalidPath)
    );
    assert_eq!(
        destination.register_request_path(
            "/page/zero",
            RequestAccess::Public,
            0,
            1,
            empty_request_handler(),
        ),
        Err(RequestRegistrationError::InvalidLimits)
    );
    assert_eq!(
        destination.register_request_path(
            "/page/zero",
            RequestAccess::Public,
            1,
            0,
            empty_request_handler(),
        ),
        Err(RequestRegistrationError::InvalidLimits)
    );
}

#[test]
fn request_access_policies_enforce_identified_identity() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("nomadnetwork", "node"));
    let allowed = PrivateIdentity::new_from_rand(OsRng);
    let denied = PrivateIdentity::new_from_rand(OsRng);
    let callback_allowed = *destination.identity.as_identity();
    let callback_allowed_hash = callback_allowed.address_hash;
    let link = request_link(&destination);
    let request_id = [0x42; crate::hash::ADDRESS_HASH_SIZE];
    let public = destination
        .register_request_path("/public", RequestAccess::Public, 16, 16, echo_request_handler())
        .expect("public path");
    let identified = destination
        .register_request_path(
            "/identified",
            RequestAccess::Identified,
            16,
            16,
            echo_request_handler(),
        )
        .expect("identified path");
    let allow_list = destination
        .register_request_path(
            "/allowed",
            RequestAccess::AllowList(BTreeSet::from([allowed.as_identity().address_hash])),
            16,
            16,
            echo_request_handler(),
        )
        .expect("allow-list path");
    let callback = destination
        .register_request_path(
            "/callback",
            RequestAccess::Callback(Arc::new(move |remote, _| {
                remote.is_some_and(|identity| identity.address_hash == callback_allowed_hash)
            })),
            16,
            16,
            echo_request_handler(),
        )
        .expect("callback path");

    assert_eq!(
        destination.dispatch_request(&public, b"ok", None, &link, request_id),
        Ok(b"ok".to_vec())
    );
    assert_eq!(
        destination.dispatch_request(&identified, b"no", None, &link, request_id),
        Err(RequestDispatchError::Unauthorized)
    );
    assert_eq!(
        destination.dispatch_request(
            &identified,
            b"ok",
            Some(denied.as_identity()),
            &link,
            request_id,
        ),
        Ok(b"ok".to_vec())
    );
    assert_eq!(
        destination.dispatch_request(
            &allow_list,
            b"no",
            Some(denied.as_identity()),
            &link,
            request_id,
        ),
        Err(RequestDispatchError::Unauthorized)
    );
    assert_eq!(
        destination.dispatch_request(
            &allow_list,
            b"ok",
            Some(allowed.as_identity()),
            &link,
            request_id,
        ),
        Ok(b"ok".to_vec())
    );
    assert_eq!(
        destination.dispatch_request(
            &callback,
            b"no",
            Some(allowed.as_identity()),
            &link,
            request_id,
        ),
        Err(RequestDispatchError::Unauthorized)
    );
    assert_eq!(
        destination.dispatch_request(&callback, b"ok", Some(&callback_allowed), &link, request_id,),
        Ok(b"ok".to_vec())
    );
}

#[test]
fn request_and_response_size_limits_are_inclusive_and_handler_receives_context() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut destination =
        SingleInputDestination::new(identity, DestinationName::new("nomadnetwork", "node"));
    let remote = PrivateIdentity::new_from_rand(OsRng);
    let link = request_link(&destination);
    let request_id = [0x73; crate::hash::ADDRESS_HASH_SIZE];
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = Arc::clone(&calls);
    let expected_link = link;
    let expected_remote = remote.as_identity().address_hash;
    let handler = Arc::new(
        move |data: &[u8],
              remote: Option<&crate::identity::Identity>,
              context: &RequestLinkContext,
              id| {
            handler_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(data, b"1234");
            assert_eq!(remote.map(|identity| identity.address_hash), Some(expected_remote));
            assert_eq!(*context, expected_link);
            assert_eq!(id, request_id);
            vec![0x55; 5]
        },
    );
    let path = destination
        .register_request_path("/bounded", RequestAccess::Identified, 4, 5, handler)
        .expect("bounded path");

    assert_eq!(
        destination.dispatch_request(
            &path,
            b"12345",
            Some(remote.as_identity()),
            &link,
            request_id,
        ),
        Err(RequestDispatchError::RequestTooLarge)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "oversized request must not invoke handler");
    assert_eq!(
        destination
            .dispatch_request(&path, b"1234", Some(remote.as_identity()), &link, request_id,),
        Ok(vec![0x55; 5])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let oversized_response = destination
        .register_request_path(
            "/oversized-response",
            RequestAccess::Public,
            1,
            5,
            Arc::new(|_, _, _, _| vec![0; 6]),
        )
        .expect("response-limited path");
    assert_eq!(
        destination.dispatch_request(&oversized_response, b"x", None, &link, request_id),
        Err(RequestDispatchError::ResponseTooLarge)
    );
}

#[test]
fn ingress_handler_registration_is_duplicate_safe_and_unregisters() {
    let mut destination = SingleInputDestination::new(
        PrivateIdentity::new_from_name("ingress-registration"),
        DestinationName::new("test", "ingress"),
    );
    let first = Arc::new(|_: &[u8], _: &super::IngressContext| true);
    let second = Arc::new(|_: &[u8], _: &super::IngressContext| false);

    destination.register_ingress_handler(first).unwrap();
    assert_eq!(
        destination.register_ingress_handler(second),
        Err(IngressRegistrationError::DuplicateHandler)
    );
    assert!(destination.unregister_ingress_handler());
    assert!(!destination.unregister_ingress_handler());
    destination
        .register_ingress_handler(Arc::new(|_, _| true))
        .expect("slot should be reusable after unregister");
}
