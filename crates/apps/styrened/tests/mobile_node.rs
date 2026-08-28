use std::path::PathBuf;
use std::time::Duration;

use styrene_ipc::traits::DaemonStatus;
use styrene_ipc::types::InterfaceDetail;
use styrened::mobile::{
    load_mobile_tcp_endpoint, persist_mobile_tcp_endpoint, IdentityBackend, MobileBearerKind,
    MobileBearerObservation, MobileBearerReason, MobileBearerState, MobileConfig,
    MobileConnectionPhase, MobileDeliveryMethod, MobileDraftClearDisposition, MobileFailureCode,
    MobileInterfaceConfig, MobileMessageEvent, MobileMessageEventKind, MobileMessageSubscription,
    MobileNode, MobilePeerAspect, MobilePeerSource, MobilePeerSubscription, MobileRetryDisposition,
    MobileSendDisposition, MobileSendRequest, MobileUsbFallbackDisposition,
};
use styrened::startup_contract::{RuntimeKind, capabilities, components};
use styrened::storage::messages::MessageRecord;

fn config(root: &std::path::Path, hub_address: Option<String>) -> MobileConfig {
    MobileConfig {
        config_dir: root.join("config"),
        data_dir: root.join("data"),
        hub_address,
        hub_delivery_hash: None,
        display_name: Some("  Classroom Red  ".into()),
        identity_backend: IdentityBackend::PlaintextFile,
        interfaces: Vec::new(),
        enable_rnode_channel: false,
    }
}

async fn shutdown(node: MobileNode) {
    tokio::time::timeout(std::time::Duration::from_secs(5), node.shutdown())
        .await
        .expect("mobile node shutdown timed out")
        .expect("mobile transport shutdown failed");
}

