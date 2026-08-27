use styrene_ipc::traits::DaemonStatus;
use styrene_rbac::Capability;
use styrened::daemon::{self, DaemonConfig2};
use styrened::daemon_facade::DaemonFacade;
use styrened::startup_contract::{capabilities, components, RuntimeKind};

#[tokio::test]
async fn canonical_runtime_advertises_only_composed_capabilities() {
    let root = tempfile::tempdir().unwrap();
    let handle = daemon::start(DaemonConfig2 {
        db: Some(root.path().join("messages.db")),
        config: None,
        identity: None,
        socket: Some(root.path().join("daemon.sock")),
        ephemeral: true,
    })
    .await
    .unwrap();

    let contract = handle.startup_contract();
    assert_eq!(contract.runtime(), RuntimeKind::Canonical);
    assert!(contract.has_component(components::LXMF_DELIVERY));
    assert!(contract.has_component(components::NOMADNET_NODE_DESTINATION));
    assert!(contract.has_component(components::NOMADNET_NODE_ANNOUNCE));
    assert!(contract.has_component(components::RPC_REQUEST_HANDLER));
    assert!(contract.has_component(components::PROPAGATION_EXPIRY_SCHEDULER));
    assert!(contract.has_component(components::SERVICE_RECEIPT_BRIDGE));
    assert_eq!(components::SERVICE_RECEIPT_BRIDGE.id, "service-rns-delivery-receipts");
    assert!(contract.has_component(components::OUTBOUND_RESOURCE_COMPLETION_WORKER));
    assert!(contract.has_component(components::ROUTE_WORKER));
    assert!(contract.has_component(components::CONFIG_SERVICE));
    assert!(contract.has_component(components::POLICY_SERVICE));
    assert!(contract.has_component(components::FLEET_SERVICE));
    assert!(contract.has_component(components::REQUEST_STATE_SERVICE));
    assert!(contract.has_component(components::REQUEST_CANCELLATION_SERVICE));
    assert!(contract.has_component(components::RESOURCE_STATE_SERVICE));
    assert!(contract.has_component(components::RESOURCE_CANCELLATION_SERVICE));
    assert!(contract.has_component(components::TRANSPORT_REQUEST_OBSERVATION_BRIDGE));
    assert!(contract.has_component(components::TRANSPORT_RESOURCE_OBSERVATION_BRIDGE));
    assert!(contract.has_component(components::NATIVE_RESOURCE_RETRY_SCHEDULER));
    assert!(contract.has_component(components::LXMF_ROUTER_DEADLINE_SCHEDULER));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER));
    #[cfg(feature = "ipc-server")]
    assert!(contract.has_component(components::IPC_EVENT_BRIDGE));
    #[cfg(not(feature = "ipc-server"))]
    assert!(!contract.has_component(components::IPC_EVENT_BRIDGE));
    assert!(!contract.has_component(components::LEGACY_RECEIPT_BRIDGE));
    assert!(contract.advertises(capabilities::LXMF_DIRECT.id()));
    assert!(contract.advertises(capabilities::STYRENE_RPC.id()));
    assert!(contract.advertises(capabilities::LOCAL_CONFIG.id()));
    assert!(contract.advertises(capabilities::LOCAL_POLICY.id()));
    assert!(contract.advertises(capabilities::RNS_REQUESTS.id()));
    assert!(contract.advertises(capabilities::RNS_REQUEST_CANCELLATION.id()));
    assert!(contract.advertises(capabilities::RNS_RESOURCE_CANCELLATION.id()));
    assert!(contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION_CLIENT.id()));
    assert!(contract.has_component(components::NATIVE_NOMADNET_REQUEST_HANDLER));
    assert!(contract.advertises(capabilities::NATIVE_NOMADNET_HOST.id()));
    assert!(contract.missing_requirements(capabilities::NATIVE_NOMADNET_HOST).is_empty());
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(!contract.has_component(components::STANDARD_LXMF_PROPAGATION_DESTINATION));
    assert_eq!(handle.app_context.protocol().handler_count().await, 2);
    let active = handle.active_capabilities("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(active.runtime().contains(&capabilities::LXMF_DIRECT.id()));
    assert!(active.authorized_operations().iter().any(|cap| cap == Capability::CHAT_SEND));
    assert!(!active.authorized_operations().iter().any(|cap| cap == Capability::RPC_EXEC));
    let status = handle.daemon_facade.query_status().await.unwrap();
    assert!(!status.propagation_enabled);
    assert!(!status.standard_lxmf_propagation_destination_registered);
    assert!(!status.standard_lxmf_propagation_active);
    let negotiated = status.active_capabilities.expect("published startup capabilities");
    assert_eq!(negotiated.version, styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION);
    assert!(negotiated.runtime.iter().any(|id| id == capabilities::LXMF_DIRECT.id()));
    assert!(negotiated.authorized_operations.iter().any(|id| id == Capability::RPC_EXEC));
    assert_eq!(status.connection_generation, None);
    let peer =
        DaemonFacade::new(handle.app_context.clone(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let peer_capabilities = peer.query_status().await.unwrap().active_capabilities.unwrap();
    assert!(!peer_capabilities.authorized_operations.iter().any(|id| id == Capability::RPC_EXEC));

    tokio::time::timeout(std::time::Duration::from_secs(5), handle.shutdown())
        .await
        .expect("canonical daemon shutdown timed out");
}

#[tokio::test]
async fn canonical_hub_activates_standard_propagation_after_complete_composition() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    std::fs::write(&config_path, "role = \"hub\"\n").unwrap();
    let handle = daemon::start(DaemonConfig2 {
        db: Some(root.path().join("messages.db")),
        config: Some(config_path),
        identity: Some(root.path().join("identity")),
        socket: Some(root.path().join("daemon.sock")),
        ephemeral: true,
    })
    .await
    .unwrap();
    let contract = handle.startup_contract();
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_DESTINATION));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_ANNOUNCE));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_OFFER_HANDLER));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_GET_HANDLER));
    assert!(contract.has_component(components::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER));
    assert!(contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION_CLIENT.id()));
    assert!(!contract.has_component(components::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER));
    assert!(contract.missing_requirements(capabilities::STANDARD_LXMF_PROPAGATION).is_empty());
    let components = contract.components();
    let ready = components
        .iter()
        .position(|component| *component == components::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER)
        .unwrap();
    let announced = components
        .iter()
        .position(|component| *component == components::STANDARD_LXMF_PROPAGATION_ANNOUNCE)
        .unwrap();
    assert!(ready < announced);
    assert!(handle.standard_propagation_destination_hash().await.is_some());
    let status = handle.daemon_facade.query_status().await.unwrap();
    assert!(status.propagation_enabled);
    assert!(status.standard_lxmf_propagation_destination_registered);
    assert!(status.standard_lxmf_propagation_active);
    let propagation = handle.daemon_facade.query_standard_propagation().await.unwrap();
    assert!(propagation.registered);
    assert!(propagation.active);
    let policy = propagation.policy.unwrap();
    assert_eq!(policy.target_cost, 16);
    assert_eq!(policy.flexibility, 3);
    assert_eq!(policy.peering_cost, 18);
    assert_eq!(policy.transfer_limit_kb, 256);
    assert_eq!(policy.sync_limit_kb, 4000);
    assert_eq!(policy.queue_max_count, 4096);
    assert_eq!(policy.queue_max_bytes, 16 * 1024 * 1024);
    assert_eq!(policy.expiry_secs, 30 * 24 * 60 * 60);
    assert_eq!(policy.throttle_secs, 180);
    assert_eq!(policy.max_offer_links, 3);
    handle.shutdown().await;
}

