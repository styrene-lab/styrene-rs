use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// NomadNet/Styrene page browsing.
#[async_trait]
pub trait DaemonPages: Send + Sync {
    /// Fetch a page from a remote node.
    ///
    /// The `host` is empty for local pages or a destination hash. Implementations
    /// must validate the pair with `PageAddress::from_request_parts` before dispatch.
    async fn browse_page(
        &self,
        host: &str,
        path: &str,
        timeout: Option<u64>,
    ) -> Result<PageContent, IpcError>;

    async fn browse_page_for_owner(
        &self,
        owner: u64,
        host: &str,
        path: &str,
        timeout: Option<u64>,
    ) -> Result<PageContent, IpcError> {
        let _ = owner;
        self.browse_page(host, path, timeout).await
    }

    /// Navigate a daemon-owned browse session.
    async fn navigate_page(&self, request: PageNavigationRequest) -> Result<PageContent, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("navigate_page"))
    }

    async fn navigate_page_for_owner(
        &self,
        owner: u64,
        request: PageNavigationRequest,
    ) -> Result<PageContent, IpcError> {
        let _ = owner;
        self.navigate_page(request).await
    }

    /// Close the session connection without changing its current history entry.
    async fn close_page_session(&self, session_id: &str) -> Result<PageNavigationInfo, IpcError> {
        let _ = session_id;
        Err(IpcError::not_implemented("close_page_session"))
    }

    async fn close_page_session_for_owner(
        &self,
        owner: u64,
        session_id: &str,
    ) -> Result<PageNavigationInfo, IpcError> {
        let _ = owner;
        self.close_page_session(session_id).await
    }

    /// Start a bounded native `/file/...` packet/resource transfer.
    async fn start_file_download(
        &self,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = request;
        Err(IpcError::not_implemented("start_file_download"))
    }

    async fn start_file_download_for_owner(
        &self,
        owner: u64,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = owner;
        self.start_file_download(request).await
    }

    async fn file_download(&self, download_id: &str) -> Result<FileDownloadInfo, IpcError> {
        let _ = download_id;
        Err(IpcError::not_implemented("file_download"))
    }

    async fn file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = owner;
        self.file_download(download_id).await
    }

    /// Cancel and return the completed authoritative transfer state.
    async fn cancel_file_download(&self, download_id: &str) -> Result<FileDownloadInfo, IpcError> {
        let _ = download_id;
        Err(IpcError::not_implemented("cancel_file_download"))
    }

    async fn cancel_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = owner;
        self.cancel_file_download(download_id).await
    }

    /// Atomically hand verified daemon-owned bytes to an explicit local path.
    async fn save_file_download(
        &self,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = (download_id, destination);
        Err(IpcError::not_implemented("save_file_download"))
    }

    async fn save_file_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
        destination: &str,
    ) -> Result<FileDownloadInfo, IpcError> {
        let _ = owner;
        self.save_file_download(download_id, destination).await
    }

    async fn cleanup_page_owner(&self, owner: u64) -> Result<(), IpcError> {
        let _ = owner;
        Ok(())
    }

    /// List known pages on a remote node (if the node advertises a page index).
    async fn list_pages(&self, host: &str, timeout: Option<u64>)
    -> Result<Vec<PageInfo>, IpcError>;

    /// List all nodes that advertise page hosting capability.
    async fn page_hosts(&self) -> Result<Vec<DeviceInfo>, IpcError>;
}
