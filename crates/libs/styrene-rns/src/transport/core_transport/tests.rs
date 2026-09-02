use super::announce::handle_announce;
use super::*;
use alloc::collections::BTreeSet;

use crate::destination::{DestinationName, SingleInputDestination};
use crate::identity::PrivateIdentity;
use crate::packet::{Header, HeaderType, PacketType};
use crate::ratchets::encrypt_for_public_key;
use crate::transport::destination_ext::link::{
    Link, LinkCloseReason, LinkEvent, LinkEventData, LinkPayload,
};
use crate::transport::iface::{InterfaceMode, InterfaceState, RxMessage, TxMessageType};
use crate::transport::resource::{ResourceEventKind, ResourceFailure};
use crate::transport::time::{ManualMonotonicClock, MonotonicClock};
use rand_core::OsRng;
use tokio::time::{Duration, timeout};

async fn test_interface_channel(
    transport: &Transport,
) -> crate::transport::iface::InterfaceChannel {
    let iface_manager = {
        let handler = transport.get_handler();
        handler.lock().await.iface_manager.clone()
    };
    iface_manager.lock().await.new_channel(8)
}

#[tokio::test]
async fn ingress_observation_and_announce_route_share_canonical_hops() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("canonical-ingress", &identity, true));
    let channel = test_interface_channel(&transport).await;
    let mut iface_events = transport.iface_rx();
    let mut announce_events = transport.recv_announces().await;
    let mut route_events = transport.route_events().await;

    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote.announce(OsRng, None).expect("canonical announce");
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, announce, 500))
        .await
        .expect("inject physical announce");

    let admitted = timeout(Duration::from_secs(1), iface_events.recv())
        .await
        .expect("ingress observation deadline")
        .expect("ingress observation");
    let announced = timeout(Duration::from_secs(1), announce_events.recv())
        .await
        .expect("announce observation deadline")
        .expect("announce observation");
    let route = timeout(Duration::from_secs(1), route_events.recv())
        .await
        .expect("route observation deadline")
        .expect("route observation");

    assert_eq!(admitted.packet.header.hops, 1);
    assert_eq!(announced.hops, admitted.packet.header.hops);
    assert_eq!(route.route.hops, admitted.packet.header.hops);
}

#[tokio::test]
async fn physical_ingress_hops_reach_received_data_without_another_increment() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport = Transport::new(TransportConfig::new("received-hops", &identity, true));
    let destination = transport
        .add_destination_checked(identity.clone(), DestinationName::new("styrene", "hops"))
        .await
        .expect("destination registration");
    let destination_hash = destination.lock().await.desc.address_hash;
    let public_identity = *identity.as_identity();
    let ciphertext = encrypt_for_public_key(
        &public_identity.public_key,
        public_identity.address_hash.as_slice(),
        b"canonical hops",
        OsRng,
    )
    .expect("destination encryption");
    let mut packet = Packet {
        destination: destination_hash,
        data: PacketDataBuffer::new_from_slice(&ciphertext),
        ..Default::default()
    };
    packet.header.hops = 7;

    let channel = test_interface_channel(&transport).await;
    let mut received_data = transport.received_data_events();
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, packet, 500))
        .await
        .expect("inject physical packet");

    let received = timeout(Duration::from_secs(1), received_data.recv())
        .await
        .expect("received data deadline")
        .expect("received data event");
    assert_eq!(received.data.as_slice(), b"canonical hops");
    assert_eq!(received.hops, Some(8));
}

async fn interface_stats_for(
    transport: &Transport,
    address: AddressHash,
) -> crate::transport::iface::InterfaceStatsSnapshot {
    transport.interface_stats().await[&address]
}

#[tokio::test]
async fn transport_exposes_canonical_ingress_queue_defaults() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("ingress-defaults", &identity, true));

    assert_eq!(
        transport.ingress_snapshot().await,
        IngressSnapshot {
            data: crate::transport::iface::IngressClassSnapshot {
                capacity: 1024,
                ..Default::default()
            },
            announce: crate::transport::iface::IngressClassSnapshot {
                capacity: 128,
                ..Default::default()
            },
            path_request: crate::transport::iface::IngressClassSnapshot {
                capacity: 128,
                ..Default::default()
            },
            ingress_limited: crate::transport::iface::IngressClassSnapshot {
                capacity: 8,
                ..Default::default()
            },
        }
    );

    let mut config = TransportConfig::new("ingress-overrides", &identity, true);
    config.set_ingress_queue_capacities(IngressQueueCapacities::new(4, 3, 2, 1).unwrap());
    let transport = Transport::new(config);
    let snapshot = transport.ingress_snapshot().await;
    assert_eq!(snapshot.data.capacity, 4);
    assert_eq!(snapshot.announce.capacity, 3);
    assert_eq!(snapshot.path_request.capacity, 2);
    assert_eq!(snapshot.ingress_limited.capacity, 1);
}

#[tokio::test]
async fn path_requests_batch_by_destination_and_answer_each_active_waiter_once() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut config = TransportConfig::new("path-batching", &identity, true);
    config.set_retransmit(true);
    let transport = Transport::new(config);
    let mut iface_a = test_interface_channel(&transport).await;
    let mut iface_b = test_interface_channel(&transport).await;
    let mut iface_c = test_interface_channel(&transport).await;
    let mut ingress = transport.iface_rx();

    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote.announce(OsRng, None).expect("matching announce");
    let destination = announce.destination;
    let first =
        transport.handler.lock().await.path_requests.generate(&destination, Some(vec![0xA1; 16]));
    let second =
        transport.handler.lock().await.path_requests.generate(&destination, Some(vec![0xB2; 16]));
    let limited =
        transport.handler.lock().await.path_requests.generate(&destination, Some(vec![0xC3; 16]));

    iface_a
        .rx_channel
        .send(RxMessage::physical(iface_a.address, first, 500))
        .await
        .expect("first path request");
    timeout(Duration::from_secs(1), ingress.recv()).await.unwrap().unwrap();
    let recursive_b =
        timeout(Duration::from_secs(1), iface_b.tx_channel.recv()).await.unwrap().unwrap();
    let recursive_c =
        timeout(Duration::from_secs(1), iface_c.tx_channel.recv()).await.unwrap().unwrap();
    assert_eq!(recursive_b.packet.data, recursive_c.packet.data);

    iface_b
        .rx_channel
        .send(RxMessage::physical(iface_b.address, second, 500))
        .await
        .expect("batched path request");
    timeout(Duration::from_secs(1), ingress.recv()).await.unwrap().unwrap();
    assert!(timeout(Duration::from_millis(50), iface_a.tx_channel.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), iface_c.tx_channel.recv()).await.is_err());

    iface_c
        .rx_channel
        .send(RxMessage::physical(iface_c.address, limited, 500).ingress_limited())
        .await
        .expect("ingress-limited path request");
    timeout(Duration::from_secs(1), ingress.recv()).await.unwrap().unwrap();
    assert_eq!(transport.path_request_snapshot().await.in_flight, 1);

    iface_c
        .rx_channel
        .send(RxMessage::physical(iface_c.address, announce, 500))
        .await
        .expect("matching announce input");
    timeout(Duration::from_secs(1), ingress.recv()).await.unwrap().unwrap();
    let response_a =
        timeout(Duration::from_secs(1), iface_a.tx_channel.recv()).await.unwrap().unwrap();
    let response_b =
        timeout(Duration::from_secs(1), iface_b.tx_channel.recv()).await.unwrap().unwrap();
    assert_eq!(response_a.packet.context, PacketContext::PathResponse);
    assert_eq!(response_b.packet.context, PacketContext::PathResponse);
    assert_eq!(response_a.packet.destination, destination);
    assert_eq!(response_b.packet.destination, destination);
    assert!(timeout(Duration::from_millis(50), iface_c.tx_channel.recv()).await.is_err());
    assert_eq!(transport.path_request_snapshot().await.in_flight, 0);
}

