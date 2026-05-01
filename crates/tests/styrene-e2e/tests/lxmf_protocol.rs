//! LXMF protocol-level tests.
//!
//! Tests protocol paths that exist in the LXMF crate but weren't previously
//! exercised end-to-end: propagation encryption, stamps, paper messages,
//! and DestinationStripped payload mode.

use rand_core::OsRng;
use rns_core::identity::PrivateIdentity;

// ── Propagation Encryption Roundtrip ───────────────────────────────────

#[test]
fn propagation_encrypt_decrypt_roundtrip() {
    let sender = PrivateIdentity::new_from_name("prop-sender");
    let receiver = PrivateIdentity::new_from_name("prop-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    // Build, sign, and pack a message for propagation
    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"propagation test content".to_vec()),
        Some(b"Prop Title".to_vec()),
        None,
        None,
    );
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");

    // Pack for propagation (encrypts for receiver's identity)
    let (envelope_bytes, transient_id) = wire
        .pack_propagation_with_options_and_rng(
            receiver.as_identity(),
            2000.0,
            None,
            OsRng,
        )
        .expect("pack propagation");

    assert!(!envelope_bytes.is_empty());
    assert_ne!(transient_id, [0u8; 32]);

    // Decode the outer envelope (msgpack: (timestamp, [transient_payload]))
    let (timestamp, entries): (f64, Vec<serde_bytes::ByteBuf>) =
        rmp_serde::from_slice(&envelope_bytes).expect("decode envelope");
    assert_eq!(timestamp, 2000.0);
    assert_eq!(entries.len(), 1);

    let transient_payload = entries[0].as_ref();

    // The transient payload is: destination(16) || encrypted(ephemeral_pub + fernet_token)
    assert!(transient_payload.len() > 16 + 32);

    let dest_in_payload = &transient_payload[..16];
    assert_eq!(dest_in_payload, &receiver_hash);

    let encrypted = &transient_payload[16..];

    // Decrypt using receiver's private identity
    let decrypted = lxmf::message::decrypt_for_identity(&receiver, encrypted, OsRng)
        .expect("decrypt propagation");

    // The decrypted bytes are: source(16) || signature(64) || msgpack_payload
    // This is the packed wire message minus the destination prefix
    assert!(decrypted.len() > 16 + 64);

    // Reconstruct the full wire by prepending destination
    let mut full_wire = Vec::with_capacity(16 + decrypted.len());
    full_wire.extend_from_slice(&receiver_hash);
    full_wire.extend_from_slice(&decrypted);

    // Unpack and verify
    let unpacked = lxmf::WireMessage::unpack(&full_wire).expect("unpack");
    assert_eq!(unpacked.destination, receiver_hash);
    assert_eq!(unpacked.source, sender_hash);

    // Verify signature
    assert!(
        unpacked.verify(sender.as_identity()).expect("verify"),
        "signature should be valid"
    );

    // Verify content
    assert_eq!(
        unpacked.payload.content.as_ref().map(|b| b.as_ref()),
        Some(b"propagation test content".as_slice())
    );
    assert_eq!(
        unpacked.payload.title.as_ref().map(|b| b.as_ref()),
        Some(b"Prop Title".as_slice())
    );
}

#[test]
fn propagation_decrypt_with_wrong_key_fails() {
    let sender = PrivateIdentity::new_from_name("prop-wrong-sender");
    let receiver = PrivateIdentity::new_from_name("prop-wrong-receiver");
    let wrong_receiver = PrivateIdentity::new_from_name("prop-wrong-wrong");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    let payload = lxmf::Payload::new(1.0, Some(b"secret".to_vec()), None, None, None);
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");

    let (envelope, _) = wire
        .pack_propagation_with_options_and_rng(receiver.as_identity(), 1.0, None, OsRng)
        .expect("pack");

    let (_, entries): (f64, Vec<serde_bytes::ByteBuf>) =
        rmp_serde::from_slice(&envelope).expect("decode");
    let encrypted = &entries[0].as_ref()[16..];

    // Decrypt with wrong identity should fail
    let result = lxmf::message::decrypt_for_identity(&wrong_receiver, encrypted, OsRng);
    assert!(result.is_err(), "wrong key should fail decryption");
}

// ── Stamp Generation & Validation ──────────────────────────────────────

#[test]
fn stamp_embedded_in_payload_survives_roundtrip() {
    // Create a payload with a stamp (5th element in LXMF array)
    let stamp_data = vec![0xAB; 32]; // synthetic stamp
    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"stamped content".to_vec()),
        Some(b"title".to_vec()),
        None,
        Some(stamp_data.clone()),
    );

    // Encode and decode
    let encoded = payload.to_msgpack().expect("encode");
    let decoded = lxmf::Payload::from_msgpack(&encoded).expect("decode");

    assert_eq!(decoded.stamp.as_ref().map(|b| b.as_ref()), Some(stamp_data.as_slice()));
    assert_eq!(
        decoded.content.as_ref().map(|b| b.as_ref()),
        Some(b"stamped content".as_slice())
    );
}

#[test]
fn payload_without_stamp_has_4_elements() {
    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"no stamp".to_vec()),
        Some(b"title".to_vec()),
        None,
        None,
    );

    let encoded = payload.to_msgpack().expect("encode");
    let decoded_value: rmpv::Value = rmp_serde::from_slice(&encoded).expect("decode");

    match decoded_value {
        rmpv::Value::Array(items) => {
            assert_eq!(items.len(), 4, "payload without stamp should have 4 elements");
        }
        _ => panic!("expected array"),
    }
}

