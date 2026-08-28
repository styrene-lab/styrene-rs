use super::Args;
use super::announce_worker::spawn_announce_worker;
use super::bridge::{PeerCrypto, TransportBridge};
use super::receipt_worker::spawn_receipt_worker;
use rns_core::destination::{DestinationName, RequestAccess, SingleInputDestination};
use rns_core::hash::AddressHash;
use rns_core::transport::core_transport::{Transport, TransportConfig};
use rns_core::transport::iface::tcp_client::TcpClient;
use rns_core::transport::iface::tcp_server::TcpServer;
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use styrened::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use styrened::app_context::AppContext;
use styrened::config::DaemonConfig;
use styrened::daemon_facade::DaemonFacade;
use styrened::identity_store::load_or_create_identity;
use styrened::receipt_bridge::{
    CompositeReceiptHandler, PacketReceiptBridge, ReceiptBridge, ReceiptWaiters,
    ServiceReceiptBridge,
};
use styrened::rpc::{AnnounceBridge, InterfaceRecord, OutboundBridge, RpcDaemon};
use styrened::standard_propagation::{
    DEFAULT_PROPAGATION_NODE_NAME, StandardPropagationEndpoint, StandardPropagationRuntimePolicy,
};
use styrened::startup_contract::{
    ActiveCapabilities, RuntimeKind, StartupContract, StartupContractBuilder,
    capabilities as startup_capability, components as startup_component,
};
use styrened::storage::messages::MessagesStore;
use styrened::storage::standard_propagation::{
    StandardPropagationPolicy, StandardPropagationStats,
};
use styrened::transport::adapter::TokioTransportAdapter;
use styrened::transport::mesh_transport::MeshTransport;
use styrened::transport::null_transport::NullTransport;
use tokio::sync::mpsc::unbounded_channel;

#[derive(Clone, Debug)]
pub(super) struct RpcTlsConfig {
    pub(super) cert_chain_path: PathBuf,
    pub(super) private_key_path: PathBuf,
    pub(super) client_ca_path: Option<PathBuf>,
}

pub(super) struct BootstrapContext {
    pub(super) rpc_addr: SocketAddr,
    pub(super) daemon: Arc<RpcDaemon>,
    pub(super) rpc_tls: Option<RpcTlsConfig>,
    /// New service architecture — runs alongside RpcDaemon during migration.
    /// Will eventually replace RpcDaemon as the primary dispatch layer.
    #[allow(dead_code)]
    pub(super) app_context: Arc<AppContext>,
    #[allow(dead_code)]
    pub(super) daemon_facade: Arc<DaemonFacade>,
    #[allow(dead_code)]
    pub(super) startup_contract: StartupContract,
    #[allow(dead_code)]
    pub(super) standard_propagation: Option<StandardPropagationEndpoint>,
    workers: BootstrapWorkers,
    /// Unix socket IPC server — serves the Daemon trait to TUI and CLI clients.
    #[cfg(feature = "ipc-server")]
    #[allow(dead_code)]
    pub(super) ipc_server: styrene_ipc_server::IpcServer,
}

