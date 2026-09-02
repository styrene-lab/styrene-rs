//! Native NomadNet browse coordination and rendering projection.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::identity::{Identity, PrivateIdentity};
use sha2::{Digest, Sha256};
use styrene_ipc::PageAddress;
use styrene_ipc::types::{
    DiscoveredCapability, FileDownloadInfo, FileDownloadRequest, FileDownloadState,
    ObservationMetadata, ObservationSource, PageBrowseFailure, PageBrowseOutcome, PageBrowseStage,
    PageBrowseStageKind, PageBrowseStageState, PageCacheStatus, PageContent, PageFormField,
    PageFormFieldKind, PageFormSubmission, PageLinkTarget, PageNavigationAction,
    PageNavigationInfo, PageNavigationRequest, PageParserWarning, PageTransferInfo,
    PageTransferKind, RequestObservationInfo, RequestProtocolError, RequestResponseTransfer,
    RequestState, StartRequestInfo,
};
use styrene_micron::{Block, ChildBlock, Document, FormField, InlineNode, Line};
use tokio::io::AsyncWriteExt;

use super::DiscoveryService;
use crate::transport::mesh_transport::{LinkOpenResult, MeshTransport, TransportError};

const MAX_PAGE_SOURCE_SIZE: usize = 1024 * 1024;
const MAX_ENCODED_NATIVE_RESPONSE_SIZE: u64 = MAX_PAGE_SOURCE_SIZE as u64 + 6;
const MAX_FILE_SIZE: usize = 32 * 1024 * 1024;
const MAX_ENCODED_FILE_RESPONSE_SIZE: u64 = MAX_FILE_SIZE as u64 + 6;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 32;
const MAX_HISTORY_ENTRIES: usize = 64;
const MAX_SESSIONS: usize = 16;
const MAX_DOWNLOADS: usize = 8;
const MAX_CLEANUP_BUDGET: Duration = Duration::from_millis(50);
const MAX_LINK_CLEANUP_ATTEMPTS: u8 = 3;
const MAX_LINK_CLEANUP_RECORDS: usize = 64;
#[cfg(not(test))]
const DOWNLOAD_CANCELLATION_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const DOWNLOAD_CANCELLATION_WAIT: Duration = Duration::from_millis(50);
static NEXT_CORRELATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, thiserror::Error)]
enum BrowseError {
    #[error("{0}")]
    Transport(String),
    #[error("native request receipt disappeared")]
    MissingReceipt,
    #[error("browse operation deadline elapsed")]
    Deadline,
    #[error("browse operation cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BrowserLink {
    id: String,
    created: bool,
}

impl From<TransportError> for BrowseError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error.to_string())
    }
}

#[derive(Clone)]
struct NativeRequestOutcome {
    started: RequestObservationInfo,
    completed: RequestObservationInfo,
}