async fn wait_for_interface_status(
    node: &MobileNode,
    status: &str,
    deadline: Duration,
) -> InterfaceDetail {
    tokio::time::timeout(deadline, async {
        loop {
            if let Some(interface) = node
                .facade
                .list_interfaces()
                .await
                .expect("interface observations")
                .into_iter()
                .find(|interface| interface.kind == "tcp_client" && interface.status == status)
            {
                return interface;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("TCP client did not enter {status}"))
}

async fn wait_for_generation(node: &MobileNode, minimum: u64, deadline: Duration) -> u64 {
    tokio::time::timeout(deadline, async {
        loop {
            let generation = node.session_snapshot().await.generation;
            if generation >= minimum {
                return generation;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("mobile session did not reach generation {minimum}"))
}

#[tokio::test]
async fn offline_boot_reports_no_routable_destination() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();

    assert_eq!(node.delivery_hash(), None);
    assert!(!node.is_connected());
    assert_eq!(node.app_context.identity().delivery_destination_hash(), None);
    assert_eq!(node.startup_contract().runtime(), RuntimeKind::Mobile);
    assert!(!node.startup_contract().has_component(components::LXMF_DELIVERY));
    let contract = node.startup_contract();
    assert!(contract.advertises(capabilities::LOCAL_CONFIG.id()));
    assert!(contract.advertises(capabilities::LOCAL_POLICY.id()));
    assert!(contract.advertised_capabilities().iter().all(|capability| {
        [capabilities::LOCAL_CONFIG.id(), capabilities::LOCAL_POLICY.id()]
            .contains(&capability.id())
    }));
    assert!(!contract.advertises(capabilities::LXMF_DIRECT.id()));
    assert!(!contract.advertises(capabilities::STYRENE_RPC.id()));
    assert!(!contract.advertises(capabilities::NETWORK_OPERATIONS.id()));
    assert!(!contract.advertises(capabilities::RNS_REQUESTS.id()));
    assert!(!contract.advertises(capabilities::RNS_REQUEST_CANCELLATION.id()));
    assert!(!contract.advertises(capabilities::RNS_RESOURCE_CANCELLATION.id()));
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(!contract.has_component(components::STANDARD_LXMF_PROPAGATION_DESTINATION));
    let active = node.active_capabilities("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(active.runtime().contains(&capabilities::LOCAL_CONFIG.id()));
    assert!(active.runtime().contains(&capabilities::LOCAL_POLICY.id()));
    assert!(!active.authorized_operations().is_empty());
    assert_eq!(node.app_context.protocol().handler_count().await, 0);
    let status = node.facade.query_status().await.unwrap();
    assert!(!status.propagation_enabled);
    assert!(!status.standard_lxmf_propagation_destination_registered);
    assert!(!status.standard_lxmf_propagation_active);

    shutdown(node).await;
}

#[tokio::test]
async fn shutdown_then_boot_restores_the_same_identity() {
    let root = tempfile::tempdir().unwrap();
    let first = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let identity = first.app_context.identity().identity_hash().to_string();
    shutdown(first).await;

    let second = MobileNode::boot(config(root.path(), None)).await.unwrap();

    assert_eq!(second.app_context.identity().identity_hash(), identity);
    shutdown(second).await;
}

#[tokio::test]
async fn ipv4_tcp_client_reports_connected_runtime_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("IPv4 client connection timed out")
        .unwrap();
    let interface = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    assert_eq!(interface.remote_endpoint.as_deref(), Some(address.to_string().as_str()));
    assert!(node.is_connected());
    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn hostname_tcp_client_connects_to_loopback_listener() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: format!("localhost:{port}") });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("hostname client connection timed out")
        .unwrap();
    let interface = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    assert_eq!(interface.remote_endpoint.as_deref(), Some(format!("127.0.0.1:{port}").as_str()));
    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn refused_tcp_client_is_retrying_and_not_connected() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let interface = wait_for_interface_status(&node, "retrying", Duration::from_secs(2)).await;

    assert!(interface.remote_endpoint.is_none());
    assert!(!node.is_connected());
    shutdown(node).await;
}

#[tokio::test]
async fn refused_tcp_client_reconnects_within_bound_without_replacing_node() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let retrying = wait_for_interface_status(&node, "retrying", Duration::from_secs(2)).await;
    let identity = node.app_context.identity().identity_hash().to_string();
    let delivery_hash = node.delivery_hash();
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(6), listener.accept())
        .await
        .expect("TCP retry exceeded its bound")
        .unwrap();
    let connected = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    assert_eq!(connected.hash, retrying.hash);
    assert_eq!(node.app_context.identity().identity_hash(), identity);
    assert_eq!(node.delivery_hash(), delivery_hash);
    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn connected_session_snapshot_exposes_endpoint_generation_and_independent_bearers() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config.interfaces.push(MobileInterfaceConfig::TcpClient {
        remote_address: format!("localhost:{}", address.port()),
    });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("session connection timed out")
        .unwrap();
    wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;
    let snapshot = node.session_snapshot().await;

    assert_eq!(snapshot.phase, MobileConnectionPhase::Connected);
    assert_eq!(
        snapshot.endpoint.as_deref(),
        Some(format!("localhost:{}", address.port()).as_str())
    );
    assert_eq!(snapshot.generation, 1);
    assert!(snapshot.failure.is_none());
    assert_eq!(
        snapshot.bearer(MobileBearerKind::Tcp).expect("TCP bearer").state,
        MobileBearerState::Connected
    );
    assert_eq!(
        snapshot.bearer(MobileBearerKind::BluetoothRnode).expect("Bluetooth RNode bearer").state,
        MobileBearerState::Unavailable
    );
    assert_eq!(
        snapshot.bearer(MobileBearerKind::AndroidUsb).expect("Android USB bearer").state,
        MobileBearerState::Unavailable
    );

    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn platform_bearer_failures_do_not_degrade_connected_tcp() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });
    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("session connection timed out")
        .unwrap();
    wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    for observation in [
        MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Unavailable,
            reason: Some(MobileBearerReason::PermissionDenied),
        },
        MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Disconnected,
            reason: Some(MobileBearerReason::ConnectionInterrupted),
        },
        MobileBearerObservation {
            kind: MobileBearerKind::AndroidUsb,
            state: MobileBearerState::Unverified,
            reason: Some(MobileBearerReason::PhysicalEvidenceAbsent),
        },
    ] {
        node.platform_service().report(observation.clone()).await.unwrap();
        let snapshot = node.session_snapshot().await;
        assert_eq!(snapshot.phase, MobileConnectionPhase::Connected);
        assert_eq!(
            snapshot.bearer(MobileBearerKind::Tcp).expect("TCP bearer").state,
            MobileBearerState::Connected
        );
        assert_eq!(snapshot.bearer(observation.kind), Some(&observation));
    }

    let tcp_result = node
        .platform_service()
        .report(MobileBearerObservation {
            kind: MobileBearerKind::Tcp,
            state: MobileBearerState::Disconnected,
            reason: Some(MobileBearerReason::ConnectionInterrupted),
        })
        .await;
    assert_eq!(tcp_result, Err("TCP bearer state is owned by the transport runtime"));

    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn android_usb_is_explicit_fallback_and_cannot_preempt_approved_bluetooth() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let platform = node.platform_service();
    let usb_connected = MobileBearerObservation {
        kind: MobileBearerKind::AndroidUsb,
        state: MobileBearerState::Connected,
        reason: None,
    };

    assert_eq!(
        platform.report(usb_connected.clone()).await,
        Err("Android USB requires an explicit fallback request")
    );
    platform.set_bluetooth_approved(true).await;
    platform
        .report(MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Connected,
            reason: None,
        })
        .await
        .unwrap();
    assert_eq!(
        platform.request_android_usb_fallback().await,
        MobileUsbFallbackDisposition::BluetoothActive
    );
    assert_eq!(
        platform.report(usb_connected.clone()).await,
        Err("Android USB requires an explicit fallback request")
    );

    platform
        .report(MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Disconnected,
            reason: Some(MobileBearerReason::ConnectionInterrupted),
        })
        .await
        .unwrap();
    assert_eq!(
        platform.request_android_usb_fallback().await,
        MobileUsbFallbackDisposition::Accepted
    );
    platform
        .report(MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Reconnecting,
            reason: None,
        })
        .await
        .unwrap();
    assert_eq!(
        platform.report(usb_connected.clone()).await,
        Err("Android USB cannot preempt approved Bluetooth")
    );
    platform
        .report(MobileBearerObservation {
            kind: MobileBearerKind::BluetoothRnode,
            state: MobileBearerState::Disconnected,
            reason: Some(MobileBearerReason::ConnectionInterrupted),
        })
        .await
        .unwrap();
    platform.report(usb_connected.clone()).await.unwrap();
    let snapshot = node.session_snapshot().await;
    assert_eq!(snapshot.bearer(MobileBearerKind::AndroidUsb), Some(&usb_connected));

    shutdown(node).await;
}

