mod common;

use rns_core::buffer::{InputBuffer, OutputBuffer, StaticBuffer};
use rns_core::hash::AddressHash;
use rns_core::packet::{HeaderType, PACKET_MDU, Packet};
use rns_core::serde::Serialize;

#[derive(serde::Deserialize)]
struct AdmissionCase {
    id: String,
    raw_hex: String,
    accepted: bool,
    class: String,
}

fn canonical_packet(id: &str) -> Vec<u8> {
    let index = common::load_rns_index().expect("committed RNS fixture index");
    common::load_rns_vector_bytes(&index, id).expect("canonical packet fixture")
}

#[test]
fn canonical_packet_boundaries_reject_short_empty_and_excessive_hops() {
    for (id, minimum) in
        [("rns-1.5.1-packet-type1-hop127", 20), ("rns-1.5.1-packet-type2-hop127", 36)]
    {
        let raw = canonical_packet(id);
        assert_eq!(raw.len(), minimum);
        assert_eq!(Packet::from_bytes(&raw).expect("hop 127 is valid").header.hops, 127);
        for length in 0..minimum {
            assert!(Packet::from_bytes(&raw[..length]).is_err(), "{id} accepted length {length}");
        }
        for hops in [128, 255] {
            let mut invalid = raw.clone();
            invalid[1] = hops;
            assert!(Packet::from_bytes(&invalid).is_err(), "{id} accepted hops {hops}");
        }
    }
}

#[test]
fn canonical_rejection_matrix_is_fail_closed() {
    let cases: Vec<AdmissionCase> = common::load_fixture("rns/rns-1.5.1/packet-admission.json");
    assert_eq!(cases.len(), 7);
    for case in cases {
        assert!(!case.accepted, "{} must be a rejection vector", case.id);
        assert!(
            matches!(case.class.as_str(), "malformed_packet" | "empty_data" | "excessive_hops"),
            "{} has unknown rejection class {}",
            case.id,
            case.class
        );
        let raw = hex::decode(&case.raw_hex).expect("canonical matrix contains hex bytes");
        assert!(Packet::from_bytes(&raw).is_err(), "{} was accepted", case.id);
        assert!(
            Packet::deserialize(&mut InputBuffer::new(&raw)).is_err(),
            "{} was accepted by the buffer parser",
            case.id
        );
    }
}

#[test]
fn oversized_payload_is_rejected_instead_of_truncated() {
    let mut raw = canonical_packet("rns-1.5.1-packet-type1-hop127");
    raw.truncate(19);
    raw.extend(std::iter::repeat_n(0x5a, PACKET_MDU + 1));
    assert!(Packet::from_bytes(&raw).is_err());
}

#[test]
fn slice_and_buffer_parsers_have_identical_admission() {
    let valid = canonical_packet("rns-1.5.1-packet-type2-hop127");
    let mut cases = vec![valid.clone(), valid[..35].to_vec()];
    let mut excessive = valid.clone();
    excessive[1] = 128;
    cases.push(excessive);
    let mut oversized = valid;
    oversized.extend(std::iter::repeat_n(0x7f, PACKET_MDU));
    cases.push(oversized);

    for raw in cases {
        let from_slice = Packet::from_bytes(&raw);
        let from_buffer = Packet::deserialize(&mut InputBuffer::new(&raw));
        assert_eq!(
            from_slice.is_ok(),
            from_buffer.is_ok(),
            "parser disagreement at {} bytes",
            raw.len()
        );
    }
}

#[test]
fn outbound_serializers_reject_invalid_packet_state() {
    let valid = Packet::from_bytes(&canonical_packet("rns-1.5.1-packet-type2-hop127"))
        .expect("canonical packet");
    let mut cases = Vec::new();

    let mut empty = valid;
    empty.data = StaticBuffer::new();
    cases.push(empty);

    let mut excessive = valid;
    excessive.header.hops = 128;
    cases.push(excessive);

    let mut missing_transport = valid;
    missing_transport.header.header_type = HeaderType::Type2;
    missing_transport.transport = None;
    cases.push(missing_transport);

    for packet in cases {
        assert!(packet.to_bytes().is_err());
        let mut bytes = [0_u8; 1024];
        assert!(packet.serialize(&mut OutputBuffer::new(&mut bytes)).is_err());
    }
}

#[test]
fn valid_outbound_packet_still_round_trips() {
    let raw = canonical_packet("rns-1.5.1-packet-type1-hop127");
    let packet = Packet::from_bytes(&raw).expect("canonical packet");
    let mut destination = [0_u8; 16];
    destination.copy_from_slice(&raw[2..18]);
    assert_eq!(packet.destination, AddressHash::new(destination));
    assert_eq!(packet.to_bytes().expect("serialize canonical packet"), raw);
}
