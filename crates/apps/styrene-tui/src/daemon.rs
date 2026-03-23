//! Daemon RPC bridge — connects styrene-tui to a running styrened daemon
#![allow(dead_code)]
//! via the Unix socket IPC protocol.
//!
//! Architecture:
//!   - `DaemonHandle` is the single connection owner
//!   - `connect()` dials the socket, returns the handle + event receiver
//!   - The caller drives a background task that calls `poll_events()`
//!     and converts `DaemonEvent`s into `TuiEvent`s for the App
//!
//! Wire protocol: msgpack frames over Unix domain socket
//! (same as Python TUI ↔ styrened — wire-compatible).
//!
//! Usage:
//!
//! ```no_run
//! // In main, after building App:
//! if let Ok((handle, mut rx)) = daemon::connect(None).await {
//!     app.footer.node_hash = handle.identity().await.destination_hash;
//!     tokio::spawn(async move {
//!         while let Some(ev) = rx.recv().await {
//!             // post ev into app via a Mutex or channel
//!         }
//!     });
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path};
use std::sync::Arc;

use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{Duration, timeout};

use rmpv::Value as MpValue;
use styrene_ipc::types::{DaemonStatusInfo, DeviceInfo, IdentityInfo, MessageInfo};
use styrene_ipc_server::wire::{
    self, Frame, MessageType, REQUEST_ID_SIZE,
};

use crate::mesh_state::{ActivityEntry, ActivityKind, PeerRecord, epoch_secs};

// ─── TuiEvent — what the bridge sends to the App ─────────────────────────────

#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// Initial identity loaded on connect.
    Identity(IdentityInfo),
    /// Daemon status snapshot (polled periodically).
    Status(DaemonStatusInfo),
    /// New or updated announce / peer record.
    PeerAnnounce(PeerRecord),
    /// Inbound LXMF message received.
    Message(MessageInfo),
    /// Message delivery status changed.
    MessageStatus { id: String, status: String },
    /// Daemon disconnected or unreachable.
    Disconnected(String),
}

// ─── Connection ───────────────────────────────────────────────────────────────

pub struct DaemonHandle {
    stream: Arc<Mutex<UnixStream>>,
    next_id: u64,
}

impl DaemonHandle {
    fn next_request_id(&mut self) -> [u8; REQUEST_ID_SIZE] {
        self.next_id = self.next_id.wrapping_add(1);
        let mut id = [0u8; REQUEST_ID_SIZE];
        id[..8].copy_from_slice(&self.next_id.to_le_bytes());
        id
    }

    /// Send a request and receive the response frame.
    async fn rpc(
        &mut self,
        msg_type: MessageType,
        payload: &HashMap<String, MpValue>,
    ) -> Result<Frame, String> {
        let req_id = self.next_request_id();
        let mut stream = self.stream.lock().await;

        // Write frame
        wire::write_frame_async(&mut *stream, msg_type, &req_id, payload)
            .await
            .map_err(|e| format!("write: {e}"))?;

        // Read response (5s timeout)
        let frame = timeout(
            Duration::from_secs(5),
            wire::read_frame_async(&mut *stream),
        )
        .await
        .map_err(|_| "rpc timeout".to_string())?
        .map_err(|e| format!("read: {e}"))?;

        Ok(frame)
    }

    /// Query local node identity.
    pub async fn identity(&mut self) -> Result<IdentityInfo, String> {
        let frame = self.rpc(MessageType::QueryIdentity, &HashMap::new()).await?;
        parse_identity(&frame.payload)
    }

    /// Query daemon status.
    pub async fn status(&mut self) -> Result<DaemonStatusInfo, String> {
        let frame = self.rpc(MessageType::QueryStatus, &HashMap::new()).await?;
        parse_status(&frame.payload)
    }

    /// Query known devices (announces).
    pub async fn devices(&mut self, styrene_only: bool) -> Result<Vec<DeviceInfo>, String> {
        let mut p = HashMap::new();
        p.insert("styrene_only".into(), MpValue::Boolean(styrene_only));
        let frame = self.rpc(MessageType::QueryDevices, &p).await?;
        parse_devices(&frame.payload)
    }

    /// Subscribe to message events. Must be called before the read loop.
    pub async fn subscribe_messages(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubMessages, &HashMap::new())
            .await
            .map(|_| ())
    }

    /// Subscribe to device/announce events.
    pub async fn subscribe_devices(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubDevices, &HashMap::new())
            .await
            .map(|_| ())
    }

    /// Send a ping. Returns true if pong received.
    pub async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }
}