#[tokio::test]
async fn refused_session_snapshot_exposes_recoverable_typed_failure() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    wait_for_interface_status(&node, "retrying", Duration::from_secs(2)).await;
    let snapshot = node.session_snapshot().await;
    let failure = snapshot.failure.as_ref().expect("retrying TCP failure");

    assert_eq!(snapshot.phase, MobileConnectionPhase::Reconnecting);
    assert_eq!(snapshot.endpoint.as_deref(), Some(address.to_string().as_str()));
    assert_eq!(snapshot.generation, 1);
    assert_eq!(failure.code, MobileFailureCode::TcpRetrying);
    assert!(failure.retryable);
    assert_eq!(
        snapshot.bearer(MobileBearerKind::Tcp).expect("TCP bearer").state,
        MobileBearerState::Reconnecting
    );
    assert_eq!(
        snapshot.bearer(MobileBearerKind::BluetoothRnode).expect("Bluetooth RNode bearer").state,
        MobileBearerState::Unavailable
    );
    shutdown(node).await;
}

#[tokio::test]
async fn cold_boot_restores_the_persisted_tcp_endpoint_and_identity() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut first_config = config(root.path(), None);
    first_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let first = MobileNode::boot(first_config).await.unwrap();
    let (first_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("first persisted endpoint connection timed out")
        .unwrap();
    wait_for_interface_status(&first, "connected", Duration::from_secs(2)).await;
    let identity = first.app_context.identity().identity_hash().to_string();
    shutdown(first).await;
    drop(first_stream);

    let second = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let (second_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("restored endpoint did not connect automatically")
        .unwrap();
    wait_for_interface_status(&second, "connected", Duration::from_secs(2)).await;
    let snapshot = second.session_snapshot().await;

    assert_eq!(snapshot.endpoint.as_deref(), Some(address.to_string().as_str()));
    assert_eq!(second.app_context.identity().identity_hash(), identity);
    drop(second_stream);
    shutdown(second).await;
}

#[tokio::test]
async fn explicit_endpoint_edit_replaces_the_persisted_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let first_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_address = first_listener.local_addr().unwrap();
    let mut first_config = config(root.path(), None);
    first_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: first_address.to_string() });
    let first = MobileNode::boot(first_config).await.unwrap();
    let (first_stream, _) = tokio::time::timeout(Duration::from_secs(2), first_listener.accept())
        .await
        .expect("initial endpoint connection timed out")
        .unwrap();
    shutdown(first).await;
    drop(first_stream);

    let second_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second_address = second_listener.local_addr().unwrap();
    let mut edited_config = config(root.path(), None);
    edited_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: second_address.to_string() });
    let edited = MobileNode::boot(edited_config).await.unwrap();
    let (edited_stream, _) = tokio::time::timeout(Duration::from_secs(2), second_listener.accept())
        .await
        .expect("edited endpoint connection timed out")
        .unwrap();
    shutdown(edited).await;
    drop(edited_stream);

    let restored = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let (restored_stream, _) =
        tokio::time::timeout(Duration::from_secs(2), second_listener.accept())
            .await
            .expect("edited endpoint was not restored")
            .unwrap();
    let snapshot = restored.session_snapshot().await;

    assert_eq!(snapshot.endpoint.as_deref(), Some(second_address.to_string().as_str()));
    drop(restored_stream);
    shutdown(restored).await;
}

