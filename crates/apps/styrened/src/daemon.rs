//! Daemon entry point for the unified `styrene` binary.
//!
//! Clean boot path using only the new service architecture (AppContext +
//! DaemonFacade + IPC server). Does NOT start the legacy RPC server.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use crate::app_context::AppContext;
use crate::config::DaemonConfig;
use crate::daemon_facade::DaemonFacade;
use crate::identity_store::load_or_create_identity;
use crate::standard_propagation::{DEFAULT_PROPAGATION_NODE_NAME, StandardPropagationEndpoint};
use crate::startup_contract::{
    ActiveCapabilities, RuntimeKind, StartupContract, StartupContractBuilder,
    capabilities as startup_capability, components as startup_component,
};
use crate::storage::messages::MessagesStore;
use crate::transport::adapter::TokioTransportAdapter;
use crate::transport::mesh_transport::MeshTransport;
use crate::transport::null_transport::NullTransport;
use rns_core::destination::DestinationName;
use rns_core::transport::core_transport::{Transport, TransportConfig};
#[cfg(feature = "native-serial")]
use rns_core::transport::iface::rnode::RNodeInterface;
use rns_core::transport::iface::tcp_client::TcpClient;
use rns_core::transport::iface::tcp_server::TcpServer;

/// Configuration for the daemon entry point.
pub struct DaemonConfig2 {
    /// Database path (default: ~/.local/share/styrene/messages.db)
    pub db: Option<PathBuf>,
    /// Config file path
    pub config: Option<PathBuf>,
    /// Identity file path
    pub identity: Option<PathBuf>,
    /// Unix socket path for IPC server
    pub socket: Option<PathBuf>,
    /// Use ephemeral in-memory identity (no persistence)
    pub ephemeral: bool,
}

/// Handle to a running daemon.
pub struct DaemonHandle {
    pub app_context: Arc<AppContext>,
    pub daemon_facade: Arc<DaemonFacade>,
    startup_contract: StartupContract,
    standard_propagation: Option<StandardPropagationEndpoint>,
    #[cfg(feature = "ipc-server")]
    ipc_server: styrene_ipc_server::IpcServer,
    workers: DaemonWorkers,
}

struct DaemonWorkers {
    inbound: crate::workers::inbound::InboundWorkerHandle,
    announce: tokio::task::JoinHandle<()>,
    link: tokio::task::JoinHandle<()>,
    route: tokio::task::JoinHandle<()>,
    router_deadlines: tokio::task::JoinHandle<()>,
    standard_propagation_sync:
        Option<crate::workers::standard_propagation::StandardPropagationSyncWorker>,
    expiry: tokio::task::JoinHandle<()>,
    #[cfg(feature = "ipc-server")]
    ipc_events: tokio::task::JoinHandle<()>,
}

impl DaemonWorkers {
    fn abort(&self) {
        self.inbound.abort();
        self.announce.abort();
        self.link.abort();
        self.route.abort();
        self.router_deadlines.abort();
        if let Some(worker) = &self.standard_propagation_sync {
            worker.abort();
        }
        self.expiry.abort();
        #[cfg(feature = "ipc-server")]
        self.ipc_events.abort();
    }

    async fn shutdown(&mut self) {
        if let Some(worker) = &mut self.standard_propagation_sync {
            worker.shutdown().await;
        }
        self.inbound.abort();
        self.announce.abort();
        self.link.abort();
        self.route.abort();
        self.router_deadlines.abort();
        self.expiry.abort();
        #[cfg(feature = "ipc-server")]
        self.ipc_events.abort();
        self.inbound.wait().await;
        let _ = (&mut self.announce).await;
        let _ = (&mut self.link).await;
        let _ = (&mut self.route).await;
        let _ = (&mut self.router_deadlines).await;
        let _ = (&mut self.expiry).await;
        #[cfg(feature = "ipc-server")]
        let _ = (&mut self.ipc_events).await;
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        self.workers.abort();
    }
}

impl DaemonHandle {
    pub fn startup_contract(&self) -> &StartupContract {
        &self.startup_contract
    }

