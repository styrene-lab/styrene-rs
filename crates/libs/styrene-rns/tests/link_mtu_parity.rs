#![cfg(feature = "transport")]

mod common;

use ed25519_dalek::Signature;
use rand_core::OsRng;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::identity::{Identity, PrivateIdentity};
use rns_core::transport::destination_ext::link::{Link, LinkHandleResult};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LinkMtuMatrix {
    disabled_discovery: DisabledDiscovery,
    unsupported_forwarding: UnsupportedForwarding,
    vectors: Vec<LinkMtuVector>,
}

#[derive(Debug, Deserialize)]
struct DisabledDiscovery {
    request_length: usize,
    signalling_hex: String,
}

#[derive(Debug, Deserialize)]
struct UnsupportedForwarding {
    request_length_after_stripping: usize,
    confirmed_mtu: usize,
}

#[derive(Debug, Deserialize)]
struct LinkMtuVector {
    mtu: usize,
    signalling_hex: String,
    request_payload_hex: String,
    request_length: usize,
    proof_payload_hex: String,
    proof_length: usize,
    proof_signed_data_hex: String,
    destination_public_key_hex: String,
    link_id_hex: String,
    decoded_request_mtu: usize,
    decoded_proof_mtu: usize,
    mode: u8,
    packet_mdu: usize,
    channel_mdu: usize,
    resource_sdu: usize,
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII fixture hex");
            u8::from_str_radix(pair, 16).expect("valid fixture hex")
        })
        .collect()
}

#[test]
fn negotiated_link_limits_match_pinned_reticulum_vectors() {
    let index = common::load_rns_index().expect("committed RNS fixture index");
    let bytes = common::load_rns_vector_bytes(&index, "rns-1.5.1-link-mtu-vectors")
        .expect("canonical link MTU fixture");
    let matrix: LinkMtuMatrix = serde_json::from_slice(&bytes).expect("valid link MTU fixture");
    assert_eq!(matrix.disabled_discovery.request_length, 67);
    assert_eq!(decode_hex(&matrix.disabled_discovery.signalling_hex), [0x20, 0x01, 0xf4]);
    assert_eq!(matrix.unsupported_forwarding.request_length_after_stripping, 64);
    assert_eq!(matrix.unsupported_forwarding.confirmed_mtu, 500);

    for vector in matrix.vectors {
        let signalling = decode_hex(&vector.signalling_hex);
        let fixture_request = decode_hex(&vector.request_payload_hex);
        let fixture_proof = decode_hex(&vector.proof_payload_hex);
        let signed_data = decode_hex(&vector.proof_signed_data_hex);
        let destination_public_key = decode_hex(&vector.destination_public_key_hex);
        let link_id = decode_hex(&vector.link_id_hex);
        assert_eq!(vector.mode, 1);
        assert_eq!(vector.decoded_request_mtu, vector.mtu);
        assert_eq!(vector.decoded_proof_mtu, vector.mtu);
        assert_eq!(fixture_request.len(), vector.request_length);
        assert_eq!(fixture_proof.len(), vector.proof_length);
        assert_eq!(&fixture_request[64..], signalling);
        assert_eq!(&fixture_proof[96..], signalling);
        let mut expected_signed_data = link_id;
        expected_signed_data.extend_from_slice(&fixture_proof[64..96]);
        expected_signed_data.extend_from_slice(&destination_public_key[32..64]);
        expected_signed_data.extend_from_slice(&signalling);
        assert_eq!(signed_data, expected_signed_data);
        let fixture_identity =
            Identity::new_from_slices(&destination_public_key[..32], &destination_public_key[32..]);
        let signature = Signature::from_slice(&fixture_proof[..64]).expect("canonical signature");
        fixture_identity
            .verify(&signed_data, &signature)
            .expect("authenticated canonical proof fixture");

        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("link", "mtu-parity"),
        };
        let (events, _) = tokio::sync::broadcast::channel(4);
        let mut outbound = Link::new(destination, events.clone());
        outbound.set_request_mtu(Some(vector.mtu));
        let request = outbound.request();
        assert_eq!(request.data.len(), vector.request_length);
        assert_eq!(&request.data.as_slice()[64..], signalling);

        let mut inbound =
            Link::new_from_request(&request, signer.sign_key().clone(), destination, events)
                .expect("canonical signalled request");
        let proof = inbound.prove();
        assert_eq!(proof.data.len(), vector.proof_length);
        assert_eq!(&proof.data.as_slice()[96..], signalling);
        assert!(matches!(
            outbound.handle_packet(&proof, AddressHash::new([7; 16])),
            LinkHandleResult::Activated
        ));
        assert_eq!(outbound.confirmed_mtu(), vector.mtu);
        assert_eq!(outbound.packet_mdu(), vector.packet_mdu);
        assert_eq!(outbound.channel_mdu(), vector.channel_mdu);
        assert_eq!(outbound.resource_sdu(), vector.resource_sdu);
        assert!(outbound.data_packet(&vec![0x41; vector.packet_mdu]).is_ok());
        assert!(outbound.data_packet(&vec![0x41; vector.packet_mdu + 1]).is_err());
        assert!(outbound.send_channel_message(0x4010, vec![0x42; vector.channel_mdu]).is_ok());
        assert!(outbound.send_channel_message(0x4011, vec![0x42; vector.channel_mdu + 1]).is_err());
    }
}

#[test]
fn legacy_request_receives_authenticated_base_mtu_proof() {
    let signer = PrivateIdentity::new_from_rand(OsRng);
    let identity = *signer.as_identity();
    let destination = DestinationDesc {
        identity,
        address_hash: identity.address_hash,
        name: DestinationName::new("link", "mtu-disabled"),
    };
    let (events, _) = tokio::sync::broadcast::channel(4);
    let mut outbound = Link::new(destination, events.clone());
    outbound.set_request_mtu(None);
    let request = outbound.request();
    assert_eq!(request.data.len(), 64);

    let mut inbound =
        Link::new_from_request(&request, signer.sign_key().clone(), destination, events)
            .expect("canonical legacy request");
    let proof = inbound.prove();
    assert_eq!(proof.data.len(), 99);
    assert_eq!(&proof.data.as_slice()[96..], &[0x20, 0x01, 0xf4]);
    assert!(matches!(
        outbound.handle_packet(&proof, AddressHash::new([8; 16])),
        LinkHandleResult::Activated
    ));
    assert_eq!(outbound.confirmed_mtu(), 500);
    assert_eq!(outbound.packet_mdu(), 431);
    assert_eq!(outbound.channel_mdu(), 425);
    assert_eq!(outbound.resource_sdu(), 464);
}
