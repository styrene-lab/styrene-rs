//! Bounded concurrent client transport for Styrene local IPC.
//!
//! This crate owns request correlation and transport failure semantics. Typed
//! daemon operations are added here as they migrate out of frontend crates.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rmpv::Value;
use serde::de::DeserializeOwned;
use styrene_ipc::IpcError;
use styrene_ipc::types::{
    ACTIVE_CAPABILITIES_VERSION, ActiveCapabilitiesInfo, ConfigApplyResult, ConfigSnapshot,
    ConversationDraft, ConversationInfo, ConversationPage, DaemonStatusInfo, DeviceInfo,
    ExecResult, FileDownloadInfo, FileDownloadRequest, IdentityBackupExport, IdentityBackupImport,
    IdentityBackupMetadata, IdentityInfo, IdentityRestoreOutcome, InterfaceDetail, LinkSnapshot,
    MessageInfo, MessagePage, MessagingDisposition, MessagingOperationOutcome,
    NetworkOperationInfo, ObservationMetadata, ObservationSource, PageContent, PageInfo,
    PageNavigationRequest, PathInfo, PropagationQuery, PropagationSnapshot, RebootResult,
    RemoteStatusInfo, RequestObservationInfo, ResourceTransferInfo, RouteEventInfo, RouteEventKind,
    RouteLossReason, SendChatOutcome, SendChatRequest, StandardPropagationSnapshot,
    StartNetworkOperationInfo, StartRequestInfo, TunnelInfo, TunnelOperationInfo,
};
use styrene_ipc_wire::{self as wire, Frame, MessageType, REQUEST_ID_SIZE};
use thiserror::Error;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore, broadcast, mpsc, oneshot};
use tokio::time::timeout;

pub use styrene_ipc_wire::default_socket_path;

pub const DEFAULT_CAPACITY: usize = 32;
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(5);
pub const SEND_DEADLINE: Duration = Duration::from_secs(35);

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TunnelStatus {
    Tunnel(TunnelInfo),
    Operation(TunnelOperationInfo),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConnectionGeneration(pub u64);

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClientError {
    #[error("IPC client overloaded")]
    Overloaded,
    #[error("IPC request timed out after {deadline:?}")]
    Timeout { deadline: Duration },
    #[error("IPC request was cancelled")]
    Cancelled,
    #[error("IPC client disconnected: {message}")]
    Disconnected { message: String },
    #[error("IPC protocol error: {message}")]
    Protocol { message: String },
    #[error("daemon incompatible: {message}")]
    Incompatible { message: String },
    #[error("daemon request failed: {0}")]
    Remote(#[source] IpcError),
    #[error("daemon request failed ({kind}/{code}): {message}")]
    LegacyRemote { kind: String, code: String, message: String },
}

impl ClientError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Overloaded
            | Self::Timeout { .. }
            | Self::Cancelled
            | Self::Disconnected { .. } => true,
            Self::Remote(error) => error.is_retryable(),
            Self::LegacyRemote { kind, .. } => {
                matches!(kind.as_str(), "unavailable" | "timeout" | "transport")
            }
            Self::Protocol { .. } | Self::Incompatible { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientDiagnostics {
    pub queue_depth: usize,
    pub in_flight: usize,
    pub completed: u64,
    pub timed_out: u64,
    pub cancelled: u64,
    pub overloaded: u64,
    pub disconnected: u64,
    pub stale_responses: u64,
    pub dropped_responses: u64,
    pub last_latency_ms: u64,
    /// Event frames the daemon pushed on this connection.
    pub events_received: u64,
    /// Event frames delivered to no subscriber (nobody was listening).
    pub events_unobserved: u64,
}

/// A daemon subscription topic. Subscriptions are per connection: the daemon
/// pushes the topic's event frames on the connection that subscribed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventTopic {
    Devices,
    Messages,
    Activity,
    Links,
    Routes,
    Requests,
    NetworkOperations,
    Resources,
}

impl EventTopic {
    /// Every topic, in wire order.
    pub const ALL: [Self; 8] = [
        Self::Devices,
        Self::Messages,
        Self::Activity,
        Self::Links,
        Self::Routes,
        Self::Requests,
        Self::NetworkOperations,
        Self::Resources,
    ];

    #[must_use]
    pub const fn message_type(self) -> MessageType {
        match self {
            Self::Devices => MessageType::SubDevices,
            Self::Messages => MessageType::SubMessages,
            Self::Activity => MessageType::SubActivity,
            Self::Links => MessageType::SubLinks,
            Self::Routes => MessageType::SubRoutes,
            Self::Requests => MessageType::SubRequests,
            Self::NetworkOperations => MessageType::SubNetworkOperations,
            Self::Resources => MessageType::SubResources,
        }
    }
}

/// An event frame the daemon pushed on this connection. Events carry the
/// connection generation they arrived on so a consumer that keeps a receiver
/// across reconnects can discard frames from an earlier connection.
#[derive(Clone, Debug)]
pub struct EventFrame {
    pub message_type: MessageType,
    pub payload: HashMap<String, Value>,
    pub generation: ConnectionGeneration,
}

impl EventFrame {
    /// Decode the whole payload as a typed record.
    pub fn typed<T: DeserializeOwned>(&self) -> Result<T, ClientError> {
        decode_map(&self.payload, "event payload")
    }

    /// Decode one typed record stored under `key`.
    pub fn typed_key<T: DeserializeOwned>(&self, key: &str) -> Result<T, ClientError> {
        decode_typed_key(&self.payload, key, "event payload")
    }

    /// A string field, or the empty string when absent.
    #[must_use]
    pub fn text(&self, key: &str) -> String {
        text(&self.payload, key, "")
    }

    /// Decode a route lifecycle event. The daemon spells it flat: the route's
    /// own fields sit beside the event's, with the route observation under
    /// `route_*` keys, so the nested canonical record is rebuilt here.
    pub fn route_event(&self) -> Result<RouteEventInfo, ClientError> {
        if self.message_type != MessageType::EventRoute {
            return Err(ClientError::Protocol {
                message: format!("{:?} is not a route event", self.message_type),
            });
        }
        let payload = &self.payload;
        let destination_hash = text(payload, "destination_hash", "");
        if destination_hash.is_empty() {
            return Err(ClientError::Protocol {
                message: "route event omitted destination_hash".into(),
            });
        }
        let string = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
        let mut route = PathInfo::default();
        route.destination_hash = destination_hash;
        route.hops =
            payload.get("hops").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
        route.next_hop = string("next_hop");
        route.interface = string("interface");
        route.expires = payload.get("expires").and_then(Value::as_i64);
        route.observation = ObservationMetadata::default();
        route.observation.source = ObservationSource::TransportPathTable;
        route.observation.observed_at = payload.get("route_observed_at").and_then(Value::as_i64);
        route.observation.connection_generation =
            payload.get("route_connection_generation").and_then(Value::as_u64);
        route.observation.age_secs = payload.get("route_age_secs").and_then(Value::as_u64);
        route.observation.freshness_threshold_secs =
            payload.get("route_freshness_threshold_secs").and_then(Value::as_u64);
        route.observation.stale =
            payload.get("route_stale").and_then(Value::as_bool).unwrap_or(false);

        let mut event = RouteEventInfo::default();
        event.kind = match payload.get("kind").and_then(Value::as_str) {
            Some("discovered") => RouteEventKind::Discovered,
            Some("lost") => RouteEventKind::Lost,
            Some("rediscovered") => RouteEventKind::Rediscovered,
            _ => RouteEventKind::Unknown,
        };
        event.loss_reason =
            payload.get("loss_reason").and_then(Value::as_str).map(|reason| match reason {
                "expired" => RouteLossReason::Expired,
                "interface_unavailable" => RouteLossReason::InterfaceUnavailable,
                _ => RouteLossReason::Unknown,
            });
        event.route = route;
        event.observation = decode_map(payload, "route observation")?;
        Ok(event)
    }
}

/// Default number of event frames buffered per connection before a slow
/// subscriber starts losing the oldest ones.
pub const DEFAULT_EVENT_CAPACITY: usize = 256;

struct Request {
    request_id: [u8; REQUEST_ID_SIZE],
    message_type: MessageType,
    payload: HashMap<String, Value>,
    started: Instant,
    response: oneshot::Sender<Result<Frame, ClientError>>,
}

struct PendingRequest {
    started: Instant,
    response: oneshot::Sender<Result<Frame, ClientError>>,
}

#[derive(Default)]
struct Metrics {
    queue_depth: AtomicUsize,
    in_flight: AtomicUsize,
    completed: AtomicU64,
    timed_out: AtomicU64,
    cancelled: AtomicU64,
    overloaded: AtomicU64,
    disconnected: AtomicU64,
    stale_responses: AtomicU64,
    dropped_responses: AtomicU64,
    last_latency_ms: AtomicU64,
    events_received: AtomicU64,
    events_unobserved: AtomicU64,
}

#[derive(Clone)]
pub struct Client {
    generation: ConnectionGeneration,
    /// A never-read receiver kept only so `events()` can `resubscribe`. The
    /// reader task owns the sole sender, so every receiver ends when the
    /// connection does.
    events: Arc<broadcast::Receiver<EventFrame>>,
    outbound: mpsc::Sender<Request>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    capacity: Arc<Semaphore>,
    next_id: Arc<AtomicU64>,
    metrics: Arc<Metrics>,
    connected: Arc<AtomicBool>,
    /// Daemon-side connection generation recorded by negotiation and updated
    /// by compatibility polling; zero until negotiated.
    daemon_generation: Arc<AtomicU64>,
}

/// One page of a cursor-paginated query.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Paged<T> {
    pub page: T,
    /// The daemon rejected the requested cursor as stale and the page was
    /// fetched again from the start.
    pub reset: bool,
    /// The daemon spelled a `next_cursor` field, so it paginates; an older
    /// daemon that omits it answers with everything it has.
    pub pagination_supported: bool,
}

/// Outcome of connection negotiation: the daemon's status snapshot, the
/// connection generation it assigned to this transport, and the capability
/// set the frontend must honor for the life of the connection.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Negotiation {
    pub status: DaemonStatusInfo,
    pub daemon_generation: u64,
    pub capabilities: Option<ActiveCapabilitiesInfo>,
}

impl Negotiation {
    /// Validate a status snapshot as a negotiation result. The daemon must
    /// report a non-zero connection generation, and any capability contract it
    /// advertises must match the version this client was built against.
    pub fn from_status(status: DaemonStatusInfo) -> Result<Self, ClientError> {
        let daemon_generation = status
            .connection_generation
            .filter(|generation| *generation != 0)
            .ok_or_else(|| ClientError::Protocol {
                message: "daemon status omitted a connection generation".into(),
            })?;
        let capabilities = status.active_capabilities.clone();
        if let Some(capabilities) = &capabilities
            && capabilities.version != ACTIVE_CAPABILITIES_VERSION
        {
            return Err(ClientError::Incompatible {
                message: format!(
                    "daemon capability contract version {} differs from client version {}",
                    capabilities.version, ACTIVE_CAPABILITIES_VERSION
                ),
            });
        }
        Ok(Self { status, daemon_generation, capabilities })
    }
}

