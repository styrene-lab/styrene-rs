#![cfg(feature = "transport")]

use rns_core::hash::AddressHash;
use rns_core::packet::{Packet, PacketDataBuffer};
use rns_core::transport::iface::RxMessage;

fn message(hops: u8, physical: bool) -> RxMessage {
    let mut packet =
        Packet { data: PacketDataBuffer::new_from_slice(b"ingress"), ..Default::default() };
    packet.header.hops = hops;
    let address = AddressHash::new([0x42; 16]);
    if physical {
        RxMessage::physical(address, packet, 500)
    } else {
        RxMessage::local(address, packet)
    }
}

#[test]
fn physical_ingress_increments_hops_exactly_once() {
    let admitted = message(7, true).admit().expect("physical ingress");
    assert_eq!(admitted.packet.header.hops, 8);
    assert_eq!(admitted.admit().expect("already admitted message").packet.header.hops, 8);
}

#[test]
fn local_ingress_preserves_hops() {
    let admitted = message(7, false).admit().expect("local ingress");
    assert_eq!(admitted.packet.header.hops, 7);
}

#[test]
fn physical_hop_127_is_observable_but_not_outbound() {
    let admitted = message(127, true).admit().expect("last inbound hop");
    assert_eq!(admitted.packet.header.hops, 128);
    assert!(admitted.packet.to_bytes().is_err());
}

#[test]
fn invalid_constructed_ingress_is_rejected() {
    assert!(message(128, true).admit().is_err());
    let mut empty = message(0, false);
    empty.packet.data = PacketDataBuffer::new();
    assert!(empty.admit().is_err());
}

#[test]
fn physical_ingress_rejects_frames_over_interface_mtu() {
    let oversized = RxMessage::physical(AddressHash::new([0x42; 16]), message(0, true).packet, 20);
    assert!(oversized.admit().is_err());
}

#[test]
fn three_node_route_counts_each_physical_edge_once() {
    let source = message(0, false).admit().expect("source admission");
    assert_eq!(source.packet.header.hops, 0);

    let relay =
        RxMessage::physical(source.address, source.packet, 500).admit().expect("relay admission");
    assert_eq!(relay.packet.header.hops, 1);

    let receiver =
        RxMessage::physical(relay.address, relay.packet, 500).admit().expect("receiver admission");
    assert_eq!(receiver.packet.header.hops, 2);
}