#[tokio::test]
async fn established_tcp_interruption_reconnects_without_replacing_node() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });

    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (first_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("initial TCP connection timed out")
        .unwrap();
    let first = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;
    let first_generation = node.session_snapshot().await.generation;
    let identity = node.app_context.identity().identity_hash().to_string();

    drop(first_stream);
    let (second_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("established TCP interruption did not reconnect")
        .unwrap();
    let second = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;
    let second_generation = node.session_snapshot().await.generation;

    assert_eq!(second.hash, first.hash);
    assert_eq!(second_generation, first_generation + 1);
    assert_eq!(node.app_context.identity().identity_hash(), identity);
    shutdown(node).await;
    drop(second_stream);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), listener.accept()).await.is_err(),
        "shutdown client reconnected after ownership ended"
    );
}

#[test]
fn malformed_endpoint_edit_is_typed_and_preserves_the_durable_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = persist_mobile_tcp_endpoint(root.path(), "rns.styrene.io:4242")
        .expect("valid endpoint persists");

    let error = persist_mobile_tcp_endpoint(root.path(), "not an endpoint")
        .expect_err("malformed endpoint must fail");

    assert_eq!(error.code(), MobileFailureCode::InvalidTcpEndpoint);
    assert!(error.retryable());
    assert_eq!(
        load_mobile_tcp_endpoint(root.path()).expect("persisted endpoint loads"),
        Some(endpoint)
    );
}

#[test]
fn malformed_persisted_endpoint_is_a_recoverable_typed_failure() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("mobile.toml"),
        "schema_version = 1\ntcp_endpoint = 'not an endpoint'\n",
    )
    .unwrap();

    let error = load_mobile_tcp_endpoint(root.path()).expect_err("malformed persisted endpoint");

    assert_eq!(error.code(), MobileFailureCode::InvalidTcpEndpoint);
    assert!(error.retryable());
}

#[tokio::test]
async fn peer_snapshot_is_destination_keyed_fresh_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let destination = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let first = lxmf::announce::encode_delivery_display_name_app_data("First Name").unwrap();
    let second = lxmf::announce::encode_delivery_display_name_app_data("Current Name").unwrap();
    node.app_context.discovery().accept_delivery_announce(destination.into(), 100, &first).unwrap();
    node.app_context
        .discovery()
        .accept_delivery_announce(destination.into(), 105, &second)
        .unwrap();

    let snapshot = node.peer_snapshot_at(110).await.expect("mobile peer snapshot");

    assert_eq!(snapshot.generation, 1);
    assert_eq!(snapshot.observed_at, 110);
    assert_eq!(snapshot.peers.len(), 1);
    let peer = &snapshot.peers[0];
    assert_eq!(peer.destination_hash, destination);
    assert_eq!(peer.aspect, MobilePeerAspect::LxmfDelivery);
    assert_eq!(peer.display_name.as_deref(), Some("Current Name"));
    assert_eq!(peer.observed_at, 105);
    assert_eq!(peer.age_secs, 5);
    assert_eq!(peer.source, MobilePeerSource::CanonicalAnnounce);
    assert_eq!(peer.announce_count, 2);
    shutdown(node).await;

    let restored = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let restored_snapshot = restored.peer_snapshot_at(120).await.expect("restored peer snapshot");
    assert_eq!(restored_snapshot.peers.len(), 1);
    assert_eq!(restored_snapshot.peers[0].destination_hash, destination);
    assert_eq!(restored_snapshot.peers[0].display_name.as_deref(), Some("Current Name"));
    shutdown(restored).await;
}

#[tokio::test]
async fn local_announce_reports_dispatch_acceptance_without_remote_receipt() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });
    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("announce transport connection timed out")
        .unwrap();
    wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    let outcome = node.announce_outcome().await.expect("local announce dispatch");

    assert!(outcome.local_dispatch_accepted);
    assert!(!outcome.remote_reception_confirmed);
    assert_eq!(outcome.generation, node.session_snapshot().await.generation);
    assert!(outcome.accepted_at > 0);
    drop(stream);
    shutdown(node).await;
}

#[tokio::test]
async fn local_announce_returns_typed_failure_without_transport() {
    let root = tempfile::tempdir().unwrap();
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();

    let error = node.announce_outcome().await.expect_err("offline announce must fail");

    assert_eq!(error.code(), MobileFailureCode::TransportUnavailable);
    assert!(error.retryable());
    shutdown(node).await;
}