#[tokio::test(start_paused = true)]
async fn path_request_handler_uses_current_slowest_online_bitrate() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut config = TransportConfig::new("bitrate-path-deadline", &identity, true);
    config.set_retransmit(true);
    let transport = Transport::new(config);
    let slow = test_interface_channel(&transport).await;
    let fast = test_interface_channel(&transport).await;
    {
        let manager = transport.iface_manager.lock().await;
        assert!(manager.set_interface_bitrate(&slow.address, Some(100)));
        assert!(manager.set_interface_bitrate(&fast.address, Some(1_000)));
        assert!(manager.set_interface_state(&slow.address, InterfaceState::Active));
        assert!(manager.set_interface_state(&fast.address, InterfaceState::Active));
    }

    let destination = AddressHash::new([0x6A; 16]);
    let request =
        transport.handler.lock().await.path_requests.generate(&destination, Some(vec![0xA1; 16]));
    let now = time::Instant::now();
    let mut handler = transport.handler.lock().await;
    path::handle_path_request(&request, &mut handler, slow.address, false).await;

    assert_eq!(
        handler.path_requests.discovery_expires_at(&destination),
        Some(now + Duration::from_secs(86))
    );
}

#[tokio::test(start_paused = true)]
async fn routed_link_request_uses_selected_bitrate_hops_and_route_minimum_mtu() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("bitrate-link-deadline", &identity, true));
    let ingress = test_interface_channel(&transport).await;
    let mut egress = test_interface_channel(&transport).await;
    {
        let manager = transport.iface_manager.lock().await;
        assert!(manager.set_interface_bitrate(&egress.address, Some(500)));
        assert!(manager.set_interface_link_mtu(&ingress.address, Some(1280), true));
        assert!(manager.set_interface_link_mtu(&egress.address, Some(1024), true));
        assert!(manager.set_interface_state(&ingress.address, InterfaceState::Active));
        assert!(manager.set_interface_state(&egress.address, InterfaceState::Active));
    }

    let destination = AddressHash::new([0x6B; 16]);
    let next_hop = AddressHash::new([0x6C; 16]);
    let mut data = PacketDataBuffer::new_from_slice(&[0_u8; 64]);
    data.write(&[0x20, 0x08, 0x00]).expect("2048-byte MTU signalling");
    let packet = Packet {
        header: Header { packet_type: PacketType::LinkRequest, ..Default::default() },
        destination,
        data,
        ..Default::default()
    };
    let link_id = LinkId::from(&packet);
    let now = time::Instant::now();
    {
        let mut handler = transport.handler.lock().await;
        handler.path_table.insert_for_test(destination, next_hop, egress.address, 3);
        path::handle_link_request(&packet, ingress.address, handler).await;
    }

    assert_eq!(
        transport.handler.lock().await.link_table.proof_timeout_for_test(&link_id),
        Some(now + Duration::from_secs(26))
    );
    let forwarded = timeout(Duration::from_secs(1), egress.tx_channel.recv())
        .await
        .expect("forwarded request deadline")
        .expect("forwarded request");
    assert_eq!(forwarded.packet.data.len(), 67);
    assert_eq!(&forwarded.packet.data.as_slice()[64..], &[0x20, 0x04, 0x00]);
}

#[tokio::test]
async fn global_link_mtu_disable_signals_the_canonical_base_mtu() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut config = TransportConfig::new("disabled-link-mtu", &identity, true);
    config.set_link_mtu_discovery(false);
    let transport = Transport::new(config);
    let remote = PrivateIdentity::new_from_rand(OsRng);
    let destination = DestinationDesc {
        identity: *remote.as_identity(),
        address_hash: remote.as_identity().address_hash,
        name: DestinationName::new("link", "mtu-disabled"),
    };

    let (_, request) = transport.register_pending_outbound_link(destination).await;

    assert_eq!(request.data.len(), 67);
    assert_eq!(&request.data.as_slice()[64..], &[0x20, 0x01, 0xf4]);
}

#[tokio::test]
async fn local_path_resolution_answers_existing_batched_waiters_and_current_requester() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport =
        Transport::new(TransportConfig::new("local-path-batching", &identity, true));
    let local = transport
        .add_destination_checked(
            PrivateIdentity::new_from_rand(OsRng),
            DestinationName::new("lxmf", "delivery"),
        )
        .await
        .expect("local destination");
    let destination = local.lock().await.desc.address_hash;
    let mut iface_a = test_interface_channel(&transport).await;
    let mut iface_b = test_interface_channel(&transport).await;
    let mut iface_c = test_interface_channel(&transport).await;
    let active = BTreeSet::from([iface_a.address, iface_b.address, iface_c.address]);
    {
        let mut handler = transport.handler.lock().await;
        assert_eq!(
            handler.path_requests.register_discovery(
                &path_requests::PathRequest {
                    destination,
                    requesting_transport: None,
                    tag_bytes: vec![0xA1; 16],
                },
                iface_a.address,
                false,
                &active,
            ),
            path_requests::DiscoveryAction::StartDiscovery
        );
        assert_eq!(
            handler.path_requests.register_discovery(
                &path_requests::PathRequest {
                    destination,
                    requesting_transport: None,
                    tag_bytes: vec![0xB2; 16],
                },
                iface_b.address,
                false,
                &active,
            ),
            path_requests::DiscoveryAction::Batched
        );
    }
    let request =
        transport.handler.lock().await.path_requests.generate(&destination, Some(vec![0xC3; 16]));
    let mut ingress = transport.iface_rx();
    iface_c
        .rx_channel
        .send(RxMessage::physical(iface_c.address, request, 500))
        .await
        .expect("current path request");
    timeout(Duration::from_secs(1), ingress.recv()).await.unwrap().unwrap();

    for channel in [&mut iface_a, &mut iface_b, &mut iface_c] {
        let response =
            timeout(Duration::from_secs(1), channel.tx_channel.recv()).await.unwrap().unwrap();
        assert_eq!(response.packet.context, PacketContext::PathResponse);
        assert_eq!(response.packet.destination, destination);
    }
    assert_eq!(transport.path_request_snapshot().await.in_flight, 0);
}

#[tokio::test]
async fn invalid_announce_is_side_effect_free_and_ingress_continues() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport = Transport::new(TransportConfig::new("invalid-announce", &identity, true));
    let local = transport
        .add_destination_checked(identity.clone(), DestinationName::new("local", "announce"))
        .await
        .expect("local destination");
    let channel = test_interface_channel(&transport).await;
    let mut iface_events = transport.iface_rx();
    let mut announce_events = transport.recv_announces().await;
    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let valid = remote.announce(OsRng, None).expect("valid announce");
    let mut invalid = valid;
    let last = invalid.data.len() - 1;
    invalid.data.as_mut_slice()[last] ^= 0x01;

    let ingress_before = transport.ingress_snapshot().await;
    let outcome = channel
        .rx_channel
        .send(RxMessage::physical(channel.address, invalid, 500))
        .await
        .expect("invalid announce input");
    assert_eq!(outcome, crate::transport::iface::IngressEnqueueOutcome::Rejected);
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), announce_events.recv()).await.is_err());

    let mut local_invalid = local.lock().await.announce(OsRng, None).expect("local announce");
    let last = local_invalid.data.len() - 1;
    local_invalid.data.as_mut_slice()[last] ^= 0x01;
    let outcome = channel
        .rx_channel
        .send(RxMessage::physical(channel.address, local_invalid, 500))
        .await
        .expect("invalid local announce input");
    assert_eq!(outcome, crate::transport::iface::IngressEnqueueOutcome::Rejected);
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), announce_events.recv()).await.is_err());
    let stats = interface_stats_for(&transport, channel.address).await;
    assert_eq!(stats.rx_bytes, 0);
    assert_eq!(stats.violations.invalid_announce, 2);
    assert_eq!(stats.filters.valid_blackhole, 0);
    assert_eq!(transport.ingress_snapshot().await, ingress_before);

    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, valid, 500))
        .await
        .expect("valid announce input");
    timeout(Duration::from_secs(1), iface_events.recv())
        .await
        .expect("worker progress deadline")
        .expect("valid ingress observation");
    timeout(Duration::from_secs(1), announce_events.recv())
        .await
        .expect("announce deadline")
        .expect("valid announce");
}

