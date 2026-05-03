//! Daemon bridge — hybrid IPC/embedded connectivity for the desktop app.
//!
//! Boot sequence:
//! 1. Check for running `styrened` on the default IPC socket
//! 2. If found → connect via msgpack wire protocol (multi-client capable)
//! 3. If not found → boot daemon in-process via `styrened::daemon::start()`
//! 4. Either path produces an event stream for reactive UI updates
//!
//! The UI code never knows which mode is active — it just reads signals.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, info_span, warn, Instrument};

use rmpv::Value as MpValue;
use styrene_ipc::types::{DaemonStatusInfo, DeviceInfo, IdentityInfo, MessageInfo};
use styrene_ipc_server::wire::{self, Frame, MessageType, REQUEST_ID_SIZE};

/// A single entry from the path table — routing info for one destination.
#[derive(Debug, Clone)]
pub struct PathTableEntry {
    pub destination_hash: String,
    pub hops: u8,
    pub next_hop: String,
    pub interface: String,
}

/// Events pushed from the daemon to the UI.
#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Connected { mode: ConnectionMode },
    Identity(IdentityInfo),
    Status(DaemonStatusInfo),
    PeerDiscovered(DeviceInfo),
    MessageReceived(MessageInfo),
    MessageStatusChanged { id: String, status: String },
    LinkUpdate { peer_hash: String, status: String, rtt_ms: Option<f64> },
    PathTable(Vec<PathTableEntry>),
    Disconnected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionMode {
    Ipc,
    Embedded,
}

/// Commands sent from the UI to the daemon.
#[derive(Debug)]
pub enum DaemonCommand {
    SendChat { peer_hash: String, content: String },
    Announce,
    BlockPeer { hash: String },
    UnblockPeer { hash: String },
    MarkRead { peer_hash: String },
    BrowsePage { host: String, path: String },
    RefreshPathTable,
    LoadConversations,
    LoadMessages { peer_hash: String },
}

/// Handle to the daemon connection. Owns the IPC stream or embedded daemon.
pub struct DaemonBridge {
    stream: Arc<Mutex<UnixStream>>,
    next_id: u64,
}

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
        debug!(target: "dx::rpc", ?msg_type, "acquiring stream lock");
        let mut stream = self.stream.lock().await;
        let lock_ms = start.elapsed().as_millis();
        if lock_ms > 100 {
            warn!(target: "dx::rpc", ?msg_type, lock_ms, "slow lock acquisition");
        }
        wire::write_frame_async(&mut *stream, msg_type, &req_id, payload).await.map_err(|e| {
            error!(target: "dx::rpc", ?msg_type, %e, "write failed");
            format!("write: {e}")
        })?;
        let result = timeout(Duration::from_secs(5), wire::read_frame_async(&mut *stream))
            .await
            .map_err(|_| {
                error!(target: "dx::rpc", ?msg_type, "rpc timeout (5s)");
                "rpc timeout".to_string()
            })?
            .map_err(|e| {
                error!(target: "dx::rpc", ?msg_type, %e, "read failed");
                format!("read: {e}")
            });
        let elapsed_ms = start.elapsed().as_millis();
        debug!(target: "dx::rpc", ?msg_type, elapsed_ms, "rpc complete");
        result
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
        let arr =
            frame.payload.get("paths").and_then(|v| v.as_array()).cloned().unwrap_or_default();
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
                let hops = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("hops"))
                    .and_then(|(_, v)| v.as_u64())
                    .unwrap_or(0) as u8;
                let dest = get("destination_hash");
                if dest.is_empty() {
                    return None;
                }
                Some(PathTableEntry {
                    destination_hash: dest,
                    hops,
                    next_hop: get("next_hop"),
                    interface: get("interface"),
                })
            })
            .collect())
    }

    pub async fn send_chat(&mut self, peer_hash: &str, content: &str) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        p.insert("content".into(), MpValue::from(content));
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    pub async fn announce(&mut self) -> Result<(), String> {
        self.rpc(MessageType::CmdAnnounce, &HashMap::new()).await.map(|_| ())
    }

    pub async fn browse_page(&mut self, host: &str, path: &str) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        p.insert("path".into(), MpValue::from(path));
        let frame = self.rpc(MessageType::QueryPage, &p).await?;
        Ok(mp_str(&frame.payload, "source"))
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
        p.insert("include_unread".into(), MpValue::Boolean(true));
        let frame = self.rpc(MessageType::QueryConversations, &p).await?;
        let arr = frame
            .payload
            .get("conversations")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .into_iter()
            .filter_map(|v| {
                let m = v.as_map()?;
                let mut map = HashMap::new();
                for (k, v) in m {
                    if let Some(key) = k.as_str() {
                        map.insert(key.to_string(), v.clone());
                    }
                }
                Some(map)
            })
            .collect())
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
        let arr =
            frame.payload.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
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
                let mut msg = MessageInfo::default();
                msg.id = get("id");
                msg.source_hash = get("source_hash");
                msg.destination_hash = get("destination_hash");
                msg.content = get("content");
                msg.timestamp = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("timestamp"))
                    .and_then(|(_, v)| v.as_i64())
                    .unwrap_or(0);
                msg.is_outgoing = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("is_outgoing"))
                    .and_then(|(_, v)| v.as_bool())
                    .unwrap_or(false);
                if msg.id.is_empty() {
                    return None;
                }
                Some(msg)
            })
            .collect())
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
        Ok(())
    }

    async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }
}

