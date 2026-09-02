#![cfg(feature = "transport")]

//! LinkRTT payloads against the canonical Reticulum 1.5.1 fixture authority.

mod common;

use rand_core::OsRng;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::packet::{DestinationType, Header, Packet, PacketContext, PacketDataBuffer};
use rns_core::transport::destination_ext::link::{Link, LinkEvent, LinkHandleResult};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct LinkRttMatrix {
    vectors: Vec<LinkRttVector>,
}

#[derive(Debug, Deserialize)]
struct LinkRttVector {
    id: String,
    packed_hex: String,
    rust_accepts: bool,
    #[serde(default)]
    expected_nanos: Option<u64>,
    python_unpack: PythonUnpack,
}

#[derive(Debug, Deserialize)]
struct PythonUnpack {
    outcome: String,
}

fn load_matrix() -> LinkRttMatrix {
    let index = common::load_rns_index().expect("shared RNS fixture index");
    let vector = common::rns_vector(&index, "rns-1.5.1-link-rtt-vectors").expect("vector");
    assert_eq!(vector.authority_id, "rns-1.5.1");
    let bytes = common::load_rns_vector_bytes(&index, "rns-1.5.1-link-rtt-vectors")
        .expect("checksummed artifact");
    serde_json::from_slice(&bytes).expect("link RTT matrix parses")
}

struct Pair {
    initiator: Link,
    responder: Link,
    iface: AddressHash,
    events:
        tokio::sync::broadcast::Receiver<rns_core::transport::destination_ext::link::LinkEventData>,
}

fn active_pair() -> Pair {
    let signer = PrivateIdentity::new_from_rand(OsRng);
    let identity = *signer.as_identity();
    let destination = DestinationDesc {
        identity,
        address_hash: identity.address_hash,
        name: DestinationName::new("lxmf", "rtt-parity"),
    };
    let (tx, mut events) = tokio::sync::broadcast::channel(16);
    let mut initiator = Link::new(destination, tx.clone());
    let request = initiator.request();
    let mut responder =
        Link::new_from_request(&request, signer.sign_key().clone(), destination, tx)
            .expect("link request should parse");
    let iface = AddressHash::new_from_rand(OsRng);
    assert!(matches!(
        initiator.handle_packet(&responder.prove(), iface),
        LinkHandleResult::Activated
    ));
    responder.set_ingress_iface(iface);
    while events.try_recv().is_ok() {}
    Pair { initiator, responder, iface, events }
}

fn encrypted_rtt_packet(link: &Link, payload: &[u8]) -> Packet {
    let mut data = PacketDataBuffer::new();
    let len = link.encrypt(payload, data.accuire_buf_max()).expect("encrypt RTT payload").len();
    data.resize(len);
    Packet {
        header: Header { destination_type: DestinationType::Link, ..Default::default() },
        destination: *link.id(),
        context: PacketContext::LinkRTT,
        data,
        ..Default::default()
    }
}

#[test]
fn canonical_python_rtt_payloads_activate_the_responder_and_invalid_ones_do_not() {
    let matrix = load_matrix();
    assert!(matrix.vectors.iter().any(|v| v.id == "f64-lora-slow"));
    for vector in &matrix.vectors {
        let payload = common::hex_decode(&vector.packed_hex);
        let mut pair = active_pair();
        let rtt_before = pair.responder.rtt();
        let packet = encrypted_rtt_packet(&pair.initiator, &payload);
        let outcome = pair.responder.handle_packet(&packet, pair.iface);
        assert!(matches!(outcome, LinkHandleResult::None), "{}", vector.id);
        let mut rtt_event = false;
        while let Ok(event) = pair.events.try_recv() {
            rtt_event |= matches!(event.event, LinkEvent::RttUpdated);
        }
        if vector.rust_accepts {
            assert!(rtt_event, "{}: accepted payload records the RTT", vector.id);
            if let Some(nanos) = vector.expected_nanos {
                let peer = Duration::from_nanos(nanos);
                assert!(
                    pair.responder.rtt() >= peer,
                    "{}: link RTT is at least the peer's value",
                    vector.id
                );
                if peer >= Duration::from_millis(100) {
                    assert_eq!(
                        pair.responder.rtt(),
                        peer,
                        "{}: the larger peer value wins",
                        vector.id
                    );
                }
            }
        } else {
            assert!(!rtt_event, "{}: rejected payload emits no RTT event", vector.id);
            assert_eq!(
                pair.responder.rtt(),
                rtt_before,
                "{}: rejected payload keeps the RTT",
                vector.id
            );
        }
    }
}

#[test]
fn rust_rtt_payload_is_a_canonical_64_bit_float_python_can_unpack() {
    let matrix = load_matrix();
    let pair = active_pair();
    let packet = pair.initiator.create_rtt();
    assert_eq!(packet.context, PacketContext::LinkRTT);
    let mut buffer = [0u8; 600];
    let plain =
        pair.responder.decrypt(packet.data.as_slice(), &mut buffer).expect("responder decrypts");
    assert_eq!(plain.len(), 9, "one MessagePack 64-bit float");
    assert_eq!(plain[0], 0xcb, "canonical float marker");
    let seconds = f64::from_be_bytes(plain[1..9].try_into().expect("8 bytes"));
    assert!(seconds.is_finite() && seconds >= 0.0);
    assert_eq!(seconds, pair.initiator.rtt().as_secs_f64());

    // The same encoding round-trips through every accepted canonical vector.
    for vector in matrix.vectors.iter().filter(|v| v.rust_accepts && v.id.starts_with("f64-")) {
        let payload = common::hex_decode(&vector.packed_hex);
        assert_eq!(payload[0], 0xcb, "{}", vector.id);
        assert_eq!(vector.python_unpack.outcome, "value", "{}", vector.id);
        let nanos = vector.expected_nanos.expect("accepted vectors carry nanos");
        let seconds = f64::from_be_bytes(payload[1..9].try_into().expect("8 bytes"));
        assert_eq!(Duration::from_secs_f64(seconds).as_nanos() as u64, nanos, "{}", vector.id);
    }
}