#[tokio::test]
async fn valid_blackholed_announce_is_a_policy_drop_only() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let blocked_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut config = TransportConfig::new("blackhole", &identity, true);
    config.set_blackholed_identities([blocked_identity.as_identity().address_hash]);
    let mut transport = Transport::new(config);
    transport
        .add_destination_checked(blocked_identity.clone(), DestinationName::new("lxmf", "delivery"))
        .await
        .expect("blackholed local destination");
    let channel = test_interface_channel(&transport).await;
    let mut iface_events = transport.iface_rx();
    let mut announce_events = transport.recv_announces().await;
    let mut blocked =
        SingleInputDestination::new(blocked_identity, DestinationName::new("lxmf", "delivery"));

    channel
        .rx_channel
        .send(RxMessage::physical(
            channel.address,
            blocked.announce(OsRng, None).expect("signed blocked announce"),
            500,
        ))
        .await
        .expect("blocked announce input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), announce_events.recv()).await.is_err());
    let stats = interface_stats_for(&transport, channel.address).await;
    assert_eq!(stats.violations.invalid_announce, 0);
    assert_eq!(stats.filters.valid_blackhole, 1);

    let allowed_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut allowed =
        SingleInputDestination::new(allowed_identity, DestinationName::new("lxmf", "delivery"));
    channel
        .rx_channel
        .send(RxMessage::physical(
            channel.address,
            allowed.announce(OsRng, None).expect("allowed announce"),
            500,
        ))
        .await
        .expect("allowed announce input");
    timeout(Duration::from_secs(1), announce_events.recv())
        .await
        .expect("allowed announce deadline")
        .expect("allowed announce event");
}

#[tokio::test]
async fn excessive_path_tag_and_pending_link_data_are_counted_before_observation() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("typed-drops", &identity, true));
    let channel = test_interface_channel(&transport).await;
    let mut iface_events = transport.iface_rx();
    let fixed_destination = transport.handler.lock().await.fixed_dest_path_requests;
    let requested = AddressHash::new_from_rand(OsRng);
    let requesting_transport = AddressHash::new_from_rand(OsRng);
    let mut path_data = requested.as_slice().to_vec();
    path_data.extend_from_slice(requesting_transport.as_slice());
    path_data.extend_from_slice(&[0x55; crate::hash::ADDRESS_HASH_SIZE + 1]);
    let path_packet = Packet {
        header: Header { destination_type: DestinationType::Plain, ..Default::default() },
        destination: fixed_destination,
        data: PacketDataBuffer::new_from_slice(&path_data),
        ..Default::default()
    };
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, path_packet, 500))
        .await
        .expect("excessive path tag input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());

    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "pending-link-drop"),
    };
    let (pending, _) = transport.register_pending_outbound_link(destination).await;
    let pending_id = *pending.lock().await.id();
    let pending_before = pending.lock().await.state_snapshot();
    let link_packet = Packet {
        header: Header { destination_type: DestinationType::Link, ..Default::default() },
        destination: pending_id,
        data: PacketDataBuffer::new_from_slice(b"pre-validation"),
        ..Default::default()
    };
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, link_packet, 500))
        .await
        .expect("pending link input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    let pending_after = pending.lock().await.state_snapshot();
    assert_eq!(pending_after.id, pending_before.id);
    assert_eq!(pending_after.status, pending_before.status);
    assert_eq!(pending_after.interface, pending_before.interface);
    assert_eq!(pending_after.rtt, pending_before.rtt);
    assert_eq!(pending_after.remote_identity, pending_before.remote_identity);
    assert_eq!(pending_after.close_reason, pending_before.close_reason);

    let premature_proof = Packet {
        header: Header {
            destination_type: DestinationType::Link,
            packet_type: PacketType::Proof,
            ..Default::default()
        },
        destination: pending_id,
        context: PacketContext::LinkProof,
        data: PacketDataBuffer::new_from_slice(b"premature proof"),
        ..Default::default()
    };
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, premature_proof, 500))
        .await
        .expect("pending link proof input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());

    let forged_activation = Packet {
        header: Header {
            destination_type: DestinationType::Link,
            packet_type: PacketType::Proof,
            ..Default::default()
        },
        destination: pending_id,
        context: PacketContext::LinkRequestProof,
        data: PacketDataBuffer::new_from_slice(&[0xAA; 96]),
        ..Default::default()
    };
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, forged_activation, 500))
        .await
        .expect("forged activation proof input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());

    let stats = interface_stats_for(&transport, channel.address).await;
    assert_eq!(stats.rx_bytes, 0);
    assert_eq!(stats.violations.excessive_path_request_tags, 1);
    assert_eq!(stats.violations.pre_validation_link, 3);
    assert_eq!(stats.violations.malformed_frame, 0);

    let valid =
        Packet { data: PacketDataBuffer::new_from_slice(b"worker survives"), ..Default::default() };
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, valid, 500))
        .await
        .expect("valid packet input");
    timeout(Duration::from_secs(1), iface_events.recv())
        .await
        .expect("worker progress deadline")
        .expect("valid packet observation");
}

#[tokio::test]
async fn link_in_payload_is_forwarded_to_received_data() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &identity, true);
    let transport = Transport::new(config);

    let mut rx = transport.received_data_events();

    let address_hash = AddressHash::new_from_rand(OsRng);
    let payload = LinkPayload::new_from_slice(b"hello");

    let _ = transport.link_in_event_tx.send(LinkEventData {
        id: AddressHash::new_from_rand(OsRng),
        address_hash,
        interface: None,
        rtt: None,
        remote_identity: None,
        observed_at: std::time::SystemTime::now(),
        event: LinkEvent::Data(Box::new(payload)),
    });

    let received = timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("expected forwarded payload")
        .expect("broadcast receive");

    assert_eq!(received.destination, address_hash);
    assert_eq!(received.data.as_slice(), b"hello");
    assert_eq!(received.payload_mode, ReceivedPayloadMode::FullWire);
}

#[tokio::test]
async fn announce_reports_local_dispatch_failure_without_claiming_network_completion() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("announce-outcome", &identity, true));
    let destination = Arc::new(Mutex::new(SingleInputDestination::new(
        identity,
        DestinationName::new("lxmf", "delivery"),
    )));

    let outcome = transport.send_announce(&destination, None).await;

    assert_eq!(outcome, SendPacketOutcome::DroppedNoRoute);
}

#[tokio::test]
async fn checked_destination_registration_rejects_duplicate_hash_without_replacement() {
    let identity = PrivateIdentity::new_from_name("duplicate-destination-root");
    let mut transport =
        Transport::new(TransportConfig::new("duplicate-destination", &identity, true));
    let name = DestinationName::new("lxmf", "propagation");
    let original = transport
        .add_destination_checked(identity.clone(), name)
        .await
        .expect("first registration");
    let address = original.lock().await.desc.address_hash;

    let duplicate = transport.add_destination_checked(identity, name).await;

    assert!(
        matches!(duplicate, Err(DestinationRegistrationError::Duplicate(hash)) if hash == address)
    );
    let retained = transport
        .get_handler()
        .lock()
        .await
        .single_in_destinations
        .get(&address)
        .cloned()
        .expect("original retained");
    assert!(Arc::ptr_eq(&original, &retained));
}

