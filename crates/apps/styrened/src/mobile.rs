//! Mobile embedding — lightweight daemon boot and background poll.
//!
//! Provides the in-process daemon API for iOS/Android apps. No IPC server,
//! no PTY terminal, no Unix sockets. The host app calls these functions
//! directly via FFI or Rust → Swift/Kotlin bridge.
//!
//! # Usage (from Swift via UniFFI or C bridge)
//!
//! ```ignore
//! use styrened::mobile::{MobileNode, MobileConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = MobileConfig {
//!     config_dir: PathBuf::from("/var/mobile/Containers/.../styrene/config"),
//!     data_dir: PathBuf::from("/var/mobile/Containers/.../styrene/data"),
//!     hub_address: Some("hub.example.com:4242".into()),
//!     hub_delivery_hash: Some("aabbccdd...".into()),
//!     interfaces: Vec::new(),
//! };
//!
//! let node = MobileNode::boot(config).await?;
//!
//! // Foreground: full interactive use
//! let peers = node.list_peers().await?;
//! node.send_chat("deadbeef...", "hello from phone").await?;
//!
//! // Background (BGProcessingTask): poll hub for queued messages
//! let count = node.poll_hub().await?;
//! // → returns number of new messages fetched
//! # Ok(())
//! # }
//! ```
//!
//! # iOS Integration
//!
//! The host app should:
//! 1. Call `MobileNode::boot()` on first launch (stores identity in app container)
//! 2. Keep the `MobileNode` alive for the foreground session
//! 3. In `BGAppRefreshTask` handler: boot a fresh `MobileNode`, call `poll_hub()`, drop
//! 4. Post local notifications for new messages from `poll_hub()` results

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use crate::app_context::AppContext;
use crate::config::PlatformPaths;
use crate::daemon_facade::DaemonFacade;
use crate::services::messaging::InboundAcceptOutcome;
use crate::startup_contract::{
    ActiveCapabilities, RuntimeKind, StartupContract, StartupContractBuilder,
    capabilities as startup_capability, components as startup_component,
};
use crate::storage::messages::MessagesStore;
use crate::transport::mesh_transport::MeshTransport;

use rns_core::identity::PrivateIdentity;
use styrene_ipc::traits::{Daemon, DaemonIdentity, DaemonStatus};
use tokio::task::JoinHandle;

/// Mobile node configuration — provided by the host app.
/// How to store the identity private keys.
#[derive(Debug, Clone, Default)]
pub enum IdentityBackend {
    /// Platform keychain with biometric protection (iOS Keychain / macOS Keychain).
    /// Root secret stored in Secure Enclave, RNS keys derived via HKDF.
    /// Requires `mobile-keychain` feature.
    #[default]
    Keychain,
    /// Encrypted file with passphrase (argon2id + ChaCha20Poly1305).
    /// Requires `mobile-identity` feature.
    EncryptedFile,
    /// Plaintext file (development/testing only — NOT for production mobile).
    PlaintextFile,
}