#[tokio::test]
async fn typed_direct_send_returns_and_restores_the_authoritative_failed_projection() {
    let root = tempfile::tempdir().unwrap();
    let destination = "cccccccccccccccccccccccccccccccc";
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();

    let outcome = node
        .send_text(MobileSendRequest {
            destination_hash: destination.into(),
            content: "persist before dispatch".into(),
            requested_method: MobileDeliveryMethod::Direct,
            draft_revision: None,
        })
        .await
        .expect("authoritative persisted send outcome");

    assert_eq!(outcome.disposition, MobileSendDisposition::Failed);
    assert_eq!(outcome.requested_method, MobileDeliveryMethod::Direct);
    assert_eq!(outcome.actual_method, MobileDeliveryMethod::Direct);
    assert_eq!(outcome.message.id, outcome.message_id);
    assert_eq!(outcome.message.destination_hash, destination);
    assert_eq!(outcome.message.correlation_id.as_deref(), Some(outcome.message_id.as_str()));
    assert!(outcome.message.projection_complete);
    assert!(outcome.terminal_failure.as_ref().is_some_and(|failure| failure.retryable));

    let exact =
        node.message(&outcome.message_id).await.expect("message query").expect("persisted message");
    assert_eq!(exact, outcome.message);
    assert_eq!(node.get_messages(destination, 10).await.unwrap().len(), 1);
    let message_id = outcome.message_id;
    shutdown(node).await;

    let restored = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let exact = restored
        .message(&message_id)
        .await
        .expect("restored message query")
        .expect("restored persisted message");
    assert_eq!(exact.id, message_id);
    assert_eq!(exact.destination_hash, destination);
    assert_eq!(restored.get_messages(destination, 10).await.unwrap().len(), 1);
    let retry = restored.retry_text(&message_id).await.expect("restart-safe retry");
    assert_eq!(retry.disposition, MobileRetryDisposition::Applied);
    assert_eq!(retry.message.id, message_id);
    assert_eq!(retry.message.correlation_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(
        retry.message.attempts.iter().map(|attempt| attempt.number).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(restored.get_messages(destination, 10).await.unwrap().len(), 1);
    shutdown(restored).await;
}

#[tokio::test]
async fn draft_revision_prevents_an_older_send_from_clearing_a_newer_edit() {
    let root = tempfile::tempdir().unwrap();
    let destination = "dddddddddddddddddddddddddddddddd";
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let submitted = node.set_draft(destination, "submitted text").await.unwrap();
    let newer = node.set_draft(destination, "newer edit").await.unwrap();

    assert_eq!(submitted.revision, 1);
    assert_eq!(newer.revision, 2);
    assert_eq!(
        node.clear_draft_if_revision(destination, submitted.revision).await.unwrap(),
        MobileDraftClearDisposition::Superseded
    );
    assert_eq!(node.draft(destination).await.unwrap(), Some(newer.clone()));
    shutdown(node).await;

    let restored = MobileNode::boot(config(root.path(), None)).await.unwrap();
    assert_eq!(restored.draft(destination).await.unwrap(), Some(newer.clone()));
    assert_eq!(
        restored.clear_draft_if_revision(destination, newer.revision).await.unwrap(),
        MobileDraftClearDisposition::Cleared
    );
    assert!(restored.draft(destination).await.unwrap().is_none());
    shutdown(restored).await;
}

#[tokio::test]
async fn active_conversation_marks_new_inbound_messages_read_and_restores_unread_state() {
    let root = tempfile::tempdir().unwrap();
    let inactive_peer = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let active_peer = "ffffffffffffffffffffffffffffffff";
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();
    node.set_active_conversation(Some(active_peer)).await.unwrap();

    let inactive = MessageRecord {
        id: "inactive-inbound".into(),
        source: inactive_peer.into(),
        destination: "00".repeat(16),
        title: String::new(),
        content: "unread while inactive".into(),
        timestamp: 100,
        direction: "in".into(),
        fields: None,
        receipt_status: None,
        read: false,
    };
    let active = MessageRecord {
        id: "active-inbound".into(),
        source: active_peer.into(),
        destination: "00".repeat(16),
        title: String::new(),
        content: "read while active".into(),
        timestamp: 101,
        direction: "in".into(),
        fields: None,
        receipt_status: None,
        read: false,
    };
    assert!(node.app_context.messaging().accept_inbound_record(&inactive).unwrap());
    node.app_context.events().emit_message_new(&inactive, None);
    assert!(node.app_context.messaging().accept_inbound_record(&active).unwrap());
    node.app_context.events().emit_message_new(&active, None);

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let conversations = node.list_conversations().await.unwrap();
            let inactive_unread = conversations
                .iter()
                .find(|conversation| conversation.peer_hash == inactive_peer)
                .map(|conversation| conversation.unread_count);
            let active_unread = conversations
                .iter()
                .find(|conversation| conversation.peer_hash == active_peer)
                .map(|conversation| conversation.unread_count);
            if inactive_unread == Some(1) && active_unread == Some(0) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("active conversation read update timed out");
    shutdown(node).await;

    let restored = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let conversations = restored.list_conversations().await.unwrap();
    assert_eq!(
        conversations
            .iter()
            .find(|conversation| conversation.peer_hash == inactive_peer)
            .map(|conversation| conversation.unread_count),
        Some(1)
    );
    assert_eq!(
        conversations
            .iter()
            .find(|conversation| conversation.peer_hash == active_peer)
            .map(|conversation| conversation.unread_count),
        Some(0)
    );
    shutdown(restored).await;
}

#[tokio::test]
async fn message_events_are_generation_scoped_complete_canonical_projections() {
    let root = tempfile::tempdir().unwrap();
    let peer = "abababababababababababababababab";
    let node = MobileNode::boot(config(root.path(), None)).await.unwrap();
    let mut subscription = node.subscribe_message_events().await;
    let generation = node.session_snapshot().await.generation;
    let inbound = MessageRecord {
        id: "event-inbound".into(),
        source: peer.into(),
        destination: "00".repeat(16),
        title: String::new(),
        content: "complete event projection".into(),
        timestamp: 200,
        direction: "in".into(),
        fields: None,
        receipt_status: None,
        read: false,
    };
    assert!(node.app_context.messaging().accept_inbound_record(&inbound).unwrap());
    node.app_context.events().emit_message_new(&inbound, None);

    let event = tokio::time::timeout(Duration::from_secs(1), subscription.recv())
        .await
        .expect("message event timeout")
        .expect("message event projection");

    assert_eq!(event.generation, generation);
    assert!(event.message.projection_complete);
    assert_eq!(event.message.id, inbound.id);
    assert_eq!(event.message.source_hash, peer);
    assert_eq!(event.message.content, inbound.content);
    assert!(!event.message.read);
    shutdown(node).await;
}

#[tokio::test]
async fn peer_event_retains_subscription_generation_across_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut mobile_config = config(root.path(), None);
    mobile_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: address.to_string() });
    let node = MobileNode::boot(mobile_config).await.unwrap();
    let (first_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("initial peer-event connection timed out")
        .unwrap();
    wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;
    let mut subscription = node.subscribe_peer_events().await;
    let subscription_generation = node.session_snapshot().await.generation;

    drop(first_stream);
    let (second_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("peer-event reconnect timed out")
        .unwrap();
    let current_generation =
        wait_for_generation(&node, subscription_generation + 1, Duration::from_secs(1)).await;
    assert!(current_generation > subscription_generation);

    let destination = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let app_data = lxmf::announce::encode_delivery_display_name_app_data("Event Peer").unwrap();
    node.app_context
        .discovery()
        .accept_delivery_announce(destination.into(), 100, &app_data)
        .unwrap();
    let device = node.app_context.discovery().device(destination).unwrap();
    node.app_context.events().emit_device(device);
    let event = tokio::time::timeout(Duration::from_secs(1), subscription.recv())
        .await
        .expect("peer event timeout")
        .expect("peer event");

    assert_eq!(event.generation, subscription_generation);
    assert_ne!(event.generation, current_generation);
    assert_eq!(event.peer.destination_hash, destination);
    drop(second_stream);
    shutdown(node).await;
}

#[tokio::test]
async fn hub_boot_publishes_destination_and_normalized_display_name() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = listener.local_addr().unwrap().to_string();
    let accept = tokio::spawn(async move { listener.accept().await.unwrap().0 });

    let node = MobileNode::boot(config(root.path(), Some(hub))).await.unwrap();
    let stream = tokio::time::timeout(Duration::from_secs(2), accept)
        .await
        .expect("hub connection timed out")
        .expect("hub accept task failed");
    wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;
    let delivery_hash = node.delivery_hash().expect("hub-backed delivery hash");

    assert_eq!(delivery_hash.len(), 32);
    assert_ne!(delivery_hash, "00000000000000000000000000000000");
    assert_eq!(node.app_context.identity().delivery_destination_hash(), Some(delivery_hash));
    assert_eq!(node.app_context.identity().display_name().as_deref(), Some("Classroom Red"));
    assert!(node.is_connected());
    assert!(node.startup_contract().has_component(components::LXMF_DELIVERY));
    assert!(node.startup_contract().has_component(components::SERVICE_RECEIPT_BRIDGE));
    assert_eq!(components::SERVICE_RECEIPT_BRIDGE.id, "service-rns-delivery-receipts");
    assert!(node.startup_contract().has_component(components::OUTBOUND_RESOURCE_COMPLETION_WORKER));
    assert!(node.startup_contract().has_component(components::NATIVE_RESOURCE_RETRY_SCHEDULER));
    assert!(node.startup_contract().has_component(components::LXMF_ROUTER_DEADLINE_SCHEDULER));
    assert!(node.startup_contract().advertises(capabilities::LXMF_DIRECT.id()));
    assert!(!node.startup_contract().advertises(capabilities::STYRENE_RPC.id()));
    assert!(!node.startup_contract().advertises(capabilities::NATIVE_NOMADNET_HOST.id()));
    assert_eq!(node.app_context.protocol().handler_count().await, 0);

    shutdown(node).await;
    drop(stream);
}

