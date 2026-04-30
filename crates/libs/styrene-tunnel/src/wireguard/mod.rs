//! WireGuard backend — manages tunnels via the `wg` and `ip` CLI tools.
//!
//! Each peer is a WireGuard peer on a shared interface. Peers are identified
//! by their RNS identity hash. The peer's WireGuard public key is stored in
//! TunnelInfo for teardown and rekey operations.
//!
//! # System Requirements
//!
//! - Linux kernel 5.6+ (WireGuard built-in) or `wireguard-go`
//! - `wg` tool from `wireguard-tools`
//! - `CAP_NET_ADMIN` capability

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;

use base64::Engine;
use tokio::process::Command;

use crate::error::TunnelError;
use crate::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};

const DEFAULT_LISTEN_PORT: u16 = 51820;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// WireGuard tunnel backend.
pub struct WireGuardBackend {
    interface_name: String,
    listen_port: u16,
    private_key_b64: Mutex<Option<String>>,
    tunnels: Mutex<HashMap<String, TunnelInfo>>,
    /// Guarded by tokio::sync::OnceCell pattern — only one init runs.
    interface_initialized: tokio::sync::OnceCell<()>,
}

impl WireGuardBackend {
    pub fn new() -> Self {
        Self {
            interface_name: "wg-styrene".into(),
            listen_port: DEFAULT_LISTEN_PORT,
            private_key_b64: Mutex::new(None),
            tunnels: Mutex::new(HashMap::new()),
            interface_initialized: tokio::sync::OnceCell::new(),
        }
    }

    pub fn with_interface(name: impl Into<String>, port: u16) -> Self {
        Self {
            interface_name: name.into(),
            listen_port: port,
            private_key_b64: Mutex::new(None),
            tunnels: Mutex::new(HashMap::new()),
            interface_initialized: tokio::sync::OnceCell::new(),
        }
    }

    /// Set the local private key (32-byte Curve25519 from StyreneIdentity).
    pub fn set_private_key(&self, key: &[u8; 32]) {
        let b64 = B64.encode(key);
        *self.private_key_b64.lock().expect("lock") = Some(b64);
    }

    /// Initialize the WireGuard interface (idempotent via OnceCell).
    async fn ensure_interface(&self) -> Result<(), TunnelError> {
        self.interface_initialized
            .get_or_try_init(|| async { self.create_interface().await })
            .await?;
        Ok(())
    }

