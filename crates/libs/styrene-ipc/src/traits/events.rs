use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::IpcError;
use crate::types::{
    DaemonEvent, LinkSnapshot, NetworkOperationInfo, RequestObservationInfo, ResourceTransferInfo,
    StartNetworkOperationInfo, StartRequestInfo,
};

/// Event subscriptions via `tokio::sync::broadcast`.
#[async_trait]
pub trait DaemonEvents: Send + Sync {
    /// Query active links separately from bounded lifecycle history.
    async fn link_snapshot(&self) -> Result<LinkSnapshot, IpcError>;

    /// Subscribe to message events, optionally filtered to specific peers.
    /// An empty slice subscribes to all message events.
    async fn subscribe_messages(
        &self,
        peer_hashes: &[String],
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;

    /// Subscribe to device discovery/status events.
    async fn subscribe_devices(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;

    /// Subscribe to link telemetry events (activated, closed, RTT updated).
    ///
    /// Each event is a `DaemonEvent::Link { event: LinkEvent }`.
    /// Returns a broadcast receiver that sees all link events published by
    /// the daemon's transport layer.
    async fn subscribe_links(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;

    /// Subscribe to authoritative route discovery, loss, and rediscovery events.
    async fn subscribe_routes(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError>;

    /// Subscribe to native Reticulum request progress and terminal observations.
    async fn subscribe_requests(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        Err(IpcError::not_implemented("subscribe_requests"))
    }

    async fn start_request(
        &self,
        _request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, IpcError> {
        Err(IpcError::not_implemented("start_request"))
    }

    async fn request_receipt(
        &self,
        _request_id: &str,
    ) -> Result<Option<RequestObservationInfo>, IpcError> {
        Err(IpcError::not_implemented("request_receipt"))
    }

    async fn request_receipts(&self) -> Result<Vec<RequestObservationInfo>, IpcError> {
        Err(IpcError::not_implemented("request_receipts"))
    }

    async fn cancel_request(&self, _request_id: &str) -> Result<RequestObservationInfo, IpcError> {
        Err(IpcError::not_implemented("cancel_request"))
    }

    async fn resource_transfers(&self) -> Result<Vec<ResourceTransferInfo>, IpcError> {
        Err(IpcError::not_implemented("resource_transfers"))
    }

    async fn cancel_resource(&self, _resource_hash: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("cancel_resource"))
    }

    async fn subscribe_resources(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        Err(IpcError::not_implemented("subscribe_resources"))
    }

    async fn subscribe_network_operations(
        &self,
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        Err(IpcError::not_implemented("subscribe_network_operations"))
    }

    async fn start_network_operation(
        &self,
        _request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, IpcError> {
        Err(IpcError::not_implemented("start_network_operation"))
    }

    async fn network_operation(
        &self,
        _operation_id: &str,
    ) -> Result<Option<NetworkOperationInfo>, IpcError> {
        Err(IpcError::not_implemented("network_operation"))
    }

    async fn network_operations(&self) -> Result<Vec<NetworkOperationInfo>, IpcError> {
        Err(IpcError::not_implemented("network_operations"))
    }

    async fn cancel_network_operation(
        &self,
        _operation_id: &str,
    ) -> Result<NetworkOperationInfo, IpcError> {
        Err(IpcError::not_implemented("cancel_network_operation"))
    }
}