#[tokio::test]
async fn invalid_display_name_is_omitted() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path(), None);
    config.display_name = Some("bad\nname".into());

    let node = MobileNode::boot(config).await.unwrap();

    assert_eq!(node.app_context.identity().display_name(), None);
    shutdown(node).await;
}

#[test]
fn mobile_config_paths_remain_host_owned() {
    let config = MobileConfig {
        config_dir: PathBuf::from("/app/config"),
        data_dir: PathBuf::from("/app/data"),
        hub_address: None,
        hub_delivery_hash: None,
        display_name: None,
        identity_backend: IdentityBackend::PlaintextFile,
        interfaces: Vec::new(),
        enable_rnode_channel: false,
    };

    assert_eq!(config.config_dir, PathBuf::from("/app/config"));
    assert_eq!(config.data_dir, PathBuf::from("/app/data"));
}

#[tokio::test]
async fn tcp_server_reports_actual_ephemeral_listener() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path(), None);
    config.interfaces.push(MobileInterfaceConfig::TcpServer { bind_address: "127.0.0.1:0".into() });

    let node = MobileNode::boot(config).await.unwrap();
    let listeners = node.tcp_listen_addresses();

    assert_eq!(listeners.len(), 1);
    assert_eq!(listeners[0].ip().to_string(), "127.0.0.1");
    assert_ne!(listeners[0].port(), 0);
    assert!(node.delivery_hash().is_some());
    let listener = listeners[0];
    shutdown(node).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(tokio::net::TcpStream::connect(listener).await.is_err());
}