/// A compatibility observation from periodic status polling.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompatibilityEvent {
    /// The daemon reports a different connection generation than the one last
    /// observed: it restarted or the transport was re-established.
    GenerationChanged { previous: u64, current: u64 },
    /// The daemon's active capability set differs from the one last observed.
    CapabilitiesChanged {
        previous: Option<ActiveCapabilitiesInfo>,
        current: Option<ActiveCapabilitiesInfo>,
    },
    /// A fresh status snapshot from a successful poll, sent after any change
    /// events that poll produced.
    Status(DaemonStatusInfo),
    /// A poll failed. Polling stops; the connection is no longer trustworthy.
    Lost { error: String },
}

/// Whether two capability snapshots differ in anything but their generation
/// counter, which the daemon bumps with every connection generation.
fn capability_set_changed(
    previous: Option<&ActiveCapabilitiesInfo>,
    current: Option<&ActiveCapabilitiesInfo>,
) -> bool {
    let masked = |capabilities: Option<&ActiveCapabilitiesInfo>| {
        capabilities.cloned().map(|mut capabilities| {
            capabilities.generation = None;
            capabilities
        })
    };
    masked(previous) != masked(current)
}

/// A running compatibility poll. Dropping the watch stops polling.
#[derive(Debug)]
pub struct CompatibilityWatch {
    events: mpsc::Receiver<CompatibilityEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl CompatibilityWatch {
    /// Next compatibility event, or `None` once polling has stopped.
    pub async fn recv(&mut self) -> Option<CompatibilityEvent> {
        self.events.recv().await
    }
}

impl Drop for CompatibilityWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Client {
    /// Connect to the daemon's Unix socket and negotiate the connection.
    pub async fn connect_unix(
        path: &Path,
        generation: ConnectionGeneration,
        deadline: Duration,
    ) -> Result<(Self, Negotiation), ClientError> {
        let stream = UnixStream::connect(path).await.map_err(|error| {
            ClientError::Disconnected { message: format!("connect {}: {error}", path.display()) }
        })?;
        let client = Self::from_unix_stream(stream, generation);
        let negotiation = client.negotiate(deadline).await?;
        Ok((client, negotiation))
    }

    /// Negotiate the connection: confirm the daemon answers, fetch its status,
    /// and validate the generation and capability contract. Records the
    /// daemon generation for [`Client::daemon_generation`].
    pub async fn negotiate(&self, deadline: Duration) -> Result<Negotiation, ClientError> {
        let frame = self.request(MessageType::Ping, HashMap::new(), deadline).await?;
        if frame.msg_type != MessageType::Pong {
            return Err(ClientError::Protocol {
                message: format!("ping returned {:?} instead of Pong", frame.msg_type),
            });
        }
        let frame = self.request(MessageType::QueryStatus, HashMap::new(), deadline).await?;
        let negotiation = Negotiation::from_status(decode_payload(&frame.payload, "status")?)?;
        self.daemon_generation.store(negotiation.daemon_generation, Ordering::Release);
        Ok(negotiation)
    }

    /// The daemon connection generation observed by negotiation or polling.
    #[must_use]
    pub fn daemon_generation(&self) -> Option<u64> {
        match self.daemon_generation.load(Ordering::Acquire) {
            0 => None,
            generation => Some(generation),
        }
    }

    /// Poll daemon status every `interval`, emitting changes relative to
    /// `baseline` and then the fresh snapshot. The first poll happens after
    /// one interval. Polling stops at the first failure or when the watch is
    /// dropped.
    #[must_use]
    pub fn watch_compatibility(
        &self,
        baseline: &Negotiation,
        interval: Duration,
        deadline: Duration,
    ) -> CompatibilityWatch {
        let (tx, events) = mpsc::channel(16);
        let client = self.clone();
        let mut generation = baseline.daemon_generation;
        let mut capabilities = baseline.capabilities.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let polled = match client
                    .request(MessageType::QueryStatus, HashMap::new(), deadline)
                    .await
                {
                    Ok(frame) => decode_payload::<DaemonStatusInfo>(&frame.payload, "status"),
                    Err(error) => Err(error),
                };
                let status = match polled {
                    Ok(status) => status,
                    Err(error) => {
                        let _ =
                            tx.send(CompatibilityEvent::Lost { error: error.to_string() }).await;
                        return;
                    }
                };
                if let Some(current) = status.connection_generation.filter(|g| *g != 0)
                    && current != generation
                {
                    let event =
                        CompatibilityEvent::GenerationChanged { previous: generation, current };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                    generation = current;
                    client.daemon_generation.store(current, Ordering::Release);
                }
                if capability_set_changed(
                    capabilities.as_ref(),
                    status.active_capabilities.as_ref(),
                ) {
                    let event = CompatibilityEvent::CapabilitiesChanged {
                        previous: capabilities.take(),
                        current: status.active_capabilities.clone(),
                    };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
                capabilities = status.active_capabilities.clone();
                if tx.send(CompatibilityEvent::Status(status)).await.is_err() {
                    return;
                }
            }
        });
        CompatibilityWatch { events, task }
    }
}

impl Client {
    #[must_use]
    pub fn from_unix_stream(stream: UnixStream, generation: ConnectionGeneration) -> Self {
        Self::from_unix_stream_with_capacity(stream, generation, DEFAULT_CAPACITY)
    }

    #[must_use]
    pub fn from_unix_stream_with_capacity(
        stream: UnixStream,
        generation: ConnectionGeneration,
        capacity: usize,
    ) -> Self {
        Self::from_unix_stream_with_capacities(stream, generation, capacity, DEFAULT_EVENT_CAPACITY)
    }