/// Connect to the daemon — tries IPC socket first, falls back to embedded.
///
/// Returns (bridge, event_receiver, connection_mode).
pub async fn connect(
) -> Result<(Arc<Mutex<DaemonBridge>>, mpsc::Receiver<DaemonEvent>, ConnectionMode), String> {
    let socket_path = styrene_ipc_server::default_socket_path();
    info!(target: "dx::bridge", path = %socket_path.display(), "checking for external daemon");
    if socket_path.exists() {
        info!(target: "dx::bridge", "socket exists, attempting IPC connect");
        match connect_ipc(&socket_path).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                warn!(target: "dx::bridge", %e, "IPC connect failed, falling back to embedded")
            }
        }
    } else {
        info!(target: "dx::bridge", "no external daemon, booting embedded");
    }
    connect_embedded().await
}

async fn connect_ipc(
    socket_path: &std::path::Path,
) -> Result<(Arc<Mutex<DaemonBridge>>, mpsc::Receiver<DaemonEvent>, ConnectionMode), String> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| format!("connect: {e}"))?;

    let stream = Arc::new(Mutex::new(stream));
    let mut bridge = DaemonBridge { stream: stream.clone(), next_id: 0 };

    if !bridge.ping().await {
        return Err("daemon not responsive".into());
    }

    bridge.subscribe_all().await?;

    let (tx, rx) = mpsc::channel(512);
    let _ = tx.send(DaemonEvent::Connected { mode: ConnectionMode::Ipc }).await;

    // Fetch initial state before event reader takes the lock
    if let Ok(info) = bridge.identity().await {
        let _ = tx.send(DaemonEvent::Identity(info)).await;
    }
    if let Ok(status) = bridge.status().await {
        let _ = tx.send(DaemonEvent::Status(status)).await;
    }
    if let Ok(devices) = bridge.devices().await {
        for dev in devices {
            let _ = tx.send(DaemonEvent::PeerDiscovered(dev)).await;
        }
    }

    let bridge = Arc::new(Mutex::new(bridge));

    // Spawn event reader
    let reader_stream = stream.clone();
    tokio::spawn(event_reader(reader_stream, tx.clone()));

    // Spawn periodic poller
    spawn_poller(bridge.clone(), tx);

    Ok((bridge, rx, ConnectionMode::Ipc))
}

