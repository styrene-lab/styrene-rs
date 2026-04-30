use super::announce_worker::spawn_announce_worker;
use super::bridge::{PeerCrypto, TransportBridge};
use super::inbound_worker::spawn_inbound_worker;
use super::receipt_worker::spawn_receipt_worker;
use super::Args;
use rns_core::destination::{DestinationName, SingleInputDestination};
use rns_core::transport::core_transport::{Transport, TransportConfig};
use rns_core::transport::iface::tcp_client::TcpClient;
use rns_core::transport::iface::tcp_server::TcpServer;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use styrened::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use styrened::app_context::AppContext;
use styrened::config::DaemonConfig;
use styrened::daemon_facade::DaemonFacade;
use styrened::identity_store::load_or_create_identity;
use styrened::receipt_bridge::ReceiptBridge;
use styrened::rpc::{AnnounceBridge, InterfaceRecord, OutboundBridge, RpcDaemon};
use styrened::storage::messages::MessagesStore;
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
    /// Unix socket IPC server — serves the Daemon trait to TUI and CLI clients.
    #[allow(dead_code)]
    pub(super) ipc_server: styrene_ipc_server::IpcServer,
}

pub(super) async fn bootstrap(args: Args) -> BootstrapContext {
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

    let identity_path =
        args.identity.clone().unwrap_or_else(styrened::config::default_identity_path);
    let identity = load_or_create_identity(&identity_path).expect("load identity");
    let identity_hash = hex::encode(identity.address_hash().as_slice());
    let local_display_name =
        std::env::var("LXMF_DISPLAY_NAME").ok().and_then(|value| normalize_display_name(&value));
    // Try explicit --config, then default path
    let config_path = args.config.clone().or_else(|| {
        let default = styrened::config::default_config_path();
        if default.exists() {
            Some(default)
        } else {
            None
        }
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
    let mut delivery_destination_hash_hex: Option<String> = None;
    let mut delivery_source_hash = [0u8; 16];
    let receipt_map: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let (receipt_tx, receipt_rx) = unbounded_channel();

    if let Some(addr) = args.transport.clone().filter(|_| node_role.runs_transport()) {
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
        let config = TransportConfig::new("daemon", &transport_identity, true);
        let mut transport_instance = Transport::new(config);
        transport_instance
            .set_receipt_handler(Box::new(ReceiptBridge::new(
                receipt_map.clone(),
                receipt_tx.clone(),
            )))
            .await;
        let iface_manager = transport_instance.iface_manager();
        let (tcp_server, _bound_addr_rx) = TcpServer::new(addr.clone(), iface_manager.clone());
        let server_iface = iface_manager
            .lock()
            .await
            .spawn(tcp_server, TcpServer::spawn);
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
                receipt_tx.clone(),
            ))
        });

    let outbound_bridge: Option<Arc<dyn OutboundBridge>> =
        bridge.as_ref().map(|bridge| bridge.clone() as Arc<dyn OutboundBridge>);
    let announce_bridge: Option<Arc<dyn AnnounceBridge>> =
        bridge.as_ref().map(|bridge| bridge.clone() as Arc<dyn AnnounceBridge>);

    let daemon = Arc::new(RpcDaemon::with_store_and_bridges(
        store,
        identity_hash,
        outbound_bridge,
        announce_bridge,
    ));
    let local_delivery_hash = delivery_destination_hash_hex.clone();
    daemon.set_delivery_destination_hash(delivery_destination_hash_hex.clone());
    daemon.replace_interfaces(configured_interfaces);
    daemon.set_propagation_state(transport.is_some(), None, 0);

    // Make the local delivery destination visible on startup.
    if let Some(bridge) = bridge.as_ref() {
        let _ = bridge.announce_now();
    }

    if transport.is_some() {
        spawn_receipt_worker(daemon.clone(), receipt_rx);
    }

    if args.announce_interval_secs > 0 {
        let _handle = daemon.clone().start_announce_scheduler(args.announce_interval_secs);
    }

    // Capture transport and announce destination for service architecture before
    // they're moved into workers.
    let transport_for_services = transport.clone();
    let announce_dest_for_services = announce_destination.clone();

    if let Some(transport) = transport {
        spawn_inbound_worker(daemon.clone(), transport.clone());
        spawn_announce_worker(daemon.clone(), transport, peer_crypto);
    }

    // --- New service architecture (runs alongside RpcDaemon during migration) ---
    // Wire TokioTransportAdapter when real transport exists, NullTransport otherwise.
    // Share the same MessagesStore that RpcDaemon uses (via a new in-memory instance
    // for now — will share the actual store once RpcDaemon field collapse progresses).
    let mesh_transport: Arc<dyn MeshTransport> = if let (Some(ref tp), Some(ref ann_dest)) =
        (&transport_for_services, &announce_dest_for_services)
    {
        let mut id_hash = [0u8; 16];
        id_hash.copy_from_slice(identity.address_hash().as_slice());
        let adapter = TokioTransportAdapter::new(
            tp.clone(),
            rns_core::hash::AddressHash::new(id_hash),
            rns_core::hash::AddressHash::new(delivery_source_hash),
            ann_dest.clone(),
            local_display_name
                .as_ref()
                .and_then(|name| encode_delivery_display_name_app_data(name)),
        )
        .await;
        eprintln!("[daemon] TokioTransportAdapter wired into service architecture");
        Arc::new(adapter)
    } else {
        Arc::new(NullTransport::new())
    };
    // Share the real SQLite store with the service architecture.
    // RpcDaemon holds its own Arc<Mutex<MessagesStore>>; we create a second
    // handle to the same on-disk database so both codepaths see the same data.
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

    let app_context = Arc::new(AppContext::with_node_store(
        mesh_transport,
        hex::encode(identity.address_hash().as_slice()),
        shared_store,
        node_store,
    ));
    let daemon_facade = Arc::new(DaemonFacade::new(
        app_context.clone(),
        hex::encode(identity.address_hash().as_slice()),
    ));
    // Load config into ConfigService if a config file was provided
    if let Some(config_path) = config_path.as_ref() {
        if let Err(e) = app_context.config().load(config_path) {
            eprintln!("[daemon] failed to load config into ConfigService: {}", e);
        }
    }
    // Wire signing identity into services that need outbound delivery
    app_context.set_signer(Arc::new(identity.clone()));
    // Wire delivery destination hash into IdentityService so DaemonFacade can
    // return it in query_identity responses (needed for LXMF messaging).
    app_context
        .identity()
        .set_delivery_destination_hash(local_delivery_hash.clone());
    eprintln!("[daemon] service architecture initialized (AppContext + DaemonFacade + signer)");

    // Enable propagation if node role is Hub
    if node_role == styrened::config::NodeRole::Hub {
        app_context.propagation().set_enabled(true);
        eprintln!("[daemon] propagation store enabled (hub mode)");
    }

    // --- Service-layer workers (inbound + announce processing) ---
    styrened::workers::inbound::spawn_inbound_worker_with_auto_reply(
        app_context.transport_arc(),
        app_context.messaging_arc(),
        app_context.protocol_arc(),
        app_context.events_arc(),
        app_context.propagation_arc(),
        local_delivery_hash,
        Some(app_context.auto_reply_arc()),
    );

    // Spawn propagation expiry cleanup task
    styrened::services::propagation::spawn_expiry_task(app_context.propagation_arc());
    styrened::workers::announce::spawn_announce_worker(
        app_context.transport_arc(),
        app_context.discovery_arc(),
        app_context.events_arc(),
    );
    styrened::workers::link::spawn_link_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    // Register RPC response handler for StyreneProtocol responses
    app_context
        .protocol()
        .register(std::sync::Arc::new(styrened::workers::rpc_response::RpcResponseHandler::new(
            app_context.fleet_arc(),
        )))
        .await;
    app_context
        .protocol()
        .register(std::sync::Arc::new(styrened::workers::rpc_request::RpcRequestHandler::new(
            app_context.transport_arc(),
            std::sync::Arc::new(identity.clone()),
            app_context.auth_arc(),
        )))
        .await;
    eprintln!("[daemon] service workers started (inbound + announce + rpc-request + rpc-response)");

    // --- Unix socket IPC server ---
    let ipc_config = styrene_ipc_server::IpcServerConfig {
        socket_path: args.socket.clone().unwrap_or_else(styrene_ipc_server::default_socket_path),
        event_capacity: 256,
    };
    let mut ipc_server = styrene_ipc_server::IpcServer::new(
        daemon_facade.clone() as Arc<dyn styrene_ipc::traits::Daemon>,
        ipc_config,
    );
    match ipc_server.start().await {
        Ok(()) => {
            eprintln!("[daemon] IPC server listening on {}", ipc_server.socket_path().display())
        }
        Err(e) => eprintln!("[daemon] IPC server failed to start: {e}"),
    }

    BootstrapContext { rpc_addr, daemon, rpc_tls, app_context, daemon_facade, ipc_server }
}
