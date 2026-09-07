//! Browser sessions, cache, download and link-reference ownership.
#![allow(clippy::field_reassign_with_default)]

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::PageAddress;
use crate::PageParserWarning;
use crate::models::*;
use crate::{decode_binary_response, decode_file_response};
use crate::{encode_submission, render_projection};
use async_trait::async_trait;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::identity::Identity;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const MAX_PAGE_SOURCE_SIZE: usize = 1024 * 1024;
const MAX_ENCODED_NATIVE_RESPONSE_SIZE: u64 = MAX_PAGE_SOURCE_SIZE as u64 + 6;
const MAX_FILE_SIZE: usize = 32 * 1024 * 1024;
const MAX_ENCODED_FILE_RESPONSE_SIZE: u64 = MAX_FILE_SIZE as u64 + 6;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 32;
const MAX_HISTORY_ENTRIES: usize = 64;
const MAX_SESSIONS: usize = 16;
const MAX_DOWNLOADS: usize = 8;
const MAX_LINK_CLEANUP_ATTEMPTS: u8 = 3;
const MAX_LINK_CLEANUP_RECORDS: usize = 64;
#[cfg(not(test))]
const DOWNLOAD_CANCELLATION_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const DOWNLOAD_CANCELLATION_WAIT: Duration = Duration::from_millis(50);
static NEXT_CORRELATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, thiserror::Error)]
pub enum BrowseError {
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
pub struct BrowserLink {
    pub id: String,
    pub created: bool,
}

#[derive(Clone)]
pub struct NativeRequestOutcome {
    pub started: RequestObservationInfo,
    pub completed: RequestObservationInfo,
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

pub struct NativeRequest {
    pub link_id: String,
    pub path: String,
    pub correlation_id: String,
    pub data: Vec<u8>,
    pub max_response_size: u64,
    pub cancellation: tokio_util::sync::CancellationToken,
    pub progress: Option<Arc<dyn Fn(RequestObservationInfo) + Send + Sync>>,
    pub deadline: tokio::time::Instant,
}

#[async_trait]
pub trait BrowseBackend: Send + Sync {
    fn identification_enabled(&self) -> bool;
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
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(), BrowseError>;
    async fn request(&self, request: NativeRequest) -> Result<NativeRequestOutcome, BrowseError>;
    async fn close_link(&self, link_id: &str) -> Result<(), BrowseError>;
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
    discovery: Arc<dyn Discovery>,
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
    pub fn new(backend: Arc<dyn BrowseBackend>, discovery: Arc<dyn Discovery>) -> Self {
        Self {
            cleanup: Arc::new(LinkCleanupSupervisor::new(Arc::clone(&backend))),
            backend,
            discovery,
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
    fn with_backend(backend: Arc<dyn BrowseBackend>, discovery: Arc<dyn Discovery>) -> Self {
        Self {
            cleanup: Arc::new(LinkCleanupSupervisor::new(Arc::clone(&backend))),
            backend,
            discovery,
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
        let Some(native_host) = self.discovery.native_host(host) else {
            fail(&mut result, 0, "capability_unknown", "host has no native NomadNet announce");
            return FetchedPage { page: result, link: None };
        };
        if !native_host {
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
        if self.backend.identification_enabled() {
            if let Err(error) =
                self.backend.identify_link(&link.link().id, &cancellation, deadline).await
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

        // Link variables can carry request data even without an explicit form submission.
        let cacheable = request.submission.is_none() && data == [0xc0];
        let bypass = request.bypass_cache || request.action == PageNavigationAction::Reload;
        let cache_key = address.to_string();
        let mut page = if !bypass && cacheable { self.cached(&cache_key) } else { None }
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
            if page.outcome == PageBrowseOutcome::Succeeded && cacheable {
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
        let Some(native_host) = self.discovery.native_host(host) else {
            return Err(BrowseError::Transport("host has no native NomadNet announce".into()));
        };
        if !native_host {
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
            if self.backend.identification_enabled() {
                self.backend.identify_link(&link.link().id, &cancellation, deadline).await?;
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
                request.completed.response.as_deref().and_then(decode_file_response).ok_or_else(
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
            crate::NomadNetHost::parse(host).map_err(|error| error.to_string())?.to_string(),
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
    if !matches!(crate::NomadNetPath::parse(&path), Ok(crate::NomadNetPath::File(_))) {
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

fn correlation_id() -> String {
    let sequence = NEXT_CORRELATION.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("page-{timestamp:032x}-{sequence:016x}")
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

/// A lookup of known native host capability; None means unannounced.
pub trait Discovery: Send + Sync {
    fn native_host(&self, host: &str) -> Option<bool>;
}
impl<F: Fn(&str) -> Option<bool> + Send + Sync> Discovery for F {
    fn native_host(&self, host: &str) -> Option<bool> {
        self(host)
    }
}

fn remaining(deadline: tokio::time::Instant) -> Result<Duration, BrowseError> {
    deadline.checked_duration_since(tokio::time::Instant::now()).ok_or(BrowseError::Deadline)
}