async fn connect_embedded(
) -> Result<(Arc<Mutex<DaemonBridge>>, mpsc::Receiver<DaemonEvent>, ConnectionMode), String> {
    let sock = std::env::temp_dir().join(format!("styrene-dx-{}.sock", std::process::id()));

    // Boot daemon in-process — same capabilities as standalone styrened.
    // Loads ~/.config/styrene/config.toml if present (TCP clients, hub
    // connections, RBAC policy, etc.). No capability difference vs external.
    let config = styrened::daemon::DaemonConfig2 {
        db: None,
        config: None,
        identity: None,
        socket: Some(sock.clone()),
        ephemeral: false,
    };

    info!(target: "dx::bridge", "starting embedded daemon");
    let handle = styrened::daemon::start(config)
        .await
        .map_err(|e| format!("embedded daemon boot failed: {e}"))?;
    info!(target: "dx::bridge", "embedded daemon started, waiting for socket");

    tokio::time::sleep(Duration::from_millis(1000)).await;

    info!(target: "dx::bridge", path = %sock.display(), "connecting to embedded socket");
    let stream =
        UnixStream::connect(&sock).await.map_err(|e| format!("connect to embedded: {e}"))?;
    info!(target: "dx::bridge", "connected to embedded daemon");

    let stream = Arc::new(Mutex::new(stream));
    let mut bridge = DaemonBridge { stream: stream.clone(), next_id: 0 };

    info!(target: "dx::bridge", "subscribing to events");
    bridge.subscribe_all().await.map_err(|e| format!("subscribe: {e}"))?;

    let (tx, rx) = mpsc::channel(512);
    let _ = tx.send(DaemonEvent::Connected { mode: ConnectionMode::Embedded }).await;

    info!(target: "dx::bridge", "fetching initial state");
    if let Ok(info) = bridge.identity().await {
        debug!(target: "dx::bridge", hash = %info.destination_hash, "got identity");
        let _ = tx.send(DaemonEvent::Identity(info)).await;
    }
    if let Ok(status) = bridge.status().await {
        debug!(target: "dx::bridge", devices = status.device_count, "got status");
        let _ = tx.send(DaemonEvent::Status(status)).await;
    }
    if let Ok(devices) = bridge.devices().await {
        info!(target: "dx::bridge", count = devices.len(), "got devices");
        for dev in devices {
            let _ = tx.send(DaemonEvent::PeerDiscovered(dev)).await;
        }
    }
    // Fetch initial path table
    match bridge.path_table().await {
        Ok(paths) => {
            info!(target: "dx::bridge", count = paths.len(), "got path table");
            let _ = tx.send(DaemonEvent::PathTable(paths)).await;
        }
        Err(e) => warn!(target: "dx::bridge", %e, "initial path table fetch failed"),
    }

    let bridge = Arc::new(Mutex::new(bridge));
    let reader_stream = stream.clone();
    tokio::spawn(event_reader(reader_stream, tx.clone()).instrument(info_span!("event_reader")));
    spawn_poller(bridge.clone(), tx);

    std::mem::forget(handle);
    info!(target: "dx::bridge", "embedded daemon fully initialized");

    Ok((bridge, rx, ConnectionMode::Embedded))
}

async fn event_reader(stream: Arc<Mutex<UnixStream>>, tx: mpsc::Sender<DaemonEvent>) {
    info!(target: "dx::reader", "event reader started");
    loop {
        let lock_start = std::time::Instant::now();
        let frame_result = {
            let mut guard = stream.lock().await;
            let lock_ms = lock_start.elapsed().as_millis();
            if lock_ms > 50 {
                debug!(target: "dx::reader", lock_ms, "stream lock acquired (slow)");
            }
            // Use a shorter timeout so the poller can interleave
            timeout(Duration::from_secs(2), wire::read_frame_async(&mut *guard)).await
        };

        match frame_result {
            Ok(Ok(frame)) => {
                debug!(target: "dx::reader", msg_type = ?frame.msg_type, "received frame");
                if let Some(ev) = frame_to_event(frame) {
                    if tx.send(ev).await.is_err() {
                        warn!(target: "dx::reader", "channel closed, stopping");
                        break;
                    }
                }
            }
            Ok(Err(e)) => {
                error!(target: "dx::reader", %e, "stream read error");
                let _ = tx.send(DaemonEvent::Disconnected(e.to_string())).await;
                break;
            }
            Err(_) => continue, // timeout — releases lock so poller can run
        }
    }
}