    /// Build a client with explicit request and event buffer bounds.
    pub fn from_unix_stream_with_capacities(
        stream: UnixStream,
        generation: ConnectionGeneration,
        capacity: usize,
        event_capacity: usize,
    ) -> Self {
        let capacity = capacity.max(1);
        let (outbound, requests) = mpsc::channel(capacity);
        let (events, event_prototype) = broadcast::channel(event_capacity.max(1));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let metrics = Arc::new(Metrics::default());
        let connected = Arc::new(AtomicBool::new(true));
        let (reader, writer) = stream.into_split();
        tokio::spawn(writer_task(
            writer,
            requests,
            pending.clone(),
            metrics.clone(),
            connected.clone(),
        ));
        tokio::spawn(reader_task(
            reader,
            generation,
            pending.clone(),
            metrics.clone(),
            connected.clone(),
            events,
        ));
        Self {
            generation,
            events: Arc::new(event_prototype),
            outbound,
            pending,
            capacity: Arc::new(Semaphore::new(capacity)),
            next_id: Arc::new(AtomicU64::new(0)),
            metrics,
            connected,
            daemon_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    #[must_use]
    pub fn generation(&self) -> ConnectionGeneration {
        self.generation
    }

    /// Receive every event frame the daemon pushes on this connection from now
    /// on. Each receiver sees each frame once; a receiver that falls more than
    /// the event capacity behind observes a `Lagged` error and skips the
    /// oldest frames rather than stalling the reader. The receiver ends when
    /// the connection closes.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<EventFrame> {
        self.events.resubscribe()
    }

    /// Ask the daemon to push a topic's events on this connection. Subscribe
    /// before the events of interest can occur, and take an `events()`
    /// receiver first so nothing is missed.
    pub async fn subscribe(&self, topic: EventTopic) -> Result<(), ClientError> {
        self.request(topic.message_type(), HashMap::new(), DEFAULT_DEADLINE).await.map(|_| ())
    }

    /// Subscribe to several topics in order, stopping at the first failure.
    pub async fn subscribe_all(&self, topics: &[EventTopic]) -> Result<(), ClientError> {
        for topic in topics {
            self.subscribe(*topic).await?;
        }
        Ok(())
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub async fn request(
        &self,
        message_type: MessageType,
        payload: HashMap<String, Value>,
        deadline: Duration,
    ) -> Result<Frame, ClientError> {
        if !message_type.is_request() {
            return Err(ClientError::Protocol {
                message: format!("{:?} is not a request message", message_type),
            });
        }
        if !self.is_connected() {
            return Err(ClientError::Disconnected { message: "connection is closed".into() });
        }
        let _permit = self.capacity.clone().try_acquire_owned().map_err(|_| {
            self.metrics.overloaded.fetch_add(1, Ordering::Relaxed);
            ClientError::Overloaded
        })?;
        let request_id = self.next_request_id();
        let (response, receiver) = oneshot::channel();
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = self.outbound.try_send(Request {
            request_id,
            message_type,
            payload,
            started: Instant::now(),
            response,
        }) {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            return match error {
                mpsc::error::TrySendError::Full(_) => {
                    self.metrics.overloaded.fetch_add(1, Ordering::Relaxed);
                    Err(ClientError::Overloaded)
                }
                mpsc::error::TrySendError::Closed(_) => {
                    self.connected.store(false, Ordering::Release);
                    Err(ClientError::Disconnected { message: "request channel closed".into() })
                }
            };
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
                Err(ClientError::Disconnected { message: "response channel closed".into() })
            }
            Err(_) => {
                self.metrics.timed_out.fetch_add(1, Ordering::Relaxed);
                guard.disarm();
                if self.pending.lock().await.remove(&request_id).is_some() {
                    self.metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
                }
                Err(ClientError::Timeout { deadline })
            }
        }
    }

    pub async fn ping(&self) -> Result<(), ClientError> {
        let frame = self.request(MessageType::Ping, HashMap::new(), DEFAULT_DEADLINE).await?;
        if frame.msg_type != MessageType::Pong {
            return Err(ClientError::Protocol {
                message: format!("ping returned {:?} instead of Pong", frame.msg_type),
            });
        }
        Ok(())
    }

    pub async fn identity(&self) -> Result<IdentityInfo, ClientError> {
        let frame =
            self.request(MessageType::QueryIdentity, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "identity")
    }

    pub async fn identity_backup_metadata(&self) -> Result<IdentityBackupMetadata, ClientError> {
        let frame = self
            .request(MessageType::QueryIdentityBackupMetadata, HashMap::new(), DEFAULT_DEADLINE)
            .await?;
        decode_map(&frame.payload, "identity backup metadata")
    }

    pub async fn export_identity_backup(&self) -> Result<IdentityBackupExport, ClientError> {
        let frame = self
            .request(MessageType::CmdExportIdentityBackup, HashMap::new(), DEFAULT_DEADLINE)
            .await?;
        decode_map(&frame.payload, "identity backup export")
    }

    pub async fn restore_identity_backup(
        &self,
        backup: &IdentityBackupImport,
    ) -> Result<IdentityRestoreOutcome, ClientError> {
        let payload = HashMap::from([(
            "encrypted_bytes".into(),
            Value::Binary(backup.encrypted_bytes.clone()),
        )]);
        let frame =
            self.request(MessageType::CmdRestoreIdentityBackup, payload, DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["outcome"], "identity restore outcome")
    }

    pub async fn status(&self) -> Result<DaemonStatusInfo, ClientError> {
        let frame =
            self.request(MessageType::QueryStatus, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "status")
    }

    /// One page of the propagation queue snapshot. A stale cursor is a typed
    /// `Conflict` remote error, left to the caller to restart from the top.
    pub async fn propagation_snapshot(
        &self,
        query: &PropagationQuery,
    ) -> Result<PropagationSnapshot, ClientError> {
        let mut payload = HashMap::from([("limit".into(), Value::from(query.limit))]);
        if let Some(cursor) = &query.cursor {
            payload.insert("cursor".into(), Value::from(cursor.as_str()));
        }
        let frame = self.request(MessageType::QueryPropagation, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "propagation snapshot")
    }

    /// Typed page inventory for a host (`"local"` for this node).
    pub async fn page_inventory(
        &self,
        host: &str,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<PageInfo>, ClientError> {
        let mut payload = HashMap::from([("host".into(), Value::from(host))]);
        if let Some(timeout_secs) = timeout_secs {
            payload.insert("timeout".into(), Value::from(timeout_secs));
        }
        let deadline = Duration::from_secs(timeout_secs.unwrap_or(5).saturating_add(5));
        let frame = self.request(MessageType::CmdPageListSites, payload, deadline).await?;
        decode_key(&frame.payload, &["pages"], "page inventory")
    }

    pub async fn standard_propagation(&self) -> Result<StandardPropagationSnapshot, ClientError> {
        let frame = self
            .request(MessageType::QueryStandardPropagation, HashMap::new(), DEFAULT_DEADLINE)
            .await?;
        decode_map(&frame.payload, "standard propagation snapshot")
    }

    pub async fn devices(&self, styrene_only: bool) -> Result<Vec<DeviceInfo>, ClientError> {
        let payload = HashMap::from([("styrene_only".into(), Value::from(styrene_only))]);
        let frame = self.request(MessageType::QueryDevices, payload, DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["devices", "result"], "devices")
    }

    pub async fn conversations(&self) -> Result<Vec<ConversationInfo>, ClientError> {
        let frame =
            self.request(MessageType::QueryConversations, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["conversations", "result"], "conversations")
    }

    pub async fn messages(
        &self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, ClientError> {
        let payload = HashMap::from([
            ("peer_hash".into(), Value::from(peer_hash)),
            ("limit".into(), Value::from(limit)),
        ]);
        let frame = self.request(MessageType::QueryMessages, payload, DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["messages", "result"], "messages")
    }

    /// Browse a NomadNet page through the daemon coordinator.
    pub async fn browse_page(
        &self,
        host: &str,
        path: &str,
        timeout_secs: Option<u64>,
    ) -> Result<PageContent, ClientError> {
        let mut payload =
            HashMap::from([("host".into(), Value::from(host)), ("path".into(), Value::from(path))]);
        if let Some(timeout_secs) = timeout_secs {
            payload.insert("timeout".into(), Value::from(timeout_secs));
        }
        let deadline = Duration::from_secs(timeout_secs.unwrap_or(30).saturating_add(15));
        let frame = self.request(MessageType::QueryPage, payload, deadline).await?;
        decode_typed_key(&frame.payload, "page", "page")
    }

    /// Navigate a daemon-owned page session, optionally submitting form values.
    pub async fn navigate_page(
        &self,
        request: &PageNavigationRequest,
    ) -> Result<PageContent, ClientError> {
        let payload = HashMap::from([("navigation".into(), encode_typed(request, "navigation")?)]);
        let deadline = Duration::from_secs(request.timeout_secs.unwrap_or(30).saturating_add(15));
        let frame = self.request(MessageType::CmdPageNavigate, payload, deadline).await?;
        decode_typed_key(&frame.payload, "page", "page")
    }

    /// Start a NomadNet file download owned by the daemon.
    pub async fn start_file_download(
        &self,
        request: &FileDownloadRequest,
    ) -> Result<FileDownloadInfo, ClientError> {
        let payload = HashMap::from([(
            "download_request".into(),
            encode_typed(request, "download_request")?,
        )]);
        let frame =
            self.request(MessageType::CmdFileDownloadStart, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "download", "download")
    }

    /// Observe a daemon-owned file download.
    pub async fn file_download(&self, download_id: &str) -> Result<FileDownloadInfo, ClientError> {
        let payload = HashMap::from([("download_id".into(), Value::from(download_id))]);
        let frame = self.request(MessageType::QueryFileDownload, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "download", "download")
    }

    pub async fn send_chat_outcome(
        &self,
        request: &SendChatRequest,
    ) -> Result<SendChatOutcome, ClientError> {
        if request.reply_to_hash.is_some() {
            return Err(ClientError::Protocol {
                message: "reply_to_hash has no local IPC wire field".into(),
            });
        }
        if request.attachment.is_some() && !request.attachments.is_empty() {
            return Err(ClientError::Protocol {
                message: "legacy attachment and attachments are mutually exclusive".into(),
            });
        }
        let mut payload = HashMap::from([
            ("peer_hash".into(), Value::from(request.peer_hash.as_str())),
            ("content".into(), Value::from(request.content.as_str())),
        ]);
        insert_optional_text(&mut payload, "title", request.title.as_deref());
        insert_optional_text(&mut payload, "delivery_method", request.delivery_method.as_deref());
        if let Some(attachment) = &request.attachment {
            payload.insert("attachment".into(), Value::Binary(attachment.clone()));
            insert_optional_text(
                &mut payload,
                "attachment_name",
                request.attachment_name.as_deref(),
            );
        } else if request.attachment_name.is_some() {
            return Err(ClientError::Protocol {
                message: "attachment_name requires a legacy attachment".into(),
            });
        }
        if !request.attachments.is_empty() {
            payload.insert(
                "attachments".into(),
                Value::Array(request.attachments.iter().map(attachment_value).collect()),
            );
        }
        let frame = self.request(MessageType::CmdSendChatOutcome, payload, SEND_DEADLINE).await?;
        let outcome: SendChatOutcome = decode_key(&frame.payload, &["outcome"], "send outcome")?;
        if outcome.message_id.is_empty() || outcome.message.id != outcome.message_id {
            return Err(ClientError::Protocol {
                message: "send outcome omitted its authoritative message projection".into(),
            });
        }
        Ok(outcome)
    }

    pub async fn announce(&self) -> Result<bool, ClientError> {
        let frame =
            self.request(MessageType::CmdAnnounce, HashMap::new(), DEFAULT_DEADLINE).await?;
        required_bool(&frame.payload, "success", "announce response")
    }

    pub async fn config(&self) -> Result<ConfigSnapshot, ClientError> {
        let frame =
            self.request(MessageType::QueryConfig, HashMap::new(), DEFAULT_DEADLINE).await?;
        let mut values = BTreeMap::new();
        for (key, value) in frame.payload {
            let json = rmpv::ext::from_value(value).map_err(|error| ClientError::Protocol {
                message: format!("invalid config value {key}: {error}"),
            })?;
            values.insert(key, json);
        }
        let mut snapshot = ConfigSnapshot::default();
        snapshot.values = values;
        Ok(snapshot)
    }

    pub async fn path_info(&self, destination: &str) -> Result<Option<PathInfo>, ClientError> {
        let payload = HashMap::from([("destination_hash".into(), Value::from(destination))]);
        let frame = self.request(MessageType::QueryPathInfo, payload, DEFAULT_DEADLINE).await?;
        if !required_bool(&frame.payload, "found", "path response")? {
            return Ok(None);
        }
        decode_map(&frame.payload, "path").map(Some)
    }

    pub async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, ClientError> {
        let frame =
            self.request(MessageType::QueryTunnels, HashMap::new(), DEFAULT_DEADLINE).await?;
        required_array(&frame.payload, "tunnels", "tunnel list")?.iter().map(tunnel_info).collect()
    }

    pub async fn tunnel_status(&self, peer_hash: &str) -> Result<TunnelStatus, ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        let frame = self.request(MessageType::QueryTunnelStatus, payload, DEFAULT_DEADLINE).await?;
        if frame.payload.contains_key("operation_id") {
            decode_map(&frame.payload, "tunnel operation").map(TunnelStatus::Operation)
        } else {
            tunnel_info_from_payload(&frame.payload).map(TunnelStatus::Tunnel)
        }
    }

    pub async fn tunnel_establish(
        &self,
        peer_hash: &str,
    ) -> Result<TunnelOperationInfo, ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        let frame = self.request(MessageType::CmdTunnelEstablish, payload, SEND_DEADLINE).await?;
        Ok(TunnelOperationInfo {
            operation_id: required_text(&frame.payload, "operation_id", "tunnel establish")?,
            peer_hash: required_text(&frame.payload, "peer_hash", "tunnel establish")?,
            kind: "establish".into(),
            state: required_text(&frame.payload, "state", "tunnel establish")?,
            ..TunnelOperationInfo::default()
        })
    }

    pub async fn tunnel_teardown(&self, peer_hash: &str) -> Result<bool, ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        let frame = self.request(MessageType::CmdTunnelTeardown, payload, DEFAULT_DEADLINE).await?;
        required_bool(&frame.payload, "success", "tunnel teardown")
    }