#[tokio::test]
async fn link_out_payload_is_forwarded_to_received_data() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &identity, true);
    let transport = Transport::new(config);

    let mut rx = transport.received_data_events();

    let address_hash = AddressHash::new_from_rand(OsRng);
    let payload = LinkPayload::new_from_slice(b"outbound");

    let _ = transport.link_out_event_tx.send(LinkEventData {
        id: AddressHash::new_from_rand(OsRng),
        address_hash,
        interface: None,
        rtt: None,
        remote_identity: None,
        observed_at: std::time::SystemTime::now(),
        event: LinkEvent::Data(Box::new(payload)),
    });

    let received = timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("expected forwarded payload")
        .expect("broadcast receive");

    assert_eq!(received.destination, address_hash);
    assert_eq!(received.data.as_slice(), b"outbound");
    assert_eq!(received.payload_mode, ReceivedPayloadMode::FullWire);
}

#[tokio::test]
async fn fast_link_proof_is_processed_inside_the_link_send_path() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("fast-proof", &local_identity, true));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "fast-proof"),
    };

    let proof_iface = AddressHash::new_from_rand(OsRng);
    let handler = transport.handler.clone();
    let event_tx = transport.link_in_event_tx.clone();

    let pending = transport
        .link_with_dispatch(destination, move |message| async move {
            let request = message.expect("broadcast link request").packet;
            let mut responder =
                Link::new_from_request(&request, peer.sign_key().clone(), destination, event_tx)
                    .expect("valid link request");
            super::wire::handle_proof(responder.prove(), handler, proof_iface).await;
            SendPacketOutcome::SentBroadcast
        })
        .await;

    let pending = pending.lock().await;
    assert_eq!(pending.status(), LinkStatus::Active);
    assert_eq!(pending.ingress_iface(), Some(proof_iface));
}

#[tokio::test]
async fn concurrent_same_destination_link_creation_is_single_flight() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport =
        Arc::new(Transport::new(TransportConfig::new("single-flight", &local_identity, true)));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "single-flight"),
    };
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let first_transport = transport.clone();
    let first_entered = entered.clone();
    let first_release = release.clone();
    let first = tokio::spawn(async move {
        first_transport
            .link_with_dispatch(destination, move |_| async move {
                first_entered.notify_one();
                first_release.notified().await;
                SendPacketOutcome::SentBroadcast
            })
            .await
    });
    entered.notified().await;

    let second = transport
        .link_with_dispatch(destination, |_| async {
            panic!("second caller must reuse the registered pending link")
        })
        .await;
    release.notify_one();
    let first = first.await.expect("first link task");

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(transport.handler.lock().await.out_links.len(), 1);
}

#[tokio::test]
async fn cancellable_link_dispatch_reports_created_then_reused_pending_registration() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("link-disposition", &local_identity, true));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "link-disposition"),
    };

    let first = transport
        .link_with_dispatch_cancellable(destination, CancellationToken::new(), |_| async {
            SendPacketOutcome::SentBroadcast
        })
        .await
        .expect("created link dispatch");
    let second = transport
        .link_with_dispatch_cancellable(destination, CancellationToken::new(), |_| async {
            panic!("reused pending link must not dispatch another request")
        })
        .await
        .expect("reused link dispatch");

    let (LinkDispatch::Created(first), LinkDispatch::Reused(second)) = (first, second) else {
        panic!("link dispatch ownership was not preserved");
    };
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.lock().await.status(), LinkStatus::Pending);
}

#[tokio::test]
async fn cancelled_link_dispatch_removes_pending_real_transport_state_before_return() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport =
        Arc::new(Transport::new(TransportConfig::new("cancel-link", &local_identity, true)));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity,
        address_hash: identity.address_hash,
        name: DestinationName::new("test", "cancel-link"),
    };
    let cancellation = CancellationToken::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    let transport_task = transport.clone();
    let cancellation_task = cancellation.clone();
    let entered_task = entered.clone();
    let task = tokio::spawn(async move {
        transport_task
            .link_with_dispatch_cancellable(destination, cancellation_task, move |_| async move {
                entered_task.notify_one();
                std::future::pending::<SendPacketOutcome>().await
            })
            .await
    });
    entered.notified().await;

    cancellation.cancel();
    assert!(task.await.expect("link task").is_none());
    let lifecycle = transport.link_lifecycle_snapshot().await;
    assert!(lifecycle.active.is_empty());
    assert_eq!(lifecycle.history.len(), 1);
    assert_eq!(lifecycle.history[0].close_reason, Some(LinkCloseReason::Teardown));
    assert!(transport.handler.lock().await.out_links.is_empty());
}

#[tokio::test]
async fn cancelling_a_dispatched_pending_link_removes_real_transport_registration() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport =
        Transport::new(TransportConfig::new("cancel-dispatched-link", &local_identity, true));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity,
        address_hash: identity.address_hash,
        name: DestinationName::new("nomadnetwork", "node"),
    };
    let dispatch = transport
        .link_with_dispatch_cancellable(destination, CancellationToken::new(), |_| async {
            SendPacketOutcome::SentBroadcast
        })
        .await
        .expect("link registration accepted");
    let LinkDispatch::Created(link) = dispatch else {
        panic!("first pending link must be owned by this caller");
    };
    let link_id = *link.lock().await.id();
    assert_eq!(link.lock().await.status(), LinkStatus::Pending);

    assert!(transport.cancel_link_open(&link_id).await);
    assert!(transport.handler.lock().await.out_links.is_empty());
    let lifecycle = transport.link_lifecycle_snapshot().await;
    assert!(lifecycle.active.is_empty());
    assert_eq!(
        lifecycle.history.last().and_then(|link| link.close_reason),
        Some(LinkCloseReason::Teardown)
    );
}

#[tokio::test]
async fn failed_send_does_not_remove_a_concurrent_replacement_link() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport =
        Transport::new(TransportConfig::new("conditional-cleanup", &local_identity, true));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "conditional-cleanup"),
    };
    let handler = transport.handler.clone();
    let replacement_tx = transport.link_out_event_tx.clone();

    let original = transport
        .link_with_dispatch(destination, move |_| async move {
            let mut replacement = Link::new(destination, replacement_tx);
            replacement.request();
            handler
                .lock()
                .await
                .out_links
                .insert(destination.address_hash, Arc::new(Mutex::new(replacement)));
            SendPacketOutcome::DroppedNoRoute
        })
        .await;

    let registered = transport.handler.lock().await.out_links[&destination.address_hash].clone();
    assert!(!Arc::ptr_eq(&original, &registered));
    assert_eq!(original.lock().await.status(), LinkStatus::Closed);
    assert_eq!(registered.lock().await.status(), LinkStatus::Pending);
    let lifecycle = transport.link_lifecycle_snapshot().await;
    assert_eq!(lifecycle.history.len(), 1);
    assert_eq!(lifecycle.history[0].close_reason, Some(LinkCloseReason::SendFailure));
}

#[tokio::test]
async fn route_change_during_send_cannot_change_the_bound_proof_interface() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("route-change", &local_identity, false));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "route-change"),
    };
    let route_a = AddressHash::new_from_rand(OsRng);
    let route_b = AddressHash::new_from_rand(OsRng);
    let announce = Packet {
        header: Header { packet_type: PacketType::Announce, ..Default::default() },
        destination: destination.address_hash,
        ..Default::default()
    };
    transport.handler.lock().await.path_table.handle_announce(
        &announce,
        None,
        route_a,
        InterfaceMode::AccessPoint,
        [1; crate::destination::RAND_HASH_LENGTH],
    );
    let handler = transport.handler.clone();
    let proof_handler = transport.handler.clone();
    let event_tx = transport.link_in_event_tx.clone();

    let link = transport
        .link_with_dispatch(destination, move |message| async move {
            let message = message.expect("routed link request");
            assert_eq!(message.tx_type, TxMessageType::Direct(route_a));
            let mut responder = Link::new_from_request(
                &message.packet,
                peer.sign_key().clone(),
                destination,
                event_tx,
            )
            .expect("valid routed link request");
            let proof = responder.prove();
            {
                let mut handler = handler.lock().await;
                handler.path_table.cull(
                    std::time::Instant::now(),
                    std::time::SystemTime::now(),
                    &[],
                );
                handler.path_table.handle_announce(
                    &announce,
                    None,
                    route_b,
                    InterfaceMode::AccessPoint,
                    [2; crate::destination::RAND_HASH_LENGTH],
                );
            }
            super::wire::handle_proof(proof, proof_handler.clone(), route_b).await;
            let pending = proof_handler.lock().await.out_links[&destination.address_hash].clone();
            assert_eq!(pending.lock().await.status(), LinkStatus::Pending);
            super::wire::handle_proof(proof, proof_handler, route_a).await;
            SendPacketOutcome::SentDirect
        })
        .await;

    let link = link.lock().await;
    assert_eq!(link.status(), LinkStatus::Active);
    assert_eq!(link.ingress_iface(), Some(route_a));
}

