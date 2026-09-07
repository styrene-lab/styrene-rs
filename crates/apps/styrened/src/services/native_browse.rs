//! Daemon transport and IPC adaptation for the NomadNet coordinator.
use super::{
    DiscoveryService,
    nomadnet_conversion::{IntoDomain, IntoIpc},
};
use crate::transport::mesh_transport::{LinkOpenResult, MeshTransport, TransportError};
use async_trait::async_trait;
use rns_core::{
    destination::DestinationDesc,
    hash::AddressHash,
    identity::{Identity, PrivateIdentity},
};
use std::{
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};
use styrene_ipc::types::{
    DiscoveredCapability, FileDownloadInfo, FileDownloadRequest, PageContent, PageNavigationInfo,
    PageNavigationRequest, StartRequestInfo,
};
use styrene_nomadnet::{
    coordinator::{
        BrowseBackend, BrowseError, BrowserLink, Discovery, NativeRequest, NativeRequestOutcome,
    },
    models::{RequestObservationInfo, RequestProtocolError, RequestState},
};
const MAX_CLEANUP_BUDGET: Duration = Duration::from_millis(50);
fn transport_error(error: TransportError) -> BrowseError {
    BrowseError::Transport(error.to_string())
}
struct DiscoveryAdapter(Arc<DiscoveryService>);
impl Discovery for DiscoveryAdapter {
    fn native_host(&self, host: &str) -> Option<bool> {
        self.0
            .device(host)
            .map(|d| d.discovered_capabilities.contains(&DiscoveredCapability::NativeNomadNetHost))
    }
}
struct TransportBrowseBackend {
    transport: Arc<dyn MeshTransport>,
    identity: RwLock<Option<Arc<PrivateIdentity>>>,
}

