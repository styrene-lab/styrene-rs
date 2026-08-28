use std::path::PathBuf;
use std::time::Duration;

use styrene_ipc::traits::DaemonStatus;
use styrene_ipc::types::InterfaceDetail;
use styrened::mobile::{
    load_mobile_tcp_endpoint, persist_mobile_tcp_endpoint, IdentityBackend, MobileBearerKind,
    MobileBearerState, MobileConfig, MobileConnectionPhase, MobileFailureCode,
    MobileInterfaceConfig, MobileNode,
};
use styrened::startup_contract::{RuntimeKind, capabilities, components};

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
    let identity = node.app_context.identity().identity_hash().to_string();

    drop(first_stream);
    let (second_stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("established TCP interruption did not reconnect")
        .unwrap();
    let second = wait_for_interface_status(&node, "connected", Duration::from_secs(2)).await;

    assert_eq!(second.hash, first.hash);
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

async fn wait_for_peer(node: &MobileNode, destination: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if node.list_peers().await.unwrap().iter().any(|peer| peer.destination_hash == destination)
        {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "peer discovery timed out");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn wait_for_content(node: &MobileNode, peer_identity: &str, content: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if node
            .get_messages(peer_identity, 20)
            .await
            .unwrap()
            .iter()
            .any(|message| !message.is_outgoing && message.content == content)
        {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "LXMF delivery timed out");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
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

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    server.announce().await.unwrap();
    client.announce().await.unwrap();
    let server_delivery = server.delivery_hash().unwrap();
    let client_delivery = client.delivery_hash().unwrap();
    wait_for_peer(&server, &client_delivery).await;
    wait_for_peer(&client, &server_delivery).await;

    let server_identity = server.app_context.identity().identity_hash().to_string();
    let client_identity = client.app_context.identity().identity_hash().to_string();
    server.send_chat(&client_delivery, "red to yellow").await.unwrap();
    wait_for_content(&client, &server_identity, "red to yellow").await;
    client.send_chat(&server_delivery, "yellow to red").await.unwrap();
    wait_for_content(&server, &client_identity, "yellow to red").await;

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