#[test]
fn payload_with_stamp_has_5_elements() {
    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"stamped".to_vec()),
        Some(b"title".to_vec()),
        None,
        Some(vec![0x42; 16]),
    );

    let encoded = payload.to_msgpack().expect("encode");
    let decoded_value: rmpv::Value = rmp_serde::from_slice(&encoded).expect("decode");

    match decoded_value {
        rmpv::Value::Array(items) => {
            assert_eq!(items.len(), 5, "payload with stamp should have 5 elements");
        }
        _ => panic!("expected array"),
    }
}

// ── Paper Message URI Roundtrip ────────────────────────────────────────

#[test]
fn paper_message_encrypt_decrypt_roundtrip() {
    let sender = PrivateIdentity::new_from_name("paper-sender");
    let receiver = PrivateIdentity::new_from_name("paper-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"paper message content".to_vec()),
        Some(b"Paper Title".to_vec()),
        None,
        None,
    );
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");

    // Pack as paper message (encrypted for receiver)
    let paper_bytes = wire
        .pack_paper_with_rng(receiver.as_identity(), OsRng)
        .expect("pack paper");

    // Paper format: destination(16) || encrypted(ephemeral_pub + fernet_token)
    assert!(paper_bytes.len() > 16 + 32);
    assert_eq!(&paper_bytes[..16], &receiver_hash);

    // Decrypt
    let encrypted = &paper_bytes[16..];
    let decrypted = lxmf::message::decrypt_for_identity(&receiver, encrypted, OsRng)
        .expect("decrypt paper");

    // Reconstruct full wire
    let mut full_wire = Vec::with_capacity(16 + decrypted.len());
    full_wire.extend_from_slice(&receiver_hash);
    full_wire.extend_from_slice(&decrypted);

    let unpacked = lxmf::WireMessage::unpack(&full_wire).expect("unpack");
    assert_eq!(unpacked.source, sender_hash);
    assert!(unpacked.verify(sender.as_identity()).expect("verify"));
    assert_eq!(
        unpacked.payload.content.as_ref().map(|b| b.as_ref()),
        Some(b"paper message content".as_slice())
    );
}

#[test]
fn paper_uri_encode_decode_roundtrip() {
    let sender = PrivateIdentity::new_from_name("uri-sender");
    let receiver = PrivateIdentity::new_from_name("uri-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"QR code message".to_vec()),
        None,
        None,
        None,
    );
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");

    // Encode as lxm:// URI
    let uri = wire
        .pack_paper_uri_with_rng(receiver.as_identity(), OsRng)
        .expect("pack paper uri");

    assert!(uri.starts_with("lxm://"), "URI should start with lxm://");
    assert!(uri.len() > 20, "URI should have substantial content");

    // Decode URI back to bytes
    let decoded_bytes = lxmf::WireMessage::decode_lxm_uri(&uri).expect("decode lxm uri");

    // The decoded bytes are the paper message format: dest(16) || encrypted
    assert_eq!(&decoded_bytes[..16], &receiver_hash);

    // Decrypt and verify
    let decrypted = lxmf::message::decrypt_for_identity(&receiver, &decoded_bytes[16..], OsRng)
        .expect("decrypt uri payload");

    let mut full_wire = Vec::with_capacity(16 + decrypted.len());
    full_wire.extend_from_slice(&receiver_hash);
    full_wire.extend_from_slice(&decrypted);

    let unpacked = lxmf::WireMessage::unpack(&full_wire).expect("unpack");
    assert_eq!(
        unpacked.payload.content.as_ref().map(|b| b.as_ref()),
        Some(b"QR code message".as_slice())
    );
}

// ── DestinationStripped Payload Mode ───────────────────────────────────

#[test]
fn destination_stripped_mode_decodes_correctly() {
    let sender = PrivateIdentity::new_from_name("stripped-sender");
    let receiver = PrivateIdentity::new_from_name("stripped-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    // Build full wire message
    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"stripped content".to_vec()),
        Some(b"Stripped Title".to_vec()),
        None,
        None,
    );
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");
    let full_bytes = wire.pack().expect("pack");

    // Full wire: dest(16) || source(16) || sig(64) || msgpack_payload
    assert!(full_bytes.len() > 96);

    // DestinationStripped = wire bytes WITHOUT the first 16 bytes (destination)
    let stripped = &full_bytes[16..];

    // Decode with fallback destination
    let decoded = lxmf::inbound_decode::decode_inbound_message(
        receiver_hash,
        stripped,
        lxmf::inbound_decode::InboundPayloadMode::DestinationStripped,
    )
    .expect("decode stripped");

    assert_eq!(decoded.source, sender_hash);
    assert_eq!(decoded.destination, receiver_hash);
    assert_eq!(decoded.title, "Stripped Title");
    assert_eq!(decoded.content, "stripped content");
}

#[test]
fn full_wire_and_stripped_produce_same_message_id() {
    let sender = PrivateIdentity::new_from_name("id-sender");
    let receiver = PrivateIdentity::new_from_name("id-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    let payload = lxmf::Payload::new(
        1000.0,
        Some(b"id test".to_vec()),
        None,
        None,
        None,
    );
    let mut wire = lxmf::WireMessage::new(receiver_hash, sender_hash, payload);
    wire.sign(&sender).expect("sign");
    let full_bytes = wire.pack().expect("pack");

    // Decode both ways
    let full_decoded = lxmf::inbound_decode::decode_inbound_message(
        receiver_hash,
        &full_bytes,
        lxmf::inbound_decode::InboundPayloadMode::FullWire,
    )
    .expect("full wire");

    let stripped_decoded = lxmf::inbound_decode::decode_inbound_message(
        receiver_hash,
        &full_bytes[16..],
        lxmf::inbound_decode::InboundPayloadMode::DestinationStripped,
    )
    .expect("stripped");

    assert_eq!(
        full_decoded.id, stripped_decoded.id,
        "message ID should be identical regardless of payload mode"
    );
}
