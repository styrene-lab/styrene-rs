//! Daemon facade contract tests — verifying the DaemonFacade can be used
//! as `Arc<dyn Daemon>` by Unix socket IPC consumers.
//!
//! Package J — dependent unlock validation.
//!
//! These tests prove:
//! 1. DaemonFacade implements the full Daemon composite trait
//! 2. It can be held behind Arc<dyn Daemon> (the IPC handler's view)
//! 3. Auth enforcement works through the trait object
//! 4. Real and stubbed methods are accessible through the trait
//! 5. Multiple facades can coexist (different callers, same AppContext)

use std::sync::{Arc, Mutex};
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::Daemon;
use styrene_ipc::types::SendChatRequest;
use styrened::app_context::AppContext;
use styrened::daemon_facade::DaemonFacade;
use styrened::storage::messages::{MessageRecord, MessagesStore};
use styrened::transport::mesh_transport::{MeshTransport, TransportError};
use styrened::transport::mock_transport::{MockCall, MockTransport};
use styrened::transport::null_transport::NullTransport;

fn make_ctx() -> Arc<AppContext> {
    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    Arc::new(AppContext::new(transport, "daemon-identity".into(), store))
}

fn make_messaging_daemon() -> (Arc<dyn Daemon>, Arc<MockTransport>) {
    let source = rns_core::hash::AddressHash::new([0x11; 16]);
    let destination = rns_core::hash::AddressHash::new([0x22; 16]);
    let transport = Arc::new(MockTransport::new(source, destination));
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let caller = "aa".repeat(16);
    let mut policy = styrene_rbac::RbacPolicy::default();
    policy.add_entry(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin));
    let nodes = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport.clone(),
        hex::encode(source.as_slice()),
        store,
        nodes,
        styrened::services::PolicyService::new(policy),
    ));
    ctx.set_signer(Arc::new(rns_core::identity::PrivateIdentity::new_from_name(
        "facade-delivery-decision",
    )));
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller));
    (daemon, transport)
}

fn tcp_route_fixture(
    destination: rns_core::hash::AddressHash,
) -> (
    rns_core::transport::core_transport::path_table::PathSnapshot,
    rns_core::transport::iface::InterfaceSnapshot,
) {
    let interface_hash = rns_core::hash::AddressHash::new([0x71; 16]);
    let observed_at = std::time::SystemTime::now();
    (
        rns_core::transport::core_transport::path_table::PathSnapshot {
            destination,
            hops: 1,
            received_from: rns_core::hash::AddressHash::new([0x72; 16]),
            iface: interface_hash,
            age: std::time::Duration::ZERO,
            observed_at,
            lifetime: std::time::Duration::from_secs(60),
            expires_at: observed_at + std::time::Duration::from_secs(60),
        },
        rns_core::transport::iface::InterfaceSnapshot {
            hash: interface_hash,
            kind: rns_core::transport::iface::InterfaceKind::TcpClient,
            mode: rns_core::transport::iface::InterfaceMode::PointToPoint,
            state: rns_core::transport::iface::InterfaceState::Connected,
            local_endpoint: None,
            remote_endpoint: None,
            parent: None,
            tx_bytes: 0,
            rx_bytes: 0,
            violations: Default::default(),
            filters: Default::default(),
            connected_peers: 1,
            generation: 6,
        },
    )
}

fn make_role_daemon(
    role: styrene_rbac::Role,
    transport: Arc<dyn MeshTransport>,
) -> Arc<dyn Daemon> {
    let caller = "aa".repeat(16);
    let mut policy = styrene_rbac::RbacPolicy::default();
    policy.add_entry(styrene_rbac::RosterEntry::new(&caller, role));
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let nodes = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport,
        "daemon-identity".into(),
        store,
        nodes,
        styrened::services::PolicyService::new(policy),
    ));
    Arc::new(DaemonFacade::new(ctx, caller))
}

fn content_for_final_ticket_wire_size(destination: [u8; 16], target: usize) -> String {
    let signer = rns_core::identity::PrivateIdentity::new_from_name("facade-delivery-decision");
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
        .saturating_add(lxmf::stamps::TICKET_EXPIRY_SECS);
    let ticket = [0x44; lxmf::stamps::TICKET_LENGTH];
    for size in 0..=target {
        let content = "x".repeat(size);
        let wire = styrened::lxmf_bridge::build_wire_message_with_options(
            [0x11; 16],
            destination,
            "",
            &content,
            None,
            &signer,
            None,
            None,
            Some((expires_at, &ticket)),
        )
        .unwrap();
        if wire.len() == target {
            return content;
        }
    }
    panic!("no LXMF content length encoded to {target} bytes");
}