    pub fn active_capabilities(&self, caller_identity: &str) -> ActiveCapabilities {
        self.startup_contract
            .active_capabilities(self.app_context.policy().authorized_capabilities(caller_identity))
    }

    pub async fn standard_propagation_destination_hash(&self) -> Option<String> {
        let endpoint = self.standard_propagation.as_ref()?;
        Some(hex::encode(endpoint.destination().lock().await.desc.address_hash.as_slice()))
    }

    pub fn standard_propagation_destination_weak(
        &self,
    ) -> Option<std::sync::Weak<tokio::sync::Mutex<rns_core::destination::SingleInputDestination>>>
    {
        self.standard_propagation.as_ref().map(|endpoint| Arc::downgrade(endpoint.destination()))
    }

    /// Stop background work and await IPC socket cleanup.
    #[allow(unused_mut)]
    pub async fn shutdown(mut self) {
        if let Some(mut endpoint) = self.standard_propagation.take() {
            endpoint.shutdown().await;
            drop(endpoint);
        }
        self.workers.shutdown().await;
        if let Err(error) = self.app_context.transport().shutdown().await {
            crate::daemon_diagnostic!("[styrene] transport shutdown error: {error}");
        }
        #[cfg(feature = "ipc-server")]
        self.ipc_server.stop().await;
    }
}