fn spawn_legacy_message_event_adapter(
    daemon: Arc<RpcDaemon>,
    app_context: Arc<AppContext>,
) -> tokio::task::JoinHandle<()> {
    let mut events = app_context.events().subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) if event.event_type == "message_received" => {
                    let Some(message_id) =
                        event.payload.get("id").and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    match app_context.messaging().get_message(message_id) {
                        Ok(Some(message)) => daemon.emit_event(styrened::rpc::RpcEvent {
                            event_type: "inbound".into(),
                            payload: serde_json::json!({ "message": message }),
                        }),
                        Ok(None) | Err(_) => daemon.emit_event(styrened::rpc::RpcEvent {
                            event_type: "reconciliation_required".into(),
                            payload: serde_json::json!({
                                "reason": "canonical inbound observation unavailable to legacy RPC adapter",
                                "message_id": message_id,
                            }),
                        }),
                    }
                }
                Ok(event)
                    if matches!(
                        event.event_type.as_str(),
                        "message_authentication_changed" | "inbound_dropped"
                    ) =>
                {
                    daemon.emit_event(event);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    daemon.emit_event(styrened::rpc::RpcEvent {
                        event_type: "reconciliation_required".into(),
                        payload: serde_json::json!({
                            "reason": "legacy RPC inbound observation adapter lagged",
                            "dropped": dropped,
                        }),
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

struct BootstrapWorkers {
    inbound: styrened::workers::inbound::InboundWorkerHandle,
    expiry: tokio::task::JoinHandle<()>,
    router: tokio::task::JoinHandle<()>,
    standard_propagation_sync:
        Option<styrened::workers::standard_propagation::StandardPropagationSyncWorker>,
    announce: tokio::task::JoinHandle<()>,
    link: tokio::task::JoinHandle<()>,
    route: tokio::task::JoinHandle<()>,
    legacy: Vec<tokio::task::JoinHandle<()>>,
}

impl BootstrapWorkers {
    fn abort(&self) {
        self.inbound.abort();
        self.expiry.abort();
        self.router.abort();
        if let Some(worker) = &self.standard_propagation_sync {
            worker.abort();
        }
        self.announce.abort();
        self.link.abort();
        self.route.abort();
        for worker in &self.legacy {
            worker.abort();
        }
    }

    async fn shutdown(&mut self) {
        if let Some(worker) = &mut self.standard_propagation_sync {
            worker.shutdown().await;
        }
        self.inbound.abort();
        self.expiry.abort();
        self.router.abort();
        self.announce.abort();
        self.link.abort();
        self.route.abort();
        for worker in &self.legacy {
            worker.abort();
        }
        self.inbound.wait().await;
        let _ = (&mut self.expiry).await;
        let _ = (&mut self.router).await;
        let _ = (&mut self.announce).await;
        let _ = (&mut self.link).await;
        let _ = (&mut self.route).await;
        for worker in &mut self.legacy {
            let _ = worker.await;
        }
    }
}

impl BootstrapContext {
    #[allow(dead_code)]
    pub(super) fn active_capabilities(&self, caller_identity: &str) -> ActiveCapabilities {
        self.startup_contract
            .active_capabilities(self.app_context.policy().authorized_capabilities(caller_identity))
    }

    pub(super) async fn shutdown(mut self) {
        if let Some(mut endpoint) = self.standard_propagation.take() {
            endpoint.shutdown().await;
            drop(endpoint);
        }
        self.workers.shutdown().await;
        let _ = self.app_context.transport().shutdown().await;
        #[cfg(feature = "ipc-server")]
        self.ipc_server.stop().await;
    }
}

impl Drop for BootstrapContext {
    fn drop(&mut self) {
        self.workers.abort();
    }
}

const PROPAGATION_CONTROL_ALLOW_LIST_ENV: &str = "STYRENE_PROPAGATION_CONTROL_ALLOWED_IDENTITIES";

pub(super) fn propagation_control_allow_list(
    local_identity: AddressHash,
    configured: Option<&str>,
) -> Result<BTreeSet<AddressHash>, String> {
    let mut allowed = BTreeSet::from([local_identity]);
    for value in configured.into_iter().flat_map(|value| value.split(',')) {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let bytes = hex::decode(value).map_err(|_| format!("invalid identity hash '{value}'"))?;
        let hash: [u8; 16] = bytes
            .try_into()
            .map_err(|_| format!("invalid identity hash '{value}' (expected 16-byte hex)"))?;
        allowed.insert(AddressHash::new(hash));
    }
    Ok(allowed)
}

pub(super) fn propagation_stats_response(
    identity_hash: &[u8],
    destination_hash: &[u8],
    policy: StandardPropagationRuntimePolicy,
    queue: &StandardPropagationStats,
    uptime: std::time::Duration,
) -> Vec<u8> {
    let mut map = vec![
        (rmpv::Value::from("identity_hash"), rmpv::Value::Binary(identity_hash.to_vec())),
        (rmpv::Value::from("destination_hash"), rmpv::Value::Binary(destination_hash.to_vec())),
        (rmpv::Value::from("uptime"), rmpv::Value::from(uptime.as_secs_f64())),
        (rmpv::Value::from("delivery_limit"), rmpv::Value::from(0)),
        (
            rmpv::Value::from("propagation_limit"),
            rmpv::Value::from(policy.transfer_limit_kb as u64),
        ),
        (rmpv::Value::from("sync_limit"), rmpv::Value::from(policy.sync_limit_kb as u64)),
        (rmpv::Value::from("target_stamp_cost"), rmpv::Value::from(policy.target_cost)),
        (rmpv::Value::from("stamp_cost_flexibility"), rmpv::Value::from(policy.flexibility)),
        (rmpv::Value::from("peering_cost"), rmpv::Value::from(policy.peering_cost)),
        (rmpv::Value::from("max_peering_cost"), rmpv::Value::from(0)),
        (rmpv::Value::from("autopeer_maxdepth"), rmpv::Value::from(0)),
        (rmpv::Value::from("from_static_only"), rmpv::Value::from(false)),
    ];
    for key in [
        "unpeered_propagation_incoming",
        "unpeered_propagation_rx_bytes",
        "static_peers",
        "discovered_peers",
        "total_peers",
        "max_peers",
    ] {
        map.push((rmpv::Value::from(key), rmpv::Value::from(0)));
    }
    map.push((
        rmpv::Value::from("messagestore"),
        rmpv::Value::Map(vec![
            (rmpv::Value::from("count"), rmpv::Value::from(queue.queued_count as u64)),
            (rmpv::Value::from("bytes"), rmpv::Value::from(queue.stored_bytes as u64)),
            (rmpv::Value::from("limit"), rmpv::Value::from(policy.queue_max_bytes as u64)),
        ]),
    ));
    map.push((
        rmpv::Value::from("clients"),
        rmpv::Value::Map(vec![
            (rmpv::Value::from("client_propagation_messages_received"), rmpv::Value::from(0)),
            (rmpv::Value::from("client_propagation_messages_served"), rmpv::Value::from(0)),
        ]),
    ));
    map.push((rmpv::Value::from("peers"), rmpv::Value::Map(Vec::new())));
    let mut response = Vec::new();
    rmpv::encode::write_value(&mut response, &rmpv::Value::Map(map))
        .expect("propagation stats encode");
    response
}

pub(super) async fn bootstrap(args: Args) -> BootstrapContext {
    bootstrap_with_transport_override(args, None).await
}

#[cfg(test)]
pub(super) async fn bootstrap_with_mesh_transport(
    args: Args,
    transport: Arc<dyn MeshTransport>,
) -> BootstrapContext {
    bootstrap_with_transport_override(args, Some(transport)).await
}

async fn bootstrap_with_transport_override(
    args: Args,
    mesh_transport_override: Option<Arc<dyn MeshTransport>>,
) -> BootstrapContext {
    let mut startup = StartupContractBuilder::production(RuntimeKind::Standalone);
    let mut legacy_workers = Vec::new();
    let rpc_addr: SocketAddr = args.rpc.parse().expect("invalid rpc address");
    let rpc_tls =
        match (args.rpc_tls_cert.clone(), args.rpc_tls_key.clone(), args.rpc_tls_client_ca.clone())
        {
            (None, None, None) => None,
            (Some(cert_chain_path), Some(private_key_path), client_ca_path) => {
                Some(RpcTlsConfig { cert_chain_path, private_key_path, client_ca_path })
            }
            (None, None, Some(_)) => {
                panic!("--rpc-tls-client-ca requires --rpc-tls-cert and --rpc-tls-key")
            }
            _ => panic!("--rpc-tls-cert and --rpc-tls-key must be provided together"),
        };
    let db_path = args.db.clone().unwrap_or_else(styrened::config::default_db_path);
    // Ensure data and config directories exist
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(styrened::config::default_config_dir()).ok();
    let store = MessagesStore::open(&db_path).expect("open sqlite");
    let standard_propagation_store = Arc::new(std::sync::Mutex::new(
        MessagesStore::open(&db_path).expect("open standard propagation sqlite"),
    ));

    let identity_path =
        args.identity.clone().unwrap_or_else(styrened::config::default_identity_path);
    let identity = load_or_create_identity(&identity_path).expect("load identity");
    let identity_hash = hex::encode(identity.address_hash().as_slice());
    let local_display_name =
        std::env::var("LXMF_DISPLAY_NAME").ok().and_then(|value| normalize_display_name(&value));
    // Try explicit --config, then default path
    let config_path = args.config.clone().or_else(|| {
        let default = styrened::config::default_config_path();
        if default.exists() { Some(default) } else { None }
    });
    let daemon_config = config_path.as_ref().and_then(|path| match DaemonConfig::from_path(path) {
        Ok(config) => Some(config),
        Err(err) => {
            eprintln!("[daemon] failed to load config {}: {}", path.display(), err);
            None
        }
    });
    let mut configured_interfaces = daemon_config
        .as_ref()
        .map(|config| {
            config
                .interfaces
                .iter()
                .map(|iface| InterfaceRecord {
                    kind: iface.kind.clone(),
                    enabled: iface.enabled.unwrap_or(false),
                    host: iface.host.clone(),
                    port: iface.port,
                    name: iface.name.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let node_role = daemon_config.as_ref().map(|c| c.role).unwrap_or_default();
    eprintln!("[daemon] node role: {}", node_role);

    let mut transport: Option<Arc<Transport>> = None;
    let peer_crypto: Arc<Mutex<HashMap<String, PeerCrypto>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut announce_destination: Option<Arc<tokio::sync::Mutex<SingleInputDestination>>> = None;
    let mut propagation_destination: Option<Arc<tokio::sync::Mutex<SingleInputDestination>>> = None;
    let mut nomadnet_destination: Option<Arc<tokio::sync::Mutex<SingleInputDestination>>> = None;
    let mut standard_propagation = None;
    let mut delivery_destination_hash_hex: Option<String> = None;
    let mut delivery_source_hash = [0u8; 16];
    let receipt_map: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let receipt_waiters: ReceiptWaiters = Arc::new(Mutex::new(HashMap::new()));
    let (receipt_tx, receipt_rx) = unbounded_channel();
    let mut service_receipt_target = None;
    let mut packet_receipt_sender = None;

    if let Some(addr) = args.transport.clone().filter(|_| node_role.runs_transport()) {
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
        let mut config = TransportConfig::new("daemon", &transport_identity, true);
        config.set_retransmit(true);
        let mut transport_instance = Transport::new(config);
        startup.record(startup_component::NATIVE_RESOURCE_RETRY_SCHEDULER);
        let service_target = Arc::new(std::sync::OnceLock::new());
        let packet_receipts = PacketReceiptBridge::new();
        transport_instance
            .set_receipt_handler(Box::new(CompositeReceiptHandler::new(vec![
                Box::new(ReceiptBridge::new(
                    receipt_map.clone(),
                    receipt_waiters.clone(),
                    receipt_tx.clone(),
                )),
                Box::new(ServiceReceiptBridge::new(service_target.clone())),
                Box::new(packet_receipts.clone()),
            ])))
            .await;
        service_receipt_target = Some(service_target);
        packet_receipt_sender = Some(packet_receipts.sender());
        startup.record(startup_component::LEGACY_RECEIPT_BRIDGE);
        startup.record(startup_component::SERVICE_RECEIPT_BRIDGE);
        let iface_manager = transport_instance.iface_manager();
        let (tcp_server, _bound_addr_rx) = TcpServer::new(addr.clone(), iface_manager.clone());
        let server_iface = iface_manager.lock().await.spawn(tcp_server, TcpServer::spawn);
        eprintln!("[daemon] tcp_server enabled iface={} bind={}", server_iface, addr);
        if let Some(config) = daemon_config.as_ref() {
            for (host, port) in config.tcp_client_endpoints() {
                let endpoint = format!("{}:{}", host, port);
                let client_iface =
                    iface_manager.lock().await.spawn(TcpClient::new(endpoint), TcpClient::spawn);
                eprintln!(
                    "[daemon] tcp_client enabled iface={} name={} host={} port={}",
                    client_iface, host, host, port
                );
            }
        }
        eprintln!("[daemon] transport enabled");
        if let Some((host, port)) = addr.rsplit_once(':') {
            configured_interfaces.push(InterfaceRecord {
                kind: "tcp_server".into(),
                enabled: true,
                host: Some(host.to_string()),
                port: port.parse::<u16>().ok(),
                name: Some("daemon-transport".into()),
            });
        }

        let destination = transport_instance
            .add_destination(transport_identity.clone(), DestinationName::new("lxmf", "delivery"))
            .await;
        {
            let dest = destination.lock().await;
            delivery_source_hash.copy_from_slice(dest.desc.address_hash.as_slice());
            delivery_destination_hash_hex = Some(hex::encode(dest.desc.address_hash.as_slice()));
            println!(
                "[daemon] delivery destination hash={}",
                hex::encode(dest.desc.address_hash.as_slice())
            );
        }
        announce_destination = Some(destination);
        startup.record(startup_component::LXMF_DELIVERY);
        let native_pages = transport_instance
            .add_destination(
                transport_identity.clone(),
                DestinationName::new("nomadnetwork", "node"),
            )
            .await;
        nomadnet_destination = Some(native_pages);
        startup.record(startup_component::NOMADNET_NODE_DESTINATION);
        if node_role == styrened::config::NodeRole::Hub {
            let propagation_name = std::env::var("STYRENE_PROPAGATION_NODE_NAME")
                .unwrap_or_else(|_| DEFAULT_PROPAGATION_NODE_NAME.to_string());
            let endpoint = StandardPropagationEndpoint::register(
                &mut transport_instance,
                transport_identity.clone(),
                &propagation_name,
                Arc::clone(&standard_propagation_store),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("standard propagation destination registration failed: {error:?}")
            });
            let propagation_hash = endpoint.destination().lock().await.desc.address_hash;
            let propagation_policy = endpoint.runtime_observation().policy();
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_DESTINATION);
            standard_propagation = Some(endpoint);
            let propagation = transport_instance
                .add_destination(
                    transport_identity.clone(),
                    DestinationName::new("lxmf", "propagation.control"),
                )
                .await;
            {
                let mut dest = propagation.lock().await;
                let control_allowed = propagation_control_allow_list(
                    *transport_identity.address_hash(),
                    std::env::var(PROPAGATION_CONTROL_ALLOW_LIST_ENV).ok().as_deref(),
                )
                .unwrap_or_else(|error| {
                    panic!("invalid {PROPAGATION_CONTROL_ALLOW_LIST_ENV}: {error}")
                });
                let stats_store = Arc::clone(&standard_propagation_store);
                let mut stats_identity_hash = [0u8; 16];
                stats_identity_hash.copy_from_slice(transport_identity.address_hash().as_slice());
                let stats_started = Instant::now();
                dest.register_request_path(
                    "/pn/get/stats",
                    RequestAccess::AllowList(control_allowed),
                    1024,
                    16 * 1024,
                    Arc::new(move |_, _, _, _| {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .and_then(|duration| i64::try_from(duration.as_secs()).ok())
                            .unwrap_or(0);
                        let queue = stats_store.lock().ok().and_then(|mut store| {
                            store
                                .standard_propagation_stats(
                                    now,
                                    StandardPropagationPolicy {
                                        queue_max_count: propagation_policy.queue_max_count,
                                        queue_max_bytes: propagation_policy.queue_max_bytes,
                                        expiry_secs: propagation_policy.expiry_secs,
                                    },
                                )
                                .ok()
                        });
                        queue.map_or_else(
                            || vec![0xc0],
                            |queue| {
                                propagation_stats_response(
                                    &stats_identity_hash,
                                    propagation_hash.as_slice(),
                                    propagation_policy,
                                    &queue,
                                    stats_started.elapsed(),
                                )
                            },
                        )
                    }),
                )
                .expect("propagation stats request registration");
                println!(
                    "[daemon] propagation control destination hash={}",
                    hex::encode(dest.desc.address_hash.as_slice())
                );
            }
            propagation_destination = Some(propagation);
            startup.record(startup_component::PARTIAL_PROPAGATION_STATS_DESTINATION);
        }
        transport = Some(Arc::new(transport_instance));
    }

    let bridge: Option<Arc<TransportBridge>> =
        transport.as_ref().zip(announce_destination.as_ref()).map(|(transport, destination)| {
            Arc::new(TransportBridge::new(
                transport.clone(),
                identity.clone(),
                delivery_source_hash,
                destination.clone(),
                local_display_name
                    .as_ref()
                    .and_then(|display_name| encode_delivery_display_name_app_data(display_name)),
                peer_crypto.clone(),
                receipt_map.clone(),
                receipt_waiters.clone(),
                receipt_tx.clone(),
            ))
        });

    let outbound_bridge: Option<Arc<dyn OutboundBridge>> =
        bridge.as_ref().map(|bridge| bridge.clone() as Arc<dyn OutboundBridge>);
    let announce_bridge: Option<Arc<dyn AnnounceBridge>> =
        bridge.as_ref().map(|bridge| bridge.clone() as Arc<dyn AnnounceBridge>);

    let daemon = Arc::new(RpcDaemon::with_compatibility_store_and_bridges(
        store,
        identity_hash,
        outbound_bridge,
        announce_bridge,
    ));
    let local_delivery_hash = delivery_destination_hash_hex.clone();
    daemon.set_delivery_destination_hash(delivery_destination_hash_hex.clone());
    daemon.replace_interfaces(configured_interfaces);
    daemon.set_propagation_state(
        node_role == styrened::config::NodeRole::Hub && transport.is_some(),
        None,
        0,
    );

    // Make the local delivery destination visible on startup.
    if let Some(bridge) = bridge.as_ref() {
        let _ = bridge.announce_now();
    }

    if transport.is_some() {
        legacy_workers.push(spawn_receipt_worker(daemon.clone(), receipt_rx));
        startup.record(startup_component::LEGACY_RECEIPT_WORKER);
    }

    if args.announce_interval_secs > 0 {
        legacy_workers.push(daemon.clone().start_announce_scheduler(args.announce_interval_secs));
        startup.record(startup_component::ANNOUNCE_SCHEDULER);
    }

    // Capture transport and announce destination for service architecture before
    // they're moved into workers.
    let transport_for_services = transport.clone();
    let announce_dest_for_services = announce_destination.clone();

    if propagation_destination.is_some() {
        startup.record(startup_component::PARTIAL_PROPAGATION_STATS_WORKER);
    }

    if let Some(transport) = transport {
        legacy_workers.push(spawn_announce_worker(daemon.clone(), transport, peer_crypto));
        startup.record(startup_component::LEGACY_ANNOUNCE_WORKER);
    }

    // --- Canonical service architecture ---
    // Wire TokioTransportAdapter when real transport exists, NullTransport otherwise.
    let mesh_transport: Arc<dyn MeshTransport> = if let Some(transport) = mesh_transport_override {
        transport
    } else if let (Some(tp), Some(ann_dest)) =
        (&transport_for_services, &announce_dest_for_services)
    {
        let mut id_hash = [0u8; 16];
        id_hash.copy_from_slice(identity.address_hash().as_slice());
        let adapter = TokioTransportAdapter::new_with_packet_receipts(
            tp.clone(),
            rns_core::hash::AddressHash::new(id_hash),
            rns_core::hash::AddressHash::new(delivery_source_hash),
            ann_dest.clone(),
            local_display_name
                .as_ref()
                .and_then(|name| encode_delivery_display_name_app_data(name)),
            packet_receipt_sender.clone().expect("native transport has packet receipt bridge"),
        )
        .await;
        eprintln!("[daemon] TokioTransportAdapter wired into service architecture");
        startup.record(startup_component::TRANSPORT_ANNOUNCE_BRIDGE);
        startup.record(startup_component::TRANSPORT_LINK_BRIDGE);
        Arc::new(adapter)
    } else {
        Arc::new(NullTransport::new())
    };
    // AppContext owns canonical conversation/message persistence. RpcDaemon's
    // separate connection above remains a legacy compatibility reader/adapter
    // over this database; it is not an independent inbound authority.
    let shared_store = Arc::new(std::sync::Mutex::new(
        MessagesStore::open(&db_path).expect("app_context shared store"),
    ));
    // Persistent node store — same directory as the message database
    let node_store_path = db_path.with_file_name("nodes.db");
    let node_store = Arc::new(
        styrene_services::node_store::NodeStore::open(
            node_store_path.to_str().expect("valid path"),
        )
        .expect("open node store"),
    );

    // --- RBAC policy: config → DB overlay → normalize ---
    let rbac_policy = {
        let mut policy = daemon_config.as_ref().and_then(|c| c.rbac.clone()).unwrap_or_default();

        // Overlay roster entries from SQLite (DB wins on conflict)
        {
            let store_guard = shared_store.lock().unwrap();
            if let Ok(db_entries) = store_guard.load_rbac_roster() {
                for entry in db_entries {
                    policy.add_entry(entry);
                }
            }
            // Merge blocked_peers table into policy
            if let Ok(blocked) = store_guard.blocked_peers() {
                for hash in blocked {
                    policy.block(&hash);
                }
            }
        }

        // Auto-roster the daemon's own identity as Admin so the local CLI
        // (which authenticates as the daemon) retains full administrative access.
        let own_hash = hex::encode(identity.address_hash().as_slice());
        if policy.get_entry(&own_hash).is_none() {
            policy.add_entry(
                styrene_rbac::RosterEntry::new(&own_hash, styrene_rbac::Role::Admin)
                    .with_label("local"),
            );
        }

        // Verify hub-signed entries against trusted hubs.
        // Entries with invalid signatures, unknown hubs, or expiry are dropped.
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let trusted = policy.trusted_hubs().to_vec();
            let hub_entries: Vec<_> = policy.hub_entries().to_vec();
            let total = hub_entries.len();
            policy.clear_hub_entries();
            // Re-add only verified entries
            for entry in hub_entries {
                if entry.is_expired(now) {
                    eprintln!(
                        "[daemon] rbac: dropping expired hub entry for {}",
                        entry.entry.identity_hash,
                    );
                } else if !trusted.iter().any(|h| h.matches(&entry)) {
                    eprintln!(
                        "[daemon] rbac: dropping hub entry for {} — hub {} not trusted",
                        entry.entry.identity_hash, entry.hub_hash,
                    );
                } else if !entry.verify() {
                    eprintln!(
                        "[daemon] rbac: dropping hub entry for {} — invalid signature",
                        entry.entry.identity_hash,
                    );
                } else {
                    policy.add_hub_entry(entry);
                }
            }
            if total > 0 {
                eprintln!(
                    "[daemon] rbac: {}/{} hub-signed entries verified ({} trusted hubs)",
                    policy.hub_entries().len(),
                    total,
                    trusted.len(),
                );
            }
        }

        let warnings = policy.normalize();
        for w in &warnings {
            eprintln!("[daemon] rbac: {w:?}");
        }
        eprintln!(
            "[daemon] RBAC policy loaded: {} roster entries, {} hub entries, {} blocked prefixes, default_role={:?}",
            policy.entries().len(),
            policy.hub_entries().len(),
            policy.blocked_count(),
            policy.default_role,
        );
        policy
    };

    let app_context = Arc::new(AppContext::with_policy(
        mesh_transport,
        hex::encode(identity.address_hash().as_slice()),
        shared_store,
        node_store,
        styrened::services::PolicyService::new(rbac_policy),
    ));
    if let Some(endpoint) = standard_propagation.as_ref() {
        app_context.publish_standard_propagation(endpoint.runtime_observation());
        endpoint.set_events(app_context.events_arc());
    } else if transport_for_services.is_some() && node_role != styrened::config::NodeRole::Hub {
        app_context.publish_standard_propagation(
            styrened::standard_propagation::StandardPropagationRuntimeObservation::client(),
        );
    }
    startup.record_local_execution_services();
    let daemon_facade = Arc::new(DaemonFacade::new(
        app_context.clone(),
        hex::encode(identity.address_hash().as_slice()),
    ));
    legacy_workers.push(spawn_legacy_message_event_adapter(daemon.clone(), app_context.clone()));
    startup.record(startup_component::LEGACY_MESSAGE_EVENT_ADAPTER);
    // Load config into ConfigService if a config file was provided
    if let Some(config_path) = config_path.as_ref()
        && let Err(e) = app_context.config().load(config_path)
    {
        eprintln!("[daemon] failed to load config into ConfigService: {}", e);
    }
    // Wire signing identity into services that need outbound delivery
    app_context.set_signer(Arc::new(identity.clone()));
    // Wire delivery destination hash into IdentityService so DaemonFacade can
    // return it in query_identity responses (needed for LXMF messaging).
    app_context.identity().set_delivery_destination_hash(local_delivery_hash.clone());
    eprintln!("[daemon] service architecture initialized (AppContext + DaemonFacade + signer)");

    // Enable propagation if node role is Hub
    if node_role == styrened::config::NodeRole::Hub {
        app_context.propagation().set_enabled(true);
        app_context.status().set_propagation_state(true, None, 0);
        eprintln!("[daemon] propagation store enabled (hub mode)");
    } else {
        app_context.status().set_propagation_state(false, None, 0);
    }

    // --- Service-layer workers (inbound + announce processing) ---
    let standard_propagation_hash = if let Some(endpoint) = standard_propagation.as_ref() {
        Some(hex::encode(endpoint.destination().lock().await.desc.address_hash.as_slice()))
    } else {
        None
    };
    if let (Some(endpoint), Some(transport)) =
        (standard_propagation.as_mut(), transport_for_services.as_ref())
    {
        endpoint
            .activate(app_context.transport_arc(), transport.as_ref())
            .await
            .unwrap_or_else(|error| panic!("standard propagation activation failed: {error:?}"));
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_OFFER_HANDLER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_GET_HANDLER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_ANNOUNCE);
        startup
            .advertise(startup_capability::STANDARD_LXMF_PROPAGATION)
            .unwrap_or_else(|error| panic!("invalid propagation startup evidence: {error}"));
    }
    let service_inbound = styrened::workers::inbound::spawn_inbound_worker_with_auto_reply(
        app_context.transport_arc(),
        app_context.messaging_arc(),
        app_context.protocol_arc(),
        app_context.events_arc(),
        app_context.propagation_arc(),
        styrened::workers::inbound::InboundDestinations::new(
            local_delivery_hash,
            standard_propagation_hash,
        ),
        Some(app_context.auto_reply_arc()),
    );
    startup.record(startup_component::INBOUND_PACKET_WORKER);
    startup.record(startup_component::INBOUND_RESOURCE_WORKER);
    startup.record(startup_component::OUTBOUND_RESOURCE_COMPLETION_WORKER);

    if let Some(target) = service_receipt_target
        && target.set(Arc::downgrade(&app_context.messaging_arc())).is_err()
    {
        panic!("standalone service receipt target initialized twice");
    }

    // Spawn propagation expiry cleanup task
    let expiry_worker =
        styrened::services::propagation::spawn_expiry_task(app_context.propagation_arc());
    startup.record(startup_component::PROPAGATION_EXPIRY_SCHEDULER);
    let router_worker =
        styrened::workers::router::spawn_router_deadline_worker(app_context.messaging_arc());
    startup.record(startup_component::LXMF_ROUTER_DEADLINE_SCHEDULER);
    let standard_propagation_sync = (transport_for_services.is_some()
        && node_role != styrened::config::NodeRole::Hub)
        .then(|| {
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR);
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER);
            styrened::workers::standard_propagation::spawn_standard_propagation_sync_worker(
                app_context.messaging_arc(),
            )
        });
    let service_announce = styrened::workers::announce::spawn_announce_worker(
        app_context.transport_arc(),
        app_context.discovery_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ANNOUNCE_WORKER);
    let service_link = styrened::workers::link::spawn_link_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::LINK_WORKER);
    let service_route = styrened::workers::route::spawn_route_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ROUTE_WORKER);
    startup.record(startup_component::NETWORK_OPERATION_COORDINATOR);
    styrened::workers::register_styrene_rpc_handlers(
        &app_context,
        std::sync::Arc::new(identity.clone()),
    )
    .await;
    startup.record(startup_component::RPC_RESPONSE_HANDLER);
    startup.record(startup_component::RPC_REQUEST_HANDLER);
    if let (Some(transport), Some(destination)) =
        (transport_for_services.clone(), nomadnet_destination)
    {
        styrened::workers::native_nomadnet::register_handlers(
            destination.clone(),
            app_context.pages_arc(),
        )
        .await
        .unwrap_or_else(|error| panic!("native NomadNet path registration failed: {error:?}"));
        startup.record(startup_component::NATIVE_NOMADNET_REQUEST_HANDLER);
        transport
            .send_announce(&destination, local_display_name.as_deref().map(str::as_bytes))
            .await;
        startup.record(startup_component::NOMADNET_NODE_ANNOUNCE);
        startup.advertise(startup_capability::NATIVE_NOMADNET_HOST).unwrap_or_else(|error| {
            panic!("invalid standalone NomadNet startup contract: {error}")
        });
    }
    // Register tunnel protocol handler
    app_context.protocol().register(app_context.tunnel_arc()).await;
    startup.record(startup_component::TUNNEL_HANDLER);

    // Register I2P proxy protocol handler (when feature is enabled)
    #[cfg(feature = "i2p-proxy")]
    {
        app_context.protocol().register(app_context.i2p_proxy_arc()).await;
        startup.record(startup_component::I2P_PROXY_HANDLER);
        eprintln!("[daemon] I2P proxy service registered");
    }

    // Wire WireGuard backend into TunnelService on Linux when the feature is enabled.
    #[cfg(all(target_os = "linux", feature = "wireguard"))]
    {
        use styrene_tunnel::TunnelBackend;
        use styrene_tunnel::wireguard::WireGuardBackend;

        // Derive a WireGuard-specific private key from the RNS identity via HKDF.
        // This ensures a stable WG key tied to the node identity without storing
        // a separate key file.
        let wg_privkey = {
            use hkdf::Hkdf;
            use sha2::Sha256;
            let identity_privkey = identity.to_private_key_bytes();
            let hk = Hkdf::<Sha256>::new(Some(b"styrene-wg-key-v1"), &identity_privkey);
            let mut okm = [0u8; 32];
            hk.expand(b"wireguard", &mut okm).expect("HKDF expand");
            okm
        };

        let wg_backend = Arc::new(WireGuardBackend::new());
        wg_backend.set_private_key(&wg_privkey);
        if wg_backend.is_available().await {
            app_context.tunnel().set_backend(wg_backend.clone());
            eprintln!("[daemon] WireGuard backend wired into TunnelService");
        } else {
            eprintln!(
                "[daemon] WireGuard tools not available — tunnel state tracked without backend"
            );
        }
    }

    eprintln!(
        "[daemon] service workers started (inbound + announce + rpc-request + rpc-response + tunnel)"
    );

    startup.advertise(startup_capability::LOCAL_CONFIG).unwrap_or_else(|error| {
        panic!("invalid standalone local-config startup contract: {error}")
    });
    startup.advertise(startup_capability::LOCAL_POLICY).unwrap_or_else(|error| {
        panic!("invalid standalone local-policy startup contract: {error}")
    });
    if transport_for_services.is_some() {
        startup.record_transport_state_services();
        for capability in [
            startup_capability::LXMF_DIRECT,
            startup_capability::LXMF_PAPER_EXPORT,
            startup_capability::NETWORK_OPERATIONS,
            startup_capability::RNS_REQUESTS,
            startup_capability::RNS_REQUEST_CANCELLATION,
            startup_capability::RNS_RESOURCE_CANCELLATION,
            startup_capability::STYRENE_RPC,
            startup_capability::LEGACY_RPC_RECEIPTS,
        ] {
            if let Err(error) = startup.advertise(capability) {
                panic!("invalid standalone startup contract: {error}");
            }
        }
        if node_role != styrened::config::NodeRole::Hub {
            startup.advertise(startup_capability::STANDARD_LXMF_PROPAGATION_CLIENT).unwrap_or_else(
                |error| panic!("invalid standalone propagation-client startup contract: {error}"),
            );
        }
    }
    app_context.publish_startup_contract(startup.clone().finish());

    // --- Unix socket IPC server (desktop only) ---
    #[cfg(feature = "ipc-server")]
    let ipc_server = {
        let ipc_config = styrene_ipc_server::IpcServerConfig {
            socket_path: args
                .socket
                .clone()
                .unwrap_or_else(styrene_ipc_server::default_socket_path),
            event_capacity: 256,
        };
        let mut server = styrene_ipc_server::IpcServer::new(
            daemon_facade.clone() as Arc<dyn styrene_ipc::traits::Daemon>,
            ipc_config,
        );
        let started = match server.start().await {
            Ok(()) => {
                eprintln!("[daemon] IPC server listening on {}", server.socket_path().display());
                true
            }
            Err(e) => {
                eprintln!("[daemon] IPC server failed to start: {e}");
                false
            }
        };

        // Bridge daemon events → IPC server so clients receive pushed events.
        {
            let event_tx = server.event_sender();
            let mut daemon_rx = app_context.events().subscribe_daemon_events();
            tokio::spawn(async move {
                loop {
                    match daemon_rx.recv().await {
                        Ok(event) => {
                            let _ = event_tx.send(event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
        if started {
            startup.record(startup_component::IPC_EVENT_BRIDGE);
        }
        server
    };

    let startup_contract = startup.finish();
    app_context.publish_startup_contract(startup_contract.clone());

    BootstrapContext {
        rpc_addr,
        daemon,
        rpc_tls,
        app_context,
        daemon_facade,
        startup_contract,
        standard_propagation,
        workers: BootstrapWorkers {
            inbound: service_inbound,
            expiry: expiry_worker,
            router: router_worker,
            standard_propagation_sync,
            announce: service_announce,
            link: service_link,
            route: service_route,
            legacy: legacy_workers,
        },
        #[cfg(feature = "ipc-server")]
        ipc_server,
    }
}