#[tokio::test]
async fn invalid_and_duplicate_tcp_profiles_fail_before_boot() {
    let root = tempfile::tempdir().unwrap();
    let mut empty = config(root.path(), None);
    empty.interfaces.push(MobileInterfaceConfig::TcpClient { remote_address: "   ".into() });
    let error = match MobileNode::boot(empty).await {
        Ok(_) => panic!("empty interface unexpectedly booted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("empty"));

    let root = tempfile::tempdir().unwrap();
    let mut duplicate = config(root.path(), Some("127.0.0.1:4242".into()));
    duplicate
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: " 127.0.0.1:4242 ".into() });
    let error = match MobileNode::boot(duplicate).await {
        Ok(_) => panic!("duplicate interface unexpectedly booted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("duplicate"));

    let root = tempfile::tempdir().unwrap();
    let mut malformed = config(root.path(), None);
    malformed
        .interfaces
        .push(MobileInterfaceConfig::TcpServer { bind_address: "localhost:not-a-port".into() });
    let error = match MobileNode::boot(malformed).await {
        Ok(_) => panic!("malformed interface unexpectedly booted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("invalid TCP server"));
}

#[tokio::test]
async fn multiple_tcp_servers_report_listeners_in_profile_order() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path(), None);
    config.interfaces.extend([
        MobileInterfaceConfig::TcpServer { bind_address: "127.0.0.1:0".into() },
        MobileInterfaceConfig::TcpServer { bind_address: "[::1]:0".into() },
    ]);

    let node = MobileNode::boot(config).await.unwrap();
    let listeners = node.tcp_listen_addresses();

    assert_eq!(listeners.len(), 2);
    assert!(listeners[0].is_ipv4());
    assert!(listeners[1].is_ipv6());
    shutdown(node).await;
}

async fn wait_for_peer_event(
    announcer: &MobileNode,
    subscription: &mut MobilePeerSubscription,
    destination: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        announcer.announce_outcome().await.expect("announce dispatch");
        let observed = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                let event = subscription.recv().await.expect("peer event");
                if event.peer.destination_hash == destination {
                    return;
                }
            }
        })
        .await;
        if observed.is_ok() {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "peer discovery event timed out");
    }
}

async fn wait_for_new_message_event(
    subscription: &mut MobileMessageSubscription,
    message_id: &str,
) -> MobileMessageEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = subscription.recv().await.expect("message event");
            if event.kind == MobileMessageEventKind::New && event.message.id == message_id {
                return event;
            }
        }
    })
    .await
    .expect("inbound message event timed out")
}

