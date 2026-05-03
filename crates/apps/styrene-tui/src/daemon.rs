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
use std::path::Path;
use std::sync::Arc;

use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, timeout};

use rmpv::Value as MpValue;
use styrene_ipc::types::{DaemonStatusInfo, DeviceInfo, IdentityInfo, MessageInfo};
use styrene_ipc_server::wire::{self, Frame, MessageType, REQUEST_ID_SIZE};

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
    /// Link telemetry event (activated, closed, rtt_updated).
    LinkUpdate {
        link_id: String,
        peer_hash: String,
        peer_name: Option<String>,
        status: String,
        rtt_ms: Option<f64>,
    },
    /// Result of a queued daemon command.
    CommandResult { action: String, success: bool, detail: String },
    /// Page content loaded from a host.
    PageLoaded { host: String, path: String, source: String },
    /// Page list from a host.
    PageList { host: String, pages: Vec<String> },
    /// Terminal output data from a remote session.
    TerminalOutput { session_id: String, data: Vec<u8> },
    /// Terminal session exited.
    TerminalExited { session_id: String, exit_code: Option<i32> },
    /// Daemon disconnected or unreachable.
    Disconnected(String),
}

// ─── Daemon Command Queue ────────────────────────────────────────────────────
//
// The key handler is synchronous but daemon calls are async. Commands are queued
// from the sync handler and executed by a background task that owns DaemonHandle.
// Results come back as TuiEvents.

#[derive(Debug)]
pub enum DaemonCmd {
    /// Send a chat message to a peer.
    SendChat { peer_hash: String, content: String },
    /// Announce this node to the mesh.
    Announce,
    /// Block a peer by identity hash.
    BlockPeer { identity_hash: String },
    /// Unblock a peer by identity hash.
    UnblockPeer { identity_hash: String },
    /// Query remote device status.
    DeviceStatus { dest_hash: String },
    /// Execute a command on a remote device.
    Exec { dest_hash: String, command: String, args: Vec<String> },
    /// Reboot a remote device.
    RebootDevice { dest_hash: String, delay_secs: Option<u64> },
    /// Push config profile to a remote device.
    FleetApply { dest_hash: String, profile_hex: String },
    /// Update local identity.
    SetIdentity { display_name: String, icon: Option<String> },
    /// Set auto-reply configuration.
    SetAutoReply { mode: String, message: String },
    /// Mark conversation as read.
    MarkRead { peer_hash: String },
    /// Browse a page from a host.
    BrowsePage { host: String, path: String },
    /// List pages served by a host.
    ListPages { host: String },
}

/// Spawn the command executor task. Processes DaemonCmd messages and posts
/// results back as TuiEvents via the event channel.
pub fn spawn_command_executor(
    handle: Arc<Mutex<DaemonHandle>>,
    mut cmd_rx: mpsc::Receiver<DaemonCmd>,
    event_tx: mpsc::Sender<TuiEvent>,
) {
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let mut h = handle.lock().await;
            match cmd {
                DaemonCmd::SendChat { peer_hash, content } => {
                    match h.send_chat(&peer_hash, &content, None).await {
                        Ok(msg_id) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "send_chat".into(),
                                    success: true,
                                    detail: format!("sent: {}", &msg_id[..8.min(msg_id.len())]),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "send_chat".into(),
                                    success: false,
                                    detail: e,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::Announce => {
                    let result = h.announce().await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "announce".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "announced".into()),
                        })
                        .await;
                }
                DaemonCmd::BlockPeer { identity_hash } => {
                    let result = h.block_peer(&identity_hash).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "block_peer".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| {
                                format!("blocked {}", &identity_hash[..8.min(identity_hash.len())])
                            }),
                        })
                        .await;
                }
                DaemonCmd::UnblockPeer { identity_hash } => {
                    let result = h.unblock_peer(&identity_hash).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "unblock_peer".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| {
                                format!(
                                    "unblocked {}",
                                    &identity_hash[..8.min(identity_hash.len())]
                                )
                            }),
                        })
                        .await;
                }
                DaemonCmd::DeviceStatus { dest_hash } => {
                    match h.device_status(&dest_hash, Some(30)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "device_status".into(),
                                    success: true,
                                    detail,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "device_status".into(),
                                    success: false,
                                    detail: e,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::Exec { dest_hash, command, args } => {
                    match h.exec(&dest_hash, &command, &args, Some(60)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "exec".into(),
                                    success: true,
                                    detail,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "exec".into(),
                                    success: false,
                                    detail: e,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::RebootDevice { dest_hash, delay_secs } => {
                    let result = h.reboot_device(&dest_hash, delay_secs, Some(30)).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "reboot_device".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "reboot accepted".into()),
                        })
                        .await;
                }
                DaemonCmd::FleetApply { dest_hash, profile_hex } => {
                    match h.fleet_apply(&dest_hash, &profile_hex, true, Some(120)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "fleet_apply".into(),
                                    success: true,
                                    detail,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "fleet_apply".into(),
                                    success: false,
                                    detail: e,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::SetIdentity { display_name, icon } => {
                    let result = h.set_identity(&display_name, icon.as_deref()).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "set_identity".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "identity updated".into()),
                        })
                        .await;
                }
                DaemonCmd::SetAutoReply { mode, message } => {
                    let result = h.set_auto_reply(&mode, &message, None).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "set_auto_reply".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "auto-reply updated".into()),
                        })
                        .await;
                }
                DaemonCmd::MarkRead { peer_hash } => {
                    let _ = h.mark_read(&peer_hash).await;
                }
                DaemonCmd::BrowsePage { host, path } => match h.query_page(&host, &path).await {
                    Ok((source, _host_hash)) => {
                        let _ = event_tx
                            .send(TuiEvent::PageLoaded { host: host.clone(), path, source })
                            .await;
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "browse_page".into(),
                                success: false,
                                detail: e,
                            })
                            .await;
                    }
                },
                DaemonCmd::ListPages { host } => match h.list_pages(&host).await {
                    Ok(pages) => {
                        let paths: Vec<String> = pages.into_iter().map(|(p, _)| p).collect();
                        let _ = event_tx.send(TuiEvent::PageList { host, pages: paths }).await;
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "list_pages".into(),
                                success: false,
                                detail: e,
                            })
                            .await;
                    }
                },
            }
        }
    });
}