/// Start the daemon with the given configuration.
///
/// Returns a handle that keeps the daemon alive. The daemon runs
/// until the handle is dropped or the process is interrupted.
pub async fn start(cfg: DaemonConfig2) -> anyhow::Result<DaemonHandle> {
    let mut startup = StartupContractBuilder::production(RuntimeKind::Canonical);
    // --- Identity ---
    let identity = if cfg.ephemeral {
        rns_core::identity::PrivateIdentity::new_from_rand(rand_core::OsRng)
    } else {
        let identity_path = cfg.identity.unwrap_or_else(crate::config::default_identity_path);
        if let Some(parent) = identity_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        load_or_create_identity(&identity_path)?
    };
    let identity_hash = hex::encode(identity.address_hash().as_slice());
    let display_name =
        std::env::var("LXMF_DISPLAY_NAME").ok().and_then(|v| normalize_display_name(&v));

    // --- Config ---
    let config_service_path = (!cfg.ephemeral)
        .then(|| cfg.config.clone().unwrap_or_else(crate::config::default_config_path));
    let config_path = cfg.config.or_else(|| {
        if cfg.ephemeral {
            return None;
        }
        let default = crate::config::default_config_path();
        default.exists().then_some(default)
    });
    let daemon_config = config_path.as_ref().and_then(|p| DaemonConfig::from_path(p).ok());
    let rnode_interfaces = daemon_config
        .as_ref()
        .map(DaemonConfig::rnode_interfaces)
        .transpose()
        .map_err(anyhow::Error::msg)?
        .unwrap_or_default();
    #[cfg(not(feature = "native-serial"))]
    if !rnode_interfaces.is_empty() {
        anyhow::bail!("RNode interfaces require the styrened native-serial feature");
    }

    let node_role = daemon_config.as_ref().map(|c| c.role).unwrap_or_default();
    crate::daemon_diagnostic!("[styrene] node role: {}", node_role);

    // Open and migrate the shared authority before any propagation handler can be registered.
    let db_path = cfg.db.clone().unwrap_or_else(crate::config::default_db_path);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(crate::config::default_config_dir()).ok();
    let store = Arc::new(Mutex::new(MessagesStore::open(&db_path)?));

    // --- Transport ---
    let mesh_transport: Arc<dyn MeshTransport>;
    let mut delivery_hash = String::new();
    let mut service_receipt_target = None;
    let mut native_nomadnet = None;
    let mut native_transport = None;
    let mut standard_propagation = None;

    if node_role.runs_transport() {
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
        let mut config = TransportConfig::new("styrene", &transport_identity, true);
        // Enable announce retransmission for nodes that run transport.
        // This allows the node to relay announces between non-adjacent peers,
        // enabling multi-hop mesh routing (equivalent to Reticulum transport.enabled).
        config
            .set_retransmit(daemon_config.as_ref().is_none_or(DaemonConfig::transport_retransmit));
        let mut transport_instance = Transport::new(config);
        startup.record(startup_component::NATIVE_RESOURCE_RETRY_SCHEDULER);
        let receipt_target = Arc::new(std::sync::OnceLock::new());
        let packet_receipts = crate::receipt_bridge::PacketReceiptBridge::new();
        transport_instance
            .set_receipt_handler(Box::new(crate::receipt_bridge::CompositeReceiptHandler::new(
                vec![
                    Box::new(crate::receipt_bridge::ServiceReceiptBridge::new(
                        receipt_target.clone(),
                    )),
                    Box::new(packet_receipts.clone()),
                ],
            )))
            .await;
        service_receipt_target = Some(receipt_target);
        startup.record(startup_component::SERVICE_RECEIPT_BRIDGE);

        // TCP server on default or configured address
        let bind_addr = tcp_server_bind_addr(daemon_config.as_ref(), cfg.ephemeral);

        let iface_manager = transport_instance.iface_manager();
        #[cfg(feature = "native-serial")]
        for interface in rnode_interfaces {
            let rnode = RNodeInterface::new(interface.device, interface.profile)?
                .with_baud_rate(interface.baud_rate)
                .with_reconnect_delay(std::time::Duration::from_millis(
                    interface.reconnect_delay_ms,
                ));
            iface_manager.lock().await.spawn(rnode, RNodeInterface::spawn);
        }
        let (tcp_server, _bound_rx) = TcpServer::new(bind_addr.clone(), iface_manager.clone());
        iface_manager.lock().await.spawn(tcp_server, TcpServer::spawn);
        crate::daemon_diagnostic!("[styrene] tcp_server bind={}", bind_addr);

        // TCP clients from config
        if let Some(ref config) = daemon_config {
            for (host, port) in config.tcp_client_endpoints() {
                let endpoint = format!("{}:{}", host, port);
                iface_manager
                    .lock()
                    .await
                    .spawn(TcpClient::new(endpoint.clone()), TcpClient::spawn);
                crate::daemon_diagnostic!("[styrene] tcp_client endpoint={}", endpoint);
            }
        }

        // LXMF delivery destination
        let destination = transport_instance
            .add_destination(transport_identity.clone(), DestinationName::new("lxmf", "delivery"))
            .await;

        // NomadNet page hosting destination — allows us to receive announces
        // from NomadNet-compatible page hosts and browse their pages.
        let nomadnet_destination = transport_instance
            .add_destination(
                transport_identity.clone(),
                DestinationName::new("nomadnetwork", "node"),
            )
            .await;
        if node_role == crate::config::NodeRole::Hub {
            let propagation_name = std::env::var("STYRENE_PROPAGATION_NODE_NAME")
                .unwrap_or_else(|_| DEFAULT_PROPAGATION_NODE_NAME.to_string());
            let endpoint = StandardPropagationEndpoint::register(
                &mut transport_instance,
                transport_identity.clone(),
                &propagation_name,
                Arc::clone(&store),
            )
            .await
            .map_err(|error| {
                anyhow::anyhow!("standard propagation registration failed: {error:?}")
            })?;
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_DESTINATION);
            standard_propagation = Some(endpoint);
        }
        startup.record(startup_component::LXMF_DELIVERY);
        startup.record(startup_component::NOMADNET_NODE_DESTINATION);
        let (dest_hash_hex, delivery_addr) = {
            let dest = destination.lock().await;
            (hex::encode(dest.desc.address_hash.as_slice()), dest.desc.address_hash)
        };
        delivery_hash = dest_hash_hex;

        let transport = Arc::new(transport_instance);
        native_transport = Some(transport.clone());
        native_nomadnet = Some((transport.clone(), nomadnet_destination));
        let mut id_hash_bytes = [0u8; 16];
        id_hash_bytes.copy_from_slice(identity.address_hash().as_slice());

        let adapter = TokioTransportAdapter::new_with_packet_receipts(
            transport.clone(),
            rns_core::hash::AddressHash::new(id_hash_bytes),
            delivery_addr,
            destination.clone(),
            display_name.as_ref().and_then(|n| encode_delivery_display_name_app_data(n)),
            packet_receipts.sender(),
        )
        .await;
        startup.record(startup_component::TRANSPORT_ANNOUNCE_BRIDGE);
        startup.record(startup_component::TRANSPORT_LINK_BRIDGE);

        mesh_transport = Arc::new(adapter);
        crate::daemon_diagnostic!("[styrene] transport enabled, delivery_hash={}", delivery_hash);
    } else {
        mesh_transport = Arc::new(NullTransport::new());
        crate::daemon_diagnostic!("[styrene] transport disabled (node role: {})", node_role);
    };

    // Node store
    let node_store_path = db_path.with_file_name("nodes.db");
    let node_store = Arc::new(styrene_services::node_store::NodeStore::open(
        node_store_path.to_str().unwrap_or("nodes.db"),
    )?);

    // --- RBAC policy: config → DB overlay → normalize ---
    let rbac_policy = {
        let mut policy = daemon_config.as_ref().and_then(|c| c.rbac.clone()).unwrap_or_default();

        // Overlay roster entries from SQLite (DB wins on conflict)
        {
            let store_guard = store.lock().unwrap();
            if let Ok(db_entries) = store_guard.load_rbac_roster() {
                for entry in db_entries {
                    policy.add_entry(entry);
                }
            }
            if let Ok(blocked) = store_guard.blocked_peers() {
                for hash in blocked {
                    policy.block(&hash);
                }
            }
        }

        // Auto-roster the daemon's own identity as Admin so the local CLI
        // (which authenticates as the daemon) retains full administrative access.
        if policy.get_entry(&identity_hash).is_none() {
            policy.add_entry(
                styrene_rbac::RosterEntry::new(&identity_hash, styrene_rbac::Role::Admin)
                    .with_label("local"),
            );
        }

        // Verify hub-signed entries against trusted hubs.
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let trusted = policy.trusted_hubs().to_vec();
            let hub_entries: Vec<_> = policy.hub_entries().to_vec();
            let total = hub_entries.len();
            policy.clear_hub_entries();
            for entry in hub_entries {
                if entry.is_expired(now) {
                    crate::daemon_diagnostic!(
                        "[styrene] rbac: dropping expired hub entry for {}",
                        entry.entry.identity_hash
                    );
                } else if !trusted.iter().any(|h| h.matches(&entry)) {
                    crate::daemon_diagnostic!(
                        "[styrene] rbac: dropping hub entry for {} — hub not trusted",
                        entry.entry.identity_hash
                    );
                } else if !entry.verify() {
                    crate::daemon_diagnostic!(
                        "[styrene] rbac: dropping hub entry for {} — invalid signature",
                        entry.entry.identity_hash
                    );
                } else {
                    policy.add_hub_entry(entry);
                }
            }
            if total > 0 {
                crate::daemon_diagnostic!(
                    "[styrene] rbac: {}/{} hub-signed entries verified ({} trusted hubs)",
                    policy.hub_entries().len(),
                    total,
                    trusted.len(),
                );
            }
        }

        let warnings = policy.normalize();
        for w in &warnings {
            crate::daemon_diagnostic!("[styrene] rbac: {w:?}");
        }
        crate::daemon_diagnostic!(
            "[styrene] RBAC policy loaded: {} roster entries, {} hub entries, {} blocked prefixes, default_role={:?}",
            policy.entries().len(),
            policy.hub_entries().len(),
            policy.blocked_count(),
            policy.default_role,
        );
        policy
    };

    let transport_active = !delivery_hash.is_empty();

    // --- AppContext ---
    let app_context = Arc::new(AppContext::with_policy(
        mesh_transport,
        identity_hash.clone(),
        store,
        node_store,
        crate::services::PolicyService::new(rbac_policy),
    ));
    if let Some(config) = daemon_config.as_ref() {
        app_context.auto_reply().set_config((&config.auto_reply).into());
    }
    if let Some(endpoint) = standard_propagation.as_ref() {
        app_context.publish_standard_propagation(endpoint.runtime_observation());
        endpoint.set_events(app_context.events_arc());
    } else if transport_active && node_role != crate::config::NodeRole::Hub {
        app_context.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::client(),
        );
    }
    startup.record_local_execution_services();

    // Wire signer + delivery hash
    app_context.set_signer(Arc::new(identity.clone()));
    app_context.identity().set_delivery_destination_hash(Some(delivery_hash.clone()));

    if let Some(config_path) = config_service_path.as_ref()
        && let Err(e) = app_context.config().load_or_default(config_path)
    {
        crate::daemon_diagnostic!("[styrene] config load error: {e}");
    }

    if node_role == crate::config::NodeRole::Hub {
        app_context.propagation().set_enabled(true);
        app_context.status().set_propagation_state(true, None, 0);
        crate::daemon_diagnostic!("[styrene] propagation enabled (hub mode)");
    } else {
        app_context.status().set_propagation_state(false, None, 0);
    }

    // --- DaemonFacade ---
    let daemon_facade = Arc::new(DaemonFacade::new(app_context.clone(), identity_hash.clone()));

    // --- Workers ---
    let local_delivery_hash = if transport_active { Some(delivery_hash) } else { None };
    let standard_propagation_hash =
        standard_propagation.as_ref().map(|endpoint| endpoint.destination().clone());
    let standard_propagation_hash = if let Some(destination) = standard_propagation_hash {
        Some(hex::encode(destination.lock().await.desc.address_hash.as_slice()))
    } else {
        None
    };

    if let (Some(endpoint), Some(transport)) =
        (standard_propagation.as_mut(), native_transport.as_ref())
    {
        endpoint.activate(app_context.transport_arc(), Arc::clone(transport)).await.map_err(
            |error| anyhow::anyhow!("standard propagation activation failed: {error:?}"),
        )?;
        app_context.network_operations().set_propagation_announce_trigger(
            endpoint.announce_trigger().expect("active endpoint"),
        );
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_OFFER_HANDLER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_GET_HANDLER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_INGRESS_WORKER);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_ANNOUNCE);
        startup
            .advertise(startup_capability::STANDARD_LXMF_PROPAGATION)
            .map_err(anyhow::Error::msg)?;
    }

    let inbound_worker = crate::workers::inbound::spawn_inbound_worker_with_auto_reply(
        app_context.transport_arc(),
        app_context.messaging_arc(),
        app_context.protocol_arc(),
        app_context.events_arc(),
        app_context.propagation_arc(),
        crate::workers::inbound::InboundDestinations::new(
            local_delivery_hash,
            standard_propagation_hash,
        ),
        Some(app_context.auto_reply_arc()),
    );
    startup.record(startup_component::INBOUND_PACKET_WORKER);
    startup.record(startup_component::INBOUND_RESOURCE_WORKER);
    startup.record(startup_component::OUTBOUND_RESOURCE_COMPLETION_WORKER);
    let announce_worker = crate::workers::announce::spawn_announce_worker(
        app_context.transport_arc(),
        app_context.discovery_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ANNOUNCE_WORKER);
    let link_worker = crate::workers::link::spawn_link_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::LINK_WORKER);
    let route_worker = crate::workers::route::spawn_route_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ROUTE_WORKER);
    startup.record(startup_component::NETWORK_OPERATION_COORDINATOR);
    let router_deadline_worker =
        crate::workers::router::spawn_router_deadline_worker(app_context.messaging_arc());
    startup.record(startup_component::LXMF_ROUTER_DEADLINE_SCHEDULER);
    let standard_propagation_sync = (transport_active && node_role != crate::config::NodeRole::Hub)
        .then(|| {
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR);
            startup.record(startup_component::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER);
            crate::workers::standard_propagation::spawn_standard_propagation_sync_worker(
                app_context.messaging_arc(),
                app_context.transport().subscribe_lifecycle(),
                app_context.transport().is_connected(),
                app_context.events_arc(),
            )
        });
    if let Some(worker) = &standard_propagation_sync {
        app_context.publish_standard_propagation_sync(worker.observation());
    }

    crate::workers::register_styrene_rpc_handlers(&app_context, Arc::new(identity.clone())).await;
    startup.record(startup_component::RPC_RESPONSE_HANDLER);
    startup.record(startup_component::RPC_REQUEST_HANDLER);

    if let Some((transport, destination)) = native_nomadnet {
        crate::workers::native_nomadnet::register_handlers(
            destination.clone(),
            app_context.pages_arc(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("native NomadNet path registration failed: {error:?}"))?;
        startup.record(startup_component::NATIVE_NOMADNET_REQUEST_HANDLER);
        transport.send_announce(&destination, display_name.as_deref().map(str::as_bytes)).await;
        let node_app_data = display_name.as_deref().map(|name| name.as_bytes().to_vec());
        let announce_transport = Arc::clone(&transport);
        let announce_destination = destination.clone();
        app_context.network_operations().set_nomadnet_announce(Arc::new(move || {
            let transport = Arc::clone(&announce_transport);
            let destination = announce_destination.clone();
            let app_data = node_app_data.clone();
            Box::pin(async move {
                transport.send_announce(&destination, app_data.as_deref()).await;
            })
        }));
        startup.record(startup_component::NOMADNET_NODE_ANNOUNCE);
        startup.advertise(startup_capability::NATIVE_NOMADNET_HOST).map_err(anyhow::Error::msg)?;
    }

    let expiry_worker =
        crate::services::propagation::spawn_expiry_task(app_context.propagation_arc());
    startup.record(startup_component::PROPAGATION_EXPIRY_SCHEDULER);
    if let Some(target) = service_receipt_target
        && target.set(Arc::downgrade(&app_context.messaging_arc())).is_err()
    {
        anyhow::bail!("service receipt target initialized twice");
    }

    crate::daemon_diagnostic!("[styrene] workers started");

    startup.advertise(startup_capability::LOCAL_CONFIG).map_err(anyhow::Error::msg)?;
    startup.advertise(startup_capability::LOCAL_POLICY).map_err(anyhow::Error::msg)?;

    if transport_active {
        startup.record_transport_state_services();
        startup.advertise(startup_capability::LXMF_DIRECT).map_err(anyhow::Error::msg)?;
        startup.advertise(startup_capability::LXMF_PAPER_EXPORT).map_err(anyhow::Error::msg)?;
        startup.advertise(startup_capability::NETWORK_OPERATIONS).map_err(anyhow::Error::msg)?;
        startup.advertise(startup_capability::RNS_REQUESTS).map_err(anyhow::Error::msg)?;
        startup
            .advertise(startup_capability::RNS_REQUEST_CANCELLATION)
            .map_err(anyhow::Error::msg)?;
        startup
            .advertise(startup_capability::RNS_RESOURCE_CANCELLATION)
            .map_err(anyhow::Error::msg)?;
        startup.advertise(startup_capability::STYRENE_RPC).map_err(anyhow::Error::msg)?;
        if node_role != crate::config::NodeRole::Hub {
            startup
                .advertise(startup_capability::STANDARD_LXMF_PROPAGATION_CLIENT)
                .map_err(anyhow::Error::msg)?;
        }
    }
    app_context.publish_startup_contract(startup.clone().finish());

    // --- IPC Server (desktop only) ---
    #[cfg(feature = "ipc-server")]
    let (ipc_server, ipc_event_worker) = {
        let socket_path = cfg.socket.unwrap_or_else(styrene_ipc_server::default_socket_path);
        let ipc_config = styrene_ipc_server::IpcServerConfig {
            socket_path: socket_path.clone(),
            event_capacity: 256,
        };
        let mut server = styrene_ipc_server::IpcServer::new(
            daemon_facade.clone() as Arc<dyn styrene_ipc::traits::Daemon>,
            ipc_config,
        );
        server.start().await?;

        // Bridge daemon events → IPC server event channel
        let event_worker = {
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
            })
        };
        startup.record(startup_component::IPC_EVENT_BRIDGE);
        crate::daemon_diagnostic!("[styrene] IPC server listening on {}", socket_path.display());
        (server, event_worker)
    };

    // Initial announce
    app_context.transport().announce(None).await;
    crate::daemon_diagnostic!("[styrene] identity={} ready", identity_hash);

    let startup_contract = startup.finish();
    app_context.publish_startup_contract(startup_contract.clone());
    let workers = DaemonWorkers {
        inbound: inbound_worker,
        announce: announce_worker,
        link: link_worker,
        route: route_worker,
        router_deadlines: router_deadline_worker,
        standard_propagation_sync,
        expiry: expiry_worker,
        #[cfg(feature = "ipc-server")]
        ipc_events: ipc_event_worker,
    };

    Ok(DaemonHandle {
        app_context,
        daemon_facade,
        startup_contract,
        standard_propagation,
        #[cfg(feature = "ipc-server")]
        ipc_server,
        workers,
    })
}

