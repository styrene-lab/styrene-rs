//! IPC adapter for live and explicitly selected embedded daemon sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, info_span, warn, Instrument};

use rmpv::Value as MpValue;
use styrene_ipc::types::{
    ActiveCapabilitiesInfo, DaemonStatusInfo, DegradedCapabilityInfo, DeviceInfo,
    DiscoveredCapability, ExecResult, IdentityInfo, LinkEvent, LinkSnapshot, MessageInfo,
    NetworkOperationInfo, NetworkOperationKind, NetworkOperationOutcome, NetworkOperationProgress,
    ObservationMetadata, ObservationSource, PropagationQueueEntry, PropagationSnapshot,
    RebootResult, RemoteStatusInfo, RequestObservationInfo, ResourceTransferInfo, RouteEventInfo,
    RouteEventKind, RouteLossReason, StandardPropagationSnapshot, StartNetworkOperationInfo,
    StartRequestInfo,
};
use styrene_ipc::IpcError;
use styrene_ipc_server::wire::{self, Frame, MessageType, REQUEST_ID_SIZE};

/// A single entry from the path table — routing info for one destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTableEntry {
    pub destination_hash: String,
    pub hops: u8,
    pub next_hop: String,
    pub interface: String,
    pub expires: Option<i64>,
    pub observation: ObservationMetadata,
}

/// Exact typed interface projection from the Unix IPC contract.
pub type InterfaceStats = styrene_ipc::types::InterfaceDetail;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageResponse {
    pub page: styrene_ipc::types::PageContent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrokerDiagnostics {
    pub queue_depth: usize,
    pub in_flight: usize,
    pub completed: u64,
    pub timed_out: u64,
    pub cancelled: u64,
    pub overloaded: u64,
    pub disconnected: u64,
    pub reconnects: u64,
    pub stale_responses: u64,
    pub dropped_responses: u64,
    pub dropped_updates: u64,
    pub last_latency_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum BridgeError {
    Ipc(IpcError),
    Internal(String),
}

impl BridgeError {
    fn cursor_stale(&self) -> bool {
        matches!(self, Self::Ipc(IpcError::Conflict { message }) if message == "cursor_stale")
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ipc(error) => error.fmt(formatter),
            Self::Internal(message) => formatter.write_str(message),
        }
    }
}

/// Events pushed from the daemon to the UI.
#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Connected,
    Identity(IdentityInfo),
    Status(DaemonStatusInfo),
    EventGeneration(u64),
    PeerDiscovered(DeviceInfo),
    LocalPageInventory(Vec<styrene_ipc::types::PageInfo>),
    MessageReceived(Box<MessageInfo>),
    MessagingOperation(Box<styrene_ipc::types::MessagingOperationOutcome>),
    LinkObservation(LinkEvent),
    PathTable(Vec<PathTableEntry>),
    RouteLifecycle(RouteEventInfo),
    NetworkOperation(NetworkOperationInfo),
    Request(RequestObservationInfo),
    Resource(ResourceTransferInfo),
    ReconcileRequests { dropped: u64, connection_generation: u64 },
    ReconcileRequired { dropped: u64, connection_generation: u64 },
    StandardPropagationChanged { connection_generation: u64 },
    Disconnected(String),
}

/// Commands sent from the UI to the daemon.
#[derive(Clone)]
pub enum DaemonCommand {
    SendChat {
        peer_hash: String,
        content: String,
        title: Option<String>,
        delivery_method: String,
        attachments: Vec<styrene_ipc::types::AttachmentInput>,
    },
    LoadDraft {
        peer_hash: String,
    },
    SaveDraft {
        peer_hash: String,
        content: String,
    },
    DiscardDraft {
        peer_hash: String,
    },
    RetryMessage {
        message_id: String,
    },
    CancelMessage {
        message_id: String,
    },
    BrowsePage {
        address: crate::state::PageAddress,
    },
    NavigatePage(styrene_ipc::types::PageNavigationRequest),
    ClosePage {
        session_id: String,
    },
    StartFileDownload(styrene_ipc::types::FileDownloadRequest),
    QueryFileDownload {
        download_id: String,
    },
    CancelFileDownload {
        download_id: String,
    },
    SaveFileDownload {
        download_id: String,
        destination: String,
    },
    RefreshPathTable,
    RefreshInterfaces,
    RefreshLinks,
    RefreshOperations,
    RefreshRequests,
    RefreshResources,
    StartNetworkOperation(StartNetworkOperationInfo),
    CancelNetworkOperation {
        operation_id: String,
    },
    StartRequest(StartRequestInfo),
    CancelRequest {
        request_id: String,
    },
    CancelResource {
        resource_hash: String,
    },
    RefreshPropagation {
        cursor: Option<String>,
    },
    RefreshStandardPropagation,
    LoadConversations {
        cursor: Option<String>,
    },
    LoadMessages {
        peer_hash: String,
        cursor: Option<String>,
    },
    QueryMessage {
        message_id: String,
    },
    FleetStatus {
        destination: String,
    },
    FleetExec {
        destination: String,
        command: String,
        args: Vec<String>,
    },
    FleetReboot {
        destination: String,
        delay: Option<u64>,
    },
    FleetApply {
        destination: String,
        profile_base64: String,
    },
    BlockPeer {
        identity_hash: String,
    },
}

impl DaemonCommand {
    pub fn required_capability(&self) -> Option<&'static str> {
        match self {
            Self::SendChat { .. } => Some("chat.send"),
            Self::LoadDraft { .. } | Self::SaveDraft { .. } | Self::DiscardDraft { .. } => {
                Some("messaging.manage")
            }
            Self::RetryMessage { .. } | Self::CancelMessage { .. } => Some("messaging.lifecycle"),
            Self::BrowsePage { .. }
            | Self::NavigatePage(_)
            | Self::ClosePage { .. }
            | Self::StartFileDownload(_)
            | Self::QueryFileDownload { .. }
            | Self::CancelFileDownload { .. }
            | Self::SaveFileDownload { .. } => Some("page.browse"),
            Self::RefreshPathTable
            | Self::RefreshInterfaces
            | Self::RefreshLinks
            | Self::RefreshOperations
            | Self::RefreshRequests
            | Self::RefreshResources
            | Self::RefreshPropagation { .. }
            | Self::RefreshStandardPropagation
            | Self::LoadConversations { .. }
            | Self::LoadMessages { .. }
            | Self::FleetStatus { .. } => Some("rpc.status"),
            Self::QueryMessage { .. } => Some("messaging.history.read"),
            Self::StartNetworkOperation(request) => match request.kind {
                NetworkOperationKind::Announce => Some("network.announce"),
                NetworkOperationKind::PathRequest => Some("network.path_request"),
                NetworkOperationKind::Probe => Some("network.probe"),
                NetworkOperationKind::LinkOpen => Some("network.link_open"),
                NetworkOperationKind::LinkClose => Some("network.link_close"),
                _ => None,
            },
            Self::CancelNetworkOperation { .. } => None,
            Self::StartRequest(_) => Some("network.request"),
            Self::CancelRequest { .. } => Some("network.request_cancel"),
            Self::CancelResource { .. } => Some("network.resource_cancel"),
            Self::FleetExec { .. } => Some("rpc.exec"),
            Self::FleetReboot { .. } => Some("rpc.reboot"),
            Self::FleetApply { .. } => Some("rpc.fleet_apply"),
            Self::BlockPeer { .. } => Some("policy.update"),
        }
    }
}

/// Handle to the daemon connection. Owns the IPC stream or embedded daemon.
pub struct DaemonBridge {
    stream: UnixStream,
    next_id: u64,
    usable: bool,
}

// Fleet, tunnel, and terminal methods are staged for their OpenSpec pages.
#[allow(dead_code)]
impl DaemonBridge {
    fn next_request_id(&mut self) -> [u8; REQUEST_ID_SIZE] {
        self.next_id = self.next_id.wrapping_add(1);
        let mut id = [0u8; REQUEST_ID_SIZE];
        id[..8].copy_from_slice(&self.next_id.to_le_bytes());
        id
    }

    async fn rpc(
        &mut self,
        msg_type: MessageType,
        payload: &HashMap<String, MpValue>,
    ) -> Result<Frame, String> {
        let req_id = self.next_request_id();
        let start = std::time::Instant::now();
        if !self.usable {
            return Err("IPC connection is unavailable".into());
        }
        wire::write_frame_async(&mut self.stream, msg_type, &req_id, payload).await.map_err(
            |e| {
                error!(target: "dx::rpc", ?msg_type, %e, "write failed");
                format!("write: {e}")
            },
        )?;
        let result = timeout(Duration::from_secs(5), wire::read_frame_async(&mut self.stream))
            .await
            .map_err(|_| {
                self.usable = false;
                error!(target: "dx::rpc", ?msg_type, "rpc timeout (5s)");
                "rpc timeout".to_string()
            })?
            .map_err(|e| {
                self.usable = false;
                error!(target: "dx::rpc", ?msg_type, %e, "read failed");
                format!("read: {e}")
            })
            .and_then(|frame| validate_response(frame, req_id).map_err(|error| error.to_string()));
        let elapsed_ms = start.elapsed().as_millis();
        debug!(target: "dx::rpc", ?msg_type, elapsed_ms, "rpc complete");
        result
    }

    /// RPC with a custom timeout (for long-running operations like remote page fetch).
    async fn rpc_timeout(
        &mut self,
        msg_type: MessageType,
        payload: &HashMap<String, MpValue>,
        timeout_secs: u64,
    ) -> Result<Frame, String> {
        let req_id = self.next_request_id();
        let start = std::time::Instant::now();
        if !self.usable {
            return Err("IPC connection is unavailable".into());
        }
        wire::write_frame_async(&mut self.stream, msg_type, &req_id, payload)
            .await
            .map_err(|e| format!("write: {e}"))?;
        let result =
            timeout(Duration::from_secs(timeout_secs), wire::read_frame_async(&mut self.stream))
                .await
                .map_err(|_| {
                    self.usable = false;
                    format!("rpc timeout ({timeout_secs}s)")
                })?
                .map_err(|e| {
                    self.usable = false;
                    format!("read: {e}")
                })
                .and_then(|frame| {
                    validate_response(frame, req_id).map_err(|error| error.to_string())
                });
        let elapsed_ms = start.elapsed().as_millis();
        debug!(target: "dx::rpc", ?msg_type, elapsed_ms, "rpc complete");
        result
    }

    fn into_stream(self) -> UnixStream {
        self.stream
    }

    pub async fn identity(&mut self) -> Result<IdentityInfo, String> {
        let frame = self.rpc(MessageType::QueryIdentity, &HashMap::new()).await?;
        Ok(parse_identity(&frame.payload))
    }

    pub async fn status(&mut self) -> Result<DaemonStatusInfo, String> {
        let frame = self.rpc(MessageType::QueryStatus, &HashMap::new()).await?;
        Ok(parse_status(&frame.payload))
    }

    pub async fn devices(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let mut p = HashMap::new();
        p.insert("styrene_only".into(), MpValue::Boolean(false));
        let frame = self.rpc(MessageType::QueryDevices, &p).await?;
        parse_devices(&frame.payload)
    }

    /// Dump the entire path table — all known routes with hop count and relay info.
    pub async fn path_table(&mut self) -> Result<Vec<PathTableEntry>, String> {
        let frame = self.rpc(MessageType::QueryPathTable, &HashMap::new()).await?;
        Ok(parse_path_table(&frame.payload))
    }

    /// Query interface stats — per-interface name, status, TX/RX bytes, peers.
    pub async fn interface_stats(&mut self) -> Result<Vec<InterfaceStats>, String> {
        let frame = self.rpc(MessageType::QueryInterfaceStats, &HashMap::new()).await?;
        Ok(parse_interface_stats(&frame.payload))
    }

