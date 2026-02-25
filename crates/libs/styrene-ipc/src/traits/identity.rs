use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::IdentityInfo;

/// Local node identity management.
#[async_trait]
pub trait DaemonIdentity: Send + Sync {
    /// Query the local node's identity.
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError>;

    /// Update identity fields. `None` leaves a field unchanged.
    async fn set_identity(
        &self,
        display_name: Option<&str>,
        icon: Option<&str>,
        short_name: Option<&str>,
    ) -> Result<bool, IpcError>;

    /// Broadcast an identity announce to the network.
    async fn announce(&self) -> Result<bool, IpcError>;
}
