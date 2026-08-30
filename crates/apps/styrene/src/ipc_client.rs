//! IPC client — connects to a running styrened daemon via Unix socket.
//!
//! Provides `DaemonClient` with typed methods matching the `Daemon` trait
//! surface. Uses the same msgpack wire protocol as the TUI and Python clients.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rmpv::Value as MpValue;
use tokio::net::UnixStream;
use tokio::time::Duration;

use styrene_ipc::types::{
    ConversationInfo, DaemonStatusInfo, DeviceInfo, IdentityInfo, MessageInfo, SendChatRequest,
    StandardPropagationSnapshot,
};
use styrene_ipc_client::{Client, ConnectionGeneration};
use styrene_ipc_wire::{Frame, MessageType};

/// Default timeout for RPC calls.
const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub struct CliSendOutcome {
    pub message_id: String,
    pub disposition: String,
    pub paper_uri: Option<String>,
}

/// Client connection to a styrened daemon.
pub struct DaemonClient {
    client: Client,
    /// Override timeout for the next RPC call (reset after use).
    next_timeout: Option<Duration>,
}

impl DaemonClient {
    /// Connect to the daemon via Unix socket.
    pub async fn connect(socket_path: Option<&Path>) -> Result<Self, String> {
        let path_str = socket_path
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| std::env::var("STYRENE_SOCKET").ok())
            .unwrap_or_else(|| default_socket_path().to_string_lossy().to_string());

        if path_str.starts_with("tcp://") {
            return Err("TCP IPC mode has been removed for security reasons. \
                 Use a Unix socket (default) or SSH tunnel for remote access."
                .into());
        }