    pub async fn device_status(
        &self,
        destination: &str,
        timeout_secs: u64,
    ) -> Result<RemoteStatusInfo, ClientError> {
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from(destination)),
            ("timeout".into(), Value::from(timeout_secs)),
        ]);
        let frame = self
            .request(MessageType::CmdDeviceStatus, payload, operation_deadline(timeout_secs))
            .await?;
        let mut status = RemoteStatusInfo::default();
        status.destination_hash =
            required_text(&frame.payload, "destination_hash", "device status")?;
        status.uptime = frame.payload.get("uptime").and_then(Value::as_u64);
        status.daemon_version = frame
            .payload
            .get("daemon_version")
            .or_else(|| frame.payload.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        for (key, value) in &frame.payload {
            if matches!(key.as_str(), "destination_hash" | "uptime" | "daemon_version" | "version")
            {
                continue;
            }
            let value =
                rmpv::ext::from_value(value.clone()).map_err(|error| ClientError::Protocol {
                    message: format!("invalid device status value {key}: {error}"),
                })?;
            status.extra.insert(key.clone(), value);
        }
        Ok(status)
    }

    pub async fn exec(
        &self,
        destination: &str,
        command: &str,
        args: &[String],
        timeout_secs: u64,
    ) -> Result<ExecResult, ClientError> {
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from(destination)),
            ("command".into(), Value::from(command)),
            (
                "args".into(),
                Value::Array(args.iter().map(|arg| Value::from(arg.as_str())).collect()),
            ),
            ("timeout".into(), Value::from(timeout_secs)),
        ]);
        let frame =
            self.request(MessageType::CmdExec, payload, operation_deadline(timeout_secs)).await?;
        decode_map(&frame.payload, "exec result")
    }

    pub async fn reboot_device(
        &self,
        destination: &str,
        delay_secs: u64,
    ) -> Result<RebootResult, ClientError> {
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from(destination)),
            ("delay".into(), Value::from(delay_secs)),
        ]);
        let frame = self.request(MessageType::CmdRebootDevice, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "reboot result")
    }

    pub async fn fleet_apply(
        &self,
        destination: &str,
        profile_bytes: &[u8],
        verify: bool,
        timeout_secs: u64,
    ) -> Result<ConfigApplyResult, ClientError> {
        use base64::Engine;
        let profile = base64::engine::general_purpose::STANDARD.encode(profile_bytes);
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from(destination)),
            ("profile".into(), Value::from(profile)),
            ("verify".into(), Value::from(verify)),
            ("timeout".into(), Value::from(timeout_secs)),
        ]);
        let frame = self
            .request(MessageType::CmdFleetApply, payload, operation_deadline(timeout_secs))
            .await?;
        decode_map(&frame.payload, "fleet apply result")
    }

    pub async fn fleet_grant(
        &self,
        identity_hash: &str,
        role: &str,
        label: &str,
        grants: &[String],
    ) -> Result<bool, ClientError> {
        let mut payload = HashMap::from([
            ("identity_hash".into(), Value::from(identity_hash)),
            ("role".into(), Value::from(role)),
            ("label".into(), Value::from(label)),
        ]);
        if !grants.is_empty() {
            payload.insert(
                "grants".into(),
                Value::Array(grants.iter().map(|grant| Value::from(grant.as_str())).collect()),
            );
        }
        let frame = self.request(MessageType::CmdFleetGrant, payload, DEFAULT_DEADLINE).await?;
        required_bool(&frame.payload, "success", "fleet grant")
    }

    pub async fn fleet_revoke(&self, identity_hash: &str) -> Result<bool, ClientError> {
        let payload = HashMap::from([("identity_hash".into(), Value::from(identity_hash))]);
        let frame = self.request(MessageType::CmdFleetRevoke, payload, DEFAULT_DEADLINE).await?;
        required_bool(&frame.payload, "success", "fleet revoke")
    }

    // ── Transport observation ───────────────────────────────────────────────

    /// Active and historical link telemetry from the daemon's link tables.
    pub async fn links(&self) -> Result<LinkSnapshot, ClientError> {
        let frame = self.request(MessageType::QueryLinks, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "link snapshot")
    }

    /// Every known route with hops, next hop, interface, and expiry.
    pub async fn path_table(&self) -> Result<Vec<PathInfo>, ClientError> {
        let frame =
            self.request(MessageType::QueryPathTable, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["paths"], "path table")
    }

    /// Per-interface counters and state.
    pub async fn interface_stats(&self) -> Result<Vec<InterfaceDetail>, ClientError> {
        let frame = self
            .request(MessageType::QueryInterfaceStats, HashMap::new(), DEFAULT_DEADLINE)
            .await?;
        decode_key(&frame.payload, &["interfaces"], "interface stats")
    }

    // ── Network operations, requests, and resources ─────────────────────────

    /// Start a bounded network operation (path request, link, probe).
    pub async fn start_network_operation(
        &self,
        request: &StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, ClientError> {
        let mut payload = HashMap::from([
            ("kind".into(), Value::from(request.kind.as_str())),
            ("timeout_ms".into(), Value::from(request.timeout_ms)),
        ]);
        if let Some(destination) = &request.destination_hash {
            payload.insert("destination_hash".into(), Value::from(destination.as_str()));
        }
        if let Some(link_id) = &request.link_id {
            payload.insert("link_id".into(), Value::from(link_id.as_str()));
        }
        let frame =
            self.request(MessageType::CmdNetworkOperationStart, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "network operation")
    }

    pub async fn cancel_network_operation(
        &self,
        operation_id: &str,
    ) -> Result<NetworkOperationInfo, ClientError> {
        let payload = HashMap::from([("operation_id".into(), Value::from(operation_id))]);
        let frame =
            self.request(MessageType::CmdNetworkOperationCancel, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "network operation")
    }

    pub async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, ClientError> {
        let frame = self
            .request(MessageType::QueryNetworkOperation, HashMap::new(), DEFAULT_DEADLINE)
            .await?;
        decode_key(&frame.payload, &["operations"], "network operations")
    }

    /// Start a native RNS request over an active link.
    pub async fn start_request(
        &self,
        request: &StartRequestInfo,
    ) -> Result<RequestObservationInfo, ClientError> {
        let mut payload = HashMap::from([
            ("link_id".into(), Value::from(request.link_id.as_str())),
            ("path".into(), Value::from(request.path.as_str())),
            ("data".into(), Value::Binary(request.data.clone())),
            ("timeout_ms".into(), Value::from(request.timeout_ms)),
            ("max_response_size".into(), Value::from(request.max_response_size)),
        ]);
        if let Some(correlation_id) = &request.correlation_id {
            payload.insert("correlation_id".into(), Value::from(correlation_id.as_str()));
        }
        let frame = self.request(MessageType::CmdRequestStart, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "request observation")
    }

    pub async fn cancel_request(
        &self,
        request_id: &str,
    ) -> Result<RequestObservationInfo, ClientError> {
        let payload = HashMap::from([("request_id".into(), Value::from(request_id))]);
        let frame = self.request(MessageType::CmdRequestCancel, payload, DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "request observation")
    }

    pub async fn requests(&self) -> Result<Vec<RequestObservationInfo>, ClientError> {
        let frame =
            self.request(MessageType::QueryRequests, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["requests"], "requests")
    }

    pub async fn resources(&self) -> Result<Vec<ResourceTransferInfo>, ClientError> {
        let frame =
            self.request(MessageType::QueryResources, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_key(&frame.payload, &["resources"], "resources")
    }

    /// Cancel a resource transfer; returns whether the daemon accepted the cancel.
    pub async fn cancel_resource(&self, resource_hash: &str) -> Result<bool, ClientError> {
        let payload = HashMap::from([("resource_hash".into(), Value::from(resource_hash))]);
        let frame = self.request(MessageType::CmdResourceCancel, payload, DEFAULT_DEADLINE).await?;
        Ok(frame.payload.get("accepted").and_then(Value::as_bool).unwrap_or(false))
    }

    // ── Conversations and messages ──────────────────────────────────────────

    /// One page of a conversation, newest first. A stale cursor is retried
    /// once from the start and reported through `reset`.
    pub async fn message_page(
        &self,
        peer_hash: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Paged<MessagePage>, ClientError> {
        let mut payload = HashMap::from([
            ("peer_hash".into(), Value::from(peer_hash)),
            ("limit".into(), Value::from(limit)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), Value::from(cursor));
        }
        let first =
            self.request(MessageType::QueryMessages, payload.clone(), DEFAULT_DEADLINE).await;
        let (frame, reset) = match first {
            Ok(frame) => (frame, false),
            Err(error) if cursor.is_some() && is_stale_cursor(&error) => {
                payload.remove("cursor");
                (self.request(MessageType::QueryMessages, payload, DEFAULT_DEADLINE).await?, true)
            }
            Err(error) => return Err(error),
        };
        let messages = match frame.payload.get("messages") {
            Some(value) => decode_value(value.clone(), "messages")?,
            None => Vec::new(),
        };
        let mut page = MessagePage::default();
        page.messages = messages;
        page.next_cursor =
            frame.payload.get("next_cursor").and_then(Value::as_str).map(str::to_owned);
        Ok(Paged { page, reset, pagination_supported: frame.payload.contains_key("next_cursor") })
    }

    pub async fn message(&self, message_id: &str) -> Result<Option<MessageInfo>, ClientError> {
        let payload = HashMap::from([("message_id".into(), Value::from(message_id))]);
        let frame = self.request(MessageType::QueryMessage, payload, DEFAULT_DEADLINE).await?;
        match frame.payload.get("message") {
            None | Some(Value::Nil) => Ok(None),
            Some(value) => decode_value(value.clone(), "message").map(Some),
        }
    }

    /// One page of conversations. A stale cursor is retried once from the start.
    pub async fn conversation_page(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Paged<ConversationPage>, ClientError> {
        let mut payload = HashMap::from([
            ("unread_only".into(), Value::Boolean(false)),
            ("limit".into(), Value::from(limit)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), Value::from(cursor));
        }
        let first =
            self.request(MessageType::QueryConversations, payload.clone(), DEFAULT_DEADLINE).await;
        let (frame, reset) = match first {
            Ok(frame) => (frame, false),
            Err(error) if cursor.is_some() && is_stale_cursor(&error) => {
                payload.remove("cursor");
                (
                    self.request(MessageType::QueryConversations, payload, DEFAULT_DEADLINE)
                        .await?,
                    true,
                )
            }
            Err(error) => return Err(error),
        };
        let conversations = match frame.payload.get("conversations") {
            Some(value) => decode_value(value.clone(), "conversations")?,
            None => Vec::new(),
        };
        let mut page = ConversationPage::default();
        page.conversations = conversations;
        page.next_cursor =
            frame.payload.get("next_cursor").and_then(Value::as_str).map(str::to_owned);
        Ok(Paged { page, reset, pagination_supported: frame.payload.contains_key("next_cursor") })
    }

    /// Mark a conversation read. Older daemons answer with a bare count.
    pub async fn mark_read(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        let frame = self.request(MessageType::CmdMarkRead, payload, DEFAULT_DEADLINE).await?;
        if let Some(outcome) = frame.payload.get("outcome") {
            return decode_value(outcome.clone(), "mark-read outcome");
        }
        let count = frame.payload.get("count").and_then(Value::as_u64).ok_or_else(|| {
            ClientError::Protocol { message: "mark-read response omitted outcome and count".into() }
        })?;
        Ok(legacy_outcome(count, peer_hash))
    }

    /// Delete one message. Older daemons answer with a bare deleted flag.
    pub async fn delete_message(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, ClientError> {
        let payload = HashMap::from([("message_id".into(), Value::from(message_id))]);
        let frame = self.request(MessageType::CmdDeleteMessage, payload, DEFAULT_DEADLINE).await?;
        if let Some(outcome) = frame.payload.get("outcome") {
            return decode_value(outcome.clone(), "delete outcome");
        }
        let deleted = frame.payload.get("deleted").and_then(Value::as_bool).ok_or_else(|| {
            ClientError::Protocol { message: "delete response omitted outcome and flag".into() }
        })?;
        Ok(legacy_outcome(u64::from(deleted), message_id))
    }

    pub async fn retry_message(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, ClientError> {
        let payload = HashMap::from([("message_id".into(), Value::from(message_id))]);
        let frame = self.request(MessageType::CmdRetryMessage, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "outcome", "retry outcome")
    }

    pub async fn cancel_message(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, ClientError> {
        let payload = HashMap::from([("message_id".into(), Value::from(message_id))]);
        let frame = self.request(MessageType::CmdCancelMessage, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "outcome", "cancel outcome")
    }

    // ── Drafts ──────────────────────────────────────────────────────────────

    pub async fn set_draft(
        &self,
        peer_hash: &str,
        content: &str,
    ) -> Result<ConversationDraft, ClientError> {
        let payload = HashMap::from([
            ("peer_hash".into(), Value::from(peer_hash)),
            ("content".into(), Value::from(content)),
        ]);
        let frame = self.request(MessageType::CmdSetDraft, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "draft", "draft")
    }

    pub async fn draft(&self, peer_hash: &str) -> Result<Option<ConversationDraft>, ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        let frame = self.request(MessageType::QueryDraft, payload, DEFAULT_DEADLINE).await?;
        match frame.payload.get("draft") {
            None | Some(Value::Nil) => Ok(None),
            Some(value) => decode_value(value.clone(), "draft").map(Some),
        }
    }

    pub async fn clear_draft(&self, peer_hash: &str) -> Result<(), ClientError> {
        let payload = HashMap::from([("peer_hash".into(), Value::from(peer_hash))]);
        self.request(MessageType::CmdClearDraft, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    // ── Identity, auto-reply, and blocking ──────────────────────────────────

    pub async fn set_identity(
        &self,
        display_name: &str,
        icon: Option<&str>,
    ) -> Result<(), ClientError> {
        let mut payload = HashMap::from([("display_name".into(), Value::from(display_name))]);
        if let Some(icon) = icon {
            payload.insert("icon".into(), Value::from(icon));
        }
        self.request(MessageType::CmdSetIdentity, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn set_auto_reply(
        &self,
        mode: &str,
        message: &str,
        cooldown_secs: Option<u64>,
    ) -> Result<(), ClientError> {
        let mut payload = HashMap::from([
            ("mode".into(), Value::from(mode)),
            ("message".into(), Value::from(message)),
        ]);
        if let Some(cooldown) = cooldown_secs {
            payload.insert("cooldown_secs".into(), Value::from(cooldown));
        }
        self.request(MessageType::CmdSetAutoReply, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn block_peer(&self, identity_hash: &str) -> Result<(), ClientError> {
        let payload = HashMap::from([("identity_hash".into(), Value::from(identity_hash))]);
        self.request(MessageType::CmdBlockPeer, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn unblock_peer(&self, identity_hash: &str) -> Result<(), ClientError> {
        let payload = HashMap::from([("identity_hash".into(), Value::from(identity_hash))]);
        self.request(MessageType::CmdUnblockPeer, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn blocked_peers(&self) -> Result<Vec<String>, ClientError> {
        let frame =
            self.request(MessageType::QueryBlockedPeers, HashMap::new(), DEFAULT_DEADLINE).await?;
        Ok(frame
            .payload
            .get("blocked_peers")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default())
    }

    /// The daemon's raw configuration map; typed access is `config()`.
    pub async fn config_map(&self) -> Result<HashMap<String, Value>, ClientError> {
        let frame =
            self.request(MessageType::QueryConfig, HashMap::new(), DEFAULT_DEADLINE).await?;
        Ok(frame.payload)
    }

    // ── Remote terminals ────────────────────────────────────────────────────

    /// Open a remote terminal session; returns the session id.
    pub async fn terminal_open(
        &self,
        destination_hash: &str,
        rows: u16,
        cols: u16,
    ) -> Result<String, ClientError> {
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from(destination_hash)),
            ("rows".into(), Value::from(u64::from(rows))),
            ("cols".into(), Value::from(u64::from(cols))),
        ]);
        let frame = self.request(MessageType::CmdTerminalOpen, payload, DEFAULT_DEADLINE).await?;
        let session_id = text(&frame.payload, "session_id", "");
        if session_id.is_empty() {
            return Err(ClientError::Protocol {
                message: "terminal open omitted the session id".into(),
            });
        }
        Ok(session_id)
    }

    pub async fn terminal_input(&self, session_id: &str, data: &[u8]) -> Result<(), ClientError> {
        let payload = HashMap::from([
            ("session_id".into(), Value::from(session_id)),
            ("data".into(), Value::Binary(data.to_vec())),
        ]);
        self.request(MessageType::CmdTerminalInput, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn terminal_resize(
        &self,
        session_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), ClientError> {
        let payload = HashMap::from([
            ("session_id".into(), Value::from(session_id)),
            ("rows".into(), Value::from(u64::from(rows))),
            ("cols".into(), Value::from(u64::from(cols))),
        ]);
        self.request(MessageType::CmdTerminalResize, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn terminal_close(&self, session_id: &str) -> Result<(), ClientError> {
        let payload = HashMap::from([("session_id".into(), Value::from(session_id))]);
        self.request(MessageType::CmdTerminalClose, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    // ── Pages and files ─────────────────────────────────────────────────────

    pub async fn close_page(&self, session_id: &str) -> Result<(), ClientError> {
        let payload = HashMap::from([("session_id".into(), Value::from(session_id))]);
        self.request(MessageType::CmdPageDisconnect, payload, DEFAULT_DEADLINE).await.map(|_| ())
    }

    pub async fn cancel_file_download(
        &self,
        download_id: &str,
    ) -> Result<FileDownloadInfo, ClientError> {
        let payload = HashMap::from([("download_id".into(), Value::from(download_id))]);
        let frame =
            self.request(MessageType::CmdFileDownloadCancel, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "download", "file download")
    }

    pub async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, ClientError> {
        let payload = HashMap::from([
            ("download_id".into(), Value::from(download_id)),
            ("destination".into(), Value::from(destination)),
        ]);
        let frame =
            self.request(MessageType::CmdFileDownloadSave, payload, DEFAULT_DEADLINE).await?;
        decode_typed_key(&frame.payload, "download", "file download")
    }

    /// Known page paths on a host as `(path, host_hash)` pairs.
    pub async fn list_pages(&self, host: &str) -> Result<Vec<(String, String)>, ClientError> {
        let payload = HashMap::from([("host".into(), Value::from(host))]);
        let frame = self.request(MessageType::CmdPageListSites, payload, DEFAULT_DEADLINE).await?;
        Ok(frame
            .payload
            .get("pages")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let map = entry.as_map()?;
                        let field = |name: &str| {
                            map.iter()
                                .find(|(key, _)| key.as_str() == Some(name))
                                .and_then(|(_, value)| value.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        Some((field("path"), field("host_hash")))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    #[must_use]
    pub fn diagnostics(&self) -> ClientDiagnostics {
        ClientDiagnostics {
            queue_depth: self.metrics.queue_depth.load(Ordering::Relaxed),
            in_flight: self.metrics.in_flight.load(Ordering::Relaxed),
            completed: self.metrics.completed.load(Ordering::Relaxed),
            timed_out: self.metrics.timed_out.load(Ordering::Relaxed),
            cancelled: self.metrics.cancelled.load(Ordering::Relaxed),
            overloaded: self.metrics.overloaded.load(Ordering::Relaxed),
            disconnected: self.metrics.disconnected.load(Ordering::Relaxed),
            stale_responses: self.metrics.stale_responses.load(Ordering::Relaxed),
            dropped_responses: self.metrics.dropped_responses.load(Ordering::Relaxed),
            last_latency_ms: self.metrics.last_latency_ms.load(Ordering::Relaxed),
            events_received: self.metrics.events_received.load(Ordering::Relaxed),
            events_unobserved: self.metrics.events_unobserved.load(Ordering::Relaxed),
        }
    }

    fn next_request_id(&self) -> [u8; REQUEST_ID_SIZE] {
        let mut request_id = [0; REQUEST_ID_SIZE];
        request_id[..8].copy_from_slice(&self.generation.0.to_le_bytes());
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        request_id[8..].copy_from_slice(&sequence.to_le_bytes());
        request_id
    }
}

struct PendingGuard {
    request_id: Option<[u8; REQUEST_ID_SIZE]>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<Metrics>,
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

async fn writer_task(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut requests: mpsc::Receiver<Request>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<Metrics>,
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
        if let Err(error) = wire::write_frame_async(
            &mut writer,
            request.message_type,
            &request_id,
            &request.payload,
        )
        .await
        {
            disconnect_pending(&pending, &metrics, &connected, format!("write failed: {error}"))
                .await;
            break;
        }
    }
}

async fn reader_task(
    mut reader: tokio::net::unix::OwnedReadHalf,
    generation: ConnectionGeneration,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    metrics: Arc<Metrics>,
    connected: Arc<AtomicBool>,
    events: broadcast::Sender<EventFrame>,
) {
    loop {
        let frame = match wire::read_frame_async(&mut reader).await {
            Ok(frame) => frame,
            Err(error) => {
                disconnect_pending(&pending, &metrics, &connected, format!("read failed: {error}"))
                    .await;
                break;
            }
        };
        // Pushed events are not correlated to a request; fan them out to every
        // live receiver without touching the pending table.
        if frame.msg_type.is_event() {
            metrics.events_received.fetch_add(1, Ordering::Relaxed);
            let event =
                EventFrame { message_type: frame.msg_type, payload: frame.payload, generation };
            // The client keeps one never-read prototype receiver so it can hand
            // out subscriptions; an event seen by no other receiver is unobserved.
            if events.receiver_count() <= 1 {
                metrics.events_unobserved.fetch_add(1, Ordering::Relaxed);
            }
            let _ = events.send(event);
            continue;
        }
        let mut generation_bytes = [0; 8];
        generation_bytes.copy_from_slice(&frame.request_id[..8]);
        if u64::from_le_bytes(generation_bytes) != generation.0 {
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
        let result = validate_response(frame);
        let _ = request.response.send(result);
    }
}

fn validate_response(frame: Frame) -> Result<Frame, ClientError> {
    if frame.msg_type == MessageType::Error {
        return Err(parse_remote_error(&frame.payload));
    }
    if !frame.msg_type.is_response() {
        return Err(ClientError::Protocol {
            message: format!("unexpected response frame {:?}", frame.msg_type),
        });
    }
    Ok(frame)
}

fn parse_remote_error(payload: &HashMap<String, Value>) -> ClientError {
    if let Some(value) = payload.get("typed_error")
        && let Ok(error) = rmpv::ext::from_value::<IpcError>(value.clone())
    {
        return ClientError::Remote(error);
    }
    ClientError::LegacyRemote {
        kind: text(payload, "kind", "internal"),
        code: text(payload, "code", "internal"),
        message: text(payload, "message", "daemon request failed"),
    }
}

/// Decode a typed daemon payload: a named-map MessagePack document carried as
/// a binary value under `key`.
fn decode_typed_key<T: DeserializeOwned>(
    payload: &HashMap<String, Value>,
    key: &str,
    context: &str,
) -> Result<T, ClientError> {
    let bytes = payload.get(key).and_then(Value::as_slice).ok_or_else(|| {
        ClientError::Protocol { message: format!("{context} response omitted typed {key}") }
    })?;
    let mut cursor = bytes;
    let value = rmpv::decode::read_value(&mut cursor).map_err(|error| ClientError::Protocol {
        message: format!("invalid typed {context} payload: {error}"),
    })?;
    decode_value(value, context)
}

fn encode_typed<T: serde::Serialize>(value: &T, context: &str) -> Result<Value, ClientError> {
    let json = serde_json::to_value(value)
        .map_err(|error| ClientError::Protocol { message: format!("encode {context}: {error}") })?;
    let value = rmpv::ext::to_value(json)
        .map_err(|error| ClientError::Protocol { message: format!("encode {context}: {error}") })?;
    let mut bytes = Vec::new();
    rmpv::encode::write_value(&mut bytes, &value)
        .map_err(|error| ClientError::Protocol { message: format!("encode {context}: {error}") })?;
    Ok(Value::Binary(bytes))
}

fn decode_map<T: DeserializeOwned>(
    payload: &HashMap<String, Value>,
    context: &str,
) -> Result<T, ClientError> {
    decode_payload(payload, context)
}

fn decode_key<T: DeserializeOwned>(
    payload: &HashMap<String, Value>,
    keys: &[&str],
    context: &str,
) -> Result<T, ClientError> {
    let value = keys.iter().find_map(|key| payload.get(*key)).cloned().ok_or_else(|| {
        ClientError::Protocol {
            message: format!("{context} response omitted {}", keys.join(" or ")),
        }
    })?;
    decode_value(value, context)
}

/// Decode a daemon value into its canonical typed record.
///
/// The daemon spells enum fields as strings (`"verified"`, `"delivered"`),
/// which rmpv's direct enum decoding rejects. Bridging through a JSON value
/// keeps every canonical `styrene_ipc::types` record decodable from the wire
/// representation, so front ends share one decoder instead of hand parsers.
pub fn decode_value<T: DeserializeOwned>(value: Value, context: &str) -> Result<T, ClientError> {
    let bridged = rmpv::ext::from_value::<serde_json::Value>(value.clone())
        .map_err(|error| format!("invalid {context} value: {error}"))
        .and_then(|json| {
            serde_json::from_value::<T>(json).map_err(|error| format!("invalid {context}: {error}"))
        });
    match bridged {
        Ok(decoded) => Ok(decoded),
        // Values encoded straight from serde through rmpv carry enums in
        // rmpv's native array form, which the JSON bridge cannot express.
        Err(bridge_error) => rmpv::ext::from_value::<T>(value)
            .map_err(|_| ClientError::Protocol { message: bridge_error }),
    }
}

/// Decode a whole frame payload as one canonical typed record.
pub fn decode_payload<T: DeserializeOwned>(
    payload: &HashMap<String, Value>,
    context: &str,
) -> Result<T, ClientError> {
    let value = Value::Map(
        payload.iter().map(|(key, value)| (Value::from(key.as_str()), value.clone())).collect(),
    );
    decode_value(value, context)
}

fn required_bool(
    payload: &HashMap<String, Value>,
    key: &str,
    context: &str,
) -> Result<bool, ClientError> {
    payload.get(key).and_then(Value::as_bool).ok_or_else(|| ClientError::Protocol {
        message: format!("{context} omitted boolean {key}"),
    })
}

fn required_text(
    payload: &HashMap<String, Value>,
    key: &str,
    context: &str,
) -> Result<String, ClientError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ClientError::Protocol { message: format!("{context} omitted string {key}") })
}

fn required_array<'a>(
    payload: &'a HashMap<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a [Value], ClientError> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| ClientError::Protocol { message: format!("{context} omitted array {key}") })
}

fn tunnel_info(value: &Value) -> Result<TunnelInfo, ClientError> {
    let map: HashMap<String, Value> = value
        .as_map()
        .ok_or_else(|| ClientError::Protocol { message: "tunnel list entry is not a map".into() })?
        .iter()
        .map(|(key, value)| {
            key.as_str().map(|key| (key.to_owned(), value.clone())).ok_or_else(|| {
                ClientError::Protocol { message: "tunnel list entry has a non-string key".into() }
            })
        })
        .collect::<Result<_, _>>()?;
    tunnel_info_from_payload(&map)
}

fn tunnel_info_from_payload(payload: &HashMap<String, Value>) -> Result<TunnelInfo, ClientError> {
    let mut info = TunnelInfo::default();
    info.peer_hash = required_text(payload, "peer_hash", "tunnel")?;
    info.backend = payload.get("backend").and_then(Value::as_str).unwrap_or_default().to_owned();
    info.state = required_text(payload, "state", "tunnel")?;
    info.remote_endpoint = optional_nonempty_text(payload, "remote_endpoint");
    info.interface_name = optional_nonempty_text(payload, "interface_name");
    info.tx_bytes = payload.get("tx_bytes").and_then(Value::as_u64).unwrap_or_default();
    info.rx_bytes = payload.get("rx_bytes").and_then(Value::as_u64).unwrap_or_default();
    info.established_at = payload.get("established_at").and_then(Value::as_i64).filter(|v| *v != 0);
    info.last_rekey = payload.get("last_rekey").and_then(Value::as_i64).filter(|v| *v != 0);
    info.pqc_session_id = optional_nonempty_text(payload, "pqc_session_id");
    Ok(info)
}

fn optional_nonempty_text(payload: &HashMap<String, Value>, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_owned)
}

fn operation_deadline(timeout_secs: u64) -> Duration {
    Duration::from_secs(timeout_secs.saturating_add(5))
}

fn insert_optional_text(payload: &mut HashMap<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        payload.insert(key.into(), Value::from(value));
    }
}

fn attachment_value(attachment: &styrene_ipc::types::AttachmentInput) -> Value {
    let mut fields = vec![
        (Value::from("name"), Value::from(attachment.name.as_str())),
        (Value::from("bytes"), Value::Binary(attachment.bytes.clone())),
    ];
    if let Some(content_type) = &attachment.content_type {
        fields.push((Value::from("content_type"), Value::from(content_type.as_str())));
    }
    if let Some(expected) = &attachment.expected_sha256 {
        fields.push((Value::from("expected_sha256"), Value::from(expected.as_str())));
    }
    Value::Map(fields)
}

/// Daemons report a stale paging cursor as a typed or legacy remote error.
fn is_stale_cursor(error: &ClientError) -> bool {
    match error {
        ClientError::Remote(remote) => format!("{remote:?}").contains("cursor_stale"),
        ClientError::LegacyRemote { code, message, .. } => {
            code.contains("cursor_stale") || message.contains("cursor_stale")
        }
        _ => false,
    }
}

/// The typed outcome older daemons implied with a bare count or flag.
fn legacy_outcome(count: u64, target_id: &str) -> MessagingOperationOutcome {
    let mut outcome = MessagingOperationOutcome::default();
    outcome.disposition =
        if count == 0 { MessagingDisposition::Unchanged } else { MessagingDisposition::Applied };
    outcome.affected_count = count;
    outcome.target_id = target_id.into();
    outcome
}

fn text(payload: &HashMap<String, Value>, key: &str, default: &str) -> String {
    payload.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
}

async fn disconnect_pending(
    pending: &Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>,
    metrics: &Metrics,
    connected: &AtomicBool,
    message: String,
) {
    if connected.swap(false, Ordering::AcqRel) {
        metrics.disconnected.fetch_add(1, Ordering::Relaxed);
    }
    let requests = std::mem::take(&mut *pending.lock().await);
    metrics.in_flight.fetch_sub(requests.len(), Ordering::Relaxed);
    for request in requests.into_values() {
        let _ = request.response.send(Err(ClientError::Disconnected { message: message.clone() }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_string_enum_fields_from_daemon_payloads() {
        use styrene_ipc::types::{
            MessageAuthenticationState, MessageInfo, MessageLifecycleState, MessageStampState,
        };
        let payload = HashMap::from([
            ("id".to_string(), Value::from("m1")),
            ("kind".to_string(), Value::from("new")),
            ("content".to_string(), Value::from("hello")),
            ("authentication_state".to_string(), Value::from("verified")),
            ("lifecycle_state".to_string(), Value::from("delivered")),
            ("stamp_state".to_string(), Value::from("not_applicable")),
        ]);
        let message: MessageInfo = decode_payload(&payload, "message").expect("decode");
        assert_eq!(message.id, "m1");
        assert_eq!(message.content, "hello");
        assert_eq!(message.authentication_state, MessageAuthenticationState::Verified);
        assert_eq!(message.lifecycle_state, MessageLifecycleState::Delivered);
        assert_eq!(message.stamp_state, MessageStampState::NotApplicable);
        let direct: Result<MessageInfo, _> = rmpv::ext::from_value(Value::Map(
            payload.iter().map(|(k, v)| (Value::from(k.as_str()), v.clone())).collect(),
        ));
        assert!(direct.is_err(), "rmpv direct enum decoding rejects string variants");

        let native = rmpv::ext::to_value(&message).expect("encode natively");
        let roundtrip: MessageInfo = decode_value(native, "message").expect("native decode");
        assert_eq!(roundtrip, message);
    }

    fn typed_value<T: serde::Serialize>(value: &T) -> Value {
        let json = serde_json::to_value(value).expect("serialize typed value");
        rmpv::ext::to_value(json).expect("project typed value")
    }

    fn typed_payload<T: serde::Serialize>(value: &T) -> HashMap<String, Value> {
        typed_value(value)
            .as_map()
            .expect("typed payload map")
            .iter()
            .map(|(key, value)| {
                (key.as_str().expect("string payload key").to_string(), value.clone())
            })
            .collect()
    }

    fn pair(capacity: usize) -> (Client, UnixStream) {
        let (client, server) = UnixStream::pair().expect("Unix stream pair");
        (Client::from_unix_stream_with_capacity(client, ConnectionGeneration(7), capacity), server)
    }

    async fn reply(
        server: &mut UnixStream,
        message_type: MessageType,
        request_id: &[u8; REQUEST_ID_SIZE],
        payload: &HashMap<String, Value>,
    ) {
        wire::write_frame_async(server, message_type, request_id, payload)
            .await
            .expect("write response");
    }

    #[tokio::test]
    async fn correlates_concurrent_out_of_order_responses() {
        let (client, mut server) = pair(4);
        let first = tokio::spawn({
            let client = client.clone();
            async move {
                client.request(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await
            }
        });
        let second = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(MessageType::QueryStatus, HashMap::new(), Duration::from_secs(1))
                    .await
            }
        });
        let request_a = wire::read_frame_async(&mut server).await.expect("first request");
        let request_b = wire::read_frame_async(&mut server).await.expect("second request");
        let response_payload = |message_type| {
            let value = if message_type == MessageType::Ping { "ping" } else { "status" };
            HashMap::from([("value".into(), Value::from(value))])
        };
        let payload_a = response_payload(request_a.msg_type);
        let payload_b = response_payload(request_b.msg_type);
        reply(&mut server, MessageType::Result, &request_b.request_id, &payload_b).await;
        reply(&mut server, MessageType::Result, &request_a.request_id, &payload_a).await;

        assert_eq!(
            first.await.expect("first task").expect("first response").payload["value"].as_str(),
            Some("ping")
        );
        assert_eq!(
            second.await.expect("second task").expect("second response").payload["value"].as_str(),
            Some("status")
        );
        assert_eq!(client.diagnostics().completed, 2);
    }

    #[tokio::test]
    async fn enforces_capacity_deadline_and_cancellation() {
        let (client, mut server) = pair(1);
        let pending = tokio::spawn({
            let client = client.clone();
            async move {
                client.request(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await
            }
        });
        let _request = wire::read_frame_async(&mut server).await.expect("pending request");
        assert!(matches!(
            client
                .request(MessageType::QueryStatus, HashMap::new(), Duration::from_millis(10))
                .await,
            Err(ClientError::Overloaded)
        ));
        pending.abort();
        tokio::task::yield_now().await;

        assert!(matches!(
            client
                .request(MessageType::QueryStatus, HashMap::new(), Duration::from_millis(10))
                .await,
            Err(ClientError::Timeout { .. })
        ));
        let diagnostics = client.diagnostics();
        assert_eq!(diagnostics.overloaded, 1);
        assert_eq!(diagnostics.cancelled, 1);
        assert_eq!(diagnostics.timed_out, 1);
    }

    #[tokio::test]
    async fn reports_disconnect_to_in_flight_requests() {
        let (client, mut server) = pair(2);
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client.request(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await
            }
        });
        let _request = wire::read_frame_async(&mut server).await.expect("request");
        drop(server);

        assert!(matches!(
            request.await.expect("request task"),
            Err(ClientError::Disconnected { .. })
        ));
        assert_eq!(client.diagnostics().disconnected, 1);
    }

    #[tokio::test]
    async fn subscribe_sends_the_topic_request_and_events_fan_out_to_every_receiver() {
        let (client, mut server) = pair(4);
        let mut first = client.events();
        let mut second = client.events();
        let subscribe = tokio::spawn({
            let client = client.clone();
            async move { client.subscribe(EventTopic::Links).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("subscribe request");
        assert_eq!(request.msg_type, MessageType::SubLinks);
        reply(&mut server, MessageType::Result, &request.request_id, &HashMap::new()).await;
        subscribe.await.expect("join").expect("subscription acknowledged");

        // Events carry no request correlation; the daemon pushes them as they occur.
        let payload = HashMap::from([
            ("link_id".to_string(), Value::from("0011223344556677")),
            ("status".to_string(), Value::from("active")),
        ]);
        reply(&mut server, MessageType::EventLink, &[0; REQUEST_ID_SIZE], &payload).await;
        let event = first.recv().await.expect("first receiver");
        let mirrored = second.recv().await.expect("second receiver");
        assert_eq!(event.message_type, MessageType::EventLink);
        assert_eq!(event.text("link_id"), "0011223344556677");
        assert_eq!(mirrored.text("status"), "active");
        assert_eq!(event.generation, ConnectionGeneration(7));
        let diagnostics = client.diagnostics();
        assert_eq!(diagnostics.events_received, 1);
        assert_eq!(diagnostics.dropped_responses, 0);
    }

    #[tokio::test]
    async fn events_never_consume_pending_requests_and_unobserved_events_are_counted() {
        let (client, mut server) = pair(4);
        let status = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(MessageType::QueryStatus, HashMap::new(), Duration::from_secs(1))
                    .await
            }
        });
        let request = wire::read_frame_async(&mut server).await.expect("status request");
        // An event arrives before the response; nobody holds a receiver.
        reply(&mut server, MessageType::EventDevice, &request.request_id, &HashMap::new()).await;
        reply(&mut server, MessageType::Result, &request.request_id, &HashMap::new()).await;
        status.await.expect("join").expect("status response still correlates");
        let diagnostics = client.diagnostics();
        assert_eq!(diagnostics.events_received, 1);
        assert_eq!(diagnostics.events_unobserved, 1);
        assert_eq!(diagnostics.completed, 1);
    }

    #[tokio::test]
    async fn event_receivers_end_when_the_connection_closes() {
        let (client, server) = pair(4);
        let mut events = client.events();
        drop(server);
        let outcome = events.recv().await;
        assert!(matches!(outcome, Err(broadcast::error::RecvError::Closed)));
        assert!(!client.is_connected());
    }

    #[tokio::test]
    async fn slow_event_receivers_lag_instead_of_stalling_the_reader() {
        let (stream, mut server) = UnixStream::pair().expect("Unix stream pair");
        let client =
            Client::from_unix_stream_with_capacities(stream, ConnectionGeneration(7), 4, 2);
        let mut events = client.events();
        for _ in 0..4 {
            reply(&mut server, MessageType::EventRoute, &[0; REQUEST_ID_SIZE], &HashMap::new())
                .await;
        }
        // A request still round-trips while the receiver is behind.
        let ping = tokio::spawn({
            let client = client.clone();
            async move { client.ping().await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("ping request");
        reply(&mut server, MessageType::Pong, &request.request_id, &HashMap::new()).await;
        ping.await.expect("join").expect("ping");
        assert!(matches!(events.recv().await, Err(broadcast::error::RecvError::Lagged(_))));
        assert_eq!(client.diagnostics().events_received, 4);
    }

    #[tokio::test]
    async fn message_page_retries_a_stale_cursor_from_the_start() {
        let (client, mut server) = pair(4);
        let page = tokio::spawn({
            let client = client.clone();
            async move { client.message_page("peer", Some("old-cursor"), 50).await }
        });
        let stale = wire::read_frame_async(&mut server).await.expect("first page request");
        assert_eq!(stale.payload.get("cursor").and_then(Value::as_str), Some("old-cursor"));
        let error = HashMap::from([
            ("kind".into(), Value::from("invalid_request")),
            ("code".into(), Value::from("cursor_stale")),
            ("message".into(), Value::from("cursor is stale")),
        ]);
        reply(&mut server, MessageType::Error, &stale.request_id, &error).await;
        let retry = wire::read_frame_async(&mut server).await.expect("retried page request");
        assert!(!retry.payload.contains_key("cursor"), "retry must start from the beginning");
        let payload = HashMap::from([
            ("messages".into(), Value::Array(Vec::new())),
            ("next_cursor".into(), Value::from("next")),
        ]);
        reply(&mut server, MessageType::Result, &retry.request_id, &payload).await;
        let Paged { page, reset, pagination_supported } = page.await.expect("join").expect("page");
        assert!(reset);
        assert!(pagination_supported);
        assert!(page.messages.is_empty());
        assert_eq!(page.next_cursor.as_deref(), Some("next"));

        // A daemon that never spells next_cursor does not paginate.
        let page = tokio::spawn({
            let client = client.clone();
            async move { client.message_page("peer", None, 50).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("legacy page request");
        let payload = HashMap::from([("messages".into(), Value::Array(Vec::new()))]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;
        let legacy = page.await.expect("join").expect("legacy page");
        assert!(!legacy.pagination_supported);
        assert!(!legacy.reset);
        assert!(legacy.page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn mark_read_accepts_the_legacy_count_response() {
        let (client, mut server) = pair(4);
        let outcome = tokio::spawn({
            let client = client.clone();
            async move { client.mark_read("peer-a").await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("mark-read request");
        assert_eq!(request.msg_type, MessageType::CmdMarkRead);
        reply(
            &mut server,
            MessageType::Result,
            &request.request_id,
            &HashMap::from([("count".into(), Value::from(3_u64))]),
        )
        .await;
        let outcome = outcome.await.expect("join").expect("outcome");
        assert_eq!(outcome.affected_count, 3);
        assert_eq!(outcome.target_id, "peer-a");
        assert_eq!(outcome.disposition, MessagingDisposition::Applied);
    }

    #[tokio::test]
    async fn terminal_open_requires_a_session_id_and_pages_decode_pairs() {
        let (client, mut server) = pair(4);
        let open = tokio::spawn({
            let client = client.clone();
            async move { client.terminal_open("dest", 24, 80).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("terminal open");
        assert_eq!(request.payload.get("rows").and_then(Value::as_u64), Some(24));
        reply(&mut server, MessageType::Result, &request.request_id, &HashMap::new()).await;
        assert!(matches!(open.await.expect("join"), Err(ClientError::Protocol { .. })));

        let pages = tokio::spawn({
            let client = client.clone();
            async move { client.list_pages("host").await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("list pages");
        let entry = Value::Map(vec![
            (Value::from("path"), Value::from("/page/index.mu")),
            (Value::from("host_hash"), Value::from("abcd")),
        ]);
        reply(
            &mut server,
            MessageType::Result,
            &request.request_id,
            &HashMap::from([("pages".into(), Value::Array(vec![entry]))]),
        )
        .await;
        let pages = pages.await.expect("join").expect("pages");
        assert_eq!(pages, vec![("/page/index.mu".to_string(), "abcd".to_string())]);
    }

    #[tokio::test]
    async fn rejects_stale_generation_before_matching_response() {
        let (client, mut server) = pair(2);
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client.request(MessageType::Ping, HashMap::new(), Duration::from_secs(1)).await
            }
        });
        let frame = wire::read_frame_async(&mut server).await.expect("request");
        let mut stale_id = frame.request_id;
        stale_id[..8].copy_from_slice(&8_u64.to_le_bytes());
        reply(&mut server, MessageType::Result, &stale_id, &HashMap::new()).await;
        reply(&mut server, MessageType::Result, &frame.request_id, &HashMap::new()).await;

        request.await.expect("request task").expect("current response");
        assert_eq!(client.diagnostics().stale_responses, 1);
    }

    #[tokio::test]
    async fn preserves_structured_remote_errors() {
        let (client, mut server) = pair(2);
        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .request(MessageType::QueryMessages, HashMap::new(), Duration::from_secs(1))
                    .await
            }
        });
        let frame = wire::read_frame_async(&mut server).await.expect("request");
        let expected = IpcError::Denied { capability: "messaging.history.read".into() };
        let payload = HashMap::from([
            ("kind".into(), Value::from("denied")),
            ("code".into(), Value::from("denied")),
            ("message".into(), Value::from(expected.to_string())),
            (
                "typed_error".into(),
                rmpv::ext::to_value(expected.clone()).expect("typed error value"),
            ),
        ]);
        reply(&mut server, MessageType::Error, &frame.request_id, &payload).await;

        match request.await.expect("request task") {
            Err(ClientError::Remote(error)) => assert_eq!(error, expected),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    fn negotiable_status(generation: u64, capability_version: u16) -> DaemonStatusInfo {
        let mut status = DaemonStatusInfo::default();
        status.daemon_version = "contract-test".into();
        status.connection_generation = Some(generation);
        let mut capabilities = ActiveCapabilitiesInfo::default();
        capabilities.version = capability_version;
        capabilities.generation = Some(generation);
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        capabilities.authorized_operations = vec!["chat.send".into()];
        status.active_capabilities = Some(capabilities);
        status
    }

    async fn answer_status(server: &mut UnixStream, status: &DaemonStatusInfo) {
        let request = wire::read_frame_async(server).await.expect("status request");
        assert_eq!(request.msg_type, MessageType::QueryStatus);
        reply(server, MessageType::Result, &request.request_id, &typed_payload(status)).await;
    }

    async fn answer_negotiation(server: &mut UnixStream, status: &DaemonStatusInfo) {
        let ping = wire::read_frame_async(server).await.expect("ping");
        assert_eq!(ping.msg_type, MessageType::Ping);
        reply(server, MessageType::Pong, &ping.request_id, &HashMap::new()).await;
        answer_status(server, status).await;
    }

    #[tokio::test]
    async fn negotiate_records_the_daemon_generation_and_capabilities() {
        let (client, mut server) = pair(2);
        assert_eq!(client.daemon_generation(), None);
        let negotiate = tokio::spawn({
            let client = client.clone();
            async move { client.negotiate(DEFAULT_DEADLINE).await }
        });
        let status = negotiable_status(42, ACTIVE_CAPABILITIES_VERSION);
        answer_negotiation(&mut server, &status).await;
        let negotiation = negotiate.await.expect("task").expect("negotiated");
        assert_eq!(negotiation.daemon_generation, 42);
        assert_eq!(negotiation.capabilities, status.active_capabilities);
        assert_eq!(negotiation.status, status);
        assert_eq!(client.daemon_generation(), Some(42));
    }

    #[tokio::test]
    async fn negotiate_rejects_missing_generations_and_foreign_capability_versions() {
        let (client, mut server) = pair(2);
        let negotiate = tokio::spawn({
            let client = client.clone();
            async move { client.negotiate(DEFAULT_DEADLINE).await }
        });
        let mut status = negotiable_status(0, ACTIVE_CAPABILITIES_VERSION);
        status.connection_generation = None;
        answer_negotiation(&mut server, &status).await;
        assert!(matches!(
            negotiate.await.expect("task"),
            Err(ClientError::Protocol { message }) if message.contains("connection generation")
        ));
        assert_eq!(client.daemon_generation(), None);

        let negotiate = tokio::spawn({
            let client = client.clone();
            async move { client.negotiate(DEFAULT_DEADLINE).await }
        });
        answer_negotiation(&mut server, &negotiable_status(5, ACTIVE_CAPABILITIES_VERSION + 1))
            .await;
        assert!(matches!(
            negotiate.await.expect("task"),
            Err(ClientError::Incompatible { message }) if message.contains("capability contract")
        ));
    }

    #[tokio::test]
    async fn compatibility_watch_reports_changes_then_loss() {
        let (client, mut server) = pair(2);
        let baseline = Negotiation::from_status(negotiable_status(42, ACTIVE_CAPABILITIES_VERSION))
            .expect("baseline");
        let mut watch =
            client.watch_compatibility(&baseline, Duration::from_millis(5), DEFAULT_DEADLINE);

        let unchanged = negotiable_status(42, ACTIVE_CAPABILITIES_VERSION);
        answer_status(&mut server, &unchanged).await;
        assert_eq!(watch.recv().await, Some(CompatibilityEvent::Status(unchanged)));

        let restarted = negotiable_status(43, ACTIVE_CAPABILITIES_VERSION);
        answer_status(&mut server, &restarted).await;
        assert_eq!(
            watch.recv().await,
            Some(CompatibilityEvent::GenerationChanged { previous: 42, current: 43 })
        );
        assert_eq!(watch.recv().await, Some(CompatibilityEvent::Status(restarted.clone())));
        assert_eq!(client.daemon_generation(), Some(43));

        let mut degraded = restarted.clone();
        degraded.active_capabilities = None;
        answer_status(&mut server, &degraded).await;
        assert_eq!(
            watch.recv().await,
            Some(CompatibilityEvent::CapabilitiesChanged {
                previous: restarted.active_capabilities.clone(),
                current: None,
            })
        );
        assert_eq!(watch.recv().await, Some(CompatibilityEvent::Status(degraded)));

        drop(server);
        assert!(matches!(watch.recv().await, Some(CompatibilityEvent::Lost { .. })));
        assert_eq!(watch.recv().await, None);
    }

    #[test]
    fn route_events_rebuild_the_nested_record_from_the_flat_wire_spelling() {
        let payload = HashMap::from([
            ("kind".to_string(), Value::from("lost")),
            ("destination_hash".to_string(), Value::from("dest")),
            ("hops".to_string(), Value::from(2_u64)),
            ("next_hop".to_string(), Value::from("relay")),
            ("interface".to_string(), Value::from("iface")),
            ("expires".to_string(), Value::from(500_i64)),
            ("loss_reason".to_string(), Value::from("interface_unavailable")),
            ("source".to_string(), Value::from("transport_path_table")),
            ("observed_at".to_string(), Value::from(100_i64)),
            ("connection_generation".to_string(), Value::from(7_u64)),
            ("stale".to_string(), Value::from(true)),
            ("correlation_id".to_string(), Value::from("corr")),
            ("route_observed_at".to_string(), Value::from(90_i64)),
            ("route_connection_generation".to_string(), Value::from(7_u64)),
            ("route_age_secs".to_string(), Value::from(10_u64)),
            ("route_stale".to_string(), Value::from(false)),
        ]);
        let frame = EventFrame {
            message_type: MessageType::EventRoute,
            payload,
            generation: ConnectionGeneration(1),
        };
        let event = frame.route_event().expect("route event");
        assert_eq!(event.kind, RouteEventKind::Lost);
        assert_eq!(event.loss_reason, Some(RouteLossReason::InterfaceUnavailable));
        assert_eq!(event.route.destination_hash, "dest");
        assert_eq!(event.route.hops, Some(2));
        assert_eq!(event.route.next_hop.as_deref(), Some("relay"));
        assert_eq!(event.route.expires, Some(500));
        assert_eq!(event.route.observation.source, ObservationSource::TransportPathTable);
        assert_eq!(event.route.observation.observed_at, Some(90));
        assert_eq!(event.route.observation.age_secs, Some(10));
        assert!(!event.route.observation.stale);
        assert_eq!(event.observation.source, ObservationSource::TransportPathTable);
        assert_eq!(event.observation.observed_at, Some(100));
        assert_eq!(event.observation.connection_generation, Some(7));
        assert!(event.observation.stale);
        assert_eq!(event.observation.correlation_id.as_deref(), Some("corr"));

        let other = EventFrame {
            message_type: MessageType::EventDevice,
            payload: HashMap::new(),
            generation: ConnectionGeneration(1),
        };
        assert!(other.route_event().is_err());
    }

    #[tokio::test]
    async fn decodes_propagation_snapshots_and_page_inventories() {
        let (client, mut server) = pair(2);
        let mut query = PropagationQuery::default();
        query.limit = 25;
        query.cursor = Some("c1".into());
        let snapshot_task = tokio::spawn({
            let client = client.clone();
            async move { client.propagation_snapshot(&query).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("propagation request");
        assert_eq!(request.msg_type, MessageType::QueryPropagation);
        assert_eq!(request.payload.get("limit").and_then(Value::as_u64), Some(25));
        assert_eq!(request.payload.get("cursor").and_then(Value::as_str), Some("c1"));
        // The daemon spells the snapshot field by field and omits empty lists.
        let entry = HashMap::from([
            ("id".to_string(), Value::from("m1")),
            ("destination_hash".to_string(), Value::from("dest")),
            ("received_at".to_string(), Value::from(10_i64)),
            ("expires_at".to_string(), Value::from(20_i64)),
            ("size_bytes".to_string(), Value::from(64_u64)),
            ("state".to_string(), Value::from("queued")),
        ]);
        let payload = HashMap::from([
            ("enabled".to_string(), Value::from(true)),
            ("queue_count".to_string(), Value::from(1_u64)),
            ("queue_size_bytes".to_string(), Value::from(64_u64)),
            ("expiry_secs".to_string(), Value::from(3600_u64)),
            (
                "queue".to_string(),
                Value::Array(vec![Value::Map(
                    entry.into_iter().map(|(k, v)| (Value::from(k), v)).collect(),
                )]),
            ),
            ("peer_state_supported".to_string(), Value::from(false)),
            ("sync_state_supported".to_string(), Value::from(false)),
            ("next_cursor".to_string(), Value::from("c2")),
        ]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;
        let snapshot = snapshot_task.await.expect("task").expect("snapshot");
        assert!(snapshot.enabled);
        assert_eq!(snapshot.queue.len(), 1);
        assert_eq!(snapshot.queue[0].id, "m1");
        assert_eq!(snapshot.queue[0].state, "queued");
        assert_eq!(snapshot.next_cursor.as_deref(), Some("c2"));
        assert!(snapshot.peers.is_empty());

        let pages_task = tokio::spawn({
            let client = client.clone();
            async move { client.page_inventory("local", None).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("pages request");
        assert_eq!(request.msg_type, MessageType::CmdPageListSites);
        assert_eq!(request.payload.get("host").and_then(Value::as_str), Some("local"));
        let mut page = PageInfo::default();
        page.path = "/index".into();
        page.host_hash = "host".into();
        page.title = Some("Index".into());
        let payload = HashMap::from([("pages".to_string(), typed_value(&vec![page.clone()]))]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;
        assert_eq!(pages_task.await.expect("task").expect("pages"), vec![page]);
    }

    #[tokio::test]
    async fn decodes_status_and_capabilities_as_canonical_records() {
        let (client, mut server) = pair(2);
        let query = tokio::spawn({
            let client = client.clone();
            async move { client.status().await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("status request");
        assert_eq!(request.msg_type, MessageType::QueryStatus);

        let mut status = DaemonStatusInfo::default();
        status.daemon_version = "contract-test".into();
        status.connection_generation = Some(42);
        let mut capabilities = styrene_ipc::types::ActiveCapabilitiesInfo::default();
        capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        capabilities.generation = Some(42);
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        capabilities.authorized_operations = vec!["chat.send".into()];
        status.active_capabilities = Some(capabilities);
        reply(&mut server, MessageType::Result, &request.request_id, &typed_payload(&status)).await;

        assert_eq!(query.await.expect("status task").expect("typed status"), status);
    }

    #[tokio::test]
    async fn encodes_send_request_and_requires_authoritative_outcome() {
        let (client, mut server) = pair(2);
        let mut send = SendChatRequest::default();
        send.peer_hash = "22".repeat(16);
        send.content = "hello".into();
        send.title = Some("greeting".into());
        send.delivery_method = Some("direct".into());
        let query = tokio::spawn({
            let client = client.clone();
            let send = send.clone();
            async move { client.send_chat_outcome(&send).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("send request");
        assert_eq!(request.msg_type, MessageType::CmdSendChatOutcome);
        assert_eq!(request.payload["peer_hash"].as_str(), Some(send.peer_hash.as_str()));
        assert_eq!(request.payload["content"].as_str(), Some("hello"));
        assert_eq!(request.payload["title"].as_str(), Some("greeting"));
        assert_eq!(request.payload["delivery_method"].as_str(), Some("direct"));

        let mut outcome = SendChatOutcome::default();
        outcome.message_id = "message-1".into();
        outcome.message.id = outcome.message_id.clone();
        outcome.message.projection_complete = true;
        let payload = HashMap::from([("outcome".into(), typed_value(&outcome))]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;

        assert_eq!(query.await.expect("send task").expect("send outcome"), outcome);
    }

    #[tokio::test]
    async fn decodes_legacy_path_and_tunnel_maps_as_canonical_records() {
        let (client, mut server) = pair(2);
        let path_query = tokio::spawn({
            let client = client.clone();
            async move { client.path_info("aa11aa11aa11aa11").await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("path request");
        assert_eq!(request.msg_type, MessageType::QueryPathInfo);
        assert_eq!(request.payload["destination_hash"].as_str(), Some("aa11aa11aa11aa11"));
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from("aa11aa11aa11aa11")),
            ("found".into(), Value::from(true)),
            ("hops".into(), Value::from(3)),
            ("interface".into(), Value::from("rnode")),
        ]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;
        let path = path_query.await.expect("path task").expect("path response").expect("path");
        assert_eq!(path.hops, Some(3));
        assert_eq!(path.interface.as_deref(), Some("rnode"));

        let tunnel_query = tokio::spawn({
            let client = client.clone();
            async move { client.list_tunnels().await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("tunnel request");
        assert_eq!(request.msg_type, MessageType::QueryTunnels);
        let tunnel = Value::Map(vec![
            (Value::from("peer_hash"), Value::from("bb22bb22bb22bb22")),
            (Value::from("backend"), Value::from("wireguard")),
            (Value::from("state"), Value::from("established")),
            (Value::from("remote_endpoint"), Value::from("")),
            (Value::from("established_at"), Value::from(0)),
        ]);
        let payload = HashMap::from([("tunnels".into(), Value::Array(vec![tunnel]))]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;
        let tunnels = tunnel_query.await.expect("tunnel task").expect("tunnel response");
        assert_eq!(tunnels[0].remote_endpoint, None);
        assert_eq!(tunnels[0].established_at, None);
    }

    #[tokio::test]
    async fn preserves_fleet_aliases_and_extra_status_fields() {
        let (client, mut server) = pair(2);
        let query = tokio::spawn({
            let client = client.clone();
            async move { client.device_status("cc33cc33cc33cc33", 20).await }
        });
        let request = wire::read_frame_async(&mut server).await.expect("fleet request");
        assert_eq!(request.msg_type, MessageType::CmdDeviceStatus);
        assert_eq!(request.payload["timeout"].as_u64(), Some(20));
        let payload = HashMap::from([
            ("destination_hash".into(), Value::from("cc33cc33cc33cc33")),
            ("uptime".into(), Value::from(45)),
            ("version".into(), Value::from("0.3.0")),
            ("battery_percent".into(), Value::from(80)),
        ]);
        reply(&mut server, MessageType::Result, &request.request_id, &payload).await;

        let status = query.await.expect("fleet task").expect("fleet response");
        assert_eq!(status.daemon_version.as_deref(), Some("0.3.0"));
        assert_eq!(status.extra["battery_percent"], serde_json::json!(80));
    }
}
