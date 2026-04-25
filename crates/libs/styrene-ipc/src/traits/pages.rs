use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// NomadNet/Styrene page browsing.
#[async_trait]
pub trait DaemonPages: Send + Sync {
    /// Fetch a page from a remote node.
    ///
    /// The `host` is a destination hash. The `path` is the page path
    /// (e.g., "/index", "/status"). Returns rendered page content.
    async fn browse_page(
        &self,
        host: &str,
        path: &str,
        timeout: Option<u64>,
    ) -> Result<PageContent, IpcError>;

    /// List known pages on a remote node (if the node advertises a page index).
    async fn list_pages(&self, host: &str, timeout: Option<u64>)
        -> Result<Vec<PageInfo>, IpcError>;

    /// List all nodes that advertise page hosting capability.
    async fn page_hosts(&self) -> Result<Vec<DeviceInfo>, IpcError>;
}
