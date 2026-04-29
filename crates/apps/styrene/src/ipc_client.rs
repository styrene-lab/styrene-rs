//! IPC client — connects to a running styrened daemon via Unix socket.
//!
//! Provides `DaemonClient` with typed methods matching the `Daemon` trait
//! surface. Uses the same msgpack wire protocol as the TUI and Python clients.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rmpv::Value as MpValue;
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

use styrene_ipc::types::{
    ConversationInfo, DaemonStatusInfo, DeviceInfo, IdentityInfo, MessageInfo,
};
use styrene_ipc_server::wire::{self, Frame, MessageType, REQUEST_ID_SIZE};

/// Timeout for RPC calls.
const RPC_TIMEOUT: Duration = Duration::from_secs(5);

/// Client connection to a styrened daemon.
pub struct DaemonClient {
    stream: UnixStream,
    next_id: u64,
}

impl DaemonClient {
    /// Connect to the daemon socket. Returns an error if the socket doesn't
    /// exist or the daemon doesn't respond to a ping.
    pub async fn connect(socket_path: Option<&Path>) -> Result<Self, String> {
        let path = socket_path.map(|p| p.to_path_buf()).unwrap_or_else(default_socket_path);

        if !path.exists() {
            return Err(format!(
                "daemon socket not found: {}\nIs styrene daemon running?",
                path.display()
            ));
        }

        let stream = UnixStream::connect(&path)
            .await
            .map_err(|e| format!("connect {}: {e}", path.display()))?;

        let mut client = Self { stream, next_id: 0 };

        if !client.ping().await {
            return Err("daemon did not respond to ping".into());
        }

        Ok(client)
    }

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

        wire::write_frame_async(&mut self.stream, msg_type, &req_id, payload)
            .await
            .map_err(|e| format!("write: {e}"))?;

        timeout(RPC_TIMEOUT, wire::read_frame_async(&mut self.stream))
            .await
            .map_err(|_| "rpc timeout".to_string())?
            .map_err(|e| format!("read: {e}"))
    }

    pub async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }

    pub async fn identity(&mut self) -> Result<IdentityInfo, String> {
        let frame = self.rpc(MessageType::QueryIdentity, &HashMap::new()).await?;
        parse_identity(&frame.payload)
    }

    pub async fn status(&mut self) -> Result<DaemonStatusInfo, String> {
        let frame = self.rpc(MessageType::QueryStatus, &HashMap::new()).await?;
        parse_status(&frame.payload)
    }

    pub async fn devices(&mut self, styrene_only: bool) -> Result<Vec<DeviceInfo>, String> {
        let mut p = HashMap::new();
        p.insert("styrene_only".into(), MpValue::Boolean(styrene_only));
        let frame = self.rpc(MessageType::QueryDevices, &p).await?;
        parse_devices(&frame.payload)
    }

    pub async fn conversations(&mut self) -> Result<Vec<ConversationInfo>, String> {
        let frame = self.rpc(MessageType::QueryConversations, &HashMap::new()).await?;
        parse_conversations(&frame.payload)
    }

    pub async fn messages(
        &mut self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::String(peer_hash.into()));
        p.insert("limit".into(), MpValue::Integer(limit.into()));
        let frame = self.rpc(MessageType::QueryMessages, &p).await?;
        parse_messages(&frame.payload)
    }

    pub async fn send_chat(
        &mut self,
        destination: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(destination.into()));
        p.insert("content".into(), MpValue::String(content.into()));
        if let Some(t) = title {
            p.insert("title".into(), MpValue::String(t.into()));
        }
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    pub async fn announce(&mut self) -> Result<bool, String> {
        let frame = self.rpc(MessageType::CmdAnnounce, &HashMap::new()).await?;
        Ok(mp_bool(&frame.payload, "success"))
    }

    pub async fn config(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryConfig, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    // ── Fleet operations ────────────────────────────────────────────────

    pub async fn device_status(
        &mut self,
        dest: &str,
        timeout_secs: u64,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(dest.into()));
        p.insert("timeout".into(), MpValue::Integer(timeout_secs.into()));
        let frame = self.rpc(MessageType::CmdDeviceStatus, &p).await?;
        Ok(frame.payload)
    }

    pub async fn exec(
        &mut self,
        dest: &str,
        cmd: &str,
        args: &[String],
        timeout_secs: u64,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(dest.into()));
        p.insert("command".into(), MpValue::String(cmd.into()));
        let mp_args: Vec<MpValue> =
            args.iter().map(|a| MpValue::String(a.clone().into())).collect();
        p.insert("args".into(), MpValue::Array(mp_args));
        p.insert("timeout".into(), MpValue::Integer(timeout_secs.into()));
        let frame = self.rpc(MessageType::CmdExec, &p).await?;
        Ok(frame.payload)
    }

    pub async fn reboot_device(
        &mut self,
        dest: &str,
        delay_secs: u64,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(dest.into()));
        p.insert("delay".into(), MpValue::Integer(delay_secs.into()));
        let frame = self.rpc(MessageType::CmdRebootDevice, &p).await?;
        Ok(frame.payload)
    }
}