    pub async fn send_chat(&mut self, peer_hash: &str, content: &str) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        p.insert("content".into(), MpValue::from(content));
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    pub async fn browse_page(&mut self, host: &str, path: &str) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        p.insert("path".into(), MpValue::from(path));
        p.insert("timeout".into(), MpValue::from(25_u64));
        // Remote page fetches need 30s — mesh link establishment + transfer
        let timeout_secs = if host.is_empty() || host == "local" { 5 } else { 30 };
        let frame = self.rpc_timeout(MessageType::QueryPage, &p, timeout_secs).await?;
        let page = parse_page_content(&frame.payload)?;
        String::from_utf8(page.source_bytes)
            .map_err(|error| format!("typed page source is not UTF-8: {error}"))
    }

    // ── Fleet Operations ─────────────────────────────────────────────

    pub async fn device_status(&mut self, dest: &str) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest));
        let frame = self.rpc(MessageType::CmdDeviceStatus, &p).await?;
        Ok(frame.payload)
    }

    pub async fn exec(
        &mut self,
        dest: &str,
        command: &str,
        args: &[String],
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest));
        p.insert("command".into(), MpValue::from(command));
        let args_vals: Vec<MpValue> = args.iter().map(|a| MpValue::from(a.as_str())).collect();
        p.insert("args".into(), MpValue::Array(args_vals));
        let frame = self.rpc(MessageType::CmdExec, &p).await?;
        Ok(frame.payload)
    }

    pub async fn reboot_device(&mut self, dest: &str, delay: Option<u64>) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest));
        if let Some(d) = delay {
            p.insert("delay".into(), MpValue::from(d));
        }
        self.rpc(MessageType::CmdRebootDevice, &p).await.map(|_| ())
    }

    pub async fn fleet_apply(
        &mut self,
        dest: &str,
        profile_hex: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest));
        p.insert("profile".into(), MpValue::from(profile_hex));
        p.insert("verify".into(), MpValue::Boolean(true));
        let frame = self.rpc(MessageType::CmdFleetApply, &p).await?;
        Ok(frame.payload)
    }

    // ── Conversation & Contact Management ───────────────────────────

    pub async fn query_conversations(&mut self) -> Result<Vec<HashMap<String, MpValue>>, String> {
        let mut p = HashMap::new();
        p.insert("unread_only".into(), MpValue::Boolean(false));
        let frame = self.rpc(MessageType::QueryConversations, &p).await?;
        Ok(parse_conversations(&frame.payload))
    }

    pub async fn query_messages(
        &mut self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        p.insert("limit".into(), MpValue::from(limit as u64));
        let frame = self.rpc(MessageType::QueryMessages, &p).await?;
        Ok(parse_messages(&frame.payload))
    }

    pub async fn block_peer(&mut self, hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(hash));
        self.rpc(MessageType::CmdBlockPeer, &p).await.map(|_| ())
    }

    pub async fn unblock_peer(&mut self, hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(hash));
        self.rpc(MessageType::CmdUnblockPeer, &p).await.map(|_| ())
    }

    pub async fn set_auto_reply(&mut self, mode: &str, message: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("mode".into(), MpValue::from(mode));
        p.insert("message".into(), MpValue::from(message));
        self.rpc(MessageType::CmdSetAutoReply, &p).await.map(|_| ())
    }

    pub async fn query_config(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryConfig, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    // ── Tunnel Management ───────────────────────────────────────────

    pub async fn tunnel_establish(&mut self, peer_hash: &str) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        let frame = self.rpc(MessageType::CmdTunnelEstablish, &p).await?;
        Ok(mp_str(&frame.payload, "tunnel_id"))
    }

    pub async fn tunnel_teardown(&mut self, peer_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        self.rpc(MessageType::CmdTunnelTeardown, &p).await.map(|_| ())
    }

    pub async fn query_tunnels(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryTunnels, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    // ── Terminal ────────────────────────────────────────────────────

    pub async fn terminal_open(
        &mut self,
        dest: &str,
        rows: u16,
        cols: u16,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest));
        p.insert("rows".into(), MpValue::from(rows as u64));
        p.insert("cols".into(), MpValue::from(cols as u64));
        let frame = self.rpc(MessageType::CmdTerminalOpen, &p).await?;
        Ok(mp_str(&frame.payload, "session_id"))
    }

    pub async fn terminal_input(&mut self, session_id: &str, data: &[u8]) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        p.insert("data".into(), MpValue::Binary(data.to_vec()));
        self.rpc(MessageType::CmdTerminalInput, &p).await.map(|_| ())
    }

    pub async fn terminal_close(&mut self, session_id: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        self.rpc(MessageType::CmdTerminalClose, &p).await.map(|_| ())
    }

    async fn subscribe_all(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubDevices, &HashMap::new()).await?;
        self.rpc(MessageType::SubMessages, &HashMap::new()).await?;
        self.rpc(MessageType::SubLinks, &HashMap::new()).await?;
        self.rpc(MessageType::SubRoutes, &HashMap::new()).await?;
        self.rpc(MessageType::SubNetworkOperations, &HashMap::new()).await?;
        self.rpc(MessageType::SubRequests, &HashMap::new()).await?;
        self.rpc(MessageType::SubResources, &HashMap::new()).await?;
        Ok(())
    }

    async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }
}

static EMBEDDED_SESSION_ID: AtomicU64 = AtomicU64::new(0);
const BROKER_CAPACITY: usize = 32;

struct BrokerRequest {
    request_id: [u8; REQUEST_ID_SIZE],
    msg_type: MessageType,
    payload: HashMap<String, MpValue>,
    started: std::time::Instant,
    response: oneshot::Sender<Result<Frame, BridgeError>>,
}

struct PendingRequest {
    started: std::time::Instant,
    response: oneshot::Sender<Result<Frame, BridgeError>>,
}

#[derive(Default)]
struct BrokerMetrics {
    queue_depth: AtomicUsize,
    in_flight: AtomicUsize,
    completed: AtomicU64,
    timed_out: AtomicU64,
    cancelled: AtomicU64,
    overloaded: AtomicU64,
    disconnected: AtomicU64,
    reconnects: AtomicU64,
    stale_responses: AtomicU64,
    dropped_responses: AtomicU64,
    dropped_updates: AtomicU64,
    last_latency_ms: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct RequestBroker {
    generation: crate::backend::ConnectionGeneration,
    outbound: mpsc::Sender<BrokerRequest>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    capacity: Arc<Semaphore>,
    next_id: Arc<AtomicU64>,
    metrics: Arc<BrokerMetrics>,
    connected: Arc<AtomicBool>,
}

impl RequestBroker {
    fn spawn(stream: UnixStream, generation: crate::backend::ConnectionGeneration) -> Self {
        Self::spawn_with_capacity(stream, generation, BROKER_CAPACITY)
    }

    fn spawn_with_capacity(
        stream: UnixStream,
        generation: crate::backend::ConnectionGeneration,
        capacity: usize,
    ) -> Self {
        let (outbound, requests) = mpsc::channel(capacity);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let metrics = Arc::new(BrokerMetrics::default());
        metrics.reconnects.store(generation.0.saturating_sub(1), Ordering::Relaxed);
        let connected = Arc::new(AtomicBool::new(true));
        let (reader, writer) = stream.into_split();
        tokio::spawn(broker_writer(
            writer,
            requests,
            pending.clone(),
            metrics.clone(),
            connected.clone(),
        ));
        tokio::spawn(broker_reader(
            reader,
            generation,
            pending.clone(),
            metrics.clone(),
            connected.clone(),
        ));
        Self {
            generation,
            outbound,
            pending,
            capacity: Arc::new(Semaphore::new(capacity)),
            next_id: Arc::new(AtomicU64::new(0)),
            metrics,
            connected,
        }
    }

    fn request_id(&self) -> [u8; REQUEST_ID_SIZE] {
        let mut id = [0; REQUEST_ID_SIZE];
        id[..8].copy_from_slice(&self.generation.0.to_le_bytes());
        id[8..].copy_from_slice(
            &self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1).to_le_bytes(),
        );
        id
    }

    async fn rpc(
        &self,
        msg_type: MessageType,
        payload: HashMap<String, MpValue>,
        deadline: Duration,
    ) -> Result<Frame, String> {
        self.rpc_typed(msg_type, payload, deadline).await.map_err(|error| error.to_string())
    }

    async fn rpc_typed(
        &self,
        msg_type: MessageType,
        payload: HashMap<String, MpValue>,
        deadline: Duration,
    ) -> Result<Frame, BridgeError> {
        if !self.connected.load(Ordering::Acquire) {
            return Err(BridgeError::Internal("IPC broker disconnected".into()));
        }
        let _permit = self.capacity.clone().try_acquire_owned().map_err(|_| {
            self.metrics.overloaded.fetch_add(1, Ordering::Relaxed);
            BridgeError::Internal("IPC broker overloaded".into())
        })?;
        let request_id = self.request_id();
        let (response, receiver) = oneshot::channel();
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        if self
            .outbound
            .try_send(BrokerRequest {
                request_id,
                msg_type,
                payload,
                started: std::time::Instant::now(),
                response,
            })
            .is_err()
        {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            self.metrics.overloaded.fetch_add(1, Ordering::Relaxed);
            return Err(BridgeError::Internal("IPC broker overloaded".into()));
        }

        let mut guard = PendingGuard {
            request_id: Some(request_id),
            pending: self.pending.clone(),
            metrics: self.metrics.clone(),
        };
        match timeout(deadline, receiver).await {
            Ok(Ok(result)) => {
                guard.disarm();
                result
            }
            Ok(Err(_)) => {
                guard.disarm();
                Err(BridgeError::Internal("IPC broker disconnected".into()))
            }
            Err(_) => {
                self.metrics.timed_out.fetch_add(1, Ordering::Relaxed);
                guard.disarm();
                if self.pending.lock().await.remove(&request_id).is_some() {
                    self.metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
                }
                Err(BridgeError::Internal("IPC request timed out".into()))
            }
        }
    }

    pub(crate) fn diagnostics(&self) -> BrokerDiagnostics {
        BrokerDiagnostics {
            queue_depth: self.metrics.queue_depth.load(Ordering::Relaxed),
            in_flight: self.metrics.in_flight.load(Ordering::Relaxed),
            completed: self.metrics.completed.load(Ordering::Relaxed),
            timed_out: self.metrics.timed_out.load(Ordering::Relaxed),
            cancelled: self.metrics.cancelled.load(Ordering::Relaxed),
            overloaded: self.metrics.overloaded.load(Ordering::Relaxed),
            disconnected: self.metrics.disconnected.load(Ordering::Relaxed),
            reconnects: self.metrics.reconnects.load(Ordering::Relaxed),
            stale_responses: self.metrics.stale_responses.load(Ordering::Relaxed),
            dropped_responses: self.metrics.dropped_responses.load(Ordering::Relaxed),
            dropped_updates: self.metrics.dropped_updates.load(Ordering::Relaxed),
            last_latency_ms: self.metrics.last_latency_ms.load(Ordering::Relaxed),
        }
    }

    pub(crate) async fn send_chat_outcome(
        &self,
        request: &styrene_ipc::types::SendChatRequest,
    ) -> Result<styrene_ipc::types::SendChatOutcome, String> {
        let mut payload = HashMap::new();
        payload.insert("peer_hash".into(), MpValue::from(request.peer_hash.as_str()));
        payload.insert("content".into(), MpValue::from(request.content.as_str()));
        if let Some(title) = &request.title {
            payload.insert("title".into(), MpValue::from(title.as_str()));
        }
        if let Some(method) = &request.delivery_method {
            payload.insert("delivery_method".into(), MpValue::from(method.as_str()));
        }
        if !request.attachments.is_empty() {
            payload.insert(
                "attachments".into(),
                MpValue::Array(
                    request
                        .attachments
                        .iter()
                        .map(|attachment| {
                            let mut fields = vec![
                                (MpValue::from("name"), MpValue::from(attachment.name.as_str())),
                                (MpValue::from("bytes"), MpValue::Binary(attachment.bytes.clone())),
                            ];
                            if let Some(content_type) = &attachment.content_type {
                                fields.push((
                                    MpValue::from("content_type"),
                                    MpValue::from(content_type.as_str()),
                                ));
                            }
                            if let Some(expected) = &attachment.expected_sha256 {
                                fields.push((
                                    MpValue::from("expected_sha256"),
                                    MpValue::from(expected.as_str()),
                                ));
                            }
                            MpValue::Map(fields)
                        })
                        .collect(),
                ),
            );
        }
        let frame =
            self.rpc(MessageType::CmdSendChatOutcome, payload, Duration::from_secs(35)).await?;
        let outcome: styrene_ipc::types::SendChatOutcome = parse_typed_value(
            frame.payload.get("outcome").cloned().ok_or("send response omitted outcome")?,
        )?;
        if outcome.message_id.is_empty() || outcome.message.id != outcome.message_id {
            return Err("send response omitted its authoritative message projection".into());
        }
        Ok(outcome)
    }

