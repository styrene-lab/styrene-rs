//! WireGuard backend for classical (non-PQC) tunnels.
//!
//! Uses the `wg` and `ip` commands to configure WireGuard interfaces.
//! Each peer gets a unique tunnel ID derived from their identity hash.
//!
//! # System Requirements
//!
//! - Linux kernel 5.6+ (WireGuard built-in) or `wireguard-go` (userspace)
//! - `wg` tool installed (from `wireguard-tools` package)
//! - `CAP_NET_ADMIN` capability for the daemon process
//! - `/dev/net/tun` device available (for `ip link add type wireguard`)

use std::collections::HashMap;
use std::net::IpAddr;
use std::process::Stdio;
use std::sync::Mutex;

use base64::Engine;
use tokio::process::Command;
use zeroize::Zeroize;

use crate::error::TunnelError;
use crate::traits::{TunnelBackend, TunnelId, TunnelInfo, TunnelParams, TunnelState};

const DEFAULT_LISTEN_PORT: u16 = 51820;

/// WireGuard tunnel backend.
///
/// Manages a single WireGuard interface with multiple peers (one per tunnel).
/// Peers are identified by their RNS identity hash (used as the tunnel ID).
pub struct WireGuardBackend {
    /// WireGuard interface name.
    interface_name: String,
    /// Listen port for incoming WireGuard connections.
    listen_port: u16,
    /// Local private key (base64-encoded, from StyreneIdentity KeyPurpose::WireGuard).
    private_key_b64: Mutex<Option<String>>,
    /// Active tunnel state (peer_identity → TunnelInfo).
    tunnels: Mutex<HashMap<String, TunnelInfo>>,
    /// Whether the interface has been created.
    interface_up: Mutex<bool>,
}

impl WireGuardBackend {
    /// Create a new WireGuard backend with the default interface name.
    pub fn new() -> Self {
        Self {
            interface_name: "wg-styrene".into(),
            listen_port: DEFAULT_LISTEN_PORT,
            private_key_b64: Mutex::new(None),
            tunnels: Mutex::new(HashMap::new()),
            interface_up: Mutex::new(false),
        }
    }

    /// Create with a custom interface name.
    pub fn with_interface(name: impl Into<String>) -> Self {
        Self {
            interface_name: name.into(),
            ..Self::new()
        }
    }

