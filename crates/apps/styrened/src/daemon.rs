//! Daemon entry point for the unified `styrene` binary.
//!
//! Clean boot path using only the new service architecture (AppContext +
//! DaemonFacade + IPC server). Does NOT start the legacy RPC server.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rns_core::destination::DestinationName;
use rns_core::transport::core_transport::{Transport, TransportConfig};
use rns_core::transport::iface::tcp_client::TcpClient;
use rns_core::transport::iface::tcp_server::TcpServer;
use tokio_util::sync::CancellationToken;

use crate::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use crate::app_context::AppContext;
use crate::config::DaemonConfig;
use crate::daemon_facade::DaemonFacade;
use crate::identity_store::load_or_create_identity;
use crate::storage::messages::MessagesStore;
use crate::transport::adapter::TokioTransportAdapter;
use crate::transport::mesh_transport::MeshTransport;
use crate::transport::null_transport::NullTransport;

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

/// Handle to a running daemon — drop to shut down.
pub struct DaemonHandle {
    pub app_context: Arc<AppContext>,
    pub daemon_facade: Arc<DaemonFacade>,
    #[cfg(feature = "ipc-server")]
    _ipc_server: styrene_ipc_server::IpcServer,
    _cancel: CancellationToken,
}

/// Start the daemon with the given configuration.
///
/// Returns a handle that keeps the daemon alive. The daemon runs
/// until the handle is dropped or the process is interrupted.
pub async fn start(cfg: DaemonConfig2) -> anyhow::Result<DaemonHandle> {
    let cancel = CancellationToken::new();

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
    let config_path = cfg.config.or_else(|| {
        let default = crate::config::default_config_path();
        default.exists().then_some(default)
    });
    let daemon_config = config_path.as_ref().and_then(|p| DaemonConfig::from_path(p).ok());

    let node_role = daemon_config.as_ref().map(|c| c.role).unwrap_or_default();
    eprintln!("[styrene] node role: {}", node_role);

    // --- Transport ---
    let mesh_transport: Arc<dyn MeshTransport>;
    let mut delivery_hash = String::new();

    if node_role.runs_transport() {
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
        let mut config = TransportConfig::new("styrene", &transport_identity, true);
        // Enable announce retransmission for nodes that run transport.
        // This allows the node to relay announces between non-adjacent peers,
        // enabling multi-hop mesh routing (equivalent to Reticulum transport.enabled).
        config.set_retransmit(true);
        let mut transport_instance = Transport::new(config);

        // TCP server on default or configured address
        let bind_addr = daemon_config
            .as_ref()
            .and_then(|c| c.tcp_server_endpoint())
            .unwrap_or_else(|| "0.0.0.0:4242".to_string());

        let iface_manager = transport_instance.iface_manager();
        let (tcp_server, _bound_rx) = TcpServer::new(bind_addr.clone(), iface_manager.clone());
        iface_manager.lock().await.spawn(tcp_server, TcpServer::spawn);
        eprintln!("[styrene] tcp_server bind={}", bind_addr);

        // TCP clients from config
        if let Some(ref config) = daemon_config {
            for (host, port) in config.tcp_client_endpoints() {
                let endpoint = format!("{}:{}", host, port);
                iface_manager
                    .lock()
                    .await
                    .spawn(TcpClient::new(endpoint.clone()), TcpClient::spawn);
                eprintln!("[styrene] tcp_client endpoint={}", endpoint);
            }
        }

        // LXMF delivery destination
        let destination = transport_instance
            .add_destination(transport_identity.clone(), DestinationName::new("lxmf", "delivery"))
            .await;
        let (dest_hash_hex, delivery_addr) = {
            let dest = destination.lock().await;
            (hex::encode(dest.desc.address_hash.as_slice()), dest.desc.address_hash)
        };
        delivery_hash = dest_hash_hex;

        let transport = Arc::new(transport_instance);
        let mut id_hash_bytes = [0u8; 16];
        id_hash_bytes.copy_from_slice(identity.address_hash().as_slice());

        let adapter = TokioTransportAdapter::new(
            transport.clone(),
            rns_core::hash::AddressHash::new(id_hash_bytes),
            delivery_addr,
            destination.clone(),
            display_name.as_ref().and_then(|n| encode_delivery_display_name_app_data(n)),
        )
        .await;

        mesh_transport = Arc::new(adapter);
        eprintln!("[styrene] transport enabled, delivery_hash={}", delivery_hash);
    } else {
        mesh_transport = Arc::new(NullTransport::new());
        eprintln!("[styrene] transport disabled (node role: {})", node_role);
    };

    // --- Database ---
    let db_path = cfg.db.unwrap_or_else(crate::config::default_db_path);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(crate::config::default_config_dir()).ok();

    let store = Arc::new(Mutex::new(MessagesStore::open(&db_path)?));

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
                    eprintln!(
                        "[styrene] rbac: dropping expired hub entry for {}",
                        entry.entry.identity_hash
                    );
                } else if !trusted.iter().any(|h| h.matches(&entry)) {
                    eprintln!(
                        "[styrene] rbac: dropping hub entry for {} — hub not trusted",
                        entry.entry.identity_hash
                    );
                } else if !entry.verify() {
                    eprintln!(
                        "[styrene] rbac: dropping hub entry for {} — invalid signature",
                        entry.entry.identity_hash
                    );
                } else {
                    policy.add_hub_entry(entry);
                }
            }
            if total > 0 {
                eprintln!(
                    "[styrene] rbac: {}/{} hub-signed entries verified ({} trusted hubs)",
                    policy.hub_entries().len(),
                    total,
                    trusted.len(),
                );
            }
        }

        let warnings = policy.normalize();
        for w in &warnings {
            eprintln!("[styrene] rbac: {w:?}");
        }
        eprintln!(
            "[styrene] RBAC policy loaded: {} roster entries, {} hub entries, {} blocked prefixes, default_role={:?}",
            policy.entries().len(),
            policy.hub_entries().len(),
            policy.blocked_count(),
            policy.default_role,
        );
        policy
    };

    // --- AppContext ---
    let app_context = Arc::new(AppContext::with_policy(
        mesh_transport,
        identity_hash.clone(),
        store,
        node_store,
        crate::services::PolicyService::new(rbac_policy),
    ));

    // Wire signer + delivery hash
    app_context.set_signer(Arc::new(identity.clone()));
    app_context.identity().set_delivery_destination_hash(Some(delivery_hash.clone()));

    if let Some(config_path) = config_path.as_ref() {
        if let Err(e) = app_context.config().load(config_path) {
            eprintln!("[styrene] config load error: {e}");
        }
    }

    if node_role == crate::config::NodeRole::Hub {
        app_context.propagation().set_enabled(true);
        eprintln!("[styrene] propagation enabled (hub mode)");
    }

    // --- DaemonFacade ---
    let daemon_facade = Arc::new(DaemonFacade::new(app_context.clone(), identity_hash.clone()));

    // --- Workers ---
    let local_delivery_hash = if delivery_hash.is_empty() { None } else { Some(delivery_hash) };

    crate::workers::inbound::spawn_inbound_worker_with_auto_reply(
        app_context.transport_arc(),
        app_context.messaging_arc(),
        app_context.protocol_arc(),
        app_context.events_arc(),
        app_context.propagation_arc(),
        local_delivery_hash,
        Some(app_context.auto_reply_arc()),
    );
    crate::workers::announce::spawn_announce_worker(
        app_context.transport_arc(),
        app_context.discovery_arc(),
        app_context.events_arc(),
    );
    crate::workers::link::spawn_link_worker(app_context.transport_arc(), app_context.events_arc());

    // RPC handlers
    app_context
        .protocol()
        .register(Arc::new(crate::workers::rpc_response::RpcResponseHandler::new(
            app_context.fleet_arc(),
        )))
        .await;
    app_context
        .protocol()
        .register(Arc::new(crate::workers::rpc_request::RpcRequestHandler::new(
            app_context.transport_arc(),
            Arc::new(identity.clone()),
            app_context.policy_arc(),
        )))
        .await;

    crate::services::propagation::spawn_expiry_task(app_context.propagation_arc());

    eprintln!("[styrene] workers started");

    // --- IPC Server (desktop only) ---
    #[cfg(feature = "ipc-server")]
    let ipc_server = {
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
        eprintln!("[styrene] IPC server listening on {}", socket_path.display());
        server
    };

    // Initial announce
    app_context.transport().announce(None).await;
    eprintln!("[styrene] identity={} ready", identity_hash);

    Ok(DaemonHandle {
        app_context,
        daemon_facade,
        #[cfg(feature = "ipc-server")]
        _ipc_server: ipc_server,
        _cancel: cancel,
    })
}

/// Run the daemon until interrupted (Ctrl+C).
pub async fn run(cfg: DaemonConfig2) -> anyhow::Result<()> {
    let _handle = start(cfg).await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    eprintln!("\n[styrene] shutting down...");
    Ok(())
}