    pub(crate) async fn draft(
        &self,
        peer_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, String> {
        let payload = HashMap::from([("peer_hash".into(), MpValue::from(peer_hash))]);
        let frame = self.rpc(MessageType::QueryDraft, payload, Duration::from_secs(5)).await?;
        match frame.payload.get("draft") {
            None | Some(MpValue::Nil) => Ok(None),
            Some(value) => parse_typed_value(value.clone()).map(Some),
        }
    }

    pub(crate) async fn save_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, String> {
        let payload = HashMap::from([
            ("peer_hash".into(), MpValue::from(peer_hash)),
            ("content".into(), MpValue::from(content)),
        ]);
        let frame = self.rpc(MessageType::CmdSetDraft, payload, Duration::from_secs(5)).await?;
        parse_typed_value(
            frame.payload.get("draft").cloned().ok_or("draft response omitted draft")?,
        )
    }

    pub(crate) async fn discard_draft(&self, peer_hash: &str) -> Result<(), String> {
        let payload = HashMap::from([("peer_hash".into(), MpValue::from(peer_hash))]);
        self.rpc(MessageType::CmdClearDraft, payload, Duration::from_secs(5)).await.map(|_| ())
    }

    pub(crate) async fn message_lifecycle(
        &self,
        message_id: &str,
        retry: bool,
    ) -> Result<styrene_ipc::types::MessagingOperationOutcome, String> {
        let payload = HashMap::from([("message_id".into(), MpValue::from(message_id))]);
        let opcode =
            if retry { MessageType::CmdRetryMessage } else { MessageType::CmdCancelMessage };
        let frame = self.rpc(opcode, payload, Duration::from_secs(35)).await?;
        parse_typed_value(
            frame.payload.get("outcome").cloned().ok_or("lifecycle response omitted outcome")?,
        )
    }

    pub(crate) async fn browse_page(&self, host: &str, path: &str) -> Result<PageResponse, String> {
        let mut payload = HashMap::new();
        payload.insert("host".into(), MpValue::from(host));
        payload.insert("path".into(), MpValue::from(path));
        payload.insert("timeout".into(), MpValue::from(25_u64));
        let deadline = if host.is_empty() || host == "local" {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(30)
        };
        let frame = self.rpc(MessageType::QueryPage, payload, deadline).await?;
        let page = parse_page_content(&frame.payload)?;
        Ok(PageResponse { page })
    }

    pub(crate) async fn local_page_inventory(
        &self,
    ) -> Result<Vec<styrene_ipc::types::PageInfo>, String> {
        let payload = HashMap::from([("host".into(), MpValue::from("local"))]);
        let frame =
            self.rpc(MessageType::CmdPageListSites, payload, Duration::from_secs(5)).await?;
        parse_typed_array(&frame.payload, "pages")
    }

    pub(crate) async fn navigate_page(
        &self,
        request: styrene_ipc::types::PageNavigationRequest,
    ) -> Result<PageResponse, String> {
        let mut payload = HashMap::new();
        let encoded = rmp_serde::to_vec_named(&request)
            .map_err(|error| format!("encode page navigation: {error}"))?;
        payload.insert("navigation".into(), MpValue::Binary(encoded));
        let frame =
            self.rpc(MessageType::CmdPageNavigate, payload, Duration::from_secs(30)).await?;
        Ok(PageResponse { page: parse_page_content(&frame.payload)? })
    }

    pub(crate) async fn close_page(&self, session_id: &str) -> Result<(), String> {
        let payload = HashMap::from([("session_id".into(), MpValue::from(session_id))]);
        self.rpc(MessageType::CmdPageDisconnect, payload, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub(crate) async fn start_file_download(
        &self,
        request: styrene_ipc::types::FileDownloadRequest,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let encoded = rmp_serde::to_vec_named(&request)
            .map_err(|error| format!("encode download request: {error}"))?;
        let payload = HashMap::from([("download_request".into(), MpValue::Binary(encoded))]);
        let frame =
            self.rpc(MessageType::CmdFileDownloadStart, payload, Duration::from_secs(5)).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub(crate) async fn file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([("download_id".into(), MpValue::from(download_id))]);
        let frame =
            self.rpc(MessageType::QueryFileDownload, payload, Duration::from_secs(5)).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub(crate) async fn cancel_file_download(
        &self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([("download_id".into(), MpValue::from(download_id))]);
        let frame =
            self.rpc(MessageType::CmdFileDownloadCancel, payload, Duration::from_secs(5)).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub(crate) async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([
            ("download_id".into(), MpValue::from(download_id)),
            ("destination".into(), MpValue::from(destination)),
        ]);
        let frame =
            self.rpc(MessageType::CmdFileDownloadSave, payload, Duration::from_secs(10)).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub(crate) async fn path_table(&self) -> Result<Vec<PathTableEntry>, String> {
        let frame =
            self.rpc(MessageType::QueryPathTable, HashMap::new(), Duration::from_secs(5)).await?;
        Ok(parse_path_table(&frame.payload))
    }

    pub(crate) async fn interface_stats(&self) -> Result<Vec<InterfaceStats>, String> {
        let frame = self
            .rpc(MessageType::QueryInterfaceStats, HashMap::new(), Duration::from_secs(5))
            .await?;
        Ok(parse_interface_stats(&frame.payload))
    }

    pub(crate) async fn links(&self) -> Result<LinkSnapshot, String> {
        let frame =
            self.rpc(MessageType::QueryLinks, HashMap::new(), Duration::from_secs(5)).await?;
        let mut snapshot = LinkSnapshot::default();
        snapshot.active = parse_typed_array(&frame.payload, "active")?;
        snapshot.history = parse_typed_array(&frame.payload, "history")?;
        Ok(snapshot)
    }

    pub(crate) async fn query_conversations(
        &self,
    ) -> Result<Vec<HashMap<String, MpValue>>, String> {
        let mut payload = HashMap::new();
        payload.insert("unread_only".into(), MpValue::Boolean(false));
        let frame =
            self.rpc(MessageType::QueryConversations, payload, Duration::from_secs(5)).await?;
        Ok(parse_conversations(&frame.payload))
    }

    pub(crate) async fn query_conversation_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<crate::backend::BackendPage<HashMap<String, MpValue>>, String> {
        let mut payload = HashMap::from([
            ("unread_only".into(), MpValue::Boolean(false)),
            ("limit".into(), MpValue::from(50_u64)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), MpValue::from(cursor));
        }
        let mut reset = false;
        let frame = match self
            .rpc_typed(MessageType::QueryConversations, payload.clone(), Duration::from_secs(5))
            .await
        {
            Err(error) if cursor.is_some() && error.cursor_stale() => {
                payload.remove("cursor");
                reset = true;
                self.rpc_typed(MessageType::QueryConversations, payload, Duration::from_secs(5))
                    .await
                    .map_err(|error| error.to_string())?
            }
            result => result.map_err(|error| error.to_string())?,
        };
        Ok(crate::backend::BackendPage {
            items: parse_conversations(&frame.payload),
            next_cursor: frame
                .payload
                .get("next_cursor")
                .and_then(MpValue::as_str)
                .map(ToOwned::to_owned),
            pagination_supported: frame.payload.contains_key("next_cursor"),
            reset,
        })
    }

    pub(crate) async fn query_messages(
        &self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, String> {
        let mut payload = HashMap::new();
        payload.insert("peer_hash".into(), MpValue::from(peer_hash));
        payload.insert("limit".into(), MpValue::from(u64::from(limit)));
        let frame = self.rpc(MessageType::QueryMessages, payload, Duration::from_secs(5)).await?;
        Ok(parse_messages(&frame.payload))
    }

    pub(crate) async fn query_message(
        &self,
        message_id: &str,
    ) -> Result<Option<MessageInfo>, String> {
        let payload = HashMap::from([("message_id".into(), MpValue::from(message_id))]);
        let frame = self.rpc(MessageType::QueryMessage, payload, Duration::from_secs(5)).await?;
        match frame.payload.get("message") {
            None | Some(MpValue::Nil) => Ok(None),
            Some(value) => rmpv::ext::from_value(value.clone())
                .map(Some)
                .map_err(|error| format!("invalid message projection: {error}")),
        }
    }

    pub(crate) async fn query_message_page(
        &self,
        peer_hash: &str,
        cursor: Option<&str>,
    ) -> Result<crate::backend::BackendPage<MessageInfo>, String> {
        let mut payload = HashMap::from([
            ("peer_hash".into(), MpValue::from(peer_hash)),
            ("limit".into(), MpValue::from(50_u64)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), MpValue::from(cursor));
        }
        let mut reset = false;
        let frame = match self
            .rpc_typed(MessageType::QueryMessages, payload.clone(), Duration::from_secs(5))
            .await
        {
            Err(error) if cursor.is_some() && error.cursor_stale() => {
                payload.remove("cursor");
                reset = true;
                self.rpc_typed(MessageType::QueryMessages, payload, Duration::from_secs(5))
                    .await
                    .map_err(|error| error.to_string())?
            }
            result => result.map_err(|error| error.to_string())?,
        };
        Ok(crate::backend::BackendPage {
            items: parse_messages(&frame.payload),
            next_cursor: frame
                .payload
                .get("next_cursor")
                .and_then(MpValue::as_str)
                .map(ToOwned::to_owned),
            pagination_supported: frame.payload.contains_key("next_cursor"),
            reset,
        })
    }

    pub(crate) async fn propagation_snapshot(
        &self,
        cursor: Option<&str>,
    ) -> Result<PropagationSnapshot, String> {
        let mut payload = HashMap::new();
        payload.insert("limit".into(), MpValue::from(100_u64));
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), MpValue::from(cursor));
        }
        let frame =
            self.rpc(MessageType::QueryPropagation, payload, Duration::from_secs(5)).await?;
        Ok(parse_propagation(&frame.payload))
    }

    #[allow(dead_code)] // Parsed for the task 11.3 UI workflow without fabricating UI stages here.
    pub(crate) async fn standard_propagation_snapshot(
        &self,
    ) -> Result<StandardPropagationSnapshot, String> {
        let frame = self
            .rpc(MessageType::QueryStandardPropagation, HashMap::new(), Duration::from_secs(5))
            .await?;
        let encoded = rmp_serde::to_vec_named(&frame.payload)
            .map_err(|error| format!("encode standard propagation snapshot: {error}"))?;
        rmp_serde::from_slice(&encoded)
            .map_err(|error| format!("decode standard propagation snapshot: {error}"))
    }

    pub(crate) async fn device_status(
        &self,
        destination: &str,
    ) -> Result<RemoteStatusInfo, String> {
        let mut payload = HashMap::new();
        payload.insert("destination_hash".into(), MpValue::from(destination));
        let frame =
            self.rpc(MessageType::CmdDeviceStatus, payload, Duration::from_secs(30)).await?;
        let mut status = RemoteStatusInfo::default();
        status.destination_hash = mp_str(&frame.payload, "destination_hash");
        status.uptime = frame.payload.get("uptime").and_then(MpValue::as_u64);
        status.daemon_version =
            frame.payload.get("version").and_then(MpValue::as_str).map(ToOwned::to_owned);
        Ok(status)
    }

    pub(crate) async fn exec(
        &self,
        destination: &str,
        command: &str,
        args: &[String],
    ) -> Result<ExecResult, String> {
        let mut payload = HashMap::new();
        payload.insert("destination_hash".into(), MpValue::from(destination));
        payload.insert("command".into(), MpValue::from(command));
        payload.insert(
            "args".into(),
            MpValue::Array(args.iter().map(|arg| MpValue::from(arg.as_str())).collect()),
        );
        let frame = self.rpc(MessageType::CmdExec, payload, Duration::from_secs(60)).await?;
        let mut result = ExecResult::default();
        result.exit_code = frame
            .payload
            .get("exit_code")
            .and_then(MpValue::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1);
        result.stdout = mp_str(&frame.payload, "stdout");
        result.stderr = mp_str(&frame.payload, "stderr");
        Ok(result)
    }

    pub(crate) async fn reboot(
        &self,
        destination: &str,
        delay: Option<u64>,
    ) -> Result<RebootResult, String> {
        let mut payload = HashMap::new();
        payload.insert("destination_hash".into(), MpValue::from(destination));
        if let Some(delay) = delay {
            payload.insert("delay".into(), MpValue::from(delay));
        }
        let frame =
            self.rpc(MessageType::CmdRebootDevice, payload, Duration::from_secs(30)).await?;
        let mut result = RebootResult::default();
        result.accepted = frame.payload.get("accepted").and_then(MpValue::as_bool).unwrap_or(false);
        result.delay_secs = frame.payload.get("delay_secs").and_then(MpValue::as_u64);
        Ok(result)
    }

    pub(crate) async fn fleet_apply(
        &self,
        destination: &str,
        profile_base64: &str,
    ) -> Result<styrene_ipc::types::ConfigApplyResult, String> {
        let mut payload = HashMap::new();
        payload.insert("destination_hash".into(), MpValue::from(destination));
        payload.insert("profile".into(), MpValue::from(profile_base64));
        payload.insert("verify".into(), MpValue::Boolean(true));
        let frame = self.rpc(MessageType::CmdFleetApply, payload, Duration::from_secs(60)).await?;
        let mut result = styrene_ipc::types::ConfigApplyResult::default();
        result.success = frame.payload.get("success").and_then(MpValue::as_bool).unwrap_or(false);
        result.verified = frame.payload.get("verified").and_then(MpValue::as_bool).unwrap_or(false);
        result.exit_code = frame
            .payload
            .get("exit_code")
            .and_then(MpValue::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1);
        result.stdout = mp_str(&frame.payload, "stdout");
        result.stderr = mp_str(&frame.payload, "stderr");
        Ok(result)
    }

    pub(crate) async fn block_peer(&self, identity_hash: &str) -> Result<(), String> {
        let mut payload = HashMap::new();
        payload.insert("identity_hash".into(), MpValue::from(identity_hash));
        self.rpc(MessageType::CmdBlockPeer, payload, Duration::from_secs(5)).await.map(|_| ())
    }

    pub(crate) async fn start_network_operation(
        &self,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String> {
        let mut payload = HashMap::new();
        payload.insert("kind".into(), MpValue::from(request.kind.as_str()));
        payload.insert("timeout_ms".into(), MpValue::from(request.timeout_ms));
        if let Some(destination) = request.destination_hash {
            payload.insert("destination_hash".into(), MpValue::from(destination));
        }
        if let Some(link_id) = request.link_id {
            payload.insert("link_id".into(), MpValue::from(link_id));
        }
        let frame = self
            .rpc(MessageType::CmdNetworkOperationStart, payload, Duration::from_secs(5))
            .await?;
        parse_network_operation(&frame.payload)
    }

    pub(crate) async fn cancel_network_operation(
        &self,
        operation_id: &str,
    ) -> Result<NetworkOperationInfo, String> {
        let payload = HashMap::from([("operation_id".into(), MpValue::from(operation_id))]);
        let frame = self
            .rpc(MessageType::CmdNetworkOperationCancel, payload, Duration::from_secs(5))
            .await?;
        parse_network_operation(&frame.payload)
    }

    pub(crate) async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, String> {
        let frame = self
            .rpc(MessageType::QueryNetworkOperation, HashMap::new(), Duration::from_secs(5))
            .await?;
        parse_typed_array(&frame.payload, "operations")
    }

    pub(crate) async fn start_request(
        &self,
        request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, String> {
        let mut payload = HashMap::from([
            ("link_id".into(), MpValue::from(request.link_id)),
            ("path".into(), MpValue::from(request.path)),
            ("data".into(), MpValue::Binary(request.data)),
            ("timeout_ms".into(), MpValue::from(request.timeout_ms)),
            ("max_response_size".into(), MpValue::from(request.max_response_size)),
        ]);
        if let Some(correlation_id) = request.correlation_id {
            payload.insert("correlation_id".into(), MpValue::from(correlation_id));
        }
        let frame = self.rpc(MessageType::CmdRequestStart, payload, Duration::from_secs(5)).await?;
        parse_typed_map(&frame.payload)
    }

    pub(crate) async fn cancel_request(
        &self,
        request_id: &str,
    ) -> Result<RequestObservationInfo, String> {
        let payload = HashMap::from([("request_id".into(), MpValue::from(request_id))]);
        let frame =
            self.rpc(MessageType::CmdRequestCancel, payload, Duration::from_secs(5)).await?;
        parse_typed_map(&frame.payload)
    }

    pub(crate) async fn requests(&self) -> Result<Vec<RequestObservationInfo>, String> {
        let frame =
            self.rpc(MessageType::QueryRequests, HashMap::new(), Duration::from_secs(5)).await?;
        parse_typed_array(&frame.payload, "requests")
    }

    pub(crate) async fn resources(&self) -> Result<Vec<ResourceTransferInfo>, String> {
        let frame =
            self.rpc(MessageType::QueryResources, HashMap::new(), Duration::from_secs(5)).await?;
        parse_typed_array(&frame.payload, "resources")
    }

    pub(crate) async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, String> {
        let payload = HashMap::from([("resource_hash".into(), MpValue::from(resource_hash))]);
        let frame =
            self.rpc(MessageType::CmdResourceCancel, payload, Duration::from_secs(5)).await?;
        Ok(frame.payload.get("accepted").and_then(MpValue::as_bool).unwrap_or(false))
    }
}

struct PendingGuard {
    request_id: Option<[u8; REQUEST_ID_SIZE]>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<BrokerMetrics>,
}

impl PendingGuard {
    fn disarm(&mut self) {
        self.request_id = None;
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        self.metrics.cancelled.fetch_add(1, Ordering::Relaxed);
        let pending = self.pending.clone();
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            if pending.lock().await.remove(&request_id).is_some() {
                metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
            }
        });
    }
}

async fn broker_writer(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut requests: mpsc::Receiver<BrokerRequest>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<BrokerMetrics>,
    connected: Arc<AtomicBool>,
) {
    while let Some(request) = requests.recv().await {
        metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
        if request.response.is_closed() {
            continue;
        }
        let request_id = request.request_id;
        pending.lock().await.insert(
            request_id,
            PendingRequest { started: request.started, response: request.response },
        );
        metrics.in_flight.fetch_add(1, Ordering::Relaxed);
        if let Err(error) =
            wire::write_frame_async(&mut writer, request.msg_type, &request_id, &request.payload)
                .await
        {
            if let Some(request) = pending.lock().await.remove(&request_id) {
                metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
                let _ = request
                    .response
                    .send(Err(BridgeError::Internal(format!("IPC write failed: {error}"))));
            }
            disconnect_pending(
                &pending,
                &metrics,
                &connected,
                format!("IPC write failed: {error}"),
            )
            .await;
            break;
        }
    }
}

async fn broker_reader(
    mut reader: tokio::net::unix::OwnedReadHalf,
    generation: crate::backend::ConnectionGeneration,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<BrokerMetrics>,
    connected: Arc<AtomicBool>,
) {
    loop {
        let frame = match wire::read_frame_async(&mut reader).await {
            Ok(frame) => frame,
            Err(error) => {
                disconnect_pending(
                    &pending,
                    &metrics,
                    &connected,
                    format!("IPC read failed: {error}"),
                )
                .await;
                break;
            }
        };
        let frame_generation =
            u64::from_le_bytes(frame.request_id[..8].try_into().unwrap_or([0; 8]));
        if frame_generation != generation.0 {
            metrics.stale_responses.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let Some(request) = pending.lock().await.remove(&frame.request_id) else {
            metrics.dropped_responses.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
        metrics.completed.fetch_add(1, Ordering::Relaxed);
        metrics.last_latency_ms.store(
            u64::try_from(request.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        let request_id = frame.request_id;
        let _ = request.response.send(validate_response(frame, request_id));
    }
}

async fn disconnect_pending(
    pending: &Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>,
    metrics: &BrokerMetrics,
    connected: &AtomicBool,
    reason: String,
) {
    if connected.swap(false, Ordering::AcqRel) {
        metrics.disconnected.fetch_add(1, Ordering::Relaxed);
    }
    let requests = std::mem::take(&mut *pending.lock().await);
    metrics.in_flight.fetch_sub(requests.len(), Ordering::Relaxed);
    for request in requests.into_values() {
        let _ = request.response.send(Err(BridgeError::Internal(reason.clone())));
    }
}

pub(crate) struct EmbeddedDaemon {
    handle: styrened::daemon::DaemonHandle,
    root: std::path::PathBuf,
}

impl EmbeddedDaemon {
    pub(crate) async fn shutdown(self) {
        self.handle.shutdown().await;
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(target: "dx::bridge", %error, path = %self.root.display(), "embedded state cleanup failed");
            }
        }
    }

    #[cfg(test)]
    fn root(&self) -> &std::path::Path {
        &self.root
    }
}

pub(crate) async fn connect_ipc(
    socket_path: &std::path::Path,
    generation: crate::backend::ConnectionGeneration,
) -> Result<(RequestBroker, mpsc::Receiver<DaemonEvent>), String> {
    let mut initial = open_bridge(socket_path).await?;
    if !initial.ping().await {
        return Err("daemon not responsive".into());
    }

    let (tx, rx) = mpsc::channel(512);
    let _ = tx.send(DaemonEvent::Connected).await;
    if let Ok(info) = initial.identity().await {
        let _ = tx.send(DaemonEvent::Identity(info)).await;
    }
    if let Ok(status) = initial.status().await {
        let _ = tx.send(DaemonEvent::Status(status)).await;
    }
    if let Ok(devices) = initial.devices().await {
        for device in devices {
            let _ = tx.send(DaemonEvent::PeerDiscovered(device)).await;
        }
    }
    if let Ok(paths) = initial.path_table().await {
        let _ = tx.send(DaemonEvent::PathTable(paths)).await;
    }

    let broker = RequestBroker::spawn(initial.into_stream(), generation);
    let mut event_client = open_bridge(socket_path).await?;
    let event_generation = event_client
        .status()
        .await?
        .connection_generation
        .ok_or("event connection did not report a generation")?;
    let _ = tx.send(DaemonEvent::EventGeneration(event_generation)).await;
    event_client.subscribe_all().await?;
    tokio::spawn(event_reader(
        event_client.into_stream(),
        tx.clone(),
        broker.metrics.clone(),
        event_generation,
    ));

    spawn_poller(open_bridge(socket_path).await?, broker.clone(), tx, broker.metrics.clone());

    Ok((broker, rx))
}

pub(crate) async fn connect_embedded(
    ephemeral: bool,
    generation: crate::backend::ConnectionGeneration,
) -> Result<(RequestBroker, mpsc::Receiver<DaemonEvent>, EmbeddedDaemon), String> {
    let session_id = EMBEDDED_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir()
        .join(format!("styrene-dx-embedded-{}-{session_id}", std::process::id()));
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create embedded state directory: {error}"))?;
    let sock = root.join("control.sock");

    // Boot daemon in-process — same capabilities as standalone styrened.
    // Loads ~/.config/styrene/config.toml if present (TCP clients, hub
    // connections, RBAC policy, etc.). No capability difference vs external.
    let config = styrened::daemon::DaemonConfig2 {
        db: Some(root.join("messages.db")),
        config: None,
        identity: None,
        socket: Some(sock.clone()),
        ephemeral,
    };

    info!(target: "dx::bridge", "starting embedded daemon");
    let handle = match styrened::daemon::start(config).await {
        Ok(handle) => handle,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&root);
            return Err(format!("embedded daemon boot failed: {error}"));
        }
    };
    let (broker, events) = match connect_ipc(&sock, generation).await {
        Ok(connection) => connection,
        Err(error) => {
            handle.shutdown().await;
            let _ = std::fs::remove_dir_all(&root);
            return Err(format!("connect embedded: {error}"));
        }
    };
    info!(target: "dx::bridge", "embedded daemon fully initialized");

    Ok((broker, events, EmbeddedDaemon { handle, root }))
}

async fn open_bridge(socket_path: &std::path::Path) -> Result<DaemonBridge, String> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| format!("connect: {e}"))?;
    Ok(DaemonBridge { stream, next_id: 0, usable: true })
}

async fn event_reader(
    mut stream: UnixStream,
    tx: mpsc::Sender<DaemonEvent>,
    metrics: Arc<BrokerMetrics>,
    event_generation: u64,
) {
    info!(target: "dx::reader", "event reader started");
    loop {
        match wire::read_frame_async(&mut stream).await {
            Ok(frame) => {
                debug!(target: "dx::reader", msg_type = ?frame.msg_type, "received frame");
                if let Some(ev) = frame_to_event(frame) {
                    match tx.try_send(ev) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            metrics.dropped_updates.fetch_add(1, Ordering::Relaxed);
                            warn!(target: "dx::reader", "event channel full, dropping update");
                            let _ = tx
                                .send(DaemonEvent::ReconcileRequired {
                                    dropped: 1,
                                    connection_generation: event_generation,
                                })
                                .await;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            warn!(target: "dx::reader", "channel closed, stopping");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                error!(target: "dx::reader", %e, "stream read error");
                let _ = tx.send(DaemonEvent::Disconnected(e.to_string())).await;
                break;
            }
        }
    }
}

fn spawn_poller(
    mut bridge: DaemonBridge,
    broker: RequestBroker,
    tx: mpsc::Sender<DaemonEvent>,
    metrics: Arc<BrokerMetrics>,
) {
    tokio::spawn(
        async move {
            info!(target: "dx::poller", "poller started");

            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;

                let start = std::time::Instant::now();
                match broker
                    .rpc(MessageType::QueryStatus, HashMap::new(), Duration::from_secs(5))
                    .await
                {
                    Ok(frame) => send_polled_event(
                        &tx,
                        DaemonEvent::Status(parse_status(&frame.payload)),
                        &metrics,
                    ),
                    Err(error) => {
                        let _ = tx.send(DaemonEvent::Disconnected(error)).await;
                        break;
                    }
                }
                if let Ok(devices) = bridge.devices().await {
                    debug!(target: "dx::poller", count = devices.len(), "polled devices");
                    for dev in devices {
                        send_polled_event(&tx, DaemonEvent::PeerDiscovered(dev), &metrics);
                    }
                }
                if let Ok(pages) = broker.local_page_inventory().await {
                    send_polled_event(&tx, DaemonEvent::LocalPageInventory(pages), &metrics);
                }
                match broker.path_table().await {
                    Ok(paths) => {
                        info!(target: "dx::poller", count = paths.len(), "polled path table");
                        send_polled_event(&tx, DaemonEvent::PathTable(paths), &metrics);
                    }
                    Err(e) => warn!(target: "dx::poller", %e, "path table poll failed"),
                }
                if let Ok(operations) = broker.network_operations().await {
                    for operation in operations {
                        send_polled_event(&tx, DaemonEvent::NetworkOperation(operation), &metrics);
                    }
                }
                if let Ok(requests) = broker.requests().await {
                    for request in requests {
                        send_polled_event(&tx, DaemonEvent::Request(request), &metrics);
                    }
                }
                if let Ok(resources) = broker.resources().await {
                    for resource in resources {
                        send_polled_event(&tx, DaemonEvent::Resource(resource), &metrics);
                    }
                }
                if let Ok(links) = broker.links().await {
                    for link in links.active.into_iter().chain(links.history) {
                        send_polled_event(&tx, DaemonEvent::LinkObservation(link), &metrics);
                    }
                }

                let elapsed_ms = start.elapsed().as_millis();
                debug!(target: "dx::poller", elapsed_ms, "tick complete");
            }
        }
        .instrument(info_span!("poller")),
    );
}

fn send_polled_event(tx: &mpsc::Sender<DaemonEvent>, event: DaemonEvent, metrics: &BrokerMetrics) {
    if tx.try_send(event).is_err() {
        metrics.dropped_updates.fetch_add(1, Ordering::Relaxed);
    }
}

fn validate_response(
    frame: Frame,
    request_id: [u8; REQUEST_ID_SIZE],
) -> Result<Frame, BridgeError> {
    if frame.request_id != request_id {
        return Err(BridgeError::Internal("IPC response request ID mismatch".into()));
    }
    if !frame.msg_type.is_response() {
        return Err(BridgeError::Internal(format!(
            "unexpected IPC frame type {:?}",
            frame.msg_type
        )));
    }
    if frame.msg_type == MessageType::Error {
        return Err(parse_bridge_error(&frame.payload));
    }
    Ok(frame)
}

fn parse_bridge_error(payload: &HashMap<String, MpValue>) -> BridgeError {
    let message = payload
        .get("message")
        .or_else(|| payload.get("error"))
        .and_then(MpValue::as_str)
        .unwrap_or("IPC error")
        .to_string();
    let detail = |prefix: &str| message.strip_prefix(prefix).unwrap_or(&message).to_string();
    match payload.get("kind").and_then(MpValue::as_str) {
        Some("not_implemented") => {
            BridgeError::Ipc(IpcError::NotImplemented { method: detail("not implemented: ") })
        }
        Some("unavailable") => {
            BridgeError::Ipc(IpcError::Unavailable { reason: detail("unavailable: ") })
        }
        Some("timeout") => BridgeError::Ipc(IpcError::Timeout { operation: detail("timeout: ") }),
        Some("invalid_request") => {
            BridgeError::Ipc(IpcError::InvalidRequest { message: detail("invalid request: ") })
        }
        Some("not_found") => {
            BridgeError::Ipc(IpcError::NotFound { resource: detail("not found: ") })
        }
        Some("conflict") => BridgeError::Ipc(IpcError::Conflict {
            message: if payload.get("code").and_then(MpValue::as_str) == Some("cursor_stale") {
                "cursor_stale".into()
            } else {
                detail("conflict: ")
            },
        }),
        Some("denied") => BridgeError::Ipc(IpcError::Denied { capability: detail("denied: ") }),
        Some("transport") => {
            BridgeError::Ipc(IpcError::Transport { message: detail("transport error: ") })
        }
        Some("internal") => {
            BridgeError::Ipc(IpcError::Internal { message: detail("internal error: ") })
        }
        _ => BridgeError::Internal(message),
    }
}

fn frame_to_event(frame: Frame) -> Option<DaemonEvent> {
    match frame.msg_type {
        MessageType::EventDevice => {
            let dev = parse_device_event(&frame.payload)?;
            Some(DaemonEvent::PeerDiscovered(dev))
        }
        MessageType::EventMessage => {
            let msg = parse_message_event(&frame.payload)?;
            Some(DaemonEvent::MessageReceived(Box::new(msg)))
        }
        MessageType::EventLink => {
            parse_typed_map(&frame.payload).ok().map(DaemonEvent::LinkObservation)
        }
        MessageType::EventRoute => {
            parse_route_event(&frame.payload).map(DaemonEvent::RouteLifecycle)
        }
        MessageType::EventNetworkOperation => {
            parse_network_operation(&frame.payload).ok().map(DaemonEvent::NetworkOperation)
        }
        MessageType::EventRequest
            if frame.payload.get("kind").and_then(MpValue::as_str)
                == Some("reconcile_required") =>
        {
            Some(DaemonEvent::ReconcileRequests {
                dropped: frame.payload.get("dropped").and_then(MpValue::as_u64).unwrap_or(0),
                connection_generation: frame
                    .payload
                    .get("connection_generation")
                    .and_then(MpValue::as_u64)
                    .unwrap_or(0),
            })
        }
        MessageType::EventRequest => parse_typed_map(&frame.payload).ok().map(DaemonEvent::Request),
        MessageType::EventResource => {
            parse_typed_map(&frame.payload).ok().map(DaemonEvent::Resource)
        }
        MessageType::EventReconcileRequired => Some(DaemonEvent::ReconcileRequired {
            dropped: frame.payload.get("dropped").and_then(MpValue::as_u64).unwrap_or(0),
            connection_generation: frame
                .payload
                .get("connection_generation")
                .and_then(MpValue::as_u64)
                .unwrap_or(0),
        }),
        MessageType::EventStandardPropagationChanged => {
            Some(DaemonEvent::StandardPropagationChanged {
                connection_generation: frame
                    .payload
                    .get("connection_generation")
                    .and_then(MpValue::as_u64)
                    .unwrap_or(0),
            })
        }
        MessageType::EventMessagingOperation => frame
            .payload
            .get("outcome")
            .cloned()
            .and_then(|value| rmpv::ext::from_value(value).ok())
            .map(Box::new)
            .map(DaemonEvent::MessagingOperation),
        _ => None,
    }
}

fn payload_value(payload: &HashMap<String, MpValue>) -> MpValue {
    MpValue::Map(
        payload.iter().map(|(key, value)| (MpValue::from(key.as_str()), value.clone())).collect(),
    )
}

fn parse_typed_map<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
) -> Result<T, String> {
    rmpv::ext::from_value(payload_value(payload))
        .map_err(|error| format!("decode typed IPC payload: {error}"))
}