struct FetchedPage {
    page: PageContent,
    link: Option<BrowserLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LinkCleanupStatus {
    Pending { attempts: u8 },
    Completed { attempts: u8 },
    TerminalError { attempts: u8, error: String },
}

struct LinkCleanupSupervisor {
    backend: Arc<dyn BrowseBackend>,
    states: tokio::sync::Mutex<HashMap<String, LinkCleanupStatus>>,
    serial: tokio::sync::Mutex<()>,
}

impl LinkCleanupSupervisor {
    fn new(backend: Arc<dyn BrowseBackend>) -> Self {
        Self {
            backend,
            states: tokio::sync::Mutex::new(HashMap::new()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    async fn cleanup(&self, link: BrowserLink) -> Result<(), BrowseError> {
        if !link.created {
            return Ok(());
        }
        let _serial = self.serial.lock().await;
        {
            let mut states = self.states.lock().await;
            if matches!(states.get(&link.id), Some(LinkCleanupStatus::Completed { .. })) {
                return Ok(());
            }
            if states.len() >= MAX_LINK_CLEANUP_RECORDS {
                let removable = states
                    .iter()
                    .find(|(_, status)| !matches!(status, LinkCleanupStatus::Pending { .. }))
                    .map(|(id, _)| id.clone());
                if let Some(id) = removable {
                    states.remove(&id);
                }
            }
            states.insert(link.id.clone(), LinkCleanupStatus::Pending { attempts: 0 });
        }
        for attempt in 1..=MAX_LINK_CLEANUP_ATTEMPTS {
            self.states
                .lock()
                .await
                .insert(link.id.clone(), LinkCleanupStatus::Pending { attempts: attempt });
            match self.backend.close_link(&link.id).await {
                Ok(()) => {
                    self.states
                        .lock()
                        .await
                        .insert(link.id, LinkCleanupStatus::Completed { attempts: attempt });
                    log::debug!(
                        "native browser created-link cleanup completed in {attempt} attempt(s)"
                    );
                    return Ok(());
                }
                Err(error) if attempt < MAX_LINK_CLEANUP_ATTEMPTS => {
                    log::warn!(
                        "native browser link cleanup attempt {attempt} failed for {}: {error}",
                        link.id
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.states.lock().await.insert(
                        link.id.clone(),
                        LinkCleanupStatus::TerminalError {
                            attempts: attempt,
                            error: message.clone(),
                        },
                    );
                    return Err(BrowseError::Transport(format!(
                        "created link {} cleanup failed after {attempt} attempts: {message}",
                        link.id
                    )));
                }
            }
        }
        unreachable!("bounded cleanup loop always returns")
    }

    #[cfg(test)]
    async fn status(&self, link_id: &str) -> Option<LinkCleanupStatus> {
        self.states.lock().await.get(link_id).cloned()
    }
}

struct UnretainedLink {
    link: Option<BrowserLink>,
    cleanup: Arc<LinkCleanupSupervisor>,
    owner_cleanup: Arc<std::sync::Mutex<HashMap<u64, HashMap<String, BrowserLink>>>>,
    owner: u64,
}

impl UnretainedLink {
    fn new(
        link: BrowserLink,
        cleanup: Arc<LinkCleanupSupervisor>,
        owner_cleanup: Arc<std::sync::Mutex<HashMap<u64, HashMap<String, BrowserLink>>>>,
        owner: u64,
    ) -> Self {
        Self { link: Some(link), cleanup, owner_cleanup, owner }
    }

    fn link(&self) -> &BrowserLink {
        match self.link.as_ref() {
            Some(link) => link,
            None => unreachable!("unretained link is present until ownership transfer"),
        }
    }

    fn take(mut self) -> BrowserLink {
        match self.link.take() {
            Some(link) => link,
            None => unreachable!("unretained link is present until ownership transfer"),
        }
    }
}

impl Drop for UnretainedLink {
    fn drop(&mut self) {
        if let Some(link) = self.link.take() {
            if !link.created {
                return;
            }
            self.owner_cleanup
                .lock()
                .unwrap_or_else(|value| value.into_inner())
                .entry(self.owner)
                .or_default()
                .insert(link.id.clone(), link.clone());
            let cleanup = Arc::clone(&self.cleanup);
            let owner_cleanup = Arc::clone(&self.owner_cleanup);
            let owner = self.owner;
            tokio::spawn(async move {
                match cleanup.cleanup(link.clone()).await {
                    Ok(()) => remove_owned_cleanup(&owner_cleanup, owner, &link.id),
                    Err(error) => {
                        log::error!(
                            "native browser created-link cleanup reached terminal error: {error}"
                        );
                    }
                }
            });
        }
    }
}

fn remove_owned_cleanup(
    owners: &std::sync::Mutex<HashMap<u64, HashMap<String, BrowserLink>>>,
    owner: u64,
    link_id: &str,
) {
    let mut owners = owners.lock().unwrap_or_else(|value| value.into_inner());
    let empty = owners.get_mut(&owner).is_some_and(|links| {
        links.remove(link_id);
        links.is_empty()
    });
    if empty {
        owners.remove(&owner);
    }
}

struct NativeRequest {
    link_id: String,
    path: String,
    correlation_id: String,
    data: Vec<u8>,
    max_response_size: u64,
    cancellation: tokio_util::sync::CancellationToken,
    progress: Option<Arc<dyn Fn(RequestObservationInfo) + Send + Sync>>,
    deadline: tokio::time::Instant,
}

#[async_trait]
trait BrowseBackend: Send + Sync {
    async fn discover_path(
        &self,
        destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError>;
    async fn resolve_identity(
        &self,
        destination: AddressHash,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Identity, BrowseError>;
    async fn open_link(
        &self,
        destination: DestinationDesc,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserLink, BrowseError>;
    async fn identify_link(
        &self,
        link_id: &str,
        identity: Arc<PrivateIdentity>,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError>;
    async fn request(&self, request: NativeRequest) -> Result<NativeRequestOutcome, BrowseError>;
    async fn close_link(&self, link_id: &str) -> Result<(), BrowseError>;
}

struct TransportBrowseBackend {
    transport: Arc<dyn MeshTransport>,
}

#[async_trait]
impl BrowseBackend for TransportBrowseBackend {
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
            .map_err(BrowseError::from)?;
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
        identity: Arc<PrivateIdentity>,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError> {
        tokio::select! {
            () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
            result = tokio::time::timeout_at(
                deadline,
                self.transport.identify_native_nomadnet_link(link_id, &identity),
            ) => result.map_err(|_| BrowseError::Deadline)??,
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
                result.map_err(|_| BrowseError::Deadline)??
            }
        };
        cancellation.set_request_id(started.request_id.clone());
        while tokio::time::Instant::now() < cancellation_deadline {
            let receipt = tokio::select! {
                () = cancellation_token.cancelled() => break,
                receipt = tokio::time::timeout_at(
                    cancellation_deadline,
                    self.transport.request_receipt(&started.request_id),
                ) => match receipt {
                    Ok(receipt) => receipt?.ok_or(BrowseError::MissingReceipt)?,
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
                let completed = completed?;
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
        self.transport.close_link(&AddressHash::new(bytes)).await?;
        Ok(())
    }
}

#[derive(Clone)]
struct BrowseSession {
    owner: u64,
    history: Vec<String>,
    position: usize,
    current: Option<PageContent>,
    link: Option<BrowserLink>,
    active: bool,
    terminal: bool,
    last_used: u64,
}

impl BrowseSession {
    fn new(owner: u64, last_used: u64) -> Self {
        Self {
            owner,
            history: Vec::new(),
            position: 0,
            current: None,
            link: None,
            active: false,
            terminal: false,
            last_used,
        }
    }
}

struct SessionReservation<'a> {
    sessions: &'a std::sync::Mutex<HashMap<String, BrowseSession>>,
    session_id: String,
    remove_on_drop: bool,
    rollback: Option<(String, BrowseSession)>,
    armed: bool,
}

impl SessionReservation<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }

    fn commit_eviction(&mut self) {
        self.rollback = None;
    }
}

impl Drop for SessionReservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
        if let Some((evicted_id, evicted)) = self.rollback.take() {
            sessions.remove(&self.session_id);
            sessions.insert(evicted_id, evicted);
            return;
        }
        if self.remove_on_drop {
            sessions.remove(&self.session_id);
        } else if let Some(session) = sessions.get_mut(&self.session_id) {
            session.active = false;
            session.terminal = true;
        }
    }
}

struct CachedPage {
    page: PageContent,
    size: usize,
}

struct DownloadRecord {
    owner: u64,
    info: FileDownloadInfo,
    bytes: Option<Vec<u8>>,
    saving: bool,
    cancellation: tokio_util::sync::CancellationToken,
    completion: tokio::sync::watch::Sender<u64>,
    last_used: u64,
}

/// Owns the complete native browse lifecycle. Frontends consume its projection only.
pub struct NativeNomadNetBrowseCoordinator {
    backend: Arc<dyn BrowseBackend>,
    discovery: Arc<DiscoveryService>,
    local_identity: RwLock<Option<Arc<PrivateIdentity>>>,
    sessions: std::sync::Mutex<HashMap<String, BrowseSession>>,
    cache: std::sync::Mutex<(HashMap<String, CachedPage>, VecDeque<String>, usize)>,
    downloads: Arc<tokio::sync::Mutex<HashMap<String, DownloadRecord>>>,
    cleanup: Arc<LinkCleanupSupervisor>,
    owner_cleanup: Arc<std::sync::Mutex<HashMap<u64, HashMap<String, BrowserLink>>>>,
    owner_cleanup_serial: tokio::sync::Mutex<()>,
    browser_links: tokio::sync::Mutex<HashMap<String, usize>>,
    #[cfg(test)]
    save_gate: tokio::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    #[cfg(test)]
    cancel_wait_gate: tokio::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    access_sequence: AtomicU64,
}

impl NativeNomadNetBrowseCoordinator {
    pub fn new(transport: Arc<dyn MeshTransport>, discovery: Arc<DiscoveryService>) -> Self {
        let backend: Arc<dyn BrowseBackend> = Arc::new(TransportBrowseBackend { transport });
        Self {
            cleanup: Arc::new(LinkCleanupSupervisor::new(Arc::clone(&backend))),
            backend,
            discovery,
            local_identity: RwLock::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            cache: std::sync::Mutex::new((HashMap::new(), VecDeque::new(), 0)),
            downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            browser_links: tokio::sync::Mutex::new(HashMap::new()),
            owner_cleanup: Arc::new(std::sync::Mutex::new(HashMap::new())),
            owner_cleanup_serial: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            save_gate: tokio::sync::Mutex::new(None),
            #[cfg(test)]
            cancel_wait_gate: tokio::sync::Mutex::new(None),
            access_sequence: AtomicU64::new(1),
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn BrowseBackend>, discovery: Arc<DiscoveryService>) -> Self {
        Self {
            cleanup: Arc::new(LinkCleanupSupervisor::new(Arc::clone(&backend))),
            backend,
            discovery,
            local_identity: RwLock::new(None),
            sessions: std::sync::Mutex::new(HashMap::new()),
            cache: std::sync::Mutex::new((HashMap::new(), VecDeque::new(), 0)),
            downloads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            browser_links: tokio::sync::Mutex::new(HashMap::new()),
            owner_cleanup: Arc::new(std::sync::Mutex::new(HashMap::new())),
            owner_cleanup_serial: tokio::sync::Mutex::new(()),
            save_gate: tokio::sync::Mutex::new(None),
            cancel_wait_gate: tokio::sync::Mutex::new(None),
            access_sequence: AtomicU64::new(1),
        }
    }

    pub fn set_identity(&self, identity: Arc<PrivateIdentity>) {
        *self.local_identity.write().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(identity);
    }

    #[cfg(test)]
    async fn browse_remote(&self, host: &str, path: &str, timeout: Duration) -> PageContent {
        let fetched = self
            .browse_remote_with_data(
                host,
                path,
                vec![0xc0],
                timeout,
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await;
        if let Some(link) = fetched.link.clone() {
            let _ = self.cleanup_or_retain(link).await;
        }
        fetched.page
    }

    async fn browse_remote_with_data(
        &self,
        host: &str,
        path: &str,
        data: Vec<u8>,
        timeout: Duration,
        cancellation: tokio_util::sync::CancellationToken,
        owner: u64,
    ) -> FetchedPage {
        let correlation = correlation_id();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut result = initial_result(host, path, &correlation, PageCacheStatus::NotUsed);
        let Some(device) = self.discovery.device(host) else {
            fail(&mut result, 0, "capability_unknown", "host has no native NomadNet announce");
            return FetchedPage { page: result, link: None };
        };
        if !device.discovered_capabilities.contains(&DiscoveredCapability::NativeNomadNetHost) {
            fail(&mut result, 0, "capability_missing", "host did not advertise native NomadNet");
            return FetchedPage { page: result, link: None };
        }
        let destination = match decode_destination(host) {
            Ok(destination) => destination,
            Err(message) => {
                fail(&mut result, 0, "invalid_destination", &message);
                return FetchedPage { page: result, link: None };
            }
        };
        if let Err(error) = self.backend.discover_path(destination, &cancellation, deadline).await {
            fail(&mut result, 0, "path_discovery_failed", &error.to_string());
            return FetchedPage { page: result, link: None };
        }
        result.stages[0].evidence_source = Some(ObservationSource::TransportPathTable);
        result.stages[0].destination_hash = Some(host.to_string());
        succeed(&mut result, 0);

        let identity =
            match self.backend.resolve_identity(destination, &cancellation, deadline).await {
                Ok(identity) => identity,
                Err(error) => {
                    fail(&mut result, 1, "identity_resolution_failed", &error.to_string());
                    return FetchedPage { page: result, link: None };
                }
            };
        result.stages[1].destination_hash = Some(host.to_string());
        succeed(&mut result, 1);
        let descriptor = DestinationDesc {
            identity,
            address_hash: destination,
            name: DestinationName::new("nomadnetwork", "node"),
        };
        let link = match self.backend.open_link(descriptor, &cancellation, deadline).await {
            Ok(link) => link,
            Err(error) => {
                fail(&mut result, 2, "link_establishment_failed", &error.to_string());
                return FetchedPage { page: result, link: None };
            }
        };
        let link = UnretainedLink::new(
            link,
            Arc::clone(&self.cleanup),
            Arc::clone(&self.owner_cleanup),
            owner,
        );
        result.request.link_id = Some(link.link().id.clone());
        result.stages[2].evidence_source = Some(ObservationSource::TransportLinkState);
        result.stages[2].link_id = Some(link.link().id.clone());
        succeed(&mut result, 2);
        let local_identity =
            self.local_identity.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        if let Some(identity) = local_identity {
            if let Err(error) =
                self.backend.identify_link(&link.link().id, identity, &cancellation, deadline).await
            {
                fail(&mut result, 3, "identification_failed", &error.to_string());
                return FetchedPage { page: result, link: None };
            }
            result.stages[3].evidence_source = Some(ObservationSource::TransportLinkState);
            result.stages[3].link_id = Some(link.link().id.clone());
            succeed(&mut result, 3);
        } else {
            skip(&mut result, 3, "no local RNS identity selected");
        }

        let request = match self
            .backend
            .request(NativeRequest {
                link_id: link.link().id.clone(),
                path: path.to_string(),
                correlation_id: correlation.clone(),
                data,
                max_response_size: MAX_ENCODED_NATIVE_RESPONSE_SIZE,
                cancellation,
                progress: None,
                deadline,
            })
            .await
        {
            Ok(request) => request,
            Err(error) => {
                fail(&mut result, 4, "request_submission_failed", &error.to_string());
                return FetchedPage { page: result, link: None };
            }
        };
        observe_request_stage(&mut result.stages[4], &request.started);
        succeed(&mut result, 4);
        apply_request_metadata(&mut result, &request.started, &request.completed);
        observe_request_stage(&mut result.stages[5], &request.completed);
        if request.completed.state != RequestState::Succeeded {
            let code = match request.completed.state {
                RequestState::TimedOut => "request_timed_out",
                RequestState::Cancelled => "request_cancelled",
                _ => "transfer_failed",
            };
            fail(
                &mut result,
                5,
                code,
                &format!("native request ended in {:?}", request.completed.state),
            );
            return FetchedPage { page: result, link: None };
        }
        let response = match request.completed.response.as_deref().and_then(decode_binary_response)
        {
            Some(response) => response,
            None => {
                fail(
                    &mut result,
                    5,
                    "invalid_response",
                    "native response was not one binary value",
                );
                return FetchedPage { page: result, link: None };
            }
        };
        if response.len() > MAX_PAGE_SOURCE_SIZE {
            fail(&mut result, 5, "response_too_large", "page exceeds IPC-safe source budget");
            return FetchedPage { page: result, link: None };
        }
        result.transfer = transfer_info(&request.completed);
        succeed(&mut result, 5);
        if remaining(deadline).is_err() {
            fail(&mut result, 6, "deadline_elapsed", "browse deadline elapsed before parsing");
            return FetchedPage { page: result, link: None };
        }
        finish_projection(&mut result, response);
        if remaining(deadline).is_err() {
            fail(&mut result, 7, "deadline_elapsed", "browse deadline elapsed while rendering");
            return FetchedPage { page: result, link: None };
        }
        let link = self.retain_browser_link(link.take()).await;
        FetchedPage { page: result, link: Some(link) }
    }

    async fn retain_browser_link(&self, mut link: BrowserLink) -> BrowserLink {
        let mut links = self.browser_links.lock().await;
        if link.created {
            *links.entry(link.id.clone()).or_default() += 1;
        } else if let Some(references) = links.get_mut(&link.id) {
            *references += 1;
            link.created = true;
        }
        link
    }

    async fn cleanup_or_retain(&self, link: BrowserLink) -> Result<(), BrowseError> {
        if !link.created {
            return Ok(());
        }
        {
            let mut links = self.browser_links.lock().await;
            if let Some(references) = links.get_mut(&link.id) {
                *references -= 1;
                if *references > 0 {
                    return Ok(());
                }
                links.remove(&link.id);
            }
        }
        self.cleanup.cleanup(link).await
    }

    async fn release_session_link(&self, link: &BrowserLink) -> Result<(), BrowseError> {
        if !link.created {
            return Ok(());
        }
        {
            let mut links = self.browser_links.lock().await;
            if let Some(references) = links.get_mut(&link.id) {
                *references -= 1;
                if *references > 0 {
                    return Ok(());
                }
                links.remove(&link.id);
            }
        }
        if let Err(error) = self.cleanup.cleanup(link.clone()).await {
            *self.browser_links.lock().await.entry(link.id.clone()).or_default() += 1;
            return Err(error);
        }
        Ok(())
    }

    async fn cleanup_owned_link(&self, owner: u64, link: BrowserLink) -> Result<(), BrowseError> {
        if !link.created {
            return Ok(());
        }
        self.owner_cleanup
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .entry(owner)
            .or_default()
            .insert(link.id.clone(), link.clone());
        let result = self.cleanup.cleanup(link.clone()).await;
        if result.is_ok() {
            remove_owned_cleanup(&self.owner_cleanup, owner, &link.id);
        }
        result
    }

    pub async fn navigate(
        &self,
        request: PageNavigationRequest,
        local_host: &str,
        local_source: impl FnOnce(&str) -> Vec<u8>,
    ) -> Result<PageContent, String> {
        self.navigate_for_owner(0, request, local_host, local_source).await
    }

    pub async fn navigate_for_owner(
        &self,
        owner: u64,
        request: PageNavigationRequest,
        local_host: &str,
        local_source: impl FnOnce(&str) -> Vec<u8>,
    ) -> Result<PageContent, String> {
        let session_id = request.session_id.clone().unwrap_or_else(correlation_id);
        let access = self.access_sequence.fetch_add(1, Ordering::Relaxed);
        let (address, desired_position, data, existed, rollback) = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            let existing = sessions.get(&session_id);
            if existing.is_some_and(|session| session.owner != owner) {
                return Err("page session is owned by another IPC connection".into());
            }
            if existing.is_some_and(|session| session.active) {
                return Err("page session already has active work".into());
            }
            let current = existing.and_then(|session| session.current.as_ref());
            let (target, desired_position) = match request.action {
                PageNavigationAction::Navigate => {
                    let raw = request.target.as_deref().ok_or("navigation target is required")?;
                    let address = if let Some(current) = current {
                        let base = PageAddress::parse(&current.navigation.address)
                            .map_err(|error| error.to_string())?;
                        PageAddress::resolve(raw, &base).map_err(|error| error.to_string())?
                    } else {
                        PageAddress::parse(raw).map_err(|error| error.to_string())?
                    };
                    (address.to_string(), None)
                }
                PageNavigationAction::Back => {
                    let session = existing.ok_or("page session was not found")?;
                    let position = session
                        .position
                        .checked_sub(1)
                        .ok_or("page history has no previous entry")?;
                    (session.history[position].clone(), Some(position))
                }
                PageNavigationAction::Forward => {
                    let session = existing.ok_or("page session was not found")?;
                    let position = session.position + 1;
                    if position >= session.history.len() {
                        return Err("page history has no next entry".into());
                    }
                    (session.history[position].clone(), Some(position))
                }
                PageNavigationAction::Reload => {
                    let current = current.ok_or("page session has no active page")?;
                    (current.navigation.address.clone(), existing.map(|session| session.position))
                }
                _ => return Err("unsupported page navigation action".into()),
            };
            let form_fields = current.map(|page| page.fields.clone()).unwrap_or_default();
            let link_fields = request
                .target
                .as_deref()
                .and_then(|raw| current?.link_targets.iter().find(|link| link.target == raw))
                .map(|link| link.submitted_fields.clone())
                .unwrap_or_default();
            let address = PageAddress::parse(&target).map_err(|error| error.to_string())?;
            let data = encode_submission(request.submission.as_ref(), &form_fields, &link_fields)?;
            let existed = existing.is_some();
            let rollback = if !existed && sessions.len() >= MAX_SESSIONS {
                let candidate = sessions
                    .iter()
                    .filter(|(_, session)| !session.active)
                    .min_by_key(|(_, session)| (!session.terminal, session.last_used))
                    .map(|(id, _)| id.clone())
                    .ok_or("page session capacity is full")?;
                sessions.remove(&candidate).map(|session| (candidate, session))
            } else {
                None
            };
            let session = sessions
                .entry(session_id.clone())
                .or_insert_with(|| BrowseSession::new(owner, access));
            session.active = true;
            session.last_used = access;
            (address, desired_position, data, existed, rollback)
        };
        let evicted_link = rollback.as_ref().and_then(|(_, session)| session.link.clone());
        let mut reservation = SessionReservation {
            sessions: &self.sessions,
            session_id: session_id.clone(),
            remove_on_drop: !existed,
            rollback,
            armed: true,
        };
        if let Some(link) = evicted_link {
            self.release_session_link(&link).await.map_err(|error| error.to_string())?;
        }
        reservation.commit_eviction();

        let bypass = request.bypass_cache || request.action == PageNavigationAction::Reload;
        let cache_key = address.to_string();
        let mut page =
            if !bypass && request.submission.is_none() { self.cached(&cache_key) } else { None }
                .unwrap_or_else(PageContent::default);
        let mut fetched_link = None;
        let cache_hit = !page.correlation_id.is_empty();
        if page.correlation_id.is_empty() {
            let (host, path) = address.parts();
            let fetched = if host.is_empty() {
                FetchedPage {
                    page: self.project_local(local_host, path, local_source(path)),
                    link: None,
                }
            } else {
                let cancellation = tokio_util::sync::CancellationToken::new();
                let cancellation_guard = cancellation.clone().drop_guard();
                let fetched = self
                    .browse_remote_with_data(
                        host,
                        path,
                        data,
                        Duration::from_secs(request.timeout_secs.unwrap_or(30).clamp(1, 120)),
                        cancellation,
                        owner,
                    )
                    .await;
                cancellation_guard.disarm();
                fetched
            };
            page = fetched.page;
            fetched_link = fetched.link;
            page.cache.status =
                if bypass { PageCacheStatus::Bypassed } else { PageCacheStatus::Miss };
            if page.outcome == PageBrowseOutcome::Succeeded && request.submission.is_none() {
                self.store_cache(cache_key.clone(), &page);
            }
        } else {
            let cache_correlation = correlation_id();
            let origin_correlation = page.correlation_id.clone();
            page.correlation_id = cache_correlation.clone();
            for stage in &mut page.stages {
                stage.correlation_id = cache_correlation.clone();
                stage.observation = page_observation(&cache_correlation, Some(epoch_now()));
                stage.evidence_source = Some(ObservationSource::OperationCoordinator);
                stage.destination_hash = None;
                stage.link_id = None;
                stage.request_id = None;
                stage.resource_hash = None;
                stage.state = PageBrowseStageState::Skipped { reason: "served from cache".into() };
            }
            page.cache.status = PageCacheStatus::Hit;
            page.cache.origin_correlation_id = Some(origin_correlation);
            page.request.request_id = None;
            page.request.link_id = None;
            page.request.request_size = 0;
            page.request.response_size = None;
            page.request.rtt_ms = None;
            let cached_bytes = page.source_bytes.len().try_into().unwrap_or(u64::MAX);
            page.transfer = PageTransferInfo::default();
            page.transfer.kind = PageTransferKind::Cache;
            page.transfer.received_bytes = cached_bytes;
            page.transfer.total_bytes = cached_bytes;
            page.transfer.progress = 1.0;
            page.transfer.verified = true;
            page.failure = None;
            page.outcome = PageBrowseOutcome::Succeeded;
            page.started_unix_ms = Some(epoch_millis());
            terminalize(&mut page, PageBrowseOutcome::Succeeded);
        }

        let failed = page_failed(&page);
        let new_link = fetched_link;
        let old_link = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            let session = sessions.get_mut(&session_id).ok_or("page session disappeared")?;
            if let Some(position) = desired_position {
                session.position = position;
            } else if request.action == PageNavigationAction::Navigate
                && session.history.get(session.position).map(String::as_str)
                    != Some(cache_key.as_str())
            {
                let truncate_at = if session.history.is_empty() { 0 } else { session.position + 1 };
                session.history.truncate(truncate_at);
                session.history.push(cache_key.clone());
                if session.history.len() > MAX_HISTORY_ENTRIES {
                    session.history.remove(0);
                }
                session.position = session.history.len().saturating_sub(1);
            }
            let old_link = if !cache_hit {
                let old = session.link.take();
                if failed {
                    old
                } else {
                    match (old, new_link.clone()) {
                        (Some(old), Some(new)) if old.id == new.id => {
                            session.link = Some(BrowserLink {
                                id: old.id,
                                created: old.created || new.created,
                            });
                            Some(new)
                        }
                        (old, new) => {
                            session.link = new;
                            old
                        }
                    }
                }
            } else {
                None
            };
            session.active = false;
            session.terminal = failed;
            session.last_used = self.access_sequence.fetch_add(1, Ordering::Relaxed);
            if failed {
                page.navigation.connection_open = false;
            }
            page.navigation = navigation_info(&session_id, &cache_key, session, true);
            session.current = Some(page.clone());
            old_link
        };
        reservation.disarm();
        if let Some(link) = old_link
            && let Err(error) = self.cleanup_or_retain(link.clone()).await
        {
            self.owner_cleanup
                .lock()
                .unwrap_or_else(|value| value.into_inner())
                .entry(owner)
                .or_default()
                .insert(link.id.clone(), link);
            log::error!("replaced browser link cleanup reached terminal error: {error}");
        }
        Ok(page)
    }

    pub async fn close_session(&self, session_id: &str) -> Result<PageNavigationInfo, String> {
        self.close_session_for_owner(0, session_id).await
    }

    pub async fn close_session_for_owner(
        &self,
        owner: u64,
        session_id: &str,
    ) -> Result<PageNavigationInfo, String> {
        let (link, address, mut info) = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            let session = sessions.get_mut(session_id).ok_or("page session was not found")?;
            if session.owner != owner {
                return Err("page session is owned by another IPC connection".into());
            }
            if session.active {
                return Err("page session still has active work".into());
            }
            let address = session
                .current
                .as_ref()
                .map(|page| page.navigation.address.clone())
                .unwrap_or_default();
            let mut info = navigation_info(session_id, &address, session, false);
            info.connection_open = false;
            session.active = true;
            (session.link.clone(), address, info)
        };
        let mut reservation = SessionReservation {
            sessions: &self.sessions,
            session_id: session_id.to_string(),
            remove_on_drop: false,
            rollback: None,
            armed: true,
        };
        if let Some(link) = link
            && let Err(error) = self.release_session_link(&link).await
        {
            let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            if let Some(session) = sessions.get_mut(session_id) {
                session.active = false;
            }
            reservation.disarm();
            return Err(error.to_string());
        }
        self.sessions
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .remove(session_id)
            .ok_or("page session was not found")?;
        reservation.disarm();
        info.address = address;
        Ok(info)
    }

    fn cached(&self, key: &str) -> Option<PageContent> {
        let cache = self.cache.lock().unwrap_or_else(|value| value.into_inner());
        cache.0.get(key).map(|entry| entry.page.clone())
    }

    fn store_cache(&self, key: String, page: &PageContent) {
        let mut cache = self.cache.lock().unwrap_or_else(|value| value.into_inner());
        let size = page.source_bytes.len();
        if size > MAX_CACHE_BYTES {
            return;
        }
        if let Some(previous) = cache.0.remove(&key) {
            cache.2 = cache.2.saturating_sub(previous.size);
            cache.1.retain(|existing| existing != &key);
        }
        while cache.0.len() >= MAX_CACHE_ENTRIES || cache.2.saturating_add(size) > MAX_CACHE_BYTES {
            let Some(oldest) = cache.1.pop_front() else { break };
            if let Some(removed) = cache.0.remove(&oldest) {
                cache.2 = cache.2.saturating_sub(removed.size);
            }
        }
        let mut stored = page.clone();
        stored.cache.stored_at = Some(epoch_seconds());
        cache.2 += size;
        cache.1.push_back(key.clone());
        cache.0.insert(key, CachedPage { page: stored, size });
    }

    pub async fn start_download(
        self: &Arc<Self>,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, String> {
        self.start_download_for_owner(0, request).await
    }

    pub async fn start_download_for_owner(
        self: &Arc<Self>,
        owner: u64,
        request: FileDownloadRequest,
    ) -> Result<FileDownloadInfo, String> {
        let current = if let Some(id) = request.session_id.as_deref() {
            let sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            let session = sessions.get(id).ok_or("page session was not found")?;
            if session.owner != owner {
                return Err("page session is owned by another IPC connection".into());
            }
            session
                .current
                .as_ref()
                .and_then(|page| PageAddress::parse(&page.navigation.address).ok())
        } else {
            None
        };
        let (host, path) = resolve_file_target(&request.target, current.as_ref())?;
        let download_id = correlation_id().replacen("page-", "download-", 1);
        let correlation = correlation_id();
        let mut info = FileDownloadInfo::default();
        info.download_id = download_id.clone();
        info.correlation_id = correlation;
        info.host_hash = host;
        info.native_path = path;
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (completion, _) = tokio::sync::watch::channel(0_u64);
        let access = self.access_sequence.fetch_add(1, Ordering::Relaxed);
        {
            let mut downloads = self.downloads.lock().await;
            if downloads.len() >= MAX_DOWNLOADS {
                let removable = downloads
                    .iter()
                    .filter(|(_, record)| record.info.state.is_terminal() && !record.saving)
                    .min_by_key(|(_, record)| record.last_used)
                    .map(|(id, _)| id.clone())
                    .ok_or("download capacity is full")?;
                downloads.remove(&removable);
            }
            downloads.insert(
                download_id.clone(),
                DownloadRecord {
                    owner,
                    info: info.clone(),
                    bytes: None,
                    saving: false,
                    cancellation: cancellation.clone(),
                    completion,
                    last_used: access,
                },
            );
        }
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator
                .run_download(
                    download_id,
                    request.expected_sha256,
                    request.timeout_secs,
                    cancellation,
                )
                .await;
        });
        Ok(info)
    }

    async fn run_download(
        &self,
        download_id: String,
        expected_sha256: Option<String>,
        timeout_secs: Option<u64>,
        cancellation: tokio_util::sync::CancellationToken,
    ) {
        let (owner, host, path, correlation) = {
            let downloads = self.downloads.lock().await;
            let Some(record) = downloads.get(&download_id) else { return };
            (
                record.owner,
                record.info.host_hash.clone(),
                record.info.native_path.clone(),
                record.info.correlation_id.clone(),
            )
        };
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(timeout_secs.unwrap_or(120).clamp(1, 600));
        let outcome =
            self.download_native(owner, &host, &path, &correlation, deadline, cancellation).await;
        let mut downloads = self.downloads.lock().await;
        let Some(record) = downloads.get_mut(&download_id) else { return };
        if record.info.state.is_terminal() {
            record.completion.send_modify(|version| *version = version.wrapping_add(1));
            return;
        }
        match outcome {
            Ok((receipt, bytes)) => {
                let checksum = hex::encode(Sha256::digest(&bytes));
                let integrity_verified = expected_sha256
                    .as_deref()
                    .is_none_or(|expected| expected.eq_ignore_ascii_case(&checksum));
                record.info.received_bytes = receipt.received_bytes;
                record.info.total_bytes = receipt.total_bytes;
                record.info.progress = receipt.progress;
                record.info.transfer = transfer_info(&receipt).kind;
                record.info.resource_hash = receipt.resource_hash;
                record.info.sha256 = Some(checksum);
                record.info.integrity_verified = integrity_verified;
                if integrity_verified {
                    record.info.state = FileDownloadState::Completed;
                    record.bytes = Some(bytes);
                } else {
                    record.info.state = FileDownloadState::Failed;
                    record.info.error =
                        Some("download SHA-256 did not match the expected checksum".into());
                }
            }
            Err(error) => {
                record.info.state = if record.cancellation.is_cancelled() {
                    FileDownloadState::Cancelled
                } else {
                    FileDownloadState::Failed
                };
                record.info.error = Some(error.to_string());
            }
        }
        record.last_used = self.access_sequence.fetch_add(1, Ordering::Relaxed);
        record.completion.send_modify(|version| *version = version.wrapping_add(1));
    }

    async fn download_native(
        &self,
        owner: u64,
        host: &str,
        path: &str,
        correlation: &str,
        deadline: tokio::time::Instant,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<(RequestObservationInfo, Vec<u8>), BrowseError> {
        let Some(device) = self.discovery.device(host) else {
            return Err(BrowseError::Transport("host has no native NomadNet announce".into()));
        };
        if !device.discovered_capabilities.contains(&DiscoveredCapability::NativeNomadNetHost) {
            return Err(BrowseError::Transport("host did not advertise native NomadNet".into()));
        }
        let destination = decode_destination(host).map_err(BrowseError::Transport)?;
        self.backend.discover_path(destination, &cancellation, deadline).await?;
        let identity = self.backend.resolve_identity(destination, &cancellation, deadline).await?;
        let descriptor = DestinationDesc {
            identity,
            address_hash: destination,
            name: DestinationName::new("nomadnetwork", "node"),
        };
        let link = self.backend.open_link(descriptor, &cancellation, deadline).await?;
        let link = UnretainedLink::new(
            link,
            Arc::clone(&self.cleanup),
            Arc::clone(&self.owner_cleanup),
            owner,
        );
        let outcome = async {
            let local_identity =
                { self.local_identity.read().unwrap_or_else(|value| value.into_inner()).clone() };
            if let Some(identity) = local_identity {
                self.backend
                    .identify_link(&link.link().id, identity, &cancellation, deadline)
                    .await?;
            }
            let downloads = Arc::clone(&self.downloads);
            let progress_correlation = correlation.to_string();
            let progress: Arc<dyn Fn(RequestObservationInfo) + Send + Sync> =
                Arc::new(move |receipt| {
                    let downloads = Arc::clone(&downloads);
                    let progress_correlation = progress_correlation.clone();
                    tokio::spawn(async move {
                        let mut records = downloads.lock().await;
                        if let Some(record) = records
                            .values_mut()
                            .find(|record| record.info.correlation_id == progress_correlation)
                        {
                            if !matches!(
                                record.info.state,
                                FileDownloadState::Pending | FileDownloadState::Receiving
                            ) {
                                return;
                            }
                            record.info.state = FileDownloadState::Receiving;
                            record.info.received_bytes = receipt.received_bytes;
                            record.info.total_bytes = receipt.total_bytes;
                            record.info.progress = receipt.progress;
                            record.info.transfer = transfer_info(&receipt).kind;
                            record.info.resource_hash = receipt.resource_hash;
                        }
                    });
                });
            let request = self
                .backend
                .request(NativeRequest {
                    link_id: link.link().id.clone(),
                    path: path.to_string(),
                    correlation_id: correlation.to_string(),
                    data: vec![0xc0],
                    max_response_size: MAX_ENCODED_FILE_RESPONSE_SIZE,
                    cancellation,
                    progress: Some(progress),
                    deadline,
                })
                .await?;
            if request.completed.state != RequestState::Succeeded {
                return Err(BrowseError::Transport(format!(
                    "native file request ended in {:?}",
                    request.completed.state
                )));
            }
            let bytes =
                request.completed.response.as_deref().and_then(decode_binary_response).ok_or_else(
                    || BrowseError::Transport("native file response was malformed".into()),
                )?;
            if bytes.len() > MAX_FILE_SIZE {
                return Err(BrowseError::Transport("file exceeds bounded download storage".into()));
            }
            Ok((request.completed, bytes))
        }
        .await;
        let cleanup = self.cleanup_owned_link(owner, link.take()).await;
        match (outcome, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub async fn download(&self, download_id: &str) -> Option<FileDownloadInfo> {
        self.download_for_owner(0, download_id).await
    }

    pub async fn download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Option<FileDownloadInfo> {
        let mut downloads = self.downloads.lock().await;
        let access = self.access_sequence.fetch_add(1, Ordering::Relaxed);
        downloads.get_mut(download_id).filter(|record| record.owner == owner).map(|record| {
            record.last_used = access;
            record.info.clone()
        })
    }

    pub async fn cancel_download(&self, download_id: &str) -> Option<FileDownloadInfo> {
        self.cancel_download_for_owner(0, download_id).await
    }

    pub async fn cancel_download_for_owner(
        &self,
        owner: u64,
        download_id: &str,
    ) -> Option<FileDownloadInfo> {
        let (cancellation, mut completion) = {
            let downloads = self.downloads.lock().await;
            let record = downloads.get(download_id)?;
            if record.owner != owner {
                return None;
            }
            if record.saving {
                return Some(record.info.clone());
            }
            (record.cancellation.clone(), record.completion.subscribe())
        };
        cancellation.cancel();
        loop {
            let current = self.download_for_owner(owner, download_id).await?;
            if current.state.is_terminal() {
                return Some(current);
            }
            #[cfg(test)]
            if let Some(gate) = self.cancel_wait_gate.lock().await.take()
                && let Ok(permit) = gate.acquire().await
            {
                permit.forget();
            }
            match tokio::time::timeout(DOWNLOAD_CANCELLATION_WAIT, completion.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return self.download_for_owner(owner, download_id).await,
                Err(_) => {
                    let mut downloads = self.downloads.lock().await;
                    let record = downloads.get_mut(download_id)?;
                    if record.owner != owner {
                        return None;
                    }
                    if !record.info.state.is_terminal() {
                        record.info.state = FileDownloadState::Cancelled;
                        record.info.error =
                            Some("download cancellation completion timed out".into());
                        record.completion.send_modify(|version| *version = version.wrapping_add(1));
                    }
                    return Some(record.info.clone());
                }
            }
        }
    }

    pub async fn save_download(
        self: &Arc<Self>,
        download_id: &str,
        destination: &Path,
    ) -> Result<FileDownloadInfo, String> {
        self.save_download_for_owner(0, download_id, destination).await
    }

    pub async fn save_download_for_owner(
        self: &Arc<Self>,
        owner: u64,
        download_id: &str,
        destination: &Path,
    ) -> Result<FileDownloadInfo, String> {
        if !destination.is_absolute() {
            return Err("save destination must be an explicit absolute path".into());
        }
        if destination.components().any(|component| {
            matches!(component, std::path::Component::CurDir | std::path::Component::ParentDir)
        }) {
            return Err("save destination must not contain relative path components".into());
        }
        let parent = destination.parent().ok_or("save destination has no parent")?.to_path_buf();
        let file_name =
            destination.file_name().ok_or("save destination has no file name")?.to_os_string();
        let destination = destination.to_path_buf();
        let bytes = {
            let mut downloads = self.downloads.lock().await;
            let record = downloads.get_mut(download_id).ok_or("download was not found")?;
            if record.owner != owner {
                return Err("download is owned by another IPC connection".into());
            }
            if record.info.state != FileDownloadState::Completed
                || !record.info.integrity_verified
                || record.saving
            {
                return Err("only a completed verified download can be saved".into());
            }
            let bytes = record.bytes.clone().ok_or("verified download bytes are unavailable")?;
            record.saving = true;
            bytes
        };
        let coordinator = Arc::clone(self);
        let download_id = download_id.to_string();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            #[cfg(test)]
            if let Some(gate) = coordinator.save_gate.lock().await.clone()
                && let Ok(permit) = gate.acquire().await
            {
                permit.forget();
            }
            let temporary = parent.join(format!(
                ".{}.{}.styrene-download",
                file_name.to_string_lossy(),
                correlation_id()
            ));
            let handoff = async {
                if !tokio::fs::metadata(&parent).await.map_err(|error| error.to_string())?.is_dir()
                {
                    return Err("save destination parent is not a directory".into());
                }
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .await
                    .map_err(|error| format!("save temporary file was not created: {error}"))?;
                file.write_all(&bytes).await.map_err(|error| error.to_string())?;
                file.flush().await.map_err(|error| error.to_string())?;
                file.sync_all().await.map_err(|error| error.to_string())?;
                drop(file);
                tokio::fs::hard_link(&temporary, &destination).await.map_err(|error| {
                    format!("save destination was not atomically created: {error}")
                })?;
                let _ = tokio::fs::remove_file(&temporary).await;
                let sync_parent = parent.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    std::fs::File::open(sync_parent).and_then(|directory| directory.sync_all())
                })
                .await;
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = handoff {
                let _ = tokio::fs::remove_file(&temporary).await;
                let mut downloads = coordinator.downloads.lock().await;
                if let Some(record) = downloads.get_mut(&download_id) {
                    record.info.state = FileDownloadState::Completed;
                    record.info.error = Some(error.clone());
                    record.saving = false;
                    record.completion.send_modify(|version| *version = version.wrapping_add(1));
                }
                let _ = result_tx.send(Err(error));
                return;
            }
            let result = {
                let mut downloads = coordinator.downloads.lock().await;
                let record = downloads
                    .get_mut(&download_id)
                    .ok_or_else(|| "download save reservation was lost".to_string());
                match record {
                    Ok(record) if record.owner == owner && record.saving => {
                        record.saving = false;
                        record.info.state = FileDownloadState::Saved;
                        record.info.saved_path = Some(destination.to_string_lossy().into_owned());
                        record.bytes = None;
                        record.completion.send_modify(|version| *version = version.wrapping_add(1));
                        Ok(record.info.clone())
                    }
                    _ => Err("download save reservation was lost".into()),
                }
            };
            let _ = result_tx.send(result);
        });
        result_rx.await.map_err(|_| "download save task stopped".to_string())?
    }

    pub async fn cleanup_owner(&self, owner: u64) -> Result<(), String> {
        let _serial = self.owner_cleanup_serial.lock().await;
        let retained = self
            .owner_cleanup
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .get(&owner)
            .map(|links| links.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut cleanup_errors = Vec::new();
        for link in retained {
            match self.cleanup.cleanup(link.clone()).await {
                Ok(()) => {
                    let mut owners =
                        self.owner_cleanup.lock().unwrap_or_else(|value| value.into_inner());
                    let empty = owners.get_mut(&owner).is_some_and(|links| {
                        links.remove(&link.id);
                        links.is_empty()
                    });
                    if empty {
                        owners.remove(&owner);
                    }
                }
                Err(error) => cleanup_errors.push(error.to_string()),
            }
        }

        let session_ids = {
            let sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            sessions
                .iter()
                .filter(|(_, session)| session.owner == owner)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        for session_id in session_ids {
            let link = self
                .sessions
                .lock()
                .unwrap_or_else(|value| value.into_inner())
                .get(&session_id)
                .and_then(|session| (session.owner == owner).then(|| session.link.clone()))
                .flatten();
            let cleanup = match link.as_ref() {
                Some(link) => self.cleanup_or_retain(link.clone()).await,
                None => Ok(()),
            };
            let mut sessions = self.sessions.lock().unwrap_or_else(|value| value.into_inner());
            if sessions.get(&session_id).is_none_or(|session| session.owner != owner) {
                continue;
            }
            if let (Some(link), Err(error)) = (link, cleanup) {
                self.owner_cleanup
                    .lock()
                    .unwrap_or_else(|value| value.into_inner())
                    .entry(owner)
                    .or_default()
                    .insert(link.id.clone(), link);
                cleanup_errors.push(error.to_string());
            }
            sessions.remove(&session_id);
        }

        let mut downloads = {
            let downloads = self.downloads.lock().await;
            downloads
                .iter()
                .filter(|(_, record)| record.owner == owner)
                .map(|(id, record)| {
                    (
                        id.clone(),
                        record.info.state.is_terminal(),
                        record.saving,
                        record.completion.subscribe(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (id, terminal, saving, completion) in &mut downloads {
            if *saving {
                let _ = completion.changed().await;
            } else if !*terminal {
                let _ = self.cancel_download_for_owner(owner, id).await;
            }
        }
        let mut records = self.downloads.lock().await;
        for (id, _, _, _) in downloads {
            records.remove(&id);
        }
        drop(records);
        if cleanup_errors.is_empty() { Ok(()) } else { Err(cleanup_errors.join("; ")) }
    }

    pub fn project_local(&self, host: &str, path: &str, source: Vec<u8>) -> PageContent {
        let correlation = correlation_id();
        let mut result = initial_result(host, path, &correlation, PageCacheStatus::NotUsed);
        for index in 0..5 {
            skip(&mut result, index, "local page does not use a network stage");
        }
        result.transfer.kind = PageTransferKind::Local;
        result.transfer.received_bytes = source.len().try_into().unwrap_or(u64::MAX);
        result.transfer.total_bytes = source.len().try_into().unwrap_or(u64::MAX);
        result.transfer.progress = 1.0;
        result.transfer.verified = true;
        succeed(&mut result, 5);
        finish_projection(&mut result, source);
        result
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

fn initial_result(
    host: &str,
    path: &str,
    correlation: &str,
    cache: PageCacheStatus,
) -> PageContent {
    let kinds = [
        PageBrowseStageKind::PathDiscovery,
        PageBrowseStageKind::IdentityResolution,
        PageBrowseStageKind::LinkEstablishment,
        PageBrowseStageKind::Identification,
        PageBrowseStageKind::RequestSubmission,
        PageBrowseStageKind::Transfer,
        PageBrowseStageKind::Parse,
        PageBrowseStageKind::Render,
    ];
    let mut result = PageContent::default();
    result.host_hash = host.to_string();
    result.correlation_id = correlation.to_string();
    result.outcome = PageBrowseOutcome::Running;
    result.started_unix_ms = Some(epoch_millis());
    result.observation = page_observation(correlation, Some(epoch_now()));
    result.stages = kinds
        .into_iter()
        .map(|kind| {
            let mut stage = PageBrowseStage::default();
            stage.correlation_id = correlation.to_string();
            stage.kind = kind;
            stage.observation = page_observation(correlation, None);
            stage
        })
        .collect();
    result.request.native_path = path.to_string();
    result.request.path_hash = hex::encode(rns_core::destination::request_path_hash(path));
    result.cache.status = cache;
    result
}

fn succeed(result: &mut PageContent, index: usize) {
    result.stages[index].state = PageBrowseStageState::Succeeded;
    result.stages[index].observation.observed_at = Some(epoch_now());
    if result.stages[index].kind == PageBrowseStageKind::Render {
        terminalize(result, PageBrowseOutcome::Succeeded);
    }
}

fn fail(result: &mut PageContent, index: usize, code: &str, message: &str) {
    let outcome = if matches!(code, "deadline_elapsed" | "request_timed_out")
        || message == "browse operation deadline elapsed"
    {
        PageBrowseOutcome::TimedOut
    } else if code == "request_cancelled" || message == "browse operation cancelled" {
        PageBrowseOutcome::Cancelled
    } else {
        PageBrowseOutcome::Failed
    };
    result.stages[index].state =
        PageBrowseStageState::Failed { code: code.to_string(), message: message.to_string() };
    result.stages[index].observation.observed_at = Some(epoch_now());
    let mut failure = PageBrowseFailure::default();
    failure.stage = result.stages[index].kind;
    failure.code = code.to_string();
    failure.message = message.to_string();
    failure.retryable = matches!(
        code,
        "path_discovery_failed"
            | "identity_resolution_failed"
            | "link_establishment_failed"
            | "identification_failed"
            | "request_submission_failed"
            | "transfer_failed"
            | "deadline_elapsed"
            | "request_timed_out"
            | "request_cancelled"
    );
    result.failure = Some(failure);
    for stage in result.stages.iter_mut().skip(index + 1) {
        if matches!(stage.state, PageBrowseStageState::Pending) {
            stage.state = PageBrowseStageState::Skipped { reason: format!("blocked by {code}") };
            stage.observation.observed_at = Some(epoch_now());
        }
    }
    terminalize(result, outcome);
}

fn page_failed(page: &PageContent) -> bool {
    !matches!(page.outcome, PageBrowseOutcome::Succeeded)
}

fn skip(result: &mut PageContent, index: usize, reason: &str) {
    result.stages[index].state = PageBrowseStageState::Skipped { reason: reason.into() };
    result.stages[index].observation.observed_at = Some(epoch_now());
}

fn observe_request_stage(stage: &mut PageBrowseStage, request: &RequestObservationInfo) {
    stage.observation = request.observation.clone();
    stage.observation.correlation_id = Some(stage.correlation_id.clone());
    stage.evidence_source = Some(request.observation.source);
    stage.link_id = Some(request.link_id.clone());
    stage.request_id = Some(request.request_id.clone());
    stage.resource_hash = request.resource_hash.clone();
}

fn terminalize(result: &mut PageContent, outcome: PageBrowseOutcome) {
    let completed_unix_ms = epoch_millis();
    result.outcome = outcome;
    result.completed_unix_ms = Some(completed_unix_ms);
    result.elapsed_ms = result.started_unix_ms.map(|started| {
        completed_unix_ms.saturating_sub(started).max(0).try_into().unwrap_or(u64::MAX)
    });
    result.observation = page_observation(&result.correlation_id, Some(epoch_now()));
}

fn page_observation(correlation: &str, observed_at: Option<i64>) -> ObservationMetadata {
    let mut observation = ObservationMetadata::default();
    observation.source = ObservationSource::OperationCoordinator;
    observation.observed_at = observed_at;
    observation.correlation_id = Some(correlation.to_string());
    observation
}

fn epoch_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn apply_request_metadata(
    result: &mut PageContent,
    started: &RequestObservationInfo,
    completed: &RequestObservationInfo,
) {
    result.request.request_id = Some(started.request_id.clone());
    result.request.link_id = Some(started.link_id.clone());
    result.request.request_size = started.request_size;
    result.request.response_size = completed.response_size;
    result.request.rtt_ms = completed.rtt_ms;
}

fn transfer_info(receipt: &RequestObservationInfo) -> PageTransferInfo {
    let mut transfer = PageTransferInfo::default();
    transfer.kind = match receipt.response_transfer {
        RequestResponseTransfer::Packet => PageTransferKind::Packet,
        RequestResponseTransfer::Resource => PageTransferKind::Resource,
        RequestResponseTransfer::None => PageTransferKind::None,
        _ => PageTransferKind::None,
    };
    transfer.received_bytes = receipt.received_bytes;
    transfer.total_bytes = receipt.total_bytes;
    transfer.progress = receipt.progress;
    transfer.resource_hash = receipt.resource_hash.clone();
    transfer.verified = receipt.state == RequestState::Succeeded;
    transfer
}

fn finish_projection(result: &mut PageContent, source: Vec<u8>) {
    result.source_checksum = hex::encode(Sha256::digest(&source));
    result.source_bytes = source;
    let source = match String::from_utf8(result.source_bytes.clone()) {
        Ok(source) => source,
        Err(error) => {
            result.parser_warnings.push(warning("invalid_utf8", error.to_string()));
            String::from_utf8_lossy(&result.source_bytes).into_owned()
        }
    };
    let document = styrene_micron::parse(&source);
    let projection = render_projection(&document, &mut result.parser_warnings);
    result.title = projection.title;
    result.links = projection.links;
    result.fields = projection.fields;
    result.link_targets = projection.link_targets;
    succeed(result, 6);
    result.rendered_text = projection.text;
    result.fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    succeed(result, 7);
}

struct Projection {
    text: String,
    title: Option<String>,
    links: Vec<String>,
    fields: Vec<PageFormField>,
    link_targets: Vec<PageLinkTarget>,
}

fn render_projection(document: &Document, warnings: &mut Vec<PageParserWarning>) -> Projection {
    let mut projection = Projection {
        text: String::new(),
        title: None,
        links: Vec::new(),
        fields: Vec::new(),
        link_targets: Vec::new(),
    };
    for block in &document.blocks {
        render_block(block, &mut projection, warnings);
    }
    projection.text = projection.text.trim_end_matches('\n').to_string();
    projection
}

fn render_block(block: &Block, projection: &mut Projection, warnings: &mut Vec<PageParserWarning>) {
    match block {
        Block::Section { heading, children, .. } => {
            if let Some(heading) = heading {
                let text = render_line(heading, projection, warnings);
                if projection.title.is_none() && !text.is_empty() {
                    projection.title = Some(text.clone());
                }
                projection.text.push_str(&text);
                projection.text.push('\n');
            }
            for child in children {
                render_child(child, projection, warnings);
            }
        }
        Block::Line(line) => {
            let rendered = render_line(line, projection, warnings);
            projection.text.push_str(&rendered);
            projection.text.push('\n');
        }
        Block::EmptyLine => projection.text.push('\n'),
        Block::Divider { symbol } => {
            projection.text.extend(std::iter::repeat_n(*symbol, 24));
            projection.text.push('\n');
        }
        Block::Literal { content } => {
            projection.text.push_str(content);
            projection.text.push('\n');
        }
        Block::Directive { key, .. } => warnings.push(warning(
            "directive_not_rendered",
            format!("Micron directive {key:?} is retained in the parse but not rendered"),
        )),
    }
}

fn render_child(
    child: &ChildBlock,
    projection: &mut Projection,
    warnings: &mut Vec<PageParserWarning>,
) {
    match child {
        ChildBlock::Section { heading, children, .. } => {
            if let Some(heading) = heading {
                let rendered = render_line(heading, projection, warnings);
                projection.text.push_str(&rendered);
                projection.text.push('\n');
            }
            for child in children {
                render_child(child, projection, warnings);
            }
        }
        ChildBlock::Line(line) => {
            let rendered = render_line(line, projection, warnings);
            projection.text.push_str(&rendered);
            projection.text.push('\n');
        }
        ChildBlock::EmptyLine => projection.text.push('\n'),
        ChildBlock::Divider { symbol } => {
            projection.text.extend(std::iter::repeat_n(*symbol, 24));
            projection.text.push('\n');
        }
        ChildBlock::Literal { content } => {
            projection.text.push_str(content);
            projection.text.push('\n');
        }
    }
}

fn render_line(
    line: &Line,
    projection: &mut Projection,
    _warnings: &mut Vec<PageParserWarning>,
) -> String {
    let mut rendered = String::new();
    for node in &line.nodes {
        match node {
            InlineNode::Text { text, .. } => rendered.push_str(text),
            InlineNode::Newline => rendered.push('\n'),
            InlineNode::Link { label, url, fields, .. } => {
                projection.links.push(url.clone());
                let mut target = PageLinkTarget::default();
                target.label = label.clone();
                target.target = url.clone();
                target.submitted_fields = fields.clone();
                projection.link_targets.push(target);
                rendered.push_str(label.as_deref().unwrap_or(url));
            }
            InlineNode::Field { field, .. } => {
                projection.fields.push(project_field(field));
            }
        }
    }
    rendered
}

fn project_field(field: &FormField) -> PageFormField {
    let mut projected = PageFormField::default();
    match field {
        FormField::Text { name, value, width } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Text;
            projected.value = Some(value.clone());
            projected.width = Some(*width);
        }
        FormField::Password { name, width, .. } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Password;
            projected.width = Some(*width);
        }
        FormField::Checkbox { name, value, checked } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Checkbox;
            projected.value = Some(value.clone());
            projected.checked = *checked;
        }
        FormField::Radio { name, value, checked } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Radio;
            projected.value = Some(value.clone());
            projected.checked = *checked;
        }
    }
    projected
}

fn encode_submission(
    submission: Option<&PageFormSubmission>,
    fields: &[PageFormField],
    link_fields: &[String],
) -> Result<Vec<u8>, String> {
    if submission.is_none() && link_fields.is_empty() {
        return Ok(vec![0xc0]);
    }
    let submission = submission.cloned().unwrap_or_default();
    if submission.values.len() > 128 {
        return Err("submitted field count exceeds 128".into());
    }
    for (name, values) in &submission.values {
        if name.is_empty()
            || name.len() > 128
            || values.len() > 128
            || values.iter().any(|value| value.len() > 16 * 1024 || value.contains('\0'))
        {
            return Err("submitted field state exceeds its bound".into());
        }
    }

    let all_fields = link_fields.iter().any(|field| field == "*");
    let mut selected = Vec::new();
    let mut entries = Vec::new();
    for directive in link_fields {
        if directive == "*" {
            continue;
        }
        if let Some((name, value)) = directive.split_once('=') {
            if name.is_empty() || value.contains('=') || name.len() > 124 || value.len() > 16 * 1024
            {
                return Err("invalid NomadNet link assignment".into());
            }
            entries.push((rmpv::Value::from(format!("var_{name}")), rmpv::Value::from(value)));
        } else {
            selected.push(directive.as_str());
        }
    }

    // A submission without any link field directive is an explicit request to
    // send exactly these values (CLI and harness form posts). Interactive links
    // only attach a submission when the Micron link declares fields, so this
    // never widens what an ordinary link sends.
    if link_fields.is_empty() {
        for (name, values) in &submission.values {
            entries.push((
                rmpv::Value::from(format!("field_{name}")),
                rmpv::Value::from(values.join(",")),
            ));
        }
    }

    let mut emitted = std::collections::HashSet::new();
    for field in fields {
        if !(all_fields || selected.iter().any(|name| *name == field.name)) {
            continue;
        }
        let Some(values) = submission.values.get(&field.name) else { continue };
        let value = match field.kind {
            PageFormFieldKind::Text | PageFormFieldKind::Password => values.first().cloned(),
            PageFormFieldKind::Radio => {
                field.value.as_ref().filter(|value| values.contains(value)).cloned()
            }
            PageFormFieldKind::Checkbox => {
                if emitted.contains(&field.name) {
                    continue;
                }
                let checked = fields
                    .iter()
                    .filter(|candidate| {
                        candidate.name == field.name
                            && candidate.kind == PageFormFieldKind::Checkbox
                    })
                    .filter_map(|candidate| candidate.value.as_ref())
                    .filter(|value| values.contains(value))
                    .cloned()
                    .collect::<Vec<_>>();
                (!checked.is_empty()).then(|| checked.join(","))
            }
            _ => None,
        };
        if let Some(value) = value {
            emitted.insert(field.name.clone());
            entries.push((
                rmpv::Value::from(format!("field_{}", field.name)),
                rmpv::Value::from(value),
            ));
        }
    }
    let mut encoded = Vec::new();
    rmpv::encode::write_value(&mut encoded, &rmpv::Value::Map(entries))
        .map_err(|error| format!("encode native submitted fields: {error}"))?;
    Ok(encoded)
}

fn navigation_info(
    session_id: &str,
    address: &str,
    session: &BrowseSession,
    connection_open: bool,
) -> PageNavigationInfo {
    let mut info = PageNavigationInfo::default();
    info.session_id = session_id.to_string();
    info.address = address.to_string();
    info.history_index = session.position.try_into().unwrap_or(u32::MAX);
    info.history_len = session.history.len().try_into().unwrap_or(u32::MAX);
    info.can_back = session.position > 0;
    info.can_forward = session.position + 1 < session.history.len();
    info.connection_open = connection_open && session.link.is_some();
    info
}

fn resolve_file_target(
    target: &str,
    current: Option<&PageAddress>,
) -> Result<(String, String), String> {
    let target = target.trim();
    let (host, path) = if let Some((host, path)) = target.split_once(":/") {
        (
            styrene_ipc::NomadNetHost::parse(host).map_err(|error| error.to_string())?.to_string(),
            format!("/{path}"),
        )
    } else {
        let current = current.ok_or("relative file target requires an active page session")?;
        let host =
            current.host().ok_or("file download requires a remote NomadNet host")?.to_string();
        let path = if target.starts_with('/') {
            target.to_string()
        } else if target.starts_with(":/") {
            target[1..].to_string()
        } else {
            let parent = current.path().rsplit_once('/').map_or("/page", |(parent, _)| parent);
            normalize_path(&format!("{parent}/{target}"))?
        };
        (host, path)
    };
    let path = normalize_path(&path)?;
    if !matches!(styrene_ipc::NomadNetPath::parse(&path), Ok(styrene_ipc::NomadNetPath::File(_))) {
        return Err("file download target must resolve to /file/...".into());
    }
    Ok((host, path))
}

fn normalize_path(path: &str) -> Result<String, String> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err("relative target escapes the native path root".into());
                }
            }
            value => segments.push(value),
        }
    }
    Ok(format!("/{}", segments.join("/")))
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn decode_destination(host: &str) -> Result<AddressHash, String> {
    let bytes: [u8; 16] = hex::decode(host)
        .map_err(|_| "destination hash is not hexadecimal".to_string())?
        .try_into()
        .map_err(|_| "destination hash must be 16 bytes".to_string())?;
    Ok(AddressHash::new(bytes))
}

fn warning(code: &str, message: String) -> PageParserWarning {
    let mut warning = PageParserWarning::default();
    warning.code = code.to_string();
    warning.message = message;
    warning
}

fn decode_binary_response(response: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(response);
    let value = rmpv::decode::read_value(&mut cursor).ok()?;
    if usize::try_from(cursor.position()).ok() != Some(response.len()) {
        return None;
    }
    match value {
        rmpv::Value::Binary(bytes) => Some(bytes),
        _ => None,
    }
}

fn correlation_id() -> String {
    let sequence = NEXT_CORRELATION.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("page-{timestamp:032x}-{sequence:016x}")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rns_core::identity::PrivateIdentity;

    use super::*;
    use crate::services::discovery::NATIVE_NOMADNET_HOST_DEVICE_TYPE;
    use crate::transport::mock_transport::{MockCall, MockTransport};

    const HOST: &str = "0123456789abcdef0123456789abcdef";

    struct ScriptedBackend {
        identity: Option<Identity>,
        outcome: Mutex<VecDeque<NativeRequestOutcome>>,
        calls: Mutex<Vec<&'static str>>,
        path_delay: Duration,
        identity_delay: Duration,
        link_delay: Duration,
        request_delay: Duration,
        path_permits: Option<Arc<tokio::sync::Semaphore>>,
        identified_as: Mutex<Vec<AddressHash>>,
        requested_data: Mutex<Vec<Vec<u8>>>,
        next_link: AtomicU64,
        link_created: bool,
        close_failures: std::sync::atomic::AtomicUsize,
    }

    impl ScriptedBackend {
        fn success(transfer: RequestResponseTransfer, source: &[u8]) -> Self {
            let mut started = RequestObservationInfo::default();
            started.request_id = "11".repeat(16);
            started.link_id = "22".repeat(16);
            started.request_size = 24;
            started.state = RequestState::Pending;

            let mut encoded = Vec::new();
            rmpv::encode::write_value(&mut encoded, &rmpv::Value::Binary(source.to_vec()))
                .expect("encode response");
            let mut completed = started.clone();
            completed.state = RequestState::Succeeded;
            completed.response_transfer = transfer;
            completed.response = Some(encoded);
            completed.response_size = Some(source.len().try_into().unwrap_or(u64::MAX));
            completed.received_bytes = source.len().try_into().unwrap_or(u64::MAX);
            completed.total_bytes = completed.received_bytes;
            completed.progress = 1.0;
            completed.rtt_ms = Some(19);
            if transfer == RequestResponseTransfer::Resource {
                completed.resource_hash = Some("33".repeat(32));
            }

            Self {
                identity: Some(
                    *PrivateIdentity::new_from_name("native-browser-peer").as_identity(),
                ),
                outcome: Mutex::new(VecDeque::from([NativeRequestOutcome { started, completed }])),
                calls: Mutex::new(Vec::new()),
                path_delay: Duration::ZERO,
                identity_delay: Duration::ZERO,
                link_delay: Duration::ZERO,
                request_delay: Duration::ZERO,
                path_permits: None,
                identified_as: Mutex::new(Vec::new()),
                requested_data: Mutex::new(Vec::new()),
                next_link: AtomicU64::new(1),
                link_created: true,
                close_failures: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn identity_failure() -> Self {
            Self {
                identity: None,
                outcome: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
                path_delay: Duration::ZERO,
                identity_delay: Duration::ZERO,
                link_delay: Duration::ZERO,
                request_delay: Duration::ZERO,
                path_permits: None,
                identified_as: Mutex::new(Vec::new()),
                requested_data: Mutex::new(Vec::new()),
                next_link: AtomicU64::new(1),
                link_created: true,
                close_failures: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl BrowseBackend for ScriptedBackend {
        async fn discover_path(
            &self,
            _destination: AddressHash,
            cancellation: &tokio_util::sync::CancellationToken,
            deadline: tokio::time::Instant,
        ) -> Result<(), BrowseError> {
            self.calls.lock().unwrap().push("path");
            if let Some(permits) = &self.path_permits {
                let permit = permits.acquire().await.map_err(|_| BrowseError::Cancelled)?;
                permit.forget();
            }
            tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                () = tokio::time::sleep(self.path_delay) => {}
            }
            remaining(deadline)?;
            Ok(())
        }

        async fn resolve_identity(
            &self,
            _destination: AddressHash,
            cancellation: &tokio_util::sync::CancellationToken,
            deadline: tokio::time::Instant,
        ) -> Result<Identity, BrowseError> {
            self.calls.lock().unwrap().push("identity");
            tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                () = tokio::time::sleep(self.identity_delay) => {}
            }
            remaining(deadline)?;
            self.identity.ok_or_else(|| BrowseError::Transport("identity unavailable".into()))
        }

        async fn open_link(
            &self,
            _destination: DestinationDesc,
            cancellation: &tokio_util::sync::CancellationToken,
            deadline: tokio::time::Instant,
        ) -> Result<BrowserLink, BrowseError> {
            self.calls.lock().unwrap().push("link");
            tokio::select! {
                () = cancellation.cancelled() => return Err(BrowseError::Cancelled),
                () = tokio::time::sleep(self.link_delay) => {}
            }
            remaining(deadline)?;
            Ok(BrowserLink {
                id: format!("{:032x}", self.next_link.fetch_add(1, Ordering::Relaxed)),
                created: self.link_created,
            })
        }

        async fn identify_link(
            &self,
            _link_id: &str,
            identity: Arc<PrivateIdentity>,
            cancellation: &tokio_util::sync::CancellationToken,
            deadline: tokio::time::Instant,
        ) -> Result<(), BrowseError> {
            self.calls.lock().unwrap().push("identify");
            if cancellation.is_cancelled() {
                return Err(BrowseError::Cancelled);
            }
            self.identified_as.lock().unwrap().push(*identity.address_hash());
            remaining(deadline)?;
            Ok(())
        }

        async fn request(
            &self,
            request: NativeRequest,
        ) -> Result<NativeRequestOutcome, BrowseError> {
            let NativeRequest { correlation_id, data, cancellation, progress, deadline, .. } =
                request;
            self.calls.lock().unwrap().push("request");
            self.requested_data.lock().unwrap().push(data);
            tokio::select! {
                () = cancellation.cancelled() => {
                    return Err(BrowseError::Transport("request cancelled".into()));
                }
                () = tokio::time::sleep(self.request_delay) => {}
            }
            remaining(deadline)?;
            let mut outcome =
                self.outcome.lock().unwrap().pop_front().ok_or(BrowseError::MissingReceipt)?;
            outcome.started.observation.correlation_id = Some(correlation_id.clone());
            outcome.completed.observation.correlation_id = Some(correlation_id);
            if let Some(progress) = progress {
                let mut receiving = outcome.started.clone();
                receiving.state = RequestState::Receiving;
                receiving.received_bytes = outcome.completed.received_bytes / 2;
                receiving.total_bytes = outcome.completed.total_bytes;
                receiving.progress = 0.5;
                receiving.response_transfer = outcome.completed.response_transfer;
                progress(receiving);
            }
            Ok(outcome)
        }

        async fn close_link(&self, _link_id: &str) -> Result<(), BrowseError> {
            self.calls.lock().unwrap().push("close");
            if self
                .close_failures
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                    if remaining > 0 { Some(remaining - 1) } else { None }
                })
                .is_ok()
            {
                return Err(BrowseError::Transport("scripted close failure".into()));
            }
            Ok(())
        }
    }

    fn coordinator(backend: Arc<ScriptedBackend>) -> NativeNomadNetBrowseCoordinator {
        let discovery = Arc::new(DiscoveryService::new());
        discovery
            .accept_announce_with_type(
                HOST.into(),
                1,
                b"Native host",
                Some(NATIVE_NOMADNET_HOST_DEVICE_TYPE),
            )
            .expect("native announce");
        NativeNomadNetBrowseCoordinator::with_backend(backend, discovery)
    }

    fn identified_coordinator(backend: Arc<ScriptedBackend>) -> NativeNomadNetBrowseCoordinator {
        let coordinator = coordinator(backend);
        coordinator.set_identity(Arc::new(PrivateIdentity::new_from_name("selected-local-reader")));
        coordinator
    }

    #[tokio::test]
    async fn deterministic_success_uses_one_correlation_and_preserves_source_projection() {
        let source = b">Index\nHello `[next`next.mu`]\n`<name`value>`";
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
        let coordinator = coordinator(backend.clone());

        let result =
            coordinator.browse_remote(HOST, "/page/index.mu", Duration::from_secs(1)).await;

        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            ["path", "identity", "link", "request", "close"]
        );
        assert_eq!(result.source_bytes, source);
        assert_eq!(result.source_checksum, hex::encode(Sha256::digest(source)));
        assert_eq!(result.title.as_deref(), Some("Index"));
        assert_eq!(result.links, ["next.mu"]);
        assert_eq!(result.request.native_path, "/page/index.mu");
        assert_eq!(result.cache.status, PageCacheStatus::NotUsed);
        assert_eq!(result.cache.stored_at, None);
        assert_eq!(result.fields.len(), 1);
        assert_eq!(result.transfer.kind, PageTransferKind::Packet);
        assert_eq!(result.outcome, PageBrowseOutcome::Succeeded);
        assert!(result.failure.is_none());
        assert!(result.started_unix_ms.is_some());
        assert!(result.completed_unix_ms.is_some());
        assert!(result.elapsed_ms.is_some());
        assert_eq!(
            result.observation.correlation_id.as_deref(),
            Some(result.correlation_id.as_str())
        );
        assert!(result.stages.iter().all(|stage| stage.correlation_id == result.correlation_id));
        assert!(result.stages.iter().all(|stage| {
            stage.observation.correlation_id.as_deref() == Some(result.correlation_id.as_str())
                && stage.observation.observed_at.is_some()
        }));
        assert_eq!(result.stages[0].evidence_source, Some(ObservationSource::TransportPathTable));
        assert_eq!(result.stages[4].request_id, Some("11".repeat(16)));
        assert!(result.stages.iter().enumerate().all(|(index, stage)| {
            index == 3 && matches!(stage.state, PageBrowseStageState::Skipped { .. })
                || index != 3 && stage.state == PageBrowseStageState::Succeeded
        }));
    }

    #[tokio::test]
    async fn identity_failure_blocks_all_later_stages_without_fabricated_success() {
        let backend = Arc::new(ScriptedBackend::identity_failure());
        let coordinator = coordinator(backend.clone());

        let result =
            coordinator.browse_remote(HOST, "/page/index.mu", Duration::from_secs(1)).await;

        assert_eq!(backend.calls.lock().unwrap().as_slice(), ["path", "identity"]);
        assert_eq!(result.stages[0].state, PageBrowseStageState::Succeeded);
        assert!(matches!(result.stages[1].state, PageBrowseStageState::Failed { .. }));
        assert!(
            result
                .stages
                .iter()
                .skip(2)
                .all(|stage| { matches!(stage.state, PageBrowseStageState::Skipped { .. }) })
        );
        assert!(result.source_bytes.is_empty());
        assert_eq!(result.outcome, PageBrowseOutcome::Failed);
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.stage),
            Some(PageBrowseStageKind::IdentityResolution)
        );
    }

    #[tokio::test]
    async fn resource_receipt_drives_verified_resource_transfer_metadata() {
        let source = vec![b'x'; 4096];
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, &source));
        let coordinator = coordinator(backend);

        let result =
            coordinator.browse_remote(HOST, "/page/large.mu", Duration::from_secs(1)).await;

        assert_eq!(result.transfer.kind, PageTransferKind::Resource);
        assert_eq!(result.transfer.resource_hash, Some("33".repeat(32)));
        assert!(result.transfer.verified);
        assert_eq!(result.transfer.received_bytes, 4096);
        assert_eq!(result.source_bytes, source);
        assert_eq!(result.request.request_id, Some("11".repeat(16)));
    }

