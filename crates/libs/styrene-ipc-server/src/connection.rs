//! Per-client connection handling.
//!
//! Each connected client gets a spawned task that reads frames, dispatches
//! to the [`Daemon`] trait, and writes responses. Subscription state is
//! tracked per-connection.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinSet;

use styrene_ipc::traits::Daemon;
use styrene_ipc::types::DaemonEvent;

use crate::dispatch;
use crate::wire::{self, MessageType, WireError, REQUEST_ID_SIZE};

const MAX_IN_FLIGHT_REQUESTS: usize = 64;

struct OwnerCleanupGuard {
    daemon: Arc<dyn Daemon>,
    owner: u64,
    cancellation: tokio_util::sync::CancellationToken,
    armed: bool,
}

impl OwnerCleanupGuard {
    fn new(daemon: Arc<dyn Daemon>, owner: u64) -> Self {
        Self {
            daemon,
            owner,
            cancellation: tokio_util::sync::CancellationToken::new(),
            armed: owner != 0,
        }
    }

    async fn cleanup(&mut self) {
        if !self.armed {
            return;
        }
        self.cancellation.cancel();
        match self.daemon.cleanup_page_owner(self.owner).await {
            Ok(()) => self.armed = false,
            Err(error) => {
                log::warn!("IPC page-owner cleanup remains supervised after failure: {error}");
            }
        }
    }
}

impl Drop for OwnerCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancellation.cancel();
        let daemon = Arc::clone(&self.daemon);
        let owner = self.owner;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = daemon.cleanup_page_owner(owner).await {
                    log::warn!("IPC page-owner cleanup failed after connection task exit: {error}");
                }
            });
        } else {
            log::error!("IPC page-owner cleanup could not be scheduled for owner {owner}");
        }
    }
}

/// Subscription topics a client can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubTopic {
    Devices,
    Messages,
    Activity,
    Links,
    Routes,
    Requests,
    NetworkOperations,
    Resources,
}

/// Run a single client connection to completion.
///
/// Spawns a writer task for event push, then reads frames in a loop
/// and dispatches each to the daemon. Returns when the client disconnects
/// or an unrecoverable error occurs.
pub async fn handle_client(
    daemon: Arc<dyn Daemon>,
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    event_rx: broadcast::Receiver<DaemonEvent>,
) {
    handle_client_with_generation(daemon, read_half, write_half, event_rx, 0).await;
}

