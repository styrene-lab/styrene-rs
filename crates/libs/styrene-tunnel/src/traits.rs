//! Tunnel backend trait — unified interface for strongSwan and WireGuard.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Unique identifier for a tunnel instance.
pub type TunnelId = String;

/// Information about an active tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelInfo {
    /// Tunnel identifier.
    pub id: TunnelId,
    /// Backend type (e.g., "strongswan", "wireguard").
    pub backend: String,
    /// Remote peer's RNS identity hash (hex).
    pub peer_identity: String,
    /// Remote endpoint IP address.
    pub remote_endpoint: Option<IpAddr>,
    /// Local tunnel interface name (e.g., "ipsec0", "wg-styrene").
    pub interface_name: Option<String>,
    /// Current tunnel state.
    pub state: TunnelState,
    /// Bytes transmitted through this tunnel.
    pub tx_bytes: u64,
    /// Bytes received through this tunnel.
    pub rx_bytes: u64,
    /// Unix timestamp when the tunnel was established.
    pub established_at: Option<i64>,
    /// Unix timestamp of the last rekey event.
    pub last_rekey: Option<i64>,
}

/// Tunnel lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TunnelState {
    /// Tunnel is being set up (SA negotiation, peer configuration).
    Initiating,
    /// Tunnel is active and passing traffic.
    Established,
    /// Tunnel is performing a rekey operation.
    Rekeying,
    /// Tunnel has been torn down gracefully.
    Closed,
    /// Tunnel failed due to an error.
    Failed,
}

/// Parameters for establishing a tunnel.
#[derive(Debug, Clone)]
pub struct TunnelParams {
    /// Remote peer's RNS identity hash.
    pub peer_identity: String,
    /// Remote endpoint address for the tunnel.
    pub remote_endpoint: IpAddr,
    /// Remote endpoint port.
    pub remote_port: u16,
    /// Pre-shared key derived from PQC session (32 bytes).
    pub psk: [u8; 32],
    /// Peer's X25519 public key (for WireGuard), 32 bytes.
    pub peer_x25519_public: Option<[u8; 32]>,
    /// Preferred MTU for the tunnel interface.
    pub mtu: Option<u16>,
}

/// Unified interface for tunnel backends.
///
/// Both strongSwan (IPsec + ML-KEM) and WireGuard implement this trait,
/// allowing the orchestrator to manage tunnels uniformly.
#[cfg(any(feature = "strongswan", feature = "wireguard"))]
#[async_trait::async_trait]
pub trait TunnelBackend: Send + Sync {
    /// Human-readable name of this backend (e.g., "strongswan", "wireguard").
    fn name(&self) -> &str;

    /// Check if this backend is available on the system.
    async fn is_available(&self) -> bool;

    /// Establish a tunnel with the given parameters.
    async fn establish(&self, params: TunnelParams) -> Result<TunnelId, TunnelError>;

    /// Tear down an active tunnel.
    async fn teardown(&self, tunnel_id: &str) -> Result<(), TunnelError>;

    /// Perform a rekey operation on an active tunnel.
    async fn rekey(&self, tunnel_id: &str, new_psk: &[u8; 32]) -> Result<(), TunnelError>;

    /// Get status information for an active tunnel.
    async fn status(&self, tunnel_id: &str) -> Result<TunnelInfo, TunnelError>;

    /// List all active tunnels managed by this backend.
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TunnelError>;
}

/// Tunnel backend that is not yet implemented (compile-time placeholder).
///
/// Used when neither `strongswan` nor `wireguard` features are enabled.
#[cfg(not(any(feature = "strongswan", feature = "wireguard")))]
pub trait TunnelBackend: Send + Sync {
    fn name(&self) -> &str;
}