#[derive(Debug, Clone)]
pub struct MobileConfig {
    /// Path to the config directory (app container).
    pub config_dir: PathBuf,
    /// Path to the data directory (app container).
    pub data_dir: PathBuf,
    /// Hub TCP address for transport (e.g., "hub.mesh.example:4242").
    pub hub_address: Option<String>,
    /// Hub's LXMF delivery hash for propagation fetch.
    pub hub_delivery_hash: Option<String>,
    /// Display name for the node (used in announces).
    pub display_name: Option<String>,
    /// Identity storage backend.
    pub identity_backend: IdentityBackend,
    /// Direct Reticulum TCP interfaces.
    pub interfaces: Vec<MobileInterfaceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileInterfaceConfig {
    TcpServer { bind_address: String },
    TcpClient { remote_address: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ValidatedMobileInterface {
    TcpServer(SocketAddr),
    TcpClient(String),
}

/// A running mobile daemon node — in-process, no IPC server.
pub struct MobileNode {
    pub app_context: Arc<AppContext>,
    pub facade: Arc<DaemonFacade>,
    paths: PlatformPaths,
    hub_delivery_hash: Option<String>,
    workers: MobileWorkers,
    tcp_listen_addresses: Vec<SocketAddr>,
    startup_contract: StartupContract,
}

struct MobileWorkers {
    inbound: crate::workers::inbound::InboundWorkerHandle,
    announce: JoinHandle<()>,
    link: JoinHandle<()>,
    route: JoinHandle<()>,
    router_deadlines: JoinHandle<()>,
    standard_propagation_sync:
        Option<crate::workers::standard_propagation::StandardPropagationSyncWorker>,
    aborted: bool,
}

struct MobileTransportRuntime {
    transport: Arc<dyn MeshTransport>,
    delivery_hash: Option<String>,
    tcp_listen_addresses: Vec<SocketAddr>,
    service_receipt_target:
        Option<Arc<std::sync::OnceLock<std::sync::Weak<crate::services::MessagingService>>>>,
}

impl MobileWorkers {
    fn abort(&mut self) {
        if self.aborted {
            return;
        }
        self.inbound.abort();
        self.announce.abort();
        self.link.abort();
        self.route.abort();
        self.router_deadlines.abort();
        if let Some(worker) = &self.standard_propagation_sync {
            worker.abort();
        }
        self.aborted = true;
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
        self.aborted = true;
        self.inbound.wait().await;
        let _ = (&mut self.announce).await;
        let _ = (&mut self.link).await;
        let _ = (&mut self.route).await;
        let _ = (&mut self.router_deadlines).await;
    }

    #[cfg(test)]
    fn all_finished(&self) -> bool {
        self.inbound.is_finished()
            && self.announce.is_finished()
            && self.link.is_finished()
            && self.route.is_finished()
            && self.router_deadlines.is_finished()
            && self.standard_propagation_sync.as_ref().is_none_or(|worker| worker.is_finished())
    }

    #[cfg(test)]
    fn abort_handles(&self) -> Vec<tokio::task::AbortHandle> {
        let mut handles = Vec::from(self.inbound.abort_handles());
        handles.push(self.announce.abort_handle());
        handles.push(self.link.abort_handle());
        handles.push(self.route.abort_handle());
        handles.push(self.router_deadlines.abort_handle());
        if let Some(worker) = &self.standard_propagation_sync {
            handles.push(worker.abort_handle());
        }
        handles
    }
}

impl Drop for MobileNode {
    fn drop(&mut self) {
        self.workers.abort();
    }
}

/// Result of a hub poll operation.
#[derive(Debug, Clone)]
pub struct PollResult {
    /// Number of new messages fetched.
    pub message_count: usize,
    /// The fetched messages (for local notification display).
    pub messages: Vec<PollMessage>,
}

/// A message fetched during hub poll (simplified for notification display).
#[derive(Debug, Clone)]
pub struct PollMessage {
    pub source_hash: String,
    pub content_preview: String,
    pub timestamp: i64,
}

fn compose_mobile_node(
    paths: PlatformPaths,
    identity: PrivateIdentity,
    store: Arc<Mutex<MessagesStore>>,
    transport_runtime: MobileTransportRuntime,
    display_name: Option<String>,
    hub_delivery_hash: Option<String>,
) -> MobileNode {
    let MobileTransportRuntime {
        transport,
        delivery_hash,
        tcp_listen_addresses,
        service_receipt_target,
    } = transport_runtime;
    let transport_active = delivery_hash.is_some();
    let direct_capability_active = service_receipt_target.is_some();
    let mut startup = StartupContractBuilder::production(RuntimeKind::Mobile);
    if transport_active {
        startup.record(startup_component::LXMF_DELIVERY);
        startup.record(startup_component::TRANSPORT_ANNOUNCE_BRIDGE);
        startup.record(startup_component::TRANSPORT_LINK_BRIDGE);
    }
    if direct_capability_active {
        startup.record(startup_component::SERVICE_RECEIPT_BRIDGE);
        startup.record(startup_component::NATIVE_RESOURCE_RETRY_SCHEDULER);
    }
    let identity_hash = hex::encode(identity.address_hash().as_slice());
    let app_context = Arc::new(AppContext::new(transport, identity_hash.clone(), store));
    startup.record_local_execution_services();
    app_context
        .policy()
        .grant(
            styrene_rbac::RosterEntry::new(&identity_hash, styrene_rbac::Role::Admin)
                .with_label("local-mobile-host"),
            app_context.store(),
        )
        .unwrap_or_else(|error| {
            panic!("mobile local authorization initialization failed: {error}")
        });
    if let Some(target) = service_receipt_target
        && target.set(Arc::downgrade(&app_context.messaging_arc())).is_err()
    {
        panic!("mobile service receipt target initialized twice");
    }
    app_context.set_signer(Arc::new(identity));
    if direct_capability_active {
        app_context.publish_standard_propagation(
            crate::standard_propagation::StandardPropagationRuntimeObservation::client(),
        );
    }
    app_context.identity().set_delivery_destination_hash(delivery_hash.clone());
    if let Some(display_name) = display_name.as_deref() {
        app_context.identity().set_identity(Some(display_name), None, None);
    }
    if let Some(hub_hash) = &hub_delivery_hash {
        app_context.messaging().set_propagation_hub(hub_hash.clone(), app_context.fleet_arc());
    }

    let inbound = crate::workers::inbound::spawn_inbound_worker_with_auto_reply(
        app_context.transport_arc(),
        app_context.messaging_arc(),
        app_context.protocol_arc(),
        app_context.events_arc(),
        app_context.propagation_arc(),
        crate::workers::inbound::InboundDestinations::new(delivery_hash, None),
        Some(app_context.auto_reply_arc()),
    );
    startup.record(startup_component::INBOUND_PACKET_WORKER);
    startup.record(startup_component::INBOUND_RESOURCE_WORKER);
    startup.record(startup_component::OUTBOUND_RESOURCE_COMPLETION_WORKER);
    let announce = crate::workers::announce::spawn_announce_worker(
        app_context.transport_arc(),
        app_context.discovery_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ANNOUNCE_WORKER);
    let link = crate::workers::link::spawn_link_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::LINK_WORKER);
    let route = crate::workers::route::spawn_route_worker(
        app_context.transport_arc(),
        app_context.events_arc(),
    );
    startup.record(startup_component::ROUTE_WORKER);
    startup.record(startup_component::NETWORK_OPERATION_COORDINATOR);
    let router_deadlines =
        crate::workers::router::spawn_router_deadline_worker(app_context.messaging_arc());
    startup.record(startup_component::LXMF_ROUTER_DEADLINE_SCHEDULER);
    let standard_propagation_sync = direct_capability_active.then(|| {
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_CLIENT_COORDINATOR);
        startup.record(startup_component::STANDARD_LXMF_PROPAGATION_SYNC_SCHEDULER);
        crate::workers::standard_propagation::spawn_standard_propagation_sync_worker(
            app_context.messaging_arc(),
        )
    });
    let workers = MobileWorkers {
        inbound,
        announce,
        link,
        route,
        router_deadlines,
        standard_propagation_sync,
        aborted: false,
    };
    let facade = Arc::new(DaemonFacade::new(app_context.clone(), identity_hash));

    startup
        .advertise(startup_capability::LOCAL_CONFIG)
        .unwrap_or_else(|error| panic!("invalid mobile local-config startup contract: {error}"));
    startup
        .advertise(startup_capability::LOCAL_POLICY)
        .unwrap_or_else(|error| panic!("invalid mobile local-policy startup contract: {error}"));
    if direct_capability_active {
        startup.record_transport_state_services();
        if let Err(error) = startup.advertise(startup_capability::LXMF_DIRECT) {
            panic!("invalid mobile startup contract: {error}");
        }
        if let Err(error) = startup.advertise(startup_capability::LXMF_PAPER_EXPORT) {
            panic!("invalid mobile paper-export startup contract: {error}");
        }
        if let Err(error) = startup.advertise(startup_capability::NETWORK_OPERATIONS) {
            panic!("invalid mobile network-operation startup contract: {error}");
        }
        for capability in [
            startup_capability::RNS_REQUESTS,
            startup_capability::RNS_REQUEST_CANCELLATION,
            startup_capability::RNS_RESOURCE_CANCELLATION,
        ] {
            startup.advertise(capability).unwrap_or_else(|error| {
                panic!("invalid mobile transport startup contract: {error}")
            });
        }
        startup
            .advertise(startup_capability::STANDARD_LXMF_PROPAGATION_CLIENT)
            .unwrap_or_else(|error| panic!("invalid mobile propagation-client contract: {error}"));
    }
    let startup_contract = startup.finish();
    app_context.publish_startup_contract(startup_contract.clone());
    MobileNode {
        app_context,
        facade,
        paths,
        hub_delivery_hash,
        workers,
        tcp_listen_addresses,
        startup_contract,
    }
}

fn validate_interfaces(config: &MobileConfig) -> anyhow::Result<Vec<ValidatedMobileInterface>> {
    let mut validated =
        Vec::with_capacity(config.interfaces.len() + usize::from(config.hub_address.is_some()));
    for interface in &config.interfaces {
        let normalized = match interface {
            MobileInterfaceConfig::TcpServer { bind_address } => {
                let address = parse_direct_socket("TCP server", bind_address)?;
                ValidatedMobileInterface::TcpServer(address)
            }
            MobileInterfaceConfig::TcpClient { remote_address } => {
                let address = parse_direct_socket("TCP client", remote_address)?;
                ValidatedMobileInterface::TcpClient(address.to_string())
            }
        };
        if validated.contains(&normalized) {
            anyhow::bail!("duplicate mobile interface profile: {normalized:?}");
        }
        validated.push(normalized);
    }
    if let Some(hub_address) = &config.hub_address {
        let address = hub_address.trim();
        if address.is_empty() {
            anyhow::bail!("legacy hub TCP client address is empty");
        }
        let address = address
            .parse::<SocketAddr>()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| address.to_string());
        let profile = ValidatedMobileInterface::TcpClient(address);
        if validated.contains(&profile) {
            anyhow::bail!("duplicate mobile interface profile: {profile:?}");
        }
        validated.push(profile);
    }
    Ok(validated)
}