/// Run a client connection with server-assigned correlation metadata.
pub async fn handle_client_with_generation(
    daemon: Arc<dyn Daemon>,
    read_half: OwnedReadHalf,
    write_half: OwnedWriteHalf,
    event_rx: broadcast::Receiver<DaemonEvent>,
    connection_generation: u64,
) {
    let subscriptions = Arc::new(Mutex::new(HashSet::<SubTopic>::new()));
    let request_rx = daemon.subscribe_requests().await.ok();
    let mut cleanup = OwnerCleanupGuard::new(Arc::clone(&daemon), connection_generation);

    // Channel for sending response/event frames to the writer task
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(256);

    // Spawn writer task
    let subs_for_writer = subscriptions.clone();
    let mut writer_handle = tokio::spawn(writer_loop(
        write_half,
        frame_rx,
        event_rx,
        request_rx,
        subs_for_writer,
        connection_generation,
    ));

    // Keep frame decoding in one uninterrupted task: async reads are not generally
    // cancellation-safe after consuming part of a frame.
    let (incoming_tx, mut incoming_rx) = mpsc::channel(64);
    let mut reader_handle = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(read_half);
        loop {
            let frame = wire::read_frame_async(&mut reader).await;
            let terminal = frame.is_err();
            if incoming_tx.send(frame).await.is_err() || terminal {
                break;
            }
        }
    });

    // Dispatch requests independently so EOF and writer failure can cancel slow daemon calls.
    let mut dispatches = JoinSet::new();
    loop {
        tokio::select! {
            writer = &mut writer_handle => {
                if let Err(error) = writer {
                    log::warn!("IPC writer task stopped: {error}");
                }
                break;
            }
            completed = dispatches.join_next(), if !dispatches.is_empty() => {
                if let Some(Err(error)) = completed {
                    log::warn!("IPC dispatch task stopped: {error}");
                    if error.is_panic() {
                        break;
                    }
                }
            }
            reader = &mut reader_handle => {
                if let Err(error) = reader {
                    log::warn!("IPC reader task stopped: {error}");
                }
                break;
            }
            frame = incoming_rx.recv() => match frame {
            Some(Ok(frame)) => {
                log::info!(
                    "IPC frame received type={:?} request_id={:?}",
                    frame.msg_type,
                    frame.request_id
                );
                if dispatches.len() >= MAX_IN_FLIGHT_REQUESTS {
                    let payload = error_payload("too many in-flight IPC requests".into());
                    if let Some(bytes) =
                        encode_response(MessageType::Error, &frame.request_id, &payload)
                    {
                        if frame_tx.send(bytes).await.is_err() {
                            break;
                        }
                    }
                    continue;
                }
                let daemon = Arc::clone(&daemon);
                let subscriptions = Arc::clone(&subscriptions);
                let frame_tx = frame_tx.clone();
                let cancellation = cleanup.cancellation.clone();
                dispatches.spawn(async move {
                    let response_bytes = tokio::select! {
                        biased;
                        () = cancellation.cancelled() => None,
                        response = handle_frame(
                            &daemon,
                            frame.msg_type,
                            &frame.request_id,
                            frame.payload,
                            &subscriptions,
                            connection_generation,
                        ) => response,
                    };
                    if let Some(bytes) = response_bytes {
                        log::info!(
                            "IPC response encoded request_id={:?} bytes={}",
                            frame.request_id,
                            bytes.len()
                        );
                        if frame_tx.send(bytes).await.is_err() {
                            log::warn!("IPC response queue closed request_id={:?}", frame.request_id);
                        }
                    }
                });
            }
            Some(Err(WireError::Io(ref e))) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                log::debug!("client disconnected (EOF)");
                break;
            }
            Some(Err(e)) => {
                log::warn!("client frame error: {e}");
                break;
            }
            None => break,
        }}
    }

    cleanup.cancellation.cancel();
    dispatches.abort_all();
    while dispatches.join_next().await.is_some() {}
    cleanup.cleanup().await;
    drop(frame_tx);
    if !reader_handle.is_finished() {
        reader_handle.abort();
        let _ = reader_handle.await;
    }
    if !writer_handle.is_finished() {
        writer_handle.abort();
        let _ = writer_handle.await;
    }
}

