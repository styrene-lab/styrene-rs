use crate::bridge_helpers::opportunistic_payload;
use rns_core::destination_hash::parse_destination_hash_required;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::packet::PacketDataBuffer;
use rns_core::transport::core_transport::SendPacketOutcome;
use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};
use rns_core::transport::delivery::send_outcome_status;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use styrene_ipc::traits::DaemonStatus;
use styrened::rpc::{RpcRequest, RpcResponse};
use styrened::startup_contract::{RuntimeKind, capabilities, components};
use styrened::transport::mock_transport::MockTransport;

#[test]
fn opportunistic_payload_strips_destination_prefix() {
    let destination = [0xAA; 16];
    let mut payload = destination.to_vec();
    payload.extend_from_slice(&[1, 2, 3, 4]);
    assert_eq!(opportunistic_payload(&payload, &destination), &[1, 2, 3, 4]);
}

#[test]
fn opportunistic_payload_keeps_payload_without_prefix() {
    let destination = [0xAA; 16];
    let payload = vec![0xBB; 24];
    assert_eq!(opportunistic_payload(&payload, &destination), payload.as_slice());
}

#[test]
fn send_outcome_status_maps_success() {
    assert_eq!(
        send_outcome_status("opportunistic", SendPacketOutcome::SentDirect),
        "sent: opportunistic"
    );
}

#[test]
fn send_outcome_status_maps_failures() {
    assert_eq!(
        send_outcome_status("opportunistic", SendPacketOutcome::DroppedMissingDestinationIdentity),
        "failed: opportunistic missing destination identity"
    );
    assert_eq!(
        send_outcome_status("opportunistic", SendPacketOutcome::DroppedNoRoute),
        "failed: opportunistic no route"
    );
}

