use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::{
    IdentityBackupExport, IdentityBackupImport, IdentityBackupMetadata, IdentityInfo,
    IdentityRestoreOutcome,
};

/// Local node identity management.
#[async_trait]
pub trait DaemonIdentity: Send + Sync {
    /// Query the local node's identity.
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError>;

    /// Query non-secret metadata without crossing the encrypted artifact boundary.
    async fn query_identity_backup_metadata(&self) -> Result<IdentityBackupMetadata, IpcError> {
        Err(IpcError::not_implemented("query_identity_backup_metadata"))
    }

    /// Authenticate custody and export its opaque encrypted artifact.
    async fn export_identity_backup(&self) -> Result<IdentityBackupExport, IpcError> {
        Err(IpcError::not_implemented("export_identity_backup"))
    }

    /// Authenticate and restore an opaque artifact without replacing another identity.
    async fn restore_identity_backup(
        &self,
        _backup: IdentityBackupImport,
    ) -> Result<IdentityRestoreOutcome, IpcError> {
        Err(IpcError::not_implemented("restore_identity_backup"))
    }

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