#[async_trait]
impl BrowseBackend for TransportBrowseBackend {
    fn identification_enabled(&self) -> bool {
        self.identity.read().unwrap_or_else(|e| e.into_inner()).is_some()
    }
    async fn discover_path(
        &self,
        destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError> {
        if tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            result = tokio::time::timeout_at(deadline, self.transport.query_path(&destination)) => result,
        }
            .map_err(|_| BrowseError::Deadline)?
            .is_some()
        {
            return Ok(());
        }
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            result = tokio::time::timeout_at(deadline, self.transport.request_path(&destination)) => {
                result.map_err(|_| BrowseError::Deadline)?;
            }
        }
        while tokio::time::Instant::now() < deadline {
            if tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                result = tokio::time::timeout_at(deadline, self.transport.query_path(&destination)) => result,
            }
                .map_err(|_| BrowseError::Deadline)?
                .is_some()
            {
                return Ok(());
            }
            tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
        Err(BrowseError::Transport("path discovery timed out".into()))
    }

    async fn resolve_identity(
        &self,
        destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Identity, BrowseError> {
        while tokio::time::Instant::now() < deadline {
            if let Some(identity) = tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                result = tokio::time::timeout_at(deadline, self.transport.resolve_identity(&destination)) => result,
            }
                .map_err(|_| BrowseError::Deadline)?
            {
                return Ok(identity);
            }
            tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
        Err(BrowseError::Transport("identity resolution timed out".into()))
    }

    async fn open_link(
        &self,
        destination: DestinationDesc,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserLink, BrowseError> {
        let remaining = remaining(deadline)?;
        let disposition = self
            .transport
            .open_native_nomadnet_link(destination, cancellation.clone(), remaining)
            .await
            .map_err(transport_error)?;
        Ok(match disposition {
            LinkOpenResult::Created(link_id) => {
                BrowserLink { id: hex::encode(link_id.as_slice()), created: true }
            }
            LinkOpenResult::Reused(link_id) => {
                BrowserLink { id: hex::encode(link_id.as_slice()), created: false }
            }
        })
    }

    async fn identify_link(
        &self,
        link_id: &str,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError> {
        let identity = self
            .identity
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| BrowseError::Transport("no local RNS identity selected".into()))?;
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            result = tokio::time::timeout_at(
                deadline,
                self.transport.identify_native_nomadnet_link(link_id, &identity),
            ) => result.map_err(|_| BrowseError::Deadline)?.map_err(transport_error)?,
        }
        Ok(())
    }

    async fn request(&self, request: NativeRequest) -> Result<NativeRequestOutcome, BrowseError> {
        let NativeRequest {
            link_id,
            path,
            correlation_id,
            data,
            max_response_size,
            cancellation: cancellation_token,
            progress,
            deadline,
        } = request;
        let timeout = remaining(deadline)?;
        let cleanup_budget = (timeout / 10).clamp(Duration::from_millis(1), MAX_CLEANUP_BUDGET);
        let cancellation_deadline = deadline.checked_sub(cleanup_budget).unwrap_or(deadline);
        let mut request = StartRequestInfo::default();
        request.link_id = link_id;
        request.path = path;
        request.data = data;
        request.timeout_ms = timeout.as_millis().try_into().unwrap_or(u64::MAX);
        request.max_response_size = max_response_size;
        request.correlation_id = Some(correlation_id.clone());
        let mut cancellation = ActiveRequest::new(self.transport.clone(), correlation_id);
        let started = tokio::select! {
            () = cancellation_token.cancelled() => return Err(BrowseError::Cancelled),
            result = tokio::time::timeout_at(deadline, self.transport.start_request(request)) => {
                result.map_err(|_| BrowseError::Deadline)?.map_err(transport_error)?
            }
        };
        let started = started.into_domain();
        cancellation.set_request_id(started.request_id.clone());
        while tokio::time::Instant::now() < cancellation_deadline {
            let receipt = tokio::select! {
                () = cancellation_token.cancelled() => break,
                receipt = tokio::time::timeout_at(
                    cancellation_deadline,
                    self.transport.request_receipt(&started.request_id),
                ) => match receipt {
                    Ok(receipt) => receipt.map_err(transport_error)?.ok_or(BrowseError::MissingReceipt)?.into_domain(),
                    Err(_) => break,
                }
            };
            if let Some(progress) = &progress {
                progress(receipt.clone());
            }
            if receipt.state.is_terminal() {
                cancellation.disarm();
                return Ok(NativeRequestOutcome { started, completed: receipt });
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_millis(10))
                    .min(cancellation_deadline),
            )
            .await;
        }
        let completed = match tokio::time::timeout_at(
            deadline,
            self.transport.cancel_request(&started.request_id),
        )
        .await
        {
            Ok(completed) => {
                let completed = completed.map_err(transport_error)?.into_domain();
                cancellation.disarm();
                completed
            }
            Err(_) => timed_out_receipt(&started),
        };
        Ok(NativeRequestOutcome { started, completed })
    }

    async fn close_link(&self, link_id: &str) -> Result<(), BrowseError> {
        let bytes: [u8; 16] = hex::decode(link_id)
            .map_err(|_| BrowseError::Transport("invalid native link id".into()))?
            .try_into()
            .map_err(|_| BrowseError::Transport("invalid native link id".into()))?;
        self.transport.close_link(&AddressHash::new(bytes)).await.map_err(transport_error)?;
        Ok(())
    }
}

struct ActiveRequest {
    transport: Arc<dyn MeshTransport>,
    request_id: Option<String>,
    correlation_id: String,
    armed: bool,
}

impl ActiveRequest {
    fn new(transport: Arc<dyn MeshTransport>, correlation_id: String) -> Self {
        Self { transport, request_id: None, correlation_id, armed: true }
    }

    fn set_request_id(&mut self, request_id: String) {
        self.request_id = Some(request_id);
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let request_id = self.request_id.take();
        let transport = self.transport.clone();
        let correlation_id = self.correlation_id.clone();
        tokio::spawn(async move {
            if let Some(request_id) = request_id {
                let _ = transport.cancel_request(&request_id).await;
            } else {
                let _ = transport.cancel_requests_by_correlation(&correlation_id).await;
            }
        });
    }
}

