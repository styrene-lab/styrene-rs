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
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::announce_names::{encode_delivery_display_name_app_data, normalize_display_name};
use crate::app_context::AppContext;
use crate::config::{PlatformPaths, atomic_write_private};
use crate::daemon_facade::{DaemonFacade, SessionGeneration};
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
use crate::transport::mesh_transport::{MeshTransport, TransportLifecycleEvent};

use rns_core::buffer::InputBuffer;
use rns_core::hash::AddressHash;
use rns_core::identity::PrivateIdentity;
use rns_core::packet::Packet;
pub use rns_core::transport::iface::rnode::{RNodeBearerInfo, RNodeBearerKind};
use rns_core::transport::iface::rnode::{
    RNodeProtocol, RNodeProtocolPhase, RNodeRadioProfile, rnode_write_chunks,
};
use rns_core::transport::iface::{
    HostInterfaceControl, IngressEnqueueOutcome, InterfaceChannel, InterfaceDescriptor,
    InterfaceKind, InterfaceRxSender, InterfaceState, InterfaceTxReceiver, RxMessage,
};
use serde::{Deserialize, Serialize};
use styrene_ipc::traits::{Daemon, DaemonIdentity, DaemonMessaging, DaemonStatus};
use styrene_services::node_store::NodeStore;
#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Mobile node configuration — provided by the host app.
/// How to store the identity private keys.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum IdentityBackend {
    /// Device-bound platform Keychain, available after the first unlock following restart.
    /// Root secret stored in Secure Enclave, RNS keys derived via HKDF.
    /// Requires `mobile-keychain` feature.
    #[default]
    Keychain,
    /// Random root secret wrapped by a non-exportable Android Keystore key.
    /// Requires `mobile-android-keystore` feature.
    AndroidKeystore,
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
    /// Create a host-driven channel for one mobile RNode bearer.
    pub enable_rnode_channel: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MobileCustodyError {
    #[error("{backend} identity backend is unavailable in this build")]
    BackendUnavailable { backend: &'static str },
    #[error("encrypted-file identity backend requires nonempty host key material")]
    KeyMaterialRequired,
}

/// Maximum opaque identity artifact accepted from a platform file picker.
pub const MAX_MOBILE_IDENTITY_BACKUP_BYTES: usize = 4096;
/// Maximum UTF-8 protection input retained for one backup operation.
pub const MAX_MOBILE_IDENTITY_PROTECTION_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileIdentityPresence {
    Present,
    Absent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MobileIdentityRecoveryError {
    #[error("identity backup protection is required")]
    ProtectionRequired,
    #[error("identity backup protection exceeds the supported size")]
    ProtectionTooLarge,
    #[error("identity backup artifact exceeds the supported size")]
    ArtifactTooLarge,
    #[error("invalid or unsupported encrypted identity backup")]
    InvalidBackup,
    #[error("encrypted identity backup authentication failed")]
    AuthenticationFailed,
    #[error("identity recovery custody is unavailable")]
    CustodyUnavailable,
    #[error("identity restore conflicts with existing custody")]
    IdentityConflict,
    #[error("identity recovery is unavailable for this custody backend")]
    UnsupportedBackend,
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
    Offline,
    Starting,
    Connecting,
    Connected,
    Reconnecting,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum MobileRuntimeState {
    #[default]
    Ready,
    Failed,
    Stopped,
}

impl MobileRuntimeState {
    fn from_atomic(value: u8) -> Self {
        match value {
            value if value == Self::Failed as u8 => Self::Failed,
            value if value == Self::Stopped as u8 => Self::Stopped,
            _ => Self::Ready,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBootStage {
    Configuration,
    Identity,
    Storage,
    Transport,
    Composition,
    Cleanup,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBootFailureCode {
    InvalidConfiguration,
    IdentityUnavailable,
    StorageUnavailable,
    TransportUnavailable,
    CompositionFailed,
    CleanupFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[error("mobile boot failed during {stage:?}: {message}")]
#[serde(deny_unknown_fields)]
pub struct MobileBootError {
    pub stage: MobileBootStage,
    pub code: MobileBootFailureCode,
    pub retryable: bool,
    pub message: String,
}

impl MobileBootError {
    fn from_internal(error: &anyhow::Error) -> Self {
        let detail = error.to_string();
        let lower = detail.to_ascii_lowercase();
        let (stage, code, retryable, message) = if lower.contains("cleanup failed") {
            (MobileBootStage::Cleanup, MobileBootFailureCode::CleanupFailed, true, "cleanup failed")
        } else if lower.contains("key material") {
            (
                MobileBootStage::Identity,
                MobileBootFailureCode::IdentityUnavailable,
                true,
                "encrypted-file identity backend requires nonempty host key material",
            )
        } else if lower.contains("identity")
            || lower.contains("keychain")
            || lower.contains("keystore")
        {
            (
                MobileBootStage::Identity,
                MobileBootFailureCode::IdentityUnavailable,
                true,
                "identity initialization failed",
            )
        } else if lower.contains("database")
            || lower.contains("store")
            || lower.contains("metadata")
        {
            (
                MobileBootStage::Storage,
                MobileBootFailureCode::StorageUnavailable,
                true,
                "storage initialization failed",
            )
        } else if lower.contains("duplicate mobile interface profile") {
            (
                MobileBootStage::Configuration,
                MobileBootFailureCode::InvalidConfiguration,
                false,
                "mobile configuration contains a duplicate interface profile",
            )
        } else if lower.contains("address is empty") {
            (
                MobileBootStage::Configuration,
                MobileBootFailureCode::InvalidConfiguration,
                false,
                "mobile configuration contains an empty address",
            )
        } else if lower.contains("invalid tcp server") {
            (
                MobileBootStage::Configuration,
                MobileBootFailureCode::InvalidConfiguration,
                false,
                "mobile configuration contains an invalid TCP server address",
            )
        } else if lower.contains("tcp server")
            || lower.contains("transport")
            || lower.contains("bind")
        {
            (
                MobileBootStage::Transport,
                MobileBootFailureCode::TransportUnavailable,
                true,
                "transport initialization failed",
            )
        } else if lower.contains("address")
            || lower.contains("interface profile")
            || lower.contains("mobile config")
        {
            (
                MobileBootStage::Configuration,
                MobileBootFailureCode::InvalidConfiguration,
                false,
                "mobile configuration is invalid",
            )
        } else {
            (
                MobileBootStage::Composition,
                MobileBootFailureCode::CompositionFailed,
                true,
                "runtime composition failed",
            )
        };
        Self { stage, code, retryable, message: message.into() }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBearerKind {
    Tcp,
    BluetoothRnode,
    AndroidUsb,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileRNodeBearer {
    BluetoothLe,
    AndroidUsb,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileRNodeAttempt {
    generation: u64,
    bearer: MobileRNodeBearer,
    info: RNodeBearerInfo,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MobileRNodeByteStart {
    pub attempt: MobileRNodeAttempt,
    /// Ordered platform writes, each bounded by the attempt metadata.
    pub writes: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileRNodeWriteHandoff {
    generation: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MobileRNodeWriteBatch {
    pub handoff: MobileRNodeWriteHandoff,
    /// One complete KISS frame split into ordered platform writes.
    pub writes: Vec<Vec<u8>>,
}

impl MobileRNodeBearer {
    const fn observation_kind(self) -> MobileBearerKind {
        match self {
            Self::BluetoothLe => MobileBearerKind::BluetoothRnode,
            Self::AndroidUsb => MobileBearerKind::AndroidUsb,
        }
    }

    const fn accepts(self, kind: RNodeBearerKind) -> bool {
        matches!(
            (self, kind),
            (Self::BluetoothLe, RNodeBearerKind::Ble)
                | (Self::AndroidUsb, RNodeBearerKind::AndroidUsb)
        )
    }
}

fn fragment_rnode_writes(
    info: RNodeBearerInfo,
    frames: impl IntoIterator<Item = Vec<u8>>,
) -> Vec<Vec<u8>> {
    let mut writes = Vec::new();
    for frame in frames {
        writes.extend(rnode_write_chunks(Some(info), &frame).map(<[u8]>::to_vec));
    }
    writes
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
pub enum MobileBearerReason {
    NotConfigured,
    PermissionDenied,
    ConnectionInterrupted,
    PhysicalEvidenceAbsent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileFailureCode {
    InvalidTcpEndpoint,
    TcpRetrying,
    TransportUnavailable,
    CleanupFailed,
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
    pub reason: Option<MobileBearerReason>,
}

#[derive(Clone)]
pub struct MobilePlatformService {
    state: Arc<tokio::sync::RwLock<MobilePlatformState>>,
    changes: broadcast::Sender<()>,
}

struct MobilePlatformState {
    bearers: Vec<MobileBearerObservation>,
    bluetooth_approved: bool,
    usb_fallback_requested: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileUsbFallbackDisposition {
    Accepted,
    BluetoothActive,
}

impl MobilePlatformService {
    fn new(rnode_channel_enabled: bool) -> Self {
        let (changes, _) = broadcast::channel(16);
        let bluetooth = if rnode_channel_enabled {
            MobileBearerObservation {
                kind: MobileBearerKind::BluetoothRnode,
                state: MobileBearerState::Unverified,
                reason: Some(MobileBearerReason::PhysicalEvidenceAbsent),
            }
        } else {
            MobileBearerObservation {
                kind: MobileBearerKind::BluetoothRnode,
                state: MobileBearerState::Unavailable,
                reason: Some(MobileBearerReason::NotConfigured),
            }
        };
        Self {
            state: Arc::new(tokio::sync::RwLock::new(MobilePlatformState {
                bearers: vec![
                    bluetooth,
                    MobileBearerObservation {
                        kind: MobileBearerKind::AndroidUsb,
                        state: MobileBearerState::Unavailable,
                        reason: Some(MobileBearerReason::NotConfigured),
                    },
                ],
                bluetooth_approved: false,
                usb_fallback_requested: false,
            })),
            changes,
        }
    }

    pub async fn report(&self, observation: MobileBearerObservation) -> Result<(), &'static str> {
        if observation.kind == MobileBearerKind::Tcp {
            return Err("TCP bearer state is owned by the transport runtime");
        }
        let mut state = self.state.write().await;
        if observation.kind == MobileBearerKind::AndroidUsb
            && matches!(
                observation.state,
                MobileBearerState::Connecting
                    | MobileBearerState::Connected
                    | MobileBearerState::Reconnecting
            )
            && !state.usb_fallback_requested
        {
            return Err("Android USB requires an explicit fallback request");
        }
        if observation.kind == MobileBearerKind::AndroidUsb
            && matches!(
                observation.state,
                MobileBearerState::Connecting
                    | MobileBearerState::Connected
                    | MobileBearerState::Reconnecting
            )
            && approved_bluetooth_active(&state)
        {
            return Err("Android USB cannot preempt approved Bluetooth");
        }
        let bearer = state
            .bearers
            .iter_mut()
            .find(|bearer| bearer.kind == observation.kind)
            .ok_or("unsupported mobile platform bearer")?;
        *bearer = observation;
        drop(state);
        let _ = self.changes.send(());
        Ok(())
    }

    pub async fn set_bluetooth_approved(&self, approved: bool) {
        self.state.write().await.bluetooth_approved = approved;
    }

    async fn bluetooth_approved(&self) -> bool {
        self.state.read().await.bluetooth_approved
    }

    pub async fn request_android_usb_fallback(&self) -> MobileUsbFallbackDisposition {
        let mut state = self.state.write().await;
        if approved_bluetooth_active(&state) {
            return MobileUsbFallbackDisposition::BluetoothActive;
        }
        state.usb_fallback_requested = true;
        MobileUsbFallbackDisposition::Accepted
    }

    async fn snapshot(&self) -> Vec<MobileBearerObservation> {
        self.state.read().await.bearers.clone()
    }
}

fn approved_bluetooth_active(state: &MobilePlatformState) -> bool {
    state.bluetooth_approved
        && state.bearers.iter().any(|bearer| {
            bearer.kind == MobileBearerKind::BluetoothRnode
                && matches!(
                    bearer.state,
                    MobileBearerState::Connecting
                        | MobileBearerState::Connected
                        | MobileBearerState::Reconnecting
                )
        })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileSessionSnapshot {
    #[serde(default)]
    pub runtime: MobileRuntimeState,
    pub phase: MobileConnectionPhase,
    pub endpoint: Option<String>,
    pub generation: u64,
    pub failure: Option<MobileFailure>,
    pub bearers: Vec<MobileBearerObservation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileStorageStatus {
    pub generation: u64,
    pub schema_version: u32,
    pub open: crate::storage::messages::StorageOpenOutcome,
    pub recovery: crate::storage::messages::StorageRecoveryOutcome,
    pub last_commit: Option<crate::storage::messages::StorageCommitEvidence>,
    pub degraded: Option<crate::storage::messages::StorageDegradedReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MobileStateEventKind {
    Session,
    Peer,
    Message,
    Propagation,
    Conversation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MobileStateEvent {
    pub generation: u64,
    pub kind: MobileStateEventKind,
}

#[derive(Debug, thiserror::Error)]
pub enum MobileStateSubscriptionError {
    #[error("mobile state event stream lagged by {0} events")]
    Lagged(u64),
    #[error("mobile state event stream closed")]
    Closed,
}

pub struct MobileStateSubscription {
    generation: Arc<SessionGeneration>,
    transport: Arc<dyn MeshTransport>,
    lifecycle: broadcast::Receiver<TransportLifecycleEvent>,
    events: broadcast::Receiver<styrene_ipc::types::DaemonEvent>,
    platform: broadcast::Receiver<()>,
}

impl MobileStateSubscription {
    pub async fn recv(&mut self) -> Result<MobileStateEvent, MobileStateSubscriptionError> {
        loop {
            let kind = tokio::select! {
                event = self.lifecycle.recv() => match event {
                    Ok(
                        TransportLifecycleEvent::Connected
                        | TransportLifecycleEvent::Disconnected
                        | TransportLifecycleEvent::Reconnected
                        | TransportLifecycleEvent::InterfaceChanged
                        | TransportLifecycleEvent::InterfaceReconcileRequired
                        | TransportLifecycleEvent::LinkReconcileRequired,
                    ) => MobileStateEventKind::Session,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(dropped)) => {
                        return Err(MobileStateSubscriptionError::Lagged(dropped));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(MobileStateSubscriptionError::Closed);
                    }
                },
                event = self.events.recv() => match event {
                    Ok(styrene_ipc::types::DaemonEvent::Device { .. }) => {
                        MobileStateEventKind::Peer
                    }
                    Ok(styrene_ipc::types::DaemonEvent::Message { .. }) => {
                        MobileStateEventKind::Message
                    }
                    Ok(styrene_ipc::types::DaemonEvent::StandardPropagationChanged { .. }) => {
                        MobileStateEventKind::Propagation
                    }
                    Ok(styrene_ipc::types::DaemonEvent::ConversationInvalidated { .. }) => {
                        MobileStateEventKind::Conversation
                    }
                    Ok(styrene_ipc::types::DaemonEvent::ReconcileRequired { dropped }) => {
                        return Err(MobileStateSubscriptionError::Lagged(dropped));
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(dropped)) => {
                        return Err(MobileStateSubscriptionError::Lagged(dropped));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(MobileStateSubscriptionError::Closed);
                    }
                },
                event = self.platform.recv() => match event {
                    Ok(()) => MobileStateEventKind::Session,
                    Err(broadcast::error::RecvError::Lagged(dropped)) => {
                        return Err(MobileStateSubscriptionError::Lagged(dropped));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(MobileStateSubscriptionError::Closed);
                    }
                },
            };
            let interfaces = self.transport.interface_snapshots().await;
            return Ok(MobileStateEvent { generation: self.generation.observe(&interfaces), kind });
        }
    }
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileDeliveryMethod {
    Direct,
    Opportunistic,
    Propagated,
}

impl MobileDeliveryMethod {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Opportunistic => "opportunistic",
            Self::Propagated => "propagated",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "opportunistic" => Some(Self::Opportunistic),
            "propagated" => Some(Self::Propagated),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobileSendRequest {
    pub destination_hash: String,
    pub content: String,
    pub requested_method: MobileDeliveryMethod,
    pub draft_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileSendDisposition {
    Accepted,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileDraftClearDisposition {
    NotRequested,
    Cleared,
    Superseded,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileMessagingFailureCode {
    InvalidRequest,
    Unavailable,
    Timeout,
    Conflict,
    Denied,
    NotFound,
    NotImplemented,
    Internal,
    Transport,
    DispatchFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[error("{message}")]
#[serde(deny_unknown_fields)]
pub struct MobileMessagingFailure {
    pub code: MobileMessagingFailureCode,
    pub retryable: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MobileSendOutcome {
    pub generation: u64,
    pub disposition: MobileSendDisposition,
    pub message_id: String,
    pub message: styrene_ipc::types::MessageInfo,
    pub requested_method: MobileDeliveryMethod,
    pub actual_method: MobileDeliveryMethod,
    pub fallback_reason: Option<String>,
    pub terminal_failure: Option<MobileMessagingFailure>,
    pub draft_clear: MobileDraftClearDisposition,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileRetryDisposition {
    Applied,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MobileRetryOutcome {
    pub generation: u64,
    pub disposition: MobileRetryDisposition,
    pub message: styrene_ipc::types::MessageInfo,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePropagationReadiness {
    Unselected,
    Ready,
    Unavailable,
    Inactive,
    InvalidMetadata,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePropagationSyncState {
    Idle,
    InProgress,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationPolicy {
    pub transfer_limit_kb: u64,
    pub sync_limit_kb: u64,
    pub stamp_cost: u32,
    pub stamp_flexibility: u32,
    pub peering_cost: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationCandidate {
    pub identity_hash: String,
    pub destination_hash: String,
    pub active: bool,
    pub observed_at: i64,
    pub age_secs: u64,
    pub policy: Option<MobilePropagationPolicy>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationProgress {
    pub attempt_id: String,
    pub started_at: i64,
    pub deadline_at: Option<i64>,
    pub received_count: u64,
    pub received_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePropagationTriggerSource {
    InitialConnection,
    Reconnect,
    ForegroundOpportunity,
    GrantedBackgroundOpportunity,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePropagationTerminalOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationSynchronization {
    pub trigger: MobilePropagationTriggerSource,
    pub started_at: i64,
    pub finished_at: i64,
    pub outcome: MobilePropagationTerminalOutcome,
    pub new_messages: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePropagationFailureCode {
    InvalidDestination,
    NotAnnounced,
    Inactive,
    InvalidMetadata,
    Unavailable,
    Timeout,
    Cancelled,
    Transport,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[error("{message}")]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationFailure {
    pub code: MobilePropagationFailureCode,
    pub retryable: bool,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationSnapshot {
    pub generation: u64,
    pub observed_at: i64,
    pub selected_destination: Option<String>,
    pub readiness: MobilePropagationReadiness,
    pub ready: bool,
    pub selected_policy: Option<MobilePropagationPolicy>,
    pub candidates: Vec<MobilePropagationCandidate>,
    pub sync_state: MobilePropagationSyncState,
    pub new_messages: u32,
    pub in_flight: Option<MobilePropagationProgress>,
    pub failure: Option<MobilePropagationFailure>,
    pub automatic_sync_enabled: bool,
    pub automatic_sync_cooldown_secs: u64,
    pub sync_deadline_secs: u64,
    #[serde(default)]
    pub trigger_capabilities: Vec<MobilePropagationTriggerSource>,
    #[serde(default)]
    pub active_trigger: Option<MobilePropagationTriggerSource>,
    #[serde(default)]
    pub active_sync_started_at: Option<i64>,
    #[serde(default)]
    pub last_synchronization: Option<MobilePropagationSynchronization>,
    #[serde(default)]
    pub cooldown_remaining_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MobilePropagationSyncOutcome {
    pub generation: u64,
    pub new_messages: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileMessageEventKind {
    New,
    StatusChanged,
    Delivered,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MobileMessageEvent {
    pub generation: u64,
    pub kind: MobileMessageEventKind,
    pub message: styrene_ipc::types::MessageInfo,
}

pub struct MobileMessageSubscription {
    generation: u64,
    facade: Arc<DaemonFacade>,
    receiver: tokio::sync::broadcast::Receiver<styrene_ipc::types::DaemonEvent>,
}

impl MobileMessageSubscription {
    pub async fn recv(&mut self) -> Result<MobileMessageEvent, MobileMessagingFailure> {
        loop {
            match self.receiver.recv().await {
                Ok(styrene_ipc::types::DaemonEvent::Message { kind, message }) => {
                    let kind = match kind {
                        styrene_ipc::types::MessageEventKind::New => MobileMessageEventKind::New,
                        styrene_ipc::types::MessageEventKind::StatusChanged => {
                            MobileMessageEventKind::StatusChanged
                        }
                        styrene_ipc::types::MessageEventKind::Delivered => {
                            MobileMessageEventKind::Delivered
                        }
                        styrene_ipc::types::MessageEventKind::Failed => {
                            MobileMessageEventKind::Failed
                        }
                        _ => continue,
                    };
                    let complete =
                        DaemonMessaging::query_message(self.facade.as_ref(), &message.id)
                            .await
                            .map_err(mobile_messaging_failure)?
                            .ok_or_else(|| MobileMessagingFailure {
                                code: MobileMessagingFailureCode::NotFound,
                                retryable: false,
                                message: format!(
                                    "message event projection not found: {}",
                                    message.id
                                ),
                            })?;
                    return Ok(MobileMessageEvent {
                        generation: self.generation,
                        kind,
                        message: complete,
                    });
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                    return Err(MobileMessagingFailure {
                        code: MobileMessagingFailureCode::Unavailable,
                        retryable: true,
                        message: format!("mobile message event stream lagged by {dropped} events"),
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(MobileMessagingFailure {
                        code: MobileMessagingFailureCode::Unavailable,
                        retryable: true,
                        message: "mobile message event stream closed".into(),
                    });
                }
            }
        }
    }
}

fn mobile_messaging_failure(error: styrene_ipc::IpcError) -> MobileMessagingFailure {
    use styrene_ipc::IpcError;

    let code = match &error {
        IpcError::NotImplemented { .. } => MobileMessagingFailureCode::NotImplemented,
        IpcError::Unavailable { .. } => MobileMessagingFailureCode::Unavailable,
        IpcError::Timeout { .. } => MobileMessagingFailureCode::Timeout,
        IpcError::InvalidRequest { .. } => MobileMessagingFailureCode::InvalidRequest,
        IpcError::NotFound { .. } => MobileMessagingFailureCode::NotFound,
        IpcError::Conflict { .. } => MobileMessagingFailureCode::Conflict,
        IpcError::Denied { .. } => MobileMessagingFailureCode::Denied,
        IpcError::Internal { .. } => MobileMessagingFailureCode::Internal,
        IpcError::Transport { .. } => MobileMessagingFailureCode::Transport,
        _ => MobileMessagingFailureCode::Internal,
    };
    MobileMessagingFailure { code, retryable: error.is_retryable(), message: error.to_string() }
}

fn mobile_propagation_policy(
    peer: &crate::storage::standard_propagation::StandardPropagationPeerObservation,
) -> Option<MobilePropagationPolicy> {
    Some(MobilePropagationPolicy {
        transfer_limit_kb: u64::try_from(peer.transfer_limit_kb?).ok()?,
        sync_limit_kb: u64::try_from(peer.sync_limit_kb?).ok()?,
        stamp_cost: peer.stamp_cost?,
        stamp_flexibility: peer.stamp_flexibility?,
        peering_cost: peer.peering_cost?,
    })
}

fn mobile_propagation_trigger_source(
    trigger: crate::workers::standard_propagation::StandardPropagationSyncTriggerKind,
) -> MobilePropagationTriggerSource {
    use crate::workers::standard_propagation::StandardPropagationSyncTriggerKind;

    match trigger {
        StandardPropagationSyncTriggerKind::InitialConnection => {
            MobilePropagationTriggerSource::InitialConnection
        }
        StandardPropagationSyncTriggerKind::Reconnect => MobilePropagationTriggerSource::Reconnect,
        StandardPropagationSyncTriggerKind::ForegroundOpportunity => {
            MobilePropagationTriggerSource::ForegroundOpportunity
        }
        StandardPropagationSyncTriggerKind::BackgroundOpportunity => {
            MobilePropagationTriggerSource::GrantedBackgroundOpportunity
        }
        StandardPropagationSyncTriggerKind::Manual => MobilePropagationTriggerSource::Manual,
    }
}

fn mobile_propagation_terminal_outcome(
    outcome: crate::workers::standard_propagation::StandardPropagationSyncTerminalOutcome,
) -> MobilePropagationTerminalOutcome {
    use crate::workers::standard_propagation::StandardPropagationSyncTerminalOutcome;

    match outcome {
        StandardPropagationSyncTerminalOutcome::Succeeded => {
            MobilePropagationTerminalOutcome::Succeeded
        }
        StandardPropagationSyncTerminalOutcome::Failed => MobilePropagationTerminalOutcome::Failed,
        StandardPropagationSyncTerminalOutcome::TimedOut => {
            MobilePropagationTerminalOutcome::TimedOut
        }
        StandardPropagationSyncTerminalOutcome::Cancelled => {
            MobilePropagationTerminalOutcome::Cancelled
        }
    }
}

fn decode_mobile_hash(value: &str) -> Result<[u8; 16], String> {
    let bytes =
        hex::decode(value).map_err(|_| "destination hash must be hexadecimal".to_string())?;
    bytes.try_into().map_err(|_| "destination hash must contain exactly 16 bytes".to_string())
}

fn mobile_propagation_transport_failure(
    error: crate::transport::mesh_transport::TransportError,
) -> MobilePropagationFailure {
    use crate::transport::mesh_transport::TransportError;

    let (code, retryable) = match &error {
        TransportError::TimedOut => (MobilePropagationFailureCode::Timeout, true),
        TransportError::Cancelled => (MobilePropagationFailureCode::Cancelled, true),
        TransportError::Unavailable => (MobilePropagationFailureCode::Unavailable, true),
        TransportError::SendFailed(_)
        | TransportError::LinkFailed(_)
        | TransportError::CleanupFailed(_) => (MobilePropagationFailureCode::Transport, true),
        TransportError::ShutdownFailed(_) => (MobilePropagationFailureCode::Internal, false),
    };
    MobilePropagationFailure { code, retryable, message: error.to_string() }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedMobileConfig {
    schema_version: u32,
    tcp_endpoint: String,
}

const MOBILE_CONFIG_SCHEMA_VERSION: u32 = 1;

#[cfg(any(
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn lock_mobile_identity_custody(paths: &PlatformPaths) -> std::io::Result<std::fs::File> {
    use fs2::FileExt;

    std::fs::create_dir_all(&paths.config_dir)?;
    let path = paths.config_dir.join("identity-custody.lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}

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
    generation: Arc<SessionGeneration>,
    runtime_state: AtomicU8,
    transport_shutdown_complete: AtomicBool,
    #[cfg(test)]
    storage_shutdown_faults: AtomicU8,
    platform_service: MobilePlatformService,
    active_conversation: Arc<tokio::sync::RwLock<Option<String>>>,
    diagnostics: Arc<crate::mobile_diagnostics::MobileDiagnostics>,
    portable_backup_custody: Option<Arc<dyn MobilePortableBackupCustody>>,
}

#[async_trait::async_trait]
trait MobilePortableBackupCustody: Send + Sync {
    async fn export(
        &self,
        protection: zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError>;
}

struct LoadedMobileIdentity {
    identity: PrivateIdentity,
    portable_backup_custody: Option<Arc<dyn MobilePortableBackupCustody>>,
}

struct MobileWorkers {
    inbound: crate::workers::inbound::InboundWorkerHandle,
    announce: JoinHandle<()>,
    link: JoinHandle<()>,
    route: JoinHandle<()>,
    router_deadlines: JoinHandle<()>,
    active_conversation: JoinHandle<()>,
    session_generation: JoinHandle<()>,
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
    rnode_channel: Option<(InterfaceChannel, HostInterfaceControl)>,
}

struct RNodeBridge {
    address: AddressHash,
    rx: InterfaceRxSender,
    tx: AsyncMutex<InterfaceTxReceiver>,
    control: HostInterfaceControl,
    protocol: AsyncMutex<RNodeProtocol>,
    attempts: AsyncMutex<MobileRNodeAttemptState>,
}

struct MobileRNodeAttemptState {
    next_generation: u64,
    active: Option<MobileRNodeAttempt>,
    next_handoff_generation: u64,
    pending_packet: Option<MobileRNodePendingPacket>,
}

struct MobileRNodePendingPacket {
    handoff: MobileRNodeWriteHandoff,
    packet: Vec<u8>,
    offered_to: Option<MobileRNodeAttempt>,
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
        self.active_conversation.abort();
        self.session_generation.abort();
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
        self.active_conversation.abort();
        self.session_generation.abort();
        self.aborted = true;
        self.inbound.wait().await;
        let _ = (&mut self.announce).await;
        let _ = (&mut self.link).await;
        let _ = (&mut self.route).await;
        let _ = (&mut self.router_deadlines).await;
        let _ = (&mut self.active_conversation).await;
        let _ = (&mut self.session_generation).await;
    }

    #[cfg(test)]
    fn all_finished(&self) -> bool {
        self.inbound.is_finished()
            && self.announce.is_finished()
            && self.link.is_finished()
            && self.route.is_finished()
            && self.router_deadlines.is_finished()
            && self.active_conversation.is_finished()
            && self.session_generation.is_finished()
            && self.standard_propagation_sync.as_ref().is_none_or(|worker| worker.is_finished())
    }

    #[cfg(test)]
    fn abort_handles(&self) -> Vec<tokio::task::AbortHandle> {
        let mut handles = Vec::from(self.inbound.abort_handles());
        handles.push(self.announce.abort_handle());
        handles.push(self.link.abort_handle());
        handles.push(self.route.abort_handle());
        handles.push(self.router_deadlines.abort_handle());
        handles.push(self.active_conversation.abort_handle());
        handles.push(self.session_generation.abort_handle());
        if let Some(worker) = &self.standard_propagation_sync {
            handles.push(worker.abort_handle());
        }
        handles
    }
}

impl Drop for MobileNode {
    fn drop(&mut self) {
        if let Ok(workers) = self.workers.get_mut()
            && let Some(workers) = workers.as_mut()
        {
            workers.abort();
        }
    }
}

/// Result of a hub poll operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollResult {
    /// Number of new messages fetched.
    pub message_count: usize,
    /// The fetched messages (for local notification display).
    pub messages: Vec<PollMessage>,
    /// Durable local and remote acknowledgement outcome for every fetched item.
    pub items: Vec<PollItemOutcome>,
    /// Whole-batch rejection when input exceeds the legacy poll contract.
    pub batch_failure: Option<PollBatchFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollBatchFailure {
    ItemLimitExceeded { limit: usize, observed: usize },
    ByteLimitExceeded { limit: usize, observed: usize },
}

/// A message fetched during hub poll (simplified for notification display).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollMessage {
    pub source_hash: String,
    pub content_preview: String,
    pub timestamp: i64,
}

/// Local acceptance outcome for one item returned by the legacy hub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollLocalOutcome {
    Accepted { message_id: String },
    DurableDuplicate { message_id: String },
    DecodeRejected { reason: String },
    StorageFailed { message_id: Option<String>, error: String },
}

/// Remote acknowledgement outcome for one item returned by the legacy hub API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollAcknowledgementOutcome {
    Acknowledged,
    NotEligible,
    Failed { error: String },
}

/// Complete processing outcome for one fetched hub item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollItemOutcome {
    pub hub_id: String,
    pub local: PollLocalOutcome,
    pub acknowledgement: PollAcknowledgementOutcome,
}

/// Poll previews are bounded by both Unicode scalar count and UTF-8 byte length.
pub const POLL_PREVIEW_MAX_CHARS: usize = 100;
pub const POLL_PREVIEW_MAX_BYTES: usize = 100;
pub const LEGACY_HUB_POLL_MAX_ITEMS: usize = 256;
pub const LEGACY_HUB_POLL_MAX_BYTES: usize =
    crate::services::fleet::PROPAGATION_FETCH_MAX_RESPONSE_BYTES;
pub const LEGACY_HUB_POLL_DEADLINE: Duration = Duration::from_secs(30);

/// Return the longest Unicode-scalar prefix within the legacy notification limits.
pub fn legacy_poll_preview(content: &str) -> String {
    let mut end = 0;
    for (count, (index, character)) in content.char_indices().enumerate() {
        if count == POLL_PREVIEW_MAX_CHARS
            || index.saturating_add(character.len_utf8()) > POLL_PREVIEW_MAX_BYTES
        {
            break;
        }
        end = index + character.len_utf8();
    }
    content[..end].to_string()
}

fn bounded_poll_error(error: impl std::fmt::Display) -> String {
    error.to_string().chars().take(256).collect()
}

/// A locally processed legacy hub batch awaiting a remote acknowledgement attempt.
///
/// Hosts with their own background transport can use this to preserve the same
/// durable-before-ACK behavior as [`MobileNode::poll_hub`].
pub struct LegacyHubPollBatch {
    result: PollResult,
    eligible: Vec<(usize, String)>,
}

impl LegacyHubPollBatch {
    /// Hub item identifiers whose messages are durably accepted locally.
    pub fn acknowledgement_ids(&self) -> Vec<&str> {
        self.eligible.iter().map(|(_, id)| id.as_str()).collect()
    }

    /// Apply the result of one remote acknowledgement attempt to every eligible item.
    pub fn complete(mut self, acknowledgement: Result<(), String>) -> PollResult {
        for (index, _) in self.eligible {
            self.result.items[index].acknowledgement = match &acknowledgement {
                Ok(()) => PollAcknowledgementOutcome::Acknowledged,
                Err(error) => {
                    PollAcknowledgementOutcome::Failed { error: bounded_poll_error(error) }
                }
            };
        }
        self.result
    }
}

struct MobileStores {
    messages: Arc<Mutex<MessagesStore>>,
    nodes: Arc<NodeStore>,
}

struct MobileIdentityRuntime {
    metadata: crate::services::identity::PublicIdentityMetadata,
    metadata_path: PathBuf,
    custody: styrene_ipc::types::IdentityCustodyInfo,
    backup_custody: Option<Arc<dyn crate::services::identity::IdentityBackupCustody>>,
    portable_backup_custody: Option<Arc<dyn MobilePortableBackupCustody>>,
}

#[cfg(test)]
#[derive(Clone)]
struct MobileBootFault {
    evidence: Arc<MobileBootFaultEvidence>,
}

#[cfg(test)]
#[derive(Default)]
struct MobileBootFaultEvidence {
    worker_handles: Mutex<Vec<tokio::task::AbortHandle>>,
}

#[cfg(test)]
static MOBILE_BOOT_FAULTS: std::sync::LazyLock<
    Mutex<std::collections::HashMap<PathBuf, MobileBootFault>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

#[cfg(test)]
fn inject_mobile_boot_fault(data_dir: PathBuf, evidence: Arc<MobileBootFaultEvidence>) {
    let previous =
        MOBILE_BOOT_FAULTS.lock().unwrap().insert(data_dir, MobileBootFault { evidence });
    assert!(previous.is_none(), "mobile boot fault already installed for test path");
}

#[cfg(test)]
fn take_mobile_boot_fault(data_dir: &std::path::Path) -> Option<MobileBootFault> {
    MOBILE_BOOT_FAULTS.lock().unwrap().remove(data_dir)
}

async fn compose_mobile_node(
    paths: PlatformPaths,
    identity: PrivateIdentity,
    stores: MobileStores,
    transport_runtime: MobileTransportRuntime,
    identity_runtime: MobileIdentityRuntime,
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
    let generation = Arc::new(SessionGeneration::new(1));
    let mut generation_events = transport.subscribe_lifecycle();
    generation.observe(&transport.interface_snapshots().await);
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
    let pages_dir = paths.pages_dir();
    let files_dir = paths.data_dir.join("files");
    let app_context = Arc::new(AppContext::with_node_store_and_pages(
        transport.clone(),
        identity_hash.clone(),
        stores.messages,
        stores.nodes,
        crate::services::PageService::with_storage_dirs(pages_dir, files_dir),
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
    app_context.identity().configure_mobile_identity(
        identity_runtime.metadata_path,
        identity_runtime.metadata,
        identity_runtime.custody,
        identity_runtime.backup_custody,
    );
    if let Some(hub_hash) = &hub_delivery_hash {
        app_context.messaging().set_propagation_hub(hub_hash.clone(), app_context.fleet_arc());
    }

    let active_conversation = Arc::new(tokio::sync::RwLock::new(None::<String>));
    let mut message_events = app_context.events().subscribe_messages(&[]);
    let active_conversation_worker = {
        let active_conversation = Arc::clone(&active_conversation);
        let messaging = app_context.messaging_arc();
        tokio::spawn(async move {
            loop {
                match message_events.recv().await {
                    Ok(styrene_ipc::types::DaemonEvent::Message {
                        kind: styrene_ipc::types::MessageEventKind::New,
                        message,
                    }) => {
                        let active = active_conversation.read().await.clone();
                        if active.as_deref() == Some(message.source_hash.as_str()) {
                            let _ = messaging.mark_read(&message.source_hash);
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(active) = active_conversation.read().await.clone() {
                            let _ = messaging.mark_read(&active);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };
    let session_generation_worker = {
        let generation = Arc::clone(&generation);
        let transport = Arc::clone(&transport);
        tokio::spawn(async move {
            loop {
                match generation_events.recv().await {
                    Ok(
                        TransportLifecycleEvent::Connected
                        | TransportLifecycleEvent::Disconnected
                        | TransportLifecycleEvent::Reconnected
                        | TransportLifecycleEvent::InterfaceChanged
                        | TransportLifecycleEvent::InterfaceReconcileRequired,
                    )
                    | Err(broadcast::error::RecvError::Lagged(_)) => {
                        generation.observe(&transport.interface_snapshots().await);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };
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
            app_context.transport().subscribe_lifecycle(),
            app_context.transport().is_connected(),
            app_context.events_arc(),
        )
    });
    if let Some(worker) = &standard_propagation_sync {
        app_context.publish_standard_propagation_sync(worker.observation());
    }
    let mut workers = MobileWorkers {
        inbound,
        announce,
        link,
        route,
        router_deadlines,
        active_conversation: active_conversation_worker,
        session_generation: session_generation_worker,
        standard_propagation_sync,
        aborted: false,
    };
    let diagnostics =
        Arc::new(crate::mobile_diagnostics::MobileDiagnostics::new().map_err(|error| {
            anyhow::anyhow!("initialize mobile diagnostic correlation: {error}")
        })?);
    let facade = Arc::new(DaemonFacade::new_mobile(
        app_context.clone(),
        identity_hash,
        Arc::clone(&diagnostics),
        Arc::clone(&generation),
    ));

    #[cfg(test)]
    if let Some(fault) = take_mobile_boot_fault(&paths.data_dir) {
        let handles = workers.abort_handles();
        workers.shutdown().await;
        let transport_result = transport.shutdown().await;
        *fault.evidence.worker_handles.lock().unwrap() = handles;
        transport_result.map_err(|error| {
            anyhow::anyhow!("injected mobile composition cleanup failed: {error}")
        })?;
        anyhow::bail!("injected mobile failure after services and workers were composed");
    }

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
    let rnode_channel_enabled = rnode_channel.is_some();
    let rnode = rnode_channel.map(|(channel, control)| RNodeBridge {
        address: channel.address,
        rx: channel.rx_channel,
        tx: AsyncMutex::new(channel.tx_channel),
        control,
        protocol: AsyncMutex::new(RNodeProtocol::new(RNodeRadioProfile::US_915_DEVELOPMENT)),
        attempts: AsyncMutex::new(MobileRNodeAttemptState {
            next_generation: 1,
            active: None,
            next_handoff_generation: 1,
            pending_packet: None,
        }),
    });

    let node = MobileNode {
        app_context,
        facade,
        paths,
        hub_delivery_hash,
        workers: Mutex::new(Some(workers)),
        tcp_listen_addresses,
        startup_contract,
        rnode,
        tcp_endpoint,
        generation,
        runtime_state: AtomicU8::new(MobileRuntimeState::Ready as u8),
        transport_shutdown_complete: AtomicBool::new(false),
        #[cfg(test)]
        storage_shutdown_faults: AtomicU8::new(0),
        platform_service: MobilePlatformService::new(rnode_channel_enabled),
        active_conversation,
        diagnostics,
        portable_backup_custody: identity_runtime.portable_backup_custody,
    };
    node.record_diagnostic(
        styrene_ipc::types::MobileDiagnosticSource::Runtime,
        styrene_ipc::types::MobileDiagnosticStage::Boot,
        styrene_ipc::types::MobileDiagnosticSeverity::Info,
        None,
    );
    Ok(node)
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
    /// Record an allowlisted event. Correlation bytes are retained only as a one-way digest.
    pub fn record_diagnostic(
        &self,
        source: styrene_ipc::types::MobileDiagnosticSource,
        stage: styrene_ipc::types::MobileDiagnosticStage,
        severity: styrene_ipc::types::MobileDiagnosticSeverity,
        correlation: Option<&[u8]>,
    ) {
        self.diagnostics.record(source, stage, severity, self.generation.current(), correlation);
    }

    #[must_use]
    pub fn diagnostic_snapshot(&self) -> styrene_ipc::types::MobileDiagnosticSnapshot {
        self.diagnostics.snapshot()
    }

    pub fn diagnostic_export(
        &self,
    ) -> Result<styrene_ipc::types::MobileDiagnosticExport, serde_json::Error> {
        self.diagnostics.export()
    }

    pub fn startup_contract(&self) -> &StartupContract {
        &self.startup_contract
    }

    pub fn active_capabilities(&self, caller_identity: &str) -> ActiveCapabilities {
        self.startup_contract
            .active_capabilities(self.app_context.policy().authorized_capabilities(caller_identity))
            .with_generation(self.generation.current())
    }

    pub fn storage_status(&self) -> Result<MobileStorageStatus, &'static str> {
        let status = self
            .app_context
            .store()
            .lock()
            .map_err(|_| "mobile storage status unavailable")?
            .storage_status();
        Ok(MobileStorageStatus {
            generation: self.generation.current(),
            schema_version: status.schema_version,
            open: status.open,
            recovery: status.recovery,
            last_commit: status.last_commit,
            degraded: status.degraded,
        })
    }

    /// Inspect configured identity custody without creating a new identity.
    pub async fn identity_presence(
        config: &MobileConfig,
    ) -> Result<MobileIdentityPresence, MobileIdentityRecoveryError> {
        identity_presence(
            config.identity_backend,
            &PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone()),
        )
        .await
    }

    /// Restore passphrase-protected identity custody before normal boot can create one.
    pub async fn restore_identity_before_boot(
        config: &MobileConfig,
        backup: styrene_ipc::types::IdentityBackupImport,
        protection: &[u8],
    ) -> Result<styrene_ipc::types::IdentityRestoreOutcome, MobileIdentityRecoveryError> {
        restore_identity_before_boot_inner(config, backup, protection, None).await
    }

    /// Restore before boot when the active encrypted-file backend uses host-owned key material.
    pub async fn restore_identity_before_boot_with_encrypted_file_key(
        config: &MobileConfig,
        backup: styrene_ipc::types::IdentityBackupImport,
        protection: &[u8],
        key_material: &[u8],
    ) -> Result<styrene_ipc::types::IdentityRestoreOutcome, MobileIdentityRecoveryError> {
        restore_identity_before_boot_inner(config, backup, protection, Some(key_material)).await
    }

    /// Boot the daemon in-process for mobile use.
    ///
    /// Creates identity if needed, opens SQLite, starts transport.
    /// Does NOT start an IPC server or PTY terminal.
    pub async fn boot(config: MobileConfig) -> Result<Self, MobileBootError> {
        Self::boot_inner(config, None).await.map_err(|error| MobileBootError::from_internal(&error))
    }

    /// Boot with host-owned key material for `EncryptedFile` custody.
    pub async fn boot_with_encrypted_file_key(
        config: MobileConfig,
        key_material: &[u8],
    ) -> Result<Self, MobileBootError> {
        Self::boot_inner(config, Some(key_material))
            .await
            .map_err(|error| MobileBootError::from_internal(&error))
    }

    async fn boot_inner(
        config: MobileConfig,
        encrypted_file_key_material: Option<&[u8]>,
    ) -> anyhow::Result<Self> {
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
        if explicit_tcp_endpoints.is_empty()
            && let Some(endpoint) = &tcp_endpoint
        {
            interfaces.push(ValidatedMobileInterface::TcpClient(endpoint.clone()));
        }

        // Load or create identity via the configured backend.
        let backup_custody =
            identity_backup_custody(config.identity_backend, &paths, encrypted_file_key_material);
        let loaded_identity =
            load_or_create_identity(&config.identity_backend, &paths, encrypted_file_key_material)
                .await?;
        let LoadedMobileIdentity { identity, portable_backup_custody } = loaded_identity;
        let custody = active_custody(config.identity_backend);

        // Open database
        let db_path = paths.db_path();
        let store = Arc::new(Mutex::new(
            MessagesStore::open(&db_path).map_err(|e| anyhow::anyhow!("database: {e}"))?,
        ));
        let boot_result = async {
            let node_store_path = db_path.with_file_name("nodes.db");
            let node_store_path = node_store_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("mobile node store path is not valid UTF-8"))?;
            let node_store = Arc::new(NodeStore::open(node_store_path)?);

            let metadata_path = paths.config_dir.join("identity-public.json");
            let mut metadata = load_public_identity_metadata(&metadata_path)?;
            if metadata.display_name.is_none()
                && let Some(display_name) =
                    config.display_name.as_deref().and_then(normalize_display_name)
            {
                metadata.display_name = Some(display_name);
                persist_public_identity_metadata(&metadata_path, &metadata)?;
            }
            let display_name = metadata.display_name.clone();
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
                    .set_receipt_handler(Box::new(
                        crate::receipt_bridge::CompositeReceiptHandler::new(vec![
                            Box::new(crate::receipt_bridge::ServiceReceiptBridge::new(
                                receipt_target.clone(),
                            )),
                            Box::new(packet_receipts.clone()),
                        ]),
                    ))
                    .await;

                let iface_mgr = transport_instance.iface_manager();
                let rnode_channel = if config.enable_rnode_channel {
                    Some(iface_mgr.lock().await.new_host_channel(
                        128,
                        InterfaceDescriptor { kind: InterfaceKind::Kiss, ..Default::default() },
                    ))
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
                MobileStores { messages: store.clone(), nodes: node_store },
                transport_runtime,
                MobileIdentityRuntime {
                    metadata,
                    metadata_path,
                    custody,
                    backup_custody,
                    portable_backup_custody,
                },
                config.hub_delivery_hash,
                tcp_endpoint,
            )
            .await
        }
        .await;

        if boot_result.is_err() {
            let cleanup_result = store
                .lock()
                .map_err(|_| anyhow::anyhow!("failed-boot storage cleanup failed: lock poisoned"))
                .and_then(|mut store| {
                    store.mark_clean_shutdown().map_err(|error| {
                        anyhow::anyhow!("failed-boot storage cleanup failed: {error}")
                    })
                });
            cleanup_result?;
        }
        boot_result
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
        let runtime = MobileRuntimeState::from_atomic(self.runtime_state.load(Ordering::Acquire));
        let interfaces = self.app_context.transport().interface_snapshots().await;
        let generation = self.generation.observe(&interfaces);
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
        let platform_bearers = self.platform_service.snapshot().await;
        let failure = if runtime == MobileRuntimeState::Failed {
            Some(MobileFailure { code: MobileFailureCode::CleanupFailed, retryable: true })
        } else {
            (tcp == MobileBearerState::Reconnecting)
                .then_some(MobileFailure { code: MobileFailureCode::TcpRetrying, retryable: true })
        };
        let operational = interfaces.iter().any(|interface| {
            matches!(
                interface.state,
                InterfaceState::Listening | InterfaceState::Connected | InterfaceState::Active
            )
        });
        let phase = if runtime == MobileRuntimeState::Stopped {
            MobileConnectionPhase::Stopped
        } else if runtime == MobileRuntimeState::Failed {
            MobileConnectionPhase::Failed
        } else if operational {
            MobileConnectionPhase::Connected
        } else if interfaces.iter().any(|interface| interface.state == InterfaceState::Retrying) {
            MobileConnectionPhase::Reconnecting
        } else if interfaces.iter().any(|interface| {
            matches!(interface.state, InterfaceState::Starting | InterfaceState::Connecting)
        }) {
            MobileConnectionPhase::Connecting
        } else {
            MobileConnectionPhase::Offline
        };
        MobileSessionSnapshot {
            runtime,
            phase,
            endpoint: self.tcp_endpoint.clone(),
            generation,
            failure,
            bearers: std::iter::once(MobileBearerObservation {
                kind: MobileBearerKind::Tcp,
                state: tcp,
                reason: None,
            })
            .chain(platform_bearers)
            .collect(),
        }
    }

    #[must_use]
    pub fn platform_service(&self) -> MobilePlatformService {
        self.platform_service.clone()
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

    pub fn subscribe_state_events(&self) -> MobileStateSubscription {
        let transport = self.app_context.transport_arc();
        MobileStateSubscription {
            generation: Arc::clone(&self.generation),
            lifecycle: transport.subscribe_lifecycle(),
            events: self.app_context.events().subscribe_daemon_events(),
            platform: self.platform_service.changes.subscribe(),
            transport,
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

    pub async fn send_text(
        &self,
        request: MobileSendRequest,
    ) -> Result<MobileSendOutcome, MobileMessagingFailure> {
        let result = self.send_text_inner(request).await;
        let (severity, correlation) = match &result {
            Ok(outcome)
                if outcome.disposition == MobileSendDisposition::Accepted
                    && outcome.terminal_failure.is_none() =>
            {
                (
                    styrene_ipc::types::MobileDiagnosticSeverity::Info,
                    Some(outcome.message_id.as_bytes()),
                )
            }
            Ok(outcome) => (
                styrene_ipc::types::MobileDiagnosticSeverity::Warning,
                Some(outcome.message_id.as_bytes()),
            ),
            Err(_) => (styrene_ipc::types::MobileDiagnosticSeverity::Error, None),
        };
        self.record_diagnostic(
            styrene_ipc::types::MobileDiagnosticSource::Messaging,
            styrene_ipc::types::MobileDiagnosticStage::Outbound,
            severity,
            correlation,
        );
        result
    }

    async fn send_text_inner(
        &self,
        request: MobileSendRequest,
    ) -> Result<MobileSendOutcome, MobileMessagingFailure> {
        let mut daemon_request = styrene_ipc::types::SendChatRequest::default();
        daemon_request.peer_hash = request.destination_hash;
        daemon_request.content = request.content;
        daemon_request.delivery_method = Some(request.requested_method.as_str().into());
        let draft_revision = request.draft_revision;
        let outcome = DaemonMessaging::send_chat_outcome(self.facade.as_ref(), daemon_request)
            .await
            .map_err(mobile_messaging_failure)?;
        let requested_method =
            MobileDeliveryMethod::parse(&outcome.requested_method).ok_or_else(|| {
                MobileMessagingFailure {
                    code: MobileMessagingFailureCode::Internal,
                    retryable: false,
                    message: format!(
                        "backend returned unsupported requested delivery method: {}",
                        outcome.requested_method
                    ),
                }
            })?;
        let actual_method =
            MobileDeliveryMethod::parse(&outcome.actual_method).ok_or_else(|| {
                MobileMessagingFailure {
                    code: MobileMessagingFailureCode::Internal,
                    retryable: false,
                    message: format!(
                        "backend returned unsupported actual delivery method: {}",
                        outcome.actual_method
                    ),
                }
            })?;
        let disposition = match outcome.disposition {
            styrene_ipc::types::SendChatDisposition::Accepted => MobileSendDisposition::Accepted,
            styrene_ipc::types::SendChatDisposition::Failed => MobileSendDisposition::Failed,
            _ => {
                return Err(MobileMessagingFailure {
                    code: MobileMessagingFailureCode::Internal,
                    retryable: false,
                    message: "backend returned unsupported mobile send disposition".into(),
                });
            }
        };
        let terminal_failure = outcome.terminal_error.map(|message| MobileMessagingFailure {
            code: MobileMessagingFailureCode::DispatchFailed,
            retryable: true,
            message,
        });
        let draft_clear = if disposition == MobileSendDisposition::Accepted {
            if let Some(revision) = draft_revision {
                match DaemonMessaging::clear_draft_if_revision(
                    self.facade.as_ref(),
                    &outcome.message.destination_hash,
                    revision,
                )
                .await
                .map_err(mobile_messaging_failure)?
                {
                    styrene_ipc::types::MessagingDisposition::Applied => {
                        MobileDraftClearDisposition::Cleared
                    }
                    _ => MobileDraftClearDisposition::Superseded,
                }
            } else {
                MobileDraftClearDisposition::NotRequested
            }
        } else if draft_revision.is_some() {
            MobileDraftClearDisposition::Superseded
        } else {
            MobileDraftClearDisposition::NotRequested
        };
        Ok(MobileSendOutcome {
            generation: self.session_snapshot().await.generation,
            disposition,
            message_id: outcome.message_id,
            message: outcome.message,
            requested_method,
            actual_method,
            fallback_reason: outcome.fallback_reason,
            terminal_failure,
            draft_clear,
        })
    }

    pub async fn retry_text(
        &self,
        message_id: &str,
    ) -> Result<MobileRetryOutcome, MobileMessagingFailure> {
        let outcome = DaemonMessaging::retry_message_outcome(self.facade.as_ref(), message_id)
            .await
            .map_err(mobile_messaging_failure)?;
        let disposition = match outcome.disposition {
            styrene_ipc::types::MessagingDisposition::Applied => MobileRetryDisposition::Applied,
            styrene_ipc::types::MessagingDisposition::Unchanged => {
                MobileRetryDisposition::Unchanged
            }
            styrene_ipc::types::MessagingDisposition::NotFound => {
                return Err(MobileMessagingFailure {
                    code: MobileMessagingFailureCode::NotFound,
                    retryable: false,
                    message: "message not found".into(),
                });
            }
            styrene_ipc::types::MessagingDisposition::TerminalConflict => {
                return Err(MobileMessagingFailure {
                    code: MobileMessagingFailureCode::Conflict,
                    retryable: false,
                    message: outcome
                        .terminal_state
                        .unwrap_or_else(|| "message is not retryable".into()),
                });
            }
            _ => {
                return Err(MobileMessagingFailure {
                    code: MobileMessagingFailureCode::Internal,
                    retryable: false,
                    message: "backend returned unsupported mobile retry disposition".into(),
                });
            }
        };
        let message = outcome.message.ok_or_else(|| MobileMessagingFailure {
            code: MobileMessagingFailureCode::Internal,
            retryable: false,
            message: "backend retry omitted the authoritative message".into(),
        })?;
        Ok(MobileRetryOutcome {
            generation: self.session_snapshot().await.generation,
            disposition,
            message,
        })
    }

    pub async fn propagation_snapshot(
        &self,
    ) -> Result<MobilePropagationSnapshot, MobilePropagationFailure> {
        let sync_worker = self.workers.lock().ok().and_then(|workers| {
            workers
                .as_ref()
                .and_then(|workers| workers.standard_propagation_sync.as_ref())
                .map(|worker| (worker.policy(), worker.telemetry()))
        });
        let sync_policy = sync_worker.map(|(policy, _)| policy);
        let sync_telemetry = sync_worker.map(|(_, telemetry)| telemetry).unwrap_or_default();
        let observed_at = rns_core::transport::time::now_epoch_secs_i64();
        let runtime_policy =
            crate::standard_propagation::StandardPropagationRuntimeObservation::client().policy();
        let storage_policy = crate::storage::standard_propagation::StandardPropagationPolicy {
            queue_max_count: runtime_policy.queue_max_count,
            queue_max_bytes: runtime_policy.queue_max_bytes,
            expiry_secs: runtime_policy.expiry_secs,
        };
        let observation = self
            .app_context
            .store()
            .lock()
            .map_err(|_| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Unavailable,
                retryable: true,
                message: "mobile propagation store lock poisoned".into(),
            })?
            .standard_propagation_observation(observed_at, storage_policy)
            .map_err(|error| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Internal,
                retryable: false,
                message: format!("mobile propagation snapshot: {error}"),
            })?;

        let selected_peer = observation.selection.as_ref().and_then(|selection| selection.peer);
        let selected = selected_peer.and_then(|identity| {
            observation.peers.iter().find(|peer| peer.identity_hash == identity)
        });
        let selected_destination =
            selected.and_then(|peer| peer.propagation_destination).map(hex::encode);
        let selected_policy = selected.and_then(mobile_propagation_policy);
        let readiness = match (selected_peer, selected) {
            (None, _) => MobilePropagationReadiness::Unselected,
            (Some(_), None) => MobilePropagationReadiness::Unavailable,
            (Some(_), Some(peer)) if !peer.enabled => MobilePropagationReadiness::Inactive,
            (Some(_), Some(peer)) if peer.propagation_destination.is_none() => {
                MobilePropagationReadiness::Unavailable
            }
            (Some(_), Some(_)) if selected_policy.is_none() => {
                MobilePropagationReadiness::InvalidMetadata
            }
            (Some(_), Some(_)) => MobilePropagationReadiness::Ready,
        };
        let in_flight = observation
            .attempts
            .iter()
            .find(|attempt| attempt.state == "running" && attempt.direction == "sync")
            .map(|attempt| MobilePropagationProgress {
                attempt_id: hex::encode(attempt.attempt_id),
                started_at: attempt.started_at,
                deadline_at: attempt.deadline_at,
                received_count: attempt.accepted_count as u64,
                received_bytes: attempt.accepted_bytes as u64,
            });
        let latest_inbound = observation
            .attempts
            .iter()
            .filter(|attempt| attempt.direction == "sync")
            .max_by_key(|attempt| (attempt.updated_at, attempt.started_at));
        let sync_state = if sync_telemetry.active.is_some() || in_flight.is_some() {
            MobilePropagationSyncState::InProgress
        } else {
            match latest_inbound.map(|attempt| attempt.state.as_str()) {
                Some("completed") => MobilePropagationSyncState::Complete,
                Some("failed" | "interrupted") => MobilePropagationSyncState::Failed,
                _ => MobilePropagationSyncState::Idle,
            }
        };
        let failure = (sync_state == MobilePropagationSyncState::Failed).then(|| {
            let code = latest_inbound
                .and_then(|attempt| attempt.failure_code.clone())
                .unwrap_or_else(|| "client_sync_failed".into());
            MobilePropagationFailure {
                code: MobilePropagationFailureCode::Transport,
                retryable: true,
                message: code,
            }
        });
        let new_messages = latest_inbound
            .filter(|attempt| attempt.state == "completed")
            .map(|attempt| u32::try_from(attempt.accepted_count).unwrap_or(u32::MAX))
            .unwrap_or(0);
        let candidates = observation
            .peers
            .iter()
            .filter_map(|peer| {
                let destination = peer.propagation_destination?;
                Some(MobilePropagationCandidate {
                    identity_hash: hex::encode(peer.identity_hash),
                    destination_hash: hex::encode(destination),
                    active: peer.enabled,
                    observed_at: peer.last_seen_at,
                    age_secs: observed_at.saturating_sub(peer.last_seen_at).max(0) as u64,
                    policy: mobile_propagation_policy(peer),
                })
            })
            .collect();

        Ok(MobilePropagationSnapshot {
            generation: self.session_snapshot().await.generation,
            observed_at,
            selected_destination,
            readiness,
            ready: readiness == MobilePropagationReadiness::Ready,
            selected_policy,
            candidates,
            sync_state,
            new_messages,
            in_flight,
            failure,
            automatic_sync_enabled: sync_policy.is_some_and(|policy| policy.automatic),
            automatic_sync_cooldown_secs: sync_policy.map_or(0, |policy| policy.cooldown.as_secs()),
            sync_deadline_secs: sync_policy.map_or(0, |policy| policy.deadline.as_secs()),
            trigger_capabilities: if sync_policy.is_some() {
                vec![
                    MobilePropagationTriggerSource::InitialConnection,
                    MobilePropagationTriggerSource::Reconnect,
                    MobilePropagationTriggerSource::ForegroundOpportunity,
                    MobilePropagationTriggerSource::GrantedBackgroundOpportunity,
                    MobilePropagationTriggerSource::Manual,
                ]
            } else {
                vec![MobilePropagationTriggerSource::Manual]
            },
            active_trigger: sync_telemetry
                .active
                .map(|active| mobile_propagation_trigger_source(active.trigger)),
            active_sync_started_at: sync_telemetry.active.map(|active| active.started_at),
            last_synchronization: sync_telemetry.last_completed.map(|completed| {
                MobilePropagationSynchronization {
                    trigger: mobile_propagation_trigger_source(completed.trigger),
                    started_at: completed.started_at,
                    finished_at: completed.finished_at,
                    outcome: mobile_propagation_terminal_outcome(completed.outcome),
                    new_messages: u32::try_from(completed.new_messages).unwrap_or(u32::MAX),
                }
            }),
            cooldown_remaining_secs: sync_telemetry
                .cooldown_remaining
                .as_secs()
                .saturating_add(u64::from(sync_telemetry.cooldown_remaining.subsec_nanos() > 0)),
        })
    }

    pub async fn select_propagation_destination(
        &self,
        destination_hash: &str,
    ) -> Result<MobilePropagationSnapshot, MobilePropagationFailure> {
        let destination =
            decode_mobile_hash(destination_hash).map_err(|message| MobilePropagationFailure {
                code: MobilePropagationFailureCode::InvalidDestination,
                retryable: false,
                message,
            })?;
        let snapshot = self.propagation_snapshot().await?;
        let candidate = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.destination_hash == hex::encode(destination))
            .ok_or_else(|| MobilePropagationFailure {
                code: MobilePropagationFailureCode::NotAnnounced,
                retryable: true,
                message: "propagation destination has no current canonical announce".into(),
            })?;
        if !candidate.active {
            return Err(MobilePropagationFailure {
                code: MobilePropagationFailureCode::Inactive,
                retryable: true,
                message: "propagation destination is inactive".into(),
            });
        }
        if candidate.policy.is_none() {
            return Err(MobilePropagationFailure {
                code: MobilePropagationFailureCode::InvalidMetadata,
                retryable: true,
                message: "propagation destination metadata is incomplete".into(),
            });
        }
        let identity = decode_mobile_hash(&candidate.identity_hash).map_err(|message| {
            MobilePropagationFailure {
                code: MobilePropagationFailureCode::Internal,
                retryable: false,
                message,
            }
        })?;
        self.app_context
            .store()
            .lock()
            .map_err(|_| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Unavailable,
                retryable: true,
                message: "mobile propagation store lock poisoned".into(),
            })?
            .standard_propagation_set_selection(Some(identity), "manual", snapshot.observed_at)
            .map_err(|error| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Internal,
                retryable: false,
                message: format!("persist mobile propagation selection: {error}"),
            })?;
        self.propagation_snapshot().await
    }

    pub async fn clear_propagation_destination(
        &self,
    ) -> Result<MobilePropagationSnapshot, MobilePropagationFailure> {
        let now = rns_core::transport::time::now_epoch_secs_i64();
        self.app_context
            .store()
            .lock()
            .map_err(|_| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Unavailable,
                retryable: true,
                message: "mobile propagation store lock poisoned".into(),
            })?
            .standard_propagation_set_selection(None, "disabled", now)
            .map_err(|error| MobilePropagationFailure {
                code: MobilePropagationFailureCode::Internal,
                retryable: false,
                message: format!("clear mobile propagation selection: {error}"),
            })?;
        self.propagation_snapshot().await
    }

    pub async fn sync_propagation_once(
        &self,
        deadline: Duration,
    ) -> Result<MobilePropagationSyncOutcome, MobilePropagationFailure> {
        let result = self.sync_propagation_once_inner(deadline).await;
        self.record_diagnostic(
            styrene_ipc::types::MobileDiagnosticSource::Messaging,
            styrene_ipc::types::MobileDiagnosticStage::Synchronization,
            if result.is_ok() {
                styrene_ipc::types::MobileDiagnosticSeverity::Info
            } else {
                styrene_ipc::types::MobileDiagnosticSeverity::Error
            },
            None,
        );
        result
    }

    async fn sync_propagation_once_inner(
        &self,
        deadline: Duration,
    ) -> Result<MobilePropagationSyncOutcome, MobilePropagationFailure> {
        let snapshot = self.propagation_snapshot().await?;
        if !snapshot.ready {
            return Err(MobilePropagationFailure {
                code: MobilePropagationFailureCode::Unavailable,
                retryable: true,
                message: "selected propagation destination is not ready".into(),
            });
        }
        let trigger = {
            self.workers
                .lock()
                .map_err(|_| MobilePropagationFailure {
                    code: MobilePropagationFailureCode::Unavailable,
                    retryable: true,
                    message: "mobile propagation worker lock poisoned".into(),
                })?
                .as_ref()
                .and_then(|workers| workers.standard_propagation_sync.as_ref())
                .map(crate::workers::standard_propagation::StandardPropagationSyncWorker::trigger)
        };
        let count = if let Some(trigger) = trigger {
            trigger.manual(deadline).await.map_err(mobile_propagation_transport_failure)?
        } else {
            self.app_context
                .messaging()
                .sync_standard_propagation_once(
                    std::time::Instant::now() + deadline,
                    CancellationToken::new(),
                )
                .await
                .map_err(mobile_propagation_transport_failure)?
        };
        Ok(MobilePropagationSyncOutcome {
            generation: self.session_snapshot().await.generation,
            new_messages: u32::try_from(count).unwrap_or(u32::MAX),
        })
    }

    pub fn propagation_foreground_opportunity(&self) -> bool {
        self.workers
            .lock()
            .ok()
            .and_then(|workers| {
                workers.as_ref().and_then(|workers| workers.standard_propagation_sync.as_ref()).map(
                    crate::workers::standard_propagation::StandardPropagationSyncWorker::trigger,
                )
            })
            .is_some_and(|trigger| trigger.foreground_opportunity())
    }

    pub fn propagation_background_opportunity(&self) -> bool {
        self.workers
            .lock()
            .ok()
            .and_then(|workers| {
                workers.as_ref().and_then(|workers| workers.standard_propagation_sync.as_ref()).map(
                    crate::workers::standard_propagation::StandardPropagationSyncWorker::trigger,
                )
            })
            .is_some_and(|trigger| trigger.background_opportunity())
    }

    pub async fn set_draft(
        &self,
        destination_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, MobileMessagingFailure> {
        DaemonMessaging::set_draft(self.facade.as_ref(), destination_hash, content)
            .await
            .map_err(mobile_messaging_failure)
    }

    pub async fn draft(
        &self,
        destination_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, MobileMessagingFailure> {
        DaemonMessaging::draft(self.facade.as_ref(), destination_hash)
            .await
            .map_err(mobile_messaging_failure)
    }

    pub async fn clear_draft_if_revision(
        &self,
        destination_hash: &str,
        revision: u64,
    ) -> Result<MobileDraftClearDisposition, MobileMessagingFailure> {
        DaemonMessaging::clear_draft_if_revision(self.facade.as_ref(), destination_hash, revision)
            .await
            .map(|disposition| match disposition {
                styrene_ipc::types::MessagingDisposition::Applied => {
                    MobileDraftClearDisposition::Cleared
                }
                _ => MobileDraftClearDisposition::Superseded,
            })
            .map_err(mobile_messaging_failure)
    }

    pub async fn set_active_conversation(
        &self,
        destination_hash: Option<&str>,
    ) -> Result<(), MobileMessagingFailure> {
        let Some(destination_hash) = destination_hash else {
            *self.active_conversation.write().await = None;
            return Ok(());
        };
        DaemonMessaging::mark_read(self.facade.as_ref(), destination_hash)
            .await
            .map_err(mobile_messaging_failure)?;
        *self.active_conversation.write().await = Some(destination_hash.to_ascii_lowercase());
        DaemonMessaging::mark_read(self.facade.as_ref(), destination_hash)
            .await
            .map_err(mobile_messaging_failure)?;
        Ok(())
    }

    pub async fn message(
        &self,
        message_id: &str,
    ) -> Result<Option<styrene_ipc::types::MessageInfo>, MobileMessagingFailure> {
        DaemonMessaging::query_message(self.facade.as_ref(), message_id)
            .await
            .map_err(mobile_messaging_failure)
    }

    pub async fn subscribe_message_events(&self) -> MobileMessageSubscription {
        MobileMessageSubscription {
            generation: self.session_snapshot().await.generation,
            facade: Arc::clone(&self.facade),
            receiver: self.app_context.events().subscribe_messages(&[]),
        }
    }

    /// Actual addresses bound by configured TCP server profiles.
    pub fn tcp_listen_addresses(&self) -> &[SocketAddr] {
        &self.tcp_listen_addresses
    }

    /// Stop retained workers and dispatch transport shutdown.
    pub async fn shutdown(&self) -> Result<(), crate::transport::mesh_transport::TransportError> {
        use crate::transport::mesh_transport::TransportError;

        if MobileRuntimeState::from_atomic(self.runtime_state.load(Ordering::Acquire))
            == MobileRuntimeState::Stopped
        {
            self.record_diagnostic(
                styrene_ipc::types::MobileDiagnosticSource::Runtime,
                styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
                styrene_ipc::types::MobileDiagnosticSeverity::Info,
                None,
            );
            return Ok(());
        }
        let mut workers = match self.workers.lock() {
            Ok(mut workers) => workers.take(),
            Err(_) => {
                self.runtime_state.store(MobileRuntimeState::Failed as u8, Ordering::Release);
                self.record_diagnostic(
                    styrene_ipc::types::MobileDiagnosticSource::Runtime,
                    styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
                    styrene_ipc::types::MobileDiagnosticSeverity::Error,
                    None,
                );
                return Err(TransportError::ShutdownFailed(
                    "mobile worker state unavailable".into(),
                ));
            }
        };
        if let Some(workers) = workers.as_mut() {
            workers.shutdown().await;
        }
        if !self.transport_shutdown_complete.load(Ordering::Acquire) {
            if let Err(error) = self.app_context.transport().shutdown().await {
                self.runtime_state.store(MobileRuntimeState::Failed as u8, Ordering::Release);
                self.record_diagnostic(
                    styrene_ipc::types::MobileDiagnosticSource::Transport,
                    styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
                    styrene_ipc::types::MobileDiagnosticSeverity::Error,
                    None,
                );
                return Err(error);
            }
            self.transport_shutdown_complete.store(true, Ordering::Release);
        }

        #[cfg(test)]
        if self
            .storage_shutdown_faults
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| remaining.checked_sub(1))
            .is_ok()
        {
            self.runtime_state.store(MobileRuntimeState::Failed as u8, Ordering::Release);
            self.record_diagnostic(
                styrene_ipc::types::MobileDiagnosticSource::Storage,
                styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
                styrene_ipc::types::MobileDiagnosticSeverity::Error,
                None,
            );
            return Err(TransportError::ShutdownFailed(
                "injected mobile storage clean-shutdown marker failure".into(),
            ));
        }

        let storage_result = self
            .app_context
            .store()
            .lock()
            .map_err(|_| TransportError::ShutdownFailed("mobile storage state unavailable".into()))
            .and_then(|mut store| {
                store.mark_clean_shutdown().map_err(|error| {
                    TransportError::ShutdownFailed(format!(
                        "mobile storage clean-shutdown marker failed: {error}"
                    ))
                })
            });
        if let Err(error) = storage_result {
            self.runtime_state.store(MobileRuntimeState::Failed as u8, Ordering::Release);
            self.record_diagnostic(
                styrene_ipc::types::MobileDiagnosticSource::Storage,
                styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
                styrene_ipc::types::MobileDiagnosticSeverity::Error,
                None,
            );
            return Err(error);
        }
        self.runtime_state.store(MobileRuntimeState::Stopped as u8, Ordering::Release);
        self.record_diagnostic(
            styrene_ipc::types::MobileDiagnosticSource::Runtime,
            styrene_ipc::types::MobileDiagnosticStage::Lifecycle,
            styrene_ipc::types::MobileDiagnosticSeverity::Info,
            None,
        );
        Ok(())
    }

    /// Submit unframed RNS bytes received from an Android-owned RNode.
    pub async fn submit_rnode_packet(&self, packet: &[u8]) -> Result<(), String> {
        if packet.first().is_some_and(|byte| byte & 0x80 != 0) {
            return Err("IFAC packet received on open RNode interface".to_string());
        }
        let packet = Packet::deserialize(&mut InputBuffer::new(packet))
            .map_err(|error| format!("invalid RNS packet ({} bytes): {error:?}", packet.len()))?;

        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        match rnode
            .rx
            .send(RxMessage::physical(rnode.address, packet, rns_core::packet::MTU))
            .await
            .map_err(|_| "RNode receive channel closed".to_string())
        {
            Ok(IngressEnqueueOutcome::Accepted) => Ok(()),
            Ok(IngressEnqueueOutcome::Dropped) => Err("RNode receive queue is full".to_string()),
            Ok(IngressEnqueueOutcome::Rejected) => {
                Err("RNode packet rejected by transport admission".to_string())
            }
            Err(error) => Err(error),
        }
    }

    /// Poll the next unframed RNS packet destined for the active mobile RNode.
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

    /// Begin one explicitly approved host-owned RNode byte attempt.
    pub async fn start_rnode_bytes(
        &self,
        bearer: MobileRNodeBearer,
        info: RNodeBearerInfo,
    ) -> Result<MobileRNodeByteStart, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        if !bearer.accepts(info.kind) {
            return Err("RNode bearer metadata does not match the mobile bearer".into());
        }
        if bearer == MobileRNodeBearer::BluetoothLe
            && !self.platform_service.bluetooth_approved().await
        {
            return Err("Bluetooth RNode requires an approved peripheral".into());
        }
        let mut attempts = rnode.attempts.lock().await;
        if attempts.active.is_some() {
            return Err("RNode byte attempt is already active".into());
        }
        let attempt = MobileRNodeAttempt { generation: attempts.next_generation, bearer, info };
        let next_generation = attempts
            .next_generation
            .checked_add(1)
            .ok_or("RNode byte attempt generation exhausted")?;
        rnode.control.set_state(InterfaceState::Connecting);
        if let Err(error) = self
            .platform_service
            .report(MobileBearerObservation {
                kind: bearer.observation_kind(),
                state: MobileBearerState::Connecting,
                reason: None,
            })
            .await
        {
            rnode.control.set_state(InterfaceState::Closed);
            return Err(error.into());
        }
        let mut protocol = rnode.protocol.lock().await;
        *protocol = RNodeProtocol::new(RNodeRadioProfile::US_915_DEVELOPMENT);
        let writes =
            fragment_rnode_writes(info, protocol.start().map_err(|error| error.to_string())?);
        attempts.next_generation = next_generation;
        attempts.active = Some(attempt);
        Ok(MobileRNodeByteStart { attempt, writes })
    }

    /// Accept arbitrary ordered RNode bytes and return bounded protocol response writes.
    pub async fn submit_rnode_bytes(
        &self,
        attempt: MobileRNodeAttempt,
        bytes: &[u8],
    ) -> Result<Vec<Vec<u8>>, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(Vec::new());
        }
        let output = rnode.protocol.lock().await.feed(bytes).map_err(|error| error.to_string())?;
        for packet in output.packets {
            self.submit_rnode_packet(&packet).await?;
        }
        if output.became_ready {
            rnode.control.set_state(InterfaceState::Active);
            self.platform_service
                .report(MobileBearerObservation {
                    kind: attempt.bearer.observation_kind(),
                    state: MobileBearerState::Connected,
                    reason: None,
                })
                .await?;
        }
        Ok(fragment_rnode_writes(attempt.info, output.writes))
    }

    /// Return read-only RNode facts only for the current byte-attempt generation.
    pub async fn rnode_metadata(
        &self,
        attempt: MobileRNodeAttempt,
    ) -> Result<Option<rns_core::transport::iface::rnode::RNodeMetadata>, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(None);
        }
        Ok(Some(rnode.protocol.lock().await.metadata().clone()))
    }

    /// Poll and KISS-frame one outbound RNS packet as ordered bounded writes.
    pub async fn poll_rnode_bytes(
        &self,
        attempt: MobileRNodeAttempt,
    ) -> Result<Option<MobileRNodeWriteBatch>, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let mut attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(None);
        }
        if rnode.protocol.lock().await.phase() != RNodeProtocolPhase::Ready {
            return Ok(None);
        }
        let (handoff, packet) = if let Some(pending) = &attempts.pending_packet {
            if pending.offered_to.is_some() {
                return Ok(None);
            }
            (pending.handoff, pending.packet.clone())
        } else {
            let next_handoff_generation = attempts
                .next_handoff_generation
                .checked_add(1)
                .ok_or("RNode write handoff generation exhausted")?;
            let Some(packet) = self.poll_rnode_packet().await? else {
                return Ok(None);
            };
            let handoff = MobileRNodeWriteHandoff { generation: attempts.next_handoff_generation };
            attempts.next_handoff_generation = next_handoff_generation;
            attempts.pending_packet = Some(MobileRNodePendingPacket {
                handoff,
                packet: packet.clone(),
                offered_to: None,
            });
            (handoff, packet)
        };
        let frame = rnode
            .protocol
            .lock()
            .await
            .encode_packet(&packet)
            .map_err(|error| error.to_string())?;
        let pending = attempts.pending_packet.as_mut().ok_or("RNode write handoff was lost")?;
        if pending.handoff != handoff {
            return Err("RNode write handoff changed unexpectedly".into());
        }
        pending.offered_to = Some(attempt);
        Ok(Some(MobileRNodeWriteBatch {
            handoff,
            writes: fragment_rnode_writes(attempt.info, [frame]),
        }))
    }

    /// Remove one packet only after every platform write completed successfully.
    pub async fn complete_rnode_write(
        &self,
        attempt: MobileRNodeAttempt,
        handoff: MobileRNodeWriteHandoff,
    ) -> Result<bool, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let mut attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(false);
        }
        let completed = attempts.pending_packet.as_ref().is_some_and(|pending| {
            pending.handoff == handoff && pending.offered_to == Some(attempt)
        });
        if completed {
            attempts.pending_packet = None;
        }
        Ok(completed)
    }

    /// Release a failed platform write for bounded replay without removing its packet.
    pub async fn fail_rnode_write(
        &self,
        attempt: MobileRNodeAttempt,
        handoff: MobileRNodeWriteHandoff,
    ) -> Result<bool, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let mut attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(false);
        }
        let Some(pending) = attempts.pending_packet.as_mut() else {
            return Ok(false);
        };
        if pending.handoff != handoff || pending.offered_to != Some(attempt) {
            return Ok(false);
        }
        pending.offered_to = None;
        Ok(true)
    }

    /// End the current attempt and return bounded best-effort radio-off writes.
    pub async fn stop_rnode_bytes(
        &self,
        attempt: MobileRNodeAttempt,
        reason: MobileBearerReason,
    ) -> Result<Vec<Vec<u8>>, String> {
        let rnode = self.rnode.as_ref().ok_or("RNode channel is not configured")?;
        let mut attempts = rnode.attempts.lock().await;
        if attempts.active != Some(attempt) {
            return Ok(Vec::new());
        }
        let shutdown = rnode.protocol.lock().await.close();
        if let Some(pending) = attempts.pending_packet.as_mut()
            && pending.offered_to == Some(attempt)
        {
            pending.offered_to = None;
        }
        attempts.active = None;
        rnode.control.set_state(InterfaceState::Closed);
        self.platform_service
            .report(MobileBearerObservation {
                kind: attempt.bearer.observation_kind(),
                state: MobileBearerState::Disconnected,
                reason: Some(reason),
            })
            .await?;
        Ok(fragment_rnode_writes(attempt.info, [shutdown]))
    }

    /// Poll the propagation hub for queued messages.
    ///
    /// This is the core background task for iOS `BGAppRefreshTask`.
    /// Fetches all queued messages, persists them locally, ACKs the hub.
    /// Returns the count and preview of new messages for local notifications.
    ///
    /// Safe to call from a 30-second background window.
    pub async fn poll_hub(&self) -> Result<PollResult, String> {
        let result = self.poll_hub_inner().await;
        let severity = match &result {
            Ok(outcome)
                if outcome.batch_failure.is_none()
                    && outcome.items.iter().all(|item| {
                        !matches!(item.acknowledgement, PollAcknowledgementOutcome::Failed { .. })
                            && !matches!(item.local, PollLocalOutcome::StorageFailed { .. })
                    }) =>
            {
                styrene_ipc::types::MobileDiagnosticSeverity::Info
            }
            Ok(_) => styrene_ipc::types::MobileDiagnosticSeverity::Warning,
            Err(_) => styrene_ipc::types::MobileDiagnosticSeverity::Error,
        };
        self.record_diagnostic(
            styrene_ipc::types::MobileDiagnosticSource::Messaging,
            styrene_ipc::types::MobileDiagnosticStage::Inbound,
            severity,
            None,
        );
        result
    }

    async fn poll_hub_inner(&self) -> Result<PollResult, String> {
        let deadline = tokio::time::Instant::now() + LEGACY_HUB_POLL_DEADLINE;
        let hub_hash = self.hub_delivery_hash.as_deref().ok_or("no propagation hub configured")?;

        let my_delivery_hash = self
            .app_context
            .identity()
            .delivery_destination_hash()
            .ok_or("identity not configured — no delivery hash")?;

        // Fetch queued messages from hub
        let messages = tokio::time::timeout_at(
            deadline,
            self.app_context.fleet().propagation_fetch(hub_hash, &my_delivery_hash, Some(30)),
        )
        .await
        .map_err(|_| "fetch failed: poll deadline exceeded".to_string())?
        .map_err(|e| format!("fetch failed: {e}"))?;

        if messages.is_empty() {
            return Ok(PollResult {
                message_count: 0,
                messages: Vec::new(),
                items: Vec::new(),
                batch_failure: None,
            });
        }

        let pending = self.process_legacy_hub_batch(messages);
        let acknowledgement = if pending.eligible.is_empty() {
            Ok(())
        } else {
            let ids = pending.eligible.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
            match tokio::time::timeout_at(
                deadline,
                self.app_context.fleet().propagation_delete(hub_hash, &ids, Some(30)),
            )
            .await
            {
                Ok(result) => result.map_err(bounded_poll_error),
                Err(_) => Err("poll deadline exceeded during acknowledgement".into()),
            }
        };
        Ok(pending.complete(acknowledgement))
    }

    /// Decode and durably persist a fetched legacy hub batch before acknowledgement.
    ///
    /// Call [`LegacyHubPollBatch::acknowledgement_ids`] to perform the remote
    /// acknowledgement, then [`LegacyHubPollBatch::complete`] to obtain the
    /// typed per-item result.
    pub fn process_legacy_hub_batch(&self, messages: Vec<(String, Vec<u8>)>) -> LegacyHubPollBatch {
        let aggregate_bytes = messages.iter().fold(0usize, |total, (id, bytes)| {
            total.saturating_add(id.len()).saturating_add(bytes.len())
        });
        let batch_failure = if messages.len() > LEGACY_HUB_POLL_MAX_ITEMS {
            Some(PollBatchFailure::ItemLimitExceeded {
                limit: LEGACY_HUB_POLL_MAX_ITEMS,
                observed: messages.len(),
            })
        } else if aggregate_bytes > LEGACY_HUB_POLL_MAX_BYTES {
            Some(PollBatchFailure::ByteLimitExceeded {
                limit: LEGACY_HUB_POLL_MAX_BYTES,
                observed: aggregate_bytes,
            })
        } else {
            None
        };
        if let Some(batch_failure) = batch_failure {
            return LegacyHubPollBatch {
                result: PollResult {
                    message_count: 0,
                    messages: Vec::new(),
                    items: Vec::new(),
                    batch_failure: Some(batch_failure),
                },
                eligible: Vec::new(),
            };
        }
        let mut poll_messages = Vec::new();
        let mut items = Vec::with_capacity(messages.len());
        let mut eligible = Vec::new();

        for (hub_id, lxmf_bytes) in messages {
            let item_index = items.len();
            match self.app_context.messaging().accept_inbound(
                [0u8; 16], // destination filled by decoder from wire
                &lxmf_bytes,
                lxmf::inbound_decode::InboundPayloadMode::FullWire,
            ) {
                InboundAcceptOutcome::Accepted(record) => {
                    poll_messages.push(PollMessage {
                        source_hash: record.source.clone(),
                        content_preview: legacy_poll_preview(&record.content),
                        timestamp: record.timestamp,
                    });
                    eligible.push((item_index, hub_id.clone()));
                    items.push(PollItemOutcome {
                        hub_id,
                        local: PollLocalOutcome::Accepted { message_id: record.id },
                        acknowledgement: PollAcknowledgementOutcome::NotEligible,
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
                    eligible.push((item_index, hub_id.clone()));
                    items.push(PollItemOutcome {
                        hub_id,
                        local: PollLocalOutcome::DurableDuplicate { message_id },
                        acknowledgement: PollAcknowledgementOutcome::NotEligible,
                    });
                }
                InboundAcceptOutcome::Rejected { diagnostics } => {
                    let reason = bounded_poll_error(diagnostics.summary());
                    self.app_context.events().emit_inbound_drop(
                        "mobile_poll",
                        "malformed",
                        None,
                        None,
                        Some(&reason),
                    );
                    items.push(PollItemOutcome {
                        hub_id,
                        local: PollLocalOutcome::DecodeRejected { reason },
                        acknowledgement: PollAcknowledgementOutcome::NotEligible,
                    });
                }
                InboundAcceptOutcome::StorageError { message_id, error } => {
                    let error = bounded_poll_error(error);
                    self.app_context.events().emit_inbound_drop(
                        "mobile_poll",
                        "storage_error",
                        Some(&message_id),
                        None,
                        Some(&error),
                    );
                    items.push(PollItemOutcome {
                        hub_id,
                        local: PollLocalOutcome::StorageFailed {
                            message_id: (!message_id.is_empty()).then_some(message_id),
                            error,
                        },
                        acknowledgement: PollAcknowledgementOutcome::NotEligible,
                    });
                }
            }
        }

        let count = poll_messages.len();
        LegacyHubPollBatch {
            result: PollResult {
                message_count: count,
                messages: poll_messages,
                items,
                batch_failure: None,
            },
            eligible,
        }
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

    pub async fn start_conversation(
        &self,
        peer_hash: &str,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        DaemonMessaging::start_conversation(self.facade.as_ref(), peer_hash)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn conversation_page(
        &self,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<styrene_ipc::types::ConversationPage, String> {
        DaemonMessaging::query_conversation_page(self.facade.as_ref(), false, limit, cursor)
            .await
            .map_err(|error| error.to_string())
    }

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

    pub async fn identity_backup_metadata(
        &self,
    ) -> Result<styrene_ipc::types::IdentityBackupMetadata, styrene_ipc::IpcError> {
        self.facade.query_identity_backup_metadata().await
    }

    pub async fn export_identity_backup(
        &self,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, styrene_ipc::IpcError> {
        self.facade.export_identity_backup().await
    }

    /// Export the active identity as a portable Argon2id passphrase-protected artifact.
    pub async fn export_portable_identity_backup(
        &self,
        protection: &[u8],
    ) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError> {
        validate_identity_protection(protection)?;
        let protection = zeroize::Zeroizing::new(protection.to_vec());
        self.portable_backup_custody
            .as_ref()
            .ok_or(MobileIdentityRecoveryError::UnsupportedBackend)?
            .export(protection)
            .await
    }

    pub async fn restore_identity_backup(
        &self,
        backup: styrene_ipc::types::IdentityBackupImport,
    ) -> Result<styrene_ipc::types::IdentityRestoreOutcome, styrene_ipc::IpcError> {
        self.facade.restore_identity_backup(backup).await
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

#[cfg(feature = "mobile-identity")]
struct EncryptedFilePortableBackupCustody {
    path: PathBuf,
    key_material: zeroize::Zeroizing<Vec<u8>>,
    expected_identity_hash: String,
}

#[cfg(feature = "mobile-identity")]
#[async_trait::async_trait]
impl MobilePortableBackupCustody for EncryptedFilePortableBackupCustody {
    async fn export(
        &self,
        protection: zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError> {
        let path = self.path.clone();
        let key_material = self.key_material.clone();
        let expected_identity_hash = self.expected_identity_hash.clone();
        tokio::task::spawn_blocking(move || {
            let signer = styrene_identity::file_signer::FileSigner::with_static_passphrase(
                path,
                &key_material,
            );
            let root = signer
                .load(&key_material)
                .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
            portable_backup_export(&root, &protection, &expected_identity_hash)
        })
        .await
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
    }
}

#[cfg(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")))]
struct KeychainPortableBackupCustody {
    expected_identity_hash: String,
}

#[cfg(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")))]
#[async_trait::async_trait]
impl MobilePortableBackupCustody for KeychainPortableBackupCustody {
    async fn export(
        &self,
        protection: zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError> {
        use styrene_identity::IdentitySigner;

        let expected_identity_hash = self.expected_identity_hash.clone();
        let root = styrene_identity::keychain_signer::KeychainSigner::default()
            .root_secret()
            .await
            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
        tokio::task::spawn_blocking(move || {
            portable_backup_export(&root, &protection, &expected_identity_hash)
        })
        .await
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
    }
}

#[cfg(all(feature = "mobile-android-keystore", target_os = "android"))]
struct AndroidPortableBackupCustody {
    expected_identity_hash: String,
}

#[cfg(all(feature = "mobile-android-keystore", target_os = "android"))]
#[async_trait::async_trait]
impl MobilePortableBackupCustody for AndroidPortableBackupCustody {
    async fn export(
        &self,
        protection: zeroize::Zeroizing<Vec<u8>>,
    ) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError> {
        let expected_identity_hash = self.expected_identity_hash.clone();
        let signer = styrene_identity::android_keystore_signer::AndroidKeystoreSigner::new(
            styrene_identity::android_keystore_signer::SERVICE,
            styrene_identity::android_keystore_signer::ACCOUNT,
        )
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
        let root = signer
            .load_root_secret()
            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
        tokio::task::spawn_blocking(move || {
            portable_backup_export(&root, &protection, &expected_identity_hash)
        })
        .await
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
    }
}

#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn portable_backup_export(
    root: &styrene_identity::signer::RootSecret,
    protection: &[u8],
    expected_identity_hash: &str,
) -> Result<styrene_ipc::types::IdentityBackupExport, MobileIdentityRecoveryError> {
    let identity = private_identity_from_root(root)
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
    if hex::encode(identity.address_hash().as_slice()) != expected_identity_hash {
        return Err(MobileIdentityRecoveryError::IdentityConflict);
    }
    let backup =
        styrene_identity::vault::EncryptedIdentityBackup::protect_root_secret(root, protection)
            .map_err(map_mobile_recovery_vault_error)?;
    let mut exported = styrene_ipc::types::IdentityBackupExport::default();
    exported.metadata = identity_backup_metadata(backup.metadata());
    exported.encrypted_bytes = backup.encrypted_bytes().to_vec();
    Ok(exported)
}

fn validate_identity_protection(protection: &[u8]) -> Result<(), MobileIdentityRecoveryError> {
    if protection.is_empty() {
        return Err(MobileIdentityRecoveryError::ProtectionRequired);
    }
    if protection.len() > MAX_MOBILE_IDENTITY_PROTECTION_BYTES {
        return Err(MobileIdentityRecoveryError::ProtectionTooLarge);
    }
    Ok(())
}

async fn identity_presence(
    backend: IdentityBackend,
    paths: &PlatformPaths,
) -> Result<MobileIdentityPresence, MobileIdentityRecoveryError> {
    match backend {
        IdentityBackend::Keychain => {
            #[cfg(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")))]
            {
                let signer = styrene_identity::keychain_signer::KeychainSigner::default();
                signer
                    .presence()
                    .map(|present| {
                        if present {
                            MobileIdentityPresence::Present
                        } else {
                            MobileIdentityPresence::Absent
                        }
                    })
                    .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)
            }
            #[cfg(not(all(
                feature = "mobile-keychain",
                any(target_os = "macos", target_os = "ios")
            )))]
            Err(MobileIdentityRecoveryError::CustodyUnavailable)
        }
        IdentityBackend::AndroidKeystore => {
            #[cfg(all(feature = "mobile-android-keystore", target_os = "android"))]
            {
                let signer = styrene_identity::android_keystore_signer::AndroidKeystoreSigner::new(
                    styrene_identity::android_keystore_signer::SERVICE,
                    styrene_identity::android_keystore_signer::ACCOUNT,
                )
                .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                signer
                    .exists()
                    .map(|exists| {
                        if exists {
                            MobileIdentityPresence::Present
                        } else {
                            MobileIdentityPresence::Absent
                        }
                    })
                    .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)
            }
            #[cfg(not(all(feature = "mobile-android-keystore", target_os = "android")))]
            Err(MobileIdentityRecoveryError::CustodyUnavailable)
        }
        IdentityBackend::EncryptedFile | IdentityBackend::PlaintextFile => {
            Ok(if paths.identity_path().exists() {
                MobileIdentityPresence::Present
            } else {
                MobileIdentityPresence::Absent
            })
        }
    }
}

async fn restore_identity_before_boot_inner(
    config: &MobileConfig,
    backup: styrene_ipc::types::IdentityBackupImport,
    protection: &[u8],
    _encrypted_file_key_material: Option<&[u8]>,
) -> Result<styrene_ipc::types::IdentityRestoreOutcome, MobileIdentityRecoveryError> {
    validate_identity_protection(protection)?;
    if backup.encrypted_bytes.len() > MAX_MOBILE_IDENTITY_BACKUP_BYTES {
        return Err(MobileIdentityRecoveryError::ArtifactTooLarge);
    }
    #[cfg(any(
        feature = "mobile-identity",
        all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
        all(feature = "mobile-android-keystore", target_os = "android")
    ))]
    let recovered = {
        let protection = zeroize::Zeroizing::new(protection.to_vec());
        tokio::task::spawn_blocking(move || {
            let backup = styrene_identity::vault::EncryptedIdentityBackup::from_encrypted_bytes(
                backup.encrypted_bytes,
            )
            .map_err(map_mobile_recovery_vault_error)?;
            backup.decrypt_root_secret(&protection).map_err(map_mobile_recovery_vault_error)
        })
        .await
        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)??
    };
    #[cfg(not(any(
        feature = "mobile-identity",
        all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
        all(feature = "mobile-android-keystore", target_os = "android")
    )))]
    {
        let _ = (config, backup, _encrypted_file_key_material);
        Err(MobileIdentityRecoveryError::UnsupportedBackend)
    }

    #[cfg(any(
        feature = "mobile-identity",
        all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
        all(feature = "mobile-android-keystore", target_os = "android")
    ))]
    {
        match config.identity_backend {
            IdentityBackend::Keychain => {
                #[cfg(all(
                    feature = "mobile-keychain",
                    any(target_os = "macos", target_os = "ios")
                ))]
                {
                    use styrene_identity::IdentitySigner;

                    let signer = styrene_identity::keychain_signer::KeychainSigner::default();
                    let installed = {
                        let paths =
                            PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
                        let _guard = lock_mobile_identity_custody(&paths)
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                        if signer
                            .presence()
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
                        {
                            false
                        } else {
                            signer
                                .restore_root_secret(&recovered)
                                .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                            true
                        }
                    };
                    if installed {
                        Ok(styrene_ipc::types::IdentityRestoreOutcome::Restored)
                    } else {
                        let existing = signer
                            .root_secret()
                            .await
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                        same_recovered_identity(&existing, &recovered)
                    }
                }
                #[cfg(not(all(
                    feature = "mobile-keychain",
                    any(target_os = "macos", target_os = "ios")
                )))]
                Err(MobileIdentityRecoveryError::CustodyUnavailable)
            }
            IdentityBackend::AndroidKeystore => {
                #[cfg(all(feature = "mobile-android-keystore", target_os = "android"))]
                {
                    use styrene_identity::IdentitySigner;

                    let signer =
                        styrene_identity::android_keystore_signer::AndroidKeystoreSigner::new(
                            styrene_identity::android_keystore_signer::SERVICE,
                            styrene_identity::android_keystore_signer::ACCOUNT,
                        )
                        .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                    let installed = {
                        let paths =
                            PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
                        let _guard = lock_mobile_identity_custody(&paths)
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                        if signer
                            .exists()
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
                        {
                            false
                        } else {
                            signer
                                .restore_root_secret(&recovered)
                                .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                            true
                        }
                    };
                    if installed {
                        Ok(styrene_ipc::types::IdentityRestoreOutcome::Restored)
                    } else {
                        let existing = signer
                            .root_secret()
                            .await
                            .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                        same_recovered_identity(&existing, &recovered)
                    }
                }
                #[cfg(not(all(feature = "mobile-android-keystore", target_os = "android")))]
                Err(MobileIdentityRecoveryError::CustodyUnavailable)
            }
            IdentityBackend::EncryptedFile => {
                #[cfg(feature = "mobile-identity")]
                {
                    let key_material = _encrypted_file_key_material
                        .filter(|material| !material.is_empty())
                        .ok_or(MobileIdentityRecoveryError::CustodyUnavailable)?;
                    let paths =
                        PlatformPaths::new(config.config_dir.clone(), config.data_dir.clone());
                    let path = paths.identity_path();
                    let key_material = zeroize::Zeroizing::new(key_material.to_vec());
                    tokio::task::spawn_blocking(move || {
                        let signer =
                            styrene_identity::file_signer::FileSigner::with_static_passphrase(
                                path,
                                &key_material,
                            );
                        if signer.exists() {
                            let existing = signer
                                .load(&key_material)
                                .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?;
                            return same_recovered_identity(&existing, &recovered);
                        }
                        signer.restore_root_secret(&recovered).map_err(|error| match error {
                            styrene_identity::signer::SignerError::Io(io)
                                if io.kind() == std::io::ErrorKind::AlreadyExists =>
                            {
                                MobileIdentityRecoveryError::IdentityConflict
                            }
                            _ => MobileIdentityRecoveryError::CustodyUnavailable,
                        })?;
                        Ok(styrene_ipc::types::IdentityRestoreOutcome::Restored)
                    })
                    .await
                    .map_err(|_| MobileIdentityRecoveryError::CustodyUnavailable)?
                }
                #[cfg(not(feature = "mobile-identity"))]
                Err(MobileIdentityRecoveryError::CustodyUnavailable)
            }
            IdentityBackend::PlaintextFile => Err(MobileIdentityRecoveryError::UnsupportedBackend),
        }
    }
}

#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn same_recovered_identity(
    existing: &styrene_identity::signer::RootSecret,
    recovered: &styrene_identity::signer::RootSecret,
) -> Result<styrene_ipc::types::IdentityRestoreOutcome, MobileIdentityRecoveryError> {
    if bool::from(existing.as_bytes().ct_eq(recovered.as_bytes())) {
        Ok(styrene_ipc::types::IdentityRestoreOutcome::AlreadyPresent)
    } else {
        Err(MobileIdentityRecoveryError::IdentityConflict)
    }
}

#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn map_mobile_recovery_vault_error(
    error: styrene_identity::vault::VaultError,
) -> MobileIdentityRecoveryError {
    use styrene_identity::vault::VaultError;

    match error {
        VaultError::ProtectionRequired => MobileIdentityRecoveryError::ProtectionRequired,
        VaultError::InvalidBackup => MobileIdentityRecoveryError::InvalidBackup,
        VaultError::BackupAuthenticationFailed => MobileIdentityRecoveryError::AuthenticationFailed,
        VaultError::IdentityConflict => MobileIdentityRecoveryError::IdentityConflict,
        _ => MobileIdentityRecoveryError::CustodyUnavailable,
    }
}

#[cfg(feature = "mobile-identity")]
struct EncryptedFileBackupCustody {
    vault: styrene_identity::vault::IdentityVault,
}

#[cfg(feature = "mobile-identity")]
impl crate::services::identity::IdentityBackupCustody for EncryptedFileBackupCustody {
    fn metadata(
        &self,
    ) -> Result<styrene_ipc::types::IdentityBackupMetadata, styrene_ipc::IpcError> {
        self.vault
            .encrypted_backup_metadata()
            .map(identity_backup_metadata)
            .map_err(identity_backup_error)
    }

    fn export(&self) -> Result<styrene_ipc::types::IdentityBackupExport, styrene_ipc::IpcError> {
        let backup = self.vault.export_encrypted_backup().map_err(identity_backup_error)?;
        let mut exported = styrene_ipc::types::IdentityBackupExport::default();
        exported.metadata = identity_backup_metadata(backup.metadata());
        exported.encrypted_bytes = backup.encrypted_bytes().to_vec();
        Ok(exported)
    }

    fn restore(
        &self,
        backup: styrene_ipc::types::IdentityBackupImport,
    ) -> Result<styrene_ipc::types::IdentityRestoreOutcome, styrene_ipc::IpcError> {
        use styrene_identity::vault::IdentityRestoreOutcome;

        let backup = styrene_identity::vault::EncryptedIdentityBackup::from_encrypted_bytes(
            backup.encrypted_bytes,
        )
        .map_err(identity_backup_error)?;
        match self.vault.restore_encrypted_backup(&backup).map_err(identity_backup_error)? {
            IdentityRestoreOutcome::Restored => {
                Ok(styrene_ipc::types::IdentityRestoreOutcome::Restored)
            }
            IdentityRestoreOutcome::AlreadyPresent => {
                Ok(styrene_ipc::types::IdentityRestoreOutcome::AlreadyPresent)
            }
        }
    }
}

fn identity_backup_custody(
    backend: IdentityBackend,
    paths: &PlatformPaths,
    key_material: Option<&[u8]>,
) -> Option<Arc<dyn crate::services::identity::IdentityBackupCustody>> {
    #[cfg(feature = "mobile-identity")]
    if backend == IdentityBackend::EncryptedFile {
        let key_material = key_material.filter(|material| !material.is_empty())?;
        return Some(Arc::new(EncryptedFileBackupCustody {
            vault: styrene_identity::vault::IdentityVault::new(
                paths.identity_path(),
                Box::new(styrene_identity::file_signer::StaticPassphraseProvider::new(
                    key_material,
                )),
            ),
        }));
    }
    let _ = (backend, paths, key_material);
    None
}

#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn identity_backup_metadata(
    metadata: styrene_identity::vault::EncryptedIdentityBackupMetadata,
) -> styrene_ipc::types::IdentityBackupMetadata {
    use styrene_identity::vault::EncryptedIdentityBackupFormat;

    let mut projected = styrene_ipc::types::IdentityBackupMetadata::default();
    projected.contract_version = metadata.contract_version;
    projected.format = match metadata.format {
        EncryptedIdentityBackupFormat::LegacyV0 => {
            styrene_ipc::types::IdentityBackupFormat::LegacyV0
        }
        EncryptedIdentityBackupFormat::StidV1 => styrene_ipc::types::IdentityBackupFormat::StidV1,
    };
    projected.encrypted_size = metadata.encrypted_size;
    projected
}

#[cfg(feature = "mobile-identity")]
fn identity_backup_error(error: styrene_identity::vault::VaultError) -> styrene_ipc::IpcError {
    use styrene_identity::vault::VaultError;

    match error {
        VaultError::InvalidBackup => styrene_ipc::IpcError::invalid_request(
            "invalid or unsupported encrypted identity backup",
        ),
        VaultError::BackupAuthenticationFailed => styrene_ipc::IpcError::invalid_request(
            "encrypted identity backup authentication failed",
        ),
        VaultError::IdentityConflict => styrene_ipc::IpcError::Conflict {
            message: "identity restore conflicts with existing custody".into(),
        },
        VaultError::CustodyUnavailable => styrene_ipc::IpcError::Unavailable {
            reason: "identity backup custody unavailable".into(),
        },
        _ => styrene_ipc::IpcError::Unavailable {
            reason: "identity backup custody unavailable".into(),
        },
    }
}

/// Load or create an RNS identity using the configured backend.
///
/// On first launch, creates a new identity seamlessly — no passphrase prompts
/// on keychain backends, no manual key management. The user just opens the app.
async fn load_or_create_identity(
    backend: &IdentityBackend,
    paths: &PlatformPaths,
    encrypted_file_key_material: Option<&[u8]>,
) -> anyhow::Result<LoadedMobileIdentity> {
    match backend {
        IdentityBackend::Keychain => load_or_create_keychain(paths).await,
        IdentityBackend::AndroidKeystore => load_or_create_android_keystore(paths).await,
        IdentityBackend::EncryptedFile => {
            load_or_create_encrypted_file(paths, encrypted_file_key_material).await
        }
        IdentityBackend::PlaintextFile => load_or_create_plaintext_file(paths)
            .map(|identity| LoadedMobileIdentity { identity, portable_backup_custody: None }),
    }
}

async fn load_or_create_android_keystore(
    _paths: &PlatformPaths,
) -> anyhow::Result<LoadedMobileIdentity> {
    #[cfg(all(feature = "mobile-android-keystore", target_os = "android"))]
    {
        use styrene_identity::android_keystore_signer::AndroidKeystoreSigner;

        let signer = AndroidKeystoreSigner::new(
            styrene_identity::android_keystore_signer::SERVICE,
            styrene_identity::android_keystore_signer::ACCOUNT,
        )?;
        let root = {
            let _guard = lock_mobile_identity_custody(_paths)
                .map_err(|_| anyhow::anyhow!("Android Keystore custody lock unavailable"))?;
            signer.load_or_create_root_secret()?
        };
        let identity = private_identity_from_root(&root)?;
        let expected_identity_hash = hex::encode(identity.address_hash().as_slice());
        Ok(LoadedMobileIdentity {
            identity,
            portable_backup_custody: Some(Arc::new(AndroidPortableBackupCustody {
                expected_identity_hash,
            })),
        })
    }

    #[cfg(not(all(feature = "mobile-android-keystore", target_os = "android")))]
    Err(MobileCustodyError::BackendUnavailable { backend: "Android Keystore" }.into())
}

#[cfg(any(
    feature = "mobile-identity",
    all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")),
    all(feature = "mobile-android-keystore", target_os = "android")
))]
fn private_identity_from_root(
    root: &styrene_identity::signer::RootSecret,
) -> anyhow::Result<PrivateIdentity> {
    use styrene_identity::{KeyDeriver, KeyPurpose};

    let deriver = KeyDeriver::new(root.as_bytes());
    let encryption_seed = deriver.derive(KeyPurpose::RnsEncryption);
    let signing_seed = deriver.derive(KeyPurpose::Signing);
    let mut key_bytes = [0_u8; 64];
    key_bytes[..32].copy_from_slice(&encryption_seed);
    key_bytes[32..].copy_from_slice(&signing_seed);
    PrivateIdentity::from_private_key_bytes(&key_bytes)
        .map_err(|error| anyhow::anyhow!("key derivation: {error:?}"))
}

/// Keychain backend: root secret in platform keychain → HKDF → RNS keys.
///
/// On iOS: the device's first passcode unlock after restart gates access.
/// On macOS: the login Keychain provides the equivalent device protection.
async fn load_or_create_keychain(_paths: &PlatformPaths) -> anyhow::Result<LoadedMobileIdentity> {
    #[cfg(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios")))]
    {
        use styrene_identity::IdentitySigner;
        use styrene_identity::keychain_signer::{
            KeychainSigner, LEGACY_BIOMETRIC_ACCOUNT, SERVICE,
        };

        let signer = KeychainSigner::default();
        let root = {
            let _guard = lock_mobile_identity_custody(_paths)
                .map_err(|_| anyhow::anyhow!("Keychain custody lock unavailable"))?;
            if signer.presence().map_err(|e| anyhow::anyhow!("keychain lookup: {e}"))? {
                signer.root_secret().await.map_err(|e| anyhow::anyhow!("keychain access: {e}"))?
            } else {
                let legacy = KeychainSigner::new(SERVICE, LEGACY_BIOMETRIC_ACCOUNT);
                if legacy.presence().map_err(|e| anyhow::anyhow!("legacy keychain lookup: {e}"))? {
                    let root = legacy
                        .root_secret()
                        .await
                        .map_err(|e| anyhow::anyhow!("legacy keychain access: {e}"))?;
                    signer
                        .create_from_root_secret(&root)
                        .map_err(|e| anyhow::anyhow!("keychain migration: {e}"))?;
                    crate::daemon_diagnostic!(
                        "[mobile] migrated identity to first-unlock keychain policy"
                    );
                    root
                } else {
                    let root = signer
                        .create_root_secret()
                        .map_err(|e| anyhow::anyhow!("keychain create: {e}"))?;
                    crate::daemon_diagnostic!("[mobile] created new identity in platform keychain");
                    root
                }
            }
        };

        let identity = private_identity_from_root(&root)?;
        let expected_identity_hash = hex::encode(identity.address_hash().as_slice());
        Ok(LoadedMobileIdentity {
            identity,
            portable_backup_custody: Some(Arc::new(KeychainPortableBackupCustody {
                expected_identity_hash,
            })),
        })
    }

    #[cfg(not(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios"))))]
    Err(MobileCustodyError::BackendUnavailable { backend: "Keychain" }.into())
}

/// Encrypted file backend: argon2id + ChaCha20Poly1305 encrypted root secret.
///
/// Requires a passphrase — the host app must provide it via a prompt.
/// Less seamless than keychain but works on any platform.
async fn load_or_create_encrypted_file(
    paths: &PlatformPaths,
    key_material: Option<&[u8]>,
) -> anyhow::Result<LoadedMobileIdentity> {
    let key_material = key_material
        .filter(|material| !material.is_empty())
        .ok_or(MobileCustodyError::KeyMaterialRequired)?;
    #[cfg(not(feature = "mobile-identity"))]
    let _ = (paths, key_material);
    #[cfg(feature = "mobile-identity")]
    {
        use styrene_identity::{IdentitySigner, KeyDeriver, KeyPurpose};

        let identity_path = paths.identity_path();

        let signer = styrene_identity::file_signer::FileSigner::new(
            identity_path.clone(),
            Box::new(styrene_identity::file_signer::StaticPassphraseProvider::new(key_material)),
        );

        let created = !identity_path.exists();
        if created {
            signer
                .generate(key_material)
                .map_err(|e| anyhow::anyhow!("encrypted file create: {e}"))?;
        }
        let root = signer
            .root_secret()
            .await
            .map_err(|e| anyhow::anyhow!("encrypted file access: {e}"))?;

        if created {
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

        let identity = PrivateIdentity::from_private_key_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("key derivation: {e:?}"))?;
        let expected_identity_hash = hex::encode(identity.address_hash().as_slice());
        Ok(LoadedMobileIdentity {
            identity,
            portable_backup_custody: Some(Arc::new(EncryptedFilePortableBackupCustody {
                path: identity_path,
                key_material: zeroize::Zeroizing::new(key_material.to_vec()),
                expected_identity_hash,
            })),
        })
    }

    #[cfg(not(feature = "mobile-identity"))]
    Err(MobileCustodyError::BackendUnavailable { backend: "encrypted-file" }.into())
}

fn active_custody(backend: IdentityBackend) -> styrene_ipc::types::IdentityCustodyInfo {
    use styrene_ipc::types::{
        IdentityCustodyAuthentication as Authentication,
        IdentityCustodyAvailability as Availability, IdentityCustodyBackend as Backend,
        IdentityCustodyDowngrade as Downgrade, IdentityCustodyInfo,
        IdentityCustodyProtection as Protection,
    };

    let (backend, protection, authentication) = match backend {
        IdentityBackend::Keychain => {
            (Backend::Keychain, Protection::PlatformProtected, Authentication::DeviceAuthentication)
        }
        IdentityBackend::AndroidKeystore => {
            (Backend::AndroidKeystore, Protection::PlatformProtected, Authentication::None)
        }
        IdentityBackend::EncryptedFile => {
            (Backend::EncryptedFile, Protection::EncryptedAtRest, Authentication::HostKeyMaterial)
        }
        IdentityBackend::PlaintextFile => {
            (Backend::PlaintextFile, Protection::DevelopmentPlaintext, Authentication::None)
        }
    };
    IdentityCustodyInfo {
        requested_backend: backend,
        active_backend: Some(backend),
        protection: Some(protection),
        authentication,
        availability: Availability::Available,
        downgrade: Downgrade::None,
        failure: None,
    }
}

fn load_public_identity_metadata(
    path: &std::path::Path,
) -> anyhow::Result<crate::services::identity::PublicIdentityMetadata> {
    if !path.exists() {
        return Ok(crate::services::identity::PublicIdentityMetadata::default());
    }
    let bytes = std::fs::read(path)
        .map_err(|error| anyhow::anyhow!("read public identity metadata: {error}"))?;
    let metadata: crate::services::identity::PublicIdentityMetadata =
        serde_json::from_slice(&bytes)
            .map_err(|error| anyhow::anyhow!("invalid public identity metadata: {error}"))?;
    for (kind, value) in [
        ("display name", metadata.display_name.as_deref()),
        ("icon", metadata.icon.as_deref()),
        ("short name", metadata.short_name.as_deref()),
    ] {
        if let Some(value) = value {
            crate::services::identity::validate_public_field(kind, value)
                .map_err(anyhow::Error::msg)?;
        }
    }
    Ok(metadata)
}

fn persist_public_identity_metadata(
    path: &std::path::Path,
    metadata: &crate::services::identity::PublicIdentityMetadata,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(metadata)?;
    atomic_write_private(path, &bytes)
        .map_err(|error| anyhow::anyhow!("persist public identity metadata: {error}"))
}

/// Plaintext file backend: 64-byte raw identity on disk.
///
/// For development and testing only. NOT secure for production mobile.
fn load_or_create_plaintext_file(paths: &PlatformPaths) -> anyhow::Result<PrivateIdentity> {
    let identity_path = paths.identity_path();
    match load_plaintext_identity(&identity_path) {
        Ok(identity) => return Ok(identity),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let identity = PrivateIdentity::new_from_rand(rand_core::OsRng);
    let parent = identity_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("plaintext identity path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = identity_path.file_name().and_then(|name| name.to_str()).unwrap_or("identity");
    let (temporary_path, mut file) = loop {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{file_name}.create-{}-{sequence}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };

    use std::io::Write;
    let staged = file.write_all(&identity.to_private_key_bytes()).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error.into());
    }

    match std::fs::hard_link(&temporary_path, &identity_path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&temporary_path);
            sync_plaintext_identity_directory(parent)?;
            crate::daemon_diagnostic!(
                "[mobile] created new plaintext identity at {}",
                identity_path.display()
            );
            Ok(identity)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&temporary_path);
            load_plaintext_identity(&identity_path).map_err(Into::into)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            Err(error.into())
        }
    }
}

fn load_plaintext_identity(path: &std::path::Path) -> std::io::Result<PrivateIdentity> {
    let bytes = std::fs::read(path)?;
    PrivateIdentity::from_private_key_bytes(&bytes).map_err(|error| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid identity: {error:?}"))
    })
}

#[cfg(unix)]
fn sync_plaintext_identity_directory(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_plaintext_identity_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mesh_transport::TransportLifecycleEvent;
    use crate::transport::mock_transport::{MockCall, MockTransport};
    use rns_core::destination::{DestinationName, SingleOutputDestination};
    use rns_core::hash::AddressHash;
    use rns_core::transport::core_transport::{ReceivedData, ReceivedPayloadMode};

    #[test]
    fn android_keystore_custody_reports_wrapping_without_device_authentication() {
        use styrene_ipc::types::{
            IdentityCustodyAuthentication, IdentityCustodyBackend, IdentityCustodyProtection,
        };

        let custody = active_custody(IdentityBackend::AndroidKeystore);

        assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::AndroidKeystore));
        assert_eq!(custody.protection, Some(IdentityCustodyProtection::PlatformProtected));
        assert_eq!(custody.authentication, IdentityCustodyAuthentication::None);
    }

    #[test]
    fn keychain_custody_reports_device_authentication() {
        use styrene_ipc::types::{
            IdentityCustodyAuthentication, IdentityCustodyBackend, IdentityCustodyProtection,
        };

        let custody = active_custody(IdentityBackend::Keychain);

        assert_eq!(custody.active_backend, Some(IdentityCustodyBackend::Keychain));
        assert_eq!(custody.protection, Some(IdentityCustodyProtection::PlatformProtected));
        assert_eq!(custody.authentication, IdentityCustodyAuthentication::DeviceAuthentication);
    }

    #[test]
    fn concurrent_plaintext_creators_converge_on_one_durable_private_identity() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let creators = (0..16)
            .map(|_| {
                let paths = paths.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    load_or_create_plaintext_file(&paths).unwrap().to_private_key_bytes()
                })
            })
            .collect::<Vec<_>>();

        barrier.wait();
        let identities =
            creators.into_iter().map(|creator| creator.join().unwrap()).collect::<Vec<_>>();
        let durable = std::fs::read(paths.identity_path()).unwrap();

        assert_eq!(durable.len(), 64);
        assert!(identities.iter().all(|identity| identity.as_slice() == durable));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(paths.identity_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn android_keystore_backend_never_falls_back_to_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();

        let Err(error) =
            load_or_create_identity(&IdentityBackend::AndroidKeystore, &paths, None).await
        else {
            panic!("unsupported Android Keystore backend created an identity");
        };

        assert!(error.to_string().contains("unavailable in this build"));
        assert!(!paths.identity_path().exists());
    }

    #[cfg(not(all(feature = "mobile-keychain", any(target_os = "macos", target_os = "ios"))))]
    #[tokio::test]
    async fn keychain_backend_never_falls_back_to_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();

        let result = load_or_create_identity(&IdentityBackend::Keychain, &paths, None).await;

        assert!(result.is_err());
        assert!(!paths.identity_path().exists());
    }

    #[tokio::test]
    async fn encrypted_file_backend_rejects_missing_key_material_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();

        for key_material in [None, Some([].as_slice())] {
            let result =
                load_or_create_identity(&IdentityBackend::EncryptedFile, &paths, key_material)
                    .await;
            assert!(result.is_err());
        }
        assert!(!paths.identity_path().exists());
    }

    #[cfg(not(feature = "mobile-identity"))]
    #[tokio::test]
    async fn unavailable_encrypted_file_backend_never_falls_back_to_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();

        let result =
            load_or_create_identity(&IdentityBackend::EncryptedFile, &paths, Some(b"host-key"))
                .await;

        assert!(result.is_err());
        assert!(!paths.identity_path().exists());
    }

    #[cfg(feature = "mobile-identity")]
    #[tokio::test]
    async fn encrypted_file_backend_uses_host_key_and_restores_identity() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();

        let first = load_or_create_identity(
            &IdentityBackend::EncryptedFile,
            &paths,
            Some(b"host-owned-test-key"),
        )
        .await
        .unwrap();
        let second = load_or_create_identity(
            &IdentityBackend::EncryptedFile,
            &paths,
            Some(b"host-owned-test-key"),
        )
        .await
        .unwrap();

        assert_eq!(first.identity.address_hash(), second.identity.address_hash());
        assert_ne!(std::fs::read(paths.identity_path()).unwrap().len(), 64);
    }

    #[cfg(feature = "mobile-identity")]
    #[tokio::test]
    async fn encrypted_backup_mobile_operations_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let config = MobileConfig {
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: Some("Backup Test".into()),
            identity_backend: IdentityBackend::EncryptedFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };

        let first =
            MobileNode::boot_with_encrypted_file_key(config.clone(), b"host-owned-backup-key")
                .await
                .unwrap();
        let exported = first.export_identity_backup().await.unwrap();
        assert_eq!(first.identity_backup_metadata().await.unwrap(), exported.metadata);
        let mut imported = styrene_ipc::types::IdentityBackupImport::default();
        imported.encrypted_bytes = exported.encrypted_bytes.clone();
        assert_eq!(
            first.restore_identity_backup(imported).await.unwrap(),
            styrene_ipc::types::IdentityRestoreOutcome::AlreadyPresent
        );
        first.shutdown().await.unwrap();
        drop(first);

        let restarted = MobileNode::boot_with_encrypted_file_key(config, b"host-owned-backup-key")
            .await
            .unwrap();
        let after_restart = restarted.export_identity_backup().await.unwrap();
        assert_eq!(after_restart.metadata, exported.metadata);
        assert_eq!(after_restart.encrypted_bytes, exported.encrypted_bytes);
        restarted.shutdown().await.unwrap();
    }

    #[cfg(feature = "mobile-identity")]
    #[tokio::test]
    async fn portable_backup_restores_before_create_under_a_different_host_key() {
        let source_temp = tempfile::tempdir().unwrap();
        let target_temp = tempfile::tempdir().unwrap();
        let config_for = |root: &std::path::Path| MobileConfig {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: None,
            identity_backend: IdentityBackend::EncryptedFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };
        let source_config = config_for(source_temp.path());
        let target_config = config_for(target_temp.path());
        let source =
            MobileNode::boot_with_encrypted_file_key(source_config, b"source-device-host-key")
                .await
                .unwrap();
        let source_identity = source.app_context.identity().identity_hash().to_owned();
        let exported = source
            .export_portable_identity_backup(b"user-entered recovery passphrase")
            .await
            .unwrap();
        let exported_bytes = exported.encrypted_bytes.clone();
        source.shutdown().await.unwrap();

        assert_eq!(
            MobileNode::identity_presence(&target_config).await.unwrap(),
            MobileIdentityPresence::Absent
        );
        let mut imported = styrene_ipc::types::IdentityBackupImport::default();
        imported.encrypted_bytes = exported.encrypted_bytes;
        assert_eq!(
            MobileNode::restore_identity_before_boot_with_encrypted_file_key(
                &target_config,
                imported,
                b"user-entered recovery passphrase",
                b"different-target-host-key",
            )
            .await
            .unwrap(),
            styrene_ipc::types::IdentityRestoreOutcome::Restored
        );
        let mut imported = styrene_ipc::types::IdentityBackupImport::default();
        imported.encrypted_bytes = exported_bytes;
        assert_eq!(
            MobileNode::restore_identity_before_boot_with_encrypted_file_key(
                &target_config,
                imported,
                b"user-entered recovery passphrase",
                b"different-target-host-key",
            )
            .await
            .unwrap(),
            styrene_ipc::types::IdentityRestoreOutcome::AlreadyPresent
        );
        let target =
            MobileNode::boot_with_encrypted_file_key(target_config, b"different-target-host-key")
                .await
                .unwrap();
        assert_eq!(target.app_context.identity().identity_hash(), source_identity);
        target.shutdown().await.unwrap();
    }

    #[cfg(feature = "mobile-identity")]
    #[tokio::test]
    async fn failed_portable_restore_does_not_create_identity() {
        let source_temp = tempfile::tempdir().unwrap();
        let target_temp = tempfile::tempdir().unwrap();
        let config_for = |root: &std::path::Path| MobileConfig {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: None,
            identity_backend: IdentityBackend::EncryptedFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };
        let source = MobileNode::boot_with_encrypted_file_key(
            config_for(source_temp.path()),
            b"source-host-key",
        )
        .await
        .unwrap();
        let exported = source.export_portable_identity_backup(b"correct protection").await.unwrap();
        source.shutdown().await.unwrap();
        let target_config = config_for(target_temp.path());
        let mut imported = styrene_ipc::types::IdentityBackupImport::default();
        imported.encrypted_bytes = exported.encrypted_bytes;

        assert_eq!(
            MobileNode::restore_identity_before_boot_with_encrypted_file_key(
                &target_config,
                imported,
                b"wrong protection",
                b"target-host-key",
            )
            .await
            .unwrap_err(),
            MobileIdentityRecoveryError::AuthenticationFailed
        );
        assert_eq!(
            MobileNode::identity_presence(&target_config).await.unwrap(),
            MobileIdentityPresence::Absent
        );
    }

    #[cfg(feature = "mobile-identity")]
    #[tokio::test]
    async fn portable_export_rejects_custody_changed_after_boot() {
        let temp = tempfile::tempdir().unwrap();
        let config = MobileConfig {
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: None,
            identity_backend: IdentityBackend::EncryptedFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };
        let node = MobileNode::boot_with_encrypted_file_key(config, b"host-key").await.unwrap();
        let identity_path = node.paths().identity_path();
        std::fs::remove_file(&identity_path).unwrap();
        styrene_identity::file_signer::FileSigner::with_static_passphrase(
            identity_path,
            b"host-key",
        )
        .generate(b"host-key")
        .unwrap();

        assert_eq!(
            node.export_portable_identity_backup(b"user passphrase").await.unwrap_err(),
            MobileIdentityRecoveryError::IdentityConflict
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn portable_restore_rejects_bounds_before_custody_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let config = MobileConfig {
            config_dir: temp.path().join("config"),
            data_dir: temp.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: None,
            identity_backend: IdentityBackend::EncryptedFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };
        assert_eq!(
            MobileNode::restore_identity_before_boot(
                &config,
                styrene_ipc::types::IdentityBackupImport::default(),
                b"",
            )
            .await
            .unwrap_err(),
            MobileIdentityRecoveryError::ProtectionRequired
        );
        let mut oversized = styrene_ipc::types::IdentityBackupImport::default();
        oversized.encrypted_bytes = vec![0; MAX_MOBILE_IDENTITY_BACKUP_BYTES + 1];
        assert_eq!(
            MobileNode::restore_identity_before_boot(&config, oversized, b"protection")
                .await
                .unwrap_err(),
            MobileIdentityRecoveryError::ArtifactTooLarge
        );
        assert!(!PlatformPaths::new(config.config_dir, config.data_dir).identity_path().exists());
    }
    use styrene_ipc::types::DaemonEvent;
    use tokio::time::{Duration, timeout};

    #[test]
    fn propagation_snapshot_deserializes_without_additive_telemetry_fields() {
        let snapshot: MobilePropagationSnapshot = serde_json::from_value(serde_json::json!({
            "generation": 1,
            "observed_at": 2,
            "selected_destination": null,
            "readiness": "unselected",
            "ready": false,
            "selected_policy": null,
            "candidates": [],
            "sync_state": "idle",
            "new_messages": 0,
            "in_flight": null,
            "failure": null,
            "automatic_sync_enabled": true,
            "automatic_sync_cooldown_secs": 30,
            "sync_deadline_secs": 32
        }))
        .unwrap();

        assert!(snapshot.trigger_capabilities.is_empty());
        assert_eq!(snapshot.active_trigger, None);
        assert_eq!(snapshot.active_sync_started_at, None);
        assert_eq!(snapshot.last_synchronization, None);
        assert_eq!(snapshot.cooldown_remaining_secs, 0);
    }

    fn test_rnode_info(bearer: MobileRNodeBearer, max_write_size: usize) -> RNodeBearerInfo {
        RNodeBearerInfo {
            kind: match bearer {
                MobileRNodeBearer::BluetoothLe => RNodeBearerKind::Ble,
                MobileRNodeBearer::AndroidUsb => RNodeBearerKind::AndroidUsb,
            },
            negotiated_mtu: None,
            max_write_size: Some(max_write_size),
        }
    }

    async fn ready_test_rnode(
        node: &MobileNode,
        bearer: MobileRNodeBearer,
    ) -> MobileRNodeByteStart {
        use rns_core::transport::iface::kiss::kiss_encode_command;
        use rns_core::transport::iface::rnode::{
            CMD_BANDWIDTH, CMD_CODING_RATE, CMD_DETECT, CMD_FREQUENCY, CMD_RADIO_STATE,
            CMD_SPREADING_FACTOR, CMD_TX_POWER, RNodeRadioProfile,
        };

        match bearer {
            MobileRNodeBearer::BluetoothLe => {
                node.platform_service().set_bluetooth_approved(true).await;
            }
            MobileRNodeBearer::AndroidUsb => {
                assert_eq!(
                    node.platform_service().request_android_usb_fallback().await,
                    MobileUsbFallbackDisposition::Accepted
                );
            }
        }
        let start = node.start_rnode_bytes(bearer, test_rnode_info(bearer, 512)).await.unwrap();
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let configured = [
            kiss_encode_command(CMD_DETECT, &[0x46]),
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[1]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &configured).await.unwrap();
        start
    }

    async fn poll_test_handoff(
        node: &MobileNode,
        attempt: MobileRNodeAttempt,
    ) -> MobileRNodeWriteBatch {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(batch) = node.poll_rnode_bytes(attempt).await.unwrap() {
                    break batch;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

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
            MobileIdentityRuntime {
                metadata: crate::services::identity::PublicIdentityMetadata {
                    display_name,
                    ..Default::default()
                },
                metadata_path: PathBuf::from("test-config/identity-public.json"),
                custody: active_custody(IdentityBackend::PlaintextFile),
                backup_custody: None,
                portable_backup_custody: None,
            },
            None,
            None,
        )
        .await
        .unwrap()
    }

    async fn compose_with_mock_identity_state(
        mock: Arc<MockTransport>,
        paths: PlatformPaths,
        identity: PrivateIdentity,
        metadata: crate::services::identity::PublicIdentityMetadata,
    ) -> MobileNode {
        let delivery_hash = hex::encode(mock.destination_hash().as_slice());
        compose_mobile_node(
            paths.clone(),
            identity,
            MobileStores {
                messages: Arc::new(Mutex::new(MessagesStore::in_memory().unwrap())),
                nodes: Arc::new(NodeStore::in_memory().unwrap()),
            },
            MobileTransportRuntime {
                transport: mock,
                delivery_hash: Some(delivery_hash),
                tcp_listen_addresses: Vec::new(),
                service_receipt_target: None,
                rnode_channel: None,
            },
            MobileIdentityRuntime {
                metadata,
                metadata_path: paths.config_dir.join("identity-public.json"),
                custody: active_custody(IdentityBackend::PlaintextFile),
                backup_custody: None,
                portable_backup_custody: None,
            },
            None,
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn restarted_node_announces_restored_metadata_and_invalid_edit_is_silent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = PlatformPaths::new(temp.path().join("config"), temp.path().join("data"));
        paths.ensure_dirs().unwrap();
        let metadata_path = paths.config_dir.join("identity-public.json");
        let first_identity = load_or_create_plaintext_file(&paths).unwrap();
        let identity_hash = first_identity.address_hash().to_owned();
        let first_mock = Arc::new(MockTransport::new_default());
        let first = compose_with_mock_identity_state(
            first_mock.clone(),
            paths.clone(),
            first_identity,
            crate::services::identity::PublicIdentityMetadata::default(),
        )
        .await;

        assert!(
            first
                .facade
                .set_identity(Some("  Field Node  "), Some("radio"), Some("FN"))
                .await
                .unwrap()
        );
        let expected = encode_delivery_display_name_app_data("Field Node");
        assert!(matches!(
            first_mock.calls().as_slice(),
            [MockCall::Announce { app_data }] if app_data == &expected
        ));
        first.shutdown().await.unwrap();

        let restored_identity = load_or_create_plaintext_file(&paths).unwrap();
        assert_eq!(*restored_identity.address_hash(), identity_hash);
        let restored_metadata = load_public_identity_metadata(&metadata_path).unwrap();
        let second_mock = Arc::new(MockTransport::new_default());
        let second = compose_with_mock_identity_state(
            second_mock.clone(),
            paths,
            restored_identity,
            restored_metadata,
        )
        .await;
        let projected = second.facade.query_identity().await.unwrap();
        assert_eq!(projected.display_name, "Field Node");
        assert_eq!(projected.icon.as_deref(), Some("radio"));
        assert_eq!(projected.short_name.as_deref(), Some("FN"));
        assert!(second_mock.calls().is_empty());

        second.app_context.identity().announce(None).await;
        assert!(matches!(
            second_mock.calls().as_slice(),
            [MockCall::Announce { app_data }] if app_data == &expected
        ));
        let persisted_before_invalid = std::fs::read(&metadata_path).unwrap();
        let calls_before_invalid = second_mock.call_count();
        assert!(
            second.facade.set_identity(Some("bad\nname"), Some("changed"), None).await.is_err()
        );
        assert_eq!(second_mock.call_count(), calls_before_invalid);
        assert_eq!(std::fs::read(metadata_path).unwrap(), persisted_before_invalid);
        second.shutdown().await.unwrap();
    }

    fn poll_wire(content: &str) -> Vec<u8> {
        let sender = PrivateIdentity::new_from_name("legacy mobile poll sender");
        let sender_destination = SingleOutputDestination::new(
            *sender.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let mut source = [0; 16];
        source.copy_from_slice(sender_destination.as_slice());
        crate::lxmf_bridge::build_wire_message(source, [0; 16], "", content, None, &sender).unwrap()
    }

    #[test]
    fn poll_preview_is_unicode_safe_and_bounded() {
        let ascii_boundary = "a".repeat(POLL_PREVIEW_MAX_CHARS);
        assert_eq!(legacy_poll_preview(&ascii_boundary), ascii_boundary);
        assert_eq!(
            legacy_poll_preview(&"a".repeat(POLL_PREVIEW_MAX_CHARS + 1)),
            "a".repeat(POLL_PREVIEW_MAX_CHARS)
        );
        assert_eq!(legacy_poll_preview(""), "");

        let multibyte = "界".repeat(40);
        let preview = legacy_poll_preview(&multibyte);
        assert!(preview.is_char_boundary(preview.len()));
        assert!(preview.len() <= POLL_PREVIEW_MAX_BYTES);
        assert!(preview.chars().count() <= POLL_PREVIEW_MAX_CHARS);

        let combining = "e\u{301}".repeat(80);
        let preview = legacy_poll_preview(&combining);
        assert!(preview.is_char_boundary(preview.len()));
        assert!(preview.len() <= POLL_PREVIEW_MAX_BYTES);
        assert!(preview.chars().count() <= POLL_PREVIEW_MAX_CHARS);

        let over_limit = format!("{}界", "a".repeat(POLL_PREVIEW_MAX_BYTES - 1));
        assert_eq!(legacy_poll_preview(&over_limit), "a".repeat(POLL_PREVIEW_MAX_BYTES - 1));
    }

    #[tokio::test]
    async fn poll_hub_reports_accepted_duplicate_rejected_and_mixed_acknowledgement() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None).await;
        let wire = poll_wire("notification text");
        let pending = node.process_legacy_hub_batch(vec![
            ("accepted".into(), wire.clone()),
            ("duplicate".into(), wire),
            ("rejected".into(), b"not lxmf".to_vec()),
        ]);

        assert_eq!(
            pending.eligible.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>(),
            ["accepted", "duplicate",]
        );
        let result = pending.complete(Ok(()));
        assert_eq!(result.message_count, 1);
        assert_eq!(result.messages.len(), 1);
        assert!(matches!(result.items[0].local, PollLocalOutcome::Accepted { .. }));
        assert!(matches!(result.items[1].local, PollLocalOutcome::DurableDuplicate { .. }));
        assert!(matches!(result.items[2].local, PollLocalOutcome::DecodeRejected { .. }));
        assert_eq!(
            result.items.iter().map(|item| &item.acknowledgement).collect::<Vec<_>>(),
            [
                &PollAcknowledgementOutcome::Acknowledged,
                &PollAcknowledgementOutcome::Acknowledged,
                &PollAcknowledgementOutcome::NotEligible,
            ]
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn poll_hub_storage_failure_is_not_acknowledgeable() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None).await;
        let store = node.app_context.store().clone();
        let _ = std::thread::spawn(move || {
            let _guard = store.lock().unwrap();
            panic!("poison poll store");
        })
        .join();

        let pending =
            node.process_legacy_hub_batch(vec![("storage-failure".into(), poll_wire("lost"))]);
        assert!(pending.eligible.is_empty());
        let result = pending.complete(Ok(()));
        assert_eq!(result.message_count, 0);
        assert!(matches!(result.items[0].local, PollLocalOutcome::StorageFailed { .. }));
        assert_eq!(result.items[0].acknowledgement, PollAcknowledgementOutcome::NotEligible);
    }

    #[tokio::test]
    async fn poll_hub_surfaces_remote_acknowledgement_failure_per_eligible_item() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None).await;
        let pending =
            node.process_legacy_hub_batch(vec![("accepted".into(), poll_wire("keep me"))]);
        let result = pending.complete(Err("hub refused deletion".into()));

        assert_eq!(result.message_count, 1);
        assert_eq!(
            result.items[0].acknowledgement,
            PollAcknowledgementOutcome::Failed { error: "hub refused deletion".into() }
        );
        node.shutdown().await.unwrap();
    }

    fn propagation_candidate(
        node: &MobileNode,
        name: &str,
        active: bool,
        observed_at: i64,
    ) -> (PrivateIdentity, [u8; 16], String) {
        let identity = PrivateIdentity::new_from_name(name);
        let destination = SingleOutputDestination::new(
            *identity.as_identity(),
            DestinationName::new("lxmf", "propagation"),
        )
        .desc
        .address_hash;
        let mut destination_bytes = [0; 16];
        destination_bytes.copy_from_slice(destination.as_slice());
        let mut identity_bytes = [0; 16];
        identity_bytes.copy_from_slice(identity.address_hash().as_slice());
        let mut metadata = if active {
            lxmf::propagation_announce::StandardPropagationAnnounce::active(
                observed_at,
                Some(name),
                256,
                4_000,
            )
            .unwrap()
        } else {
            lxmf::propagation_announce::StandardPropagationAnnounce::inactive(
                observed_at,
                Some(name),
            )
            .unwrap()
        };
        metadata.stamp_cost = 0;
        metadata.stamp_cost_flexibility = 0;
        metadata.peering_cost = 0;
        node.app_context
            .discovery()
            .accept_standard_propagation_announce(
                hex::encode(destination_bytes),
                identity_bytes,
                destination_bytes,
                observed_at,
                &metadata,
            )
            .unwrap();
        (identity, destination_bytes, hex::encode(destination_bytes))
    }

    fn propagation_response(value: rmpv::Value) -> styrene_ipc::types::RequestObservationInfo {
        let mut response = styrene_ipc::types::RequestObservationInfo::default();
        response.request_id = "55".repeat(16);
        response.state = styrene_ipc::types::RequestState::Succeeded;
        response.response = Some(rmp_serde::to_vec(&value).unwrap());
        response
    }

    #[tokio::test]
    async fn selected_propagation_destination_and_policy_survive_mobile_restart() {
        let root = tempfile::tempdir().unwrap();
        let mobile_config = MobileConfig {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: Some("mobile propagation client".into()),
            identity_backend: IdentityBackend::PlaintextFile,
            interfaces: Vec::new(),
            enable_rnode_channel: false,
        };
        let first = MobileNode::boot(mobile_config.clone()).await.unwrap();
        let now = rns_core::transport::time::now_epoch_secs_i64();
        let (_, _, destination) =
            propagation_candidate(&first, "selected propagation node", true, now);

        let selected = first.select_propagation_destination(&destination).await.unwrap();
        assert!(selected.ready);
        assert_eq!(selected.readiness, MobilePropagationReadiness::Ready);
        assert_eq!(selected.selected_destination.as_deref(), Some(destination.as_str()));
        assert_eq!(selected.selected_policy.as_ref().unwrap().sync_limit_kb, 4_000);
        first.shutdown().await.unwrap();

        let second = MobileNode::boot(mobile_config).await.unwrap();
        let restored = second.propagation_snapshot().await.unwrap();
        assert!(restored.ready);
        assert_eq!(restored.selected_destination, Some(destination));
        second.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn inactive_or_missing_propagation_metadata_never_reports_ready() {
        let node = compose_with_mock(Arc::new(MockTransport::new_default()), None).await;
        let now = rns_core::transport::time::now_epoch_secs_i64();
        let (_, _, inactive) =
            propagation_candidate(&node, "inactive propagation node", false, now);

        let failure = node.select_propagation_destination(&inactive).await.unwrap_err();
        assert_eq!(failure.code, MobilePropagationFailureCode::Inactive);
        assert!(!node.propagation_snapshot().await.unwrap().ready);

        let missing = "ab".repeat(16);
        let failure = node.select_propagation_destination(&missing).await.unwrap_err();
        assert_eq!(failure.code, MobilePropagationFailureCode::NotAnnounced);
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn manual_propagation_sync_identifies_then_requests_inventory() {
        let mock = Arc::new(MockTransport::new_default());
        let node = compose_with_mock(mock.clone(), None).await;
        let now = rns_core::transport::time::now_epoch_secs_i64();
        let (identity, _, destination) =
            propagation_candidate(&node, "inventory propagation node", true, now);
        node.select_propagation_destination(&destination).await.unwrap();
        mock.queue_resolve(Some(*identity.as_identity()));
        mock.queue_open_link(Ok(AddressHash::new([0x42; 16])));
        mock.queue_request(Ok(propagation_response(rmpv::Value::Array(Vec::new()))));

        let outcome = node.sync_propagation_once(Duration::from_secs(1)).await.unwrap();
        assert_eq!(outcome.new_messages, 0);
        let calls = mock.calls();
        let identified =
            calls.iter().position(|call| matches!(call, MockCall::IdentifyLink { .. }));
        let inventory = calls.iter().position(|call| matches!(call, MockCall::StartRequest { .. }));
        assert!(identified.is_some_and(|index| inventory.is_some_and(|request| index < request)));
        let snapshot = node.propagation_snapshot().await.unwrap();
        assert_eq!(snapshot.sync_state, MobilePropagationSyncState::Complete);
        assert_eq!(snapshot.new_messages, 0);
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn failed_manual_propagation_sync_is_retryable_and_durable() {
        let mock = Arc::new(MockTransport::new_default());
        let node = compose_with_mock(mock.clone(), None).await;
        let now = rns_core::transport::time::now_epoch_secs_i64();
        let (identity, _, destination) =
            propagation_candidate(&node, "failing propagation node", true, now);
        node.select_propagation_destination(&destination).await.unwrap();
        mock.queue_resolve(Some(*identity.as_identity()));
        mock.queue_open_link(Ok(AddressHash::new([0x43; 16])));
        mock.queue_request(Ok(propagation_response(rmpv::Value::Boolean(true))));

        let failure = node.sync_propagation_once(Duration::from_secs(1)).await.unwrap_err();
        assert!(failure.retryable);
        let snapshot = node.propagation_snapshot().await.unwrap();
        assert_eq!(snapshot.sync_state, MobilePropagationSyncState::Failed);
        assert!(snapshot.failure.as_ref().is_some_and(|failure| failure.retryable));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn explicit_mobile_propagated_send_records_upload_without_claiming_delivery() {
        let mock = Arc::new(MockTransport::new_default());
        let node = compose_with_mock(mock.clone(), None).await;
        let now = rns_core::transport::time::now_epoch_secs_i64();
        let (propagation_identity, _, propagation_destination) =
            propagation_candidate(&node, "upload propagation node", true, now);
        node.select_propagation_destination(&propagation_destination).await.unwrap();
        let recipient = PrivateIdentity::new_from_name("mobile propagated recipient");
        let recipient_destination = SingleOutputDestination::new(
            *recipient.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        mock.queue_resolve(Some(*recipient.as_identity()));
        mock.queue_resolve(Some(*propagation_identity.as_identity()));
        mock.queue_open_link(Ok(AddressHash::new([0x44; 16])));
        mock.queue_request(Ok(propagation_response(rmpv::Value::Boolean(false))));
        mock.queue_close(Ok(()));

        let outcome = node
            .send_text(MobileSendRequest {
                destination_hash: hex::encode(recipient_destination.as_slice()),
                content: "store this for offline delivery".into(),
                requested_method: MobileDeliveryMethod::Propagated,
                draft_revision: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.disposition, MobileSendDisposition::Accepted);
        assert_eq!(outcome.actual_method, MobileDeliveryMethod::Propagated);
        assert!(outcome.message.delivery_evidence.is_empty());
        assert_eq!(outcome.message.propagation_correlations.len(), 1);
        let correlation = &outcome.message.propagation_correlations[0];
        assert_eq!(correlation.state, "accepted");
        assert_eq!(
            correlation.peer_hash.as_deref(),
            Some(hex::encode(propagation_identity.address_hash().as_slice()).as_str())
        );
        assert!(!correlation.transient_id.is_empty());
        assert!(correlation.attempt_id.is_some());
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn propagated_retrieval_is_durable_before_ack_and_duplicate_safe() {
        let destination = [0x51; 16];
        let mock = Arc::new(MockTransport::new(
            AddressHash::new([0x50; 16]),
            AddressHash::new(destination),
        ));
        let node = compose_with_mock(mock, None).await;
        let sender = PrivateIdentity::new_from_name("propagated mobile sender");
        let sender_destination = SingleOutputDestination::new(
            *sender.as_identity(),
            DestinationName::new("lxmf", "delivery"),
        )
        .desc
        .address_hash;
        let mut source = [0; 16];
        source.copy_from_slice(sender_destination.as_slice());
        let wire = crate::lxmf_bridge::build_wire_message(
            source,
            destination,
            "",
            "retrieved once",
            None,
            &sender,
        )
        .unwrap();
        let transient_id = lxmf::propagation::transient_id(&wire);
        let attempt_id = [0x52; 16];
        let peer = [0x53; 16];

        let first = node.app_context.messaging().accept_propagated_inbound(
            destination,
            &wire,
            transient_id,
            attempt_id,
            peer,
        );
        let second = node.app_context.messaging().accept_propagated_inbound(
            destination,
            &wire,
            transient_id,
            attempt_id,
            peer,
        );

        assert!(matches!(first, InboundAcceptOutcome::Accepted(_)));
        assert!(matches!(second, InboundAcceptOutcome::Duplicate { .. }));
        {
            let store = node.app_context.store().lock().unwrap();
            assert_eq!(store.list_messages(10, None).unwrap().len(), 1);
            assert_eq!(
                store.standard_propagation_pending_haves(peer, 10).unwrap(),
                vec![transient_id]
            );
        }
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rejected_propagated_retrieval_is_not_acknowledgeable() {
        let destination = [0x61; 16];
        let node = compose_with_mock(
            Arc::new(MockTransport::new(
                AddressHash::new([0x60; 16]),
                AddressHash::new(destination),
            )),
            None,
        )
        .await;
        let transient_id = [0x62; 32];
        let peer = [0x63; 16];

        let outcome = node.app_context.messaging().accept_propagated_inbound(
            destination,
            b"invalid LXMF wire",
            transient_id,
            [0x64; 16],
            peer,
        );

        assert!(matches!(outcome, InboundAcceptOutcome::Rejected { .. }));
        {
            let store = node.app_context.store().lock().unwrap();
            assert!(store.list_messages(10, None).unwrap().is_empty());
            assert!(store.standard_propagation_pending_haves(peer, 10).unwrap().is_empty());
        }
        node.shutdown().await.unwrap();
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
            MobileIdentityRuntime {
                metadata: crate::services::identity::PublicIdentityMetadata::default(),
                metadata_path: PathBuf::from("test-config/identity-public.json"),
                custody: active_custody(IdentityBackend::PlaintextFile),
                backup_custody: None,
                portable_backup_custody: None,
            },
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
    async fn failed_transport_shutdown_is_typed_and_retryable_without_workers() {
        let mock = Arc::new(MockTransport::new_default());
        mock.queue_shutdown(Err(crate::transport::mesh_transport::TransportError::ShutdownFailed(
            "injected transport failure".into(),
        )));
        let node = compose_with_mock(mock.clone(), None).await;

        let error = node.shutdown().await.unwrap_err();

        assert!(matches!(
            error,
            crate::transport::mesh_transport::TransportError::ShutdownFailed(ref message)
                if message == "injected transport failure"
        ));
        let failed = node.session_snapshot().await;
        assert_eq!(failed.runtime, MobileRuntimeState::Failed);
        assert_eq!(failed.phase, MobileConnectionPhase::Failed);
        assert_eq!(
            failed.failure,
            Some(MobileFailure { code: MobileFailureCode::CleanupFailed, retryable: true })
        );
        assert_eq!(
            node.storage_status().unwrap().last_commit.unwrap().kind,
            crate::storage::messages::StorageCommitKind::SessionOpened
        );

        node.shutdown().await.unwrap();
        let stopped = node.session_snapshot().await;
        assert_eq!(stopped.runtime, MobileRuntimeState::Stopped);
        assert_eq!(stopped.phase, MobileConnectionPhase::Stopped);
        assert_eq!(stopped.failure, None);
        assert_eq!(
            node.storage_status().unwrap().last_commit.unwrap().kind,
            crate::storage::messages::StorageCommitKind::CleanShutdown
        );
        assert_eq!(
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count(),
            2
        );
    }

    #[tokio::test]
    async fn failed_storage_marker_is_retryable_without_repeating_transport_shutdown() {
        let mock = Arc::new(MockTransport::new_default());
        let node = compose_with_mock(mock.clone(), None).await;
        node.storage_shutdown_faults.store(1, Ordering::Release);

        let error = node.shutdown().await.unwrap_err();

        assert!(matches!(
            error,
            crate::transport::mesh_transport::TransportError::ShutdownFailed(ref message)
                if message == "injected mobile storage clean-shutdown marker failure"
        ));
        let failed = node.session_snapshot().await;
        assert_eq!(failed.runtime, MobileRuntimeState::Failed);
        assert_eq!(failed.phase, MobileConnectionPhase::Failed);
        assert_eq!(
            node.storage_status().unwrap().last_commit.unwrap().kind,
            crate::storage::messages::StorageCommitKind::SessionOpened
        );

        node.shutdown().await.unwrap();

        assert_eq!(node.session_snapshot().await.runtime, MobileRuntimeState::Stopped);
        assert_eq!(
            node.storage_status().unwrap().last_commit.unwrap().kind,
            crate::storage::messages::StorageCommitKind::CleanShutdown
        );
        assert_eq!(
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count(),
            1
        );
    }

    #[tokio::test]
    async fn post_composition_boot_failure_releases_workers_listener_and_store_before_retry() {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mobile_config = MobileConfig {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("data"),
            hub_address: None,
            hub_delivery_hash: None,
            display_name: None,
            identity_backend: IdentityBackend::PlaintextFile,
            interfaces: vec![MobileInterfaceConfig::TcpServer {
                bind_address: address.to_string(),
            }],
            enable_rnode_channel: false,
        };
        let evidence = Arc::new(MobileBootFaultEvidence::default());
        inject_mobile_boot_fault(mobile_config.data_dir.clone(), Arc::clone(&evidence));

        let error = match MobileNode::boot(mobile_config.clone()).await {
            Ok(node) => {
                node.shutdown().await.unwrap();
                panic!("injected post-composition boot unexpectedly succeeded");
            }
            Err(error) => error,
        };

        assert_eq!(error.stage, MobileBootStage::Composition);
        assert_eq!(error.code, MobileBootFailureCode::CompositionFailed);
        assert!(error.retryable);
        assert_eq!(error.message, "runtime composition failed");
        assert!(!error.to_string().contains(root.path().to_string_lossy().as_ref()));
        {
            let handles = evidence.worker_handles.lock().unwrap();
            assert_eq!(handles.len(), 10);
            assert!(handles.iter().all(tokio::task::AbortHandle::is_finished));
        }

        let rebound =
            std::net::TcpListener::bind(address).expect("failed boot released TCP listener");
        drop(rebound);
        let node =
            MobileNode::boot(mobile_config).await.expect("boot retry after composed failure");
        assert_eq!(node.tcp_listen_addresses(), [address]);
        assert_eq!(
            node.storage_status().unwrap().recovery,
            crate::storage::messages::StorageRecoveryOutcome::CleanShutdown
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn storage_shutdown_failure_is_typed_and_does_not_mark_runtime_stopped() {
        let mock = Arc::new(MockTransport::new_default());
        let node = compose_with_mock(mock.clone(), None).await;
        let store = node.app_context.store().clone();
        let _ = std::thread::spawn(move || {
            let _guard = store.lock().unwrap();
            panic!("poison mobile storage lock");
        })
        .join();

        let error = node.shutdown().await.unwrap_err();

        assert!(matches!(
            error,
            crate::transport::mesh_transport::TransportError::ShutdownFailed(ref message)
                if message == "mobile storage state unavailable"
        ));
        let failed = node.session_snapshot().await;
        assert_eq!(failed.runtime, MobileRuntimeState::Failed);
        assert_eq!(failed.phase, MobileConnectionPhase::Failed);
        assert_eq!(
            mock.calls().into_iter().filter(|call| matches!(call, MockCall::Shutdown)).count(),
            1
        );
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
        let propagation = node.propagation_snapshot().await.unwrap();
        assert!(propagation.automatic_sync_enabled);
        assert_eq!(propagation.automatic_sync_cooldown_secs, 30);
        assert_eq!(propagation.sync_deadline_secs, 32);
        assert!(node.propagation_foreground_opportunity());

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
    async fn configured_rnode_bytes_attribute_bluetooth_without_usb_fallback() {
        use rns_core::transport::iface::kiss::kiss_encode_command;
        use rns_core::transport::iface::rnode::{
            CMD_BANDWIDTH, CMD_CODING_RATE, CMD_DETECT, CMD_FIRMWARE_VERSION, CMD_FREQUENCY,
            CMD_MCU, CMD_PLATFORM, CMD_RADIO_STATE, CMD_SPREADING_FACTOR, CMD_TX_POWER,
            RNodeFirmwareVersion, RNodeRadioProfile,
        };

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

        assert_eq!(
            node.start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::BluetoothLe, 512),
            )
            .await
            .unwrap_err(),
            "Bluetooth RNode requires an approved peripheral"
        );
        node.platform_service().set_bluetooth_approved(true).await;
        let start = node
            .start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::BluetoothLe, 512),
            )
            .await
            .unwrap();
        assert_eq!(start.writes.len(), 8);
        let connecting = node.session_snapshot().await;
        assert_eq!(
            connecting.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Connecting
        );
        assert_eq!(
            connecting.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Unavailable
        );

        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let metadata = [
            kiss_encode_command(CMD_FIRMWARE_VERSION, &[1, 86]),
            kiss_encode_command(CMD_PLATFORM, &[0x70]),
            kiss_encode_command(CMD_MCU, &[0x71]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &metadata).await.unwrap();
        let observed = node.rnode_metadata(start.attempt).await.unwrap().unwrap();
        assert_eq!(observed.firmware_version, Some(RNodeFirmwareVersion { major: 1, minor: 86 }));
        assert_eq!(observed.platform, Some(0x70));
        assert_eq!(observed.mcu, Some(0x71));
        let detect = kiss_encode_command(CMD_DETECT, &[0x46]);
        assert_eq!(node.submit_rnode_bytes(start.attempt, &detect).await.unwrap().len(), 6);
        let readback = [
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[1]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &readback[..readback.len() / 2]).await.unwrap();
        node.submit_rnode_bytes(start.attempt, &readback[readback.len() / 2..]).await.unwrap();

        let connected = node.session_snapshot().await;
        assert_eq!(
            connected.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Connected
        );
        assert_eq!(
            connected.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Unavailable
        );
        assert_eq!(
            node.platform_service().request_android_usb_fallback().await,
            MobileUsbFallbackDisposition::BluetoothActive
        );

        let shutdown = node
            .stop_rnode_bytes(start.attempt, MobileBearerReason::ConnectionInterrupted)
            .await
            .unwrap();
        assert!(!shutdown.is_empty());
        assert_eq!(node.rnode_metadata(start.attempt).await.unwrap(), None);
        let stopped = node.session_snapshot().await;
        assert_eq!(
            stopped.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Disconnected
        );
        assert_eq!(
            stopped.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Unavailable
        );
    }

    #[tokio::test]
    async fn configured_rnode_bytes_activate_usb_and_frame_outbound_packets() {
        use rns_core::transport::iface::kiss::{KissDecoder, kiss_encode_command};
        use rns_core::transport::iface::rnode::{
            CMD_BANDWIDTH, CMD_CODING_RATE, CMD_DETECT, CMD_FREQUENCY, CMD_RADIO_STATE,
            CMD_SPREADING_FACTOR, CMD_TX_POWER, RNodeRadioProfile,
        };

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

        assert_eq!(
            node.start_rnode_bytes(
                MobileRNodeBearer::AndroidUsb,
                test_rnode_info(MobileRNodeBearer::AndroidUsb, 512),
            )
            .await
            .unwrap_err(),
            "Android USB requires an explicit fallback request"
        );
        assert_eq!(
            node.platform_service().request_android_usb_fallback().await,
            MobileUsbFallbackDisposition::Accepted
        );
        let start = node
            .start_rnode_bytes(
                MobileRNodeBearer::AndroidUsb,
                test_rnode_info(MobileRNodeBearer::AndroidUsb, 512),
            )
            .await
            .unwrap();
        assert_eq!(start.writes.len(), 8);

        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let detect = kiss_encode_command(CMD_DETECT, &[0x46]);
        let config_writes = node.submit_rnode_bytes(start.attempt, &detect).await.unwrap();
        assert_eq!(config_writes.len(), 6);
        let readback = [
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[1]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &readback).await.unwrap();

        let snapshot = node.session_snapshot().await;
        assert_eq!(snapshot.phase, MobileConnectionPhase::Connected);
        assert_eq!(
            snapshot.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Connected
        );
        node.announce().await.unwrap();
        let framed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(writes) = node.poll_rnode_bytes(start.attempt).await.unwrap() {
                    break writes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut decoder = KissDecoder::new();
        decoder.feed(&framed.writes.concat());
        assert!(!decoder.take_frame().expect("outbound RNS packet").is_empty());

        let shutdown = node
            .stop_rnode_bytes(start.attempt, MobileBearerReason::ConnectionInterrupted)
            .await
            .unwrap();
        assert!(!shutdown.is_empty());
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Disconnected
        );

        let mut state_events = node.subscribe_state_events();
        node.platform_service()
            .report(MobileBearerObservation {
                kind: MobileBearerKind::AndroidUsb,
                state: MobileBearerState::Disconnected,
                reason: Some(MobileBearerReason::PermissionDenied),
            })
            .await
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(1), state_events.recv())
            .await
            .expect("platform report invalidation timed out")
            .expect("platform report invalidation failed");
        assert_eq!(event.kind, MobileStateEventKind::Session);
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::AndroidUsb).unwrap().reason,
            Some(MobileBearerReason::PermissionDenied)
        );
    }

    #[tokio::test]
    async fn rnode_attempt_generation_rejects_conflicts_and_stale_callbacks() {
        use rns_core::transport::iface::kiss::kiss_encode_command;
        use rns_core::transport::iface::rnode::CMD_DETECT;

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
        node.platform_service().set_bluetooth_approved(true).await;

        let bluetooth = node
            .start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::BluetoothLe, 512),
            )
            .await
            .unwrap();
        assert_eq!(
            node.start_rnode_bytes(
                MobileRNodeBearer::AndroidUsb,
                test_rnode_info(MobileRNodeBearer::AndroidUsb, 512),
            )
            .await
            .unwrap_err(),
            "RNode byte attempt is already active"
        );
        assert!(
            !node
                .stop_rnode_bytes(bluetooth.attempt, MobileBearerReason::ConnectionInterrupted,)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            node.platform_service().request_android_usb_fallback().await,
            MobileUsbFallbackDisposition::Accepted
        );
        let usb = node
            .start_rnode_bytes(
                MobileRNodeBearer::AndroidUsb,
                test_rnode_info(MobileRNodeBearer::AndroidUsb, 512),
            )
            .await
            .unwrap();
        assert_ne!(bluetooth.attempt, usb.attempt);

        let detect = kiss_encode_command(CMD_DETECT, &[0x46]);
        assert!(node.submit_rnode_bytes(bluetooth.attempt, &detect).await.unwrap().is_empty());
        assert!(node.poll_rnode_bytes(bluetooth.attempt).await.unwrap().is_none());
        assert!(
            node.stop_rnode_bytes(bluetooth.attempt, MobileBearerReason::ConnectionInterrupted)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Connecting
        );

        assert!(
            !node
                .stop_rnode_bytes(usb.attempt, MobileBearerReason::ConnectionInterrupted)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            node.stop_rnode_bytes(usb.attempt, MobileBearerReason::ConnectionInterrupted)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::AndroidUsb).unwrap().state,
            MobileBearerState::Disconnected
        );
    }

    #[tokio::test]
    async fn mismatched_rnode_readback_keeps_backend_bearer_connecting() {
        use rns_core::transport::iface::kiss::kiss_encode_command;
        use rns_core::transport::iface::rnode::{
            CMD_BANDWIDTH, CMD_CODING_RATE, CMD_DETECT, CMD_FREQUENCY, CMD_RADIO_STATE,
            CMD_SPREADING_FACTOR, CMD_TX_POWER, RNodeRadioProfile,
        };

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
        node.platform_service().set_bluetooth_approved(true).await;
        let start = node
            .start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::BluetoothLe, 512),
            )
            .await
            .unwrap();
        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let mismatched = [
            kiss_encode_command(CMD_DETECT, &[0x46]),
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm - 1]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[1]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &mismatched).await.unwrap();
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Connecting
        );
        assert!(node.poll_rnode_bytes(start.attempt).await.unwrap().is_none());

        let corrected = kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]);
        node.submit_rnode_bytes(start.attempt, &corrected).await.unwrap();
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Connected
        );
    }

    #[tokio::test]
    async fn rnode_attempt_metadata_bounds_every_host_write_path() {
        use rns_core::transport::iface::kiss::kiss_encode_command;
        use rns_core::transport::iface::rnode::{
            CMD_BANDWIDTH, CMD_CODING_RATE, CMD_DETECT, CMD_FREQUENCY, CMD_RADIO_STATE,
            CMD_SPREADING_FACTOR, CMD_TX_POWER, RNodeRadioProfile,
        };

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
        assert_eq!(
            node.start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::AndroidUsb, 3),
            )
            .await
            .unwrap_err(),
            "RNode bearer metadata does not match the mobile bearer"
        );
        assert_eq!(
            node.session_snapshot().await.bearer(MobileBearerKind::BluetoothRnode).unwrap().state,
            MobileBearerState::Unverified
        );

        node.platform_service().set_bluetooth_approved(true).await;
        let start = node
            .start_rnode_bytes(
                MobileRNodeBearer::BluetoothLe,
                test_rnode_info(MobileRNodeBearer::BluetoothLe, 3),
            )
            .await
            .unwrap();
        assert!(start.writes.iter().all(|write| write.len() <= 3));

        let profile = RNodeRadioProfile::US_915_DEVELOPMENT;
        let detect = kiss_encode_command(CMD_DETECT, &[0x46]);
        let configuration = node.submit_rnode_bytes(start.attempt, &detect).await.unwrap();
        assert!(configuration.iter().all(|write| write.len() <= 3));
        let readback = [
            kiss_encode_command(CMD_FREQUENCY, &profile.frequency_hz.to_be_bytes()),
            kiss_encode_command(CMD_BANDWIDTH, &profile.bandwidth_hz.to_be_bytes()),
            kiss_encode_command(CMD_TX_POWER, &[profile.tx_power_dbm]),
            kiss_encode_command(CMD_SPREADING_FACTOR, &[profile.spreading_factor]),
            kiss_encode_command(CMD_CODING_RATE, &[profile.coding_rate]),
            kiss_encode_command(CMD_RADIO_STATE, &[1]),
        ]
        .concat();
        node.submit_rnode_bytes(start.attempt, &readback).await.unwrap();

        node.announce().await.unwrap();
        let packet_writes = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(writes) = node.poll_rnode_bytes(start.attempt).await.unwrap() {
                    break writes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(packet_writes.writes.iter().all(|write| write.len() <= 3));

        let shutdown = node
            .stop_rnode_bytes(start.attempt, MobileBearerReason::ConnectionInterrupted)
            .await
            .unwrap();
        assert!(!shutdown.is_empty());
        assert!(shutdown.iter().all(|write| write.len() <= 3));
    }

    #[tokio::test]
    async fn outbound_rnode_handoff_replays_after_failure_and_cancellation_until_completed() {
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

        let first = ready_test_rnode(&node, MobileRNodeBearer::BluetoothLe).await;
        node.announce().await.unwrap();
        let failed = poll_test_handoff(&node, first.attempt).await;
        assert!(node.poll_rnode_bytes(first.attempt).await.unwrap().is_none());
        assert!(node.fail_rnode_write(first.attempt, failed.handoff).await.unwrap());
        node.stop_rnode_bytes(first.attempt, MobileBearerReason::ConnectionInterrupted)
            .await
            .unwrap();

        let second = ready_test_rnode(&node, MobileRNodeBearer::BluetoothLe).await;
        let replayed_after_failure = poll_test_handoff(&node, second.attempt).await;
        assert_eq!(replayed_after_failure.handoff, failed.handoff);
        assert_eq!(replayed_after_failure.writes.concat(), failed.writes.concat());
        assert!(!node.complete_rnode_write(first.attempt, failed.handoff).await.unwrap());
        assert!(node.poll_rnode_bytes(second.attempt).await.unwrap().is_none());
        node.stop_rnode_bytes(second.attempt, MobileBearerReason::ConnectionInterrupted)
            .await
            .unwrap();

        let third = ready_test_rnode(&node, MobileRNodeBearer::BluetoothLe).await;
        let replayed_after_cancellation = poll_test_handoff(&node, third.attempt).await;
        assert_eq!(replayed_after_cancellation.handoff, failed.handoff);
        assert!(
            node.complete_rnode_write(third.attempt, replayed_after_cancellation.handoff)
                .await
                .unwrap()
        );
        assert!(
            !node
                .complete_rnode_write(third.attempt, replayed_after_cancellation.handoff)
                .await
                .unwrap()
        );
        assert!(node.poll_rnode_bytes(third.attempt).await.unwrap().is_none());
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
