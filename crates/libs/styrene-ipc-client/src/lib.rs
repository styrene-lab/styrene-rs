//! Bounded concurrent client transport for Styrene local IPC.
//!
//! This crate owns request correlation and transport failure semantics. Typed
//! daemon operations are added here as they migrate out of frontend crates.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rmpv::Value;
use styrene_ipc::IpcError;
use styrene_ipc_wire::{self as wire, Frame, MessageType, REQUEST_ID_SIZE};
use thiserror::Error;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio::time::timeout;

pub const DEFAULT_CAPACITY: usize = 32;

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
}