#[test]
fn parse_destination_hex_required_rejects_invalid_hashes() {
    let err = parse_destination_hash_required("not-hex").expect_err("invalid hash");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn propagation_control_allow_list_defaults_to_local_and_parses_configured_identities() {
    let local = rns_core::hash::AddressHash::new([0x11; 16]);
    let configured = format!("{}, {}", hex::encode([0x22; 16]), hex::encode([0x33; 16]));
    let allowed = crate::bootstrap::propagation_control_allow_list(local, Some(&configured))
        .expect("valid control allow-list");

    assert_eq!(allowed.len(), 3);
    assert!(allowed.contains(&local));
    assert!(allowed.contains(&rns_core::hash::AddressHash::new([0x22; 16])));
    assert!(allowed.contains(&rns_core::hash::AddressHash::new([0x33; 16])));
}

#[test]
fn propagation_control_allow_list_rejects_malformed_or_wrong_length_hashes() {
    let local = rns_core::hash::AddressHash::new([0x11; 16]);
    assert!(crate::bootstrap::propagation_control_allow_list(local, Some("not-hex")).is_err());
    assert!(crate::bootstrap::propagation_control_allow_list(local, Some("0011")).is_err());
}

#[test]
fn propagation_stats_encode_python_status_schema_with_runtime_values() {
    let encoded = crate::bootstrap::propagation_stats_response(
        &[0x11; 16],
        &[0x22; 16],
        styrened::standard_propagation::StandardPropagationRuntimePolicy {
            target_cost: 16,
            flexibility: 3,
            peering_cost: 18,
            transfer_limit_kb: 256,
            sync_limit_kb: 4000,
            queue_max_count: 4096,
            queue_max_bytes: 16 * 1024 * 1024,
            expiry_secs: 30 * 24 * 60 * 60,
            throttle_secs: 180,
            max_offer_links: 3,
        },
        &styrened::storage::standard_propagation::StandardPropagationStats {
            queued_count: 7,
            stored_bytes: 8192,
        },
        Duration::from_millis(1250),
    );
    let decoded = rmpv::decode::read_value(&mut encoded.as_slice()).expect("MessagePack stats");
    let stats = decoded.as_map().expect("stats map");
    let value = |key: &str| {
        stats
            .iter()
            .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
            .unwrap_or_else(|| panic!("missing stats key {key}"))
    };

    assert_eq!(value("identity_hash").as_slice(), Some([0x11; 16].as_slice()));
    assert_eq!(value("destination_hash").as_slice(), Some([0x22; 16].as_slice()));
    assert_eq!(value("uptime").as_f64(), Some(1.25));
    assert_eq!(value("propagation_limit").as_u64(), Some(256));
    assert_eq!(value("sync_limit").as_u64(), Some(4000));
    assert_eq!(value("target_stamp_cost").as_u64(), Some(16));
    assert_eq!(value("stamp_cost_flexibility").as_u64(), Some(3));
    assert_eq!(value("peering_cost").as_u64(), Some(18));
    let message_store = value("messagestore").as_map().expect("messagestore map");
    assert!(
        message_store
            .iter()
            .any(|(key, value)| { key.as_str() == Some("count") && value.as_u64() == Some(7) })
    );
    assert!(
        message_store
            .iter()
            .any(|(key, value)| { key.as_str() == Some("bytes") && value.as_u64() == Some(8192) })
    );
    assert!(message_store.iter().any(|(key, value)| {
        key.as_str() == Some("limit") && value.as_u64() == Some(16 * 1024 * 1024)
    }));
    assert!(value("clients").is_map());
    assert!(value("peers").is_map());
}

#[tokio::test]
async fn standalone_runtime_advertises_only_composed_capabilities() {
    let root = tempfile::tempdir().unwrap();
    #[allow(unused_mut)]
    let mut context = crate::bootstrap::bootstrap(crate::Args {
        rpc: "127.0.0.1:0".into(),
        db: Some(root.path().join("messages.db")),
        config: None,
        identity: Some(root.path().join("identity")),
        announce_interval_secs: 0,
        transport: Some("127.0.0.1:0".into()),
        rpc_tls_cert: None,
        rpc_tls_key: None,
        rpc_tls_client_ca: None,
        socket: Some(root.path().join("daemon.sock")),
    })
    .await
    .expect("bootstrap");

    let contract = &context.startup_contract;
    assert_eq!(contract.runtime(), RuntimeKind::Standalone);
    assert!(contract.has_component(components::LXMF_DELIVERY));
    assert!(!contract.has_component(components::PARTIAL_PROPAGATION_STATS_DESTINATION));
    assert!(contract.has_component(components::LEGACY_RECEIPT_BRIDGE));
    assert!(contract.has_component(components::SERVICE_RECEIPT_BRIDGE));
    assert_eq!(components::SERVICE_RECEIPT_BRIDGE.id, "service-rns-delivery-receipts");
    assert!(contract.has_component(components::OUTBOUND_RESOURCE_COMPLETION_WORKER));
    assert!(contract.has_component(components::ROUTE_WORKER));
    assert!(contract.has_component(components::NATIVE_RESOURCE_RETRY_SCHEDULER));
    assert!(contract.has_component(components::LXMF_ROUTER_DEADLINE_SCHEDULER));
    assert!(!contract.has_component(components::PARTIAL_PROPAGATION_STATS_WORKER));
    assert!(contract.has_component(components::LEGACY_MESSAGE_EVENT_ADAPTER));
    assert!(!contract.has_component(components::LEGACY_INBOUND_WORKER));
    assert!(contract.has_component(components::TUNNEL_HANDLER));
    assert!(contract.has_component(components::NOMADNET_NODE_DESTINATION));
    assert!(contract.has_component(components::NOMADNET_NODE_ANNOUNCE));
    assert!(contract.has_component(components::NATIVE_NOMADNET_REQUEST_HANDLER));
    assert!(contract.advertises(capabilities::LXMF_DIRECT.id()));
    assert!(contract.advertises(capabilities::STYRENE_RPC.id()));
    assert!(contract.advertises(capabilities::LOCAL_CONFIG.id()));
    assert!(contract.advertises(capabilities::LOCAL_POLICY.id()));
    assert!(contract.advertises(capabilities::RNS_REQUESTS.id()));
    assert!(contract.advertises(capabilities::RNS_REQUEST_CANCELLATION.id()));
    assert!(contract.advertises(capabilities::RNS_RESOURCE_CANCELLATION.id()));
    assert!(contract.advertises(capabilities::LEGACY_RPC_RECEIPTS.id()));
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(!contract.has_component(components::STANDARD_LXMF_PROPAGATION_DESTINATION));
    assert!(contract.advertises(capabilities::NATIVE_NOMADNET_HOST.id()));
    let expected_handlers = if cfg!(feature = "i2p-proxy") { 4 } else { 3 };
    assert_eq!(context.app_context.protocol().handler_count().await, expected_handlers);
    let active = context.active_capabilities("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(active.runtime().contains(&capabilities::LXMF_DIRECT.id()));
    let status = context.daemon_facade.query_status().await.unwrap();
    assert!(!status.propagation_enabled);
    assert!(!status.standard_lxmf_propagation_destination_registered);
    assert!(!status.standard_lxmf_propagation_active);

    tokio::time::timeout(std::time::Duration::from_secs(5), context.shutdown())
        .await
        .expect("standalone daemon shutdown timed out");
}

#[tokio::test]
async fn standalone_hub_activates_complete_standard_propagation_composition() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    std::fs::write(&config, "role = \"hub\"\n").unwrap();
    let context = crate::bootstrap::bootstrap(crate::Args {
        rpc: "127.0.0.1:0".into(),
        db: Some(root.path().join("messages.db")),
        config: Some(config),
        identity: Some(root.path().join("identity")),
        announce_interval_secs: 0,
        transport: Some("127.0.0.1:0".into()),
        rpc_tls_cert: None,
        rpc_tls_key: None,
        rpc_tls_client_ca: None,
        socket: Some(root.path().join("daemon.sock")),
    })
    .await
    .expect("bootstrap");
    assert!(
        context.startup_contract.has_component(components::STANDARD_LXMF_PROPAGATION_DESTINATION)
    );
    assert!(
        context.startup_contract.has_component(components::PARTIAL_PROPAGATION_STATS_DESTINATION)
    );
    assert!(context.startup_contract.has_component(components::PARTIAL_PROPAGATION_STATS_WORKER));
    assert!(context.startup_contract.has_component(components::STANDARD_LXMF_PROPAGATION_ANNOUNCE));
    assert!(
        context.startup_contract.has_component(components::STANDARD_LXMF_PROPAGATION_OFFER_HANDLER)
    );
    assert!(
        context.startup_contract.has_component(components::STANDARD_LXMF_PROPAGATION_GET_HANDLER)
    );
    assert!(
        context
            .startup_contract
            .has_component(components::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER)
    );
    assert!(context.startup_contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(context.standard_propagation.is_some());
    let status = context.daemon_facade.query_status().await.unwrap();
    assert!(status.propagation_enabled);
    assert!(status.standard_lxmf_propagation_destination_registered);
    assert!(status.standard_lxmf_propagation_active);
    context.shutdown().await;
}

#[tokio::test]
async fn standalone_hub_restart_keeps_standard_destination_hash_and_drops_old_registry() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    std::fs::write(&config, "role = \"hub\"\n").unwrap();
    let db_path = root.path().join("messages.db");
    let mut data = vec![0x31; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
    data[..16].copy_from_slice(&[0x32; 16]);
    let item = styrened::storage::standard_propagation::StandardPropagationItem {
        transient_id: Sha256::digest(&data).into(),
        destination: [0x32; 16],
        stored_size: data.len() + 32,
        lxmf_data: data,
        stamp: [0x33; 32],
        stamp_value: 0,
        received_at: 1,
        expires_at: i64::MAX,
    };
    styrened::storage::messages::MessagesStore::open(&db_path)
        .unwrap()
        .standard_propagation_ingest_batch(
            styrened::storage::standard_propagation::StandardPropagationIngestRequest {
                items: &[item],
                source_peer: None,
                attempt: styrened::storage::standard_propagation::StandardPropagationAttemptStatus::Untracked,
                protocol: styrened::storage::standard_propagation::StandardPropagationProtocolStatus::Valid,
                now: 1,
                policy: styrened::storage::standard_propagation::StandardPropagationPolicy {
                    queue_max_count: 4096,
                    queue_max_bytes: 16 * 1024 * 1024,
                    expiry_secs: 30 * 24 * 60 * 60,
                },
            },
        )
        .unwrap();
    let args = || crate::Args {
        rpc: "127.0.0.1:0".into(),
        db: Some(db_path.clone()),
        config: Some(config.clone()),
        identity: Some(root.path().join("identity")),
        announce_interval_secs: 0,
        transport: Some("127.0.0.1:0".into()),
        rpc_tls_cert: None,
        rpc_tls_key: None,
        rpc_tls_client_ca: None,
        socket: Some(root.path().join("daemon.sock")),
    };
    let first = crate::bootstrap::bootstrap(args()).await.expect("first bootstrap");
    assert_eq!(
        first.standard_propagation.as_ref().unwrap().queue_stats(2).unwrap().queued_count,
        1
    );
    let first_destination = first.standard_propagation.as_ref().unwrap().destination().clone();
    let first_hash = first_destination.lock().await.desc.address_hash;
    let old_destination = Arc::downgrade(&first_destination);
    drop(first_destination);
    first.shutdown().await;
    assert!(old_destination.upgrade().is_none());

    let second = crate::bootstrap::bootstrap(args()).await.expect("second bootstrap");
    assert_eq!(
        second.standard_propagation.as_ref().unwrap().queue_stats(3).unwrap().queued_count,
        1
    );
    let second_hash =
        second.standard_propagation.as_ref().unwrap().destination().lock().await.desc.address_hash;
    assert_eq!(first_hash, second_hash);
    second.shutdown().await;
}

#[tokio::test]
async fn standalone_without_transport_advertises_no_transport_capabilities() {
    let root = tempfile::tempdir().unwrap();
    #[allow(unused_mut)]
    let mut context = crate::bootstrap::bootstrap(crate::Args {
        rpc: "127.0.0.1:0".into(),
        db: Some(root.path().join("messages.db")),
        config: None,
        identity: Some(root.path().join("identity")),
        announce_interval_secs: 0,
        transport: None,
        rpc_tls_cert: None,
        rpc_tls_key: None,
        rpc_tls_client_ca: None,
        socket: Some(root.path().join("daemon.sock")),
    })
    .await
    .expect("bootstrap");

    let contract = &context.startup_contract;
    assert!(!contract.has_component(components::LXMF_DELIVERY));
    assert!(!contract.has_component(components::LEGACY_RECEIPT_BRIDGE));
    assert!(!contract.has_component(components::SERVICE_RECEIPT_BRIDGE));
    assert!(!contract.has_component(components::NATIVE_RESOURCE_RETRY_SCHEDULER));
    assert!(contract.advertises(capabilities::LOCAL_CONFIG.id()));
    assert!(contract.advertises(capabilities::LOCAL_POLICY.id()));
    assert!(!contract.advertises(capabilities::NETWORK_OPERATIONS.id()));
    assert!(!contract.advertises(capabilities::RNS_REQUESTS.id()));
    assert!(!contract.advertises(capabilities::STYRENE_RPC.id()));
    let expected_handlers = if cfg!(feature = "i2p-proxy") { 4 } else { 3 };
    assert_eq!(context.app_context.protocol().handler_count().await, expected_handlers);

    tokio::time::timeout(std::time::Duration::from_secs(5), context.shutdown())
        .await
        .expect("standalone daemon shutdown timed out");
}

#[tokio::test]
async fn production_bootstrap_has_one_canonical_inbound_persistence_owner() {
    let root = tempfile::tempdir().unwrap();
    let destination = [0x61; 16];
    let sender = PrivateIdentity::new_from_name("bootstrap-inbound-owner");
    let mut source = [0u8; 16];
    source.copy_from_slice(sender.address_hash().as_slice());
    let wire = styrened::lxmf_bridge::build_wire_message(
        source,
        destination,
        "ownership",
        "persist exactly once",
        None,
        &sender,
    )
    .expect("valid LXMF wire");
    let transport = std::sync::Arc::new(MockTransport::new(
        AddressHash::new(source),
        AddressHash::new(destination),
    ));
    transport.queue_resolve(Some(*sender.as_identity()));

    let context = crate::bootstrap::bootstrap_with_mesh_transport(
        crate::Args {
            rpc: "127.0.0.1:0".into(),
            db: Some(root.path().join("messages.db")),
            config: None,
            identity: Some(root.path().join("identity")),
            announce_interval_secs: 0,
            transport: None,
            rpc_tls_cert: None,
            rpc_tls_key: None,
            rpc_tls_client_ca: None,
            socket: Some(root.path().join("daemon.sock")),
        },
        transport.clone(),
    )
    .await
    .expect("bootstrap");
    assert!(context.startup_contract.has_component(components::INBOUND_PACKET_WORKER));
    assert!(context.startup_contract.has_component(components::LEGACY_MESSAGE_EVENT_ADAPTER));
    assert!(!context.startup_contract.has_component(components::LEGACY_INBOUND_WORKER));

    let mut legacy_events = context.daemon.subscribe_events();
    transport.inject_inbound(ReceivedData {
        destination: AddressHash::new(destination),
        link_id: None,
        data: PacketDataBuffer::new_from_slice(&wire),
        payload_mode: ReceivedPayloadMode::FullWire,
        ratchet_used: false,
        context: None,
        request_id: None,
        hops: None,
        interface: None,
        packet_hash: None,
        receiving_iface: None,
    });

    let legacy_observation = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = legacy_events.recv().await.expect("legacy event channel");
            if event.event_type == "inbound" {
                break event;
            }
        }
    })
    .await
    .expect("legacy inbound observation timeout");
    let message_id =
        legacy_observation.payload["message"]["id"].as_str().expect("observed message id");

    let messages = context.app_context.messaging().list_messages(10, None).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message_id);
    let canonical = context
        .app_context
        .messaging()
        .canonical_inbound(message_id)
        .unwrap()
        .expect("canonical authentication record");
    assert_eq!(canonical.authentication_state, "verified");

    let compatibility_observation = context
        .daemon
        .handle_rpc(RpcRequest {
            id: 2,
            method: "receive_message".into(),
            params: Some(serde_json::json!({
                "id": "legacy-must-not-persist",
                "source": hex::encode(source),
                "destination": hex::encode(destination),
                "title": "observation only",
                "content": "not canonical",
            })),
        })
        .unwrap();
    assert!(compatibility_observation.error.is_none());
    assert_eq!(context.app_context.messaging().list_messages(10, None).unwrap().len(), 1);

    let response: RpcResponse = context
        .daemon
        .handle_rpc(RpcRequest { id: 1, method: "list_messages".into(), params: None })
        .unwrap();
    assert_eq!(response.result.unwrap()["messages"].as_array().unwrap().len(), 1);

    tokio::time::timeout(std::time::Duration::from_secs(5), context.shutdown())
        .await
        .expect("standalone daemon shutdown timed out");
}