fn packet_send_result(
    destination: [u8; 16],
    data: &[u8],
) -> rns_core::transport::delivery::LinkSendResult {
    use rns_core::packet::*;
    rns_core::transport::delivery::LinkSendResult::Packet(Box::new(Packet {
        header: Header {
            ifac_flag: IfacFlag::Open,
            header_type: HeaderType::Type1,
            context_flag: ContextFlag::Unset,
            propagation_type: PropagationType::Broadcast,
            destination_type: DestinationType::Single,
            packet_type: PacketType::Data,
            hops: 0,
        },
        ifac: None,
        destination: rns_core::hash::AddressHash::new(destination),
        transport: None,
        context: PacketContext::None,
        data: PacketDataBuffer::new_from_slice(data),
    }))
}

#[test]
fn facade_usable_as_arc_dyn_daemon() {
    let ctx = make_ctx();
    let facade = DaemonFacade::new(ctx, "local".into());
    let daemon: Arc<dyn Daemon> = Arc::new(facade);
    // The IPC handler would hold this Arc<dyn Daemon>
    let _ = daemon;
}

#[tokio::test]
async fn facade_projects_service_captured_route_without_inferred_bearer() {
    let (daemon, transport) = make_messaging_daemon();
    let destination = rns_core::hash::AddressHash::new([0x73; 16]);
    let (path, interface) = tcp_route_fixture(destination);
    transport.set_path_snapshot(path);
    transport.set_interface_snapshots(vec![interface]);
    let mut request = SendChatRequest::default();
    request.peer_hash = hex::encode(destination.as_slice());
    request.content = "facade route projection".into();
    request.delivery_method = Some("opportunistic".into());

    let sent = daemon.send_chat_outcome(request).await.unwrap();
    let projected = daemon.query_message(&sent.message_id).await.unwrap().unwrap();
    let attempt = projected.attempts.first().unwrap();
    assert_eq!(attempt.route.outcome, styrene_ipc::types::MessageAttemptRouteOutcome::Observed);
    assert_eq!(attempt.route.connection_generation, Some(6));
    assert_eq!(attempt.route.interface.as_ref().unwrap().kind, "tcp_client");
    assert_eq!(attempt.bearer, None);
    assert_eq!(projected.actual_delivery_method.as_deref(), Some("opportunistic"));
    assert!(projected.delivery_evidence.is_empty());
}

#[tokio::test]
async fn daemon_trait_object_query_status() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let status = daemon.query_status().await.unwrap();
    assert!(!status.rns_initialized);
    assert_eq!(status.device_count, 0);
}

#[tokio::test]
async fn daemon_trait_object_query_identity() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let identity = daemon.query_identity().await.unwrap();
    assert_eq!(identity.identity_hash, "daemon-identity");
}

#[tokio::test]
async fn query_identity_uses_transport_identity_and_destination() {
    let identity = rns_core::hash::AddressHash::new([0x11; 16]);
    let destination = rns_core::hash::AddressHash::new([0x22; 16]);
    let transport: Arc<dyn MeshTransport> = Arc::new(MockTransport::new(identity, destination));
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let ctx = Arc::new(AppContext::new(transport, "configured-wrong-value".into(), store));
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));

    let actual = daemon.query_identity().await.unwrap();

    assert_eq!(actual.identity_hash, hex::encode(identity.as_slice()));
    assert_eq!(actual.destination_hash, hex::encode(destination.as_slice()));
    assert_eq!(actual.lxmf_destination_hash, hex::encode(destination.as_slice()));
}