fn parse_typed_value<T: serde::de::DeserializeOwned>(value: MpValue) -> Result<T, String> {
    rmpv::ext::from_value(value).map_err(|error| format!("decode typed IPC value: {error}"))
}

fn parse_typed_array<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
    key: &str,
) -> Result<Vec<T>, String> {
    payload
        .get(key)
        .and_then(MpValue::as_array)
        .ok_or_else(|| format!("daemon response omitted {key}"))?
        .iter()
        .cloned()
        .map(|value| rmpv::ext::from_value(value).map_err(|error| format!("decode {key}: {error}")))
        .collect()
}

fn parse_typed_payload_key<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
    key: &str,
) -> Result<T, String> {
    let bytes = payload
        .get(key)
        .and_then(MpValue::as_slice)
        .ok_or_else(|| format!("daemon response omitted typed {key} payload"))?;
    rmp_serde::from_slice(bytes).map_err(|error| format!("decode typed {key}: {error}"))
}

fn parse_network_operation(
    payload: &HashMap<String, MpValue>,
) -> Result<NetworkOperationInfo, String> {
    let mut operation = NetworkOperationInfo::default();
    operation.operation_id = mp_str(payload, "operation_id");
    if operation.operation_id.is_empty() {
        return Err("daemon response omitted operation_id".into());
    }
    operation.kind = match payload.get("kind").and_then(MpValue::as_str) {
        Some("announce") => NetworkOperationKind::Announce,
        Some("path_request") => NetworkOperationKind::PathRequest,
        Some("probe") => NetworkOperationKind::Probe,
        Some("link_open") => NetworkOperationKind::LinkOpen,
        Some("link_close") => NetworkOperationKind::LinkClose,
        _ => NetworkOperationKind::Unknown,
    };
    operation.destination_hash =
        payload.get("destination_hash").and_then(MpValue::as_str).map(ToOwned::to_owned);
    operation.link_id = payload.get("link_id").and_then(MpValue::as_str).map(ToOwned::to_owned);
    operation.started_unix_ms =
        payload.get("started_unix_ms").and_then(MpValue::as_i64).unwrap_or(0);
    operation.deadline_unix_ms =
        payload.get("deadline_unix_ms").and_then(MpValue::as_i64).unwrap_or(0);
    operation.cancellable = payload.get("cancellable").and_then(MpValue::as_bool).unwrap_or(false);
    operation.progress = match payload.get("progress").and_then(MpValue::as_str) {
        Some("accepted") => NetworkOperationProgress::Accepted,
        Some("dispatched") => NetworkOperationProgress::Dispatched,
        Some("awaiting_path") => NetworkOperationProgress::AwaitingPath,
        Some("awaiting_link") => NetworkOperationProgress::AwaitingLink,
        Some("awaiting_probe") => NetworkOperationProgress::AwaitingProbe,
        Some("awaiting_close") => NetworkOperationProgress::AwaitingClose,
        _ => NetworkOperationProgress::Unknown,
    };
    operation.outcome = payload.get("outcome").and_then(MpValue::as_str).map(|value| match value {
        "succeeded" => NetworkOperationOutcome::Succeeded,
        "dispatched" => NetworkOperationOutcome::Dispatched,
        "timed_out" => NetworkOperationOutcome::TimedOut,
        "denied" => NetworkOperationOutcome::Denied,
        "unavailable" => NetworkOperationOutcome::Unavailable,
        "cancelled" => NetworkOperationOutcome::Cancelled,
        "failed" => NetworkOperationOutcome::Failed,
        _ => NetworkOperationOutcome::Unknown,
    });
    operation.detail = payload.get("detail").and_then(MpValue::as_str).map(ToOwned::to_owned);
    operation.rtt_ms = payload.get("rtt_ms").and_then(MpValue::as_f64);
    operation.observation.source = ObservationSource::OperationCoordinator;
    operation.observation.observed_at = payload.get("observed_at").and_then(MpValue::as_i64);
    operation.observation.connection_generation =
        payload.get("connection_generation").and_then(MpValue::as_u64);
    operation.observation.correlation_id =
        payload.get("correlation_id").and_then(MpValue::as_str).map(ToOwned::to_owned);
    Ok(operation)
}