        let path = PathBuf::from(&path_str);
        if !path.exists() {
            return Err(format!(
                "daemon socket not found: {}\nIs styrene daemon running?",
                path.display()
            ));
        }
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|e| format!("connect {}: {e}", path.display()))?;
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        let generation = ConnectionGeneration(NEXT_GENERATION.fetch_add(1, Ordering::Relaxed));
        let client =
            Self { client: Client::from_unix_stream(stream, generation), next_timeout: None };

        client.client.ping().await.map_err(|error| error.to_string())?;

        Ok(client)
    }

    async fn rpc(
        &mut self,
        msg_type: MessageType,
        payload: &HashMap<String, MpValue>,
    ) -> Result<Frame, String> {
        let rpc_timeout = self.next_timeout.take().unwrap_or(RPC_TIMEOUT);
        self.client.request(msg_type, payload.clone(), rpc_timeout).await.map_err(|e| e.to_string())
    }

    /// Set a custom timeout for the next RPC call only.
    /// Consumed by `take()` inside `rpc()` — if `rpc()` is never called,
    /// the timeout carries over to the next call. Always call `rpc()` after
    /// `with_timeout()`.
    fn with_timeout(&mut self, secs: u64) {
        self.next_timeout = Some(Duration::from_secs(secs.saturating_add(5)));
    }

    pub async fn ping(&mut self) -> bool {
        self.client.ping().await.is_ok()
    }

    pub async fn identity(&mut self) -> Result<IdentityInfo, String> {
        self.client.identity().await.map_err(|error| error.to_string())
    }

    pub async fn status(&mut self) -> Result<DaemonStatusInfo, String> {
        self.client.status().await.map_err(|error| error.to_string())
    }

    pub async fn standard_propagation(&mut self) -> Result<StandardPropagationSnapshot, String> {
        self.client.standard_propagation().await.map_err(|error| error.to_string())
    }

    pub async fn devices(&mut self, styrene_only: bool) -> Result<Vec<DeviceInfo>, String> {
        self.client.devices(styrene_only).await.map_err(|error| error.to_string())
    }

    pub async fn conversations(&mut self) -> Result<Vec<ConversationInfo>, String> {
        self.client.conversations().await.map_err(|error| error.to_string())
    }

    pub async fn messages(
        &mut self,
        peer_hash: &str,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, String> {
        self.client.messages(peer_hash, limit).await.map_err(|error| error.to_string())
    }

    pub async fn send_chat(
        &mut self,
        destination: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        // Daemon dispatch expects "peer_hash" not "destination_hash"
        p.insert("peer_hash".into(), MpValue::String(destination.into()));
        p.insert("content".into(), MpValue::String(content.into()));
        if let Some(t) = title {
            p.insert("title".into(), MpValue::String(t.into()));
        }
        // Send may need to establish an LXMF link — give it more time
        self.with_timeout(30);
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    pub async fn send_chat_outcome(
        &mut self,
        destination: &str,
        content: &str,
        title: Option<&str>,
        delivery_method: &str,
    ) -> Result<CliSendOutcome, String> {
        let mut request = SendChatRequest::default();
        request.peer_hash = destination.into();
        request.content = content.into();
        request.title = title.map(str::to_owned);
        request.delivery_method = Some(delivery_method.into());
        let outcome =
            self.client.send_chat_outcome(&request).await.map_err(|error| error.to_string())?;
        let disposition = match outcome.disposition {
            styrene_ipc::types::SendChatDisposition::Accepted => "accepted",
            styrene_ipc::types::SendChatDisposition::Failed => "failed",
            styrene_ipc::types::SendChatDisposition::PaperExported => "paper_exported",
            styrene_ipc::types::SendChatDisposition::Unknown => "unknown",
            _ => "unknown",
        };
        Ok(CliSendOutcome {
            message_id: outcome.message_id,
            disposition: disposition.into(),
            paper_uri: outcome.paper_uri,
        })
    }

    pub async fn announce(&mut self) -> Result<bool, String> {
        self.client.announce().await.map_err(|error| error.to_string())
    }

    pub async fn config(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryConfig, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    pub async fn path_info(
        &mut self,
        destination: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(destination.into()));
        let frame = self.rpc(MessageType::QueryPathInfo, &p).await?;
        Ok(frame.payload)
    }

    // ── Tunnel operations ───────────────────────────────────────────────

    pub async fn list_tunnels(&mut self) -> Result<Vec<HashMap<String, MpValue>>, String> {
        let frame = self.rpc(MessageType::QueryTunnels, &HashMap::new()).await?;
        let arr = frame
            .payload
            .get("tunnels")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "no tunnels array".to_string())?;
        Ok(arr
            .iter()
            .filter_map(|v| {
                let m = v.as_map()?;
                Some(
                    m.iter()
                        .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
                        .collect(),
                )
            })
            .collect())
    }

    pub async fn tunnel_status_rpc(
        &mut self,
        peer: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::String(peer.into()));
        let frame = self.rpc(MessageType::QueryTunnelStatus, &p).await?;
        Ok(frame.payload)
    }

    pub async fn tunnel_establish(
        &mut self,
        peer: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::String(peer.into()));
        self.with_timeout(30);
        let frame = self.rpc(MessageType::CmdTunnelEstablish, &p).await?;
        Ok(frame.payload)
    }

    pub async fn tunnel_teardown_rpc(
        &mut self,
        peer: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::String(peer.into()));
        let frame = self.rpc(MessageType::CmdTunnelTeardown, &p).await?;
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
        self.with_timeout(timeout_secs);
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
        self.with_timeout(timeout_secs);
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

    pub async fn fleet_apply(
        &mut self,
        dest: &str,
        profile_bytes: &[u8],
        verify: bool,
        timeout_secs: u64,
    ) -> Result<HashMap<String, MpValue>, String> {
        use base64::Engine;
        let profile_b64 = base64::engine::general_purpose::STANDARD.encode(profile_bytes);
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String(dest.into()));
        p.insert("profile".into(), MpValue::String(profile_b64.into()));
        p.insert("verify".into(), MpValue::Boolean(verify));
        p.insert("timeout".into(), MpValue::Integer(rmpv::Integer::from(timeout_secs)));
        self.with_timeout(timeout_secs);
        let frame = self.rpc(MessageType::CmdFleetApply, &p).await?;
        Ok(frame.payload)
    }

    pub async fn fleet_grant(
        &mut self,
        identity_hash: &str,
        role: &str,
        label: &str,
        grants: &[String],
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::String(identity_hash.into()));
        p.insert("role".into(), MpValue::String(role.into()));
        p.insert("label".into(), MpValue::String(label.into()));
        if !grants.is_empty() {
            let mp_grants: Vec<MpValue> =
                grants.iter().map(|g| MpValue::String(g.clone().into())).collect();
            p.insert("grants".into(), MpValue::Array(mp_grants));
        }
        let frame = self.rpc(MessageType::CmdFleetGrant, &p).await?;
        Ok(frame.payload)
    }

    pub async fn fleet_revoke(
        &mut self,
        identity_hash: &str,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::String(identity_hash.into()));
        let frame = self.rpc(MessageType::CmdFleetRevoke, &p).await?;
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
