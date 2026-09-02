use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// Operator profile lifecycle: inventory, creation, promotion, snapshots,
/// restore, import, export, adoption, and progress. Every method defaults to
/// `NotImplemented` so backends without managed profiles stay valid.
#[async_trait]
pub trait DaemonProfiles: Send + Sync {
    async fn profile_inventory(&self) -> Result<ProfileInventory, IpcError> {
        Err(IpcError::not_implemented("profile_inventory"))
    }

    async fn create_profile(
        &self,
        request: ProfileCreateRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("create_profile"))
    }

    async fn promote_profile(
        &self,
        request: ProfilePromoteRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("promote_profile"))
    }

    async fn snapshot_profile(
        &self,
        request: ProfileSnapshotRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("snapshot_profile"))
    }

    async fn restore_profile(
        &self,
        request: ProfileRestoreRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("restore_profile"))
    }

    async fn export_profile(
        &self,
        request: ProfileExportRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("export_profile"))
    }

    async fn import_profile(
        &self,
        request: ProfileRestoreRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("import_profile"))
    }

    async fn adopt_profile(
        &self,
        request: ProfileAdoptRequest,
    ) -> Result<ProfileOperationOutcome, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("adopt_profile"))
    }

    async fn profile_operation(
        &self,
        operation_id: &str,
    ) -> Result<ProfileOperationProgress, IpcError> {
        let _ = operation_id;
        Err(IpcError::not_implemented("profile_operation"))
    }
}
