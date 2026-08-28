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
use styrened::receipt_bridge::ServiceReceiptBridge;
use styrened::startup_contract::{
    RuntimeKind, StartupContract, StartupContractBuilder, capabilities as startup_capability,
    components as startup_component,
};
use styrened::storage::messages::MessagesStore;
use styrened::transport::adapter::TokioTransportAdapter;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

struct TestNodeWorkers {
    inbound: styrened::workers::inbound::InboundWorkerHandle,
    announce: tokio::task::JoinHandle<()>,
    link: tokio::task::JoinHandle<()>,
}

impl TestNodeWorkers {
    fn abort(&self) {
        self.inbound.abort();
        self.announce.abort();
        self.link.abort();
    }
}

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
    /// Internal-only composition evidence for this test node.
    pub startup_contract: StartupContract,
    workers: Mutex<Option<TestNodeWorkers>>,
    interface_shutdown: CancellationToken,
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
        let mut startup = StartupContractBuilder::internal_test(RuntimeKind::E2eTest);
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
        startup.record(startup_component::NATIVE_RESOURCE_RETRY_SCHEDULER);

        // 4. TCP server (if requested)
        let iface_manager = transport_instance.iface_manager();
        let interface_shutdown = iface_manager.lock().await.shutdown_token();
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
        startup.record(startup_component::LXMF_DELIVERY);
        let (delivery_hash, delivery_addr) = {
            let dest = destination.lock().await;
            (hex::encode(dest.desc.address_hash.as_slice()), dest.desc.address_hash)
        };

        let receipt_target = Arc::new(std::sync::OnceLock::new());
        transport_instance
            .set_receipt_handler(Box::new(ServiceReceiptBridge::new(receipt_target.clone())))
            .await;

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
        startup.record(startup_component::TRANSPORT_ANNOUNCE_BRIDGE);
        startup.record(startup_component::TRANSPORT_LINK_BRIDGE);

        // 9. AppContext with in-memory stores
        let store =
            Arc::new(Mutex::new(MessagesStore::in_memory().expect("in-memory message store")));
        let app_context =
            Arc::new(AppContext::new(Arc::new(adapter), identity_hash.clone(), store));
        app_context
            .policy()
            .grant(
                styrene_rbac::RosterEntry::new(&identity_hash, styrene_rbac::Role::Admin),
                app_context.store(),
            )
            .expect("test node must authorize its own local control identity");
        let messaging = app_context.messaging_arc();
        receipt_target
            .set(Arc::downgrade(&messaging))
            .expect("receipt target should be initialized once");

        // 10. Wire signer + delivery hash into IdentityService
        app_context.set_signer(Arc::new(identity.clone()));
        app_context.identity().set_delivery_destination_hash(Some(delivery_hash.clone()));

        // 11. Spawn workers (with auto-reply support from AppContext's own service)
        let inbound_worker = styrened::workers::inbound::spawn_inbound_worker_with_auto_reply(
            app_context.transport_arc(),
            app_context.messaging_arc(),
            app_context.protocol_arc(),
            app_context.events_arc(),
            app_context.propagation_arc(),
            styrened::workers::inbound::InboundDestinations::new(Some(delivery_hash.clone()), None),
            Some(app_context.auto_reply_arc()),
        );
        startup.record(startup_component::INBOUND_PACKET_WORKER);
        startup.record(startup_component::INBOUND_RESOURCE_WORKER);
        startup.record(startup_component::OUTBOUND_RESOURCE_COMPLETION_WORKER);
        let announce_worker = styrened::workers::announce::spawn_announce_worker(
            app_context.transport_arc(),
            app_context.discovery_arc(),
            app_context.events_arc(),
        );
        startup.record(startup_component::ANNOUNCE_WORKER);
        let link_worker = styrened::workers::link::spawn_link_worker(
            app_context.transport_arc(),
            app_context.events_arc(),
        );
        startup.record(startup_component::LINK_WORKER);

        styrened::workers::register_styrene_rpc_handlers(&app_context, Arc::new(identity.clone()))
            .await;
        startup.record(startup_component::RPC_RESPONSE_HANDLER);
        startup.record(startup_component::RPC_REQUEST_HANDLER);

        // Register page request handler (all nodes can serve pages)
        app_context
            .protocol()
            .register(Arc::new(styrened::workers::page_handler::PageRequestHandler::new(
                app_context.transport_arc(),
                Arc::new(identity.clone()),
                app_context.pages_arc(),
            )))
            .await;
        startup.record(startup_component::STYRENE_PAGE_REQUEST_HANDLER);
        if let Err(error) = startup.advertise(startup_capability::STYRENE_PAGE_HOST) {
            panic!("invalid E2E page capability: {error}");
        }

        // Wire propagation hub if configured
        if let Some(hub_hash) = &self.propagation_hub {
            app_context.messaging().set_propagation_hub(hub_hash.clone(), app_context.fleet_arc());
        }

        // Register propagation handler if enabled
        if self.propagation_enabled {
            app_context.propagation().set_enabled(true);
            startup.record(startup_component::STYRENE_PROPAGATION_SERVICE);
            app_context
                .protocol()
                .register(Arc::new(
                    styrened::workers::propagation_handler::PropagationRequestHandler::new(
                        app_context.transport_arc(),
                        Arc::new(identity.clone()),
                        app_context.propagation_arc(),
                        app_context.messaging_arc(),
                        app_context.events_arc(),
                        Some(delivery_hash.clone()),
                    ),
                ))
                .await;
            startup.record(startup_component::STYRENE_PROPAGATION_REQUEST_HANDLER);
            if let Err(error) = startup.advertise(startup_capability::STYRENE_PROPAGATION_HOST) {
                panic!("invalid E2E propagation capability: {error}");
            }
        }

        let startup_contract = startup.finish();

        TestNode {
            name: self.name,
            identity,
            identity_hash,
            delivery_hash,
            delivery_addr,
            listen_addr,
            app_context,
            transport,
            startup_contract,
            workers: Mutex::new(Some(TestNodeWorkers {
                inbound: inbound_worker,
                announce: announce_worker,
                link: link_worker,
            })),
            interface_shutdown,
        }
    }
}

