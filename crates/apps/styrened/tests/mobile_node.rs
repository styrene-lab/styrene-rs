use std::path::PathBuf;

use styrene_ipc::traits::DaemonStatus;
use styrened::mobile::{IdentityBackend, MobileConfig, MobileInterfaceConfig, MobileNode};
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
async fn hub_boot_publishes_destination_and_normalized_display_name() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = listener.local_addr().unwrap().to_string();
    let accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    let node = MobileNode::boot(config(root.path(), Some(hub))).await.unwrap();
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
    accept.abort();
    let _ = accept.await;
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

    shutdown(client).await;
    shutdown(server).await;
}