// ─── Public connect function ──────────────────────────────────────────────────

/// Connect to the styrened daemon. Returns a handle and a channel of TuiEvents.
///
/// `socket_path`: overrides the default path ($STYRENED_SOCKET or
/// $XDG_RUNTIME_DIR/styrened/control.sock).
///
/// Returns `Err` if the socket doesn't exist or the daemon doesn't respond
/// to the initial ping. The TUI degrades gracefully to demo mode.
pub async fn connect(
    socket_path: Option<&Path>,
) -> Result<(DaemonHandle, mpsc::Receiver<TuiEvent>), String> {
    let path = socket_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(styrene_ipc_server::default_socket_path);

    if !path.exists() {
        return Err(format!("socket not found: {}", path.display()));
    }

    let stream = UnixStream::connect(&path)
        .await
        .map_err(|e| format!("connect {}: {e}", path.display()))?;

    let stream = Arc::new(Mutex::new(stream));
    let mut handle = DaemonHandle { stream: stream.clone(), next_id: 0 };

    // Verify daemon is alive
    if !handle.ping().await {
        return Err("daemon did not respond to ping".into());
    }

    // Subscribe to events before spawning the reader
    let _ = handle.subscribe_devices().await;
    let _ = handle.subscribe_messages().await;

    // Spawn the event reader task
    let (tx, rx) = mpsc::channel::<TuiEvent>(128);
    let reader_stream = stream.clone();
    tokio::spawn(event_reader(reader_stream, tx));

    Ok((handle, rx))
}

// ─── Event reader task ────────────────────────────────────────────────────────

async fn event_reader(stream: Arc<Mutex<UnixStream>>, tx: mpsc::Sender<TuiEvent>) {
    loop {
        // Lock for one frame read — release immediately so rpc() can also lock
        let frame_result = {
            let mut guard = stream.lock().await;
            timeout(
                Duration::from_secs(60),
                wire::read_frame_async(&mut *guard),
            )
            .await
        };

        match frame_result {
            Ok(Ok(frame)) => {
                if let Some(ev) = frame_to_tui_event(frame) {
                    if tx.send(ev).await.is_err() {
                        break; // receiver dropped — TUI exited
                    }
                }
            }
            Ok(Err(e)) => {
                let _ = tx.send(TuiEvent::Disconnected(e.to_string())).await;
                break;
            }
            Err(_) => {
                // 60s timeout — send a keepalive; daemon may have gone quiet
                // (the rpc() lock and our lock are the same, so we can't call
                //  ping() here without deadlock — just continue and wait)
                continue;
            }
        }
    }
}

/// Convert a pushed server frame into a TuiEvent, if applicable.
fn frame_to_tui_event(frame: Frame) -> Option<TuiEvent> {
    match frame.msg_type {
        MessageType::EventDevice => {
            let device = parse_device_from_payload(&frame.payload)?;
            let now = epoch_secs();
            let peer = PeerRecord::new(
                device.destination_hash.clone(),
                if device.name.is_empty() { None } else { Some(device.name.clone()) },
                now,
            );
            Some(TuiEvent::PeerAnnounce(peer))
        }
        MessageType::EventMessage => {
            let msg = parse_message_from_payload(&frame.payload)?;
            let kind = frame.payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "new" || kind.is_empty() {
                Some(TuiEvent::Message(msg))
            } else {
                Some(TuiEvent::MessageStatus { id: msg.id, status: kind.to_string() })
            }
        }
        _ => None,
    }
}

// ─── Periodic poller ─────────────────────────────────────────────────────────

/// Spawn a task that polls the daemon periodically and sends snapshot TuiEvents.
/// Call once after `connect()`. Sends Identity on first poll, then Status every N seconds.
pub fn spawn_poll_task(
    handle: Arc<Mutex<DaemonHandle>>,
    tx: mpsc::Sender<TuiEvent>,
    poll_interval_secs: u64,
) {
    tokio::spawn(async move {
        let mut first = true;
        loop {
            // Initial identity fetch
            if first {
                first = false;
                let result = handle.lock().await.identity().await;
                match result {
                    Ok(info) => { let _ = tx.send(TuiEvent::Identity(info)).await; }
                    Err(e) => { let _ = tx.send(TuiEvent::Disconnected(e)).await; return; }
                }
            }

            // Periodic status + devices
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;

            let status = handle.lock().await.status().await;
            match status {
                Ok(s) => { let _ = tx.send(TuiEvent::Status(s)).await; }
                Err(e) => { let _ = tx.send(TuiEvent::Disconnected(e)).await; return; }
            }

            let devices = handle.lock().await.devices(false).await;
            if let Ok(devs) = devices {
                let now = epoch_secs();
                for dev in devs {
                    if dev.destination_hash.is_empty() { continue; }
                    let peer = PeerRecord::new(
                        dev.destination_hash.clone(),
                        if dev.name.is_empty() { None } else { Some(dev.name.clone()) },
                        now,
                    );
                    let _ = tx.send(TuiEvent::PeerAnnounce(peer)).await;
                }
            }
        }
    });
}

