#![cfg(feature = "interop-tests")]

mod common;

use rns_core::destination::DestinationAnnounce;
use rns_core::packet::Packet;

#[derive(serde::Deserialize)]
struct AnnounceVector {
    description: String,
    raw_packet_hex: String,
    #[allow(dead_code)]
    app_name: String,
    #[allow(dead_code)]
    aspects: String,
    has_ratchet: bool,
    app_data_hex: Option<String>,
    public_key_hex: String,
    verifying_key_hex: String,
    destination_hash_hex: String,
    #[allow(dead_code)]
    name_hash_hex: String,
    #[allow(dead_code)]
    rand_hash_hex: String,
    #[allow(dead_code)]
    signature_hex: String,
    #[allow(dead_code)]
    private_key_hex: String,
}

#[test]
fn announce_validate() {
    let vectors: Vec<AnnounceVector> = common::load_fixture("announce_vectors.json");
    assert!(!vectors.is_empty(), "no announce vectors loaded");

    for v in &vectors {
        let raw = common::hex_decode(&v.raw_packet_hex);
        let packet = Packet::from_bytes(&raw)
            .unwrap_or_else(|e| panic!("{}: packet parse failed: {e:?}", v.description));

        let info = DestinationAnnounce::validate(&packet)
            .unwrap_or_else(|e| panic!("{}: announce validation failed: {e:?}", v.description));

        // Verify the recovered identity matches
        assert_eq!(
            hex::encode(info.destination.identity.public_key_bytes()),
            v.public_key_hex,
            "{}: public key mismatch",
            v.description
        );
        assert_eq!(
            hex::encode(info.destination.identity.verifying_key_bytes()),
            v.verifying_key_hex,
            "{}: verifying key mismatch",
            v.description
        );

        // Verify destination address hash
        assert_eq!(
            hex::encode(info.destination.desc.address_hash.as_slice()),
            v.destination_hash_hex,
            "{}: destination hash mismatch",
            v.description
        );

        // Verify ratchet state
        assert_eq!(
            info.ratchet.is_some(),
            v.has_ratchet,
            "{}: ratchet presence mismatch",
            v.description
        );

        // Verify app_data
        match &v.app_data_hex {
            Some(expected_hex) => {
                assert_eq!(
                    hex::encode(info.app_data),
                    *expected_hex,
                    "{}: app_data mismatch",
                    v.description
                );
            }
            None => {
                assert!(
                    info.app_data.is_empty(),
                    "{}: expected empty app_data but got {} bytes",
                    v.description,
                    info.app_data.len()
                );
            }
        }
    }
}

#[test]
fn announce_rejects_tampered_signature() {
    let vectors: Vec<AnnounceVector> = common::load_fixture("announce_vectors.json");

    for v in &vectors {
        let mut raw = common::hex_decode(&v.raw_packet_hex);

        // Flip a bit in the signature area (near the end of packet data)
        let tamper_idx = raw.len() - 10;
        raw[tamper_idx] ^= 0x01;

        let packet = Packet::from_bytes(&raw);
        match packet {
            Ok(pkt) => {
                assert!(
                    DestinationAnnounce::validate(&pkt).is_err(),
                    "{}: tampered announce should fail validation",
                    v.description,
                );
            }
            Err(_) => {
                // Packet parse failure is also acceptable for corrupted data
            }
        }
    }
}
