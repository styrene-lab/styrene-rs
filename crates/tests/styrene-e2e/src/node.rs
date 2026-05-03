//! TestNode — in-process daemon node for e2e testing.
//!
//! Replicates the bootstrap sequence from `styrened` in miniature:
//! deterministic identity, ephemeral TCP, in-memory SQLite, workers spawned.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use rns_core::destination::DestinationName;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::transport::core_transport::{Transport, TransportConfig};
use rns_core::transport::iface::tcp_client::TcpClient;
use rns_core::transport::iface::tcp_server::TcpServer;
use styrened::announce_names::encode_delivery_display_name_app_data;
use styrened::app_context::AppContext;
use styrened::storage::messages::MessagesStore;
use styrened::transport::adapter::TokioTransportAdapter;
use tokio::sync::watch;

/// A running daemon node for e2e testing.
pub struct TestNode {
    /// Human-readable name for logs.
    pub name: String,
    /// The node's RNS identity (core layer).
    pub identity: PrivateIdentity,
    /// Hex-encoded identity address hash (32 chars).
    pub identity_hash: String,
    /// Hex-encoded LXMF delivery destination hash.
    pub delivery_hash: String,
    /// Parsed delivery destination address hash.
    pub delivery_addr: AddressHash,
    /// Actual TCP listen address (after ephemeral port resolution), if serving.
    pub listen_addr: Option<SocketAddr>,
    /// The daemon's composition root — access services through this.
    pub app_context: Arc<AppContext>,
    /// Raw RNS transport handle (for direct transport operations).
    pub transport: Arc<Transport>,
}

/// Builder for constructing test nodes.
pub struct TestNodeBuilder {
    name: String,
    tcp_server_addr: Option<String>,
    tcp_client_addrs: Vec<SocketAddr>,
    identity: Option<PrivateIdentity>,
    retransmit: bool,
    propagation_enabled: bool,
    propagation_hub: Option<String>,
}