fn parse_direct_socket(kind: &str, value: &str) -> anyhow::Result<SocketAddr> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{kind} address is empty");
    }
    value.parse().map_err(|error| anyhow::anyhow!("invalid {kind} address '{value}': {error}"))
}

async fn await_tcp_binding(
    mut receiver: tokio::sync::watch::Receiver<Option<SocketAddr>>,
) -> anyhow::Result<SocketAddr> {
    tokio::time::timeout(Duration::from_secs(5), async move {
        loop {
            if let Some(address) = *receiver.borrow() {
                return Ok(address);
            }
            receiver
                .changed()
                .await
                .map_err(|_| anyhow::anyhow!("TCP server stopped before binding"))?;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for TCP server to bind"))?
}

impl MobileNode {
    pub fn startup_contract(&self) -> &StartupContract {
        &self.startup_contract
    }

    pub fn active_capabilities(&self, caller_identity: &str) -> ActiveCapabilities {
        self.startup_contract
            .active_capabilities(self.app_context.policy().authorized_capabilities(caller_identity))
    }

    /// Boot the daemon in-process for mobile use.
    ///
    /// Creates identity if needed, opens SQLite, starts transport.
    /// Does NOT start an IPC server or PTY terminal.
    pub async fn boot(config: MobileConfig) -> anyhow::Result<Self> {
        let interfaces = validate_interfaces(&config)?;
        let paths = PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
        paths.ensure_dirs()?;

        // Load or create identity via the configured backend.
        let identity = load_or_create_identity(&config.identity_backend, &paths)?;

        // Open database
        let db_path = paths.db_path();
        let store = Arc::new(Mutex::new(
            MessagesStore::open(&db_path).map_err(|e| anyhow::anyhow!("database: {e}"))?,
        ));

        let display_name = config.display_name.as_deref().and_then(normalize_display_name);
        let announce_app_data =
            display_name.as_deref().and_then(encode_delivery_display_name_app_data);

        // All configured interfaces share one transport identity and delivery destination.
        let transport_runtime = if !interfaces.is_empty() {
            use rns_core::destination::DestinationName;
            use rns_core::transport::core_transport::{Transport, TransportConfig};
            use rns_core::transport::iface::tcp_client::TcpClient;
            use rns_core::transport::iface::tcp_server::TcpServer;

            let transport_id =
                rns_core::transport::identity_bridge::to_transport_private_identity(&identity);
            let config_t = TransportConfig::new("styrene-mobile", &transport_id, true);
            let mut transport_instance = Transport::new(config_t);
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

            let iface_mgr = transport_instance.iface_manager();
            let mut server_bindings = Vec::new();
            for interface in &interfaces {
                if let ValidatedMobileInterface::TcpServer(bind_address) = interface {
                    let (server, binding) =
                        TcpServer::new(bind_address.to_string(), iface_mgr.clone());
                    iface_mgr.lock().await.spawn(server, TcpServer::spawn);
                    server_bindings.push(binding);
                }
            }
            for interface in &interfaces {
                if let ValidatedMobileInterface::TcpClient(remote_address) = interface {
                    iface_mgr
                        .lock()
                        .await
                        .spawn(TcpClient::new(remote_address.clone()), TcpClient::spawn);
                }
            }

            // Add LXMF delivery destination
            let _destination = transport_instance
                .add_destination(transport_id, DestinationName::new("lxmf", "delivery"))
                .await;

            let transport = Arc::new(transport_instance);
            let mut id_bytes = [0u8; 16];
            id_bytes.copy_from_slice(identity.address_hash().as_slice());

            let delivery_addr = {
                let dest = _destination.lock().await;
                dest.desc.address_hash
            };

            let adapter =
                crate::transport::adapter::TokioTransportAdapter::new_with_packet_receipts(
                    transport.clone(),
                    rns_core::hash::AddressHash::new(id_bytes),
                    delivery_addr,
                    _destination,
                    announce_app_data,
                    packet_receipts.sender(),
                )
                .await;

            let mut bound = Vec::with_capacity(server_bindings.len());
            for receiver in server_bindings {
                match await_tcp_binding(receiver).await {
                    Ok(address) => bound.push(address),
                    Err(error) => {
                        iface_mgr.lock().await.shutdown();
                        return Err(error);
                    }
                }
            }
            MobileTransportRuntime {
                transport: Arc::new(adapter),
                delivery_hash: Some(hex::encode(delivery_addr.as_slice())),
                tcp_listen_addresses: bound,
                service_receipt_target: Some(receipt_target),
            }
        } else {
            MobileTransportRuntime {
                transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
                delivery_hash: None,
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: None,
            }
        };

        Ok(compose_mobile_node(
            paths,
            identity,
            store,
            transport_runtime,
            display_name,
            config.hub_delivery_hash,
        ))
    }

    /// The local LXMF delivery destination, if a transport was configured.
    pub fn delivery_hash(&self) -> Option<String> {
        self.app_context.identity().delivery_destination_hash()
    }

    /// Whether the configured transport is operational.
    pub fn is_connected(&self) -> bool {
        self.app_context.transport().is_connected()
    }

    /// Actual addresses bound by configured TCP server profiles.
    pub fn tcp_listen_addresses(&self) -> &[SocketAddr] {
        &self.tcp_listen_addresses
    }

    /// Stop retained workers and dispatch transport shutdown.
    pub async fn shutdown(
        mut self,
    ) -> Result<(), crate::transport::mesh_transport::TransportError> {
        self.workers.shutdown().await;
        self.app_context.transport().shutdown().await
    }

    /// Poll the propagation hub for queued messages.
    ///
    /// This is the core background task for iOS `BGAppRefreshTask`.
    /// Fetches all queued messages, persists them locally, ACKs the hub.
    /// Returns the count and preview of new messages for local notifications.
    ///
    /// Safe to call from a 30-second background window.
    pub async fn poll_hub(&self) -> Result<PollResult, String> {
        let hub_hash = self.hub_delivery_hash.as_deref().ok_or("no propagation hub configured")?;

        let my_delivery_hash = self
            .app_context
            .identity()
            .delivery_destination_hash()
            .ok_or("identity not configured — no delivery hash")?;

        // Fetch queued messages from hub
        let messages = self
            .app_context
            .fleet()
            .propagation_fetch(hub_hash, &my_delivery_hash, Some(15))
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

        if messages.is_empty() {
            return Ok(PollResult { message_count: 0, messages: Vec::new() });
        }

        let mut poll_messages = Vec::new();

        // Decode and persist each message. Duplicate imports are ACKed at the
        // hub but do not produce another notification.
        for (_id, lxmf_bytes) in &messages {
            match self.app_context.messaging().accept_inbound(
                [0u8; 16], // destination filled by decoder from wire
                lxmf_bytes,
                lxmf::inbound_decode::InboundPayloadMode::FullWire,
            ) {
                InboundAcceptOutcome::Accepted(record) => {
                    poll_messages.push(PollMessage {
                        source_hash: record.source.clone(),
                        content_preview: record.content[..record.content.len().min(100)]
                            .to_string(),
                        timestamp: record.timestamp,
                    });
                }
                InboundAcceptOutcome::Duplicate { message_id } => {
                    self.app_context.events().emit_inbound_drop(
                        "mobile_poll",
                        "duplicate",
                        Some(&message_id),
                        None,
                        None,
                    );
                }
                InboundAcceptOutcome::Rejected { diagnostics } => {
                    self.app_context.events().emit_inbound_drop(
                        "mobile_poll",
                        "malformed",
                        None,
                        None,
                        Some(&diagnostics.summary()),
                    );
                }
                InboundAcceptOutcome::StorageError { message_id, error } => {
                    self.app_context.events().emit_inbound_drop(
                        "mobile_poll",
                        "storage_error",
                        Some(&message_id),
                        None,
                        Some(&error.to_string()),
                    );
                }
            }
        }

        // ACK all fetched messages so hub deletes them
        let ids: Vec<String> = messages.into_iter().map(|(id, _)| id).collect();
        let _ = self.app_context.fleet().propagation_delete(hub_hash, &ids, Some(15)).await;

        let count = poll_messages.len();
        Ok(PollResult { message_count: count, messages: poll_messages })
    }

    /// Send a chat message to a peer.
    pub async fn send_chat(
        &self,
        peer_delivery_hash: &str,
        content: &str,
    ) -> Result<String, String> {
        self.app_context
            .messaging()
            .send_chat(peer_delivery_hash, content, None)
            .await
            .map_err(|e| e.to_string())
    }

    /// List known peers.
    pub async fn list_peers(&self) -> Result<Vec<styrene_ipc::types::DeviceInfo>, String> {
        DaemonStatus::query_devices(self.facade.as_ref(), false).await.map_err(|e| e.to_string())
    }

    /// Query daemon status.
    pub async fn status(&self) -> Result<styrene_ipc::types::DaemonStatusInfo, String> {
        DaemonStatus::query_status(self.facade.as_ref()).await.map_err(|e| e.to_string())
    }

    /// Trigger a mesh announce.
    pub async fn announce(&self) -> Result<(), String> {
        DaemonIdentity::announce(self.facade.as_ref()).await.map(|_| ()).map_err(|e| e.to_string())
    }

    // ── Conversation & Contact Management ───────────────────────────

    /// List conversations with unread counts.
    pub async fn list_conversations(&self) -> Result<Vec<ConversationSummary>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_conversations(self.facade.as_ref(), false)
            .await
            .map(|convos| {
                convos
                    .into_iter()
                    .map(|c| ConversationSummary {
                        peer_hash: c.peer_hash,
                        unread_count: c.unread_count,
                        message_count: c.message_count,
                        last_activity: c.last_message_timestamp.unwrap_or(0),
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    /// Get messages for a specific peer.
    pub async fn get_messages(
        &self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<styrene_ipc::types::MessageInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_messages(self.facade.as_ref(), peer_hash, limit, None)
            .await
            .map_err(|e| e.to_string())
    }

    /// Search messages by content.
    pub async fn search_messages(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<styrene_ipc::types::MessageInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::search_messages(self.facade.as_ref(), query, None, limit)
            .await
            .map_err(|e| e.to_string())
    }

    /// Set a contact alias for a peer.
    pub async fn set_contact(&self, peer_hash: &str, alias: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::set_contact(self.facade.as_ref(), peer_hash, Some(alias), None)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Remove a contact.
    pub async fn remove_contact(&self, peer_hash: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::remove_contact(self.facade.as_ref(), peer_hash)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// List all contacts.
    pub async fn list_contacts(&self) -> Result<Vec<styrene_ipc::types::ContactInfo>, String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::query_contacts(self.facade.as_ref()).await.map_err(|e| e.to_string())
    }

    /// Mark a conversation as read.
    pub async fn mark_read(&self, peer_hash: &str) -> Result<(), String> {
        use styrene_ipc::traits::DaemonMessaging;
        DaemonMessaging::mark_read(self.facade.as_ref(), peer_hash)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Browse a Micron page.
    pub async fn browse_page(&self, host: &str, path: &str) -> Result<String, String> {
        use styrene_ipc::traits::DaemonPages;
        DaemonPages::browse_page(self.facade.as_ref(), host, path, Some(30))
            .await
            .map(|p| String::from_utf8_lossy(&p.source_bytes).into_owned())
            .map_err(|e| e.to_string())
    }

    /// Get platform paths (for diagnostics).
    pub fn paths(&self) -> &PlatformPaths {
        &self.paths
    }

    /// Access the full Daemon trait for advanced operations.
    pub fn daemon(&self) -> &dyn Daemon {
        self.facade.as_ref()
    }
}

/// Conversation summary for mobile UI.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub peer_hash: String,
    pub unread_count: u32,
    pub message_count: u32,
    pub last_activity: i64,
}

// ── Identity Storage Backends ───────────────────────────────────────────────

/// Load or create an RNS identity using the configured backend.
///
/// On first launch, creates a new identity seamlessly — no passphrase prompts
/// on keychain backends, no manual key management. The user just opens the app.
fn load_or_create_identity(
    backend: &IdentityBackend,
    paths: &PlatformPaths,
) -> anyhow::Result<PrivateIdentity> {
    match backend {
        IdentityBackend::Keychain => load_or_create_keychain(paths),
        IdentityBackend::EncryptedFile => load_or_create_encrypted_file(paths),
        IdentityBackend::PlaintextFile => load_or_create_plaintext_file(paths),
    }
}

/// Keychain backend: root secret in platform keychain → HKDF → RNS keys.
///
/// On iOS: Face ID / Touch ID protects access. Zero-interaction on create.
/// On macOS: Keychain Access with biometric. Same behavior.
/// Fallback: if keychain feature not compiled, falls back to plaintext file.
fn load_or_create_keychain(_paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    #[cfg(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")))]
    {
        use styrene_identity::keychain_signer::KeychainSigner;
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let signer = KeychainSigner::default();

        // Create if needed — generates random 32-byte root secret in Keychain.
        // No passphrase prompt, no user interaction. Biometric required on read.
        if !signer.exists() {
            signer.create().map_err(|e| anyhow::anyhow!("keychain create: {e}"))?;
            crate::daemon_diagnostic!("[mobile] created new identity in platform keychain");
        }

        // Retrieve root secret (triggers biometric on iOS)
        let root = tokio::runtime::Handle::current()
            .block_on(signer.root_secret())
            .map_err(|e| anyhow::anyhow!("keychain access: {e}"))?;

        // Derive RNS identity from root secret via HKDF.
        // Construct the 64-byte canonical format: [X25519_secret || Ed25519_secret]
        let deriver = KeyDeriver::new(root.as_bytes());
        let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);
        let signing_seed = deriver.derive(KeyPurpose::Signing);

        let mut key_bytes = [0u8; 64];
        key_bytes[..32].copy_from_slice(&encryption_seed);
        key_bytes[32..].copy_from_slice(&signing_seed);

        PrivateIdentity::from_private_key_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("key derivation: {e:?}"))
    }

    #[cfg(not(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios"))))]
    {
        crate::daemon_diagnostic!(
            "[mobile] keychain feature not enabled, falling back to plaintext file"
        );
        load_or_create_plaintext_file(_paths)
    }
}

/// Encrypted file backend: argon2id + ChaCha20Poly1305 encrypted root secret.
///
/// Requires a passphrase — the host app must provide it via a prompt.
/// Less seamless than keychain but works on any platform.
fn load_or_create_encrypted_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    #[cfg(feature = "mobile-identity")]
    {
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let identity_path = paths.identity_path();

        let signer = styrene_identity::file_signer::FileSigner::new(
            identity_path.clone(),
            Box::new(styrene_identity::file_signer::StaticPassphraseProvider::new(b"")),
        );

        // FileSigner auto-creates on first root_secret() if file doesn't exist
        let root = tokio::runtime::Handle::current()
            .block_on(signer.root_secret())
            .map_err(|e| anyhow::anyhow!("encrypted file access: {e}"))?;

        if !identity_path.exists() {
            crate::daemon_diagnostic!(
                "[mobile] created new encrypted identity at {}",
                identity_path.display()
            );
        }

        let deriver = KeyDeriver::new(root.as_bytes());
        let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);
        let signing_seed = deriver.derive(KeyPurpose::Signing);

        let mut key_bytes = [0u8; 64];
        key_bytes[..32].copy_from_slice(&encryption_seed);
        key_bytes[32..].copy_from_slice(&signing_seed);

        PrivateIdentity::from_private_key_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("key derivation: {e:?}"))
    }

    #[cfg(not(feature = "mobile-identity"))]
    {
        crate::daemon_diagnostic!(
            "[mobile] file-signer feature not enabled, falling back to plaintext"
        );
        load_or_create_plaintext_file(paths)
    }
}

/// Plaintext file backend: 64-byte raw identity on disk.
///
/// For development and testing only. NOT secure for production mobile.
fn load_or_create_plaintext_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    let identity_path = paths.identity_path();

    if identity_path.exists() {
        let bytes = std::fs::read(&identity_path)?;
        PrivateIdentity::from_private_key_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("invalid identity: {e:?}"))
    } else {
        // Generate deterministic-ish identity for new installs
        let id = PrivateIdentity::new_from_name(&format!(
            "styrene-mobile-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&identity_path, id.to_private_key_bytes())?;
        crate::daemon_diagnostic!(
            "[mobile] created new plaintext identity at {}",
            identity_path.display()
        );
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mesh_transport::TransportLifecycleEvent;
    use crate::transport::mock_transport::{MockCall, MockTransport};
    use rns_core::hash::AddressHash;
    use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};
    use styrene_ipc::types::DaemonEvent;
    use tokio::time::{Duration, timeout};

    fn compose_with_mock(mock: Arc<MockTransport>, display_name: Option<String>) -> MobileNode {
        let identity = PrivateIdentity::new_from_name("mobile-node-test");
        let delivery_hash = hex::encode(mock.destination_hash().as_slice());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        compose_mobile_node(
            PlatformPaths::new("test-config".into(), "test-data".into()),
            identity,
            store,
            MobileTransportRuntime {
                transport: mock,
                delivery_hash: Some(delivery_hash),
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: None,
            },
            display_name,
            None,
        )
    }

    #[tokio::test]
    async fn composition_publishes_metadata_and_starts_link_worker() {
        let destination = AddressHash::new([7; 16]);
        let mock = Arc::new(MockTransport::new(AddressHash::new([3; 16]), destination));
        let node = compose_with_mock(mock.clone(), Some("Classroom Yellow".into()));
        let mut links = node.app_context.events().subscribe_links();

        assert_eq!(node.delivery_hash(), Some(hex::encode(destination.as_slice())));
        assert_eq!(node.app_context.identity().display_name().as_deref(), Some("Classroom Yellow"));
        assert!(node.startup_contract().has_component(startup_component::ROUTE_WORKER));
        assert!(!node.workers.all_finished());

        mock.inject_lifecycle(TransportLifecycleEvent::LinkActivated {
            link_id: "1234567890abcdef".into(),
            peer_hash: "fedcba0987654321fedcba0987654321".into(),
            interface: Some("mobile-interface".into()),
            rtt_ms: 12.5,
        });
        let event = timeout(Duration::from_secs(1), links.recv()).await.unwrap().unwrap();
        assert!(matches!(
            event,
            DaemonEvent::Link { event }
                if event.status == "active" && event.rtt_ms == Some(12.5)
        ));
    }

    #[tokio::test]
    async fn composition_processes_inbound_lxmf_with_retained_worker() {
        let destination = [7_u8; 16];
        let source = [9_u8; 16];
        let mock =
            Arc::new(MockTransport::new(AddressHash::new([3; 16]), AddressHash::new(destination)));
        let node = compose_with_mock(mock.clone(), None);
        let mut events = node.app_context.events().subscribe();
        let payload = rmp_serde::to_vec(&rmpv::Value::Array(vec![
            rmpv::Value::from(1_770_000_000_i64),
            rmpv::Value::from(""),
            rmpv::Value::from("mobile inbound"),
            rmpv::Value::Nil,
        ]))
        .unwrap();
        let mut wire = Vec::new();
        wire.extend_from_slice(&destination);
        wire.extend_from_slice(&source);
        wire.extend_from_slice(&[0x33; 64]);
        wire.extend_from_slice(&payload);

        mock.inject_inbound(ReceivedData {
            destination: AddressHash::new(destination),
            link_id: None,
            data: rns_core::packet::PacketDataBuffer::new_from_slice(&wire),
            payload_mode: ReceivedPayloadMode::FullWire,
            ratchet_used: false,
            context: None,
            request_id: None,
            hops: None,
            interface: None,
        });

        let event = timeout(Duration::from_secs(1), events.recv()).await.unwrap().unwrap();
        assert_eq!(event.event_type, "message_received");
        let messages = node.app_context.store().lock().unwrap().list_messages(10, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "mobile inbound");
    }

    #[tokio::test]
    async fn explicit_shutdown_aborts_workers_and_dispatches_transport_shutdown_once() {
        let mock = Arc::new(MockTransport::new_default());
        let mut node = compose_with_mock(mock.clone(), None);

        node.workers.abort();
        tokio::task::yield_now().await;
        assert!(node.workers.all_finished());

        node.shutdown().await.unwrap();
        let shutdowns =
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count();
        assert_eq!(shutdowns, 1);
    }

    #[tokio::test]
    async fn dropping_node_aborts_every_retained_worker() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None);
        let handles = node.workers.abort_handles();

        drop(node);
        tokio::task::yield_now().await;

        assert!(handles.iter().all(tokio::task::AbortHandle::is_finished));
    }
}
