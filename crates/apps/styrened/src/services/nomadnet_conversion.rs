//! Explicit field conversion between domain models and stable IPC DTOs.
use std::collections::BTreeMap;
use styrene_ipc::types as ipc;
use styrene_nomadnet::{self as content, models as d};
pub(super) trait IntoDomain {
    type Domain;
    fn into_domain(self) -> Self::Domain;
}
pub(super) trait IntoIpc {
    type Ipc;
    fn into_ipc(self) -> Self::Ipc;
}
impl IntoDomain for String {
    type Domain = String;
    fn into_domain(self) -> String {
        self
    }
}
impl IntoDomain for bool {
    type Domain = bool;
    fn into_domain(self) -> bool {
        self
    }
}
impl IntoDomain for u8 {
    type Domain = u8;
    fn into_domain(self) -> u8 {
        self
    }
}
impl IntoDomain for u32 {
    type Domain = u32;
    fn into_domain(self) -> u32 {
        self
    }
}
impl IntoDomain for u64 {
    type Domain = u64;
    fn into_domain(self) -> u64 {
        self
    }
}
impl IntoDomain for i64 {
    type Domain = i64;
    fn into_domain(self) -> i64 {
        self
    }
}
impl IntoDomain for f32 {
    type Domain = f32;
    fn into_domain(self) -> f32 {
        self
    }
}
impl<T: IntoDomain> IntoDomain for Option<T> {
    type Domain = Option<T::Domain>;
    fn into_domain(self) -> Self::Domain {
        self.map(|v| v.into_domain())
    }
}
impl<T: IntoDomain> IntoDomain for Vec<T> {
    type Domain = Vec<T::Domain>;
    fn into_domain(self) -> Self::Domain {
        self.into_iter().map(|v| v.into_domain()).collect()
    }
}
impl<T: IntoDomain> IntoDomain for BTreeMap<String, T> {
    type Domain = BTreeMap<String, T::Domain>;
    fn into_domain(self) -> Self::Domain {
        self.into_iter().map(|(k, v)| (k, v.into_domain())).collect()
    }
}
impl IntoIpc for String {
    type Ipc = String;
    fn into_ipc(self) -> String {
        self
    }
}
impl IntoIpc for bool {
    type Ipc = bool;
    fn into_ipc(self) -> bool {
        self
    }
}
impl IntoIpc for u8 {
    type Ipc = u8;
    fn into_ipc(self) -> u8 {
        self
    }
}
impl IntoIpc for u32 {
    type Ipc = u32;
    fn into_ipc(self) -> u32 {
        self
    }
}
impl IntoIpc for u64 {
    type Ipc = u64;
    fn into_ipc(self) -> u64 {
        self
    }
}
impl IntoIpc for i64 {
    type Ipc = i64;
    fn into_ipc(self) -> i64 {
        self
    }
}
impl IntoIpc for f32 {
    type Ipc = f32;
    fn into_ipc(self) -> f32 {
        self
    }
}
impl<T: IntoIpc> IntoIpc for Option<T> {
    type Ipc = Option<T::Ipc>;
    fn into_ipc(self) -> Self::Ipc {
        self.map(|v| v.into_ipc())
    }
}
impl<T: IntoIpc> IntoIpc for Vec<T> {
    type Ipc = Vec<T::Ipc>;
    fn into_ipc(self) -> Self::Ipc {
        self.into_iter().map(|v| v.into_ipc()).collect()
    }
}
impl<T: IntoIpc> IntoIpc for BTreeMap<String, T> {
    type Ipc = BTreeMap<String, T::Ipc>;
    fn into_ipc(self) -> Self::Ipc {
        self.into_iter().map(|(k, v)| (k, v.into_ipc())).collect()
    }
}
impl IntoDomain for ipc::PageNavigationAction {
    type Domain = d::PageNavigationAction;
    fn into_domain(self) -> d::PageNavigationAction {
        match self {
            ipc::PageNavigationAction::Navigate => d::PageNavigationAction::Navigate,
            ipc::PageNavigationAction::Back => d::PageNavigationAction::Back,
            ipc::PageNavigationAction::Forward => d::PageNavigationAction::Forward,
            ipc::PageNavigationAction::Reload => d::PageNavigationAction::Reload,
            _ => d::PageNavigationAction::Unsupported,
        }
    }
}
impl IntoDomain for ipc::PageFormSubmission {
    type Domain = content::PageFormSubmission;
    fn into_domain(self) -> content::PageFormSubmission {
        content::PageFormSubmission { values: self.values.into_domain() }
    }
}
impl IntoDomain for ipc::PageNavigationRequest {
    type Domain = d::PageNavigationRequest;
    fn into_domain(self) -> d::PageNavigationRequest {
        d::PageNavigationRequest {
            session_id: self.session_id.into_domain(),
            action: self.action.into_domain(),
            target: self.target.into_domain(),
            bypass_cache: self.bypass_cache.into_domain(),
            timeout_secs: self.timeout_secs.into_domain(),
            submission: self.submission.into_domain(),
        }
    }
}
impl IntoDomain for ipc::FileDownloadRequest {
    type Domain = d::FileDownloadRequest;
    fn into_domain(self) -> d::FileDownloadRequest {
        d::FileDownloadRequest {
            session_id: self.session_id.into_domain(),
            target: self.target.into_domain(),
            expected_sha256: self.expected_sha256.into_domain(),
            timeout_secs: self.timeout_secs.into_domain(),
        }
    }
}
impl IntoDomain for ipc::RequestResponseTransfer {
    type Domain = d::RequestResponseTransfer;
    fn into_domain(self) -> d::RequestResponseTransfer {
        match self {
            ipc::RequestResponseTransfer::Packet => d::RequestResponseTransfer::Packet,
            ipc::RequestResponseTransfer::Resource => d::RequestResponseTransfer::Resource,
            ipc::RequestResponseTransfer::None => d::RequestResponseTransfer::None,
            _ => d::RequestResponseTransfer::None,
        }
    }
}
impl IntoDomain for ipc::RequestState {
    type Domain = d::RequestState;
    fn into_domain(self) -> d::RequestState {
        match self {
            ipc::RequestState::Pending => d::RequestState::Pending,
            ipc::RequestState::Receiving => d::RequestState::Receiving,
            ipc::RequestState::Succeeded => d::RequestState::Succeeded,
            ipc::RequestState::LinkClosed => d::RequestState::LinkClosed,
            ipc::RequestState::TimedOut => d::RequestState::TimedOut,
            ipc::RequestState::MalformedResponse => d::RequestState::MalformedResponse,
            ipc::RequestState::Cancelled => d::RequestState::Cancelled,
            ipc::RequestState::ResponseTooLarge => d::RequestState::ResponseTooLarge,
            ipc::RequestState::ResourceFailed => d::RequestState::ResourceFailed,
            ipc::RequestState::TransportFailed => d::RequestState::TransportFailed,
            ipc::RequestState::Unknown => d::RequestState::Unknown,
            _ => d::RequestState::Unknown,
        }
    }
}
impl IntoDomain for ipc::RequestProtocolError {
    type Domain = d::RequestProtocolError;
    fn into_domain(self) -> d::RequestProtocolError {
        match self {
            ipc::RequestProtocolError::LinkClosed => d::RequestProtocolError::LinkClosed,
            ipc::RequestProtocolError::Timeout => d::RequestProtocolError::Timeout,
            ipc::RequestProtocolError::MalformedResponse => {
                d::RequestProtocolError::MalformedResponse
            }
            ipc::RequestProtocolError::Cancelled => d::RequestProtocolError::Cancelled,
            ipc::RequestProtocolError::ResponseTooLarge => {
                d::RequestProtocolError::ResponseTooLarge
            }
            ipc::RequestProtocolError::ResourceFailed => d::RequestProtocolError::ResourceFailed,
            ipc::RequestProtocolError::TransportFailed => d::RequestProtocolError::TransportFailed,
            ipc::RequestProtocolError::Unknown => d::RequestProtocolError::Unknown,
            _ => d::RequestProtocolError::Unknown,
        }
    }
}
impl IntoDomain for ipc::ObservationSource {
    type Domain = d::ObservationSource;
    fn into_domain(self) -> d::ObservationSource {
        match self {
            ipc::ObservationSource::RuntimeInterfaceRegistry => {
                d::ObservationSource::RuntimeInterfaceRegistry
            }
            ipc::ObservationSource::TransportPathTable => d::ObservationSource::TransportPathTable,
            ipc::ObservationSource::TransportLinkState => d::ObservationSource::TransportLinkState,
            ipc::ObservationSource::TransportRequestState => {
                d::ObservationSource::TransportRequestState
            }
            ipc::ObservationSource::TransportResourceState => {
                d::ObservationSource::TransportResourceState
            }
            ipc::ObservationSource::OperationCoordinator => {
                d::ObservationSource::OperationCoordinator
            }
            ipc::ObservationSource::Fixture => d::ObservationSource::Fixture,
            ipc::ObservationSource::Unknown => d::ObservationSource::Unknown,
            _ => d::ObservationSource::Unknown,
        }
    }
}
impl IntoDomain for ipc::ObservationMetadata {
    type Domain = d::ObservationMetadata;
    fn into_domain(self) -> d::ObservationMetadata {
        d::ObservationMetadata {
            source: self.source.into_domain(),
            observed_at: self.observed_at.into_domain(),
            connection_generation: self.connection_generation.into_domain(),
            ipc_connection_generation: self.ipc_connection_generation.into_domain(),
            interface_generation: self.interface_generation.into_domain(),
            age_secs: self.age_secs.into_domain(),
            freshness_threshold_secs: self.freshness_threshold_secs.into_domain(),
            stale: self.stale.into_domain(),
            correlation_id: self.correlation_id.into_domain(),
        }
    }
}
impl IntoDomain for ipc::RequestObservationInfo {
    type Domain = d::RequestObservationInfo;
    fn into_domain(self) -> d::RequestObservationInfo {
        d::RequestObservationInfo {
            request_id: self.request_id.into_domain(),
            path_hash: self.path_hash.into_domain(),
            link_id: self.link_id.into_domain(),
            started_monotonic_ms: self.started_monotonic_ms.into_domain(),
            deadline_monotonic_ms: self.deadline_monotonic_ms.into_domain(),
            request_size: self.request_size.into_domain(),
            response_size: self.response_size.into_domain(),
            response_transfer_size: self.response_transfer_size.into_domain(),
            received_bytes: self.received_bytes.into_domain(),
            total_bytes: self.total_bytes.into_domain(),
            progress: self.progress.into_domain(),
            response_transfer: self.response_transfer.into_domain(),
            response: self.response.into_domain(),
            state: self.state.into_domain(),
            protocol_error: self.protocol_error.into_domain(),
            completed_monotonic_ms: self.completed_monotonic_ms.into_domain(),
            rtt_ms: self.rtt_ms.into_domain(),
            request_resource_hash: self.request_resource_hash.into_domain(),
            resource_hash: self.resource_hash.into_domain(),
            observation: self.observation.into_domain(),
        }
    }
}
impl IntoIpc for d::PageBrowseOutcome {
    type Ipc = ipc::PageBrowseOutcome;
    fn into_ipc(self) -> ipc::PageBrowseOutcome {
        match self {
            d::PageBrowseOutcome::Running => ipc::PageBrowseOutcome::Running,
            d::PageBrowseOutcome::Succeeded => ipc::PageBrowseOutcome::Succeeded,
            d::PageBrowseOutcome::Failed => ipc::PageBrowseOutcome::Failed,
            d::PageBrowseOutcome::TimedOut => ipc::PageBrowseOutcome::TimedOut,
            d::PageBrowseOutcome::Cancelled => ipc::PageBrowseOutcome::Cancelled,
            d::PageBrowseOutcome::Unknown => ipc::PageBrowseOutcome::Unknown,
        }
    }
}
impl IntoIpc for d::PageBrowseStageKind {
    type Ipc = ipc::PageBrowseStageKind;
    fn into_ipc(self) -> ipc::PageBrowseStageKind {
        match self {
            d::PageBrowseStageKind::PathDiscovery => ipc::PageBrowseStageKind::PathDiscovery,
            d::PageBrowseStageKind::IdentityResolution => {
                ipc::PageBrowseStageKind::IdentityResolution
            }
            d::PageBrowseStageKind::LinkEstablishment => {
                ipc::PageBrowseStageKind::LinkEstablishment
            }
            d::PageBrowseStageKind::Identification => ipc::PageBrowseStageKind::Identification,
            d::PageBrowseStageKind::RequestSubmission => {
                ipc::PageBrowseStageKind::RequestSubmission
            }
            d::PageBrowseStageKind::Transfer => ipc::PageBrowseStageKind::Transfer,
            d::PageBrowseStageKind::Parse => ipc::PageBrowseStageKind::Parse,
            d::PageBrowseStageKind::Render => ipc::PageBrowseStageKind::Render,
        }
    }
}
impl IntoIpc for d::PageBrowseFailure {
    type Ipc = ipc::PageBrowseFailure;
    fn into_ipc(self) -> ipc::PageBrowseFailure {
        let mut value = ipc::PageBrowseFailure::default();
        value.stage = self.stage.into_ipc();
        value.code = self.code.into_ipc();
        value.message = self.message.into_ipc();
        value.retryable = self.retryable.into_ipc();
        value
    }
}
impl IntoIpc for d::ObservationSource {
    type Ipc = ipc::ObservationSource;
    fn into_ipc(self) -> ipc::ObservationSource {
        match self {
            d::ObservationSource::RuntimeInterfaceRegistry => {
                ipc::ObservationSource::RuntimeInterfaceRegistry
            }
            d::ObservationSource::TransportPathTable => ipc::ObservationSource::TransportPathTable,
            d::ObservationSource::TransportLinkState => ipc::ObservationSource::TransportLinkState,
            d::ObservationSource::TransportRequestState => {
                ipc::ObservationSource::TransportRequestState
            }
            d::ObservationSource::TransportResourceState => {
                ipc::ObservationSource::TransportResourceState
            }
            d::ObservationSource::OperationCoordinator => {
                ipc::ObservationSource::OperationCoordinator
            }
            d::ObservationSource::Fixture => ipc::ObservationSource::Fixture,
            d::ObservationSource::Unknown => ipc::ObservationSource::Unknown,
        }
    }
}
impl IntoIpc for d::ObservationMetadata {
    type Ipc = ipc::ObservationMetadata;
    fn into_ipc(self) -> ipc::ObservationMetadata {
        let mut value = ipc::ObservationMetadata::default();
        value.source = self.source.into_ipc();
        value.observed_at = self.observed_at.into_ipc();
        value.connection_generation = self.connection_generation.into_ipc();
        value.ipc_connection_generation = self.ipc_connection_generation.into_ipc();
        value.interface_generation = self.interface_generation.into_ipc();
        value.age_secs = self.age_secs.into_ipc();
        value.freshness_threshold_secs = self.freshness_threshold_secs.into_ipc();
        value.stale = self.stale.into_ipc();
        value.correlation_id = self.correlation_id.into_ipc();
        value
    }
}
impl IntoIpc for d::PageBrowseStageState {
    type Ipc = ipc::PageBrowseStageState;
    fn into_ipc(self) -> ipc::PageBrowseStageState {
        match self {
            d::PageBrowseStageState::Pending => ipc::PageBrowseStageState::Pending,
            d::PageBrowseStageState::Succeeded => ipc::PageBrowseStageState::Succeeded,
            d::PageBrowseStageState::Failed { code, message } => {
                ipc::PageBrowseStageState::Failed {
                    code: code.into_ipc(),
                    message: message.into_ipc(),
                }
            }
            d::PageBrowseStageState::Skipped { reason } => {
                ipc::PageBrowseStageState::Skipped { reason: reason.into_ipc() }
            }
        }
    }
}
impl IntoIpc for d::PageBrowseStage {
    type Ipc = ipc::PageBrowseStage;
    fn into_ipc(self) -> ipc::PageBrowseStage {
        let mut value = ipc::PageBrowseStage::default();
        value.correlation_id = self.correlation_id.into_ipc();
        value.kind = self.kind.into_ipc();
        value.state = self.state.into_ipc();
        value.observation = self.observation.into_ipc();
        value.evidence_source = self.evidence_source.into_ipc();
        value.destination_hash = self.destination_hash.into_ipc();
        value.link_id = self.link_id.into_ipc();
        value.request_id = self.request_id.into_ipc();
        value.resource_hash = self.resource_hash.into_ipc();
        value
    }
}
impl IntoIpc for content::PageParserWarning {
    type Ipc = ipc::PageParserWarning;
    fn into_ipc(self) -> ipc::PageParserWarning {
        let mut value = ipc::PageParserWarning::default();
        value.code = self.code.into_ipc();
        value.message = self.message.into_ipc();
        value
    }
}
impl IntoIpc for d::PageRequestMetadata {
    type Ipc = ipc::PageRequestMetadata;
    fn into_ipc(self) -> ipc::PageRequestMetadata {
        let mut value = ipc::PageRequestMetadata::default();
        value.native_path = self.native_path.into_ipc();
        value.path_hash = self.path_hash.into_ipc();
        value.request_id = self.request_id.into_ipc();
        value.link_id = self.link_id.into_ipc();
        value.request_size = self.request_size.into_ipc();
        value.response_size = self.response_size.into_ipc();
        value.rtt_ms = self.rtt_ms.into_ipc();
        value
    }
}
impl IntoIpc for d::PageTransferKind {
    type Ipc = ipc::PageTransferKind;
    fn into_ipc(self) -> ipc::PageTransferKind {
        match self {
            d::PageTransferKind::None => ipc::PageTransferKind::None,
            d::PageTransferKind::Local => ipc::PageTransferKind::Local,
            d::PageTransferKind::Packet => ipc::PageTransferKind::Packet,
            d::PageTransferKind::Resource => ipc::PageTransferKind::Resource,
            d::PageTransferKind::Cache => ipc::PageTransferKind::Cache,
        }
    }
}
impl IntoIpc for d::PageTransferInfo {
    type Ipc = ipc::PageTransferInfo;
    fn into_ipc(self) -> ipc::PageTransferInfo {
        let mut value = ipc::PageTransferInfo::default();
        value.kind = self.kind.into_ipc();
        value.received_bytes = self.received_bytes.into_ipc();
        value.total_bytes = self.total_bytes.into_ipc();
        value.progress = self.progress.into_ipc();
        value.resource_hash = self.resource_hash.into_ipc();
        value.verified = self.verified.into_ipc();
        value
    }
}
impl IntoIpc for d::PageCacheStatus {
    type Ipc = ipc::PageCacheStatus;
    fn into_ipc(self) -> ipc::PageCacheStatus {
        match self {
            d::PageCacheStatus::NotUsed => ipc::PageCacheStatus::NotUsed,
            d::PageCacheStatus::Hit => ipc::PageCacheStatus::Hit,
            d::PageCacheStatus::Miss => ipc::PageCacheStatus::Miss,
            d::PageCacheStatus::Bypassed => ipc::PageCacheStatus::Bypassed,
        }
    }
}
impl IntoIpc for d::PageCacheInfo {
    type Ipc = ipc::PageCacheInfo;
    fn into_ipc(self) -> ipc::PageCacheInfo {
        let mut value = ipc::PageCacheInfo::default();
        value.status = self.status.into_ipc();
        value.stored_at = self.stored_at.into_ipc();
        value.origin_correlation_id = self.origin_correlation_id.into_ipc();
        value
    }
}
impl IntoIpc for d::PageNavigationInfo {
    type Ipc = ipc::PageNavigationInfo;
    fn into_ipc(self) -> ipc::PageNavigationInfo {
        let mut value = ipc::PageNavigationInfo::default();
        value.session_id = self.session_id.into_ipc();
        value.address = self.address.into_ipc();
        value.history_index = self.history_index.into_ipc();
        value.history_len = self.history_len.into_ipc();
        value.can_back = self.can_back.into_ipc();
        value.can_forward = self.can_forward.into_ipc();
        value.connection_open = self.connection_open.into_ipc();
        value
    }
}
impl IntoIpc for content::PageFormFieldKind {
    type Ipc = ipc::PageFormFieldKind;
    fn into_ipc(self) -> ipc::PageFormFieldKind {
        match self {
            content::PageFormFieldKind::Text => ipc::PageFormFieldKind::Text,
            content::PageFormFieldKind::Password => ipc::PageFormFieldKind::Password,
            content::PageFormFieldKind::Checkbox => ipc::PageFormFieldKind::Checkbox,
            content::PageFormFieldKind::Radio => ipc::PageFormFieldKind::Radio,
            content::PageFormFieldKind::Unknown => ipc::PageFormFieldKind::Text,
        }
    }
}
impl IntoIpc for content::PageFormField {
    type Ipc = ipc::PageFormField;
    fn into_ipc(self) -> ipc::PageFormField {
        let mut value = ipc::PageFormField::default();
        value.name = self.name.into_ipc();
        value.kind = self.kind.into_ipc();
        value.value = self.value.into_ipc();
        value.width = self.width.into_ipc();
        value.checked = self.checked.into_ipc();
        value
    }
}
impl IntoIpc for content::PageLinkTarget {
    type Ipc = ipc::PageLinkTarget;
    fn into_ipc(self) -> ipc::PageLinkTarget {
        let mut value = ipc::PageLinkTarget::default();
        value.label = self.label.into_ipc();
        value.target = self.target.into_ipc();
        value.submitted_fields = self.submitted_fields.into_ipc();
        value
    }
}
impl IntoIpc for d::PageContent {
    type Ipc = ipc::PageContent;
    fn into_ipc(self) -> ipc::PageContent {
        let mut value = ipc::PageContent::default();
        value.title = self.title.into_ipc();
        value.host_hash = self.host_hash.into_ipc();
        value.fetched_at = self.fetched_at.into_ipc();
        value.links = self.links.into_ipc();
        value.correlation_id = self.correlation_id.into_ipc();
        value.outcome = self.outcome.into_ipc();
        value.failure = self.failure.into_ipc();
        value.started_unix_ms = self.started_unix_ms.into_ipc();
        value.completed_unix_ms = self.completed_unix_ms.into_ipc();
        value.elapsed_ms = self.elapsed_ms.into_ipc();
        value.observation = self.observation.into_ipc();
        value.stages = self.stages.into_ipc();
        value.source_bytes = self.source_bytes.into_ipc();
        value.rendered_text = self.rendered_text.into_ipc();
        value.parser_warnings = self.parser_warnings.into_ipc();
        value.source_checksum = self.source_checksum.into_ipc();
        value.request = self.request.into_ipc();
        value.transfer = self.transfer.into_ipc();
        value.cache = self.cache.into_ipc();
        value.navigation = self.navigation.into_ipc();
        value.fields = self.fields.into_ipc();
        value.link_targets = self.link_targets.into_ipc();
        value
    }
}
impl IntoIpc for d::FileDownloadState {
    type Ipc = ipc::FileDownloadState;
    fn into_ipc(self) -> ipc::FileDownloadState {
        match self {
            d::FileDownloadState::Pending => ipc::FileDownloadState::Pending,
            d::FileDownloadState::Receiving => ipc::FileDownloadState::Receiving,
            d::FileDownloadState::Completed => ipc::FileDownloadState::Completed,
            d::FileDownloadState::Cancelled => ipc::FileDownloadState::Cancelled,
            d::FileDownloadState::Failed => ipc::FileDownloadState::Failed,
            d::FileDownloadState::Saved => ipc::FileDownloadState::Saved,
        }
    }
}
impl IntoIpc for d::FileDownloadInfo {
    type Ipc = ipc::FileDownloadInfo;
    fn into_ipc(self) -> ipc::FileDownloadInfo {
        let mut value = ipc::FileDownloadInfo::default();
        value.download_id = self.download_id.into_ipc();
        value.correlation_id = self.correlation_id.into_ipc();
        value.host_hash = self.host_hash.into_ipc();
        value.native_path = self.native_path.into_ipc();
        value.state = self.state.into_ipc();
        value.received_bytes = self.received_bytes.into_ipc();
        value.total_bytes = self.total_bytes.into_ipc();
        value.progress = self.progress.into_ipc();
        value.transfer = self.transfer.into_ipc();
        value.resource_hash = self.resource_hash.into_ipc();
        value.sha256 = self.sha256.into_ipc();
        value.integrity_verified = self.integrity_verified.into_ipc();
        value.error = self.error.into_ipc();
        value.saved_path = self.saved_path.into_ipc();
        value
    }
}