#[tokio::test]
async fn direct_ipc_request_falls_back_after_sending_full_lxmf_wire_over_link() {
    let (daemon, transport) = make_messaging_daemon();
    let peer = rns_core::identity::PrivateIdentity::new_from_name("direct-peer");
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.queue_send_link(Err(TransportError::SendFailed("test stop".into())));
    let mut request = SendChatRequest::default();
    request.peer_hash = hex::encode([0x33; 16]);
    request.content = "direct boundary".into();
    request.delivery_method = Some("direct".into());

    let message_id = daemon.send_chat(request).await.unwrap();

    let calls = transport.calls();
    let data = calls
        .iter()
        .find_map(|call| match call {
            MockCall::SendViaLink { data, .. } => Some(data),
            _ => None,
        })
        .expect("direct delivery must select link wire representation");
    assert_eq!(&data[..16], &[0x33; 16]);
    let decoded = lxmf::inbound_decode::decode_inbound_message(
        [0x33; 16],
        data,
        lxmf::inbound_decode::InboundPayloadMode::FullWire,
    )
    .unwrap();
    assert_eq!(decoded.content, b"direct boundary");
    assert!(calls.iter().any(|call| matches!(call, MockCall::SendRaw { .. })));
    let messages = daemon.query_messages(&hex::encode([0x33; 16]), 10, None).await.unwrap();
    let sent = messages.iter().find(|message| message.id == message_id).unwrap();
    assert_eq!(sent.requested_delivery_method.as_deref(), Some("direct"));
    assert_eq!(sent.actual_delivery_method.as_deref(), Some("opportunistic"));
    assert!(sent.fallback_reason.as_deref().is_some_and(|reason| reason.contains("test stop")));
    assert_eq!(sent.attempts.len(), 1);
}