fn spawn_poller(bridge: Arc<Mutex<DaemonBridge>>, tx: mpsc::Sender<DaemonEvent>) {
    tokio::spawn(
        async move {
            info!(target: "dx::poller", "poller started");

            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;

                let start = std::time::Instant::now();
                debug!(target: "dx::poller", "tick: acquiring bridge lock");
                let mut b = bridge.lock().await;
                let lock_ms = start.elapsed().as_millis();
                debug!(target: "dx::poller", lock_ms, "bridge lock acquired");

                if let Ok(status) = b.status().await {
                    let _ = tx.send(DaemonEvent::Status(status)).await;
                }
                if let Ok(devices) = b.devices().await {
                    debug!(target: "dx::poller", count = devices.len(), "polled devices");
                    for dev in devices {
                        let _ = tx.send(DaemonEvent::PeerDiscovered(dev)).await;
                    }
                }
                match b.path_table().await {
                    Ok(paths) => {
                        info!(target: "dx::poller", count = paths.len(), "polled path table");
                        let _ = tx.send(DaemonEvent::PathTable(paths)).await;
                    }
                    Err(e) => warn!(target: "dx::poller", %e, "path table poll failed"),
                }

                let elapsed_ms = start.elapsed().as_millis();
                debug!(target: "dx::poller", elapsed_ms, "tick complete");
            }
        }
        .instrument(info_span!("poller")),
    );
}

fn frame_to_event(frame: Frame) -> Option<DaemonEvent> {
    match frame.msg_type {
        MessageType::EventDevice => {
            let dev = parse_device_event(&frame.payload)?;
            Some(DaemonEvent::PeerDiscovered(dev))
        }
        MessageType::EventMessage => {
            let msg = parse_message_event(&frame.payload)?;
            let kind = frame.payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "new" || kind.is_empty() {
                Some(DaemonEvent::MessageReceived(msg))
            } else {
                Some(DaemonEvent::MessageStatusChanged { id: msg.id, status: kind.to_string() })
            }
        }
        MessageType::EventLink => {
            let peer_hash = mp_str(&frame.payload, "peer_hash");
            let status = mp_str(&frame.payload, "status");
            let rtt_ms = frame.payload.get("rtt_ms").and_then(|v| v.as_f64());
            Some(DaemonEvent::LinkUpdate { peer_hash, status, rtt_ms })
        }
        _ => None,
    }
}

// ── Payload Parsers ─────────────────────────────────────────────────────────

fn mp_str(p: &HashMap<String, MpValue>, key: &str) -> String {
    p.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn parse_identity(p: &HashMap<String, MpValue>) -> IdentityInfo {
    let mut info = IdentityInfo::default();
    info.identity_hash = mp_str(p, "identity_hash");
    info.destination_hash = mp_str(p, "destination_hash");
    info.display_name = mp_str(p, "display_name");
    info.icon = p.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());
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
    s
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
            dev.name = get("name");
            dev.status = get("status");
            dev.device_type = get("device_type");
            dev.is_styrene_node = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("is_styrene_node"))
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false);
            Some(dev)
        })
        .collect())
}

fn parse_device_event(p: &HashMap<String, MpValue>) -> Option<DeviceInfo> {
    let mut dev = DeviceInfo::default();
    dev.destination_hash = mp_str(p, "destination_hash");
    dev.name = mp_str(p, "name");
    dev.status = mp_str(p, "status");
    dev.device_type = mp_str(p, "device_type");
    dev.is_styrene_node = p.get("is_styrene_node").and_then(|v| v.as_bool()).unwrap_or(false);
    if dev.destination_hash.is_empty() {
        return None;
    }
    Some(dev)
}

fn parse_message_event(p: &HashMap<String, MpValue>) -> Option<MessageInfo> {
    let id = mp_str(p, "id");
    if id.is_empty() {
        return None;
    }
    let mut msg = MessageInfo::default();
    msg.id = id;
    msg.source_hash = mp_str(p, "source_hash");
    msg.destination_hash = mp_str(p, "destination_hash");
    msg.content = mp_str(p, "content");
    msg.timestamp = p.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
    msg.is_outgoing = p.get("is_outgoing").and_then(|v| v.as_bool()).unwrap_or(false);
    Some(msg)
}