fn default_socket_path() -> PathBuf {
    styrene_ipc_server::default_socket_path()
}

// ── Payload parsers ─────────────────────────────────────────────────────────

fn mp_str(p: &HashMap<String, MpValue>, key: &str) -> String {
    p.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn mp_bool(p: &HashMap<String, MpValue>, key: &str) -> bool {
    p.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn mp_u64(p: &HashMap<String, MpValue>, key: &str) -> u64 {
    p.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn mp_i64(p: &HashMap<String, MpValue>, key: &str) -> i64 {
    p.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
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
    s.device_count = mp_u64(p, "device_count") as u32;
    s.interface_count = mp_u64(p, "interface_count") as u32;
    s.hub_status = p.get("hub_status").and_then(|v| v.as_str()).map(|s| s.to_string());
    s.propagation_enabled = mp_bool(p, "propagation_enabled");
    s.transport_enabled = mp_bool(p, "transport_enabled");
    s.active_links = mp_u64(p, "active_links") as u32;
    Ok(s)
}

fn parse_devices(p: &HashMap<String, MpValue>) -> Result<Vec<DeviceInfo>, String> {
    let arr = p
        .get("devices")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no 'devices' array in response".to_string())?;

    Ok(arr.iter().filter_map(parse_device_value).collect())
}

fn parse_device_value(v: &MpValue) -> Option<DeviceInfo> {
    let m = v.as_map()?;
    let get = |key: &str| -> String {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_bool = |key: &str| -> bool {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_bool())
            .unwrap_or(false)
    };
    let mut dev = DeviceInfo::default();
    dev.destination_hash = get("destination_hash");
    dev.identity_hash = get("identity_hash");
    dev.name = get("name");
    dev.device_type = get("device_type");
    dev.status = get("status");
    dev.is_styrene_node = get_bool("is_styrene_node");
    dev.lxmf_destination_hash = get("lxmf_destination_hash");
    Some(dev)
}

fn parse_conversations(p: &HashMap<String, MpValue>) -> Result<Vec<ConversationInfo>, String> {
    let arr = p
        .get("conversations")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no 'conversations' array".to_string())?;

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
            let get_opt = |key: &str| -> Option<String> {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_str())
                    .map(|s| s.to_string())
            };
            let get_u32 = |key: &str| -> u32 {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_u64())
                    .unwrap_or(0) as u32
            };
            let get_i64 = |key: &str| -> Option<i64> {
                m.iter().find(|(k, _)| k.as_str() == Some(key)).and_then(|(_, v)| v.as_i64())
            };
            let mut c = ConversationInfo::default();
            c.peer_hash = get("peer_hash");
            c.peer_name = get_opt("peer_name");
            c.last_message_content = get_opt("last_message_content");
            c.last_message_timestamp = get_i64("last_message_timestamp");
            c.unread_count = get_u32("unread_count");
            c.message_count = get_u32("message_count");
            Some(c)
        })
        .collect())
}

fn parse_messages(p: &HashMap<String, MpValue>) -> Result<Vec<MessageInfo>, String> {
    let arr = p
        .get("messages")
        .or_else(|| p.get("result"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no 'messages' array".to_string())?;

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
            let get_bool = |key: &str| -> bool {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_bool())
                    .unwrap_or(false)
            };
            let get_i64 = |key: &str| -> i64 {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_i64())
                    .unwrap_or(0)
            };
            let mut msg = MessageInfo::default();
            msg.id = get("id");
            if msg.id.is_empty() {
                return None;
            }
            msg.source_hash = get("source_hash");
            msg.destination_hash = get("destination_hash");
            msg.timestamp = get_i64("timestamp");
            msg.content = get("content");
            msg.title = m
                .iter()
                .find(|(k, _)| k.as_str() == Some("title"))
                .and_then(|(_, v)| v.as_str())
                .map(|s| s.to_string());
            msg.status = get("status");
            msg.is_outgoing = get_bool("is_outgoing");
            msg.read = get_bool("read");
            Some(msg)
        })
        .collect())
}