#[tokio::test]
async fn opportunistic_ipc_request_sends_destination_stripped_lxmf_packet() {
    let (daemon, transport) = make_messaging_daemon();
    let mut request = SendChatRequest::default();
    request.peer_hash = hex::encode([0x44; 16]);
    request.content = "opportunistic boundary".into();
    request.delivery_method = Some("opportunistic".into());

    let message_id = daemon.send_chat(request).await.unwrap();

    let calls = transport.calls();
    assert!(!calls.iter().any(|call| matches!(call, MockCall::SendViaLink { .. })));
    let data = calls
        .iter()
        .find_map(|call| match call {
            MockCall::SendRaw { data, .. } => Some(data),
            _ => None,
        })
        .expect("opportunistic delivery must select raw packet representation");
    assert_ne!(&data[..16], &[0x44; 16]);
    let decoded = lxmf::inbound_decode::decode_inbound_message(
        [0x44; 16],
        data,
        lxmf::inbound_decode::InboundPayloadMode::DestinationStripped,
    )
    .unwrap();
    assert_eq!(decoded.content, b"opportunistic boundary");
    let messages = daemon.query_messages(&hex::encode([0x44; 16]), 10, None).await.unwrap();
    let sent = messages.iter().find(|message| message.id == message_id).unwrap();
    assert_eq!(sent.delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(sent.requested_delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(sent.actual_delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(sent.correlation_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(sent.attempts.len(), 1);
    assert_eq!(sent.attempts[0].state, "sent");
}

#[tokio::test]
async fn oversized_opportunistic_request_reports_direct_lxmf_fallback() {
    let (daemon, transport) = make_messaging_daemon();
    let peer = rns_core::identity::PrivateIdentity::new_from_name("fallback-peer");
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.queue_send_link(Err(TransportError::SendFailed("test stop".into())));
    let peer_hash = hex::encode([0x45; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash.clone();
    request.content = "x".repeat(1_000);
    request.delivery_method = Some("opportunistic".into());

    assert!(
        matches!(daemon.send_chat(request).await, Err(IpcError::Internal { message }) if message.contains("test stop"))
    );

    let calls = transport.calls();
    assert!(calls.iter().any(|call| matches!(call, MockCall::SendViaLink { .. })));
    assert!(!calls.iter().any(|call| matches!(call, MockCall::SendRaw { .. })));
    let messages = daemon.query_messages(&peer_hash, 10, None).await.unwrap();
    let sent = &messages[0];
    let message_id = sent.id.clone();
    assert_eq!(sent.delivery_method.as_deref(), Some("direct"));
    assert_eq!(sent.requested_delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(sent.actual_delivery_method.as_deref(), Some("direct"));
    assert!(sent.fallback_reason.as_deref().is_some_and(|reason| reason.contains("packet limit")));
    assert_eq!(sent.correlation_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(sent.attempts.len(), 1);
    assert_eq!(sent.attempts[0].message_id, message_id);
    assert_eq!(sent.attempts[0].number, 1);
    assert!(sent.attempts[0].started_unix_ms > 0);
    assert!(sent.attempts[0].deadline_unix_ms >= sent.attempts[0].started_unix_ms);
    assert_eq!(sent.attempts[0].state, "failed");
}

#[tokio::test]
async fn selected_packet_representation_rejection_uses_opportunistic_fallback() {
    let (daemon, transport) = make_messaging_daemon();
    let peer = rns_core::identity::PrivateIdentity::new_from_name("representation-peer");
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.queue_send_link(Ok(rns_core::transport::delivery::LinkSendResult::Resource(
        rns_core::hash::Hash::new_from_slice(b"unexpected-resource"),
    )));
    let peer_hash = hex::encode([0x46; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash.clone();
    request.content = "packet-sized".into();
    request.delivery_method = Some("direct".into());

    let message_id = daemon.send_chat(request).await.unwrap();

    let messages = daemon.query_messages(&peer_hash, 10, None).await.unwrap();
    let sent = messages.iter().find(|message| message.id == message_id).unwrap();
    assert_eq!(sent.requested_delivery_method.as_deref(), Some("direct"));
    assert_eq!(sent.actual_delivery_method.as_deref(), Some("opportunistic"));
    assert!(
        sent.fallback_reason.as_deref().is_some_and(
            |reason| reason.contains("mock refused Resource result for selected Packet")
        )
    );
    assert_eq!(sent.attempts.len(), 1);
}

#[tokio::test]
async fn ipc_success_selects_exact_packet_and_resource_boundary_representations() {
    use styrened::transport::mesh_transport::LinkRepresentation;

    let (daemon, transport) = make_messaging_daemon();
    let peer = rns_core::identity::PrivateIdentity::new_from_name("boundary-peer");
    let packet_destination = [0x48; 16];
    let resource_destination = [0x49; 16];
    let packet_content = content_for_final_ticket_wire_size(
        packet_destination,
        rns_core::transport::resource::LINK_PACKET_MDU,
    );
    let resource_content = content_for_final_ticket_wire_size(
        resource_destination,
        rns_core::transport::resource::LINK_PACKET_MDU + 1,
    );
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.queue_send_link(Ok(packet_send_result(packet_destination, b"packet-proof")));
    transport.queue_resolve(Some(*peer.as_identity()));
    transport.queue_send_link(Ok(rns_core::transport::delivery::LinkSendResult::Resource(
        rns_core::hash::Hash::new_from_slice(b"resource-proof"),
    )));
    let mut packet_request = SendChatRequest::default();
    packet_request.peer_hash = hex::encode(packet_destination);
    packet_request.content = packet_content;
    packet_request.delivery_method = Some("direct".into());
    let mut resource_request = SendChatRequest::default();
    resource_request.peer_hash = hex::encode(resource_destination);
    resource_request.content = resource_content;
    resource_request.delivery_method = Some("direct".into());

    daemon.send_chat(packet_request).await.unwrap();
    daemon.send_chat(resource_request).await.unwrap();

    let selected: Vec<_> = transport
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            MockCall::SendViaLink { data, representation: Some(representation), .. } => {
                Some((data.len(), representation))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        selected,
        [
            (rns_core::transport::resource::LINK_PACKET_MDU, LinkRepresentation::Packet),
            (rns_core::transport::resource::LINK_PACKET_MDU + 1, LinkRepresentation::Resource),
        ]
    );
}

#[tokio::test]
async fn retry_retains_original_requested_method() {
    let (daemon, transport) = make_messaging_daemon();
    transport.queue_send_raw(Err(styrened::transport::mesh_transport::TransportError::SendFailed(
        "offline".into(),
    )));
    let peer_hash = hex::encode([0x47; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash;
    request.content = "retry opportunistically".into();
    request.delivery_method = Some("opportunistic".into());
    assert!(
        matches!(daemon.send_chat(request).await, Err(IpcError::Internal { message }) if message.contains("offline"))
    );
    let messages = daemon.query_messages(&hex::encode([0x47; 16]), 10, None).await.unwrap();
    let message_id = messages[0].id.clone();
    let first_payload = transport
        .calls()
        .into_iter()
        .find_map(|call| match call {
            MockCall::SendRaw { data, .. } => Some(data),
            _ => None,
        })
        .expect("initial opportunistic payload");

    assert!(daemon.retry_message(&message_id).await.unwrap());

    let calls = transport.calls();
    assert_eq!(calls.iter().filter(|call| matches!(call, MockCall::SendRaw { .. })).count(), 2);
    assert!(!calls.iter().any(|call| matches!(call, MockCall::SendViaLink { .. })));
    let retried_payload = calls
        .iter()
        .filter_map(|call| match call {
            MockCall::SendRaw { data, .. } => Some(data),
            _ => None,
        })
        .nth(1)
        .expect("retried opportunistic payload");
    assert_eq!(retried_payload, &first_payload);
    let messages = daemon.query_messages(&hex::encode([0x47; 16]), 10, None).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message_id);
    assert_eq!(messages[0].requested_delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(messages[0].actual_delivery_method.as_deref(), Some("opportunistic"));
    assert_eq!(messages[0].correlation_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(messages[0].attempts.len(), 2);
    assert_eq!(
        messages[0].attempts.iter().map(|attempt| attempt.number).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn dispatched_packet_cannot_be_reported_as_cancelled() {
    let (daemon, transport) = make_messaging_daemon();
    let mut request = SendChatRequest::default();
    request.peer_hash = hex::encode([0x48; 16]);
    request.content = "cancel me".into();
    request.delivery_method = Some("opportunistic".into());
    let message_id = daemon.send_chat(request).await.unwrap();

    assert!(!daemon.cancel_message(&message_id).await.unwrap());
    assert!(!daemon.cancel_message(&message_id).await.unwrap());
    let messages = daemon.query_messages(&hex::encode([0x48; 16]), 10, None).await.unwrap();
    assert_eq!(messages[0].status, "sent: opportunistic");
    assert_eq!(
        transport.calls().iter().filter(|call| matches!(call, MockCall::SendRaw { .. })).count(),
        1
    );
}

#[tokio::test]
async fn unavailable_propagated_request_is_rejected_without_direct_substitution() {
    let (daemon, transport) = make_messaging_daemon();
    let mut request = SendChatRequest::default();
    request.peer_hash = hex::encode([0x55; 16]);
    request.content = "must propagate".into();
    request.delivery_method = Some("propagated".into());

    let error = daemon.send_chat(request).await.unwrap_err();

    assert!(
        matches!(error, IpcError::Unavailable { ref reason } if reason.contains("propagation peer")),
        "unexpected propagated error: {error:?}"
    );
    assert!(
        !transport.calls().iter().any(|call| {
            matches!(call, MockCall::SendRaw { .. } | MockCall::SendViaLink { .. })
        })
    );
    let messages = daemon.query_messages(&hex::encode([0x55; 16]), 10, None).await.unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn legacy_paper_is_rejected_before_persistence_or_transmission() {
    let (daemon, transport) = make_messaging_daemon();
    transport.set_connected(false);
    let peer_hash = hex::encode([0x56; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash.clone();
    request.content = "export this paper message".into();
    request.delivery_method = Some("paper".into());

    let error = daemon.send_chat(request).await.unwrap_err();

    assert!(
        matches!(error, IpcError::InvalidRequest { ref message } if message.contains("send_chat_outcome"))
    );
    assert!(
        !transport
            .calls()
            .iter()
            .any(|call| matches!(call, MockCall::SendRaw { .. } | MockCall::SendViaLink { .. }))
    );
    let messages = daemon.query_messages(&peer_hash, 10, None).await.unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn typed_failed_send_returns_authoritative_id_while_legacy_returns_error() {
    let (daemon, transport) = make_messaging_daemon();
    transport.set_connected(false);
    let peer_hash = hex::encode([0x57; 16]);
    let mut typed = SendChatRequest::default();
    typed.peer_hash = peer_hash.clone();
    typed.content = "typed failure".into();
    typed.delivery_method = Some("direct".into());

    let outcome = daemon.send_chat_outcome(typed).await.unwrap();
    assert_eq!(outcome.disposition, styrene_ipc::types::SendChatDisposition::Failed);
    assert!(!outcome.message_id.is_empty());
    assert_eq!(outcome.message.id, outcome.message_id);
    assert!(outcome.terminal_error.as_deref().is_some_and(|error| error.contains("not connected")));

    let mut legacy = SendChatRequest::default();
    legacy.peer_hash = peer_hash;
    legacy.content = "legacy failure".into();
    legacy.delivery_method = Some("direct".into());
    assert!(
        matches!(daemon.send_chat(legacy).await, Err(IpcError::Internal { message }) if message.contains("not connected"))
    );
}

#[tokio::test]
async fn offline_direct_failure_persists_the_failed_transport_attempt() {
    let (daemon, transport) = make_messaging_daemon();
    transport.set_connected(false);
    let peer_hash = hex::encode([0x58; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash.clone();
    request.content = "offline direct".into();
    request.delivery_method = Some("direct".into());

    let error = daemon.send_chat(request).await.unwrap_err();

    assert!(
        matches!(error, IpcError::Internal { ref message } if message.contains("transport not connected"))
    );
    assert!(transport.calls().is_empty());
    let messages = daemon.query_messages(&peer_hash, 10, None).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].requested_delivery_method.as_deref(), Some("direct"));
    assert_eq!(messages[0].actual_delivery_method.as_deref(), Some("direct"));
    assert!(messages[0].fallback_reason.is_none());
    assert_eq!(messages[0].correlation_id.as_deref(), Some(messages[0].id.as_str()));
    assert_eq!(messages[0].status, "failed: transport not connected");
    assert_eq!(messages[0].attempts.len(), 1);
    assert_eq!(messages[0].attempts[0].number, 1);
}

#[tokio::test]
async fn unknown_delivery_method_is_rejected_before_transport_or_persistence() {
    let (daemon, transport) = make_messaging_daemon();
    let peer_hash = hex::encode([0x57; 16]);
    let mut request = SendChatRequest::default();
    request.peer_hash = peer_hash.clone();
    request.content = "do not substitute".into();
    request.delivery_method = Some("automatic".into());

    assert!(matches!(daemon.send_chat(request).await, Err(IpcError::InvalidRequest { .. })));
    assert!(transport.calls().is_empty());
    assert!(daemon.query_messages(&peer_hash, 10, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn page_ipc_boundary_rejects_untyped_native_addresses() {
    let ctx = make_ctx();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));

    assert!(matches!(
        daemon.browse_page("not-a-hash", "/page/index.mu", None).await,
        Err(IpcError::InvalidRequest { .. })
    ));
    assert!(matches!(
        daemon.browse_page("0123456789abcdef0123456789abcdef", "/file/secret", None).await,
        Err(IpcError::InvalidRequest { .. })
    ));
    assert!(matches!(
        daemon.browse_page("local", "/page/../secret", None).await,
        Err(IpcError::InvalidRequest { .. })
    ));
}

#[tokio::test]
async fn daemon_trait_object_announce() {
    let ctx = make_ctx();
    let caller = "ca".repeat(16);
    ctx.policy()
        .grant(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin), ctx.store())
        .unwrap();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller));
    assert!(daemon.announce().await.unwrap());
}

#[tokio::test]
async fn daemon_trait_object_auto_reply_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_ctx();
    ctx.config().load_or_default(&dir.path().join("config.toml")).unwrap();
    let caller = "ca".repeat(16);
    ctx.policy()
        .grant(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin), ctx.store())
        .unwrap();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller));

    // Set
    daemon.set_auto_reply("all", Some("Away"), Some(120)).await.unwrap();

    // Get
    let config = daemon.query_auto_reply().await.unwrap();
    assert_eq!(config.mode, "all");
    assert_eq!(config.message, Some("Away".into()));
    assert_eq!(config.cooldown_secs, Some(120));
}

#[tokio::test]
async fn direct_ipc_config_mutations_require_config_update_capability() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_ctx();
    ctx.config().load_or_default(&dir.path().join("config.toml")).unwrap();
    let peer: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx.clone(), "peer".into()));

    assert!(peer.query_auto_reply().await.is_ok());
    assert!(matches!(
        peer.set_auto_reply("all", Some("Away"), Some(120)).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::RPC_CONFIG_UPDATE
    ));
    assert!(matches!(
        peer.save_config(Default::default()).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::RPC_CONFIG_UPDATE
    ));

    let admin_hash = "ad".repeat(16);
    ctx.policy()
        .grant(styrene_rbac::RosterEntry::new(&admin_hash, styrene_rbac::Role::Admin), ctx.store())
        .unwrap();
    let admin: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, admin_hash));
    assert!(admin.set_auto_reply("all", Some("Away"), Some(120)).await.unwrap());
    assert!(admin.save_config(Default::default()).await.unwrap());
}

#[tokio::test]
async fn direct_dispatch_splits_local_config_fleet_apply_and_policy_update() {
    let operator = make_role_daemon(styrene_rbac::Role::Operator, Arc::new(NullTransport::new()));
    assert!(operator.save_config(Default::default()).await.unwrap());
    assert!(matches!(
        operator.fleet_apply("11", Vec::new(), true, Some(1)).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::RPC_FLEET_APPLY
    ));
    assert!(matches!(
        operator.block_peer(&"bb".repeat(16)).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::POLICY_UPDATE
    ));

    let admin = make_role_daemon(styrene_rbac::Role::Admin, Arc::new(NullTransport::new()));
    let fleet_result = admin.fleet_apply("11", Vec::new(), true, Some(1)).await;
    assert!(
        !matches!(fleet_result, Err(IpcError::Denied { .. })),
        "admin fleet apply was rejected before execution"
    );
    assert!(admin.block_peer(&"bb".repeat(16)).await.unwrap());
}

#[tokio::test]
async fn direct_dispatch_enforces_and_executes_each_network_state_capability() {
    let monitor = make_role_daemon(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));
    assert!(matches!(
        monitor.start_request(Default::default()).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::NETWORK_REQUEST
    ));
    assert!(matches!(
        monitor.cancel_request("request").await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::NETWORK_REQUEST_CANCEL
    ));
    assert!(matches!(
        monitor.cancel_resource(&"11".repeat(32)).await,
        Err(IpcError::Denied { capability }) if capability == styrene_rbac::Capability::NETWORK_RESOURCE_CANCEL
    ));

    let transport = Arc::new(MockTransport::new_default());
    let operator = make_role_daemon(styrene_rbac::Role::Operator, transport.clone());
    operator.start_request(Default::default()).await.unwrap();
    operator.cancel_request("request").await.unwrap();
    operator.cancel_resource(&"11".repeat(32)).await.unwrap();
    let calls = transport.calls();
    assert!(calls.iter().any(|call| matches!(call, MockCall::StartRequest { .. })));
    assert!(calls.iter().any(|call| matches!(call, MockCall::CancelRequest { .. })));
    assert!(calls.iter().any(|call| matches!(call, MockCall::CancelResource { .. })));
}

#[tokio::test]
async fn daemon_trait_object_blocked_caller() {
    use styrene_rbac::RbacPolicy;
    use styrened::services::PolicyService;

    let mut policy = RbacPolicy::default();
    policy.block("deadbeef");

    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport,
        "daemon-identity".into(),
        store,
        node_store,
        PolicyService::new(policy),
    ));

    let daemon: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx, "deadbeef11112222333344445555aaaa".into()));

    let result = daemon.query_status().await;
    assert!(matches!(result, Err(IpcError::Denied { .. })));
}

#[tokio::test]
async fn multiple_facades_same_context() {
    use styrene_rbac::{RbacPolicy, RosterEntry};
    use styrened::services::PolicyService;

    let mut policy = RbacPolicy::default();
    policy.add_entry(
        RosterEntry::new("aaaa1111bbbb2222cccc3333dddd4444", styrene_rbac::Role::Admin)
            .with_label("admin"),
    );

    let transport: Arc<dyn MeshTransport> = Arc::new(NullTransport::new());
    let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
    let node_store = Arc::new(styrene_services::node_store::NodeStore::in_memory().unwrap());
    let ctx = Arc::new(AppContext::with_policy(
        transport,
        "daemon-identity".into(),
        store,
        node_store,
        PolicyService::new(policy),
    ));

    // Two facades with different caller identities
    let admin_facade: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx.clone(), "aaaa1111bbbb2222cccc3333dddd4444".into()));
    let peer_facade: Arc<dyn Daemon> =
        Arc::new(DaemonFacade::new(ctx.clone(), "bbbb2222cccc3333dddd4444eeee5555".into()));

    // Both can query status
    assert!(admin_facade.query_status().await.is_ok());
    assert!(peer_facade.query_status().await.is_ok());

    // Admin can exec (returns Internal in test mode — no transport)
    let admin_exec = admin_facade.exec("dest", "ls", vec![], None).await;
    assert!(matches!(admin_exec, Err(IpcError::Internal { .. })));

    // Peer cannot exec
    let peer_exec = peer_facade.exec("dest", "ls", vec![], None).await;
    assert!(matches!(peer_exec, Err(IpcError::Denied { .. })));
}

#[tokio::test]
async fn daemon_trait_object_query_devices() {
    let ctx = make_ctx();

    // Discover a device
    ctx.discovery()
        .accept_announce_with_details("node1".into(), 1000, Some("Hub".into()), None, None)
        .unwrap();

    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, "caller".into()));
    let devices = daemon.query_devices(false).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Hub");
}

