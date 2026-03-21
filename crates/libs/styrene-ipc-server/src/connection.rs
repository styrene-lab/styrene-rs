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

use styrene_ipc::traits::Daemon;
use styrene_ipc::types::DaemonEvent;

use crate::dispatch;
use crate::wire::{self, MessageType, WireError, REQUEST_ID_SIZE};

/// Subscription topics a client can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubTopic {
    Devices,
    Messages,
    Activity,
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
    let subscriptions = Arc::new(Mutex::new(HashSet::<SubTopic>::new()));

    // Channel for sending response/event frames to the writer task
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(256);

    // Spawn writer task
    let subs_for_writer = subscriptions.clone();
    let writer_handle = tokio::spawn(writer_loop(
        write_half,
        frame_rx,
        event_rx,
        subs_for_writer,
    ));

    // Read loop
    let mut reader = tokio::io::BufReader::new(read_half);
    loop {
        match wire::read_frame_async(&mut reader).await {
            Ok(frame) => {
                let response_bytes = handle_frame(
                    &daemon,
                    frame.msg_type,
                    &frame.request_id,
                    frame.payload,
                    &subscriptions,
                )
                .await;

                if let Some(bytes) = response_bytes {
                    if frame_tx.send(bytes).await.is_err() {
                        break; // Writer gone
                    }
                }
            }
            Err(WireError::Io(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                log::debug!("client disconnected (EOF)");
                break;
            }
            Err(e) => {
                log::warn!("client frame error: {e}");
                break;
            }
        }
    }

    // Cleanup: drop sender so writer task exits
    drop(frame_tx);
    let _ = writer_handle.await;
}

/// Handle a single frame, returning optional response bytes.
async fn handle_frame(
    daemon: &Arc<dyn Daemon>,
    msg_type: MessageType,
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: HashMap<String, rmpv::Value>,
    subscriptions: &Arc<Mutex<HashSet<SubTopic>>>,
) -> Option<Vec<u8>> {
    match msg_type {
        // Keepalive
        MessageType::Ping => {
            let empty = HashMap::new();
            wire::encode_frame(MessageType::Pong, request_id, &empty).ok()
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
        MessageType::Unsub => {
            // Unsubscribe from the topic specified in payload, or all
            let mut subs = subscriptions.lock().await;
            if let Some(topic) = payload.get("topic").and_then(|v| v.as_str()) {
                match topic {
                    "devices" => { subs.remove(&SubTopic::Devices); }
                    "messages" => { subs.remove(&SubTopic::Messages); }
                    "activity" => { subs.remove(&SubTopic::Activity); }
                    _ => {}
                }
            } else {
                subs.clear();
            }
            some_result(request_id, HashMap::new())
        }

        // Dispatch to daemon
        _ if msg_type.is_request() => {
            let result = dispatch::dispatch(daemon, msg_type, payload).await;
            match result {
                Ok(resp_payload) => {
                    wire::encode_frame(MessageType::Result, request_id, &resp_payload).ok()
                }
                Err(err_msg) => {
                    let mut p = HashMap::new();
                    p.insert("error".to_string(), rmpv::Value::from(err_msg));
                    wire::encode_frame(MessageType::Error, request_id, &p).ok()
                }
            }
        }

        // Responses and events from client are unexpected — ignore
        _ => None,
    }
}

fn some_result(
    request_id: &[u8; REQUEST_ID_SIZE],
    payload: HashMap<String, rmpv::Value>,
) -> Option<Vec<u8>> {
    wire::encode_frame(MessageType::Result, request_id, &payload).ok()
}

/// Writer loop: sends response frames and pushes subscription events.
async fn writer_loop(
    mut writer: OwnedWriteHalf,
    mut frame_rx: mpsc::Receiver<Vec<u8>>,
    mut event_rx: broadcast::Receiver<DaemonEvent>,
    subscriptions: Arc<Mutex<HashSet<SubTopic>>>,
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
                        if let Some(bytes) = event_to_frame(&daemon_event, &subscriptions).await {
                            if writer.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = writer.flush().await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("client event lag: dropped {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Convert a DaemonEvent into a wire frame if the client is subscribed.
async fn event_to_frame(
    event: &DaemonEvent,
    subscriptions: &Arc<Mutex<HashSet<SubTopic>>>,
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
            p.insert(
                "identity_hash".to_string(),
                rmpv::Value::from(device.identity_hash.as_str()),
            );
            p.insert(
                "device_type".to_string(),
                rmpv::Value::from(device.device_type.as_str()),
            );
            p.insert(
                "status".to_string(),
                rmpv::Value::from(device.status.as_str()),
            );
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
            let mut p = HashMap::new();
            p.insert("kind".to_string(), rmpv::Value::from(kind_str));
            p.insert("id".to_string(), rmpv::Value::from(message.id.as_str()));
            p.insert(
                "source_hash".to_string(),
                rmpv::Value::from(message.source_hash.as_str()),
            );
            p.insert(
                "content".to_string(),
                rmpv::Value::from(message.content.as_str()),
            );
            (MessageType::EventMessage, SubTopic::Messages, p)
        }
        DaemonEvent::TerminalOutput { session_id, data } => {
            let mut p = HashMap::new();
            p.insert(
                "session_id".to_string(),
                rmpv::Value::from(session_id.as_str()),
            );
            p.insert("data".to_string(), rmpv::Value::from(data.as_slice()));
            (MessageType::EventTerminalOutput, SubTopic::Activity, p)
        }
        DaemonEvent::TerminalStateChange { session_id, .. } => {
            let mut p = HashMap::new();
            p.insert(
                "session_id".to_string(),
                rmpv::Value::from(session_id.as_str()),
            );
            (MessageType::EventTerminalReady, SubTopic::Activity, p)
        }
        DaemonEvent::TunnelStateChange {
            peer_hash,
            state,
            backend,
        } => {
            let mut p = HashMap::new();
            p.insert(
                "peer_hash".to_string(),
                rmpv::Value::from(peer_hash.as_str()),
            );
            p.insert("state".to_string(), rmpv::Value::from(state.as_str()));
            p.insert("backend".to_string(), rmpv::Value::from(backend.as_str()));
            (MessageType::EventActivity, SubTopic::Activity, p)
        }
        // Future event variants — skip unknown
        _ => return None,
    };

    if !subs.contains(&topic) {
        return None;
    }

    // Use zero request_id for pushed events
    let zero_id = [0u8; 16];
    wire::encode_frame(msg_type, &zero_id, &payload).ok()
}
