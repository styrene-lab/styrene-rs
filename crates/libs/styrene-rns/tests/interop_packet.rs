#![cfg(feature = "interop-tests")]

mod common;

use rns_core::packet::{
    ContextFlag, DestinationType, HeaderType, IfacFlag, Packet, PacketContext, PacketType,
    PropagationType,
};

#[derive(serde::Deserialize)]
struct PacketVector {
    description: String,
    raw_hex: String,
    header_flags: u8,
    hops: u8,
    ifac_flag: String,
    header_type: String,
    context_flag: String,
    propagation_type: String,
    destination_type: String,
    packet_type: String,
    destination_hex: String,
    transport_hex: Option<String>,
    context: u8,
    data_hex: String,
}

fn parse_ifac(s: &str) -> IfacFlag {
    match s {
        "open" => IfacFlag::Open,
        "authenticated" => IfacFlag::Authenticated,
        other => panic!("unknown ifac_flag: {other}"),
    }
}

fn parse_header_type(s: &str) -> HeaderType {
    match s {
        "type1" => HeaderType::Type1,
        "type2" => HeaderType::Type2,
        other => panic!("unknown header_type: {other}"),
    }
}

fn parse_context_flag(s: &str) -> ContextFlag {
    match s {
        "unset" => ContextFlag::Unset,
        "set" => ContextFlag::Set,
        other => panic!("unknown context_flag: {other}"),
    }
}

fn parse_propagation(s: &str) -> PropagationType {
    match s {
        "broadcast" => PropagationType::Broadcast,
        "transport" => PropagationType::Transport,
        other => panic!("unknown propagation_type: {other}"),
    }
}

fn parse_dest_type(s: &str) -> DestinationType {
    match s {
        "single" => DestinationType::Single,
        "group" => DestinationType::Group,
        "plain" => DestinationType::Plain,
        "link" => DestinationType::Link,
        other => panic!("unknown destination_type: {other}"),
    }
}

fn parse_pkt_type(s: &str) -> PacketType {
    match s {
        "data" => PacketType::Data,
        "announce" => PacketType::Announce,
        "linkrequest" => PacketType::LinkRequest,
        "proof" => PacketType::Proof,
        other => panic!("unknown packet_type: {other}"),
    }
}

#[test]
fn packet_from_bytes() {
    let vectors: Vec<PacketVector> = common::load_fixture("packet_vectors.json");
    assert!(!vectors.is_empty(), "no packet vectors loaded");

    for v in &vectors {
        let raw = common::hex_decode(&v.raw_hex);
        let packet = Packet::from_bytes(&raw)
            .unwrap_or_else(|e| panic!("{}: from_bytes failed: {e:?}", v.description));

        // Header flags
        assert_eq!(
            packet.header.ifac_flag,
            parse_ifac(&v.ifac_flag),
            "{}: ifac_flag",
            v.description
        );
        assert_eq!(
            packet.header.header_type,
            parse_header_type(&v.header_type),
            "{}: header_type",
            v.description
        );
        assert_eq!(
            packet.header.context_flag,
            parse_context_flag(&v.context_flag),
            "{}: context_flag",
            v.description
        );
        assert_eq!(
            packet.header.propagation_type,
            parse_propagation(&v.propagation_type),
            "{}: propagation_type",
            v.description
        );
        assert_eq!(
            packet.header.destination_type,
            parse_dest_type(&v.destination_type),
            "{}: destination_type",
            v.description
        );
        assert_eq!(
            packet.header.packet_type,
            parse_pkt_type(&v.packet_type),
            "{}: packet_type",
            v.description
        );
        assert_eq!(packet.header.hops, v.hops, "{}: hops", v.description);

        // Destination
        assert_eq!(
            hex::encode(packet.destination.as_slice()),
            v.destination_hex,
            "{}: destination",
            v.description
        );

        // Transport (Type2 only)
        match (&v.transport_hex, packet.transport) {
            (Some(expected), Some(actual)) => {
                assert_eq!(
                    hex::encode(actual.as_slice()),
                    *expected,
                    "{}: transport",
                    v.description
                );
            }
            (None, None) => {}
            (Some(_), None) => panic!("{}: expected transport but got None", v.description),
            (None, Some(_)) => panic!("{}: unexpected transport present", v.description),
        }

        // Context
        assert_eq!(packet.context, PacketContext::from(v.context), "{}: context", v.description);

        // Data payload
        assert_eq!(hex::encode(packet.data.as_slice()), v.data_hex, "{}: data", v.description);
    }
}

#[test]
fn packet_roundtrip() {
    let vectors: Vec<PacketVector> = common::load_fixture("packet_vectors.json");

    for v in &vectors {
        let raw = common::hex_decode(&v.raw_hex);
        let packet = Packet::from_bytes(&raw).expect(&v.description);
        let serialized = packet
            .to_bytes()
            .unwrap_or_else(|e| panic!("{}: to_bytes failed: {e:?}", v.description));

        assert_eq!(hex::encode(&serialized), v.raw_hex, "{}: roundtrip mismatch", v.description);
    }
}

#[test]
fn header_meta_roundtrip() {
    let vectors: Vec<PacketVector> = common::load_fixture("packet_vectors.json");

    for v in &vectors {
        let meta = v.header_flags;
        let header = rns_core::packet::Header::from_meta(meta);
        let roundtripped = header.to_meta();

        assert_eq!(
            roundtripped, meta,
            "{}: header meta roundtrip failed (expected 0x{meta:02x}, got 0x{roundtripped:02x})",
            v.description
        );
    }
}
