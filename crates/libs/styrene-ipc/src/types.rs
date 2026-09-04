use serde::{Deserialize, Serialize};

pub const MAX_MESSAGE_QUERY_LIMIT: u32 = 256;
pub const MAX_PAGE_CURSOR_LENGTH: usize = 128;
pub const MAX_CHAT_CONTENT_BYTES: usize = 64 * 1024;
pub const MAX_CHAT_ATTACHMENTS: usize = 8;
pub const MAX_CHAT_ATTACHMENT_NAME_BYTES: usize = 255;
pub const MAX_CHAT_ATTACHMENT_BYTES: usize = 768 * 1024;
pub const MOBILE_DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;
pub const MOBILE_DIAGNOSTIC_MAX_EVENTS: u32 = 4096;
pub const MOBILE_DIAGNOSTIC_MAX_BYTES: u64 = 1024 * 1024;
use std::collections::BTreeMap;
use std::fmt;

// ── Type aliases ──────────────────────────────────────────────────────────────

/// Hex-encoded message identifier.
pub type MessageId = String;

/// Hex-encoded peer/destination hash.
pub type PeerHash = String;

/// Terminal session identifier.
pub type SessionId = String;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MobileDiagnosticSource {
    Runtime,
    Transport,
    Messaging,
    Storage,
    Platform,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MobileDiagnosticStage {
    Boot,
    Lifecycle,
    Inbound,
    Outbound,
    Synchronization,
    Persistence,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MobileDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Payload-free diagnostic event projected from the mobile runtime's private ring.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobileDiagnosticEvent {
    pub sequence: u64,
    pub unix_time_ms: Option<u64>,
    pub source: MobileDiagnosticSource,
    pub stage: MobileDiagnosticStage,
    pub severity: MobileDiagnosticSeverity,
    pub generation: u64,
    /// Runtime-keyed correlation, never a caller-provided identifier or unkeyed digest.
    pub safe_correlation: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobileDiagnosticSnapshot {
    pub schema_version: u32,
    pub backend_revision: String,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub event_count: u32,
    pub retained_bytes: u64,
    pub max_events: u32,
    pub max_bytes: u64,
    pub truncated: bool,
    pub dropped_events: u64,
    pub events: Vec<MobileDiagnosticEvent>,
}

/// Canonical export bytes and the metadata needed by a frontend share adapter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobileDiagnosticExport {
    pub schema_version: u32,
    pub backend_revision: String,
    pub content_type: String,
    pub digest_sha256: String,
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub event_count: u32,
    pub byte_count: u64,
    pub max_events: u32,
    pub max_bytes: u64,
    pub truncated: bool,
    pub dropped_events: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiscoveredCapability {
    /// Evidence came from an announce for the canonical `nomadnetwork.node` destination.
    #[serde(rename = "native_nomadnet_host")]
    NativeNomadNetHost,
    /// Evidence came from a valid standard `lxmf.propagation` announce.
    StandardLxmfPropagationHost,
    /// A capability spelled by a newer daemon that this build does not know.
    #[serde(other)]
    Unknown,
}

impl DiscoveredCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NativeNomadNetHost => "native_nomadnet_host",
            Self::StandardLxmfPropagationHost => "standard_lxmf_propagation_host",
            Self::Unknown => "unknown",
        }
    }
}

// ── Device discovery ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct DeviceInfo {
    pub destination_hash: String,
    pub identity_hash: String,
    pub name: String,
    pub device_type: String,
    pub status: String,
    pub is_styrene_node: bool,
    pub lxmf_destination_hash: String,
    pub last_announce: Option<i64>,
    pub announce_count: u32,
    pub short_name: Option<String>,
    /// Capabilities derived from canonical network announce evidence.
    pub discovered_capabilities: Vec<DiscoveredCapability>,
    /// Advertised standard propagation handler state, when proven by canonical app data.
    pub standard_lxmf_propagation_active: Option<bool>,
    /// Hops the most recent announce travelled, when the reception was observed.
    pub hops: Option<u8>,
    /// Interface kind the most recent announce arrived on, when resolvable.
    pub interface_kind: Option<String>,
}