fn parse_route_event(payload: &HashMap<String, MpValue>) -> Option<RouteEventInfo> {
    let destination_hash = mp_str(payload, "destination_hash");
    if destination_hash.is_empty() {
        return None;
    }
    let mut event = RouteEventInfo::default();
    event.kind = match payload.get("kind").and_then(MpValue::as_str) {
        Some("discovered") => RouteEventKind::Discovered,
        Some("lost") => RouteEventKind::Lost,
        Some("rediscovered") => RouteEventKind::Rediscovered,
        _ => RouteEventKind::Unknown,
    };
    event.loss_reason =
        payload.get("loss_reason").and_then(MpValue::as_str).map(|reason| match reason {
            "expired" => RouteLossReason::Expired,
            "interface_unavailable" => RouteLossReason::InterfaceUnavailable,
            _ => RouteLossReason::Unknown,
        });
    event.route.destination_hash = destination_hash;
    event.route.hops =
        payload.get("hops").and_then(MpValue::as_u64).and_then(|value| u32::try_from(value).ok());
    event.route.next_hop = payload.get("next_hop").and_then(MpValue::as_str).map(ToOwned::to_owned);
    event.route.interface =
        payload.get("interface").and_then(MpValue::as_str).map(ToOwned::to_owned);
    event.route.expires = payload.get("expires").and_then(MpValue::as_i64);
    event.route.observation.source = ObservationSource::TransportPathTable;
    event.route.observation.observed_at =
        payload.get("route_observed_at").and_then(MpValue::as_i64);
    event.route.observation.connection_generation =
        payload.get("route_connection_generation").and_then(MpValue::as_u64);
    event.route.observation.age_secs = payload.get("route_age_secs").and_then(MpValue::as_u64);
    event.route.observation.freshness_threshold_secs =
        payload.get("route_freshness_threshold_secs").and_then(MpValue::as_u64);
    event.route.observation.stale =
        payload.get("route_stale").and_then(MpValue::as_bool).unwrap_or(false);
    event.observation.source = match payload.get("source").and_then(MpValue::as_str) {
        Some("transport_path_table") => ObservationSource::TransportPathTable,
        _ => ObservationSource::Unknown,
    };
    event.observation.observed_at = payload.get("observed_at").and_then(MpValue::as_i64);
    event.observation.connection_generation =
        payload.get("connection_generation").and_then(MpValue::as_u64);
    event.observation.age_secs = payload.get("age_secs").and_then(MpValue::as_u64);
    event.observation.freshness_threshold_secs =
        payload.get("freshness_threshold_secs").and_then(MpValue::as_u64);
    event.observation.stale = payload.get("stale").and_then(MpValue::as_bool).unwrap_or(false);
    event.observation.correlation_id =
        payload.get("correlation_id").and_then(MpValue::as_str).map(ToOwned::to_owned);
    Some(event)
}