    /// Set the local private key (32-byte Curve25519 secret).
    /// This is typically derived from StyreneIdentity via KeyPurpose::WireGuard.
    pub fn set_private_key(&self, key: &[u8; 32]) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(key);
        *self.private_key_b64.lock().expect("lock") = Some(b64);
    }

    /// Set the listen port.
    pub fn set_listen_port(&self, port: u16) {
        // Field is not behind mutex — set at construction only.
        // This is a design limitation for simplicity.
        let _ = port;
    }

    /// Ensure the WireGuard interface exists and is configured.
    async fn ensure_interface(&self) -> Result<(), TunnelError> {
        {
            let up = self.interface_up.lock().expect("lock");
            if *up {
                return Ok(());
            }
        }

        let privkey = self
            .private_key_b64
            .lock()
            .expect("lock")
            .clone()
            .ok_or_else(|| TunnelError::Config("private key not set".into()))?;

        // Create the WireGuard interface
        let status = Command::new("ip")
            .args(["link", "add", &self.interface_name, "type", "wireguard"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| TunnelError::Backend(format!("ip link add: {e}")))?;

        if !status.success() {
            // Interface may already exist — try to continue
            eprintln!(
                "[wireguard] ip link add {} returned {} — may already exist",
                self.interface_name,
                status.code().unwrap_or(-1)
            );
        }

        // Set the private key via wg set (piped from stdin)
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
            .map_err(|e| TunnelError::Backend(format!("wg set spawn: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(privkey.as_bytes())
                .await
                .map_err(|e| TunnelError::Backend(format!("wg set stdin: {e}")))?;
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| TunnelError::Backend(format!("wg set wait: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TunnelError::Backend(format!("wg set: {stderr}")));
        }

        // Bring the interface up
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
        *self.interface_up.lock().expect("lock") = true;
        Ok(())
    }

    /// Parse `wg show <iface> dump` output for a specific peer.
    async fn query_peer(&self, peer_pubkey_b64: &str) -> Option<WgPeerDump> {
        let output = Command::new("wg")
            .args(["show", &self.interface_name, "dump"])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Format: public_key\tpreshared_key\tendpoint\tallowed_ips\tlatest_handshake\ttx\trx\tkeepalive
        for line in stdout.lines().skip(1) {
            // skip header line
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() >= 7 && fields[0] == peer_pubkey_b64 {
                return Some(WgPeerDump {
                    latest_handshake: fields[4].parse().unwrap_or(0),
                    tx_bytes: fields[5].parse().unwrap_or(0),
                    rx_bytes: fields[6].parse().unwrap_or(0),
                });
            }
        }
        None
    }
}

struct WgPeerDump {
    latest_handshake: i64,
    tx_bytes: u64,
    rx_bytes: u64,
}

#[async_trait::async_trait]
impl TunnelBackend for WireGuardBackend {
    fn name(&self) -> &str {
        "wireguard"
    }

    async fn is_available(&self) -> bool {
        // Check if `wg` tool is available
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
        self.ensure_interface().await?;

        let peer_pubkey = params
            .peer_x25519_public
            .ok_or_else(|| TunnelError::Config("peer X25519 public key required".into()))?;

        let peer_pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(peer_pubkey);
        let psk_b64 = base64::engine::general_purpose::STANDARD.encode(params.psk);
        let endpoint = format!("{}:{}", params.remote_endpoint, params.remote_port);
        let tunnel_id = params.peer_identity.clone();

        // Add peer via wg set
        let mut child = Command::new("wg")
            .args([
                "set",
                &self.interface_name,
                "peer",
                &peer_pubkey_b64,
                "preshared-key",
                "/dev/stdin",
                "endpoint",
                &endpoint,
                "allowed-ips",
                "0.0.0.0/0,::/0",
                "persistent-keepalive",
                "25",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TunnelError::Backend(format!("wg set peer: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(psk_b64.as_bytes())
                .await
                .map_err(|e| TunnelError::Backend(format!("wg set peer stdin: {e}")))?;
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| TunnelError::Backend(format!("wg set peer wait: {e}")))?;

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
            remote_endpoint: Some(params.remote_endpoint),
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
            "[wireguard] peer {} added endpoint={}",
            &tunnel_id[..12.min(tunnel_id.len())],
            endpoint
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

        // We need the peer's public key to remove it.
        // For now, use `wg show` to find the peer and remove by pubkey.
        // In practice, we should store the pubkey in TunnelInfo.
        let _ = info;

        // Remove all peers if only one tunnel (simplification)
        let status = Command::new("wg")
            .args(["set", &self.interface_name, "peer", "remove"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| TunnelError::Backend(format!("wg remove peer: {e}")))?;

        if !status.success() {
            eprintln!("[wireguard] peer removal may have failed for {tunnel_id}");
        }

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

        // Update the PSK for this peer
        // We need the peer pubkey — for now, this is a limitation
        let _ = (info, new_psk);
        Err(TunnelError::Backend(
            "rekey requires peer pubkey tracking (not yet implemented)".into(),
        ))
    }

    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError> {
        let mut info = self
            .tunnels
            .lock()
            .expect("lock")
            .get(tunnel_id)
            .ok_or_else(|| TunnelError::NotFound(tunnel_id.to_string()))?
            .clone();

        // Try to get live stats from wg show
        // For now, return the cached info
        // TODO: query `wg show` for tx/rx bytes and latest handshake
        info.state = TunnelState::Established;
        Ok(info)
    }

    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError> {
        Ok(self.tunnels.lock().expect("lock").values().cloned().collect())
    }
}