/// Handle a single frame, returning optional response bytes.
async fn handle_frame(
    daemon: &Arc<dyn Daemon>,
    msg_type: MessageType,
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: HashMap<String, rmpv::Value>,
    subscriptions: &Arc<Mutex<HashSet<SubTopic>>>,
    connection_generation: u64,
) -> Option<Vec<u8>> {
    match msg_type {
        // Keepalive
        MessageType::Ping => {
            let empty = HashMap::new();
            encode_response(MessageType::Pong, request_id, &empty)
        }

        // Subscriptions
        MessageType::SubDevices => {
            subscriptions.lock().await.insert(SubTopic::Devices);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubMessages => {
            subscriptions.lock().await.insert(SubTopic::Messages);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubActivity => {
            subscriptions.lock().await.insert(SubTopic::Activity);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubLinks => {
            subscriptions.lock().await.insert(SubTopic::Links);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubRoutes => {
            subscriptions.lock().await.insert(SubTopic::Routes);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubRequests => {
            subscriptions.lock().await.insert(SubTopic::Requests);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubNetworkOperations => {
            subscriptions.lock().await.insert(SubTopic::NetworkOperations);
            some_result(request_id, HashMap::new())
        }
        MessageType::SubResources => {
            subscriptions.lock().await.insert(SubTopic::Resources);
            some_result(request_id, HashMap::new())
        }
        MessageType::Unsub => {
            // Unsubscribe from the topic specified in payload, or all
            let mut subs = subscriptions.lock().await;
            if let Some(topic) = payload.get("topic").and_then(|v| v.as_str()) {
                match topic {
                    "devices" => {
                        subs.remove(&SubTopic::Devices);
                    }
                    "messages" => {
                        subs.remove(&SubTopic::Messages);
                    }
                    "activity" => {
                        subs.remove(&SubTopic::Activity);
                    }
                    "links" => {
                        subs.remove(&SubTopic::Links);
                    }
                    "routes" => {
                        subs.remove(&SubTopic::Routes);
                    }
                    "requests" => {
                        subs.remove(&SubTopic::Requests);
                    }
                    "network_operations" => {
                        subs.remove(&SubTopic::NetworkOperations);
                    }
                    "resources" => {
                        subs.remove(&SubTopic::Resources);
                    }
                    _ => {}
                }
            } else {
                subs.clear();
            }
            some_result(request_id, HashMap::new())
        }

        // Dispatch to daemon
        _ if msg_type.is_request() => {
            let result =
                dispatch::dispatch_for_connection(daemon, msg_type, payload, connection_generation)
                    .await;
            match result {
                Ok(resp_payload) => encode_response(MessageType::Result, request_id, &resp_payload),
                Err(err_msg) => {
                    let p = error_payload(err_msg);
                    encode_response(MessageType::Error, request_id, &p)
                }
            }
        }

        // Responses and events from client are unexpected — ignore
        _ => None,
    }
}

fn error_payload(message: String) -> HashMap<String, rmpv::Value> {
    if let Ok(error) = serde_json::from_str::<styrene_ipc::IpcError>(&message) {
        let (kind, code) = classify_ipc_error(&error);
        let display = error.to_string();
        return HashMap::from([
            ("error".into(), rmpv::Value::from(display.as_str())),
            ("message".into(), rmpv::Value::from(display)),
            ("kind".into(), rmpv::Value::from(kind)),
            ("code".into(), rmpv::Value::from(code)),
        ]);
    }
    let (kind, code) = classify_error(&message);
    HashMap::from([
        ("error".into(), rmpv::Value::from(message.as_str())),
        ("message".into(), rmpv::Value::from(message)),
        ("kind".into(), rmpv::Value::from(kind)),
        ("code".into(), rmpv::Value::from(code)),
    ])
}

fn classify_ipc_error(error: &styrene_ipc::IpcError) -> (&'static str, &'static str) {
    use styrene_ipc::IpcError;
    match error {
        IpcError::NotImplemented { .. } => ("not_implemented", "not_implemented"),
        IpcError::Unavailable { .. } => ("unavailable", "unavailable"),
        IpcError::Timeout { .. } => ("timeout", "timeout"),
        IpcError::InvalidRequest { .. } => ("invalid_request", "invalid_request"),
        IpcError::NotFound { .. } => ("not_found", "not_found"),
        IpcError::Conflict { .. } => ("conflict", "conflict"),
        IpcError::Denied { .. } => ("denied", "denied"),
        IpcError::Internal { .. } => ("internal", "internal"),
        IpcError::Transport { .. } => ("transport", "transport"),
        _ => ("internal", "internal"),
    }
}

fn classify_error(message: &str) -> (&'static str, &'static str) {
    if message == "conflict: cursor_stale" || message == "cursor_stale" {
        return ("conflict", "cursor_stale");
    }
    for (prefix, kind) in [
        ("not implemented:", "not_implemented"),
        ("unavailable:", "unavailable"),
        ("timeout:", "timeout"),
        ("invalid request:", "invalid_request"),
        ("not found:", "not_found"),
        ("conflict:", "conflict"),
        ("denied:", "denied"),
        ("internal error:", "internal"),
        ("transport error:", "transport"),
    ] {
        if message.starts_with(prefix) {
            return (kind, kind);
        }
    }
    ("internal", "internal")
}

fn some_result(
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: HashMap<String, rmpv::Value>,
) -> Option<Vec<u8>> {
    encode_response(MessageType::Result, request_id, &payload)
}

fn encode_response(
    message_type: MessageType,
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: &HashMap<String, rmpv::Value>,
) -> Option<Vec<u8>> {
    match wire::encode_frame(message_type, request_id, payload) {
        Ok(frame) => Some(frame),
        Err(error) if message_type != MessageType::Error => {
            let error_payload =
                error_payload(format!("IPC response could not be represented: {error}"));
            match wire::encode_frame(MessageType::Error, request_id, &error_payload) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    log::error!("failed to encode bounded IPC error response: {error}");
                    None
                }
            }
        }
        Err(error) => {
            log::error!("failed to encode IPC error response: {error}");
            None
        }
    }
}

/// Writer loop: sends response frames and pushes subscription events.
async fn writer_loop(
    mut writer: OwnedWriteHalf,
    mut frame_rx: mpsc::Receiver<Vec<u8>>,
    mut event_rx: broadcast::Receiver<DaemonEvent>,
    mut request_rx: Option<broadcast::Receiver<DaemonEvent>>,
    subscriptions: Arc<Mutex<HashSet<SubTopic>>>,
    connection_generation: u64,
) {
    loop {
        tokio::select! {
            // Response frames from the handler
            frame = frame_rx.recv() => {
                match frame {
                    Some(bytes) => {
                        if writer.write_all(&bytes).await.is_err() {
                            break;
                        }
                        if writer.flush().await.is_err() {
                            break;
                        }
                    }
                    None => break, // Channel closed
                }
            }

            // Pushed events
            event = event_rx.recv() => {
                match event {
                    Ok(daemon_event) => {
                        if let Some(bytes) = event_to_frame(
                            &daemon_event,
                            &subscriptions,
                            connection_generation,
                        ).await {
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("client event lag: dropped {n} events");
                        if let Some(bytes) = event_to_frame(
                            &DaemonEvent::ReconcileRequired { dropped: n },
                            &subscriptions,
                            connection_generation,
                        ).await {
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            event = recv_optional(&mut request_rx) => {
                match event {
                    Some(Ok(daemon_event)) => {
                        if let Some(bytes) = event_to_frame(
                            &daemon_event,
                            &subscriptions,
                            connection_generation,
                        ).await {
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Some(Err(broadcast::error::RecvError::Lagged(n))) => {
                        log::warn!("client request event lag: dropped {n} events");
                        if let Some(bytes) = event_to_frame(
                            &DaemonEvent::RequestReconcileRequired { dropped: n },
                            &subscriptions,
                            connection_generation,
                        ).await {
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Some(Err(broadcast::error::RecvError::Closed)) => request_rx = None,
                    None => {}
                }
            }
        }
    }
}

async fn recv_optional(
    receiver: &mut Option<broadcast::Receiver<DaemonEvent>>,
) -> Option<Result<DaemonEvent, broadcast::error::RecvError>> {
    match receiver {
        Some(receiver) => Some(receiver.recv().await),
        None => std::future::pending().await,
    }
}

/// Convert a DaemonEvent into a wire frame if the client is subscribed.
async fn event_to_frame(
    event: &DaemonEvent,
    subscriptions: &Arc<Mutex<HashSet<SubTopic>>>,
    connection_generation: u64,
) -> Option<Vec<u8>> {
    let subs = subscriptions.lock().await;

    let (msg_type, topic, payload) = match event {
        DaemonEvent::Device { device } => {
            let mut p = HashMap::new();
            p.insert(
                "destination_hash".to_string(),
                rmpv::Value::from(device.destination_hash.as_str()),
            );
            p.insert("name".to_string(), rmpv::Value::from(device.name.as_str()));
            p.insert("identity_hash".to_string(), rmpv::Value::from(device.identity_hash.as_str()));
            p.insert("device_type".to_string(), rmpv::Value::from(device.device_type.as_str()));
            p.insert("status".to_string(), rmpv::Value::from(device.status.as_str()));
            p.insert(
                "discovered_capabilities".to_string(),
                rmpv::Value::Array(
                    device
                        .discovered_capabilities
                        .iter()
                        .map(|capability| rmpv::Value::from(capability.as_str()))
                        .collect(),
                ),
            );
            if let Some(active) = device.standard_lxmf_propagation_active {
                p.insert("standard_lxmf_propagation_active".to_string(), rmpv::Value::from(active));
            }
            (MessageType::EventDevice, SubTopic::Devices, p)
        }
        DaemonEvent::Message { kind, message } => {
            let kind_str = match kind {
                styrene_ipc::types::MessageEventKind::New => "new",
                styrene_ipc::types::MessageEventKind::StatusChanged => "status_changed",
                styrene_ipc::types::MessageEventKind::Delivered => "delivered",
                styrene_ipc::types::MessageEventKind::Failed => "failed",
                _ => "unknown",
            };
            let mut p = match crate::dispatch::message_info_value(message) {
                rmpv::Value::Map(fields) => fields
                    .into_iter()
                    .filter_map(|(key, value)| key.as_str().map(|key| (key.to_string(), value)))
                    .collect(),
                _ => HashMap::new(),
            };
            p.insert("kind".to_string(), rmpv::Value::from(kind_str));
            p.insert("connection_generation".into(), rmpv::Value::from(connection_generation));
            (MessageType::EventMessage, SubTopic::Messages, p)
        }
        DaemonEvent::TerminalOutput { session_id, data } => {
            let mut p = HashMap::new();
            p.insert("session_id".to_string(), rmpv::Value::from(session_id.as_str()));
            p.insert("data".to_string(), rmpv::Value::from(data.as_slice()));
            (MessageType::EventTerminalOutput, SubTopic::Activity, p)
        }
        DaemonEvent::TerminalStateChange { session_id, .. } => {
            let mut p = HashMap::new();
            p.insert("session_id".to_string(), rmpv::Value::from(session_id.as_str()));
            (MessageType::EventTerminalReady, SubTopic::Activity, p)
        }
        DaemonEvent::TunnelStateChange { peer_hash, state, backend } => {
            let mut p = HashMap::new();
            p.insert("peer_hash".to_string(), rmpv::Value::from(peer_hash.as_str()));
            p.insert("state".to_string(), rmpv::Value::from(state.as_str()));
            p.insert("backend".to_string(), rmpv::Value::from(backend.as_str()));
            (MessageType::EventActivity, SubTopic::Activity, p)
        }
        DaemonEvent::Link { event } => {
            use styrene_ipc::types::{LinkActivity, LinkEventKind, LinkLifecycleReason};

            let mut p = HashMap::new();
            p.insert("link_id".to_string(), rmpv::Value::from(event.link_id.as_str()));
            p.insert("peer_hash".to_string(), rmpv::Value::from(event.peer_hash.as_str()));
            if let Some(name) = &event.peer_name {
                p.insert("peer_name".to_string(), rmpv::Value::from(name.as_str()));
            }
            if let Some(interface) = &event.interface {
                p.insert("interface".to_string(), rmpv::Value::from(interface.as_str()));
            }
            p.insert("status".to_string(), rmpv::Value::from(event.status.as_str()));
            let kind = match event.kind {
                LinkEventKind::Established => "established",
                LinkEventKind::Identified => "identified",
                LinkEventKind::Activity => "activity",
                LinkEventKind::RttUpdated => "rtt_updated",
                LinkEventKind::Teardown => "teardown",
                LinkEventKind::Timeout => "timeout",
                _ => "unknown",
            };
            p.insert("kind".into(), rmpv::Value::from(kind));
            let activity = match event.activity {
                LinkActivity::Active => "active",
                LinkActivity::Historical => "historical",
                _ => "unknown",
            };
            p.insert("activity".into(), rmpv::Value::from(activity));
            if let Some(reason) = event.reason {
                let reason = match reason {
                    LinkLifecycleReason::LocalTeardown => "local_teardown",
                    LinkLifecycleReason::StaleTimeout => "stale_timeout",
                    LinkLifecycleReason::EstablishmentTimeout => "establishment_timeout",
                    LinkLifecycleReason::ChannelTimeout => "channel_timeout",
                    LinkLifecycleReason::SendFailure => "send_failure",
                    _ => "unknown",
                };
                p.insert("reason".into(), rmpv::Value::from(reason));
            }
            p.insert("identified".into(), rmpv::Value::from(event.identified));
            if let Some(identity) = &event.remote_identity_hash {
                p.insert("remote_identity_hash".into(), rmpv::Value::from(identity.as_str()));
            }
            if let Some(rtt) = event.rtt_ms {
                p.insert("rtt_ms".to_string(), rmpv::Value::F64(rtt));
            }
            p.insert("timestamp".to_string(), rmpv::Value::Integer(event.timestamp.into()));
            p.insert("source".into(), rmpv::Value::from(event.observation.source.as_str()));
            if let Some(observed_at) = event.observation.observed_at {
                p.insert("observed_at".into(), rmpv::Value::from(observed_at));
            }
            p.insert("connection_generation".into(), rmpv::Value::from(connection_generation));
            if let Some(age) = event.observation.age_secs {
                p.insert("age_secs".into(), rmpv::Value::from(age));
            }
            if let Some(threshold) = event.observation.freshness_threshold_secs {
                p.insert("freshness_threshold_secs".into(), rmpv::Value::from(threshold));
            }
            p.insert("stale".into(), rmpv::Value::from(event.observation.stale));
            (MessageType::EventLink, SubTopic::Links, p)
        }
        DaemonEvent::Route { event } => {
            use styrene_ipc::types::{RouteEventKind, RouteLossReason};

            let mut p = HashMap::new();
            let kind = match event.kind {
                RouteEventKind::Discovered => "discovered",
                RouteEventKind::Lost => "lost",
                RouteEventKind::Rediscovered => "rediscovered",
                _ => "unknown",
            };
            p.insert("kind".into(), rmpv::Value::from(kind));
            p.insert(
                "destination_hash".into(),
                rmpv::Value::from(event.route.destination_hash.as_str()),
            );
            if let Some(hops) = event.route.hops {
                p.insert("hops".into(), rmpv::Value::from(hops));
            }
            if let Some(next_hop) = &event.route.next_hop {
                p.insert("next_hop".into(), rmpv::Value::from(next_hop.as_str()));
            }
            if let Some(interface) = &event.route.interface {
                p.insert("interface".into(), rmpv::Value::from(interface.as_str()));
            }
            if let Some(expires) = event.route.expires {
                p.insert("expires".into(), rmpv::Value::from(expires));
            }
            if let Some(reason) = event.loss_reason {
                let reason = match reason {
                    RouteLossReason::Expired => "expired",
                    RouteLossReason::InterfaceUnavailable => "interface_unavailable",
                    _ => "unknown",
                };
                p.insert("loss_reason".into(), rmpv::Value::from(reason));
            }
            p.insert("source".into(), rmpv::Value::from(event.observation.source.as_str()));
            if let Some(observed_at) = event.observation.observed_at {
                p.insert("observed_at".into(), rmpv::Value::from(observed_at));
            }
            p.insert("connection_generation".into(), rmpv::Value::from(connection_generation));
            if let Some(age) = event.observation.age_secs {
                p.insert("age_secs".into(), rmpv::Value::from(age));
            }
            if let Some(threshold) = event.observation.freshness_threshold_secs {
                p.insert("freshness_threshold_secs".into(), rmpv::Value::from(threshold));
            }
            p.insert("stale".into(), rmpv::Value::from(event.observation.stale));
            if let Some(correlation_id) = &event.observation.correlation_id {
                p.insert("correlation_id".into(), rmpv::Value::from(correlation_id.as_str()));
            }
            if let Some(route_observed_at) = event.route.observation.observed_at {
                p.insert("route_observed_at".into(), rmpv::Value::from(route_observed_at));
            }
            p.insert(
                "route_connection_generation".into(),
                rmpv::Value::from(connection_generation),
            );
            if let Some(route_age) = event.route.observation.age_secs {
                p.insert("route_age_secs".into(), rmpv::Value::from(route_age));
            }
            if let Some(route_threshold) = event.route.observation.freshness_threshold_secs {
                p.insert(
                    "route_freshness_threshold_secs".into(),
                    rmpv::Value::from(route_threshold),
                );
            }
            p.insert("route_stale".into(), rmpv::Value::from(event.route.observation.stale));
            (MessageType::EventRoute, SubTopic::Routes, p)
        }
        DaemonEvent::Request { event } => {
            let p = dispatch::request_info_payload(event.clone(), connection_generation).ok()?;
            (MessageType::EventRequest, SubTopic::Requests, p)
        }
        DaemonEvent::RequestReconcileRequired { dropped } => {
            let p = HashMap::from([
                ("kind".into(), rmpv::Value::from("reconcile_required")),
                ("dropped".into(), rmpv::Value::from(*dropped)),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
            ]);
            (MessageType::EventRequest, SubTopic::Requests, p)
        }
        DaemonEvent::NetworkOperation { operation } => {
            let p = dispatch::network_operation_payload(operation.clone(), connection_generation)
                .ok()?;
            (MessageType::EventNetworkOperation, SubTopic::NetworkOperations, p)
        }
        DaemonEvent::Resource { transfer } => {
            let p =
                dispatch::resource_info_payload(transfer.clone(), connection_generation).ok()?;
            (MessageType::EventResource, SubTopic::Resources, p)
        }
        DaemonEvent::AttachmentTransfer { transfer } => {
            let value = rmpv::ext::to_value(serde_json::to_value(transfer).ok()?).ok()?;
            let p = HashMap::from([
                ("attachment_transfer".into(), value),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
            ]);
            (MessageType::EventResource, SubTopic::Messages, p)
        }
        DaemonEvent::MessagingOperation { outcome } => {
            let value = rmpv::ext::to_value(serde_json::to_value(outcome).ok()?).ok()?;
            let p = HashMap::from([
                ("outcome".into(), value),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
            ]);
            (MessageType::EventMessagingOperation, SubTopic::Messages, p)
        }
        DaemonEvent::StandardPropagationChanged { observed_at } => {
            let p = HashMap::from([
                ("kind".into(), rmpv::Value::from("standard_propagation_changed")),
                ("observed_at".into(), rmpv::Value::from(*observed_at)),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
            ]);
            (MessageType::EventStandardPropagationChanged, SubTopic::Activity, p)
        }
        DaemonEvent::ReconcileRequired { dropped } => {
            let p = HashMap::from([
                ("dropped".into(), rmpv::Value::from(*dropped)),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
            ]);
            (MessageType::EventReconcileRequired, SubTopic::Activity, p)
        }
        // Future event variants — skip unknown
        _ => return None,
    };

    if !subs.contains(&topic) && !matches!(event, DaemonEvent::ReconcileRequired { .. }) {
        return None;
    }

    // Use zero request_id for pushed events
    let zero_id = [0u8; 16];
    match wire::encode_frame(msg_type, &zero_id, &payload) {
        Ok(frame) => Some(frame),
        Err(error) => {
            log::warn!("IPC event could not be represented; requesting reconciliation: {error}");
            let reconcile = HashMap::from([
                ("dropped".into(), rmpv::Value::from(1_u64)),
                ("connection_generation".into(), rmpv::Value::from(connection_generation)),
                ("reason".into(), rmpv::Value::from(error.to_string())),
            ]);
            match wire::encode_frame(MessageType::EventReconcileRequired, &zero_id, &reconcile) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    log::error!("failed to encode IPC reconciliation event: {error}");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_ipc::types::{
        LinkActivity, LinkEvent, LinkEventKind, LinkLifecycleReason, NetworkOperationInfo,
        NetworkOperationKind, NetworkOperationOutcome, NetworkOperationProgress, ObservationSource,
        RequestObservationInfo, RequestProtocolError, RequestResponseTransfer, RequestState,
        ResourceDirection, ResourceTransferInfo, ResourceTransferState,
    };

    #[tokio::test]
    async fn link_event_serialization_preserves_typed_lifecycle_metadata() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::Links])));
        let mut event = LinkEvent::new("link-1", "peer-1", "closed", Some(12.5));
        event.interface = Some("iface-1".into());
        event.kind = LinkEventKind::Timeout;
        event.activity = LinkActivity::Historical;
        event.reason = Some(LinkLifecycleReason::StaleTimeout);
        event.remote_identity_hash = Some("identity-1".into());
        event.observation.source = ObservationSource::TransportLinkState;
        event.observation.observed_at = Some(100);

        let bytes = event_to_frame(&DaemonEvent::Link { event }, &subscriptions, 9)
            .await
            .expect("subscribed event frame");
        let frame = wire::decode_frame(&bytes).expect("valid event frame");

        assert_eq!(frame.payload["kind"].as_str(), Some("timeout"));
        assert_eq!(frame.payload["activity"].as_str(), Some("historical"));
        assert_eq!(frame.payload["reason"].as_str(), Some("stale_timeout"));
        assert_eq!(frame.payload["interface"].as_str(), Some("iface-1"));
        assert_eq!(frame.payload["source"].as_str(), Some("transport_link_state"));
        assert_eq!(frame.payload["remote_identity_hash"].as_str(), Some("identity-1"));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(9));
    }

    #[tokio::test]
    async fn standard_propagation_event_is_metadata_only_activity_requery_trigger() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::Activity])));
        let bytes = event_to_frame(
            &DaemonEvent::StandardPropagationChanged { observed_at: 123 },
            &subscriptions,
            9,
        )
        .await
        .expect("subscribed standard propagation event");
        let frame = wire::decode_frame(&bytes).expect("valid event frame");
        assert_eq!(frame.msg_type, MessageType::EventStandardPropagationChanged);
        assert_eq!(frame.payload.len(), 3);
        assert_eq!(frame.payload["kind"].as_str(), Some("standard_propagation_changed"));
        assert_eq!(frame.payload["observed_at"].as_i64(), Some(123));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(9));
    }

    #[tokio::test]
    async fn request_event_serialization_preserves_progress_bytes_and_terminal_error() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::Requests])));
        let mut event = RequestObservationInfo::default();
        event.request_id = "11".repeat(16);
        event.path_hash = "22".repeat(16);
        event.link_id = "33".repeat(16);
        event.started_monotonic_ms = 10;
        event.deadline_monotonic_ms = 20;
        event.request_size = 4;
        event.response_size = Some(3);
        event.progress = 1.0;
        event.response_transfer = RequestResponseTransfer::Packet;
        event.response = Some(vec![1, 2, 3]);
        event.request_resource_hash = Some("44".repeat(32));
        event.state = RequestState::MalformedResponse;
        event.protocol_error = Some(RequestProtocolError::MalformedResponse);
        event.observation.source = ObservationSource::TransportRequestState;
        event.observation.correlation_id = Some(event.request_id.clone());

        let bytes = event_to_frame(&DaemonEvent::Request { event }, &subscriptions, 7)
            .await
            .expect("subscribed request event frame");
        let frame = wire::decode_frame(&bytes).expect("valid request event frame");

        assert_eq!(frame.msg_type, MessageType::EventRequest);
        assert_eq!(frame.payload["state"].as_str(), Some("malformed_response"));
        assert_eq!(frame.payload["protocol_error"].as_str(), Some("malformed_response"));
        assert_eq!(frame.payload["response"].as_slice(), Some(&[1, 2, 3][..]));
        assert_eq!(
            frame.payload["request_resource_hash"].as_str(),
            Some("4444444444444444444444444444444444444444444444444444444444444444")
        );
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(7));
    }

    #[tokio::test]
    async fn request_reconcile_event_tells_client_to_query_requests() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::Requests])));
        let bytes = event_to_frame(
            &DaemonEvent::RequestReconcileRequired { dropped: 12 },
            &subscriptions,
            8,
        )
        .await
        .expect("reconcile event frame");
        let frame = wire::decode_frame(&bytes).expect("valid event frame");

        assert_eq!(frame.msg_type, MessageType::EventRequest);
        assert_eq!(frame.payload["kind"].as_str(), Some("reconcile_required"));
        assert_eq!(frame.payload["dropped"].as_u64(), Some(12));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(8));
    }

    #[tokio::test]
    async fn resource_event_preserves_transfer_progress_and_generation() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::Resources])));
        let mut transfer = ResourceTransferInfo::default();
        transfer.resource_hash = "44".repeat(32);
        transfer.link_id = "33".repeat(16);
        transfer.direction = ResourceDirection::Inbound;
        transfer.state = ResourceTransferState::Transferring;
        transfer.received_bytes = 512;
        transfer.total_bytes = 1_024;
        transfer.progress = 0.5;
        transfer.cancellable = true;
        transfer.observation.source = ObservationSource::TransportResourceState;

        let bytes = event_to_frame(&DaemonEvent::Resource { transfer }, &subscriptions, 9)
            .await
            .expect("subscribed resource event frame");
        let frame = wire::decode_frame(&bytes).expect("valid resource event frame");

        assert_eq!(frame.msg_type, MessageType::EventResource);
        assert_eq!(frame.payload["state"].as_str(), Some("transferring"));
        assert_eq!(frame.payload["received_bytes"].as_u64(), Some(512));
        assert_eq!(frame.payload["total_bytes"].as_u64(), Some(1_024));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(9));
    }

    #[tokio::test]
    async fn general_reconcile_event_reaches_clients_without_activity_subscription() {
        let subscriptions = Arc::new(Mutex::new(HashSet::new()));
        let bytes =
            event_to_frame(&DaemonEvent::ReconcileRequired { dropped: 5 }, &subscriptions, 12)
                .await
                .expect("general reconcile event frame");
        let frame = wire::decode_frame(&bytes).expect("valid reconcile event frame");

        assert_eq!(frame.msg_type, MessageType::EventReconcileRequired);
        assert_eq!(frame.payload["dropped"].as_u64(), Some(5));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(12));
    }

    #[tokio::test]
    async fn network_operation_event_uses_daemon_stage_outcome_and_connection_generation() {
        let subscriptions = Arc::new(Mutex::new(HashSet::from([SubTopic::NetworkOperations])));
        let mut operation = NetworkOperationInfo::default();
        operation.operation_id = "11".repeat(16);
        operation.kind = NetworkOperationKind::Probe;
        operation.link_id = Some("22".repeat(16));
        operation.progress = NetworkOperationProgress::AwaitingProbe;
        operation.outcome = Some(NetworkOperationOutcome::TimedOut);
        operation.observation.source = ObservationSource::OperationCoordinator;
        operation.observation.correlation_id = Some(operation.operation_id.clone());

        let bytes =
            event_to_frame(&DaemonEvent::NetworkOperation { operation }, &subscriptions, 13)
                .await
                .expect("subscribed operation event frame");
        let frame = wire::decode_frame(&bytes).expect("valid operation event frame");

        assert_eq!(frame.msg_type, MessageType::EventNetworkOperation);
        assert_eq!(frame.payload["progress"].as_str(), Some("awaiting_probe"));
        assert_eq!(frame.payload["outcome"].as_str(), Some("timed_out"));
        assert_eq!(frame.payload["source"].as_str(), Some("operation_coordinator"));
        assert_eq!(frame.payload["connection_generation"].as_u64(), Some(13));
        assert_eq!(frame.payload["correlation_id"], frame.payload["operation_id"]);
    }
}