// ── Payload Parsers ─────────────────────────────────────────────────────────

fn mp_str(p: &HashMap<String, MpValue>, key: &str) -> String {
    p.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn parse_page_content(
    payload: &HashMap<String, MpValue>,
) -> Result<styrene_ipc::types::PageContent, String> {
    let bytes = match payload.get("page") {
        Some(MpValue::Binary(bytes)) => bytes,
        _ => return Err("daemon page response omitted typed page payload".into()),
    };
    rmp_serde::from_slice(bytes).map_err(|error| format!("decode typed page payload: {error}"))
}

fn parse_observation(map: &[(MpValue, MpValue)]) -> ObservationMetadata {
    let item =
        |key: &str| map.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value);
    let mut observation = ObservationMetadata::default();
    observation.source = match item("source").and_then(MpValue::as_str) {
        Some("runtime_interface_registry") => ObservationSource::RuntimeInterfaceRegistry,
        Some("transport_path_table") => ObservationSource::TransportPathTable,
        Some("fixture") => ObservationSource::Fixture,
        _ => ObservationSource::Unknown,
    };
    observation.observed_at = item("observed_at").and_then(MpValue::as_i64);
    observation.connection_generation = item("connection_generation").and_then(MpValue::as_u64);
    observation.age_secs = item("age_secs").and_then(MpValue::as_u64);
    observation.freshness_threshold_secs =
        item("freshness_threshold_secs").and_then(MpValue::as_u64);
    observation.stale = item("stale").and_then(MpValue::as_bool).unwrap_or(false);
    observation.correlation_id =
        item("correlation_id").and_then(MpValue::as_str).map(ToOwned::to_owned);
    observation
}

fn parse_path_table(payload: &HashMap<String, MpValue>) -> Vec<PathTableEntry> {
    payload
        .get("paths")
        .and_then(MpValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let map = value.as_map()?;
            let get = |key: &str| -> String {
                map.iter()
                    .find(|(item, _)| item.as_str() == Some(key))
                    .and_then(|(_, item)| item.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let destination_hash = get("destination_hash");
            if destination_hash.is_empty() {
                return None;
            }
            let hops = map
                .iter()
                .find(|(item, _)| item.as_str() == Some("hops"))
                .and_then(|(_, item)| item.as_u64())
                .and_then(|item| u8::try_from(item).ok())
                .unwrap_or(0);
            Some(PathTableEntry {
                destination_hash,
                hops,
                next_hop: get("next_hop"),
                interface: get("interface"),
                expires: map
                    .iter()
                    .find(|(item, _)| item.as_str() == Some("expires"))
                    .and_then(|(_, item)| item.as_i64()),
                observation: parse_observation(map),
            })
        })
        .collect()
}

fn parse_interface_stats(payload: &HashMap<String, MpValue>) -> Vec<InterfaceStats> {
    parse_typed_array(payload, "interfaces").unwrap_or_default()
}

fn parse_conversations(payload: &HashMap<String, MpValue>) -> Vec<HashMap<String, MpValue>> {
    payload
        .get("conversations")
        .and_then(MpValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let mut result = HashMap::new();
            for (key, value) in value.as_map()? {
                if let Some(key) = key.as_str() {
                    result.insert(key.to_string(), value.clone());
                }
            }
            Some(result)
        })
        .collect()
}

fn parse_messages(payload: &HashMap<String, MpValue>) -> Vec<MessageInfo> {
    payload
        .get("messages")
        .and_then(MpValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let map = value.as_map()?;
            let message = parse_message_map(map);
            (!message.id.is_empty()).then_some(message)
        })
        .collect()
}

fn parse_propagation(payload: &HashMap<String, MpValue>) -> PropagationSnapshot {
    let mut snapshot = PropagationSnapshot::default();
    snapshot.enabled = payload.get("enabled").and_then(MpValue::as_bool).unwrap_or(false);
    snapshot.queue_count = payload
        .get("queue_count")
        .and_then(MpValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    snapshot.queue_size_bytes =
        payload.get("queue_size_bytes").and_then(MpValue::as_u64).unwrap_or(0);
    snapshot.expiry_secs = payload.get("expiry_secs").and_then(MpValue::as_u64).unwrap_or(0);
    snapshot.capacity_bytes = payload.get("capacity_bytes").and_then(MpValue::as_u64);
    snapshot.peer_state_supported =
        payload.get("peer_state_supported").and_then(MpValue::as_bool).unwrap_or(false);
    snapshot.sync_state_supported =
        payload.get("sync_state_supported").and_then(MpValue::as_bool).unwrap_or(false);
    snapshot.next_cursor =
        payload.get("next_cursor").and_then(MpValue::as_str).map(ToOwned::to_owned);
    snapshot.queue = payload
        .get("queue")
        .and_then(MpValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let map = value.as_map()?;
            let item = |key: &str| {
                map.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value)
            };
            let mut entry = PropagationQueueEntry::default();
            entry.id = item("id").and_then(MpValue::as_str)?.to_string();
            entry.destination_hash =
                item("destination_hash").and_then(MpValue::as_str)?.to_string();
            entry.source_hash =
                item("source_hash").and_then(MpValue::as_str).map(ToOwned::to_owned);
            entry.received_at = item("received_at").and_then(MpValue::as_i64).unwrap_or(0);
            entry.expires_at = item("expires_at").and_then(MpValue::as_i64).unwrap_or(0);
            entry.size_bytes = item("size_bytes").and_then(MpValue::as_u64).unwrap_or(0);
            entry.attempts = item("attempts")
                .and_then(MpValue::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            entry.state = item("state").and_then(MpValue::as_str).unwrap_or("unknown").to_string();
            Some(entry)
        })
        .collect();
    snapshot
}

fn parse_identity(p: &HashMap<String, MpValue>) -> IdentityInfo {
    let mut info = IdentityInfo::default();
    info.identity_hash = mp_str(p, "identity_hash");
    info.destination_hash = mp_str(p, "destination_hash");
    info.lxmf_destination_hash = mp_str(p, "lxmf_destination_hash");
    info.display_name = mp_str(p, "display_name");
    info.icon = p.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());
    info.short_name = p.get("short_name").and_then(|v| v.as_str()).map(ToOwned::to_owned);
    info
}

fn parse_status(p: &HashMap<String, MpValue>) -> DaemonStatusInfo {
    let mut s = DaemonStatusInfo::default();
    s.uptime = p.get("uptime").and_then(|v| v.as_u64()).unwrap_or(0);
    s.daemon_version = mp_str(p, "daemon_version");
    s.rns_initialized = p.get("rns_initialized").and_then(|v| v.as_bool()).unwrap_or(false);
    s.transport_enabled = p.get("transport_enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    s.device_count = p.get("device_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.active_links = p.get("active_links").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.interface_count = p.get("interface_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.propagation_enabled = p.get("propagation_enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    s.standard_lxmf_propagation_destination_registered = p
        .get("standard_lxmf_propagation_destination_registered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    s.standard_lxmf_propagation_active =
        p.get("standard_lxmf_propagation_active").and_then(|v| v.as_bool()).unwrap_or(false);
    s.active_capabilities = p.get("active_capabilities").and_then(parse_capabilities);
    s.connection_generation = p.get("connection_generation").and_then(MpValue::as_u64);
    s
}

fn parse_capabilities(value: &MpValue) -> Option<ActiveCapabilitiesInfo> {
    let map = value.as_map()?;
    let item = |key: &str| map.iter().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v);
    let strings = |key: &str| {
        item(key)?
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(ToOwned::to_owned))
            .collect::<Option<Vec<_>>>()
    };
    let degraded = item("degraded")?
        .as_array()?
        .iter()
        .map(|value| {
            let map = value.as_map()?;
            let get = |key: &str| {
                map.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, value)| value.as_str())
                    .map(ToOwned::to_owned)
            };
            let mut degraded = DegradedCapabilityInfo::default();
            degraded.id = get("id")?;
            degraded.reason = get("reason")?;
            Some(degraded)
        })
        .collect::<Option<Vec<_>>>()?;
    let mut capabilities = ActiveCapabilitiesInfo::default();
    capabilities.version = u16::try_from(item("version")?.as_u64()?).ok()?;
    capabilities.runtime = strings("runtime")?;
    capabilities.degraded = degraded;
    capabilities.authorized_operations = strings("authorized_operations")?;
    Some(capabilities)
}

fn parse_devices(p: &HashMap<String, MpValue>) -> Result<Vec<DeviceInfo>, String> {
    let arr = p
        .get("devices")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or("no devices array")?;

    Ok(arr
        .iter()
        .filter_map(|v| {
            let m = v.as_map()?;
            let get = |key: &str| -> String {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let mut dev = DeviceInfo::default();
            dev.destination_hash = get("destination_hash");
            dev.identity_hash = get("identity_hash");
            dev.name = get("name");
            dev.status = get("status");
            dev.device_type = get("device_type");
            dev.is_styrene_node = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("is_styrene_node"))
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false);
            dev.last_announce = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("last_announce"))
                .and_then(|(_, v)| v.as_i64());
            dev.announce_count = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("announce_count"))
                .and_then(|(_, v)| v.as_u64())
                .unwrap_or(0) as u32;
            dev.discovered_capabilities = parse_discovered_capabilities(m);
            dev.standard_lxmf_propagation_active = m
                .iter()
                .find(|(key, _)| key.as_str() == Some("standard_lxmf_propagation_active"))
                .and_then(|(_, value)| value.as_bool());
            Some(dev)
        })
        .collect())
}

fn parse_device_event(p: &HashMap<String, MpValue>) -> Option<DeviceInfo> {
    let mut dev = DeviceInfo::default();
    dev.destination_hash = mp_str(p, "destination_hash");
    dev.identity_hash = mp_str(p, "identity_hash");
    dev.name = mp_str(p, "name");
    dev.status = mp_str(p, "status");
    dev.device_type = mp_str(p, "device_type");
    dev.is_styrene_node = p.get("is_styrene_node").and_then(|v| v.as_bool()).unwrap_or(false);
    dev.discovered_capabilities = p
        .get("discovered_capabilities")
        .and_then(MpValue::as_array)
        .map(|values| parse_discovered_capability_values(values))
        .unwrap_or_default();
    dev.standard_lxmf_propagation_active =
        p.get("standard_lxmf_propagation_active").and_then(|value| value.as_bool());
    if dev.destination_hash.is_empty() {
        return None;
    }
    Some(dev)
}

fn parse_discovered_capabilities(map: &[(MpValue, MpValue)]) -> Vec<DiscoveredCapability> {
    map.iter()
        .find(|(key, _)| key.as_str() == Some("discovered_capabilities"))
        .and_then(|(_, value)| value.as_array())
        .map(|values| parse_discovered_capability_values(values))
        .unwrap_or_default()
}

fn parse_discovered_capability_values(values: &[MpValue]) -> Vec<DiscoveredCapability> {
    values
        .iter()
        .filter_map(|value| match value.as_str() {
            Some("native_nomadnet_host") => Some(DiscoveredCapability::NativeNomadNetHost),
            Some("standard_lxmf_propagation_host") => {
                Some(DiscoveredCapability::StandardLxmfPropagationHost)
            }
            _ => None,
        })
        .collect()
}

fn parse_message_event(p: &HashMap<String, MpValue>) -> Option<MessageInfo> {
    let map: Vec<_> =
        p.iter().map(|(key, value)| (MpValue::from(key.as_str()), value.clone())).collect();
    let message = parse_message_map(&map);
    (!message.id.is_empty()).then_some(message)
}