#[tokio::test]
async fn failed_link_request_send_removes_pending_registration() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("send-failure", &local_identity, false));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "send-failure"),
    };

    let link = transport.link(destination).await;

    assert_eq!(link.lock().await.status(), LinkStatus::Closed);
    assert!(transport.find_out_link(&destination.address_hash).await.is_none());
}

#[tokio::test]
async fn supervised_scheduler_retries_channel_to_bounded_failure() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let clock = Arc::new(ManualMonotonicClock::default());
    let transport = Transport::new_with_protocol_clock(
        TransportConfig::new("channel-deadline", &local_identity, true),
        clock.clone(),
    );
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "channel-deadline"),
    };
    let iface = AddressHash::new_from_rand(OsRng);
    let (outbound, request) = transport.register_pending_outbound_link(destination).await;
    let mut inbound = Link::new_from_request(
        &request,
        peer.sign_key().clone(),
        destination,
        transport.link_in_event_tx.clone(),
    )
    .expect("link request");
    let proof = inbound.prove();
    assert!(matches!(
        outbound.lock().await.handle_packet(&proof, iface),
        crate::transport::destination_ext::link::LinkHandleResult::Activated
    ));
    let (sequence, _) = outbound
        .lock()
        .await
        .send_channel_message_at(0x100, b"retry".to_vec(), clock.now())
        .expect("channel send");

    for _ in 1..=6 {
        clock.advance(Duration::from_secs(60));
        super::jobs::handle_protocol_deadlines(transport.handler.lock().await).await;
    }

    let outbound = outbound.lock().await;
    assert_eq!(outbound.channel_state(sequence), crate::transport::channel::MessageState::Failed);
    assert_eq!(outbound.status(), LinkStatus::Closed);
    assert_eq!(outbound.state_snapshot().close_reason, Some(LinkCloseReason::ChannelTimeout));
}

#[tokio::test]
async fn duplicate_channel_packet_replays_proof_without_redelivery() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport =
        Transport::new(TransportConfig::new("channel-proof-loss", &local_identity, true));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "channel-proof-loss"),
    };
    let iface = AddressHash::new_from_rand(OsRng);
    let (outbound, request) = transport.register_pending_outbound_link(destination).await;
    let mut inbound = Link::new_from_request(
        &request,
        peer.sign_key().clone(),
        destination,
        transport.link_in_event_tx.clone(),
    )
    .expect("link request");
    assert!(matches!(
        outbound.lock().await.handle_packet(&inbound.prove(), iface),
        crate::transport::destination_ext::link::LinkHandleResult::Activated
    ));
    let deliveries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = deliveries.clone();
    inbound.register_channel_handler(0x101, move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    });
    let (_, packet) = outbound
        .lock()
        .await
        .send_channel_message(0x101, b"proof-loss".to_vec())
        .expect("channel packet");

    assert!(transport.handler.lock().await.filter_duplicate_packets(&packet).await);
    assert!(matches!(inbound.handle_packet(&packet, iface), LinkHandleResult::Proof(_)));
    assert!(transport.handler.lock().await.filter_duplicate_packets(&packet).await);
    assert!(matches!(inbound.handle_packet(&packet, iface), LinkHandleResult::Proof(_)));
    assert_eq!(deliveries.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn cache_request_replays_only_matching_resource_proof() {
    let link_id = AddressHash::new_from_rand(OsRng);
    let proof = Packet {
        header: Header {
            destination_type: DestinationType::Link,
            packet_type: PacketType::Proof,
            ..Default::default()
        },
        destination: link_id,
        context: PacketContext::ResourceProof,
        data: PacketDataBuffer::new_from_slice(&[0x5a; HASH_SIZE * 2]),
        ..Default::default()
    };
    let mut cache = super::packet_cache::PacketCache::new();
    cache.update(&proof);
    let request = Packet {
        header: Header { destination_type: DestinationType::Link, ..Default::default() },
        destination: link_id,
        context: PacketContext::CacheRequest,
        data: PacketDataBuffer::new_from_slice(proof.hash().as_slice()),
        ..Default::default()
    };

    assert_eq!(super::wire::cached_resource_proof(&cache, &request), Some(proof));
    let wrong_link = Packet { destination: AddressHash::new_from_rand(OsRng), ..request };
    assert!(super::wire::cached_resource_proof(&cache, &wrong_link).is_none());
}

#[tokio::test]
async fn aborted_resource_dispatches_are_reaped_by_injected_clock() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let clock = Arc::new(ManualMonotonicClock::default());
    let mut config = TransportConfig::new("resource-abort", &local_identity, true);
    config.set_resource_retry_interval_secs(1);
    let transport = Arc::new(Transport::new_with_protocol_clock(config, clock.clone()));
    let peer = PrivateIdentity::new_from_rand(OsRng);
    let peer_identity = *peer.as_identity();
    let destination = DestinationDesc {
        identity: peer_identity,
        address_hash: peer_identity.address_hash,
        name: DestinationName::new("test", "resource-abort"),
    };
    let iface = AddressHash::new_from_rand(OsRng);
    let (outbound, request) = transport.register_pending_outbound_link(destination).await;
    let mut inbound = Link::new_from_request(
        &request,
        peer.sign_key().clone(),
        destination,
        transport.link_in_event_tx.clone(),
    )
    .expect("link request");
    assert!(matches!(
        outbound.lock().await.handle_packet(&inbound.prove(), iface),
        LinkHandleResult::Activated
    ));
    let mut events = transport.resource_events();

    for is_response in [false, true] {
        let entered = Arc::new(tokio::sync::Notify::new());
        let task_transport = transport.clone();
        let task_link = outbound.clone();
        let task_entered = entered.clone();
        let task = tokio::spawn(async move {
            let dispatch = move |_| async move {
                task_entered.notify_one();
                std::future::pending::<bool>().await
            };
            if is_response {
                task_transport
                    .respond_to_link_request_resource_with_dispatch(
                        task_link,
                        [0x42; crate::hash::ADDRESS_HASH_SIZE],
                        vec![0x51; 1024],
                        dispatch,
                    )
                    .await
            } else {
                task_transport
                    .send_resource_with_dispatch(task_link, vec![0x51; 1024], None, dispatch)
                    .await
            }
        });
        entered.notified().await;
        assert_eq!(transport.resource_state_counts().await.pending_outgoing, 1);
        task.abort();
        let _ = task.await;

        clock.advance(Duration::from_secs(2));
        super::jobs::handle_protocol_deadlines(transport.handler.lock().await).await;
        assert_eq!(transport.resource_state_counts().await.total(), 0);
        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("terminal resource event")
            .expect("resource event channel");
        assert!(matches!(event.kind, ResourceEventKind::Failed(ResourceFailure::TimedOut)));
    }
}

#[tokio::test]
async fn protocol_scheduler_is_owned_and_joined_by_transport() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("scheduler-supervision", &identity, true));
    assert!(!transport.manager_task_finished());
    timeout(Duration::from_secs(1), transport.shutdown_manager())
        .await
        .expect("manager shutdown deadline")
        .expect("manager task must not panic");
    assert!(transport.manager_task_finished());
    assert_eq!(
        transport.supervision_outcome().await,
        Some(SupervisionOutcome::Shutdown { drained: true }),
        "ordinary cancellation must drain every worker and report no failure"
    );
    assert_eq!(transport.worker_failure().await, None);
}