    async fn create_interface(&self) -> Result<(), TunnelError> {
        let privkey = self
            .private_key_b64
            .lock()
            .expect("lock")
            .clone()
            .ok_or_else(|| TunnelError::Config("private key not set".into()))?;

        // Create interface (may already exist — that's fine)
        let _ = Command::new("ip")
            .args(["link", "add", &self.interface_name, "type", "wireguard"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        // Set private key + listen port
        let mut child = Command::new("wg")
            .args([
                "set",
                &self.interface_name,
                "listen-port",
                &self.listen_port.to_string(),
                "private-key",
                "/dev/stdin",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TunnelError::Backend(format!("wg not found: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(privkey.as_bytes()).await.ok();
            drop(stdin);
        }

        let output = child.wait_with_output().await.map_err(|e| {
            TunnelError::Backend(format!("wg set: {e}"))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TunnelError::Backend(format!("wg set: {stderr}")));
        }

        // Bring up
        let status = Command::new("ip")
            .args(["link", "set", &self.interface_name, "up"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| TunnelError::Backend(format!("ip link set up: {e}")))?;

        if !status.success() {
            return Err(TunnelError::Backend("failed to bring interface up".into()));
        }

        eprintln!(
            "[wireguard] interface {} up, listen-port {}",
            self.interface_name, self.listen_port
        );
        Ok(())
    }
}

#[async_trait::async_trait]
impl TunnelBackend for WireGuardBackend {
    fn name(&self) -> &str {
        "wireguard"
    }

    async fn is_available(&self) -> bool {
        Command::new("wg")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn establish(&self, params: TunnelParams) -> Result<TunnelId, TunnelError> {
        if !self.is_available().await {
            return Err(TunnelError::Backend(
                "wg tool not found — install wireguard-tools".into(),
            ));
        }

        self.ensure_interface().await?;

        let peer_pubkey = params
            .peer_x25519_public
            .ok_or_else(|| TunnelError::Config("peer X25519 public key required".into()))?;
        let peer_pubkey_b64 = B64.encode(peer_pubkey);
        let psk_b64 = B64.encode(params.psk);
        let tunnel_id = params.peer_identity.clone();

        // Build allowed-ips from peer's mesh IP (specific route, not catch-all)
        let allowed_ips = match &params.peer_mesh_ip {
            Some(ip) => format!("{ip}/128"),
            None => "0.0.0.0/0,::/0".to_string(), // fallback for unknown mesh IP
        };

        // Build wg set args
        let mut args = vec![
            "set".to_string(),
            self.interface_name.clone(),
            "peer".to_string(),
            peer_pubkey_b64.clone(),
            "preshared-key".to_string(),
            "/dev/stdin".to_string(),
            "allowed-ips".to_string(),
            allowed_ips,
            "persistent-keepalive".to_string(),
            "25".to_string(),
        ];

        // Add endpoint if available (NAT'd peers may not have one)
        if let (Some(ip), Some(port)) = (params.remote_endpoint, params.remote_port) {
            args.push("endpoint".to_string());
            args.push(format!("{ip}:{port}"));
        }

        let mut child = Command::new("wg")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TunnelError::Backend(format!("wg set peer: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(psk_b64.as_bytes()).await.ok();
            drop(stdin);
        }

        let output = child.wait_with_output().await.map_err(|e| {
            TunnelError::Backend(format!("wg set peer: {e}"))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TunnelError::Backend(format!("wg set peer: {stderr}")));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let info = TunnelInfo {
            id: tunnel_id.clone(),
            backend: "wireguard".into(),
            peer_identity: params.peer_identity,
            peer_wg_pubkey: Some(peer_pubkey_b64.clone()),
            peer_mesh_ip: params.peer_mesh_ip,
            remote_endpoint: params.remote_endpoint,
            interface_name: Some(self.interface_name.clone()),
            state: TunnelState::Established,
            tx_bytes: 0,
            rx_bytes: 0,
            established_at: Some(now),
            last_rekey: None,
        };

        self.tunnels
            .lock()
            .expect("lock")
            .insert(tunnel_id.clone(), info);

        eprintln!(
            "[wireguard] peer added: {}",
            &tunnel_id[..12.min(tunnel_id.len())]
        );
        Ok(tunnel_id)
    }

    async fn teardown(&self, tunnel_id: &str) -> Result<(), TunnelError> {
        let info = self
            .tunnels
            .lock()
            .expect("lock")
            .remove(tunnel_id)
            .ok_or_else(|| TunnelError::NotFound(tunnel_id.to_string()))?;

        let pubkey = info
            .peer_wg_pubkey
            .ok_or_else(|| TunnelError::Backend("no peer pubkey stored".into()))?;

        let status = Command::new("wg")
            .args([
                "set",
                &self.interface_name,
                "peer",
                &pubkey,
                "remove",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| TunnelError::Backend(format!("wg remove peer: {e}")))?;

        if !status.success() {
            eprintln!("[wireguard] peer removal may have failed for {tunnel_id}");
        }

        eprintln!(
            "[wireguard] peer removed: {}",
            &tunnel_id[..12.min(tunnel_id.len())]
        );
        Ok(())
    }

    async fn rekey(&self, tunnel_id: &str, new_psk: &[u8; 32]) -> Result<(), TunnelError> {
        let info = self
            .tunnels
            .lock()
            .expect("lock")
            .get(tunnel_id)
            .ok_or_else(|| TunnelError::NotFound(tunnel_id.to_string()))?
            .clone();

        let pubkey = info
            .peer_wg_pubkey
            .ok_or_else(|| TunnelError::Backend("no peer pubkey stored — cannot rekey".into()))?;

        let psk_b64 = B64.encode(new_psk);

        let mut child = Command::new("wg")
            .args([
                "set",
                &self.interface_name,
                "peer",
                &pubkey,
                "preshared-key",
                "/dev/stdin",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TunnelError::Backend(format!("wg rekey: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(psk_b64.as_bytes()).await.ok();
            drop(stdin);
        }

        let output = child.wait_with_output().await.map_err(|e| {
            TunnelError::Backend(format!("wg rekey: {e}"))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TunnelError::Backend(format!("wg rekey: {stderr}")));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if let Some(info) = self.tunnels.lock().expect("lock").get_mut(tunnel_id) {
            info.last_rekey = Some(now);
        }

        eprintln!(
            "[wireguard] rekeyed: {}",
            &tunnel_id[..12.min(tunnel_id.len())]
        );
        Ok(())
    }

    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError> {
        let mut info = self
            .tunnels
            .lock()
            .expect("lock")
            .get(tunnel_id)
            .ok_or_else(|| TunnelError::NotFound(tunnel_id.to_string()))?
            .clone();

        // Query live stats from wg show dump
        if let Some(ref pubkey) = info.peer_wg_pubkey {
            if let Ok(output) = Command::new("wg")
                .args(["show", &self.interface_name, "dump"])
                .output()
                .await
            {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    for line in stdout.lines().skip(1) {
                        let fields: Vec<&str> = line.split('\t').collect();
                        if fields.len() >= 7 && fields[0] == *pubkey {
                            let handshake: i64 = fields[4].parse().unwrap_or(0);
                            info.tx_bytes = fields[5].parse().unwrap_or(0);
                            info.rx_bytes = fields[6].parse().unwrap_or(0);
                            // No handshake in >180s = likely dead
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            if handshake == 0 {
                                info.state = TunnelState::Initiating;
                            } else if now - handshake > 180 {
                                info.state = TunnelState::Failed;
                            } else {
                                info.state = TunnelState::Established;
                            }
                            break;
                        }
                    }
                }
            }
        }

        Ok(info)
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError> {
        Ok(self
            .tunnels
            .lock()
            .expect("lock")
            .values()
            .cloned()
            .collect())
    }
}
