#![cfg(all(feature = "transport", feature = "interop-tests"))]

mod common;

use rns_core::hash::AddressHash;
use rns_core::transport::interface_discovery::InterfaceDiscoveryMetadata;

#[derive(serde::Deserialize)]
struct DiscoveryCase {
    id: String,
    packed_hex: String,
    accepted: bool,
}

#[test]
fn interface_discovery_matches_pinned_reticulum_1_5_1_vectors() {
    let index = common::load_rns_index().expect("valid RNS fixture index");
    let bytes = common::load_rns_vector_bytes(&index, "rns-1.5.1-interface-discovery-vectors")
        .expect("discovery fixture");
    let cases: Vec<DiscoveryCase> = serde_json::from_slice(&bytes).expect("discovery cases");

    for case in cases {
        let packed = hex::decode(&case.packed_hex).expect("packed MessagePack hex");
        let decoded = InterfaceDiscoveryMetadata::decode(&packed);
        assert_eq!(decoded.is_ok(), case.accepted, "{}", case.id);
        if case.id == "valid-operator" {
            let decoded = decoded.as_ref().expect("valid discovery metadata");
            assert_eq!(decoded.interface_type, "TCPServerInterface");
            assert!(decoded.transport);
            assert_eq!(
                decoded.transport_id,
                AddressHash::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
            );
            assert_eq!(decoded.implementation, "RNS");
            assert_eq!(decoded.version, "1.5.1");
            assert_eq!(decoded.name, "Relay One");
            assert_eq!(
                decoded.operator_lxmf_address,
                Some(AddressHash::new([
                    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
                    0x1d, 0x1e, 0x1f,
                ]))
            );
            assert_eq!(decoded.encode().expect("re-encoded metadata"), packed);
        }
        if case.id == "absent-operator" {
            assert_eq!(
                decoded.as_ref().expect("compatible omitted operator").operator_lxmf_address,
                None
            );
        }
    }
}