#[tokio::test]
async fn transport_supervision_is_pending_while_workers_run() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("supervision-live", &identity, false));
    tokio::task::yield_now().await;
    assert_eq!(transport.supervision_outcome().await, None);
    assert!(!transport.manager_task_finished());
    timeout(Duration::from_secs(1), transport.shutdown_manager())
        .await
        .expect("manager shutdown deadline")
        .expect("manager task must not panic");
    assert_eq!(transport.worker_failure().await, None);
}

#[test]
fn only_data_uses_generic_rebroadcast_path() {
    assert!(super::jobs::should_rebroadcast(PacketType::Data));
    assert!(!super::jobs::should_rebroadcast(PacketType::Announce));
    assert!(!super::jobs::should_rebroadcast(PacketType::LinkRequest));
    assert!(!super::jobs::should_rebroadcast(PacketType::Proof));
}

#[tokio::test]
async fn drop_duplicates() {
    let mut config: TransportConfig = Default::default();
    config.set_retransmit(true);

    let transport = Transport::new(config);
    let handler = transport.get_handler();

    let _source1 = AddressHash::new_from_slice(&[1u8; 32]);
    let _source2 = AddressHash::new_from_slice(&[2u8; 32]);
    let next_hop_iface = AddressHash::new_from_slice(&[3u8; 32]);
    let destination = AddressHash::new_from_slice(&[4u8; 32]);

    let mut announce: Packet = Default::default();
    announce.header.header_type = HeaderType::Type2;
    announce.header.packet_type = PacketType::Announce;
    announce.header.hops = 3;
    announce.transport = Some(destination);

    assert!(handler.lock().await.filter_duplicate_packets(&announce).await);

    handle_announce(&announce, handler.lock().await, next_hop_iface).await;

    let data_packet: Packet = Packet {
        data: PacketDataBuffer::new_from_slice(b"foo"),
        destination,
        ..Default::default()
    };
    let duplicate: Packet = data_packet;

    let mut different_packet = data_packet;
    different_packet.data = PacketDataBuffer::new_from_slice(b"bar");

    assert!(handler.lock().await.filter_duplicate_packets(&data_packet).await);
    assert!(!handler.lock().await.filter_duplicate_packets(&duplicate).await);
    assert!(handler.lock().await.filter_duplicate_packets(&different_packet).await);

    tokio::time::sleep(Duration::from_secs(2)).await;
    handler.lock().await.packet_cache.lock().await.release(Duration::from_secs(1));

    // Packet should have been removed from cache (stale)
    assert!(handler.lock().await.filter_duplicate_packets(&duplicate).await);
}

#[tokio::test]
async fn announce_retransmit_key_uses_destination_hash() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut config = TransportConfig::new("test", &local_identity, true);
    config.set_retransmit(true);
    let transport = Transport::new(config);
    let handler = transport.get_handler();

    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");

    let announced_destination = announce.destination;
    let announced_identity = *remote_destination.identity.address_hash();
    assert_ne!(
        announced_destination, announced_identity,
        "destination hash must differ from identity hash for named destinations"
    );

    let iface = AddressHash::new_from_rand(OsRng);
    handle_announce(&announce, handler.lock().await, iface).await;
    tokio::time::sleep(Duration::from_millis(550)).await;

    let mut guard = handler.lock().await;
    let transport_id = *guard.config.identity.address_hash();
    let keyed_by_destination =
        guard.announce_table.new_packet(&announced_destination, &transport_id);
    assert!(
        keyed_by_destination.is_some(),
        "announce retransmit should be keyed by destination hash"
    );
    let keyed_by_identity = guard.announce_table.new_packet(&announced_identity, &transport_id);
    assert!(
        keyed_by_identity.is_none(),
        "identity hash must not be used as announce retransmit key"
    );
}

#[tokio::test]
async fn send_packet_with_outcome_reports_missing_identity() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &identity, true);
    let transport = Transport::new(config);

    let packet = Packet { destination: AddressHash::new_from_rand(OsRng), ..Default::default() };
    let outcome = transport.send_packet_with_outcome(packet).await;

    assert_eq!(outcome, SendPacketOutcome::DroppedMissingDestinationIdentity);
}

#[tokio::test]
async fn send_packet_with_outcome_reports_no_route() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &identity, false);
    let transport = Transport::new(config);

    let packet = Packet {
        header: Header { packet_type: PacketType::Data, ..Default::default() },
        context: PacketContext::KeepAlive,
        data: PacketDataBuffer::new_from_slice(&[KEEP_ALIVE_REQUEST]),
        destination: AddressHash::new_from_rand(OsRng),
        ..Default::default()
    };
    let outcome = transport.send_packet_with_outcome(packet).await;

    assert_eq!(outcome, SendPacketOutcome::DroppedNoRoute);
}

#[tokio::test]
async fn send_packet_with_outcome_drops_announce_without_route() {
    let identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &identity, false);
    let transport = Transport::new(config);

    let packet = Packet {
        header: Header { packet_type: PacketType::Announce, ..Default::default() },
        destination: AddressHash::new_from_rand(OsRng),
        ..Default::default()
    };
    let outcome = transport.send_packet_with_outcome(packet).await;

    assert_eq!(outcome, SendPacketOutcome::DroppedNoRoute);
}

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct CountingReceiptHandler {
    count: Arc<AtomicUsize>,
}

impl ReceiptHandler for CountingReceiptHandler {
    fn on_receipt(&self, _receipt: &DeliveryReceipt) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

struct ReentrantReceiptHandler {
    transport: Arc<Mutex<TransportHandler>>,
    callbacks: Arc<AtomicUsize>,
    sends: Arc<AtomicUsize>,
}

impl ReceiptHandler for ReentrantReceiptHandler {
    fn on_receipt(&self, _receipt: &DeliveryReceipt) {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        let outcome = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.transport
                    .lock()
                    .await
                    .send_packet_with_outcome(Packet {
                        destination: AddressHash::new([0x91; crate::hash::ADDRESS_HASH_SIZE]),
                        data: PacketDataBuffer::new_from_slice(b"receipt callback send"),
                        ..Default::default()
                    })
                    .await
            })
        });
        if outcome == SendPacketOutcome::DroppedMissingDestinationIdentity {
            self.sends.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[tokio::test]
async fn handle_inbound_for_test_rejects_forged_destination_proof() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &local_identity, true);
    let mut transport = Transport::new(config);
    let handler = transport.get_handler();

    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;

    let count = Arc::new(AtomicUsize::new(0));
    transport.set_receipt_handler(Box::new(CountingReceiptHandler { count: count.clone() })).await;

    let mut data = PacketDataBuffer::new();
    data.safe_write(&[0x44u8; HASH_SIZE]);
    data.safe_write(&[0xAAu8; ed25519_dalek::SIGNATURE_LENGTH]);
    let packet = Packet {
        header: Header { packet_type: PacketType::Proof, ..Default::default() },
        destination: announce.destination,
        context: PacketContext::None,
        data,
        ..Default::default()
    };

    transport.handle_inbound_for_test(packet).await;

    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn handle_inbound_for_test_accepts_valid_destination_proof() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &local_identity, true);
    let mut transport = Transport::new(config);
    let handler = transport.get_handler();

    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;

    let count = Arc::new(AtomicUsize::new(0));
    transport.set_receipt_handler(Box::new(CountingReceiptHandler { count: count.clone() })).await;

    let packet_hash = [0x55u8; HASH_SIZE];
    let signature = remote_destination.identity.sign(&packet_hash).to_bytes();
    let mut data = PacketDataBuffer::new();
    data.safe_write(&packet_hash);
    data.safe_write(&signature);
    let packet = Packet {
        header: Header { packet_type: PacketType::Proof, ..Default::default() },
        destination: announce.destination,
        context: PacketContext::None,
        data,
        ..Default::default()
    };

    transport.handle_inbound_for_test(packet).await;

    assert_eq!(count.load(Ordering::SeqCst), 1);
}