    #[tokio::test]
    async fn file_download_rejects_unannounced_host_before_transport() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"not reached"));
        let coordinator = NativeNomadNetBrowseCoordinator::with_backend(
            backend.clone(),
            Arc::new(DiscoveryService::new()),
        );
        let cancellation = tokio_util::sync::CancellationToken::new();

        let error = coordinator
            .download_native(
                1,
                HOST,
                "/file/manual.bin",
                "file-correlation",
                tokio::time::Instant::now() + Duration::from_secs(1),
                cancellation,
            )
            .await
            .expect_err("unannounced host must fail closed");

        assert!(error.to_string().contains("no native NomadNet announce"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn selected_identity_authenticates_link_and_correlation_reaches_request_observations() {
        let backend = Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b">Allowed\nauthenticated",
        ));
        let coordinator = identified_coordinator(backend.clone());

        let result =
            coordinator.browse_remote(HOST, "/page/allowed.mu", Duration::from_secs(1)).await;

        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            ["path", "identity", "link", "identify", "request", "close"]
        );
        assert_eq!(result.stages[3].state, PageBrowseStageState::Succeeded);
        assert_eq!(
            backend.identified_as.lock().unwrap().as_slice(),
            [*PrivateIdentity::new_from_name("selected-local-reader").address_hash()]
        );
        assert!(result.transfer.verified);
    }

    #[tokio::test]
    async fn identified_denial_is_terminal_and_blocks_parse_and_render() {
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::None, b""));
        {
            let mut outcome = backend.outcome.lock().unwrap();
            let request = outcome.front_mut().expect("scripted request");
            request.completed.state = RequestState::TimedOut;
            request.completed.response = None;
        }
        let coordinator = identified_coordinator(backend);

        let result =
            coordinator.browse_remote(HOST, "/page/private.mu", Duration::from_secs(1)).await;

        assert_eq!(result.stages[3].state, PageBrowseStageState::Succeeded);
        assert!(matches!(result.stages[5].state, PageBrowseStageState::Failed { .. }));
        assert!(
            result
                .stages
                .iter()
                .skip(6)
                .all(|stage| matches!(stage.state, PageBrowseStageState::Skipped { .. }))
        );
        assert_eq!(result.outcome, PageBrowseOutcome::TimedOut);
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("request_timed_out")
        );
    }

    #[tokio::test]
    async fn one_absolute_deadline_is_not_restarted_between_stages() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">late");
        scripted.path_delay = Duration::from_millis(20);
        let backend = Arc::new(scripted);
        let coordinator = coordinator(backend.clone());

        let result =
            coordinator.browse_remote(HOST, "/page/late.mu", Duration::from_millis(5)).await;

        assert_eq!(backend.calls.lock().unwrap().as_slice(), ["path"]);
        assert!(matches!(result.stages[0].state, PageBrowseStageState::Failed { .. }));
        assert!(
            result
                .stages
                .iter()
                .skip(1)
                .all(|stage| { matches!(stage.state, PageBrowseStageState::Skipped { .. }) })
        );
        assert_eq!(result.outcome, PageBrowseOutcome::TimedOut);
    }

    #[tokio::test]
    async fn oversized_source_is_rejected_before_parse() {
        let source = vec![b'x'; MAX_PAGE_SOURCE_SIZE + 1];
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, &source));
        let coordinator = coordinator(backend);

        let result =
            coordinator.browse_remote(HOST, "/page/oversize.mu", Duration::from_secs(1)).await;

        assert!(matches!(
            &result.stages[5].state,
            PageBrowseStageState::Failed { code, .. } if code == "response_too_large"
        ));
        assert!(result.source_bytes.is_empty());
    }

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
        let backend = TransportBrowseBackend { transport: transport.clone() };

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
        let backend = TransportBrowseBackend { transport: transport.clone() };
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

    #[tokio::test]
    async fn daemon_history_cache_and_reload_are_deterministic() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"unused"));
        let coordinator = coordinator(backend);
        let loads = std::sync::atomic::AtomicUsize::new(0);
        let mut navigate = PageNavigationRequest::default();
        navigate.target = Some("/page/index.mu".into());
        let first = coordinator
            .navigate(navigate, HOST, |path| {
                loads.fetch_add(1, Ordering::Relaxed);
                format!(">{path}").into_bytes()
            })
            .await
            .expect("first page");
        let session_id = first.navigation.session_id.clone();

        let mut next = PageNavigationRequest::default();
        next.session_id = Some(session_id.clone());
        next.target = Some("docs.mu".into());
        let second = coordinator
            .navigate(next, HOST, |path| {
                loads.fetch_add(1, Ordering::Relaxed);
                format!(">{path}").into_bytes()
            })
            .await
            .expect("second page");
        assert_eq!(second.navigation.history_len, 2);

        let mut back = PageNavigationRequest::default();
        back.session_id = Some(session_id.clone());
        back.action = PageNavigationAction::Back;
        let previous = coordinator
            .navigate(back, HOST, |_| panic!("back must use the daemon cache"))
            .await
            .expect("back");
        assert_eq!(previous.cache.status, PageCacheStatus::Hit);
        assert_ne!(previous.correlation_id, first.correlation_id);
        assert_eq!(
            previous.cache.origin_correlation_id.as_deref(),
            Some(first.correlation_id.as_str())
        );
        assert_eq!(previous.transfer.kind, PageTransferKind::Cache);
        assert_eq!(previous.outcome, PageBrowseOutcome::Succeeded);
        assert!(previous.stages.iter().all(|stage| {
            matches!(stage.state, PageBrowseStageState::Skipped { .. })
                && stage.observation.correlation_id.as_deref()
                    == Some(previous.correlation_id.as_str())
        }));
        assert_eq!(previous.navigation.history_len, 2);
        assert!(previous.navigation.can_forward);

        let mut reload = PageNavigationRequest::default();
        reload.session_id = Some(session_id.clone());
        reload.action = PageNavigationAction::Reload;
        let reloaded = coordinator
            .navigate(reload, HOST, |path| {
                loads.fetch_add(1, Ordering::Relaxed);
                format!(">reloaded {path}").into_bytes()
            })
            .await
            .expect("reload");
        assert_eq!(reloaded.cache.status, PageCacheStatus::Bypassed);
        assert_eq!(reloaded.navigation.history_len, 2);
        assert_eq!(loads.load(Ordering::Relaxed), 3);

        let closed = coordinator.close_session(&session_id).await.expect("close session");
        assert_eq!(closed.history_len, 2);
        assert!(!closed.connection_open);
    }

    #[test]
    fn form_projection_redacts_passwords_and_submission_is_native_messagepack() {
        let mut page = initial_result(HOST, "/page/form.mu", "form", PageCacheStatus::NotUsed);
        finish_projection(
            &mut page,
            b"`<name`Ada> `<12!|password`secret> `[Submit`next.mu`name|password]".to_vec(),
        );
        assert_eq!(page.fields.len(), 2);
        assert_eq!(page.fields[0].value.as_deref(), Some("Ada"));
        assert_eq!(page.fields[1].kind, PageFormFieldKind::Password);
        assert_eq!(page.fields[1].value, None);
        assert_eq!(page.link_targets[0].submitted_fields, ["name", "password"]);

        let mut submission = PageFormSubmission::default();
        submission.values.insert("name".into(), vec!["Grace".into()]);
        submission.values.insert("password".into(), vec!["swordfish".into()]);
        submission.values.insert("opts".into(), vec!["blue".into(), "red".into()]);
        assert!(!format!("{submission:?}").contains("swordfish"));
        let mut checkbox_red = PageFormField::default();
        checkbox_red.name = "opts".into();
        checkbox_red.kind = PageFormFieldKind::Checkbox;
        checkbox_red.value = Some("red".into());
        let mut checkbox_blue = checkbox_red.clone();
        checkbox_blue.value = Some("blue".into());
        let encoded = encode_submission(
            Some(&submission),
            &[page.fields[0].clone(), page.fields[1].clone(), checkbox_red, checkbox_blue],
            &["mode=safe".into(), "*".into()],
        )
        .expect("native map");
        assert_eq!(
            hex::encode(&encoded),
            "84a87661725f6d6f6465a473616665aa6669656c645f6e616d65a54772616365ae6669656c645f70617373776f7264a973776f726466697368aa6669656c645f6f707473a87265642c626c7565"
        );
        let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
        assert!(matches!(decoded, rmpv::Value::Map(values) if values.len() == 4));
    }

    #[test]
    fn explicit_submission_without_link_directive_sends_named_fields() {
        let mut submission = PageFormSubmission::default();
        submission.values.insert("name".into(), vec!["rust".into()]);
        submission.values.insert("opts".into(), vec!["red".into(), "blue".into()]);
        let encoded = encode_submission(Some(&submission), &[], &[]).expect("native map");
        let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
        let rmpv::Value::Map(values) = decoded else { panic!("map") };
        assert_eq!(
            values,
            vec![
                (rmpv::Value::from("field_name"), rmpv::Value::from("rust")),
                (rmpv::Value::from("field_opts"), rmpv::Value::from("red,blue")),
            ]
        );
        assert_eq!(encode_submission(None, &[], &[]).expect("nil"), vec![0xc0]);
    }

    #[tokio::test]
    async fn submitted_fields_reach_native_request_and_close_does_not_navigate() {
        let source = b"`<name`Ada> `<12!|password`> `[Submit`next.mu`mode=safe|name|password]";
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
        let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
        backend.outcome.lock().unwrap().push_back(duplicate);
        let coordinator = coordinator(backend.clone());
        let mut initial = PageNavigationRequest::default();
        initial.target = Some(format!("{HOST}:/page/form.mu"));
        let first = coordinator.navigate(initial, HOST, |_| Vec::new()).await.expect("form page");
        let mut submission = PageFormSubmission::default();
        submission.values.insert("name".into(), vec!["Ada".into()]);
        submission.values.insert("password".into(), vec!["secret".into()]);
        let mut request = PageNavigationRequest::default();
        request.session_id = Some(first.navigation.session_id.clone());
        request.target = Some("next.mu".into());
        request.submission = Some(submission);
        let page = coordinator.navigate(request, HOST, |_| Vec::new()).await.expect("submit");
        let encoded = backend.requested_data.lock().unwrap()[1].clone();
        assert!(matches!(
            rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap(),
            rmpv::Value::Map(values) if values.len() == 3
        ));
        let calls_before_close = backend.calls.lock().unwrap().len();
        let closed = coordinator.close_session(&page.navigation.session_id).await.expect("close");
        assert_eq!(closed.history_len, 2);
        assert_eq!(&backend.calls.lock().unwrap()[calls_before_close..], ["close"]);
    }

    #[tokio::test]
    async fn resource_request_reports_deterministic_progress_before_completion() {
        let backend = ScriptedBackend::success(RequestResponseTransfer::Resource, b"resource");
        let observations = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&observations);
        backend
            .request(NativeRequest {
                link_id: "22".repeat(16),
                path: "/file/resource.bin".into(),
                correlation_id: "download-progress".into(),
                data: vec![0xc0],
                max_response_size: MAX_ENCODED_FILE_RESPONSE_SIZE,
                cancellation: tokio_util::sync::CancellationToken::new(),
                progress: Some(Arc::new(move |receipt| captured.lock().unwrap().push(receipt))),
                deadline: tokio::time::Instant::now() + Duration::from_secs(1),
            })
            .await
            .expect("resource request");
        let observations = observations.lock().unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].state, RequestState::Receiving);
        assert_eq!(observations[0].progress, 0.5);
    }

    #[tokio::test]
    async fn file_download_verifies_integrity_and_requires_explicit_save() {
        let bytes = b"verified file bytes";
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Resource, bytes));
        let coordinator = Arc::new(coordinator(backend.clone()));
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("saved.bin");
        let mut request = FileDownloadRequest::default();
        request.target = format!("{HOST}:/file/archive.bin");
        request.expected_sha256 = Some(hex::encode(Sha256::digest(bytes)));
        let started = coordinator.start_download(request).await.expect("start download");
        let completed = loop {
            let current = coordinator.download(&started.download_id).await.unwrap();
            if current.state.is_terminal() {
                break current;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(completed.state, FileDownloadState::Completed);
        assert!(completed.integrity_verified);
        assert!(!destination.exists());
        assert!(
            coordinator
                .save_download(&started.download_id, Path::new("saved.bin"))
                .await
                .unwrap_err()
                .contains("absolute path")
        );
        let existing = root.path().join("existing.bin");
        std::fs::write(&existing, b"operator data").unwrap();
        assert!(coordinator.save_download(&started.download_id, &existing).await.is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"operator data");
        let missing_parent = root.path().join("missing").join("saved.bin");
        assert!(coordinator.save_download(&started.download_id, &missing_parent).await.is_err());
        assert!(!missing_parent.exists());
        let saved = coordinator
            .save_download(&started.download_id, &destination)
            .await
            .expect("explicit save");
        assert_eq!(saved.state, FileDownloadState::Saved);
        assert_eq!(std::fs::read(destination).unwrap(), bytes);
        assert_eq!(
            backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
            1,
            "download link leaked or closed more than once"
        );
    }

    #[tokio::test]
    async fn file_download_cancellation_returns_completed_cancelled_state() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Resource, b"late");
        scripted.request_delay = Duration::from_secs(60);
        let coordinator = Arc::new(coordinator(Arc::new(scripted)));
        let mut request = FileDownloadRequest::default();
        request.target = format!("{HOST}:/file/late.bin");
        let started = coordinator.start_download(request).await.expect("start download");
        tokio::task::yield_now().await;
        let cancelled = coordinator.cancel_download(&started.download_id).await.expect("cancel");
        assert_eq!(cancelled.state, FileDownloadState::Cancelled);
        assert!(cancelled.error.is_some());
    }

    #[tokio::test]
    async fn session_capacity_evicts_terminal_then_lru_and_never_active_work() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b"unused"));
        let bounded = coordinator(backend.clone());
        {
            let mut sessions = bounded.sessions.lock().unwrap();
            for index in 0..MAX_SESSIONS {
                let mut session = BrowseSession::new(0, index as u64 + 1);
                session.active = index == 0;
                session.terminal = index == 1;
                session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
                sessions.insert(format!("session-{index}"), session);
            }
        }
        let mut request = PageNavigationRequest::default();
        request.target = Some("/page/new.mu".into());
        bounded
            .navigate(request, HOST, |_| b">new".to_vec())
            .await
            .expect("terminal session is evicted");
        {
            let sessions = bounded.sessions.lock().unwrap();
            assert!(sessions.contains_key("session-0"), "active work was evicted");
            assert!(!sessions.contains_key("session-1"), "terminal session was retained");
        }
        assert!(backend.calls.lock().unwrap().contains(&"close"));

        let coordinator = coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        )));
        {
            let mut sessions = coordinator.sessions.lock().unwrap();
            for index in 0..MAX_SESSIONS {
                let mut session = BrowseSession::new(0, index as u64);
                session.active = true;
                sessions.insert(format!("active-{index}"), session);
            }
        }
        let mut request = PageNavigationRequest::default();
        request.target = Some("/page/rejected.mu".into());
        assert_eq!(
            coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap_err(),
            "page session capacity is full"
        );
        assert_eq!(coordinator.sessions.lock().unwrap().len(), MAX_SESSIONS);
    }

    #[tokio::test]
    async fn full_capacity_invalid_submission_does_not_evict_sessions_or_links() {
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">new"));
        let coordinator = coordinator(backend.clone());
        {
            let mut sessions = coordinator.sessions.lock().unwrap();
            for index in 0..MAX_SESSIONS {
                let mut session = BrowseSession::new(0, index as u64);
                session.terminal = true;
                session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
                sessions.insert(format!("preserved-{index}"), session);
            }
        }
        let before = coordinator
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), session.link.clone()))
            .collect::<HashMap<_, _>>();
        let mut submission = PageFormSubmission::default();
        submission.values.insert(String::new(), vec!["invalid".into()]);
        let mut request = PageNavigationRequest::default();
        request.session_id = Some("replacement".into());
        request.target = Some(format!("{HOST}:/page/new.mu"));
        request.submission = Some(submission);

        assert_eq!(
            coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap_err(),
            "submitted field state exceeds its bound"
        );
        let after = coordinator
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), session.link.clone()))
            .collect::<HashMap<_, _>>();
        assert_eq!(after, before);
        assert!(!backend.calls.lock().unwrap().contains(&"close"));
    }

    #[tokio::test]
    async fn concurrent_session_reservations_are_atomic_and_bounded() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">race");
        let outcome = scripted.outcome.lock().unwrap().front().cloned().unwrap();
        scripted.outcome.lock().unwrap().extend(std::iter::repeat_n(outcome, MAX_SESSIONS - 1));
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        scripted.path_permits = Some(Arc::clone(&permits));
        let backend = Arc::new(scripted);
        let coordinator = Arc::new(coordinator(backend.clone()));
        let callers = MAX_SESSIONS * 2;
        let start = Arc::new(tokio::sync::Barrier::new(callers + 1));
        let mut tasks = Vec::new();
        for index in 0..callers {
            let coordinator = Arc::clone(&coordinator);
            let start = Arc::clone(&start);
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                let mut request = PageNavigationRequest::default();
                request.session_id = Some(format!("race-{index}"));
                request.target = Some(format!("{HOST}:/page/race.mu"));
                coordinator.navigate(request, HOST, |_| Vec::new()).await
            }));
        }
        start.wait().await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let calls =
                    backend.calls.lock().unwrap().iter().filter(|call| **call == "path").count();
                let rejected = tasks.iter().filter(|task| task.is_finished()).count();
                if calls == MAX_SESSIONS && rejected == MAX_SESSIONS {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all capacity contenders reached a deterministic state");
        {
            let sessions = coordinator.sessions.lock().unwrap();
            assert_eq!(sessions.len(), MAX_SESSIONS);
            assert!(sessions.values().all(|session| session.active));
        }
        permits.add_permits(MAX_SESSIONS);
        let mut succeeded = 0;
        let mut rejected = 0;
        for task in tasks {
            match task.await.expect("navigation task") {
                Ok(_) => succeeded += 1,
                Err(error) if error == "page session capacity is full" => rejected += 1,
                Err(error) => panic!("unexpected navigation error: {error}"),
            }
        }
        assert_eq!(succeeded, MAX_SESSIONS);
        assert_eq!(rejected, MAX_SESSIONS);
        assert!(coordinator.sessions.lock().unwrap().len() <= MAX_SESSIONS);
    }

    #[tokio::test]
    async fn concurrent_navigation_of_one_session_is_rejected() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">first");
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        scripted.path_permits = Some(Arc::clone(&permits));
        let backend = Arc::new(scripted);
        let coordinator = Arc::new(coordinator(backend.clone()));
        let first_coordinator = Arc::clone(&coordinator);
        let first = tokio::spawn(async move {
            let mut request = PageNavigationRequest::default();
            request.session_id = Some("shared".into());
            request.target = Some(format!("{HOST}:/page/index.mu"));
            first_coordinator.navigate(request, HOST, |_| Vec::new()).await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !backend.calls.lock().unwrap().contains(&"path") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first navigation acquired the session");
        let mut concurrent = PageNavigationRequest::default();
        concurrent.session_id = Some("shared".into());
        concurrent.target = Some(format!("{HOST}:/page/other.mu"));
        assert_eq!(
            coordinator.navigate(concurrent, HOST, |_| Vec::new()).await.unwrap_err(),
            "page session already has active work"
        );
        permits.add_permits(1);
        first.await.expect("first task").expect("first navigation");
    }

    #[tokio::test]
    async fn close_removes_session_and_superseded_links_are_retired() {
        let source = b"`[Next`next.mu]";
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, source));
        let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
        backend.outcome.lock().unwrap().push_back(duplicate);
        let coordinator = coordinator(backend.clone());
        let mut first = PageNavigationRequest::default();
        first.target = Some(format!("{HOST}:/page/index.mu"));
        let first = coordinator.navigate(first, HOST, |_| Vec::new()).await.unwrap();
        let mut next = PageNavigationRequest::default();
        next.session_id = Some(first.navigation.session_id.clone());
        next.target = Some("next.mu".into());
        let next = coordinator.navigate(next, HOST, |_| Vec::new()).await.unwrap();
        coordinator.close_session(&next.navigation.session_id).await.unwrap();
        assert!(!coordinator.sessions.lock().unwrap().contains_key(&next.navigation.session_id));
        assert_eq!(
            backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
            2,
            "superseded and current links must each be retired exactly once"
        );
    }

    #[tokio::test]
    async fn failed_close_preserves_the_session_for_retry() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">still open"));
        let coordinator = coordinator(backend.clone());
        let mut request = PageNavigationRequest::default();
        request.target = Some(format!("{HOST}:/page/index.mu"));
        let page = coordinator.navigate(request, HOST, |_| Vec::new()).await.unwrap();
        let link_id = coordinator
            .sessions
            .lock()
            .unwrap()
            .get(&page.navigation.session_id)
            .and_then(|session| session.link.as_ref())
            .map(|link| link.id.clone())
            .unwrap();
        backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);

        assert!(coordinator.close_session(&page.navigation.session_id).await.is_err());
        {
            let sessions = coordinator.sessions.lock().unwrap();
            let session = sessions.get(&page.navigation.session_id).expect("session retained");
            assert!(!session.active);
            assert_eq!(
                session.current.as_ref().unwrap().navigation.address,
                page.navigation.address
            );
        }
        assert_eq!(
            coordinator.cleanup.status(&link_id).await,
            Some(LinkCleanupStatus::TerminalError {
                attempts: MAX_LINK_CLEANUP_ATTEMPTS,
                error: "scripted close failure".into(),
            })
        );
        assert_eq!(
            backend.calls.lock().unwrap().iter().filter(|call| **call == "close").count(),
            usize::from(MAX_LINK_CLEANUP_ATTEMPTS)
        );
    }

    #[tokio::test]
    async fn owner_cleanup_retains_terminal_link_ownership_until_recovery() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">owned"));
        let coordinator = coordinator(backend.clone());
        let mut request = PageNavigationRequest::default();
        request.target = Some(format!("{HOST}:/page/owned.mu"));
        let page = coordinator.navigate_for_owner(42, request, HOST, |_| Vec::new()).await.unwrap();
        let link_id = coordinator
            .sessions
            .lock()
            .unwrap()
            .get(&page.navigation.session_id)
            .and_then(|session| session.link.as_ref())
            .map(|link| link.id.clone())
            .unwrap();
        backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);

        let terminal = coordinator.cleanup_owner(42).await.unwrap_err();

        assert!(terminal.contains("cleanup failed after 3 attempts"));
        assert!(!coordinator.sessions.lock().unwrap().contains_key(&page.navigation.session_id));
        assert!(
            coordinator
                .owner_cleanup
                .lock()
                .unwrap()
                .get(&42)
                .is_some_and(|links| links.contains_key(&link_id))
        );
        assert!(matches!(
            coordinator.cleanup.status(&link_id).await,
            Some(LinkCleanupStatus::TerminalError { attempts: 3, .. })
        ));

        coordinator.cleanup_owner(42).await.expect("retained owner cleanup recovers");

        assert!(coordinator.owner_cleanup.lock().unwrap().get(&42).is_none_or(HashMap::is_empty));
        assert_eq!(
            coordinator.cleanup.status(&link_id).await,
            Some(LinkCleanupStatus::Completed { attempts: 1 })
        );
    }

    #[tokio::test]
    async fn reused_browser_link_is_never_closed() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b">reused");
        scripted.link_created = false;
        let backend = Arc::new(scripted);
        let coordinator = coordinator(backend.clone());

        coordinator.browse_remote(HOST, "/page/reused.mu", Duration::from_secs(1)).await;

        assert!(!backend.calls.lock().unwrap().contains(&"close"));
    }

    #[tokio::test]
    async fn committed_navigation_supervises_transient_old_link_cleanup() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">first"));
        let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
        backend.outcome.lock().unwrap().push_back(duplicate);
        let coordinator = coordinator(backend.clone());
        let mut first = PageNavigationRequest::default();
        first.target = Some(format!("{HOST}:/page/first.mu"));
        let first = coordinator.navigate(first, HOST, |_| Vec::new()).await.unwrap();
        backend.close_failures.store(1, Ordering::Relaxed);
        let mut second = PageNavigationRequest::default();
        second.session_id = Some(first.navigation.session_id.clone());
        second.target = Some(format!("{HOST}:/page/second.mu"));

        let second = coordinator.navigate(second, HOST, |_| Vec::new()).await.unwrap();

        assert_eq!(second.navigation.address, format!("{HOST}:/page/second.mu"));
        let cleaned_link = format!("{:032x}", 1);
        assert_eq!(
            coordinator.cleanup.status(&cleaned_link).await,
            Some(LinkCleanupStatus::Completed { attempts: 2 })
        );
        {
            let sessions = coordinator.sessions.lock().unwrap();
            assert_eq!(
                sessions[&first.navigation.session_id].current.as_ref().unwrap().navigation.address,
                second.navigation.address
            );
        }
    }

    #[tokio::test]
    async fn failed_eviction_close_rolls_back_session_ownership() {
        let backend = Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">new"));
        let coordinator = coordinator(backend.clone());
        {
            let mut sessions = coordinator.sessions.lock().unwrap();
            for index in 0..MAX_SESSIONS {
                let mut session = BrowseSession::new(0, index as u64);
                session.terminal = true;
                session.link = Some(BrowserLink { id: format!("{index:032x}"), created: true });
                sessions.insert(format!("old-{index}"), session);
            }
        }
        backend.close_failures.store(MAX_LINK_CLEANUP_ATTEMPTS.into(), Ordering::Relaxed);
        let mut request = PageNavigationRequest::default();
        request.session_id = Some("replacement".into());
        request.target = Some(format!("{HOST}:/page/new.mu"));

        assert!(coordinator.navigate(request, HOST, |_| Vec::new()).await.is_err());

        let sessions = coordinator.sessions.lock().unwrap();
        assert_eq!(sessions.len(), MAX_SESSIONS);
        assert!(!sessions.contains_key("replacement"));
        assert_eq!(
            sessions.values().filter(|session| session.link.is_some()).count(),
            MAX_SESSIONS
        );
    }

    #[tokio::test]
    async fn submitted_response_is_not_reused_as_ordinary_url_cache() {
        let backend =
            Arc::new(ScriptedBackend::success(RequestResponseTransfer::Packet, b">personalized"));
        let duplicate = backend.outcome.lock().unwrap().front().cloned().unwrap();
        backend.outcome.lock().unwrap().push_back(duplicate);
        let coordinator = coordinator(backend.clone());
        let mut submitted = PageNavigationRequest::default();
        submitted.target = Some(format!("{HOST}:/page/dynamic.mu"));
        submitted.submission = Some(PageFormSubmission::default());
        coordinator.navigate(submitted, HOST, |_| Vec::new()).await.unwrap();
        let mut ordinary = PageNavigationRequest::default();
        ordinary.target = Some(format!("{HOST}:/page/dynamic.mu"));

        let ordinary = coordinator.navigate(ordinary, HOST, |_| Vec::new()).await.unwrap();

        assert_eq!(ordinary.cache.status, PageCacheStatus::Miss);
        assert_eq!(backend.requested_data.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn owner_scope_blocks_known_session_and_download_ids() {
        let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        ))));
        let mut request = PageNavigationRequest::default();
        request.target = Some("/page/local.mu".into());
        let page =
            coordinator.navigate_for_owner(7, request, HOST, |_| b">local".to_vec()).await.unwrap();
        assert!(coordinator.close_session_for_owner(8, &page.navigation.session_id).await.is_err());
        let mut download = FileDownloadRequest::default();
        download.session_id = Some(page.navigation.session_id.clone());
        download.target = "/file/data.bin".into();
        assert!(coordinator.start_download_for_owner(8, download).await.is_err());
    }

    #[tokio::test]
    async fn saving_reservation_blocks_duplicate_save_and_eviction() {
        let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        ))));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *coordinator.save_gate.lock().await = Some(Arc::clone(&gate));
        let mut info = FileDownloadInfo::default();
        info.download_id = "saving".into();
        info.state = FileDownloadState::Completed;
        info.integrity_verified = true;
        coordinator.downloads.lock().await.insert(
            info.download_id.clone(),
            DownloadRecord {
                owner: 0,
                info,
                bytes: Some(b"atomic bytes".to_vec()),
                saving: false,
                cancellation: tokio_util::sync::CancellationToken::new(),
                completion: tokio::sync::watch::channel::<u64>(0).0,
                last_used: 0,
            },
        );
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("saved.bin");
        let duplicate = root.path().join("duplicate.bin");
        let saving = {
            let coordinator = Arc::clone(&coordinator);
            let destination = destination.clone();
            tokio::spawn(async move { coordinator.save_download("saving", &destination).await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if coordinator.downloads.lock().await["saving"].saving {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("save reservation");
        assert!(coordinator.save_download("saving", &duplicate).await.is_err());
        {
            let mut downloads = coordinator.downloads.lock().await;
            for index in 1..MAX_DOWNLOADS {
                let mut info = FileDownloadInfo::default();
                info.download_id = format!("active-{index}");
                downloads.insert(
                    info.download_id.clone(),
                    DownloadRecord {
                        owner: 0,
                        info,
                        bytes: None,
                        saving: false,
                        cancellation: tokio_util::sync::CancellationToken::new(),
                        completion: tokio::sync::watch::channel::<u64>(0).0,
                        last_used: index as u64,
                    },
                );
            }
        }
        let mut request = FileDownloadRequest::default();
        request.target = format!("{HOST}:/file/full.bin");
        assert_eq!(
            coordinator.start_download(request).await.unwrap_err(),
            "download capacity is full"
        );
        gate.add_permits(1);
        let saved = saving.await.unwrap().unwrap();
        assert_eq!(saved.state, FileDownloadState::Saved);
        assert_eq!(std::fs::read(destination).unwrap(), b"atomic bytes");
        assert!(!duplicate.exists());
    }

    #[tokio::test]
    async fn dropped_save_waiter_does_not_abandon_handoff_or_reservation() {
        let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        ))));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *coordinator.save_gate.lock().await = Some(Arc::clone(&gate));
        let mut info = FileDownloadInfo::default();
        info.download_id = "detached-save".into();
        info.state = FileDownloadState::Completed;
        info.integrity_verified = true;
        coordinator.downloads.lock().await.insert(
            info.download_id.clone(),
            DownloadRecord {
                owner: 0,
                info,
                bytes: Some(b"detached bytes".to_vec()),
                saving: false,
                cancellation: tokio_util::sync::CancellationToken::new(),
                completion: tokio::sync::watch::channel::<u64>(0).0,
                last_used: 0,
            },
        );
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("detached.bin");
        let waiter = {
            let coordinator = Arc::clone(&coordinator);
            let destination = destination.clone();
            tokio::spawn(
                async move { coordinator.save_download("detached-save", &destination).await },
            )
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while !coordinator.downloads.lock().await["detached-save"].saving {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("save reservation");
        waiter.abort();
        gate.add_permits(1);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if coordinator.downloads.lock().await["detached-save"].info.state
                    == FileDownloadState::Saved
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached save completion");
        assert_eq!(std::fs::read(destination).unwrap(), b"detached bytes");
    }

    #[tokio::test]
    async fn cancellation_interrupts_every_download_setup_stage() {
        for stage in ["path", "identity", "link"] {
            let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b"late");
            match stage {
                "path" => scripted.path_delay = Duration::from_secs(60),
                "identity" => scripted.identity_delay = Duration::from_secs(60),
                "link" => scripted.link_delay = Duration::from_secs(60),
                _ => unreachable!(),
            }
            let backend = Arc::new(scripted);
            let coordinator = Arc::new(coordinator(backend.clone()));
            let mut request = FileDownloadRequest::default();
            request.target = format!("{HOST}:/file/setup.bin");
            let started = coordinator.start_download(request).await.unwrap();
            for _ in 0..100 {
                if backend.calls.lock().unwrap().contains(&stage) {
                    break;
                }
                tokio::task::yield_now().await;
            }
            let cancelled = tokio::time::timeout(
                Duration::from_millis(100),
                coordinator.cancel_download(&started.download_id),
            )
            .await
            .expect("setup cancellation did not return promptly")
            .expect("download disappeared");
            assert_eq!(cancelled.state, FileDownloadState::Cancelled, "stage {stage}");
        }
    }

    #[tokio::test]
    async fn download_completion_before_wait_registration_is_observed() {
        let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        ))));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *coordinator.cancel_wait_gate.lock().await = Some(Arc::clone(&gate));
        let cancellation = tokio_util::sync::CancellationToken::new();
        let observed_cancellation = cancellation.clone();
        let (completion, _) = tokio::sync::watch::channel(0_u64);
        let mut info = FileDownloadInfo::default();
        info.download_id = "interleaving".into();
        coordinator.downloads.lock().await.insert(
            info.download_id.clone(),
            DownloadRecord {
                owner: 0,
                info,
                bytes: None,
                saving: false,
                cancellation,
                completion,
                last_used: 0,
            },
        );
        let cancellation = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.cancel_download("interleaving").await })
        };
        tokio::time::timeout(Duration::from_secs(1), observed_cancellation.cancelled())
            .await
            .expect("cancellation reached the interleaving point");
        {
            let mut downloads = coordinator.downloads.lock().await;
            let record = downloads.get_mut("interleaving").unwrap();
            record.info.state = FileDownloadState::Cancelled;
            record.completion.send_modify(|version| *version += 1);
        }
        gate.add_permits(1);

        let cancelled = tokio::time::timeout(Duration::from_millis(100), cancellation)
            .await
            .expect("retained completion notification prevents a lost wakeup")
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.state, FileDownloadState::Cancelled);
    }

    #[tokio::test]
    async fn download_cancellation_wait_has_a_hard_terminal_bound() {
        let coordinator = Arc::new(coordinator(Arc::new(ScriptedBackend::success(
            RequestResponseTransfer::Packet,
            b"unused",
        ))));
        let cancellation = tokio_util::sync::CancellationToken::new();
        let observed_cancellation = cancellation.clone();
        let (completion, _) = tokio::sync::watch::channel(0_u64);
        let mut info = FileDownloadInfo::default();
        info.download_id = "bounded-cancel".into();
        coordinator.downloads.lock().await.insert(
            info.download_id.clone(),
            DownloadRecord {
                owner: 0,
                info,
                bytes: None,
                saving: false,
                cancellation,
                completion,
                last_used: 0,
            },
        );
        let waiter = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.cancel_download("bounded-cancel").await })
        };
        observed_cancellation.cancelled().await;

        let cancelled = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("bounded cancellation waiter")
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.state, FileDownloadState::Cancelled);
        assert_eq!(cancelled.error.as_deref(), Some("download cancellation completion timed out"));
    }

    #[tokio::test]
    async fn download_capacity_never_evicts_active_work_and_uses_terminal_lru() {
        let mut scripted = ScriptedBackend::success(RequestResponseTransfer::Packet, b"late");
        scripted.path_delay = Duration::from_secs(60);
        let coordinator = Arc::new(coordinator(Arc::new(scripted)));
        {
            let mut downloads = coordinator.downloads.lock().await;
            for index in 0..MAX_DOWNLOADS {
                let mut info = FileDownloadInfo::default();
                info.download_id = format!("active-{index}");
                downloads.insert(
                    info.download_id.clone(),
                    DownloadRecord {
                        owner: 0,
                        info,
                        bytes: None,
                        saving: false,
                        cancellation: tokio_util::sync::CancellationToken::new(),
                        completion: tokio::sync::watch::channel::<u64>(0).0,
                        last_used: index as u64,
                    },
                );
            }
        }
        let mut request = FileDownloadRequest::default();
        request.target = format!("{HOST}:/file/full.bin");
        assert_eq!(
            coordinator.start_download(request.clone()).await.unwrap_err(),
            "download capacity is full"
        );
        {
            let mut downloads = coordinator.downloads.lock().await;
            downloads.get_mut("active-0").unwrap().info.state = FileDownloadState::Completed;
        }
        let started = coordinator.start_download(request).await.expect("terminal LRU eviction");
        assert!(!coordinator.downloads.lock().await.contains_key("active-0"));
        let cancelled = coordinator.cancel_download(&started.download_id).await.unwrap();
        assert_eq!(cancelled.state, FileDownloadState::Cancelled);
    }
}
