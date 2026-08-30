//! Bounded concurrent client transport for Styrene local IPC.
//!
//! This crate owns request correlation and transport failure semantics. Typed
//! daemon operations are added here as they migrate out of frontend crates.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rmpv::Value;
use serde::de::DeserializeOwned;
use styrene_ipc::IpcError;
use styrene_ipc::types::{
    ConfigApplyResult, ConfigSnapshot, ConversationInfo, DaemonStatusInfo, DeviceInfo, ExecResult,
    IdentityInfo, MessageInfo, PathInfo, RebootResult, RemoteStatusInfo, SendChatOutcome,
    SendChatRequest, StandardPropagationSnapshot, TunnelInfo, TunnelOperationInfo,
};
use styrene_ipc_wire::{self as wire, Frame, MessageType, REQUEST_ID_SIZE};
use thiserror::Error;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
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
            Self::Protocol { .. } => false,
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
}

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
}

#[derive(Clone)]
pub struct Client {
    generation: ConnectionGeneration,
    outbound: mpsc::Sender<Request>,
    pending: Arc<Mutex<HashMap<[u8; REQUEST_ID_SIZE], PendingRequest>>>,
    capacity: Arc<Semaphore>,
    next_id: Arc<AtomicU64>,
    metrics: Arc<Metrics>,
    connected: Arc<AtomicBool>,
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
        let capacity = capacity.max(1);
        let (outbound, requests) = mpsc::channel(capacity);
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

    #[must_use]
    pub fn generation(&self) -> ConnectionGeneration {
        self.generation
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

    pub async fn status(&self) -> Result<DaemonStatusInfo, ClientError> {
        let frame =
            self.request(MessageType::QueryStatus, HashMap::new(), DEFAULT_DEADLINE).await?;
        decode_map(&frame.payload, "status")
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

fn decode_map<T: DeserializeOwned>(
    payload: &HashMap<String, Value>,
    context: &str,
) -> Result<T, ClientError> {
    decode_value(
        Value::Map(
            payload.iter().map(|(key, value)| (Value::from(key.as_str()), value.clone())).collect(),
        ),
        context,
    )
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

fn decode_value<T: DeserializeOwned>(value: Value, context: &str) -> Result<T, ClientError> {
    let json: serde_json::Value = rmpv::ext::from_value(value).map_err(|error| {
        ClientError::Protocol { message: format!("invalid {context} value: {error}") }
    })?;
    serde_json::from_value(json)
        .map_err(|error| ClientError::Protocol { message: format!("invalid {context}: {error}") })
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