/// Build a remote destination announced to the transport plus a data packet
/// addressed to it, registered as a transmitted provable packet.
async fn pending_packet_fixture(
    transport: &mut Transport,
) -> (SingleInputDestination, AddressHash, [u8; HASH_SIZE], Arc<AtomicUsize>) {
    let handler = transport.get_handler();
    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;

    let count = Arc::new(AtomicUsize::new(0));
    transport.set_receipt_handler(Box::new(CountingReceiptHandler { count: count.clone() })).await;

    let data_packet = Packet {
        destination: announce.destination,
        data: PacketDataBuffer::new_from_slice(b"opportunistic lxmf payload"),
        ..Default::default()
    };
    let packet_hash = data_packet.hash().to_bytes();
    handler.lock().await.register_pending_packet_receipt(packet_hash, announce.destination);
    (remote_destination, announce.destination, packet_hash, count)
}

fn proof_addressed_to_packet(packet_hash: [u8; HASH_SIZE], data: &[u8]) -> Packet {
    Packet {
        header: Header { packet_type: PacketType::Proof, ..Default::default() },
        destination: AddressHash::new_from_hash(&Hash::new(packet_hash)),
        context: PacketContext::None,
        data: PacketDataBuffer::new_from_slice(data),
        ..Default::default()
    }
}

#[tokio::test]
async fn implicit_proof_addressed_to_transmitted_packet_hash_is_accepted() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport = Transport::new(TransportConfig::new("test", &local_identity, true));
    let (remote_destination, _, packet_hash, count) = pending_packet_fixture(&mut transport).await;

    let signature = remote_destination.identity.sign(&packet_hash).to_bytes();
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &signature)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // The concluded receipt is terminal: a replayed proof is ignored.
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &signature)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn explicit_proof_addressed_to_transmitted_packet_hash_is_accepted() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport = Transport::new(TransportConfig::new("test", &local_identity, true));
    let (remote_destination, _, packet_hash, count) = pending_packet_fixture(&mut transport).await;

    let mut data = Vec::from(packet_hash);
    data.extend_from_slice(&remote_destination.identity.sign(&packet_hash).to_bytes());
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &data)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proofs_for_transmitted_packets_reject_forged_or_unknown_evidence() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut transport = Transport::new(TransportConfig::new("test", &local_identity, true));
    let (remote_destination, _, packet_hash, count) = pending_packet_fixture(&mut transport).await;

    // Signed by a stranger.
    let stranger = PrivateIdentity::new_from_rand(OsRng);
    let forged = stranger.sign(&packet_hash).to_bytes();
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &forged)).await;

    // Explicit proof whose embedded hash disagrees with the transmitted packet.
    let mut mismatched = Vec::from([0x11u8; HASH_SIZE]);
    mismatched
        .extend_from_slice(&remote_destination.identity.sign(&[0x11u8; HASH_SIZE]).to_bytes());
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &mismatched)).await;

    // Valid signature over a packet this transport never sent.
    let unknown_hash = [0x77u8; HASH_SIZE];
    let unknown = remote_destination.identity.sign(&unknown_hash).to_bytes();
    transport.handle_inbound_for_test(proof_addressed_to_packet(unknown_hash, &unknown)).await;

    // Malformed length.
    transport.handle_inbound_for_test(proof_addressed_to_packet(packet_hash, &[0u8; 40])).await;

    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn implicit_proof_built_for_a_received_packet_is_accepted_by_the_sender_side() {
    // Receiver side: the destination that got the packet builds the proof.
    let receiver_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut receiver_destination =
        SingleInputDestination::new(receiver_identity, DestinationName::new("lxmf", "delivery"));
    let announce = receiver_destination.announce(OsRng, None).expect("valid announce packet");

    // Sender side: a transport that transmitted a packet to that destination.
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut sender = Transport::new(TransportConfig::new("sender", &local_identity, true));
    let handler = sender.get_handler();
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;
    let count = Arc::new(AtomicUsize::new(0));
    sender.set_receipt_handler(Box::new(CountingReceiptHandler { count: count.clone() })).await;
    let packet = Packet {
        destination: announce.destination,
        data: PacketDataBuffer::new_from_slice(b"opportunistic lxmf payload"),
        ..Default::default()
    };
    let packet_hash = packet.hash().to_bytes();
    handler.lock().await.register_pending_packet_receipt(packet_hash, announce.destination);

    let proof =
        super::wire::build_implicit_packet_proof(&receiver_destination.identity, packet_hash);
    assert_eq!(proof.header.packet_type, PacketType::Proof);
    assert_eq!(proof.context, PacketContext::None);
    assert_eq!(proof.destination, AddressHash::new_from_hash(&Hash::new(packet_hash)));
    assert_eq!(
        proof.data.len(),
        ed25519_dalek::SIGNATURE_LENGTH,
        "implicit proofs carry only a signature"
    );

    sender.handle_inbound_for_test(proof).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // A proof for the same packet signed by another destination is rejected.
    let stranger = PrivateIdentity::new_from_rand(OsRng);
    handler.lock().await.register_pending_packet_receipt([0x66; HASH_SIZE], announce.destination);
    sender
        .handle_inbound_for_test(super::wire::build_implicit_packet_proof(
            &stranger,
            [0x66; HASH_SIZE],
        ))
        .await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prove_received_packet_requires_a_local_input_destination() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("receiver", &local_identity, true));
    assert!(
        !transport
            .prove_received_packet(AddressHash::new_from_rand(OsRng), [0x11; HASH_SIZE], None)
            .await,
        "unknown destinations must not be proved"
    );
}

#[tokio::test]
async fn transmitted_single_packets_report_their_hash_and_register_pending_receipts() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("test", &local_identity, true));
    let handler = transport.get_handler();
    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "delivery"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;

    let packet = Packet {
        destination: announce.destination,
        data: PacketDataBuffer::new_from_slice(b"no interface is attached"),
        ..Default::default()
    };
    let trace = transport.send_packet_with_trace(packet).await;
    // Without an interface the packet is dropped, so nothing is provable.
    assert_eq!(trace.packet_hash, None);
    assert!(
        handler
            .lock()
            .await
            .pending_packet_receipt(&AddressHash::new_from_hash(&Hash::new([0u8; HASH_SIZE])))
            .is_none()
    );

    let registered = [0x42u8; HASH_SIZE];
    handler.lock().await.register_pending_packet_receipt(registered, announce.destination);
    let pending = handler
        .lock()
        .await
        .pending_packet_receipt(&AddressHash::new_from_hash(&Hash::new(registered)))
        .expect("registered packet is pending");
    assert_eq!(pending.packet_hash, registered);
    assert_eq!(pending.destination, announce.destination);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receipt_callback_reenters_send_and_duplicate_expiry_race_is_terminal() {
    let local_identity = PrivateIdentity::new_from_rand(OsRng);
    let config = TransportConfig::new("test", &local_identity, true);
    let mut transport = Transport::new(config);
    let handler = transport.get_handler();
    let remote_identity = PrivateIdentity::new_from_rand(OsRng);
    let mut remote_destination =
        SingleInputDestination::new(remote_identity, DestinationName::new("lxmf", "receipt"));
    let announce = remote_destination.announce(OsRng, None).expect("valid announce packet");
    handle_announce(&announce, handler.lock().await, AddressHash::new_from_rand(OsRng)).await;

    let callbacks = Arc::new(AtomicUsize::new(0));
    let sends = Arc::new(AtomicUsize::new(0));
    transport
        .set_receipt_handler(Box::new(ReentrantReceiptHandler {
            transport: handler.clone(),
            callbacks: callbacks.clone(),
            sends: sends.clone(),
        }))
        .await;
    let packet_hash = [0x92; HASH_SIZE];
    let signature = remote_destination.identity.sign(&packet_hash).to_bytes();
    let mut data = PacketDataBuffer::new();
    data.safe_write(&packet_hash);
    data.safe_write(&signature);
    let proof = Packet {
        header: Header { packet_type: PacketType::Proof, ..Default::default() },
        destination: announce.destination,
        context: PacketContext::None,
        data,
        ..Default::default()
    };

    tokio::time::timeout(Duration::from_secs(1), transport.handle_inbound_for_test(proof))
        .await
        .expect("reentrant callback completed");
    {
        let handler = handler.lock().await;
        let mut cache = handler.packet_cache.lock().await;
        cache.update(&proof);
        cache.release(Duration::ZERO);
    }
    let ((), ()) = tokio::join!(
        transport.handle_inbound_for_test(proof),
        transport.handle_inbound_for_test(proof),
    );

    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
}