// ── Identity ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyBackend {
    Keychain,
    AndroidKeystore,
    EncryptedFile,
    PlaintextFile,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyProtection {
    PlatformProtected,
    EncryptedAtRest,
    DevelopmentPlaintext,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyAuthentication {
    DeviceAuthentication,
    HostKeyMaterial,
    None,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyAvailability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyDowngrade {
    None,
    ActiveBackendMismatch,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCustodyFailureCode {
    UnsupportedTarget,
    FeatureDisabled,
    AuthenticationRequired,
    KeyMaterialRequired,
    BackendFailure,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityCustodyFailure {
    pub code: IdentityCustodyFailureCode,
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityCustodyInfo {
    pub requested_backend: IdentityCustodyBackend,
    pub active_backend: Option<IdentityCustodyBackend>,
    pub protection: Option<IdentityCustodyProtection>,
    pub authentication: IdentityCustodyAuthentication,
    pub availability: IdentityCustodyAvailability,
    pub downgrade: IdentityCustodyDowngrade,
    pub failure: Option<IdentityCustodyFailure>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct IdentityInfo {
    pub identity_hash: String,
    pub destination_hash: String,
    pub lxmf_destination_hash: String,
    pub display_name: String,
    pub icon: Option<String>,
    pub short_name: Option<String>,
    pub custody: Option<IdentityCustodyInfo>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IdentityBackupFormat {
    LegacyV0,
    StidV1,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct IdentityBackupMetadata {
    pub contract_version: u8,
    pub format: IdentityBackupFormat,
    pub encrypted_size: u64,
}

/// Opaque encrypted artifact returned only by the explicit backup export operation.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct IdentityBackupExport {
    pub metadata: IdentityBackupMetadata,
    pub encrypted_bytes: Vec<u8>,
}

impl std::fmt::Debug for IdentityBackupExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityBackupExport")
            .field("metadata", &self.metadata)
            .field("encrypted_bytes", &"[REDACTED]")
            .finish()
    }
}

/// Opaque encrypted artifact accepted only by the explicit backup restore operation.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct IdentityBackupImport {
    pub encrypted_bytes: Vec<u8>,
}

impl std::fmt::Debug for IdentityBackupImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityBackupImport").field("encrypted_bytes", &"[REDACTED]").finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IdentityRestoreOutcome {
    Restored,
    AlreadyPresent,
    #[default]
    #[serde(other)]
    Unknown,
}

// ── Daemon status ─────────────────────────────────────────────────────────────

pub const ACTIVE_CAPABILITIES_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CapabilityFailureCode {
    Unavailable,
    Unauthorized,
    Degraded,
    Unverified,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct DegradedCapabilityInfo {
    pub id: String,
    pub reason: String,
    pub reason_code: CapabilityFailureCode,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct CapabilityFailureInfo {
    pub id: String,
    pub code: CapabilityFailureCode,
    pub retryable: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ActiveCapabilitiesInfo {
    pub version: u16,
    pub generation: Option<u64>,
    pub runtime: Vec<String>,
    pub degraded: Vec<DegradedCapabilityInfo>,
    pub failures: Vec<CapabilityFailureInfo>,
    pub authorized_operations: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct DaemonStatusInfo {
    pub uptime: u64,
    pub daemon_version: String,
    pub rns_initialized: bool,
    pub lxmf_initialized: bool,
    pub device_count: u32,
    pub interface_count: u32,
    pub hub_status: Option<String>,
    /// Existing Styrene-specific CBOR store-and-forward service state.
    pub propagation_enabled: bool,
    /// Whether this process registered the standard `lxmf.propagation` destination.
    pub standard_lxmf_propagation_destination_registered: bool,
    /// Whether standard propagation request handlers are ready and advertised active.
    pub standard_lxmf_propagation_active: bool,
    pub propagation_count: u32,
    pub propagation_size_bytes: u64,
    pub transport_enabled: bool,
    pub active_links: u32,
    pub active_capabilities: Option<ActiveCapabilitiesInfo>,
    pub connection_generation: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct PropagationQuery {
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct PropagationSnapshot {
    pub enabled: bool,
    pub queue_count: u32,
    pub queue_size_bytes: u64,
    pub expiry_secs: u64,
    pub capacity_bytes: Option<u64>,
    pub queue: Vec<PropagationQueueEntry>,
    pub peers: Vec<PropagationPeerInfo>,
    pub peer_state_supported: bool,
    pub sync_state_supported: bool,
    pub failures: Vec<PropagationFailureInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct PropagationQueueEntry {
    pub id: String,
    pub destination_hash: String,
    pub source_hash: Option<String>,
    pub received_at: i64,
    pub expires_at: i64,
    pub size_bytes: u64,
    pub attempts: Option<u32>,
    pub state: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct PropagationPeerInfo {
    pub peer_hash: String,
    pub state: String,
    pub last_sync_at: Option<i64>,
    pub queued_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct PropagationFailureInfo {
    pub operation: String,
    pub peer_hash: Option<String>,
    pub message: String,
    pub timestamp: i64,
}

pub const STANDARD_PROPAGATION_SNAPSHOT_VERSION: u16 = 1;
pub const MAX_STANDARD_PROPAGATION_PEERS: usize = 128;
pub const MAX_STANDARD_PROPAGATION_ATTEMPTS: usize = 256;
pub const MAX_STANDARD_PROPAGATION_CHECKPOINTS: usize = 128;
pub const MAX_STANDARD_PROPAGATION_FAILURES: usize = 128;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationTriggerSource {
    InitialConnection,
    Reconnect,
    ForegroundOpportunity,
    GrantedBackgroundOpportunity,
    Manual,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationOpportunityState {
    Unsupported,
    Available,
    Denied,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationPlatformCapability {
    AutomaticForeground,
    AutomaticBackground,
    Manual,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationSelectionReadiness {
    Ready,
    NoSelection,
    Unavailable,
    Unknown,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationSyncReadiness {
    Ready,
    InFlight,
    CoolingDown,
    Unavailable,
    Unknown,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationSyncTerminalOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationTriggerCapabilityInfo {
    pub source: StandardPropagationTriggerSource,
    pub platform_capability: StandardPropagationPlatformCapability,
    pub opportunity: StandardPropagationOpportunityState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationActiveSyncInfo {
    pub trigger: StandardPropagationTriggerSource,
    pub started_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationLastSynchronizationInfo {
    pub trigger: StandardPropagationTriggerSource,
    pub started_at: i64,
    pub finished_at: i64,
    pub outcome: StandardPropagationSyncTerminalOutcome,
    pub new_messages: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationDirection {
    Ingress,
    Egress,
    Sync,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationStage {
    Offer,
    Transfer,
    Get,
    Fetch,
    Download,
    Sync,
    Complete,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationAttemptState {
    Running,
    Completed,
    Failed,
    Interrupted,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StandardPropagationOutcome {
    Pending,
    Completed,
    Failed,
    Interrupted,
    CapacityRejected,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationPolicyInfo {
    pub target_cost: u32,
    pub flexibility: u32,
    pub peering_cost: u32,
    /// Transfer limit in decimal kilobytes (1 kB = 1000 bytes).
    pub transfer_limit_kb: u64,
    /// Synchronization limit in decimal kilobytes (1 kB = 1000 bytes).
    pub sync_limit_kb: u64,
    pub queue_max_count: u64,
    pub queue_max_bytes: u64,
    pub expiry_secs: u64,
    pub throttle_secs: u64,
    pub max_offer_links: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationQueueStats {
    pub queued_count: u64,
    pub queued_bytes: u64,
    pub acknowledged_count: u64,
    pub expired_count: u64,
    pub terminal_count: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationSelectionInfo {
    pub peer_hash: Option<String>,
    pub mode: String,
    pub selected_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationPeerObservation {
    pub peer_hash: String,
    pub propagation_destination_hash: Option<String>,
    pub configured: bool,
    pub enabled: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub retry_at: Option<i64>,
    pub backoff_count: u64,
    pub offered_count: u64,
    pub wanted_count: u64,
    pub accepted_count: u64,
    /// LXMF payload bytes transferred; stamps, storage overhead, and acknowledgements are excluded.
    pub accepted_bytes: u64,
    pub failure_count: u64,
    pub transfer_limit_kb: Option<u64>,
    pub sync_limit_kb: Option<u64>,
    pub stamp_cost: Option<u32>,
    pub stamp_flexibility: Option<u32>,
    pub peering_cost: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationAttemptObservation {
    pub attempt_id: String,
    pub correlation_id: String,
    pub peer_hash: Option<String>,
    pub direction: StandardPropagationDirection,
    pub stage: StandardPropagationStage,
    pub state: StandardPropagationAttemptState,
    pub outcome: StandardPropagationOutcome,
    pub started_at: i64,
    pub updated_at: i64,
    pub deadline_at: Option<i64>,
    pub offered_count: u64,
    pub wanted_count: u64,
    pub accepted_count: u64,
    pub accepted_bytes: u64,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationCheckpointObservation {
    pub peer_hash: String,
    pub direction: StandardPropagationDirection,
    pub completed_stage: StandardPropagationStage,
    pub item_count: u64,
    /// LXMF payload bytes transferred; stamps, storage overhead, and acknowledgements are excluded.
    pub byte_count: u64,
    pub last_attempt_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationFailureObservation {
    pub code: String,
    pub occurred_at: i64,
    pub peer_hash: Option<String>,
    pub attempt_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StandardPropagationSnapshot {
    pub version: u16,
    pub registered: bool,
    pub active: bool,
    pub observed_at: Option<i64>,
    pub connection_generation: Option<u64>,
    pub policy: Option<StandardPropagationPolicyInfo>,
    pub queue: StandardPropagationQueueStats,
    pub selection: Option<StandardPropagationSelectionInfo>,
    pub selection_readiness: StandardPropagationSelectionReadiness,
    pub sync_readiness: StandardPropagationSyncReadiness,
    pub automatic_sync_enabled: Option<bool>,
    pub automatic_sync_cooldown_secs: Option<u64>,
    pub sync_deadline_secs: Option<u64>,
    pub trigger_capabilities: Vec<StandardPropagationTriggerCapabilityInfo>,
    pub active_sync: Option<StandardPropagationActiveSyncInfo>,
    pub last_synchronization: Option<StandardPropagationLastSynchronizationInfo>,
    pub cooldown_remaining_secs: Option<u64>,
    pub peers: Vec<StandardPropagationPeerObservation>,
    pub attempts: Vec<StandardPropagationAttemptObservation>,
    pub checkpoints: Vec<StandardPropagationCheckpointObservation>,
    pub failures: Vec<StandardPropagationFailureObservation>,
    pub peers_truncated: bool,
    pub attempts_truncated: bool,
    pub checkpoints_truncated: bool,
    pub failures_truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct InterfaceInfo {
    pub name: String,
    #[serde(rename = "type", alias = "kind")]
    pub kind: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConfigSnapshot {
    pub values: BTreeMap<String, serde_json::Value>,
}

// ── Messaging ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageAuthenticationState {
    Verified,
    Invalid,
    UnknownIdentity,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageStampState {
    Verified,
    Invalid,
    NotApplicable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageLifecycleState {
    Queued,
    Sending,
    Sent,
    Delivered,
    Failed,
    Cancelled,
    Expired,
    Rejected,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageRetryIneligibilityReason {
    Inbound,
    MissingOutboundRoute,
    LifecycleState,
    CanonicalWireUnavailable,
    AttemptLimitReached,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageDeliveryEvidenceKind {
    PacketReceipt,
    ResourceCompletion,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageDeliveryEvidenceState {
    Tracked,
    Completed,
    Failed,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct MessageDeliveryEvidenceInfo {
    pub kind: MessageDeliveryEvidenceKind,
    /// Exact packet or resource hash as lowercase hexadecimal metadata.
    pub hash: String,
    pub representation: String,
    pub state: MessageDeliveryEvidenceState,
    pub outcome: Option<String>,
    pub attempt: Option<u32>,
    pub correlation_id: Option<String>,
    pub observed_at: i64,
    pub terminal_at: Option<i64>,
    /// Resource transfer counters. Always absent for packet evidence.
    pub transferred_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// Integer percentage in the inclusive range 0..=100.
    pub progress: Option<u8>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct MessageInfo {
    /// True only when this value is a complete daemon projection. False event
    /// values are sparse patches and must be merged or followed by a requery.
    pub projection_complete: bool,
    pub id: String,
    pub source_hash: String,
    pub destination_hash: String,
    pub timestamp: i64,
    /// Canonical LXMF timestamp, including any fractional seconds.
    pub lxmf_timestamp: Option<f64>,
    pub content: String,
    pub title: Option<String>,
    pub status: String,
    pub lifecycle_state: MessageLifecycleState,
    pub terminal_detail: Option<String>,
    /// Authoritative retry eligibility. Absent on sparse or legacy projections.
    pub retry_eligible: Option<bool>,
    /// Present only when the backend established that retry is ineligible.
    pub retry_ineligibility_reason: Option<MessageRetryIneligibilityReason>,
    pub is_outgoing: bool,
    pub delivery_method: Option<String>,
    pub requested_delivery_method: Option<String>,
    pub actual_delivery_method: Option<String>,
    pub fallback_reason: Option<String>,
    pub correlation_id: Option<String>,
    pub attempts: Vec<MessageAttemptInfo>,
    /// Authorized message-side standard LXMF propagation correlations.
    pub propagation_correlations: Vec<MessagePropagationCorrelationInfo>,
    pub read: bool,
    pub attachment_info: Option<AttachmentInfo>,
    pub attachments: Vec<AttachmentInfo>,
    pub authentication_state: MessageAuthenticationState,
    pub stamp_state: MessageStampState,
    pub stamp_value: Option<u32>,
    /// Exact target cost applied while validating this inbound message.
    pub stamp_cost: Option<u32>,
    /// Bounded, validated packet/resource delivery evidence retained by the daemon.
    pub delivery_evidence: Vec<MessageDeliveryEvidenceInfo>,
    // Canonical protocol material is retained by the daemon but deliberately
    // excluded from serde and local IPC projections.
    #[serde(skip)]
    pub canonical_title: Option<Vec<u8>>,
    #[serde(skip)]
    pub canonical_content: Option<Vec<u8>>,
    #[serde(skip)]
    pub canonical_fields_msgpack: Option<Vec<u8>>,
    #[serde(skip)]
    pub canonical_signature: Option<Vec<u8>>,
    #[serde(skip)]
    pub canonical_stamp: Option<Vec<u8>>,
    #[serde(skip)]
    pub canonical_wire: Option<Vec<u8>>,
}

impl std::fmt::Debug for MessageInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageInfo")
            .field("id", &self.id)
            .field("source_hash", &self.source_hash)
            .field("destination_hash", &self.destination_hash)
            .field("timestamp", &self.timestamp)
            .field("lxmf_timestamp", &self.lxmf_timestamp)
            .field("content", &self.content)
            .field("title", &self.title)
            .field("status", &self.status)
            .field("lifecycle_state", &self.lifecycle_state)
            .field("terminal_detail", &self.terminal_detail)
            .field("retry_eligible", &self.retry_eligible)
            .field("retry_ineligibility_reason", &self.retry_ineligibility_reason)
            .field("is_outgoing", &self.is_outgoing)
            .field("attachments", &self.attachments)
            .field("authentication_state", &self.authentication_state)
            .field("stamp_state", &self.stamp_state)
            .field("stamp_value", &self.stamp_value)
            .field("stamp_cost", &self.stamp_cost)
            .field("delivery_evidence", &self.delivery_evidence)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct MessageAttemptInfo {
    pub message_id: String,
    pub number: u32,
    pub started_unix_ms: i64,
    pub deadline_unix_ms: i64,
    pub state: String,
    /// Bearer observed for this attempt, independent of the requested delivery method.
    pub bearer: Option<String>,
    /// Immutable path-table evidence captured for this attempt.
    pub route: MessageAttemptRouteObservation,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageAttemptRouteOutcome {
    Observed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct MessageAttemptInterfaceObservation {
    /// Public interface identity hash. Endpoints and device paths are intentionally excluded.
    pub id: String,
    pub kind: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct MessageAttemptRouteObservation {
    pub outcome: MessageAttemptRouteOutcome,
    pub connection_generation: Option<u64>,
    pub observed_at: Option<i64>,
    pub next_hop: Option<String>,
    pub hops: Option<u32>,
    pub stale: bool,
    pub interface: Option<MessageAttemptInterfaceObservation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct MessagePropagationCorrelationInfo {
    pub relation: String,
    pub transient_id: String,
    pub attempt_id: Option<String>,
    pub peer_hash: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct AttachmentInfo {
    pub ordinal: u8,
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub checksum: String,
    pub availability: String,
    pub integrity: String,
    pub transfer: Option<Box<AttachmentTransferInfo>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct AttachmentTransferInfo {
    pub message_id: String,
    pub transfer_id: String,
    pub resource_hash: Option<String>,
    pub representation: String,
    pub direction: String,
    pub state: String,
    pub transferred: u64,
    pub total: u64,
    pub checksum_verified: bool,
    /// Inbound resources are cancellable through generic ResourceCancel until
    /// message identity is known; message-specific cancellation applies only afterward.
    pub cancellable: bool,
    pub error: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct AttachmentInput {
    pub name: String,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub expected_sha256: Option<String>,
}

impl std::fmt::Debug for AttachmentInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentInput")
            .field("name", &self.name)
            .field("byte_len", &self.bytes.len())
            .field("content_type", &self.content_type)
            .field("expected_sha256", &self.expected_sha256)
            .finish()
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(default)]
pub struct AttachmentChunk {
    pub attachment: AttachmentInfo,
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub done: bool,
}

impl std::fmt::Debug for AttachmentChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentChunk")
            .field("attachment", &self.attachment)
            .field("byte_len", &self.data.len())
            .field("next_offset", &self.next_offset)
            .field("done", &self.done)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct ConversationInfo {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message_timestamp: Option<i64>,
    pub last_message_content: Option<String>,
    pub unread_count: u32,
    pub message_count: u32,
    pub pinned: bool,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConversationInvalidationReason {
    ContactAliasChanged,
    ContactAliasRemoved,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ConversationInvalidation {
    pub peer_hash: String,
    pub reason: ConversationInvalidationReason,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct MessagePage {
    pub messages: Vec<MessageInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct ConversationPage {
    pub conversations: Vec<ConversationInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ConversationDraft {
    pub peer_hash: String,
    pub content: String,
    pub updated_at: i64,
    pub revision: u64,
}

impl std::fmt::Debug for ConversationDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConversationDraft")
            .field("peer_hash", &self.peer_hash)
            .field("content_byte_len", &self.content.len())
            .field("updated_at", &self.updated_at)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct ContactInfo {
    pub peer_hash: String,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Authoritative disposition of a local messaging operation.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessagingDisposition {
    Applied,
    Unchanged,
    NotFound,
    TerminalConflict,
    AlreadyCancelled,
    Created,
    Updated,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Result of a messaging mutation, including the authoritative post-commit projection.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct MessagingOperationOutcome {
    pub disposition: MessagingDisposition,
    pub affected_count: u64,
    pub target_id: String,
    pub correlated_id: Option<String>,
    pub message: Option<MessageInfo>,
    pub conversation: Option<ConversationInfo>,
    pub contact: Option<ContactInfo>,
    /// Persisted terminal state when a lifecycle operation loses a race.
    pub terminal_state: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SendChatDisposition {
    Accepted,
    Failed,
    PaperExported,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Immediate authoritative send result. Paper URI material is response-only and
/// intentionally absent from `MessageInfo` and daemon events.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SendChatOutcome {
    pub disposition: SendChatDisposition,
    pub message_id: String,
    pub message: MessageInfo,
    pub requested_method: String,
    pub actual_method: String,
    pub fallback_reason: Option<String>,
    pub terminal_error: Option<String>,
    pub paper_uri: Option<String>,
}

impl std::fmt::Debug for SendChatOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SendChatOutcome")
            .field("disposition", &self.disposition)
            .field("message_id", &self.message_id)
            .field("message", &self.message)
            .field("requested_method", &self.requested_method)
            .field("actual_method", &self.actual_method)
            .field("fallback_reason", &self.fallback_reason)
            .field("terminal_error", &self.terminal_error)
            .field("paper_uri", &self.paper_uri.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Authoritative first-page message search result. Search intentionally has no cursor.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct MessageSearchOutcome {
    pub messages: Vec<MessageInfo>,
    pub truncated: bool,
    pub returned_count: u32,
    pub matched_count: u64,
    pub order: String,
    pub query: String,
    pub peer_hash: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[serde(default)]
pub struct SendChatRequest {
    pub peer_hash: String,
    pub content: String,
    pub title: Option<String>,
    pub delivery_method: Option<String>,
    pub reply_to_hash: Option<String>,
    pub attachment: Option<Vec<u8>>,
    pub attachment_name: Option<String>,
    pub attachments: Vec<AttachmentInput>,
}

impl std::fmt::Debug for SendChatRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SendChatRequest")
            .field("peer_hash", &self.peer_hash)
            .field("content", &self.content)
            .field("title", &self.title)
            .field("delivery_method", &self.delivery_method)
            .field("reply_to_hash", &self.reply_to_hash)
            .field("attachment_byte_len", &self.attachment.as_ref().map(Vec::len))
            .field("attachment_name", &self.attachment_name)
            .field("attachments", &self.attachments)
            .finish()
    }
}

// ── Auto-reply ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct AutoReplyConfig {
    pub mode: String,
    pub message: Option<String>,
    pub cooldown_secs: Option<u64>,
}

// ── Path info ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ObservationSource {
    RuntimeInterfaceRegistry,
    TransportPathTable,
    TransportLinkState,
    TransportRequestState,
    TransportResourceState,
    OperationCoordinator,
    Fixture,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ObservationSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeInterfaceRegistry => "runtime_interface_registry",
            Self::TransportPathTable => "transport_path_table",
            Self::TransportLinkState => "transport_link_state",
            Self::TransportRequestState => "transport_request_state",
            Self::TransportResourceState => "transport_resource_state",
            Self::OperationCoordinator => "operation_coordinator",
            Self::Fixture => "fixture",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
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

impl ObservationMetadata {
    /// Physical IPC generation, falling back to the legacy overloaded field.
    pub fn ipc_generation(&self) -> Option<u64> {
        self.ipc_connection_generation.or(self.connection_generation)
    }

    pub fn at(
        source: ObservationSource,
        observed_at: Option<i64>,
        now: i64,
        threshold_secs: u64,
    ) -> Self {
        let age_secs = observed_at.map(|observed| now.saturating_sub(observed).max(0) as u64);
        Self {
            source,
            observed_at,
            connection_generation: None,
            ipc_connection_generation: None,
            interface_generation: None,
            age_secs,
            freshness_threshold_secs: Some(threshold_secs),
            stale: age_secs.is_some_and(|age| age > threshold_secs),
            correlation_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct PathInfo {
    pub destination_hash: String,
    pub hops: Option<u32>,
    pub next_hop: Option<String>,
    pub interface: Option<String>,
    pub expires: Option<i64>,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RouteEventKind {
    Discovered,
    Lost,
    Rediscovered,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RouteLossReason {
    Expired,
    InterfaceUnavailable,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct RouteEventInfo {
    pub kind: RouteEventKind,
    pub route: PathInfo,
    pub loss_reason: Option<RouteLossReason>,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    #[serde(other)]
    Unknown,
}

impl RequestState {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Receiving | Self::Unknown)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RequestResponseTransfer {
    Packet,
    Resource,
    #[default]
    #[serde(other)]
    None,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RequestProtocolError {
    LinkClosed,
    Timeout,
    MalformedResponse,
    Cancelled,
    ResponseTooLarge,
    ResourceFailed,
    TransportFailed,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
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
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct StartRequestInfo {
    pub link_id: String,
    pub path: String,
    pub data: Vec<u8>,
    pub timeout_ms: u64,
    pub max_response_size: u64,
    /// Operation correlation inherited by request and resource observations.
    pub correlation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceDirection {
    Inbound,
    Outbound,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceTransferState {
    Transferring,
    Completed,
    Cancelled,
    TimedOut,
    LinkClosed,
    IntegrityFailed,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ResourceTransferState {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Transferring | Self::Unknown)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct ResourceTransferInfo {
    pub resource_hash: String,
    pub link_id: String,
    pub direction: ResourceDirection,
    pub state: ResourceTransferState,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub received_parts: u64,
    pub total_parts: u64,
    pub progress: f32,
    pub cancellable: bool,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

// ── Reticulum operations ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NetworkOperationKind {
    Announce,
    PathRequest,
    Probe,
    LinkOpen,
    LinkClose,
    #[default]
    #[serde(other)]
    Unknown,
}

impl NetworkOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Announce => "announce",
            Self::PathRequest => "path_request",
            Self::Probe => "probe",
            Self::LinkOpen => "link_open",
            Self::LinkClose => "link_close",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NetworkOperationProgress {
    Accepted,
    Dispatched,
    AwaitingPath,
    AwaitingLink,
    AwaitingProbe,
    AwaitingClose,
    #[default]
    #[serde(other)]
    Unknown,
}

impl NetworkOperationProgress {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Dispatched => "dispatched",
            Self::AwaitingPath => "awaiting_path",
            Self::AwaitingLink => "awaiting_link",
            Self::AwaitingProbe => "awaiting_probe",
            Self::AwaitingClose => "awaiting_close",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NetworkOperationOutcome {
    Succeeded,
    /// The daemon handed an unacknowledged operation to the network successfully.
    Dispatched,
    TimedOut,
    Denied,
    Unavailable,
    Cancelled,
    Failed,
    #[serde(other)]
    Unknown,
}

impl NetworkOperationOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Dispatched => "dispatched",
            Self::TimedOut => "timed_out",
            Self::Denied => "denied",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct StartNetworkOperationInfo {
    pub kind: NetworkOperationKind,
    pub destination_hash: Option<String>,
    pub link_id: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct NetworkOperationInfo {
    pub operation_id: String,
    pub kind: NetworkOperationKind,
    pub destination_hash: Option<String>,
    pub link_id: Option<String>,
    pub started_unix_ms: i64,
    pub deadline_unix_ms: i64,
    pub cancellable: bool,
    pub progress: NetworkOperationProgress,
    pub outcome: Option<NetworkOperationOutcome>,
    pub detail: Option<String>,
    pub rtt_ms: Option<f64>,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

impl NetworkOperationInfo {
    pub const fn is_terminal(&self) -> bool {
        self.outcome.is_some()
    }
}

// ── Fleet / remote operations ─────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct RemoteStatusInfo {
    pub destination_hash: String,
    pub uptime: Option<u64>,
    pub daemon_version: Option<String>,
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct RebootResult {
    pub accepted: bool,
    pub delay_secs: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SelfUpdateResult {
    pub accepted: bool,
    pub current_version: Option<String>,
    pub target_version: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConfigApplyResult {
    pub success: bool,
    pub verified: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

// ── Terminal sessions ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct TerminalOpenRequest {
    pub destination: String,
    pub term_type: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub shell: Option<String>,
}

// ── Tunnel management ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct TunnelInfo {
    pub peer_hash: String,
    pub backend: String,
    pub state: String,
    pub remote_endpoint: Option<String>,
    pub interface_name: Option<String>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub established_at: Option<i64>,
    pub last_rekey: Option<i64>,
    pub pqc_session_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TunnelOperationInfo {
    pub operation_id: String,
    pub peer_hash: String,
    pub kind: String,
    pub state: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct TunnelSaInfo {
    pub sa_id: String,
    pub protocol: String,
    pub cipher_suite: String,
    pub local_address: Option<String>,
    pub remote_address: Option<String>,
    pub established_at: Option<i64>,
    pub rekey_at: Option<i64>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

// ── Page browsing ────────────────────────────────────────────────────────────

/// A page hosted by a NomadNet/Styrene node.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageInfo {
    /// Page path (e.g., "/index", "/status", "/id").
    pub path: String,
    /// Page title, if extractable from content.
    pub title: Option<String>,
    /// Hosting node's destination hash.
    pub host_hash: String,
    /// Hosting node's display name.
    pub host_name: Option<String>,
    /// Bounded daemon value: `page` or `file`.
    pub kind: String,
    pub dynamic: bool,
    pub restricted: bool,
    /// True only when this exact path was installed on the active native destination.
    pub handler_active: bool,
}

/// Rendered page content.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
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
    #[serde(default)]
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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PageNavigationAction {
    #[default]
    Navigate,
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageNavigationRequest {
    pub session_id: Option<String>,
    pub action: PageNavigationAction,
    /// Address or relative link for `Navigate`; ignored by history and reload actions.
    pub target: Option<String>,
    pub bypass_cache: bool,
    pub timeout_secs: Option<u64>,
    pub submission: Option<PageFormSubmission>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageFormSubmission {
    /// Current UI values keyed by the Micron field name. Repeated values preserve
    /// checked checkbox/radio state in document order.
    pub values: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for PageFormSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PageFormSubmission")
            .field("field_names", &self.values.keys().collect::<Vec<_>>())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageNavigationInfo {
    pub session_id: String,
    pub address: String,
    pub history_index: u32,
    pub history_len: u32,
    pub can_back: bool,
    pub can_forward: bool,
    pub connection_open: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PageFormFieldKind {
    #[default]
    Text,
    Password,
    Checkbox,
    Radio,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageFormField {
    pub name: String,
    pub kind: PageFormFieldKind,
    pub value: Option<String>,
    pub width: Option<u8>,
    pub checked: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageLinkTarget {
    pub label: Option<String>,
    pub target: String,
    pub submitted_fields: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PageBrowseOutcome {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageBrowseFailure {
    pub stage: PageBrowseStageKind,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageBrowseStage {
    pub correlation_id: String,
    pub kind: PageBrowseStageKind,
    pub state: PageBrowseStageState,
    #[serde(default)]
    pub observation: ObservationMetadata,
    pub evidence_source: Option<ObservationSource>,
    pub destination_hash: Option<String>,
    pub link_id: Option<String>,
    pub request_id: Option<String>,
    pub resource_hash: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageParserWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageRequestMetadata {
    pub native_path: String,
    pub path_hash: String,
    pub request_id: Option<String>,
    pub link_id: Option<String>,
    pub request_size: u64,
    pub response_size: Option<u64>,
    pub rtt_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PageTransferKind {
    #[default]
    None,
    Local,
    Packet,
    Resource,
    Cache,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageTransferInfo {
    pub kind: PageTransferKind,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub resource_hash: Option<String>,
    pub verified: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PageCacheStatus {
    #[default]
    NotUsed,
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct PageCacheInfo {
    pub status: PageCacheStatus,
    pub stored_at: Option<i64>,
    /// Original successful browse whose immutable content populated this entry.
    pub origin_correlation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileDownloadState {
    #[default]
    Pending,
    Receiving,
    Completed,
    Cancelled,
    Failed,
    Saved,
}

impl FileDownloadState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed | Self::Saved)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct FileDownloadRequest {
    pub session_id: Option<String>,
    pub target: String,
    pub expected_sha256: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
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

// ── Interface management ─────────────────────────────────────────────────────

/// Detailed interface information.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct InterfaceDetail {
    pub name: String,
    pub hash: String,
    #[serde(rename = "type", alias = "kind")]
    pub kind: String,
    pub mode: String,
    pub enabled: bool,
    pub status: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub local_endpoint: Option<String>,
    pub remote_endpoint: Option<String>,
    pub parent_hash: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    #[serde(rename = "connected_peers", alias = "peers_connected")]
    pub peers_connected: u32,
    pub failure: Option<InterfaceFailureInfo>,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InterfaceFailureCode {
    Retrying,
    Closed,
    UnknownState,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct InterfaceFailureInfo {
    pub code: InterfaceFailureCode,
    pub retryable: bool,
}

// ── Events ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)] // Preserve the established by-value Message event API.
pub enum DaemonEvent {
    Message {
        kind: MessageEventKind,
        message: MessageInfo,
    },
    Device {
        device: DeviceInfo,
    },
    TerminalOutput {
        session_id: SessionId,
        data: Vec<u8>,
    },
    TerminalStateChange {
        session_id: SessionId,
        state: TerminalState,
    },
    TunnelStateChange {
        peer_hash: PeerHash,
        state: String,
        backend: String,
    },
    Link {
        event: LinkEvent,
    },
    Route {
        event: RouteEventInfo,
    },
    Request {
        event: RequestObservationInfo,
    },
    RequestReconcileRequired {
        dropped: u64,
    },
    NetworkOperation {
        operation: NetworkOperationInfo,
    },
    Resource {
        transfer: ResourceTransferInfo,
    },
    AttachmentTransfer {
        transfer: AttachmentTransferInfo,
    },
    MessagingOperation {
        outcome: Box<MessagingOperationOutcome>,
    },
    ConversationInvalidated {
        invalidation: ConversationInvalidation,
    },
    /// Durable standard propagation state changed; clients must requery the snapshot.
    StandardPropagationChanged {
        observed_at: i64,
    },
    ReconcileRequired {
        dropped: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LinkEventKind {
    Established,
    Identified,
    Activity,
    RttUpdated,
    Teardown,
    Timeout,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LinkActivity {
    Active,
    Historical,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LinkLifecycleReason {
    LocalTeardown,
    StaleTimeout,
    EstablishmentTimeout,
    ChannelTimeout,
    SendFailure,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Link telemetry event and current-state observation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct LinkEvent {
    /// Short hex ID of the link (16 chars).
    pub link_id: String,
    /// Destination peer hash (32 chars).
    pub peer_hash: String,
    /// Cached peer display name, if known.
    pub peer_name: Option<String>,
    /// Runtime interface hash that owns this link.
    pub interface: Option<String>,
    /// New lifecycle state: "active", "stale", "closed", "pending".
    pub status: String,
    pub kind: LinkEventKind,
    pub activity: LinkActivity,
    pub reason: Option<LinkLifecycleReason>,
    pub identified: bool,
    /// Authenticated identity hash from LINKIDENTIFY, not the ephemeral link key.
    pub remote_identity_hash: Option<String>,
    /// Round-trip time in milliseconds, if measured.
    pub rtt_ms: Option<f64>,
    /// Epoch seconds of the event.
    pub timestamp: i64,
    #[serde(flatten, default)]
    pub observation: ObservationMetadata,
}

impl LinkEvent {
    pub fn new(
        link_id: impl Into<String>,
        peer_hash: impl Into<String>,
        status: impl Into<String>,
        rtt_ms: Option<f64>,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let status = status.into();
        Self {
            link_id: link_id.into(),
            peer_hash: peer_hash.into(),
            peer_name: None,
            interface: None,
            kind: match status.as_str() {
                "active" => LinkEventKind::Established,
                "rtt_updated" => LinkEventKind::RttUpdated,
                "closed" => LinkEventKind::Teardown,
                _ => LinkEventKind::Unknown,
            },
            activity: if status == "closed" {
                LinkActivity::Historical
            } else {
                LinkActivity::Active
            },
            reason: None,
            identified: false,
            remote_identity_hash: None,
            status,
            rtt_ms,
            timestamp,
            observation: ObservationMetadata::at(
                ObservationSource::TransportLinkState,
                Some(timestamp),
                timestamp,
                300,
            ),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[non_exhaustive]
pub struct LinkSnapshot {
    pub active: Vec<LinkEvent>,
    pub history: Vec<LinkEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum MessageEventKind {
    New,
    StatusChanged,
    Delivered,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum TerminalState {
    Ready,
    Exited,
    Error,
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[test]
    fn discovered_capability_serde_spelling_matches_the_wire_spelling() {
        for capability in [
            DiscoveredCapability::NativeNomadNetHost,
            DiscoveredCapability::StandardLxmfPropagationHost,
        ] {
            let encoded = serde_json::to_value(capability).expect("encode");
            assert_eq!(encoded, serde_json::Value::from(capability.as_str()));
            let decoded: DiscoveredCapability =
                serde_json::from_value(serde_json::Value::from(capability.as_str()))
                    .expect("decode wire spelling");
            assert_eq!(decoded, capability);
        }
        let unknown: DiscoveredCapability =
            serde_json::from_value(serde_json::Value::from("future_capability")).expect("decode");
        assert_eq!(unknown, DiscoveredCapability::Unknown);
    }

    #[test]
    fn legacy_device_payload_has_no_discovered_capabilities() {
        let device: DeviceInfo = serde_json::from_str(r#"{"destination_hash":"peer"}"#).unwrap();

        assert!(device.discovered_capabilities.is_empty());
        assert_eq!(device.standard_lxmf_propagation_active, None);
    }

    #[test]
    fn standard_propagation_device_state_roundtrips_active_and_inactive() {
        for active in [false, true] {
            let device = DeviceInfo {
                standard_lxmf_propagation_active: Some(active),
                ..DeviceInfo::default()
            };
            let encoded = serde_json::to_string(&device).unwrap();
            let decoded: DeviceInfo = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded.standard_lxmf_propagation_active, Some(active));
        }
    }

    #[test]
    fn legacy_message_payload_defaults_canonical_fidelity_fields() {
        let message: MessageInfo = serde_json::from_str(
            r#"{"id":"legacy","source_hash":"peer","content":"hello","timestamp":1}"#,
        )
        .unwrap();

        assert_eq!(message.id, "legacy");
        assert_eq!(message.lxmf_timestamp, None);
        assert_eq!(message.authentication_state, MessageAuthenticationState::Unknown);
        assert_eq!(message.stamp_state, MessageStampState::Unknown);
        assert_eq!(message.retry_eligible, None);
        assert_eq!(message.retry_ineligibility_reason, None);
        assert!(message.canonical_wire.is_none());
        assert!(message.attachments.is_empty());
    }

    #[test]
    fn message_retry_eligibility_roundtrips_typed_reason() {
        let message = MessageInfo {
            retry_eligible: Some(false),
            retry_ineligibility_reason: Some(
                MessageRetryIneligibilityReason::CanonicalWireUnavailable,
            ),
            ..MessageInfo::default()
        };

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded: MessageInfo = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.retry_eligible, Some(false));
        assert_eq!(
            decoded.retry_ineligibility_reason,
            Some(MessageRetryIneligibilityReason::CanonicalWireUnavailable)
        );
    }

    #[test]
    fn legacy_send_chat_defaults_additive_attachment_list() {
        let request: SendChatRequest =
            serde_json::from_str(r#"{"peer_hash":"peer","content":"hello"}"#).unwrap();
        assert!(request.attachments.is_empty());
        assert!(request.attachment.is_none());
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn message_and_attachment_debug_output_redacts_all_byte_vectors() {
        let marker = [0xde, 0xad, 0xbe, 0xef];
        let mut message = MessageInfo::default();
        message.canonical_fields_msgpack = Some(marker.to_vec());
        message.canonical_wire = Some(marker.to_vec());
        let debug = format!("{message:?}");
        assert!(!debug.contains("222"));
        assert!(!debug.contains("173"));
        assert!(!debug.contains("canonical_fields_msgpack"));
        assert!(!debug.contains("canonical_wire"));

        let mut input = AttachmentInput::default();
        input.name = "marker.bin".into();
        input.bytes = marker.to_vec();
        let debug = format!("{input:?}");
        assert!(!debug.contains("222"));
        assert!(debug.contains("byte_len: 4"));
    }

    #[test]
    fn legacy_conversation_payload_defaults_pinned_and_muted() {
        let conversation: ConversationInfo =
            serde_json::from_str(r#"{"peer_hash":"peer","message_count":1}"#).unwrap();

        assert!(!conversation.pinned);
        assert!(!conversation.muted);
    }

    #[test]
    fn old_status_payload_has_no_negotiated_capabilities() {
        let status: DaemonStatusInfo = serde_json::from_str(r#"{"uptime":1}"#).unwrap();

        assert_eq!(status.active_capabilities, None);
        assert!(!status.standard_lxmf_propagation_destination_registered);
        assert!(!status.standard_lxmf_propagation_active);
        assert_eq!(status.connection_generation, None);
    }

    #[test]
    fn old_interface_payload_uses_defaults_for_runtime_metadata() {
        let interface: InterfaceDetail = serde_json::from_str(
            r#"{"name":"legacy","kind":"tcp_server","enabled":true,"status":"active"}"#,
        )
        .unwrap();

        assert_eq!(interface.name, "legacy");
        assert!(interface.mode.is_empty());
        assert!(interface.local_endpoint.is_none());
        assert!(interface.parent_hash.is_none());
        assert_eq!(interface.observation.source, ObservationSource::Unknown);
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)] // DTO is non-exhaustive by contract.
    fn interface_uses_unix_wire_keys_and_accepts_legacy_aliases() {
        let mut interface = InterfaceDetail::default();
        interface.kind = "tcp_client".into();
        interface.peers_connected = 2;

        let encoded = serde_json::to_value(&interface).unwrap();
        assert_eq!(encoded["type"], "tcp_client", "{encoded}");
        assert_eq!(encoded["connected_peers"], 2);
        assert!(encoded.get("kind").is_none());
        assert!(encoded.get("peers_connected").is_none());

        let legacy: InterfaceDetail =
            serde_json::from_str(r#"{"kind":"udp","peers_connected":3}"#).unwrap();
        assert_eq!(legacy.kind, "udp");
        assert_eq!(legacy.peers_connected, 3);
    }

    #[test]
    fn old_path_payload_uses_defaults_for_observation_metadata() {
        let path: PathInfo = serde_json::from_str(
            r#"{"destination_hash":"11111111111111111111111111111111","hops":1}"#,
        )
        .unwrap();

        assert_eq!(path.observation, ObservationMetadata::default());
    }

    #[test]
    fn observation_metadata_roundtrips_and_accepts_future_sources() {
        let path = PathInfo {
            destination_hash: "11111111111111111111111111111111".into(),
            observation: ObservationMetadata {
                source: ObservationSource::TransportPathTable,
                observed_at: Some(100),
                connection_generation: Some(7),
                ipc_connection_generation: Some(11),
                interface_generation: Some(3),
                age_secs: Some(2),
                freshness_threshold_secs: Some(300),
                stale: false,
                correlation_id: Some("request-1".into()),
            },
            ..Default::default()
        };

        let encoded = serde_json::to_string(&path).unwrap();
        let decoded = serde_json::from_str::<PathInfo>(&encoded).unwrap();
        assert_eq!(decoded.observation.ipc_generation(), Some(11));
        assert_eq!(decoded, path);

        let future: PathInfo =
            serde_json::from_str(r#"{"destination_hash":"peer","source":"future_runtime_source"}"#)
                .unwrap();
        assert_eq!(future.observation.source, ObservationSource::Unknown);
    }

    #[test]
    fn freshness_is_deterministic_at_threshold_boundaries() {
        let below =
            ObservationMetadata::at(ObservationSource::TransportPathTable, Some(91), 100, 10);
        let at = ObservationMetadata::at(ObservationSource::TransportPathTable, Some(90), 100, 10);
        let above =
            ObservationMetadata::at(ObservationSource::TransportPathTable, Some(89), 100, 10);
        let future =
            ObservationMetadata::at(ObservationSource::TransportPathTable, Some(101), 100, 10);

        assert_eq!(below.age_secs, Some(9));
        assert!(!below.stale);
        assert_eq!(at.age_secs, Some(10));
        assert!(!at.stale);
        assert_eq!(above.age_secs, Some(11));
        assert!(above.stale);
        assert_eq!(future.age_secs, Some(0));
        assert!(!future.stale);
    }

    #[test]
    fn capability_snapshot_roundtrips_version_reason_and_generation() {
        let degraded = DegradedCapabilityInfo {
            id: "runtime.native-nomadnet.host".into(),
            reason: "request handler unavailable".into(),
            reason_code: CapabilityFailureCode::Degraded,
        };
        let capabilities = ActiveCapabilitiesInfo {
            version: ACTIVE_CAPABILITIES_VERSION,
            generation: Some(7),
            runtime: vec!["runtime.lxmf.direct".into()],
            degraded: vec![degraded],
            failures: Vec::new(),
            authorized_operations: vec!["chat.send".into()],
        };
        let status = DaemonStatusInfo {
            active_capabilities: Some(capabilities),
            connection_generation: Some(7),
            ..Default::default()
        };

        let encoded = serde_json::to_string(&status).unwrap();
        let decoded: DaemonStatusInfo = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, status);
    }

    #[test]
    fn interface_observation_exposes_age_and_stale_state() {
        let encoded = serde_json::to_value(InterfaceDetail::default()).unwrap();

        assert!(encoded.get("age_secs").is_some(), "interface DTO must expose observation age");
        assert!(encoded.get("stale").is_some(), "interface DTO must expose explicit stale state");
    }

    #[test]
    fn path_observation_exposes_age_and_stale_state() {
        let encoded = serde_json::to_value(PathInfo::default()).unwrap();

        assert!(encoded.get("age_secs").is_some(), "path DTO must expose route age");
        assert!(encoded.get("stale").is_some(), "path DTO must expose explicit stale state");
    }

    #[test]
    fn route_event_roundtrips_and_accepts_future_kinds() {
        let event = RouteEventInfo {
            kind: RouteEventKind::Lost,
            route: PathInfo {
                destination_hash: "11111111111111111111111111111111".into(),
                expires: Some(200),
                ..Default::default()
            },
            loss_reason: Some(RouteLossReason::Expired),
            observation: ObservationMetadata::at(
                ObservationSource::TransportPathTable,
                Some(100),
                100,
                300,
            ),
        };

        let encoded = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<RouteEventInfo>(&encoded).unwrap(), event);

        let future: RouteEventInfo = serde_json::from_str(
            r#"{"kind":"rerouted","loss_reason":"new_reason","route":{"destination_hash":"peer"}}"#,
        )
        .unwrap();
        assert_eq!(future.kind, RouteEventKind::Unknown);
        assert_eq!(future.loss_reason, Some(RouteLossReason::Unknown));
    }

    #[test]
    fn link_snapshot_separates_active_state_from_typed_history() {
        let mut active = LinkEvent::new("link-1", "peer-1", "active", Some(2.5));
        active.interface = Some("iface-1".into());
        active.observation.connection_generation = Some(3);
        let mut timeout = active.clone();
        timeout.activity = LinkActivity::Historical;
        timeout.kind = LinkEventKind::Timeout;
        timeout.reason = Some(LinkLifecycleReason::StaleTimeout);
        let snapshot = LinkSnapshot { active: vec![active], history: vec![timeout] };

        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: LinkSnapshot = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.active[0].activity, LinkActivity::Active);
        assert_eq!(decoded.history[0].reason, Some(LinkLifecycleReason::StaleTimeout));
    }

    #[test]
    fn legacy_link_event_defaults_typed_fields() {
        let event: LinkEvent = serde_json::from_str(
            r#"{"link_id":"link","peer_hash":"peer","status":"active","timestamp":1}"#,
        )
        .unwrap();

        assert_eq!(event.kind, LinkEventKind::Unknown);
        assert_eq!(event.activity, LinkActivity::Unknown);
        assert_eq!(event.observation.source, ObservationSource::Unknown);
    }

    #[test]
    fn request_observation_roundtrips_progress_response_and_terminal_error() {
        let event = RequestObservationInfo {
            request_id: "11".repeat(16),
            path_hash: "22".repeat(16),
            link_id: "33".repeat(16),
            started_monotonic_ms: 100,
            deadline_monotonic_ms: 5_100,
            request_size: 31,
            response_size: Some(3),
            response_transfer_size: Some(40),
            received_bytes: 40,
            total_bytes: 40,
            progress: 1.0,
            response_transfer: RequestResponseTransfer::Packet,
            response: Some(vec![0x92, 1, 2]),
            state: RequestState::MalformedResponse,
            protocol_error: Some(RequestProtocolError::MalformedResponse),
            completed_monotonic_ms: Some(125),
            rtt_ms: Some(25),
            request_resource_hash: Some("55".repeat(32)),
            resource_hash: Some("44".repeat(32)),
            observation: ObservationMetadata {
                source: ObservationSource::TransportRequestState,
                correlation_id: Some("11".repeat(16)),
                ..Default::default()
            },
            ..Default::default()
        };

        let encoded = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<RequestObservationInfo>(&encoded).unwrap(), event);
        assert!(event.state.is_terminal());
    }

    #[test]
    fn resource_observation_roundtrips_progress_and_generation() {
        let transfer = ResourceTransferInfo {
            resource_hash: "44".repeat(32),
            link_id: "33".repeat(16),
            direction: ResourceDirection::Inbound,
            state: ResourceTransferState::Transferring,
            received_bytes: 512,
            total_bytes: 1_024,
            received_parts: 2,
            total_parts: 4,
            progress: 0.5,
            cancellable: true,
            observation: ObservationMetadata {
                source: ObservationSource::TransportResourceState,
                connection_generation: Some(7),
                correlation_id: Some("request-1".into()),
                ..Default::default()
            },
        };

        let encoded = serde_json::to_string(&transfer).unwrap();
        assert_eq!(serde_json::from_str::<ResourceTransferInfo>(&encoded).unwrap(), transfer);
        assert!(!transfer.state.is_terminal());
    }

    #[test]
    fn network_operation_roundtrips_authoritative_progress_and_terminal_outcome() {
        let mut operation = NetworkOperationInfo {
            operation_id: "11".repeat(16),
            kind: NetworkOperationKind::LinkOpen,
            destination_hash: Some("22".repeat(16)),
            link_id: Some("33".repeat(16)),
            started_unix_ms: 100,
            deadline_unix_ms: 5_100,
            progress: NetworkOperationProgress::AwaitingLink,
            outcome: Some(NetworkOperationOutcome::TimedOut),
            ..Default::default()
        };
        operation.observation.source = ObservationSource::OperationCoordinator;
        operation.observation.correlation_id = Some(operation.operation_id.clone());

        let encoded = serde_json::to_string(&operation).unwrap();
        let decoded: NetworkOperationInfo = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, operation);
        assert!(decoded.is_terminal());
        assert_eq!(decoded.observation.correlation_id, Some(decoded.operation_id));
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn messaging_dispositions_and_authoritative_payloads_roundtrip() {
        for disposition in [
            MessagingDisposition::Applied,
            MessagingDisposition::Unchanged,
            MessagingDisposition::NotFound,
            MessagingDisposition::TerminalConflict,
            MessagingDisposition::AlreadyCancelled,
            MessagingDisposition::Created,
            MessagingDisposition::Updated,
            MessagingDisposition::Unknown,
        ] {
            let mut outcome = MessagingOperationOutcome::default();
            outcome.disposition = disposition;
            outcome.affected_count = 2;
            outcome.target_id = "target".into();
            outcome.correlated_id = Some("correlated".into());
            outcome.conversation = Some(ConversationInfo::default());
            let encoded = serde_json::to_string(&outcome).expect("serialize outcome");
            let decoded: MessagingOperationOutcome =
                serde_json::from_str(&encoded).expect("deserialize outcome");
            assert_eq!(decoded, outcome);
            let wire = rmp_serde::to_vec_named(&outcome).expect("serialize outcome wire");
            assert_eq!(
                rmp_serde::from_slice::<MessagingOperationOutcome>(&wire)
                    .expect("deserialize outcome wire"),
                outcome
            );
        }
        let future: MessagingDisposition =
            serde_json::from_str("\"future_disposition\"").expect("future disposition");
        assert_eq!(future, MessagingDisposition::Unknown);
    }

    #[test]
    fn send_outcome_requires_authoritative_message_projection() {
        let message_id = "committed-message".to_string();
        let outcome = SendChatOutcome {
            disposition: SendChatDisposition::Failed,
            message_id: message_id.clone(),
            message: MessageInfo { id: message_id, ..Default::default() },
            terminal_error: Some("delivery failed".into()),
            ..Default::default()
        };

        let wire = rmp_serde::to_vec_named(&outcome).expect("serialize send outcome");
        let decoded: SendChatOutcome =
            rmp_serde::from_slice(&wire).expect("deserialize send outcome");
        assert_eq!(decoded.message.id, decoded.message_id);

        let missing = serde_json::json!({
            "disposition": "failed",
            "message_id": "committed-message",
            "requested_method": "direct",
            "actual_method": "direct",
            "fallback_reason": null,
            "terminal_error": "delivery failed",
            "paper_uri": null
        });
        assert!(serde_json::from_value::<SendChatOutcome>(missing).is_err());
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn search_outcome_roundtrips_authoritative_metadata() {
        let mut outcome = MessageSearchOutcome::default();
        outcome.messages = vec![MessageInfo::default()];
        outcome.truncated = true;
        outcome.returned_count = 1;
        outcome.matched_count = 3;
        outcome.order = "timestamp_desc_id_desc".into();
        outcome.query = "%_\\".into();
        outcome.peer_hash = Some("11".repeat(16));
        outcome.limit = 1;
        let encoded = serde_json::to_string(&outcome).expect("serialize search outcome");
        assert_eq!(
            serde_json::from_str::<MessageSearchOutcome>(&encoded).expect("deserialize search"),
            outcome
        );
    }

    #[test]
    fn legacy_standard_propagation_snapshot_defaults_additive_fields() {
        let snapshot: StandardPropagationSnapshot =
            serde_json::from_str(r#"{"version":1,"registered":true,"active":true}"#)
                .expect("deserialize legacy standard propagation snapshot");

        assert_eq!(snapshot.version, 1);
        assert!(snapshot.registered);
        assert!(snapshot.active);
        assert_eq!(snapshot.queue, StandardPropagationQueueStats::default());
        assert!(snapshot.peers.is_empty());
        assert!(snapshot.attempts.is_empty());
        assert!(snapshot.checkpoints.is_empty());
        assert!(snapshot.failures.is_empty());

        let legacy = std::collections::BTreeMap::from([
            ("version", serde_json::json!(1)),
            ("registered", serde_json::json!(true)),
            ("active", serde_json::json!(false)),
        ]);
        let encoded = rmp_serde::to_vec_named(&legacy).expect("serialize legacy snapshot");
        let snapshot: StandardPropagationSnapshot =
            rmp_serde::from_slice(&encoded).expect("deserialize legacy MessagePack snapshot");
        assert!(snapshot.registered);
        assert!(!snapshot.active);
        assert!(snapshot.peers.is_empty());
        assert_eq!(snapshot.selection_readiness, StandardPropagationSelectionReadiness::Other);
        assert_eq!(snapshot.sync_readiness, StandardPropagationSyncReadiness::Other);
        assert_eq!(snapshot.automatic_sync_enabled, None);
        assert!(snapshot.trigger_capabilities.is_empty());
        assert!(snapshot.active_sync.is_none());
        assert!(snapshot.last_synchronization.is_none());
        assert_eq!(snapshot.cooldown_remaining_secs, None);
    }

    #[test]
    fn standard_propagation_snapshot_roundtrips_explicit_trigger_projection() {
        let snapshot = StandardPropagationSnapshot {
            version: 1,
            registered: true,
            active: true,
            observed_at: Some(10),
            connection_generation: Some(11),
            policy: None,
            queue: StandardPropagationQueueStats::default(),
            selection: None,
            selection_readiness: StandardPropagationSelectionReadiness::Ready,
            sync_readiness: StandardPropagationSyncReadiness::CoolingDown,
            automatic_sync_enabled: Some(false),
            automatic_sync_cooldown_secs: Some(30),
            sync_deadline_secs: Some(32),
            trigger_capabilities: vec![StandardPropagationTriggerCapabilityInfo {
                source: StandardPropagationTriggerSource::ForegroundOpportunity,
                platform_capability: StandardPropagationPlatformCapability::AutomaticForeground,
                opportunity: StandardPropagationOpportunityState::Available,
            }],
            active_sync: Some(StandardPropagationActiveSyncInfo {
                trigger: StandardPropagationTriggerSource::ForegroundOpportunity,
                started_at: 12,
            }),
            last_synchronization: Some(StandardPropagationLastSynchronizationInfo {
                trigger: StandardPropagationTriggerSource::Manual,
                started_at: 1,
                finished_at: 2,
                outcome: StandardPropagationSyncTerminalOutcome::Failed,
                new_messages: 0,
            }),
            cooldown_remaining_secs: Some(17),
            peers: Vec::new(),
            attempts: Vec::new(),
            checkpoints: Vec::new(),
            failures: Vec::new(),
            peers_truncated: false,
            attempts_truncated: false,
            checkpoints_truncated: false,
            failures_truncated: false,
        };

        let encoded =
            serde_json::to_string(&snapshot).expect("serialize standard propagation snapshot");
        let decoded: StandardPropagationSnapshot =
            serde_json::from_str(&encoded).expect("deserialize standard propagation snapshot");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn standard_propagation_snapshot_has_no_payload_inventory_fields() {
        let encoded = serde_json::to_string(&StandardPropagationSnapshot::default())
            .expect("serialize standard propagation snapshot");

        for forbidden in [
            "\"lxmf_data\"",
            "\"stamp\"",
            "\"recipient_destination_hash\"",
            "\"transient_id\"",
            "\"failure_detail\"",
            "\"cursor\"",
        ] {
            assert!(!encoded.contains(forbidden), "snapshot leaked forbidden field {forbidden}");
        }
    }
}

// ── Operator profiles ────────────────────────────────────────────────────────

/// Where a profile's durable state lives and who owns its daemon.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProfileStorageKind {
    /// Temporary managed root, removed when its owner closes it.
    Quick,
    /// Persistent managed root.
    #[default]
    Local,
    /// Encrypted removable root resolved by a stable selector.
    Portable,
    /// An external daemon owns the profile; nothing here is managed.
    Connected,
    #[serde(other)]
    Unknown,
}

/// Who holds the profile lease right now.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileOwnership {
    /// Another owner holds the exclusive writer lease.
    pub leased_elsewhere: bool,
    /// This daemon holds the lease.
    pub held_by_daemon: bool,
    /// This daemon runs from the profile.
    pub active: bool,
}

/// How the profile persists.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfilePersistence {
    pub durable: bool,
    /// The root is removed when its owner releases it.
    pub removed_on_release: bool,
    pub snapshot_count: u32,
}

/// The daemon identity custody summary of a profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileCustodyInfo {
    /// `file` or `hardware`.
    pub backend: String,
    /// Hex address hash of the daemon RNS identity.
    pub fingerprint: String,
    pub recovery_slots: u32,
    pub identity_available: bool,
}

/// Network defaults the profile's daemon applies.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileNetworkPolicy {
    /// Managed profiles bind loopback ephemeral listeners unless configured.
    pub conservative_defaults: bool,
}

/// One profile as the backend knows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileInfo {
    pub id: String,
    pub display_name: String,
    pub storage: ProfileStorageKind,
    pub generation: u64,
    pub root: String,
    pub created_at_unix: u64,
    pub ownership: ProfileOwnership,
    pub persistence: ProfilePersistence,
    pub custody: ProfileCustodyInfo,
    pub network_policy: ProfileNetworkPolicy,
    /// Present for Portable profiles: the stable volume selector.
    pub volume_selector: Option<String>,
}

/// Every profile the backend can see plus the one it runs from.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileInventory {
    pub profiles: Vec<ProfileInfo>,
    pub active_profile_id: Option<String>,
    pub profiles_root: String,
}

/// Create a managed profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileCreateRequest {
    pub storage: ProfileStorageKind,
    pub display_name: String,
    /// Local profiles: the root directory to create. Quick profiles ignore it.
    pub root: Option<String>,
    /// Portable profiles: the mount point of the encrypted media.
    pub media_root: Option<String>,
}

/// Promote a stopped Quick profile to a Local destination.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfilePromoteRequest {
    pub profile_id: String,
    pub destination: String,
}

/// Snapshot a profile in place.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileSnapshotRequest {
    pub profile_id: String,
}

/// One snapshot generation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileSnapshotInfo {
    pub snapshot_id: String,
    pub profile_id: String,
    pub profile_generation: u64,
    pub identity_fingerprint: String,
    pub created_at_unix: u64,
    pub root: String,
    pub component_count: u32,
}

/// Restore a snapshot, or import an external snapshot, to an unused destination.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileRestoreRequest {
    pub snapshot_root: String,
    pub destination: String,
}

/// Export a profile as a verified snapshot outside the profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileExportRequest {
    pub profile_id: String,
    pub destination: String,
}

/// Adopt an existing profile root into the inventory.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileAdoptRequest {
    pub root: String,
}

/// Where a profile operation stands.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProfileOperationState {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    #[serde(other)]
    Unknown,
}

/// Progress of one profile operation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileOperationProgress {
    pub operation_id: String,
    pub kind: String,
    pub state: ProfileOperationState,
    pub detail: Option<String>,
    pub profile_id: Option<String>,
}

/// The typed result of a profile mutation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileOperationOutcome {
    pub progress: ProfileOperationProgress,
    pub profile: Option<ProfileInfo>,
    pub snapshot: Option<ProfileSnapshotInfo>,
    /// The change takes effect only after the daemon restarts from the
    /// resulting profile.
    pub restart_required: bool,
}