#[tokio::test]
async fn two_mobile_nodes_exchange_lxmf_directly_without_hub() {
    let server_root = tempfile::tempdir().unwrap();
    let mut server_config = config(server_root.path(), None);
    server_config.display_name = Some("Direct Red".into());
    server_config
        .interfaces
        .push(MobileInterfaceConfig::TcpServer { bind_address: "127.0.0.1:0".into() });
    let server = MobileNode::boot(server_config).await.unwrap();
    let listen_address = server.tcp_listen_addresses()[0];

    let client_root = tempfile::tempdir().unwrap();
    let mut client_config = config(client_root.path(), None);
    client_config.display_name = Some("Direct Yellow".into());
    client_config
        .interfaces
        .push(MobileInterfaceConfig::TcpClient { remote_address: listen_address.to_string() });
    let client = MobileNode::boot(client_config).await.unwrap();

    wait_for_interface_status(&client, "connected", Duration::from_secs(2)).await;
    let mut server_peers = server.subscribe_peer_events().await;
    let mut client_peers = client.subscribe_peer_events().await;
    let server_delivery = server.delivery_hash().unwrap();
    let client_delivery = client.delivery_hash().unwrap();
    wait_for_peer_event(&server, &mut client_peers, &server_delivery).await;
    wait_for_peer_event(&client, &mut server_peers, &client_delivery).await;

    let server_identity = server.app_context.identity().identity_hash().to_string();
    let client_identity = client.app_context.identity().identity_hash().to_string();
    let mut client_messages = client.subscribe_message_events().await;
    let red_outcome = server
        .send_text(MobileSendRequest {
            destination_hash: client_delivery.clone(),
            content: "red to yellow".into(),
            requested_method: MobileDeliveryMethod::Direct,
            draft_revision: None,
        })
        .await
        .unwrap();
    assert_eq!(red_outcome.disposition, MobileSendDisposition::Accepted);
    assert_eq!(red_outcome.requested_method, MobileDeliveryMethod::Direct);
    assert_eq!(red_outcome.actual_method, MobileDeliveryMethod::Direct);
    assert_eq!(
        red_outcome.message.correlation_id.as_deref(),
        Some(red_outcome.message_id.as_str())
    );
    assert_eq!(red_outcome.message.attempts.len(), 1);
    let red_inbound =
        wait_for_new_message_event(&mut client_messages, &red_outcome.message_id).await;
    assert_eq!(red_inbound.kind, MobileMessageEventKind::New);
    assert_eq!(red_inbound.message.id, red_outcome.message_id);
    assert_eq!(red_inbound.message.source_hash, server_identity);
    assert_eq!(red_inbound.message.destination_hash, client_delivery);
    assert_eq!(red_inbound.message.content, "red to yellow");

    let mut server_messages = server.subscribe_message_events().await;
    let yellow_outcome = client
        .send_text(MobileSendRequest {
            destination_hash: server_delivery.clone(),
            content: "yellow to red".into(),
            requested_method: MobileDeliveryMethod::Direct,
            draft_revision: None,
        })
        .await
        .unwrap();
    assert_eq!(yellow_outcome.disposition, MobileSendDisposition::Accepted);
    let yellow_inbound =
        wait_for_new_message_event(&mut server_messages, &yellow_outcome.message_id).await;
    assert_eq!(yellow_inbound.message.id, yellow_outcome.message_id);
    assert_eq!(yellow_inbound.message.source_hash, client_identity);
    assert_eq!(yellow_inbound.message.destination_hash, server_delivery);
    assert_eq!(yellow_inbound.message.content, "yellow to red");

    let server_outbound = server.get_messages(&client_delivery, 20).await.unwrap();
    assert_eq!(server_outbound.len(), 1);
    assert_eq!(server_outbound[0].id, red_outcome.message_id);
    assert!(server_outbound[0].is_outgoing);
    let client_outbound = client.get_messages(&server_delivery, 20).await.unwrap();
    assert_eq!(client_outbound.len(), 1);
    assert_eq!(client_outbound[0].id, yellow_outcome.message_id);
    assert!(client_outbound[0].is_outgoing);

    let client_inbound = client
        .get_messages(&server_identity, 20)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| {
            !message.is_outgoing
                && message.source_hash == server_identity
                && message.content == "red to yellow"
        })
        .count();
    assert_eq!(client_inbound, 1);

    let server_inbound = server
        .get_messages(&client_identity, 20)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| {
            !message.is_outgoing
                && message.source_hash == client_identity
                && message.content == "yellow to red"
        })
        .count();
    assert_eq!(server_inbound, 1);

    let unread_conversation = server
        .list_conversations()
        .await
        .unwrap()
        .into_iter()
        .find(|conversation| conversation.peer_hash == client_identity)
        .expect("unread conversation is visible");
    assert_eq!(unread_conversation.unread_count, 1);

    server.mark_read(&client_identity).await.unwrap();
    let conversation = server
        .list_conversations()
        .await
        .unwrap()
        .into_iter()
        .find(|conversation| conversation.peer_hash == client_identity)
        .expect("read conversation remains visible");
    assert_eq!(conversation.unread_count, 0);

    shutdown(client).await;
    shutdown(server).await;
}