impl TestNodeBuilder {
    /// Create a new builder. If no identity is provided, one is derived
    /// deterministically from `name` via `PrivateIdentity::new_from_name`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            tcp_server_addr: None,
            tcp_client_addrs: Vec::new(),
            identity: None,
            retransmit: false,
            propagation_enabled: false,
            propagation_hub: None,
        }
    }

    /// Bind a TCP server. Use `"127.0.0.1:0"` for an ephemeral port.
    pub fn tcp_server(mut self, addr: &str) -> Self {
        self.tcp_server_addr = Some(addr.to_string());
        self
    }

    /// Connect as a TCP client to another node's listen address.
    pub fn tcp_client(mut self, addr: SocketAddr) -> Self {
        self.tcp_client_addrs.push(addr);
        self
    }

    /// Use a specific identity instead of deriving from name.
    pub fn identity(mut self, id: PrivateIdentity) -> Self {
        self.identity = Some(id);
        self
    }

    /// Enable announce retransmission (transport/relay mode).
    /// Required for hub nodes that route between non-adjacent peers.
    pub fn retransmit(mut self, enabled: bool) -> Self {
        self.retransmit = enabled;
        self
    }

    /// Enable propagation (store-and-forward for offline peers).
    /// Registers PropagationRequestHandler for handling ingest/fetch/delete.
    pub fn propagation(mut self, enabled: bool) -> Self {
        self.propagation_enabled = enabled;
        self
    }

    /// Set the propagation hub delivery hash for offline peer fallback.
    pub fn propagation_hub(mut self, hub_delivery_hash: String) -> Self {
        self.propagation_hub = Some(hub_delivery_hash);
        self
    }

    /// Build the test node, starting transport and workers.
    pub async fn build(self) -> TestNode {
        // 1. Identity
        let identity = self.identity.unwrap_or_else(|| PrivateIdentity::new_from_name(&self.name));
        let identity_hash = hex::encode(identity.address_hash().as_slice());

        // 2. Transport identity bridge
        let transport_identity =
            rns_core::transport::identity_bridge::to_transport_private_identity(&identity);

        // 3. Transport config + instance (mutable until Arc'd)
        let mut config = TransportConfig::new(&self.name, &transport_identity, true);
        if self.retransmit {
            config.set_retransmit(true);
        }
        let mut transport_instance = Transport::new(config);

        // 4. TCP server (if requested)
        let iface_manager = transport_instance.iface_manager();
        let mut bound_addr_rx: Option<watch::Receiver<Option<SocketAddr>>> = None;

        if let Some(addr) = &self.tcp_server_addr {
            let (tcp_server, rx) = TcpServer::new(addr.clone(), iface_manager.clone());
            iface_manager.lock().await.spawn(tcp_server, TcpServer::spawn);
            bound_addr_rx = Some(rx);
        }

        // 5. TCP clients
        for addr in &self.tcp_client_addrs {
            let endpoint = addr.to_string();
            iface_manager.lock().await.spawn(TcpClient::new(endpoint), TcpClient::spawn);
        }

        // 6. LXMF delivery destination
        let destination = transport_instance
            .add_destination(transport_identity.clone(), DestinationName::new("lxmf", "delivery"))
            .await;
        let (delivery_hash, delivery_addr) = {
            let dest = destination.lock().await;
            (hex::encode(dest.desc.address_hash.as_slice()), dest.desc.address_hash)
        };

        // 7. Wait for actual bound port if we started a server
        let listen_addr = if let Some(mut rx) = bound_addr_rx {
            // Wait for the watch to be populated (server binds in spawned task)
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                rx.wait_for(|addr| addr.is_some()),
            )
            .await
            .expect("tcp server failed to bind within 5s");
            *rx.borrow()
        } else {
            None
        };

        // 8. Wrap transport in Arc + adapter
        let transport = Arc::new(transport_instance);

        let mut id_hash_bytes = [0u8; 16];
        id_hash_bytes.copy_from_slice(identity.address_hash().as_slice());

        let announce_app_data = encode_delivery_display_name_app_data(&self.name);

        let adapter = TokioTransportAdapter::new(
            transport.clone(),
            AddressHash::new(id_hash_bytes),
            delivery_addr,
            destination.clone(),
            announce_app_data,
        )
        .await;

        // 9. AppContext with in-memory stores
        let store =
            Arc::new(Mutex::new(MessagesStore::in_memory().expect("in-memory message store")));
        let app_context =
            Arc::new(AppContext::new(Arc::new(adapter), identity_hash.clone(), store));

        // 10. Wire signer + delivery hash into IdentityService
        app_context.set_signer(Arc::new(identity.clone()));
        app_context.identity().set_delivery_destination_hash(Some(delivery_hash.clone()));

        // 11. Spawn workers (with auto-reply support from AppContext's own service)
        styrened::workers::inbound::spawn_inbound_worker_with_auto_reply(
            app_context.transport_arc(),
            app_context.messaging_arc(),
            app_context.protocol_arc(),
            app_context.events_arc(),
            app_context.propagation_arc(),
            Some(delivery_hash.clone()),
            Some(app_context.auto_reply_arc()),
        );
        styrened::workers::announce::spawn_announce_worker(
            app_context.transport_arc(),
            app_context.discovery_arc(),
            app_context.events_arc(),
        );
        styrened::workers::link::spawn_link_worker(
            app_context.transport_arc(),
            app_context.events_arc(),
        );

        // Register RPC handlers for protocol dispatch
        app_context
            .protocol()
            .register(Arc::new(styrened::workers::rpc_response::RpcResponseHandler::new(
                app_context.fleet_arc(),
            )))
            .await;
        app_context
            .protocol()
            .register(Arc::new(styrened::workers::rpc_request::RpcRequestHandler::new(
                app_context.transport_arc(),
                Arc::new(identity.clone()),
                app_context.policy_arc(),
            )))
            .await;

        // Register page request handler (all nodes can serve pages)
        app_context
            .protocol()
            .register(Arc::new(styrened::workers::page_handler::PageRequestHandler::new(
                app_context.transport_arc(),
                Arc::new(identity.clone()),
                app_context.pages_arc(),
            )))
            .await;

        // Wire propagation hub if configured
        if let Some(hub_hash) = &self.propagation_hub {
            app_context.messaging().set_propagation_hub(hub_hash.clone(), app_context.fleet_arc());
        }

        // Register propagation handler if enabled
        if self.propagation_enabled {
            app_context.propagation().set_enabled(true);
            app_context
                .protocol()
                .register(Arc::new(
                    styrened::workers::propagation_handler::PropagationRequestHandler::new(
                        app_context.transport_arc(),
                        Arc::new(identity.clone()),
                        app_context.propagation_arc(),
                        app_context.messaging_arc(),
                        Some(delivery_hash.clone()),
                    ),
                ))
                .await;
        }

        TestNode {
            name: self.name,
            identity,
            identity_hash,
            delivery_hash,
            delivery_addr,
            listen_addr,
            app_context,
            transport,
        }
    }
}

impl TestNode {
    /// Trigger an announce broadcast to all connected peers.
    pub async fn announce(&self) {
        self.app_context.transport().announce(None).await;
    }

    /// Send a chat message to a peer by their delivery hash (hex string).
    pub async fn send_chat(
        &self,
        peer_delivery_hash: &str,
        content: &str,
    ) -> Result<String, std::io::Error> {
        self.app_context.messaging().send_chat(peer_delivery_hash, content, None).await
    }
}