#[tokio::test]
async fn daemon_trait_object_not_implemented_methods() {
    let daemon = make_role_daemon(styrene_rbac::Role::Monitor, Arc::new(NullTransport::new()));

    // list_tunnels returns Ok(empty) because TunnelService is wired but has no peers.
    assert!(daemon.list_tunnels().await.unwrap().is_empty());
    // Validation rejects malformed input before transport dispatch.
    assert!(matches!(
        daemon.send_chat(SendChatRequest::default()).await,
        Err(IpcError::InvalidRequest { .. })
    ));
    // query_path_info returns InvalidRequest for bad hash, not NotImplemented
    assert!(matches!(daemon.query_path_info("abc").await, Err(IpcError::InvalidRequest { .. })));

    // These should now work (not NotImplemented)
    let _results = daemon.search_messages("test", None, 10).await.expect("search works");
    let _convos = daemon.query_conversations(false).await.expect("conversations work");
    let _contacts = daemon.query_contacts().await.expect("contacts work");
}

#[tokio::test]
async fn facade_pin_mute_projection_and_contact_state_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("facade-state.db");
    let peer = "ab".repeat(16);
    let caller = "cd".repeat(16);

    {
        let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
        store
            .lock()
            .unwrap()
            .insert_message(&MessageRecord {
                id: "facade-message".into(),
                source: peer.to_ascii_uppercase(),
                destination: "ef".repeat(16),
                title: String::new(),
                content: "persisted".into(),
                timestamp: 1,
                direction: "in".into(),
                fields: None,
                receipt_status: None,
                read: false,
            })
            .unwrap();
        let ctx = Arc::new(AppContext::new(Arc::new(NullTransport::new()), "ef".repeat(16), store));
        ctx.policy()
            .grant(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin), ctx.store())
            .unwrap();
        let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller.clone()));
        assert!(daemon.pin_conversation(&peer.to_ascii_uppercase()).await.unwrap());
        assert!(daemon.mute_conversation(&peer).await.unwrap());
        daemon.set_contact(&peer.to_ascii_uppercase(), Some("Peer"), None).await.unwrap();
        let summary = daemon.query_conversations(false).await.unwrap().remove(0);
        assert_eq!(summary.peer_hash, peer);
        assert!(summary.pinned);
        assert!(summary.muted);
        assert_eq!(
            daemon.query_messages(&peer.to_ascii_uppercase(), 10, None).await.unwrap().len(),
            1
        );
        assert_eq!(
            daemon
                .search_messages("persisted", Some(&peer.to_ascii_uppercase()), 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    let store = Arc::new(Mutex::new(MessagesStore::open(&path).unwrap()));
    let ctx = Arc::new(AppContext::new(Arc::new(NullTransport::new()), "ef".repeat(16), store));
    ctx.policy()
        .grant(styrene_rbac::RosterEntry::new(&caller, styrene_rbac::Role::Admin), ctx.store())
        .unwrap();
    let daemon: Arc<dyn Daemon> = Arc::new(DaemonFacade::new(ctx, caller));
    let summary = daemon.query_conversations(false).await.unwrap().remove(0);
    assert!(summary.pinned);
    assert!(summary.muted);
    assert_eq!(daemon.query_contacts().await.unwrap()[0].alias.as_deref(), Some("Peer"));
    assert!(matches!(
        daemon.pin_conversation("not-a-peer").await,
        Err(IpcError::InvalidRequest { .. })
    ));
    assert!(matches!(
        daemon.query_messages("not-a-peer", 10, None).await,
        Err(IpcError::InvalidRequest { .. })
    ));
    assert!(matches!(
        daemon.search_messages("persisted", Some("not-a-peer"), 10).await,
        Err(IpcError::InvalidRequest { .. })
    ));
}
