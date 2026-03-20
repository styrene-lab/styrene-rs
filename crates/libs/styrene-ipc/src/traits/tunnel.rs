use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::{TunnelInfo, TunnelSaInfo};

/// Tunnel management operations.
///
/// Provides IPC methods for querying and controlling PQC tunnels
/// and their underlying security associations.
#[async_trait]
pub trait DaemonTunnel: Send + Sync {
    /// List all active tunnels.
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, IpcError>;

    /// Get detailed status of a specific tunnel by peer hash.
    async fn tunnel_status(&self, peer_hash: &str) -> Result<TunnelInfo, IpcError>;

    /// Force a rekey of a specific tunnel.
    async fn tunnel_rekey(&self, peer_hash: &str) -> Result<bool, IpcError>;

    /// Tear down a specific tunnel.
    async fn tunnel_teardown(&self, peer_hash: &str) -> Result<bool, IpcError>;

    /// List Security Associations (SAs) for a tunnel.
    async fn list_tunnel_sas(&self, peer_hash: &str) -> Result<Vec<TunnelSaInfo>, IpcError>;
}