fn parse_message_map(map: &[(MpValue, MpValue)]) -> MessageInfo {
    let mut message =
        rmpv::ext::from_value::<MessageInfo>(MpValue::Map(map.to_vec())).unwrap_or_default();
    message.attempts.truncate(32);
    message.delivery_evidence.truncate(32);
    message.attachments.truncate(8);
    message.propagation_correlations.truncate(64);
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(msg_type: MessageType, request_id: [u8; REQUEST_ID_SIZE]) -> Frame {
        Frame { msg_type, request_id, payload: HashMap::new() }
    }

    #[test]
    fn response_validation_requires_matching_request_id() {
        let request_id = [7; REQUEST_ID_SIZE];
        assert!(validate_response(frame(MessageType::Result, request_id), request_id).is_ok());
        assert!(validate_response(frame(MessageType::Result, [8; REQUEST_ID_SIZE]), request_id)
            .unwrap_err()
            .to_string()
            .contains("request ID mismatch"));
    }

    #[test]
    fn response_validation_rejects_events_and_errors() {
        let request_id = [9; REQUEST_ID_SIZE];
        assert!(validate_response(
            frame(MessageType::EventDevice, [0; REQUEST_ID_SIZE]),
            request_id
        )
        .is_err());
        let mut error = frame(MessageType::Error, request_id);
        error.payload.insert("error".into(), MpValue::from("denied"));
        assert_eq!(validate_response(error, request_id).unwrap_err().to_string(), "denied");
    }

    #[test]
    fn typed_error_parser_preserves_cursor_stale_and_legacy_errors() {
        let request_id = [3; REQUEST_ID_SIZE];
        let mut typed = frame(MessageType::Error, request_id);
        typed.payload.insert("error".into(), MpValue::from("conflict: cursor_stale"));
        typed.payload.insert("message".into(), MpValue::from("conflict: cursor_stale"));
        typed.payload.insert("kind".into(), MpValue::from("conflict"));
        typed.payload.insert("code".into(), MpValue::from("cursor_stale"));
        let error = validate_response(typed, request_id).unwrap_err();
        assert!(error.cursor_stale());
        assert_eq!(error.to_string(), "conflict: cursor_stale");

        let mut legacy = frame(MessageType::Error, request_id);
        legacy.payload.insert("error".into(), MpValue::from("old daemon error"));
        assert_eq!(
            validate_response(legacy, request_id).unwrap_err(),
            BridgeError::Internal("old daemon error".into())
        );
    }

    #[test]
    fn status_event_uses_authoritative_message_status_not_event_kind() {
        for status in ["sending", "cancelled", "failed: no route"] {
            let mut payload = HashMap::new();
            payload.insert("kind".into(), MpValue::from("status_changed"));
            payload.insert("id".into(), MpValue::from("message"));
            payload.insert("status".into(), MpValue::from(status));
            let event = frame_to_event(Frame {
                msg_type: MessageType::EventMessage,
                request_id: [0; REQUEST_ID_SIZE],
                payload,
            });
            assert!(matches!(
                event,
                Some(DaemonEvent::MessageReceived(message))
                    if message.id == "message" && message.status == status
            ));
        }
    }

    #[tokio::test]
    async fn legacy_bridge_browse_returns_authoritative_typed_page_source() {
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        let server = tokio::spawn(async move {
            let request = wire::read_frame_async(&mut server).await.expect("browse request");
            assert_eq!(request.msg_type, MessageType::QueryPage);

            let mut page = styrene_ipc::types::PageContent::default();
            page.source_bytes = b">Authoritative\ntyped source".to_vec();
            let mut payload = HashMap::new();
            payload.insert("source".into(), MpValue::from("removed flat source"));
            payload.insert(
                "page".into(),
                MpValue::Binary(rmp_serde::to_vec_named(&page).expect("encode page")),
            );
            wire::write_frame_async(
                &mut server,
                MessageType::Result,
                &request.request_id,
                &payload,
            )
            .await
            .expect("browse response");
        });
        let mut bridge = DaemonBridge { stream: client, next_id: 0, usable: true };

        let source = bridge.browse_page("local", "/page/index.mu").await.expect("browse page");

        assert_eq!(source, ">Authoritative\ntyped source");
        server.await.expect("server task");
    }

    #[test]
    fn typed_page_parser_retains_authoritative_bytes_and_all_metadata() {
        let mut page = styrene_ipc::types::PageContent::default();
        page.source_bytes = vec![0xff, 1, 2];
        page.rendered_text = "projection".into();
        page.title = Some("Title".into());
        page.links.push("next.mu".into());
        page.correlation_id = "page-1".into();
        page.source_checksum = "aa".repeat(32);
        page.request.native_path = "/page/index.mu".into();
        page.transfer.verified = true;
        page.cache.status = styrene_ipc::types::PageCacheStatus::NotUsed;
        page.cache.stored_at = Some(7);
        let mut payload = HashMap::new();
        payload.insert(
            "page".into(),
            MpValue::Binary(rmp_serde::to_vec_named(&page).expect("encode page")),
        );

        let decoded = parse_page_content(&payload).expect("decode page");

        assert_eq!(decoded, page);
        assert_eq!(decoded.source_bytes, [0xff, 1, 2]);
    }

    #[test]
    fn path_parser_preserves_observation_metadata_and_defaults_legacy_payloads() {
        let mut payload = HashMap::new();
        payload.insert(
            "paths".into(),
            MpValue::Array(vec![MpValue::Map(vec![
                (MpValue::from("destination_hash"), MpValue::from("peer")),
                (MpValue::from("source"), MpValue::from("transport_path_table")),
                (MpValue::from("observed_at"), MpValue::from(90_i64)),
                (MpValue::from("connection_generation"), MpValue::from(7_u64)),
                (MpValue::from("age_secs"), MpValue::from(10_u64)),
                (MpValue::from("expires"), MpValue::from(700_i64)),
                (MpValue::from("freshness_threshold_secs"), MpValue::from(300_u64)),
                (MpValue::from("stale"), MpValue::Boolean(false)),
                (MpValue::from("correlation_id"), MpValue::from("request-1")),
            ])]),
        );

        let parsed = parse_path_table(&payload);
        assert_eq!(parsed[0].observation.source, ObservationSource::TransportPathTable);
        assert_eq!(parsed[0].observation.connection_generation, Some(7));
        assert_eq!(parsed[0].observation.age_secs, Some(10));
        assert_eq!(parsed[0].expires, Some(700));
        assert_eq!(parsed[0].observation.correlation_id.as_deref(), Some("request-1"));

        payload.insert(
            "paths".into(),
            MpValue::Array(vec![MpValue::Map(vec![(
                MpValue::from("destination_hash"),
                MpValue::from("legacy"),
            )])]),
        );
        assert_eq!(parse_path_table(&payload)[0].observation, ObservationMetadata::default());
    }

    #[test]
    fn route_event_parser_preserves_loss_and_generation_metadata() {
        let mut payload = HashMap::new();
        payload.insert("kind".into(), MpValue::from("lost"));
        payload.insert("destination_hash".into(), MpValue::from("peer"));
        payload.insert("loss_reason".into(), MpValue::from("expired"));
        payload.insert("expires".into(), MpValue::from(700_i64));
        payload.insert("source".into(), MpValue::from("transport_path_table"));
        payload.insert("connection_generation".into(), MpValue::from(9_u64));
        payload.insert("route_age_secs".into(), MpValue::from(601_u64));
        payload.insert("route_connection_generation".into(), MpValue::from(9_u64));
        payload.insert("route_freshness_threshold_secs".into(), MpValue::from(300_u64));
        payload.insert("route_stale".into(), MpValue::Boolean(true));

        let event = frame_to_event(Frame {
            msg_type: MessageType::EventRoute,
            request_id: [0; REQUEST_ID_SIZE],
            payload,
        });
        match event {
            Some(DaemonEvent::RouteLifecycle(event)) => {
                assert_eq!(event.kind, RouteEventKind::Lost);
                assert_eq!(event.loss_reason, Some(RouteLossReason::Expired));
                assert_eq!(event.route.expires, Some(700));
                assert_eq!(event.route.observation.age_secs, Some(601));
                assert_eq!(event.route.observation.connection_generation, Some(9));
                assert_eq!(event.route.observation.freshness_threshold_secs, Some(300));
                assert_eq!(event.observation.connection_generation, Some(9));
            }
            other => panic!("expected route event, got {other:?}"),
        }
    }

    #[test]
    fn message_parser_preserves_lifecycle_fields() {
        let message = MpValue::Map(vec![
            (MpValue::from("id"), MpValue::from("message")),
            (MpValue::from("status"), MpValue::from("delivered")),
            (MpValue::from("delivery_method"), MpValue::from("propagated")),
            (MpValue::from("requested_delivery_method"), MpValue::from("opportunistic")),
            (MpValue::from("actual_delivery_method"), MpValue::from("direct")),
            (MpValue::from("fallback_reason"), MpValue::from("packet limit")),
            (MpValue::from("correlation_id"), MpValue::from("send-1")),
            (
                MpValue::from("attempts"),
                MpValue::Array(vec![MpValue::Map(vec![
                    (MpValue::from("message_id"), MpValue::from("message")),
                    (MpValue::from("number"), MpValue::from(1_u64)),
                    (MpValue::from("started_unix_ms"), MpValue::from(100_i64)),
                    (MpValue::from("deadline_unix_ms"), MpValue::from(200_i64)),
                    (MpValue::from("state"), MpValue::from("delivered")),
                ])]),
            ),
            (MpValue::from("read"), MpValue::Boolean(true)),
            (
                MpValue::from("attachment_info"),
                MpValue::Map(vec![
                    (MpValue::from("name"), MpValue::from("evidence.bin")),
                    (MpValue::from("content_type"), MpValue::from("application/octet-stream")),
                    (MpValue::from("size"), MpValue::from(2048_u64)),
                ]),
            ),
        ]);
        let mut payload = HashMap::new();
        payload.insert("messages".into(), MpValue::Array(vec![message]));
        let parsed = parse_messages(&payload);
        assert_eq!(parsed[0].delivery_method.as_deref(), Some("propagated"));
        assert_eq!(parsed[0].requested_delivery_method.as_deref(), Some("opportunistic"));
        assert_eq!(parsed[0].actual_delivery_method.as_deref(), Some("direct"));
        assert_eq!(parsed[0].fallback_reason.as_deref(), Some("packet limit"));
        assert_eq!(parsed[0].correlation_id.as_deref(), Some("send-1"));
        assert_eq!(parsed[0].attempts.len(), 1);
        assert_eq!(parsed[0].attempts[0].message_id, "message");
        assert_eq!(parsed[0].attempts[0].number, 1);
        assert_eq!(parsed[0].attempts[0].started_unix_ms, 100);
        assert_eq!(parsed[0].attempts[0].deadline_unix_ms, 200);
        assert_eq!(parsed[0].attempts[0].state, "delivered");
        assert!(parsed[0].read);
        assert_eq!(parsed[0].attachment_info.as_ref().map(|item| item.size), Some(2048));
    }

    #[test]
    fn propagation_parser_preserves_safe_queue_metadata() {
        let mut payload = HashMap::new();
        payload.insert("enabled".into(), MpValue::Boolean(true));
        payload.insert("queue_count".into(), MpValue::from(1_u64));
        payload.insert("queue_size_bytes".into(), MpValue::from(512_u64));
        payload.insert("expiry_secs".into(), MpValue::from(3600_u64));
        payload.insert("peer_state_supported".into(), MpValue::Boolean(false));
        payload.insert("sync_state_supported".into(), MpValue::Boolean(false));
        payload.insert("next_cursor".into(), MpValue::from("1700000000:entry"));
        payload.insert(
            "queue".into(),
            MpValue::Array(vec![MpValue::Map(vec![
                (MpValue::from("id"), MpValue::from("entry")),
                (MpValue::from("destination_hash"), MpValue::from("destination")),
                (MpValue::from("size_bytes"), MpValue::from(512_u64)),
                (MpValue::from("state"), MpValue::from("stored")),
            ])]),
        );
        let snapshot = parse_propagation(&payload);
        assert!(snapshot.enabled);
        assert_eq!(snapshot.queue[0].size_bytes, 512);
        assert!(!snapshot.peer_state_supported);
        assert!(!snapshot.sync_state_supported);
        assert_eq!(snapshot.next_cursor.as_deref(), Some("1700000000:entry"));
    }

    #[tokio::test]
    async fn embedded_runtime_owns_and_removes_ephemeral_state() {
        let (broker, events, runtime) = tokio::time::timeout(
            Duration::from_secs(10),
            connect_embedded(true, crate::backend::ConnectionGeneration(1)),
        )
        .await
        .expect("embedded startup timed out")
        .expect("embedded startup failed");
        let root = runtime.root().to_path_buf();

        assert!(root.join("messages.db").exists());
        assert!(root.join("control.sock").exists());
        assert!(broker.path_table().await.is_ok());
        drop(broker);
        drop(events);
        runtime.shutdown().await;
        assert!(!root.exists());
    }

    fn broker_pair(capacity: usize) -> (RequestBroker, UnixStream) {
        let (client, server) = UnixStream::pair().unwrap();
        (
            RequestBroker::spawn_with_capacity(
                client,
                crate::backend::ConnectionGeneration(7),
                capacity,
            ),
            server,
        )
    }

    async fn reply(server: &mut UnixStream, request_id: &[u8; REQUEST_ID_SIZE], value: &str) {
        let mut payload = HashMap::new();
        payload.insert("value".into(), MpValue::from(value));
        wire::write_frame_async(server, MessageType::Result, request_id, &payload).await.unwrap();
    }

    #[tokio::test]
    async fn broker_correlates_concurrent_out_of_order_responses() {
        let (broker, mut server) = broker_pair(4);
        let first = tokio::spawn({
            let broker = broker.clone();
            async move { broker.rpc(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await }
        });
        let second = tokio::spawn({
            let broker = broker.clone();
            async move {
                broker.rpc(MessageType::QueryStatus, HashMap::new(), Duration::from_secs(1)).await
            }
        });
        let request_a = wire::read_frame_async(&mut server).await.unwrap();
        let request_b = wire::read_frame_async(&mut server).await.unwrap();
        assert_ne!(request_a.request_id, request_b.request_id);
        assert_eq!(u64::from_le_bytes(request_a.request_id[..8].try_into().unwrap()), 7);

        let value_a = if request_a.msg_type == MessageType::Ping { "ping" } else { "status" };
        let value_b = if request_b.msg_type == MessageType::Ping { "ping" } else { "status" };
        reply(&mut server, &request_b.request_id, value_b).await;
        reply(&mut server, &request_a.request_id, value_a).await;
        let result_a = first.await.unwrap().unwrap();
        let result_b = second.await.unwrap().unwrap();
        assert_eq!(mp_str(&result_a.payload, "value"), "ping");
        assert_eq!(mp_str(&result_b.payload, "value"), "status");
        assert_eq!(broker.diagnostics().completed, 2);
        assert_eq!(broker.diagnostics().reconnects, 6);
    }

    #[tokio::test]
    async fn old_daemon_without_next_cursor_is_cleanly_unsupported() {
        let (broker, mut server) = broker_pair(2);
        let query = tokio::spawn({
            let broker = broker.clone();
            async move { broker.query_conversation_page(None).await }
        });
        let request = wire::read_frame_async(&mut server).await.unwrap();
        let payload = HashMap::from([("conversations".into(), MpValue::Array(Vec::new()))]);
        wire::write_frame_async(&mut server, MessageType::Result, &request.request_id, &payload)
            .await
            .unwrap();

        let page = query.await.unwrap().unwrap();
        assert!(!page.pagination_supported);
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn typed_stale_message_cursor_restarts_from_first_page() {
        let (broker, mut server) = broker_pair(2);
        let query = tokio::spawn({
            let broker = broker.clone();
            async move { broker.query_message_page("2222222222222222", Some("stale")).await }
        });
        let stale_request = wire::read_frame_async(&mut server).await.unwrap();
        assert_eq!(stale_request.payload["cursor"].as_str(), Some("stale"));
        let stale = HashMap::from([
            ("error".into(), MpValue::from("conflict: cursor_stale")),
            ("message".into(), MpValue::from("conflict: cursor_stale")),
            ("kind".into(), MpValue::from("conflict")),
            ("code".into(), MpValue::from("cursor_stale")),
        ]);
        wire::write_frame_async(&mut server, MessageType::Error, &stale_request.request_id, &stale)
            .await
            .unwrap();

        let restarted = wire::read_frame_async(&mut server).await.unwrap();
        assert!(!restarted.payload.contains_key("cursor"));
        let page_payload = HashMap::from([
            ("messages".into(), MpValue::Array(Vec::new())),
            ("next_cursor".into(), MpValue::Nil),
        ]);
        wire::write_frame_async(
            &mut server,
            MessageType::Result,
            &restarted.request_id,
            &page_payload,
        )
        .await
        .unwrap();

        let page = query.await.unwrap().unwrap();
        assert!(page.pagination_supported);
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn broker_enforces_deadline_and_capacity() {
        let (broker, mut server) = broker_pair(1);
        let pending = tokio::spawn({
            let broker = broker.clone();
            async move { broker.rpc(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await }
        });
        let _request = wire::read_frame_async(&mut server).await.unwrap();
        let overloaded = broker
            .rpc(MessageType::QueryStatus, HashMap::new(), Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(overloaded.contains("overloaded"));
        pending.abort();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let timed_out = broker
            .rpc(MessageType::QueryStatus, HashMap::new(), Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(timed_out.contains("timed out"));
        let diagnostics = broker.diagnostics();
        assert_eq!(diagnostics.overloaded, 1);
        assert_eq!(diagnostics.cancelled, 1);
        assert_eq!(diagnostics.timed_out, 1);
    }

    #[tokio::test]
    async fn broker_reports_disconnect_to_in_flight_request() {
        let (broker, mut server) = broker_pair(2);
        let request = tokio::spawn({
            let broker = broker.clone();
            async move { broker.rpc(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await }
        });
        let _request = wire::read_frame_async(&mut server).await.unwrap();
        drop(server);
        assert!(request.await.unwrap().unwrap_err().contains("read failed"));
        assert_eq!(broker.diagnostics().disconnected, 1);
    }

    #[tokio::test]
    async fn broker_rejects_stale_generation_before_matching_response() {
        let (broker, mut server) = broker_pair(2);
        let request = tokio::spawn({
            let broker = broker.clone();
            async move { broker.rpc(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await }
        });
        let frame = wire::read_frame_async(&mut server).await.unwrap();
        let mut stale_id = frame.request_id;
        stale_id[..8].copy_from_slice(&8_u64.to_le_bytes());
        reply(&mut server, &stale_id, "stale").await;
        reply(&mut server, &frame.request_id, "current").await;

        assert_eq!(mp_str(&request.await.unwrap().unwrap().payload, "value"), "current");
        assert_eq!(broker.diagnostics().stale_responses, 1);
    }

    #[tokio::test]
    async fn event_reader_fans_out_events_without_broker_requests() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let (tx, mut events) = mpsc::channel(2);
        tokio::spawn(event_reader(client, tx, Arc::new(BrokerMetrics::default()), 7));
        let mut payload = HashMap::new();
        payload.insert("destination_hash".into(), MpValue::from("peer"));
        payload.insert("name".into(), MpValue::from("Peer"));
        wire::write_frame_async(
            &mut server,
            MessageType::EventDevice,
            &[0; REQUEST_ID_SIZE],
            &payload,
        )
        .await
        .unwrap();

        assert!(matches!(events.recv().await, Some(DaemonEvent::PeerDiscovered(_))));
    }

    #[test]
    fn full_event_channel_records_dropped_update() {
        let (tx, _events) = mpsc::channel(1);
        tx.try_send(DaemonEvent::Connected).unwrap();
        let metrics = BrokerMetrics::default();
        send_polled_event(&tx, DaemonEvent::Connected, &metrics);
        assert_eq!(metrics.dropped_updates.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn every_backend_command_has_an_explicit_execution_capability_policy() {
        let commands = vec![
            DaemonCommand::SendChat {
                peer_hash: String::new(),
                content: String::new(),
                title: None,
                delivery_method: "direct".into(),
                attachments: Vec::new(),
            },
            DaemonCommand::LoadDraft { peer_hash: String::new() },
            DaemonCommand::SaveDraft { peer_hash: String::new(), content: String::new() },
            DaemonCommand::DiscardDraft { peer_hash: String::new() },
            DaemonCommand::RetryMessage { message_id: String::new() },
            DaemonCommand::CancelMessage { message_id: String::new() },
            DaemonCommand::BrowsePage { address: crate::state::PageAddress::local_index() },
            DaemonCommand::NavigatePage(Default::default()),
            DaemonCommand::ClosePage { session_id: String::new() },
            DaemonCommand::StartFileDownload(Default::default()),
            DaemonCommand::QueryFileDownload { download_id: String::new() },
            DaemonCommand::CancelFileDownload { download_id: String::new() },
            DaemonCommand::SaveFileDownload {
                download_id: String::new(),
                destination: String::new(),
            },
            DaemonCommand::RefreshPathTable,
            DaemonCommand::RefreshInterfaces,
            DaemonCommand::RefreshLinks,
            DaemonCommand::RefreshOperations,
            DaemonCommand::RefreshRequests,
            DaemonCommand::RefreshResources,
            DaemonCommand::StartNetworkOperation(Default::default()),
            DaemonCommand::CancelNetworkOperation { operation_id: String::new() },
            DaemonCommand::StartRequest(Default::default()),
            DaemonCommand::CancelRequest { request_id: String::new() },
            DaemonCommand::CancelResource { resource_hash: String::new() },
            DaemonCommand::RefreshPropagation { cursor: None },
            DaemonCommand::LoadConversations { cursor: None },
            DaemonCommand::LoadMessages { peer_hash: String::new(), cursor: None },
            DaemonCommand::QueryMessage { message_id: String::new() },
            DaemonCommand::FleetStatus { destination: String::new() },
            DaemonCommand::FleetExec {
                destination: String::new(),
                command: String::new(),
                args: Vec::new(),
            },
            DaemonCommand::FleetReboot { destination: String::new(), delay: None },
            DaemonCommand::FleetApply { destination: String::new(), profile_base64: String::new() },
            DaemonCommand::BlockPeer { identity_hash: String::new() },
        ];

        assert_eq!(commands.len(), 33);
        assert!(commands.iter().enumerate().all(|(index, command)| {
            command.required_capability().is_some() || matches!(index, 19 | 20)
        }));
        assert_eq!(commands[6].required_capability(), Some("page.browse"));
        assert_eq!(
            commands
                .iter()
                .find(|command| matches!(command, DaemonCommand::FleetReboot { .. }))
                .and_then(DaemonCommand::required_capability),
            Some("rpc.reboot")
        );
    }

    #[test]
    fn unix_interface_map_preserves_every_typed_detail_and_observation() {
        let interface = MpValue::Map(vec![
            (MpValue::from("name"), MpValue::from("uplink")),
            (MpValue::from("hash"), MpValue::from("interface-hash")),
            (MpValue::from("type"), MpValue::from("tcp_client")),
            (MpValue::from("mode"), MpValue::from("point_to_point")),
            (MpValue::from("enabled"), MpValue::from(true)),
            (MpValue::from("status"), MpValue::from("active")),
            (MpValue::from("host"), MpValue::from("mesh.example")),
            (MpValue::from("port"), MpValue::from(4242_u64)),
            (MpValue::from("local_endpoint"), MpValue::from("127.0.0.1:5000")),
            (MpValue::from("remote_endpoint"), MpValue::from("192.0.2.1:4242")),
            (MpValue::from("parent_hash"), MpValue::from("parent-hash")),
            (MpValue::from("tx_bytes"), MpValue::from(12_u64)),
            (MpValue::from("rx_bytes"), MpValue::from(34_u64)),
            (MpValue::from("connected_peers"), MpValue::from(2_u64)),
            (MpValue::from("source"), MpValue::from("runtime_interface_registry")),
            (MpValue::from("observed_at"), MpValue::from(100_i64)),
            (MpValue::from("age_secs"), MpValue::from(3_u64)),
            (MpValue::from("freshness_threshold_secs"), MpValue::from(30_u64)),
            (MpValue::from("stale"), MpValue::from(false)),
            (MpValue::from("connection_generation"), MpValue::from(7_u64)),
            (MpValue::from("correlation_id"), MpValue::from("interface-correlation")),
        ]);
        let payload = HashMap::from([("interfaces".into(), MpValue::Array(vec![interface]))]);

        let parsed = parse_interface_stats(&payload);
        let interface = &parsed[0];
        assert_eq!(interface.kind, "tcp_client");
        assert_eq!(interface.mode, "point_to_point");
        assert!(interface.enabled);
        assert_eq!(interface.host.as_deref(), Some("mesh.example"));
        assert_eq!(interface.port, Some(4242));
        assert_eq!(interface.local_endpoint.as_deref(), Some("127.0.0.1:5000"));
        assert_eq!(interface.remote_endpoint.as_deref(), Some("192.0.2.1:4242"));
        assert_eq!(interface.parent_hash.as_deref(), Some("parent-hash"));
        assert_eq!(interface.peers_connected, 2);
        assert_eq!(interface.observation.source, ObservationSource::RuntimeInterfaceRegistry);
        assert_eq!(interface.observation.observed_at, Some(100));
        assert_eq!(interface.observation.age_secs, Some(3));
        assert_eq!(interface.observation.freshness_threshold_secs, Some(30));
        assert_eq!(interface.observation.connection_generation, Some(7));
        assert_eq!(interface.observation.correlation_id.as_deref(), Some("interface-correlation"));
    }

    #[test]
    fn identity_parser_preserves_every_public_identity_field() {
        let payload = HashMap::from([
            ("identity_hash".into(), MpValue::from("identity")),
            ("destination_hash".into(), MpValue::from("delivery")),
            ("lxmf_destination_hash".into(), MpValue::from("lxmf")),
            ("display_name".into(), MpValue::from("Node Seven")),
            ("icon".into(), MpValue::from("node")),
            ("short_name".into(), MpValue::from("seven")),
        ]);

        let identity = parse_identity(&payload);

        assert_eq!(identity.identity_hash, "identity");
        assert_eq!(identity.destination_hash, "delivery");
        assert_eq!(identity.lxmf_destination_hash, "lxmf");
        assert_eq!(identity.display_name, "Node Seven");
        assert_eq!(identity.icon.as_deref(), Some("node"));
        assert_eq!(identity.short_name.as_deref(), Some("seven"));
    }
}