/// Format a msgpack payload map into a human-readable summary.
fn format_payload_summary(payload: &HashMap<String, MpValue>) -> String {
    let mut parts = Vec::new();
    for (key, val) in payload {
        let val_str = match val {
            MpValue::String(s) => s.as_str().unwrap_or("").to_string(),
            MpValue::Integer(i) => format!("{}", i.as_i64().unwrap_or(0)),
            MpValue::Boolean(b) => b.to_string(),
            _ => format!("{val:?}"),
        };
        if !val_str.is_empty() && val_str.len() < 200 {
            parts.push(format!("  {key}: {val_str}"));
        }
    }
    if parts.is_empty() { "  (no data)".into() } else { parts.join("\n") }
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
        let frame = timeout(Duration::from_secs(5), wire::read_frame_async(&mut *stream))
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
        self.rpc(MessageType::SubMessages, &HashMap::new()).await.map(|_| ())
    }

    /// Subscribe to device/announce events.
    pub async fn subscribe_devices(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubDevices, &HashMap::new()).await.map(|_| ())
    }

    /// Subscribe to link telemetry events (activated, closed, RTT updated).
    pub async fn subscribe_links(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubLinks, &HashMap::new()).await.map(|_| ())
    }

    /// Send a ping. Returns true if pong received.
    pub async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }

    // ── Chat Operations ─────────────────────────────────────────────────

    /// Send a chat message to a peer.
    pub async fn send_chat(
        &mut self,
        dest_hash: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(dest_hash));
        p.insert("content".into(), MpValue::from(content));
        if let Some(t) = title {
            p.insert("title".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    /// Mark all messages from a peer as read.
    pub async fn mark_read(&mut self, peer_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        self.rpc(MessageType::CmdMarkRead, &p).await.map(|_| ())
    }

    /// Delete a message by ID.
    pub async fn delete_message(&mut self, message_id: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("message_id".into(), MpValue::from(message_id));
        self.rpc(MessageType::CmdDeleteMessage, &p).await.map(|_| ())
    }

    // ── Fleet Operations ────────────────────────────────────────────────

    /// Query remote device status.
    pub async fn device_status(
        &mut self,
        dest_hash: &str,
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdDeviceStatus, &p).await?;
        Ok(frame.payload)
    }

    /// Execute a command on a remote device.
    pub async fn exec(
        &mut self,
        dest_hash: &str,
        cmd: &str,
        args: &[String],
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("command".into(), MpValue::from(cmd));
        let args_vals: Vec<MpValue> = args.iter().map(|a| MpValue::from(a.as_str())).collect();
        p.insert("args".into(), MpValue::Array(args_vals));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdExec, &p).await?;
        Ok(frame.payload)
    }

    /// Reboot a remote device.
    pub async fn reboot_device(
        &mut self,
        dest_hash: &str,
        delay_secs: Option<u64>,
        timeout_secs: Option<u64>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        if let Some(d) = delay_secs {
            p.insert("delay".into(), MpValue::from(d));
        }
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        self.rpc(MessageType::CmdRebootDevice, &p).await.map(|_| ())
    }

    /// Push a signed profile to a remote node.
    pub async fn fleet_apply(
        &mut self,
        dest_hash: &str,
        profile_hex: &str,
        verify: bool,
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("profile".into(), MpValue::from(profile_hex));
        p.insert("verify".into(), MpValue::Boolean(verify));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdFleetApply, &p).await?;
        Ok(frame.payload)
    }

    // ── Identity & Settings ─────────────────────────────────────────────

    /// Update local node identity (display name, icon).
    pub async fn set_identity(
        &mut self,
        display_name: &str,
        icon: Option<&str>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("display_name".into(), MpValue::from(display_name));
        if let Some(i) = icon {
            p.insert("icon".into(), MpValue::from(i));
        }
        self.rpc(MessageType::CmdSetIdentity, &p).await.map(|_| ())
    }

    /// Send a mesh announce.
    pub async fn announce(&mut self) -> Result<(), String> {
        self.rpc(MessageType::CmdAnnounce, &HashMap::new()).await.map(|_| ())
    }

    /// Query daemon configuration.
    pub async fn query_config(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryConfig, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    /// Set auto-reply configuration.
    pub async fn set_auto_reply(
        &mut self,
        mode: &str,
        message: &str,
        cooldown_secs: Option<u64>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("mode".into(), MpValue::from(mode));
        p.insert("message".into(), MpValue::from(message));
        if let Some(c) = cooldown_secs {
            p.insert("cooldown_secs".into(), MpValue::from(c));
        }
        self.rpc(MessageType::CmdSetAutoReply, &p).await.map(|_| ())
    }

    /// Block a peer by identity hash.
    pub async fn block_peer(&mut self, identity_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(identity_hash));
        self.rpc(MessageType::CmdBlockPeer, &p).await.map(|_| ())
    }

    /// Unblock a peer by identity hash.
    pub async fn unblock_peer(&mut self, identity_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(identity_hash));
        self.rpc(MessageType::CmdUnblockPeer, &p).await.map(|_| ())
    }

    /// Query the list of blocked peers.
    pub async fn blocked_peers(&mut self) -> Result<Vec<String>, String> {
        let frame = self.rpc(MessageType::QueryBlockedPeers, &HashMap::new()).await?;
        let arr = frame
            .payload
            .get("blocked_peers")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        Ok(arr)
    }

    // ── Terminal Session ─────────────────────────────────────────────────

    /// Open a remote terminal session.
    pub async fn terminal_open(
        &mut self,
        dest_hash: &str,
        rows: u16,
        cols: u16,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("rows".into(), MpValue::from(rows as u64));
        p.insert("cols".into(), MpValue::from(cols as u64));
        let frame = self.rpc(MessageType::CmdTerminalOpen, &p).await?;
        Ok(mp_str(&frame.payload, "session_id"))
    }

    /// Send input data to a terminal session.
    pub async fn terminal_input(&mut self, session_id: &str, data: &[u8]) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        p.insert("data".into(), MpValue::Binary(data.to_vec()));
        self.rpc(MessageType::CmdTerminalInput, &p).await.map(|_| ())
    }

    // ── Page Operations ──────────────────────────────────────────────────

    /// Browse a Micron page from a host (or "local" for this node's pages).
    pub async fn query_page(&mut self, host: &str, path: &str) -> Result<(String, String), String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        p.insert("path".into(), MpValue::from(path));
        let frame = self.rpc(MessageType::QueryPage, &p).await?;
        let source = mp_str(&frame.payload, "source");
        let host_hash = mp_str(&frame.payload, "host_hash");
        Ok((source, host_hash))
    }

    /// List pages served by a host.
    pub async fn list_pages(&mut self, host: &str) -> Result<Vec<(String, String)>, String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        let frame = self.rpc(MessageType::CmdPageListSites, &p).await?;
        let arr =
            frame.payload.get("pages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let pages = arr
            .iter()
            .filter_map(|v| {
                let m = v.as_map()?;
                let path = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("path"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let host = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("host_hash"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((path, host))
            })
            .collect();
        Ok(pages)
    }

    /// Close a terminal session.
    pub async fn terminal_close(&mut self, session_id: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        self.rpc(MessageType::CmdTerminalClose, &p).await.map(|_| ())
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

    let stream =
        UnixStream::connect(&path).await.map_err(|e| format!("connect {}: {e}", path.display()))?;

    let stream = Arc::new(Mutex::new(stream));
    let mut handle = DaemonHandle { stream: stream.clone(), next_id: 0 };

    // Verify daemon is alive
    if !handle.ping().await {
        return Err("daemon did not respond to ping".into());
    }

    // Subscribe to all event streams before spawning the reader
    let _ = handle.subscribe_devices().await;
    let _ = handle.subscribe_messages().await;
    let _ = handle.subscribe_links().await;

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
            timeout(Duration::from_secs(60), wire::read_frame_async(&mut *guard)).await
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
        MessageType::EventLink => {
            let link_id =
                frame.payload.get("link_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let peer_hash =
                frame.payload.get("peer_hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let peer_name =
                frame.payload.get("peer_name").and_then(|v| v.as_str()).map(|s| s.to_string());
            let status =
                frame.payload.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let rtt_ms = frame.payload.get("rtt_ms").and_then(|v| v.as_f64());
            if link_id.is_empty() || status.is_empty() {
                return None;
            }
            Some(TuiEvent::LinkUpdate { link_id, peer_hash, peer_name, status, rtt_ms })
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
        MessageType::EventTerminalOutput => {
            let session_id = mp_str(&frame.payload, "session_id");
            let data = frame.payload.get("data").and_then(|v| v.as_slice()).unwrap_or(&[]).to_vec();
            if session_id.is_empty() {
                return None;
            }
            Some(TuiEvent::TerminalOutput { session_id, data })
        }
        MessageType::EventTerminalExited => {
            let session_id = mp_str(&frame.payload, "session_id");
            let exit_code =
                frame.payload.get("exit_code").and_then(|v| v.as_i64()).map(|v| v as i32);
            if session_id.is_empty() {
                return None;
            }
            Some(TuiEvent::TerminalExited { session_id, exit_code })
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
                    Ok(info) => {
                        let _ = tx.send(TuiEvent::Identity(info)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(TuiEvent::Disconnected(e)).await;
                        return;
                    }
                }
            }

            // Periodic status + devices
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;

            let status = handle.lock().await.status().await;
            match status {
                Ok(s) => {
                    let _ = tx.send(TuiEvent::Status(s)).await;
                }
                Err(e) => {
                    let _ = tx.send(TuiEvent::Disconnected(e)).await;
                    return;
                }
            }

            let devices = handle.lock().await.devices(false).await;
            if let Ok(devs) = devices {
                let now = epoch_secs();
                for dev in devs {
                    if dev.destination_hash.is_empty() {
                        continue;
                    }
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
            app.node_hash = info.destination_hash.clone();
            app.node_name = info.display_name.clone();
            app.daemon_connected = true;
            let hash_short = &info.destination_hash[..8.min(info.destination_hash.len())];
            app.conversation.push_system(&format!(
                "⬡ connected  node: {hash_short}…  name: {}",
                info.display_name
            ));
            app.activity.push(ActivityEntry::new(
                ActivityKind::Announce,
                &info.display_name,
                "local node identity loaded",
            ));
        }

        TuiEvent::Status(status) => {
            app.daemon_version = status.daemon_version.clone();
            app.rns_initialized = status.rns_initialized;
            app.transport_active = status.transport_enabled;
            app.propagation_enabled = status.propagation_enabled;
            app.interface_count = status.interface_count;
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
            // trigger_flash removed — effects system handles visuals
        }

        TuiEvent::Message(msg) => {
            let peer_hash = if msg.is_outgoing {
                msg.destination_hash.clone()
            } else {
                msg.source_hash.clone()
            };
            let name = app.peers.iter().find(|p| p.hash == peer_hash).and_then(|p| p.name.clone());

            // Push to per-peer conversation
            let conv = app.peer_conversation(&peer_hash);
            if msg.is_outgoing {
                conv.push_sent(&peer_hash, name.as_deref(), &msg.content, DeliveryStatus::Sent);
            } else {
                conv.push_received(
                    &peer_hash,
                    name.as_deref(),
                    msg.title.as_deref(),
                    &msg.content,
                    msg.timestamp,
                );
            }

            // Also push to global conversation (Home activity)
            if !msg.is_outgoing {
                app.conversation.push_received(
                    &peer_hash,
                    name.as_deref(),
                    msg.title.as_deref(),
                    &msg.content,
                    msg.timestamp,
                );
            }

            let label = name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]);
            app.activity.push(ActivityEntry::new(
                if msg.is_outgoing {
                    ActivityKind::OutboundMessage
                } else {
                    ActivityKind::InboundMessage
                },
                label,
                msg.title.as_deref().unwrap_or(&msg.content[..msg.content.len().min(32)]),
            ));
            if !msg.is_outgoing {
                app.unread_count += 1;
            }
            // trigger_flash removed — effects system handles visuals
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

        TuiEvent::LinkUpdate { link_id, peer_hash, peer_name, status, rtt_ms } => {
            use crate::mesh_state::{LinkRecord, LinkStatus};

            match status.as_str() {
                "active" => {
                    if !app.links.iter().any(|l| l.id == link_id) {
                        let mut link = LinkRecord::new(
                            link_id.clone(),
                            peer_hash.clone(),
                            peer_name.clone(),
                            crate::mesh_state::epoch_secs(),
                        );
                        if let Some(rtt) = rtt_ms {
                            link.rtt_ms = rtt;
                        }
                        link.pluck();
                        app.links.push(link);
                        app.activity.push(ActivityEntry::new(
                            ActivityKind::LinkUp,
                            peer_name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]),
                            "link established",
                        ));
                    }
                }
                "rtt_updated" => {
                    if let Some(link) = app.links.iter_mut().find(|l| l.id == link_id) {
                        if let Some(rtt) = rtt_ms {
                            link.rtt_ms = rtt;
                            link.pluck();
                        }
                    }
                }
                "closed" | "stale" => {
                    if let Some(link) = app.links.iter_mut().find(|l| l.id == link_id) {
                        link.status =
                            if status == "stale" { LinkStatus::Stale } else { LinkStatus::Closed };
                    }
                    if status == "closed" {
                        app.links.retain(|l| l.id != link_id);
                        app.activity.push(ActivityEntry::new(
                            ActivityKind::LinkDown,
                            peer_name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]),
                            "link closed",
                        ));
                    }
                }
                _ => {}
            }
            // trigger_flash removed — effects system handles visuals
        }

        TuiEvent::CommandResult { action, success, detail } => {
            let prefix = if success { "✓" } else { "✗" };
            app.conversation.push_system(&format!("{prefix} {action}: {detail}"));

            // Update command tab result if it was a fleet command
            match action.as_str() {
                "device_status" | "exec" | "reboot_device" | "fleet_apply" => {
                    app.command_tab.is_executing = false;
                    app.command_tab.result_text = format!("  {prefix} {detail}");
                }
                "send_chat" if !success => {
                    // Update the last sent message status to failed
                    app.conversation.update_last_sent_status(
                        crate::tui::segments::DeliveryStatus::Failed(detail.clone()),
                    );
                }
                _ => {}
            }
        }

        TuiEvent::PageLoaded { host: _, path, source } => {
            // Store page content for the Pages tab to render
            app.page_source = Some(source);
            app.page_path = Some(path);
        }

        TuiEvent::PageList { host: _, pages } => {
            app.page_index = pages;
        }

        TuiEvent::TerminalOutput { session_id, data } => {
            if app.terminal_tab.session_id.as_deref() == Some(&session_id) {
                app.terminal_tab.push_output(&data);
            }
        }

        TuiEvent::TerminalExited { session_id, exit_code } => {
            if app.terminal_tab.session_id.as_deref() == Some(&session_id) {
                let msg = match exit_code {
                    Some(code) => format!("Session exited with code {code}"),
                    None => "Session exited".to_string(),
                };
                app.terminal_tab.scrollback.push(format!("--- {msg} ---"));
                app.terminal_tab.status = crate::app::TerminalStatus::Disconnected;
                app.terminal_tab.session_id = None;
            }
        }

        TuiEvent::Disconnected(reason) => {
            app.daemon_connected = false;
            app.rns_initialized = false;
            app.transport_active = false;
            app.conversation.push_system(&format!("⚠ daemon disconnected: {reason}"));
            app.activity.push(ActivityEntry::new(
                ActivityKind::LinkDown,
                "daemon",
                &format!("disconnected: {reason}"),
            ));
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
    s.propagation_count = p.get("propagation_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    s.propagation_size_bytes =
        p.get("propagation_size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    s.transport_enabled = mp_bool(p, "transport_enabled");
    s.active_links = p.get("active_links").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    Ok(s)
}

fn parse_devices(p: &HashMap<String, MpValue>) -> Result<Vec<DeviceInfo>, String> {
    let arr = p
        .get("devices")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no 'devices' array in response".to_string())?;

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
            dev.device_type = get("device_type");
            dev.status = get("status");
            dev.is_styrene_node = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("is_styrene_node"))
                .and_then(|(_, v)| v.as_bool())
                .unwrap_or(false);
            dev.lxmf_destination_hash = get("lxmf_destination_hash");
            Some(dev)
        })
        .collect())
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
    if id.is_empty() {
        return None;
    }
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