/// Two registered interfaces on a non-broadcast transport, one active outbound
/// Link and one active inbound Link bound to the first interface, and one
/// pending outbound Link bound to the second interface. The destination path
/// table stays empty on purpose: established Link sends must not consult it.
struct BoundLinkFixture {
    transport: Transport,
    bound: crate::transport::iface::InterfaceChannel,
    other: crate::transport::iface::InterfaceChannel,
    destination: AddressHash,
    inactive_destination: AddressHash,
    out_link_id: AddressHash,
    in_link_id: AddressHash,
}

async fn bound_link_fixture() -> BoundLinkFixture {
    let local = PrivateIdentity::new_from_rand(OsRng);
    let transport = Transport::new(TransportConfig::new("bound-links", &local, false));
    let bound = test_interface_channel(&transport).await;
    let other = test_interface_channel(&transport).await;

    let signer = PrivateIdentity::new_from_rand(OsRng);
    let identity = *signer.as_identity();
    let destination = DestinationDesc {
        identity,
        address_hash: identity.address_hash,
        name: DestinationName::new("test", "bound"),
    };
    let tx = transport.link_out_event_tx.clone();
    let mut outbound = Link::new(destination, tx.clone());
    let request = outbound.request();
    let mut inbound =
        Link::new_from_request(&request, signer.sign_key().clone(), destination, tx.clone())
            .expect("link request should parse");
    assert!(matches!(
        outbound.handle_packet(&inbound.prove(), bound.address),
        LinkHandleResult::Activated
    ));
    inbound.set_ingress_iface(bound.address);
    outbound.open_channel();
    inbound.open_channel();
    assert_eq!(outbound.status(), LinkStatus::Active);
    assert_eq!(inbound.status(), LinkStatus::Active);
    let out_link_id = *outbound.id();
    let in_link_id = *inbound.id();

    let inactive_signer = PrivateIdentity::new_from_rand(OsRng);
    let inactive_identity = *inactive_signer.as_identity();
    let inactive_destination = DestinationDesc {
        identity: inactive_identity,
        address_hash: inactive_identity.address_hash,
        name: DestinationName::new("test", "inactive"),
    };
    let mut pending = Link::new(inactive_destination, tx);
    pending.set_ingress_iface(other.address);
    assert_eq!(pending.status(), LinkStatus::Pending);

    {
        let mut handler = transport.handler.lock().await;
        handler.out_links.insert(destination.address_hash, Arc::new(Mutex::new(outbound)));
        handler.out_links.insert(inactive_destination.address_hash, Arc::new(Mutex::new(pending)));
        handler.in_links.insert(in_link_id, Arc::new(Mutex::new(inbound)));
    }

    BoundLinkFixture {
        transport,
        bound,
        other,
        destination: destination.address_hash,
        inactive_destination: inactive_destination.address_hash,
        out_link_id,
        in_link_id,
    }
}

impl BoundLinkFixture {
    async fn expect_bound_send(&mut self, link_id: AddressHash, context: PacketContext) {
        let message = timeout(Duration::from_millis(200), self.bound.tx_channel.recv())
            .await
            .expect("bound interface must receive the Link packet")
            .expect("bound interface channel open");
        assert_eq!(message.tx_type, TxMessageType::Direct(self.bound.address));
        assert_eq!(message.packet.destination, link_id);
        assert_eq!(message.packet.header.destination_type, DestinationType::Link);
        assert_eq!(message.packet.header.packet_type, PacketType::Data);
        assert_eq!(message.packet.context, context);
    }

    fn expect_quiet(&mut self) {
        assert!(
            self.bound.tx_channel.try_recv().is_err(),
            "bound interface must not receive extra packets"
        );
        assert!(
            self.other.tx_channel.try_recv().is_err(),
            "the inactive Link's interface must receive nothing"
        );
    }
}

#[tokio::test]
async fn data_send_to_all_out_links_uses_each_active_links_bound_interface() {
    let mut fixture = bound_link_fixture().await;
    fixture.transport.send_to_all_out_links(b"fan-out").await;
    let link_id = fixture.out_link_id;
    fixture.expect_bound_send(link_id, PacketContext::None).await;
    fixture.expect_quiet();
}

#[tokio::test]
async fn channel_send_to_all_out_links_uses_each_active_links_bound_interface() {
    let mut fixture = bound_link_fixture().await;
    fixture.transport.send_channel_to_all_out_links(b"channel").await;
    let link_id = fixture.out_link_id;
    fixture.expect_bound_send(link_id, PacketContext::Channel).await;
    fixture.expect_quiet();
}

#[tokio::test]
async fn data_send_to_out_links_for_a_destination_uses_the_bound_interface() {
    let mut fixture = bound_link_fixture().await;
    let destination = fixture.destination;
    fixture.transport.send_to_out_links(&destination, b"targeted").await;
    let link_id = fixture.out_link_id;
    fixture.expect_bound_send(link_id, PacketContext::None).await;
    fixture.expect_quiet();

    let inactive = fixture.inactive_destination;
    fixture.transport.send_to_out_links(&inactive, b"never").await;
    fixture.expect_quiet();
}

#[tokio::test]
async fn data_send_to_in_links_for_a_destination_uses_the_bound_interface() {
    let mut fixture = bound_link_fixture().await;
    let destination = fixture.destination;
    fixture.transport.send_to_in_links(&destination, b"inbound").await;
    let link_id = fixture.in_link_id;
    fixture.expect_bound_send(link_id, PacketContext::None).await;
    fixture.expect_quiet();
}

#[tokio::test]
async fn established_link_sends_ignore_the_destination_path_table() {
    let mut fixture = bound_link_fixture().await;
    let wrong_route = fixture.other.address;
    let out_link_id = fixture.out_link_id;
    let announce = Packet {
        header: Header { packet_type: PacketType::Announce, ..Default::default() },
        destination: out_link_id,
        ..Default::default()
    };
    fixture.transport.handler.lock().await.path_table.handle_announce(
        &announce,
        None,
        wrong_route,
        InterfaceMode::AccessPoint,
        [3; crate::destination::RAND_HASH_LENGTH],
    );
    fixture.transport.send_to_all_out_links(b"bound wins").await;
    fixture.expect_bound_send(out_link_id, PacketContext::None).await;
    fixture.expect_quiet();
}

#[tokio::test]
async fn single_link_packet_send_uses_the_bound_interface_and_rejects_inactive_links() {
    let mut fixture = bound_link_fixture().await;
    let destination = fixture.destination;
    let inactive = fixture.inactive_destination;
    let (active, pending) = {
        let handler = fixture.transport.handler.lock().await;
        (handler.out_links[&destination].clone(), handler.out_links[&inactive].clone())
    };
    let packet = active.lock().await.data_packet(b"single").expect("active link packet");
    assert_eq!(
        fixture.transport.send_link_packet(&active, packet).await,
        SendPacketOutcome::SentDirect
    );
    let link_id = fixture.out_link_id;
    fixture.expect_bound_send(link_id, PacketContext::None).await;
    fixture.expect_quiet();

    assert_eq!(
        fixture.transport.send_link_packet(&pending, packet).await,
        SendPacketOutcome::DroppedNoRoute
    );
    fixture.expect_quiet();
}
