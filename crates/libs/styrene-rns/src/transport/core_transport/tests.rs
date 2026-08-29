use super::announce::handle_announce;
use super::*;

use crate::destination::{DestinationName, SingleInputDestination};
use crate::identity::PrivateIdentity;
use crate::packet::{Header, HeaderType};
use crate::ratchets::encrypt_for_public_key;
use crate::transport::destination_ext::link::{
    Link, LinkCloseReason, LinkEvent, LinkEventData, LinkPayload,
};
use crate::transport::iface::{InterfaceMode, RxMessage, TxMessageType};
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

    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, invalid, 500))
        .await
        .expect("invalid announce input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), announce_events.recv()).await.is_err());

    let mut local_invalid = local.lock().await.announce(OsRng, None).expect("local announce");
    let last = local_invalid.data.len() - 1;
    local_invalid.data.as_mut_slice()[last] ^= 0x01;
    channel
        .rx_channel
        .send(RxMessage::physical(channel.address, local_invalid, 500))
        .await
        .expect("invalid local announce input");
    assert!(timeout(Duration::from_millis(50), iface_events.recv()).await.is_err());
    assert!(timeout(Duration::from_millis(50), announce_events.recv()).await.is_err());
    let stats = interface_stats_for(&transport, channel.address).await;
    assert_eq!(stats.rx_bytes, 0);
    assert_eq!(stats.violations.invalid_announce, 2);
    assert_eq!(stats.filters.valid_blackhole, 0);

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
