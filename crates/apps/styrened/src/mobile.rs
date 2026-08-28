//! Mobile embedding — lightweight daemon boot and background poll.
//!
//! Provides the in-process daemon API for the shared Rust/Dioxus mobile app.
//! There is no IPC server, PTY terminal, or Unix socket in this composition.
//!
//! # Usage
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
//! The application should boot one node, retain it for the session, and route
//! foreground and best-effort background operations through that owner.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use crate::app_context::AppContext;
use crate::config::{atomic_write_private, PlatformPaths};
use crate::daemon_facade::DaemonFacade;
use crate::services::discovery::{
    LXMF_DELIVERY_DEVICE_TYPE, NATIVE_NOMADNET_HOST_DEVICE_TYPE,
    STANDARD_LXMF_PROPAGATION_ACTIVE_DEVICE_TYPE, STANDARD_LXMF_PROPAGATION_INACTIVE_DEVICE_TYPE,
};
use crate::services::messaging::InboundAcceptOutcome;
use crate::startup_contract::{
    ActiveCapabilities, RuntimeKind, StartupContract, StartupContractBuilder,
    capabilities as startup_capability, components as startup_component,
};
use crate::storage::messages::MessagesStore;
use crate::transport::mesh_transport::MeshTransport;

use rns_core::buffer::InputBuffer;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::packet::Packet;
use rns_core::transport::iface::{
    InterfaceChannel, InterfaceKind, InterfaceRxSender, InterfaceState, InterfaceTxReceiver,
    RxMessage,
};
use serde::{Deserialize, Serialize};
use styrene_ipc::traits::{Daemon, DaemonIdentity, DaemonStatus};
use styrene_services::node_store::NodeStore;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
    /// Create a host-driven channel for an Android-owned RNode interface.
    pub enable_rnode_channel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileInterfaceConfig {
    TcpServer { bind_address: String },
    TcpClient { remote_address: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileConnectionPhase {
    Stopped,
    Starting,
    Connecting,
    Connected,
    Reconnecting,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBearerKind {
    Tcp,
    BluetoothRnode,
    AndroidUsb,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBearerState {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
    Unavailable,
    Unverified,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileFailureCode {
    InvalidTcpEndpoint,
    TcpRetrying,
    TransportUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum MobileEndpointError {
    #[error("{message}")]
    Invalid { message: String },
    #[error("persist mobile TCP endpoint: {message}")]
    Persistence { message: String },
}

impl MobileEndpointError {
    #[must_use]
    pub const fn code(&self) -> MobileFailureCode {
        match self {
            Self::Invalid { .. } => MobileFailureCode::InvalidTcpEndpoint,
            Self::Persistence { .. } => MobileFailureCode::TransportUnavailable,
        }
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileFailure {
    pub code: MobileFailureCode,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileBearerObservation {
    pub kind: MobileBearerKind,
    pub state: MobileBearerState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileSessionSnapshot {
    pub phase: MobileConnectionPhase,
    pub endpoint: Option<String>,
    pub generation: u64,
    pub failure: Option<MobileFailure>,
    pub bearers: Vec<MobileBearerObservation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePeerAspect {
    LxmfDelivery,
    LxmfPropagation,
    NomadNetworkNode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePeerSource {
    CanonicalAnnounce,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePeer {
    pub destination_hash: String,
    pub aspect: MobilePeerAspect,
    pub display_name: Option<String>,
    pub observed_at: i64,
    pub age_secs: u64,
    pub source: MobilePeerSource,
    pub announce_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePeerSnapshot {
    pub generation: u64,
    pub observed_at: i64,
    pub peers: Vec<MobilePeer>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePeerEvent {
    pub generation: u64,
    pub peer: MobilePeer,
}

pub struct MobilePeerSubscription {
    generation: u64,
    receiver: tokio::sync::broadcast::Receiver<styrene_ipc::types::DaemonEvent>,
}

impl MobilePeerSubscription {
    pub async fn recv(&mut self) -> Result<MobilePeerEvent, MobileDiscoveryError> {
        loop {
            match self.receiver.recv().await {
                Ok(styrene_ipc::types::DaemonEvent::Device { device }) => {
                    if let Some(peer) = mobile_peer_from_device(
                        device,
                        rns_core::transport::time::now_epoch_secs_i64(),
                    ) {
                        return Ok(MobilePeerEvent { generation: self.generation, peer });
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    return Err(MobileDiscoveryError::EventLagged(dropped));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(MobileDiscoveryError::EventClosed);
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MobileDiscoveryError {
    #[error("mobile discovery snapshot failed: {0}")]
    Snapshot(String),
    #[error("mobile discovery event stream lagged by {0} events")]
    EventLagged(u64),
    #[error("mobile discovery event stream closed")]
    EventClosed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileAnnounceOutcome {
    pub generation: u64,
    pub accepted_at: i64,
    pub local_dispatch_accepted: bool,
    pub remote_reception_confirmed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MobileAnnounceError {
    #[error("mobile announce transport unavailable")]
    TransportUnavailable,
    #[error("mobile announce dispatch rejected: {0}")]
    DispatchRejected(String),
}

impl MobileAnnounceError {
    #[must_use]
    pub const fn code(&self) -> MobileFailureCode {
        MobileFailureCode::TransportUnavailable
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        true
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedMobileConfig {
    schema_version: u32,
    tcp_endpoint: String,
}

const MOBILE_CONFIG_SCHEMA_VERSION: u32 = 1;

impl MobileSessionSnapshot {
    #[must_use]
    pub fn bearer(&self, kind: MobileBearerKind) -> Option<&MobileBearerObservation> {
        self.bearers.iter().find(|bearer| bearer.kind == kind)
    }
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
    workers: Mutex<Option<MobileWorkers>>,
    tcp_listen_addresses: Vec<SocketAddr>,
    startup_contract: StartupContract,
    rnode: Option<RNodeBridge>,
    tcp_endpoint: Option<String>,
    generation: u64,
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
    rnode_channel: Option<InterfaceChannel>,
}

struct RNodeBridge {
    address: AddressHash,
    rx: InterfaceRxSender,
    tx: AsyncMutex<InterfaceTxReceiver>,
    _stop: CancellationToken,
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
        if let Ok(workers) = self.workers.get_mut() {
            if let Some(workers) = workers.as_mut() {
                workers.abort();
            }
        }
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

struct MobileStores {
    messages: Arc<Mutex<MessagesStore>>,
    nodes: Arc<NodeStore>,
}

async fn compose_mobile_node(
    paths: PlatformPaths,
    identity: PrivateIdentity,
    stores: MobileStores,
    transport_runtime: MobileTransportRuntime,
    display_name: Option<String>,
    hub_delivery_hash: Option<String>,
    tcp_endpoint: Option<String>,
) -> anyhow::Result<MobileNode> {
    let MobileTransportRuntime {
        transport,
        delivery_hash,
        tcp_listen_addresses,
        service_receipt_target,
        rnode_channel,
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
    let app_context = Arc::new(AppContext::with_node_store(
        transport.clone(),
        identity_hash.clone(),
        stores.messages,
        stores.nodes,
    ));
    startup.record_local_execution_services();
    let initialization = (|| -> anyhow::Result<()> {
        app_context
            .policy()
            .grant(
                styrene_rbac::RosterEntry::new(&identity_hash, styrene_rbac::Role::Admin)
                    .with_label("local-mobile-host"),
                app_context.store(),
            )
            .map_err(|error| {
                anyhow::anyhow!("mobile local authorization initialization failed: {error}")
            })?;
        if let Some(target) = &service_receipt_target {
            target
                .set(Arc::downgrade(&app_context.messaging_arc()))
                .map_err(|_| anyhow::anyhow!("mobile service receipt target initialized twice"))?;
        }
        Ok(())
    })();
    if let Err(error) = initialization {
        if let Err(shutdown_error) = transport.shutdown().await {
            return Err(anyhow::anyhow!(
                "{error}; mobile transport cleanup failed: {shutdown_error}"
            ));
        }
        return Err(error);
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
    let mut workers = MobileWorkers {
        inbound,
        announce,
        link,
        route,
        router_deadlines,
        standard_propagation_sync,
        aborted: false,
    };
    let facade = Arc::new(DaemonFacade::new(app_context.clone(), identity_hash));

    let startup_contract = (|| -> anyhow::Result<StartupContract> {
        startup.advertise(startup_capability::LOCAL_CONFIG).map_err(|error| {
            anyhow::anyhow!("invalid mobile local-config startup contract: {error}")
        })?;
        startup.advertise(startup_capability::LOCAL_POLICY).map_err(|error| {
            anyhow::anyhow!("invalid mobile local-policy startup contract: {error}")
        })?;
        if direct_capability_active {
            startup.record_transport_state_services();
            for capability in [
                startup_capability::LXMF_DIRECT,
                startup_capability::LXMF_PAPER_EXPORT,
                startup_capability::NETWORK_OPERATIONS,
                startup_capability::RNS_REQUESTS,
                startup_capability::RNS_REQUEST_CANCELLATION,
                startup_capability::RNS_RESOURCE_CANCELLATION,
                startup_capability::STANDARD_LXMF_PROPAGATION_CLIENT,
            ] {
                startup.advertise(capability).map_err(|error| {
                    anyhow::anyhow!("invalid mobile startup contract for {capability:?}: {error}")
                })?;
            }
        }
        Ok(startup.finish())
    })();
    let startup_contract = match startup_contract {
        Ok(contract) => contract,
        Err(error) => {
            workers.abort();
            if let Err(shutdown_error) = transport.shutdown().await {
                return Err(anyhow::anyhow!(
                    "{error}; mobile transport cleanup failed: {shutdown_error}"
                ));
            }
            return Err(error);
        }
    };
    app_context.publish_startup_contract(startup_contract.clone());
    let rnode = rnode_channel.map(|channel| RNodeBridge {
        address: channel.address,
        rx: channel.rx_channel,
        tx: AsyncMutex::new(channel.tx_channel),
        _stop: channel.stop,
    });

    Ok(MobileNode {
        app_context,
        facade,
        paths,
        hub_delivery_hash,
        workers: Mutex::new(Some(workers)),
        tcp_listen_addresses,
        startup_contract,
        rnode,
        tcp_endpoint,
        generation: 1,
    })
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
                ValidatedMobileInterface::TcpClient(parse_tcp_client_address(remote_address)?)
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

fn parse_tcp_client_address(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("TCP client address is empty");
    }
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok(address.to_string());
    }

    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid TCP client address '{value}': missing port"))?;
    let valid_host = !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if !valid_host {
        anyhow::bail!("invalid TCP client address '{value}': invalid hostname");
    }
    let port = port
        .parse::<u16>()
        .map_err(|error| anyhow::anyhow!("invalid TCP client address '{value}': {error}"))?;
    Ok(format!("{}:{port}", host.to_ascii_lowercase()))
}

fn mobile_peer_from_device(
    device: styrene_ipc::types::DeviceInfo,
    snapshot_observed_at: i64,
) -> Option<MobilePeer> {
    let aspect = match device.device_type.as_str() {
        LXMF_DELIVERY_DEVICE_TYPE => MobilePeerAspect::LxmfDelivery,
        STANDARD_LXMF_PROPAGATION_ACTIVE_DEVICE_TYPE
        | STANDARD_LXMF_PROPAGATION_INACTIVE_DEVICE_TYPE => MobilePeerAspect::LxmfPropagation,
        NATIVE_NOMADNET_HOST_DEVICE_TYPE => MobilePeerAspect::NomadNetworkNode,
        _ => return None,
    };
    let observed_at = device.last_announce?;
    Some(MobilePeer {
        destination_hash: device.destination_hash,
        aspect,
        display_name: (!device.name.is_empty()).then_some(device.name),
        observed_at,
        age_secs: u64::try_from(snapshot_observed_at.saturating_sub(observed_at)).unwrap_or(0),
        source: MobilePeerSource::CanonicalAnnounce,
        announce_count: device.announce_count,
    })
}

pub fn load_mobile_tcp_endpoint(
    config_dir: &std::path::Path,
) -> Result<Option<String>, MobileEndpointError> {
    let path = config_dir.join("mobile.toml");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(MobileEndpointError::Persistence {
                message: format!("read {}: {error}", path.display()),
            });
        }
    };
    let persisted: PersistedMobileConfig = toml::from_str(&contents).map_err(|error| {
        MobileEndpointError::Invalid { message: format!("parse {}: {error}", path.display()) }
    })?;
    if persisted.schema_version != MOBILE_CONFIG_SCHEMA_VERSION {
        return Err(MobileEndpointError::Invalid {
            message: format!(
                "unsupported mobile config schema {} in {}",
                persisted.schema_version,
                path.display()
            ),
        });
    }
    parse_tcp_client_address(&persisted.tcp_endpoint)
        .map(Some)
        .map_err(|error| MobileEndpointError::Invalid { message: error.to_string() })
}

pub fn persist_mobile_tcp_endpoint(
    config_dir: &std::path::Path,
    endpoint: &str,
) -> Result<String, MobileEndpointError> {
    let endpoint = parse_tcp_client_address(endpoint)
        .map_err(|error| MobileEndpointError::Invalid { message: error.to_string() })?;
    let persisted = PersistedMobileConfig {
        schema_version: MOBILE_CONFIG_SCHEMA_VERSION,
        tcp_endpoint: endpoint.clone(),
    };
    let encoded = toml::to_string_pretty(&persisted).map_err(|error| {
        MobileEndpointError::Persistence { message: format!("encode config: {error}") }
    })?;
    let path = config_dir.join("mobile.toml");
    atomic_write_private(&path, encoded.as_bytes()).map_err(|error| {
        MobileEndpointError::Persistence { message: format!("write {}: {error}", path.display()) }
    })?;
    Ok(endpoint)
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
        let mut interfaces = validate_interfaces(&config)?;
        let paths = PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
        paths.ensure_dirs()?;
        let explicit_tcp_endpoints = interfaces
            .iter()
            .filter_map(|interface| match interface {
                ValidatedMobileInterface::TcpClient(endpoint) => Some(endpoint.clone()),
                ValidatedMobileInterface::TcpServer(_) => None,
            })
            .collect::<Vec<_>>();
        let tcp_endpoint = match explicit_tcp_endpoints.as_slice() {
            [] => load_mobile_tcp_endpoint(&paths.config_dir)?,
            [endpoint] => Some(persist_mobile_tcp_endpoint(&paths.config_dir, endpoint)?),
            _ => explicit_tcp_endpoints.first().cloned(),
        };
        if explicit_tcp_endpoints.is_empty() {
            if let Some(endpoint) = &tcp_endpoint {
                interfaces.push(ValidatedMobileInterface::TcpClient(endpoint.clone()));
            }
        }

        // Load or create identity via the configured backend.
        let identity = load_or_create_identity(&config.identity_backend, &paths).await?;

        // Open database
        let db_path = paths.db_path();
        let store = Arc::new(Mutex::new(
            MessagesStore::open(&db_path).map_err(|e| anyhow::anyhow!("database: {e}"))?,
        ));
        let node_store_path = db_path.with_file_name("nodes.db");
        let node_store_path = node_store_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("mobile node store path is not valid UTF-8"))?;
        let node_store = Arc::new(NodeStore::open(node_store_path)?);

        let display_name = config.display_name.as_deref().and_then(normalize_display_name);
        let announce_app_data =
            display_name.as_deref().and_then(encode_delivery_display_name_app_data);

        // Host-driven RNode and TCP profiles share one transport identity and destination.
        let transport_runtime = if config.enable_rnode_channel || !interfaces.is_empty() {
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
            let rnode_channel = if config.enable_rnode_channel {
                Some(iface_mgr.lock().await.new_channel(128))
            } else {
                None
            };
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
                rnode_channel,
            }
        } else {
            MobileTransportRuntime {
                transport: Arc::new(crate::transport::null_transport::NullTransport::new()),
                delivery_hash: None,
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: None,
                rnode_channel: None,
            }
        };

        compose_mobile_node(
            paths,
            identity,
            MobileStores { messages: store, nodes: node_store },
            transport_runtime,
            display_name,
            config.hub_delivery_hash,
            tcp_endpoint,
        )
        .await
    }

    /// The local LXMF delivery destination, if a transport was configured.
    pub fn delivery_hash(&self) -> Option<String> {
        self.app_context.identity().delivery_destination_hash()
    }

    /// Whether the configured transport is operational.
    pub fn is_connected(&self) -> bool {
        self.app_context.transport().is_connected()
    }

    pub async fn session_snapshot(&self) -> MobileSessionSnapshot {
        let interfaces = self.app_context.transport().interface_snapshots().await;
        let generation = interfaces
            .iter()
            .map(|interface| interface.generation)
            .max()
            .unwrap_or(self.generation)
            .max(1);
        let tcp_states = interfaces
            .iter()
            .filter(|interface| {
                matches!(interface.kind, InterfaceKind::TcpClient | InterfaceKind::TcpServer)
            })
            .map(|interface| interface.state)
            .collect::<Vec<_>>();
        let tcp = if tcp_states.iter().any(|state| {
            matches!(
                state,
                InterfaceState::Listening | InterfaceState::Connected | InterfaceState::Active
            )
        }) {
            MobileBearerState::Connected
        } else if tcp_states.contains(&InterfaceState::Retrying) {
            MobileBearerState::Reconnecting
        } else if tcp_states
            .iter()
            .any(|state| matches!(state, InterfaceState::Starting | InterfaceState::Connecting))
        {
            MobileBearerState::Connecting
        } else {
            MobileBearerState::Unavailable
        };
        let rnode = if self.rnode.is_some() {
            MobileBearerState::Unverified
        } else {
            MobileBearerState::Unavailable
        };
        let failure = (tcp == MobileBearerState::Reconnecting)
            .then_some(MobileFailure { code: MobileFailureCode::TcpRetrying, retryable: true });
        let phase = match tcp {
            MobileBearerState::Connected => MobileConnectionPhase::Connected,
            MobileBearerState::Connecting => MobileConnectionPhase::Connecting,
            MobileBearerState::Reconnecting => MobileConnectionPhase::Reconnecting,
            MobileBearerState::Unavailable if rnode == MobileBearerState::Unverified => {
                MobileConnectionPhase::Degraded
            }
            MobileBearerState::Unavailable
            | MobileBearerState::Unverified
            | MobileBearerState::Disconnected => MobileConnectionPhase::Stopped,
        };
        MobileSessionSnapshot {
            phase,
            endpoint: self.tcp_endpoint.clone(),
            generation,
            failure,
            bearers: vec![
                MobileBearerObservation { kind: MobileBearerKind::Tcp, state: tcp },
                MobileBearerObservation { kind: MobileBearerKind::BluetoothRnode, state: rnode },
                MobileBearerObservation {
                    kind: MobileBearerKind::AndroidUsb,
                    state: MobileBearerState::Unavailable,
                },
            ],
        }
    }

    pub async fn peer_snapshot_at(
        &self,
        observed_at: i64,
    ) -> Result<MobilePeerSnapshot, MobileDiscoveryError> {
        let generation = self.session_snapshot().await.generation;
        let devices = self.list_peers().await.map_err(MobileDiscoveryError::Snapshot)?;
        let peers = devices
            .into_iter()
            .filter_map(|device| mobile_peer_from_device(device, observed_at))
            .collect();
        Ok(MobilePeerSnapshot { generation, observed_at, peers })
    }

    pub async fn peer_snapshot(&self) -> Result<MobilePeerSnapshot, MobileDiscoveryError> {
        self.peer_snapshot_at(rns_core::transport::time::now_epoch_secs_i64()).await
    }

    pub async fn subscribe_peer_events(&self) -> MobilePeerSubscription {
        MobilePeerSubscription {
            generation: self.session_snapshot().await.generation,
            receiver: self.app_context.events().subscribe_devices(),
        }
    }

    pub async fn announce_outcome(&self) -> Result<MobileAnnounceOutcome, MobileAnnounceError> {
        if !self.is_connected() {
            return Err(MobileAnnounceError::TransportUnavailable);
        }
        self.app_context
            .transport()
            .dispatch_announce(None)
            .await
            .map_err(|error| MobileAnnounceError::DispatchRejected(error.to_string()))?;
        Ok(MobileAnnounceOutcome {
            generation: self.session_snapshot().await.generation,
            accepted_at: rns_core::transport::time::now_epoch_secs_i64(),
            local_dispatch_accepted: true,
            remote_reception_confirmed: false,
        })
    }

    /// Actual addresses bound by configured TCP server profiles.
    pub fn tcp_listen_addresses(&self) -> &[SocketAddr] {
        &self.tcp_listen_addresses
    }

    /// Stop retained workers and dispatch transport shutdown.
    pub async fn shutdown(&self) -> Result<(), crate::transport::mesh_transport::TransportError> {
        let mut workers = self.workers.lock().ok().and_then(|mut workers| workers.take());
        if let Some(workers) = workers.as_mut() {
            workers.shutdown().await;
        }
        self.app_context.transport().shutdown().await
    }

    /// Submit unframed RNS bytes received from an Android-owned RNode.
    pub async fn submit_rnode_packet(&self, packet: &[u8]) -> Result<(), String> {
        if packet.first().is_some_and(|byte| byte & 0x80 != 0) {
            return Err("IFAC packet received on open RNode interface".to_string());
        }
        let packet = Packet::deserialize(&mut InputBuffer::new(packet))
            .map_err(|error| format!("invalid RNS packet ({} bytes): {error:?}", packet.len()))?;

        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        rnode
            .rx
            .send(RxMessage { address: rnode.address, packet })
            .await
            .map_err(|_| "RNode receive channel closed".to_string())
    }

    /// Poll the next unframed RNS packet destined for the Android-owned RNode.
    pub async fn poll_rnode_packet(&self) -> Result<Option<Vec<u8>>, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        match rnode.tx.lock().await.try_recv() {
            Ok(message) => message
                .packet
                .to_bytes()
                .map(Some)
                .map_err(|error| format!("RNS packet serialization failed: {error:?}")),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err("RNode transmit channel closed".to_string())
            }
        }
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
async fn load_or_create_identity(
    backend: &IdentityBackend,
    paths: &PlatformPaths,
) -> anyhow::Result<PrivateIdentity> {
    match backend {
        IdentityBackend::Keychain => load_or_create_keychain(paths).await,
        IdentityBackend::EncryptedFile => load_or_create_encrypted_file(paths).await,
        IdentityBackend::PlaintextFile => load_or_create_plaintext_file(paths),
    }
}

/// Keychain backend: root secret in platform keychain → HKDF → RNS keys.
///
/// On iOS: Face ID / Touch ID protects access. Zero-interaction on create.
/// On macOS: Keychain Access with biometric. Same behavior.
/// Fallback: if keychain feature not compiled, falls back to plaintext file.
async fn load_or_create_keychain(_paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
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
        let root =
            signer.root_secret().await.map_err(|e| anyhow::anyhow!("keychain access: {e}"))?;

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
async fn load_or_create_encrypted_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    #[cfg(feature = "mobile-identity")]
    {
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let identity_path = paths.identity_path();

        let signer = styrene_identity::file_signer::FileSigner::new(
            identity_path.clone(),
            Box::new(styrene_identity::file_signer::StaticPassphraseProvider::new(b"")),
        );

        // FileSigner auto-creates on first root_secret() if file doesn't exist
        let root = signer
            .root_secret()
            .await
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

    async fn compose_with_mock(
        mock: Arc<MockTransport>,
        display_name: Option<String>,
    ) -> MobileNode {
        let identity = PrivateIdentity::new_from_name("mobile-node-test");
        let delivery_hash = hex::encode(mock.destination_hash().as_slice());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        compose_mobile_node(
            PlatformPaths::new("test-config".into(), "test-data".into()),
            identity,
            MobileStores { messages: store, nodes: Arc::new(NodeStore::in_memory().unwrap()) },
            MobileTransportRuntime {
                transport: mock,
                delivery_hash: Some(delivery_hash),
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: None,
                rnode_channel: None,
            },
            display_name,
            None,
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn failed_composition_shuts_down_transport_before_returning() {
        let mock = Arc::new(MockTransport::new_default());
        let receipt_target = Arc::new(std::sync::OnceLock::new());
        assert!(receipt_target.set(std::sync::Weak::new()).is_ok());
        let result = compose_mobile_node(
            PlatformPaths::new("test-config".into(), "test-data".into()),
            PrivateIdentity::new_from_name("failed-mobile-composition"),
            MobileStores {
                messages: Arc::new(Mutex::new(MessagesStore::in_memory().unwrap())),
                nodes: Arc::new(NodeStore::in_memory().unwrap()),
            },
            MobileTransportRuntime {
                transport: mock.clone(),
                delivery_hash: Some(hex::encode(mock.destination_hash().as_slice())),
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: Some(receipt_target),
                rnode_channel: None,
            },
            None,
            None,
            None,
        )
        .await;

        let error = result.err().expect("composition must reject a reused receipt target");
        assert!(error.to_string().contains("receipt target initialized twice"));
        let shutdowns =
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count();
        assert_eq!(shutdowns, 1);
    }

    #[tokio::test]
    async fn composition_publishes_metadata_and_starts_link_worker() {
        let destination = AddressHash::new([7; 16]);
        let mock = Arc::new(MockTransport::new(AddressHash::new([3; 16]), destination));
        let node = compose_with_mock(mock.clone(), Some("Classroom Yellow".into())).await;
        let mut links = node.app_context.events().subscribe_links();

        assert_eq!(node.delivery_hash(), Some(hex::encode(destination.as_slice())));
        assert_eq!(node.app_context.identity().display_name().as_deref(), Some("Classroom Yellow"));
        assert!(node.startup_contract().has_component(startup_component::ROUTE_WORKER));
        assert!(!node.workers.lock().unwrap().as_ref().unwrap().all_finished());

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
        let node = compose_with_mock(mock.clone(), None).await;
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
        let node = compose_with_mock(mock.clone(), None).await;

        node.workers.lock().unwrap().as_mut().unwrap().abort();
        tokio::task::yield_now().await;
        assert!(node.workers.lock().unwrap().as_ref().unwrap().all_finished());

        node.shutdown().await.unwrap();
        let shutdowns =
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count();
        assert_eq!(shutdowns, 1);
    }

    #[tokio::test]
    async fn dropping_node_aborts_every_retained_worker() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None).await;
        let handles = node.workers.lock().unwrap().as_ref().unwrap().abort_handles();

        drop(node);
        tokio::task::yield_now().await;

        assert!(handles.iter().all(tokio::task::AbortHandle::is_finished));
    }

    #[tokio::test]
    async fn boot_publishes_delivery_hash_and_announce() {
        let root = tempfile::tempdir().unwrap();
        let node = MobileNode::boot(MobileConfig {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: Some("test mobile".into()),
            identity_backend: IdentityBackend::PlaintextFile,
            interfaces: Vec::new(),
            enable_rnode_channel: true,
        })
        .await
        .unwrap();

        let delivery_hash = node.app_context.identity().delivery_destination_hash().unwrap();
        assert_eq!(delivery_hash.len(), 32);

        node.announce().await.unwrap();
        let packet = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Some(packet) = node.poll_rnode_packet().await.unwrap() {
                    break packet;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!packet.is_empty());
    }

    #[tokio::test]
    async fn oversized_rnode_packet_reports_wire_length() {
        let root = tempfile::tempdir().unwrap();
        let node = MobileNode::boot(MobileConfig {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: Some("test mobile".into()),
            identity_backend: IdentityBackend::PlaintextFile,
            interfaces: Vec::new(),
            enable_rnode_channel: true,
        })
        .await
        .unwrap();

        let error = node.submit_rnode_packet(&[0; 484]).await.unwrap_err();
        assert_eq!(error, "invalid RNS packet (484 bytes): OutOfMemory");
    }
}