fn remaining(deadline: tokio::time::Instant) -> Result<Duration, BrowseError> {
    deadline.checked_duration_since(tokio::time::Instant::now()).ok_or(BrowseError::Deadline)
}

fn timed_out_receipt(started: &RequestObservationInfo) -> RequestObservationInfo {
    let mut completed = started.clone();
    completed.state = RequestState::TimedOut;
    completed.protocol_error = Some(RequestProtocolError::Timeout);
    completed.response = None;
    completed
}

/// Composition adapter; all session/cache/download ownership lives in the domain crate.
pub struct NativeNomadNetBrowseCoordinator {
    backend: Arc<TransportBrowseBackend>,
    inner: Arc<styrene_nomadnet::coordinator::NativeNomadNetBrowseCoordinator>,
}
impl NativeNomadNetBrowseCoordinator {
    pub fn new(transport: Arc<dyn MeshTransport>, discovery: Arc<DiscoveryService>) -> Self {
        let backend = Arc::new(TransportBrowseBackend { transport, identity: RwLock::new(None) });
        Self {
            inner: Arc::new(styrene_nomadnet::coordinator::NativeNomadNetBrowseCoordinator::new(
                backend.clone(),
                Arc::new(DiscoveryAdapter(discovery)),
            )),
            backend,
        }
    }
    pub async fn navigate(
        &self,
        request: PageNavigationRequest,
        host: &str,
        source: impl FnOnce(&str) -> Vec<u8>,
    ) -> Result<PageContent, String> {
        self.navigate_for_owner(0, request, host, source).await
    }
    pub async fn close_session(&self, id: &str) -> Result<PageNavigationInfo, String> {
        self.close_session_for_owner(0, id).await
    }
    pub async fn start_download(
        &self,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, String> {
        self.start_download_for_owner(0, request).await
    }
    pub async fn download(&self, id: &str) -> Option<FileDownloadInfo> {
        self.download_for_owner(0, id).await
    }
    pub async fn cancel_download(&self, id: &str) -> Option<FileDownloadInfo> {
        self.cancel_download_for_owner(0, id).await
    }
    pub async fn save_download(&self, id: &str, path: &Path) -> Result<FileDownloadInfo, String> {
        self.save_download_for_owner(0, id, path).await
    }
    pub fn project_local(&self, host: &str, path: &str, source: Vec<u8>) -> PageContent {
        self.inner.project_local(host, path, source).into_ipc()
    }
    pub fn set_identity(&self, identity: Arc<PrivateIdentity>) {
        *self.backend.identity.write().unwrap_or_else(|e| e.into_inner()) = Some(identity)
    }
    pub async fn navigate_for_owner(
        &self,
        owner: u64,
        request: PageNavigationRequest,
        host: &str,
        source: impl FnOnce(&str) -> Vec<u8>,
    ) -> Result<PageContent, String> {
        self.inner
            .navigate_for_owner(owner, request.into_domain(), host, source)
            .await
            .map(IntoIpc::into_ipc)
    }
    pub async fn close_session_for_owner(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<PageNavigationInfo, String> {
        self.inner.close_session_for_owner(owner, id).await.map(IntoIpc::into_ipc)
    }
    pub async fn start_download_for_owner(
        &self,
        owner: u64,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, String> {
        self.inner
            .start_download_for_owner(owner, request.into_domain())
            .await
            .map(IntoIpc::into_ipc)
    }
    pub async fn download_for_owner(&self, owner: u64, id: &str) -> Option<FileDownloadInfo> {
        self.inner.download_for_owner(owner, id).await.map(IntoIpc::into_ipc)
    }
    pub async fn cancel_download_for_owner(
        &self,
        owner: u64,
        id: &str,
    ) -> Option<FileDownloadInfo> {
        self.inner.cancel_download_for_owner(owner, id).await.map(IntoIpc::into_ipc)
    }
    pub async fn save_download_for_owner(
        &self,
        owner: u64,
        id: &str,
        path: &Path,
    ) -> Result<FileDownloadInfo, String> {
        self.inner.save_download_for_owner(owner, id, path).await.map(IntoIpc::into_ipc)
    }
    pub async fn cleanup_owner(&self, owner: u64) -> Result<(), String> {
        self.inner.cleanup_owner(owner).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::mock_transport::{MockCall, MockTransport};
    const MAX_ENCODED_NATIVE_RESPONSE_SIZE: u64 = 1024 * 1024 + 6;
    #[tokio::test]
    async fn dropping_active_request_cancels_correlated_request_and_resources() {
        let transport = Arc::new(MockTransport::new_default());
        let mut guard = ActiveRequest::new(transport.clone(), "page-cancel".into());
        guard.set_request_id("44".repeat(16));

        drop(guard);
        tokio::task::yield_now().await;

        assert!(transport.calls().iter().any(|call| {
            matches!(call, MockCall::CancelRequest { request_id } if request_id == &"44".repeat(16))
        }));
    }

    #[tokio::test]
    async fn dropping_request_during_startup_cancels_by_page_correlation() {
        let transport = Arc::new(MockTransport::new_default());
        let guard = ActiveRequest::new(transport.clone(), "page-startup".into());

        drop(guard);
        tokio::task::yield_now().await;

        assert!(transport.calls().iter().any(|call| matches!(
            call,
            MockCall::CancelRequestsByCorrelation { correlation_id }
                if correlation_id == "page-startup"
        )));
    }

    #[tokio::test]
    async fn request_deadline_cancels_active_transfer_with_ipc_safe_limit_and_correlation() {
        let transport = Arc::new(MockTransport::new_default());
        let backend =
            TransportBrowseBackend { transport: transport.clone(), identity: RwLock::new(None) };

        let outcome = backend
            .request(NativeRequest {
                link_id: "22".repeat(16),
                path: "/page/index.mu".into(),
                correlation_id: "page-deadline".into(),
                data: vec![0xc0],
                max_response_size: MAX_ENCODED_NATIVE_RESPONSE_SIZE,
                cancellation: tokio_util::sync::CancellationToken::new(),
                progress: None,
                deadline: tokio::time::Instant::now() + Duration::from_millis(5),
            })
            .await
            .expect("deadline returns cancelled receipt");

        assert!(matches!(
            outcome.completed.state,
            RequestState::Cancelled | RequestState::TimedOut
        ));
        assert!(transport.calls().iter().any(|call| matches!(
            call,
            MockCall::StartRequest {
                correlation_id: Some(value),
                max_response_size: MAX_ENCODED_NATIVE_RESPONSE_SIZE,
            } if value == "page-deadline"
        )));
        assert!(transport.calls().iter().any(|call| {
            matches!(call, MockCall::CancelRequest { request_id } if request_id == &"55".repeat(16))
        }));
    }

    #[tokio::test]
    async fn request_deadline_does_not_wait_for_slow_cancellation_cleanup() {
        let transport = Arc::new(MockTransport::new_default());
        transport.set_cancel_request_delay(Duration::from_secs(1));
        let backend =
            TransportBrowseBackend { transport: transport.clone(), identity: RwLock::new(None) };
        let started_at = tokio::time::Instant::now();

        let outcome = backend
            .request(NativeRequest {
                link_id: "22".repeat(16),
                path: "/page/index.mu".into(),
                correlation_id: "page-slow-cancel".into(),
                data: vec![0xc0],
                max_response_size: MAX_ENCODED_NATIVE_RESPONSE_SIZE,
                cancellation: tokio_util::sync::CancellationToken::new(),
                progress: None,
                deadline: started_at + Duration::from_millis(30),
            })
            .await
            .expect("deadline returns timeout receipt");

        assert!(started_at.elapsed() < Duration::from_millis(100));
        assert_eq!(outcome.completed.state, RequestState::TimedOut);
        assert_eq!(outcome.completed.protocol_error, Some(RequestProtocolError::Timeout));
        tokio::task::yield_now().await;
        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|call| {
                    matches!(call, MockCall::CancelRequest { request_id } if request_id == &"55".repeat(16))
                })
                .count(),
            2
        );
    }
}