#[tokio::test]
async fn canonical_hub_restart_reuses_identity_and_drops_old_destination_registry() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    std::fs::write(
        &config_path,
        "role = \"hub\"\n[[interfaces]]\ntype = \"tcp_server\"\nenabled = true\nhost = \"127.0.0.1\"\nport = 0\n",
    )
    .unwrap();
    let identity_path = root.path().join("identity");
    let start = || DaemonConfig2 {
        db: Some(root.path().join("messages.db")),
        config: Some(config_path.clone()),
        identity: Some(identity_path.clone()),
        socket: Some(root.path().join("daemon.sock")),
        ephemeral: false,
    };

    let first = daemon::start(start()).await.unwrap();
    let first_hash = first.standard_propagation_destination_hash().await.unwrap();
    let old_destination = first.standard_propagation_destination_weak().unwrap();
    first.shutdown().await;
    assert!(old_destination.upgrade().is_none());

    let second = daemon::start(start()).await.unwrap();
    assert_eq!(
        second.standard_propagation_destination_hash().await.as_deref(),
        Some(first_hash.as_str())
    );
    second.shutdown().await;
}

#[tokio::test]
async fn canonical_non_transport_role_does_not_advertise_transport_capabilities() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    std::fs::write(&config_path, "role = \"propagation_client\"\n").unwrap();
    let handle = daemon::start(DaemonConfig2 {
        db: Some(root.path().join("messages.db")),
        config: Some(config_path),
        identity: None,
        socket: Some(root.path().join("daemon.sock")),
        ephemeral: true,
    })
    .await
    .unwrap();

    let contract = handle.startup_contract();
    assert!(!contract.has_component(components::LXMF_DELIVERY));
    assert!(!contract.has_component(components::NOMADNET_NODE_DESTINATION));
    assert!(!contract.has_component(components::NOMADNET_NODE_ANNOUNCE));
    assert!(!contract.has_component(components::SERVICE_RECEIPT_BRIDGE));
    assert!(!contract.has_component(components::NATIVE_RESOURCE_RETRY_SCHEDULER));
    assert!(!contract.advertises(capabilities::LXMF_DIRECT.id()));
    assert!(!contract.advertises(capabilities::STYRENE_RPC.id()));
    assert!(contract.advertises(capabilities::LOCAL_CONFIG.id()));
    assert!(contract.advertises(capabilities::LOCAL_POLICY.id()));
    assert!(!contract.advertises(capabilities::RNS_REQUESTS.id()));
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION_CLIENT.id()));
    assert!(!contract.has_component(components::REQUEST_STATE_SERVICE));
    assert_eq!(handle.app_context.protocol().handler_count().await, 2);
    let status = handle.daemon_facade.query_status().await.unwrap();
    assert!(!status.propagation_enabled);
    assert!(!status.standard_lxmf_propagation_destination_registered);
    assert!(!status.standard_lxmf_propagation_active);
    let propagation = handle.daemon_facade.query_standard_propagation().await.unwrap();
    assert!(!propagation.registered);
    assert!(!propagation.active);
    assert!(propagation.policy.is_none());
    assert!(propagation.attempts.is_empty());

    tokio::time::timeout(std::time::Duration::from_secs(5), handle.shutdown())
        .await
        .expect("canonical daemon shutdown timed out");
}

#[tokio::test]
async fn canonical_nomadnet_handlers_are_active_before_announce() {
    let root = tempfile::tempdir().unwrap();
    let handle = daemon::start(DaemonConfig2 {
        db: Some(root.path().join("messages.db")),
        config: None,
        identity: None,
        socket: Some(root.path().join("daemon.sock")),
        ephemeral: true,
    })
    .await
    .unwrap();

    let components = handle.startup_contract().components();
    let handler = components
        .iter()
        .position(|component| *component == components::NATIVE_NOMADNET_REQUEST_HANDLER)
        .expect("native handler activation recorded");
    let announce = components
        .iter()
        .position(|component| *component == components::NOMADNET_NODE_ANNOUNCE)
        .expect("native announce recorded");
    assert!(handler < announce, "native handlers must activate before the announce");

    tokio::time::timeout(std::time::Duration::from_secs(5), handle.shutdown())
        .await
        .expect("canonical daemon shutdown timed out");
}
