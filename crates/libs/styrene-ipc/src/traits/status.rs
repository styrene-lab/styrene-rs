use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// Daemon health, configuration, and device discovery.
#[async_trait]
pub trait DaemonStatus: Send + Sync {
    /// Query daemon runtime status.
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError>;

    /// Query current daemon configuration.
    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError>;

    /// List discovered devices, optionally filtered to styrene nodes only.
    async fn query_devices(&self, styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError>;

    /// Query path information for a destination hash.
    async fn query_path_info(&self, dest_hash: &str) -> Result<PathInfo, IpcError>;

    /// Query the current auto-reply configuration.
    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError>;

    /// Update auto-reply settings.
    async fn set_auto_reply(
        &self,
        mode: &str,
        message: Option<&str>,
        cooldown_secs: Option<u64>,
    ) -> Result<bool, IpcError>;
}