impl TestNode {
    /// Trigger an announce broadcast to all connected peers.
    pub async fn announce(&self) {
        self.app_context.transport().announce(None).await;
    }

    /// Attach another ephemeral TCP path and return its exact interface hash.
    pub async fn attach_tcp_client(&self, addr: SocketAddr) -> AddressHash {
        self.transport
            .iface_manager()
            .lock()
            .await
            .spawn(TcpClient::new(addr.to_string()), TcpClient::spawn)
    }

    /// Force loss of one test-owned interface.
    pub async fn cancel_interface(&self, hash: &AddressHash) {
        assert!(
            self.transport.iface_manager().lock().await.cancel_interface_for_test(hash),
            "test interface {hash} should exist"
        );
    }

    /// Stop all interfaces and await the transport scheduler.
    pub async fn shutdown(&self) {
        let workers = self.workers.lock().expect("test worker lock").take();
        if let Some(mut workers) = workers {
            workers.abort();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                workers.inbound.wait().await;
                let _ = workers.announce.await;
                let _ = workers.link.await;
            })
            .await
            .expect("test workers should stop within 2s");
        }

        let mut interface_tasks = {
            let manager = self.transport.iface_manager();
            let mut manager = manager.lock().await;
            manager.shutdown();
            manager.take_tasks()
        };
        for task in &mut interface_tasks {
            if tokio::time::timeout(std::time::Duration::from_secs(2), &mut *task).await.is_err() {
                task.abort();
                let _ = (&mut *task).await;
                panic!("interface task should stop within 2s");
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), self.transport.shutdown_manager())
            .await
            .expect("transport manager should stop within 2s")
            .expect("transport manager should stop cleanly");
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

impl Drop for TestNode {
    fn drop(&mut self) {
        self.interface_shutdown.cancel();
        if let Ok(workers) = self.workers.get_mut()
            && let Some(workers) = workers.take()
        {
            workers.abort();
        }
        let interface_manager = self.transport.iface_manager();
        if let Ok(manager) = interface_manager.try_lock() {
            manager.shutdown();
            manager.abort_tasks();
        }
        self.transport.abort_manager();
    }
}