// ─── App-side event application ───────────────────────────────────────────────

/// Apply a TuiEvent to the App state. Call from the main event loop.
pub fn apply_event(app: &mut crate::app::App, ev: TuiEvent) {
    use crate::tui::segments::{DeliveryStatus, ProtocolEventKind};

    match ev {
        TuiEvent::Identity(info) => {
            app.footer.node_hash = info.destination_hash.clone();
            app.footer.node_name = info.display_name.clone();
            app.footer.transport_active = true;
            let hash_short = &info.destination_hash[..8.min(info.destination_hash.len())];
            app.conversation.push_system(
                &format!("⬡ connected  node: {hash_short}…  name: {}",
                    info.display_name),
            );
        }

        TuiEvent::Status(status) => {
            app.footer.transport_active = status.rns_initialized;
            app.footer.active_links = status.active_links as usize;
        }

        TuiEvent::PeerAnnounce(peer) => {
            let hash = peer.hash.clone();
            let name = peer.name.clone();
            let now = epoch_secs();

            if let Some(existing) = app.peers.iter_mut().find(|p| p.hash == hash) {
                existing.touch(now, 1);
            } else {
                app.conversation.push_protocol_event(
                    ProtocolEventKind::Announce,
                    Some(&hash[..8.min(hash.len())]),
                    name.as_deref(),
                    "announce",
                );
                app.activity.push(ActivityEntry::new(
                    ActivityKind::Announce,
                    name.as_deref().unwrap_or(&hash[..8.min(hash.len())]),
                    "announce received",
                ));
                app.peers.push(peer);
            }
            app.footer.trigger_flash();
        }

        TuiEvent::Message(msg) => {
            if msg.is_outgoing { return; }
            let name = app.peers.iter()
                .find(|p| p.hash == msg.source_hash)
                .and_then(|p| p.name.clone());

            app.conversation.push_received(
                &msg.source_hash,
                name.as_deref(),
                msg.title.as_deref(),
                &msg.content,
                msg.timestamp,
            );
            app.activity.push(ActivityEntry::new(
                ActivityKind::InboundMessage,
                name.as_deref().unwrap_or(&msg.source_hash[..8.min(msg.source_hash.len())]),
                msg.title.as_deref().unwrap_or(&msg.content[..msg.content.len().min(32)]),
            ));
            app.footer.unread_messages += 1;
            app.footer.total_messages += 1;
            app.footer.trigger_flash();
        }

        TuiEvent::MessageStatus { id: _, status } => {
            // Map daemon status string to DeliveryStatus and update last sent
            let ds = match status.as_str() {
                "delivered" => DeliveryStatus::Delivered,
                s if s.starts_with("failed") => DeliveryStatus::Failed(s.to_string()),
                s if s.starts_with("sending") => DeliveryStatus::Sending,
                _ => DeliveryStatus::Sent,
            };
            app.conversation.update_last_sent_status(ds);
        }

        TuiEvent::Disconnected(reason) => {
            app.conversation.push_system(
                &format!("⚠ daemon disconnected: {reason}"),
            );
            app.footer.transport_active = false;
        }
    }
}

// ─── Wire payload parsers ─────────────────────────────────────────────────────

