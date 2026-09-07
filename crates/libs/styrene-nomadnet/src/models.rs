use crate::{PageFormField, PageFormSubmission, PageLinkTarget, PageParserWarning};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObservationSource {
    RuntimeInterfaceRegistry,
    TransportPathTable,
    TransportLinkState,
    TransportRequestState,
    TransportResourceState,
    OperationCoordinator,
    Fixture,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservationMetadata {
    pub source: ObservationSource,
    pub observed_at: Option<i64>,
    pub connection_generation: Option<u64>,
    /// Generation of the local IPC socket carrying this observation.
    pub ipc_connection_generation: Option<u64>,
    /// Generation of the individual interface, when the observation is interface-scoped.
    pub interface_generation: Option<u64>,
    pub age_secs: Option<u64>,
    pub freshness_threshold_secs: Option<u64>,
    pub stale: bool,
    pub correlation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Receiving,
    Succeeded,
    LinkClosed,
    TimedOut,
    MalformedResponse,
    Cancelled,
    ResponseTooLarge,
    ResourceFailed,
    TransportFailed,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestResponseTransfer {
    Packet,
    Resource,
    #[default]
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestProtocolError {
    LinkClosed,
    Timeout,
    MalformedResponse,
    Cancelled,
    ResponseTooLarge,
    ResourceFailed,
    TransportFailed,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestObservationInfo {
    pub request_id: String,
    pub path_hash: String,
    pub link_id: String,
    pub started_monotonic_ms: u64,
    pub deadline_monotonic_ms: u64,
    pub request_size: u64,
    pub response_size: Option<u64>,
    pub response_transfer_size: Option<u64>,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub response_transfer: RequestResponseTransfer,
    pub response: Option<Vec<u8>>,
    pub state: RequestState,
    pub protocol_error: Option<RequestProtocolError>,
    pub completed_monotonic_ms: Option<u64>,
    pub rtt_ms: Option<u64>,
    pub request_resource_hash: Option<String>,
    pub resource_hash: Option<String>,
    pub observation: ObservationMetadata,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageContent {
    /// Page title.
    pub title: Option<String>,
    /// Hosting node destination hash.
    pub host_hash: String,
    /// Fetch timestamp.
    pub fetched_at: i64,
    /// Links found in the page (path targets).
    pub links: Vec<String>,
    /// Stable correlation shared by every stage and the native request receipt.
    pub correlation_id: String,
    /// Sticky daemon-owned terminal outcome for this browse operation.
    pub outcome: PageBrowseOutcome,
    pub failure: Option<PageBrowseFailure>,
    pub started_unix_ms: Option<i64>,
    pub completed_unix_ms: Option<i64>,
    pub elapsed_ms: Option<u64>,
    pub observation: ObservationMetadata,
    /// Authoritative ordered lifecycle reported by the daemon coordinator.
    pub stages: Vec<PageBrowseStage>,
    /// Canonical response bytes, retained independently of UTF-8 rendering.
    pub source_bytes: Vec<u8>,
    /// Daemon-produced rendering projection. Clients must not infer stage success by parsing source.
    pub rendered_text: String,
    pub parser_warnings: Vec<PageParserWarning>,
    /// Lowercase SHA-256 of `source_bytes`.
    pub source_checksum: String,
    pub request: PageRequestMetadata,
    pub transfer: PageTransferInfo,
    pub cache: PageCacheInfo,
    /// Authoritative daemon-owned navigation state.
    pub navigation: PageNavigationInfo,
    /// Interactive fields parsed from the canonical source. Password values are omitted.
    pub fields: Vec<PageFormField>,
    /// Links and the field names each link submits.
    pub link_targets: Vec<PageLinkTarget>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageNavigationAction {
    Unsupported,
    #[default]
    Navigate,
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageNavigationRequest {
    pub session_id: Option<String>,
    pub action: PageNavigationAction,
    /// Address or relative link for `Navigate`; ignored by history and reload actions.
    pub target: Option<String>,
    pub bypass_cache: bool,
    pub timeout_secs: Option<u64>,
    pub submission: Option<PageFormSubmission>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageNavigationInfo {
    pub session_id: String,
    pub address: String,
    pub history_index: u32,
    pub history_len: u32,
    pub can_back: bool,
    pub can_forward: bool,
    pub connection_open: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageBrowseStageKind {
    #[default]
    PathDiscovery,
    IdentityResolution,
    LinkEstablishment,
    Identification,
    RequestSubmission,
    Transfer,
    Parse,
    Render,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PageBrowseStageState {
    #[default]
    Pending,
    Succeeded,
    Failed {
        code: String,
        message: String,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageBrowseOutcome {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageBrowseFailure {
    pub stage: PageBrowseStageKind,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageBrowseStage {
    pub correlation_id: String,
    pub kind: PageBrowseStageKind,
    pub state: PageBrowseStageState,
    pub observation: ObservationMetadata,
    pub evidence_source: Option<ObservationSource>,
    pub destination_hash: Option<String>,
    pub link_id: Option<String>,
    pub request_id: Option<String>,
    pub resource_hash: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageRequestMetadata {
    pub native_path: String,
    pub path_hash: String,
    pub request_id: Option<String>,
    pub link_id: Option<String>,
    pub request_size: u64,
    pub response_size: Option<u64>,
    pub rtt_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageTransferKind {
    #[default]
    None,
    Local,
    Packet,
    Resource,
    Cache,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageTransferInfo {
    pub kind: PageTransferKind,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub resource_hash: Option<String>,
    pub verified: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageCacheStatus {
    #[default]
    NotUsed,
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageCacheInfo {
    pub status: PageCacheStatus,
    pub stored_at: Option<i64>,
    /// Original successful browse whose immutable content populated this entry.
    pub origin_correlation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileDownloadState {
    #[default]
    Pending,
    Receiving,
    Completed,
    Cancelled,
    Failed,
    Saved,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDownloadRequest {
    pub session_id: Option<String>,
    pub target: String,
    pub expected_sha256: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDownloadInfo {
    pub download_id: String,
    pub correlation_id: String,
    pub host_hash: String,
    pub native_path: String,
    pub state: FileDownloadState,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub transfer: PageTransferKind,
    pub resource_hash: Option<String>,
    pub sha256: Option<String>,
    pub integrity_verified: bool,
    pub error: Option<String>,
    pub saved_path: Option<String>,
}
impl RequestState {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Receiving | Self::Unknown)
    }
}
impl FileDownloadState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed | Self::Saved)
    }
}