fn tcp_server_bind_addr(config: Option<&DaemonConfig>, ephemeral: bool) -> String {
    config
        .and_then(DaemonConfig::tcp_server_endpoint)
        .unwrap_or_else(|| if ephemeral { "127.0.0.1:0" } else { "0.0.0.0:4242" }.to_string())
}

/// Run the daemon until interrupted (Ctrl+C).
pub async fn run(cfg: DaemonConfig2) -> anyhow::Result<()> {
    let handle = start(cfg).await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    crate::daemon_diagnostic!("\n[styrene] shutting down...");
    handle.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn ephemeral_daemon_does_not_default_to_production_listener() {
        assert_eq!(tcp_server_bind_addr(None, true), "127.0.0.1:0");
        assert_eq!(tcp_server_bind_addr(None, false), "0.0.0.0:4242");
    }

    #[test]
    fn explicit_listener_is_preserved_for_ephemeral_daemon() {
        let config = DaemonConfig::from_toml(
            r#"
            [[interfaces]]
            type = "tcp_server"
            enabled = true
            host = "127.0.0.1"
            port = 14242
            "#,
        )
        .unwrap();

        assert_eq!(tcp_server_bind_addr(Some(&config), true), "127.0.0.1:14242");
    }

    #[tokio::test]
    async fn canonical_hub_recovers_standard_queue_before_active_composition() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("messages.db");
        let config = root.path().join("config.toml");
        std::fs::write(&config, "role = \"hub\"\n").unwrap();
        let mut data = vec![0x41; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        data[..16].copy_from_slice(&[0x42; 16]);
        let item = crate::storage::standard_propagation::StandardPropagationItem {
            transient_id: Sha256::digest(&data).into(),
            destination: [0x42; 16],
            stored_size: data.len() + 32,
            lxmf_data: data,
            stamp: [0x43; 32],
            stamp_value: 0,
            received_at: 1,
            expires_at: i64::MAX,
        };
        MessagesStore::open(&db)
            .unwrap()
            .standard_propagation_ingest_batch(
                crate::storage::standard_propagation::StandardPropagationIngestRequest {
                    items: &[item],
                    source_peer: None,
                    attempt: crate::storage::standard_propagation::StandardPropagationAttemptStatus::Untracked,
                    protocol: crate::storage::standard_propagation::StandardPropagationProtocolStatus::Valid,
                    now: 1,
                    policy: crate::storage::standard_propagation::StandardPropagationPolicy {
                    queue_max_count: 4096,
                    queue_max_bytes: 16 * 1024 * 1024,
                    expiry_secs: 30 * 24 * 60 * 60,
                },
                },
            )
            .unwrap();

        let handle = start(DaemonConfig2 {
            db: Some(db),
            config: Some(config),
            identity: None,
            socket: Some(root.path().join("daemon.sock")),
            ephemeral: true,
        })
        .await
        .unwrap();
        let endpoint = handle.standard_propagation.as_ref().unwrap();
        assert_eq!(endpoint.queue_stats(2).unwrap().queued_count, 1);
        assert!(
            handle
                .startup_contract
                .advertises(crate::startup_contract::capabilities::STANDARD_LXMF_PROPAGATION.id())
        );
        handle.shutdown().await;
    }
}