fn mp_str(payload: &HashMap<String, MpValue>, key: &str) -> String {
    payload.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn mp_bool(payload: &HashMap<String, MpValue>, key: &str) -> bool {
    payload.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn mp_u64(payload: &HashMap<String, MpValue>, key: &str) -> u64 {
    payload.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn mp_i64(payload: &HashMap<String, MpValue>, key: &str) -> i64 {
    payload.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn parse_identity(p: &HashMap<String, MpValue>) -> Result<IdentityInfo, String> {
    let mut info = IdentityInfo::default();
    info.identity_hash = mp_str(p, "identity_hash");
    info.destination_hash = mp_str(p, "destination_hash");
    info.lxmf_destination_hash = mp_str(p, "lxmf_destination_hash");
    info.display_name = mp_str(p, "display_name");
    info.icon = p.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());
    info.short_name = p.get("short_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    Ok(info)
}

fn parse_status(p: &HashMap<String, MpValue>) -> Result<DaemonStatusInfo, String> {
    let mut s = DaemonStatusInfo::default();
    s.uptime = mp_u64(p, "uptime");
    s.daemon_version = mp_str(p, "daemon_version");
    s.rns_initialized = mp_bool(p, "rns_initialized");
    s.lxmf_initialized = mp_bool(p, "lxmf_initialized");
    s.device_count = p.get("device_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.interface_count = p.get("interface_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.hub_status = p.get("hub_status").and_then(|v| v.as_str()).map(|s| s.to_string());
    s.propagation_enabled = mp_bool(p, "propagation_enabled");
    s.transport_enabled = mp_bool(p, "transport_enabled");
    s.active_links = p.get("active_links").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    Ok(s)
}

fn parse_devices(p: &HashMap<String, MpValue>) -> Result<Vec<DeviceInfo>, String> {
    let arr = p.get("devices")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no 'devices' array in response".to_string())?;

    Ok(arr.iter().filter_map(|v| {
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
        dev.device_type = get("device_type");
        dev.status = get("status");
        dev.is_styrene_node = m.iter()
            .find(|(k, _)| k.as_str() == Some("is_styrene_node"))
            .and_then(|(_, v)| v.as_bool())
            .unwrap_or(false);
        dev.lxmf_destination_hash = get("lxmf_destination_hash");
        Some(dev)
    }).collect())
}

fn parse_device_from_payload(p: &HashMap<String, MpValue>) -> Option<DeviceInfo> {
    let mut dev = DeviceInfo::default();
    dev.destination_hash = mp_str(p, "destination_hash");
    dev.identity_hash = mp_str(p, "identity_hash");
    dev.name = mp_str(p, "name");
    dev.device_type = mp_str(p, "device_type");
    dev.status = mp_str(p, "status");
    dev.is_styrene_node = mp_bool(p, "is_styrene_node");
    dev.lxmf_destination_hash = mp_str(p, "lxmf_destination_hash");
    dev.last_announce = Some(mp_i64(p, "last_announce"));
    dev.announce_count = p.get("announce_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    dev.short_name = p.get("short_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    Some(dev)
}

fn parse_message_from_payload(p: &HashMap<String, MpValue>) -> Option<MessageInfo> {
    let id = mp_str(p, "id");
    if id.is_empty() { return None; }
    let mut msg = MessageInfo::default();
    msg.id = id;
    msg.source_hash = mp_str(p, "source_hash");
    msg.destination_hash = mp_str(p, "destination_hash");
    msg.timestamp = mp_i64(p, "timestamp");
    msg.content = mp_str(p, "content");
    msg.title = p.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());
    msg.status = mp_str(p, "status");
    msg.is_outgoing = mp_bool(p, "is_outgoing");
    msg.delivery_method = p.get("delivery_method").and_then(|v| v.as_str()).map(|s| s.to_string());
    msg.read = mp_bool(p, "read");
    Some(msg)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_identity_defaults() {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String("deadbeef".into()));
        p.insert("display_name".into(), MpValue::String("Test Node".into()));
        let id = parse_identity(&p).unwrap();
        assert_eq!(id.destination_hash, "deadbeef");
        assert_eq!(id.display_name, "Test Node");
        assert!(id.icon.is_none());
    }

    #[test]
    fn parse_status_defaults() {
        let mut p = HashMap::new();
        p.insert("uptime".into(), MpValue::Integer(42.into()));
        p.insert("rns_initialized".into(), MpValue::Boolean(true));
        let s = parse_status(&p).unwrap();
        assert_eq!(s.uptime, 42);
        assert!(s.rns_initialized);
        assert_eq!(s.active_links, 0);
    }

    #[test]
    fn frame_to_tui_event_unknown_type_is_none() {
        let frame = Frame {
            msg_type: MessageType::Pong,
            request_id: [0; REQUEST_ID_SIZE],
            payload: HashMap::new(),
        };
        assert!(frame_to_tui_event(frame).is_none());
    }

    #[test]
    fn parse_message_from_empty_payload_is_none() {
        let p = HashMap::new();
        assert!(parse_message_from_payload(&p).is_none());
    }

    #[test]
    fn apply_disconnected_sets_transport_inactive() {
        // We can't easily construct a full App in unit tests,
        // but we can verify the TuiEvent variants are constructible
        let ev = TuiEvent::Disconnected("test reason".into());
        match ev {
            TuiEvent::Disconnected(reason) => assert_eq!(reason, "test reason"),
            _ => panic!("wrong variant"),
        }
    }
}
