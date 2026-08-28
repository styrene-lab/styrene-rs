//! Application state types for the desktop UI.
//!
//! These are UI-friendly types derived from daemon IPC responses.
//! No raw crypto or wire types in the view layer.

pub use styrene_ipc::PageAddress;

/// A peer in the sidebar list.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerEntry {
    pub hash: String,
    /// Identity hash reported by discovery, if available.
    pub identity_hash: Option<String>,
    /// Clean display name (Styrene prefix stripped).
    pub name: Option<String>,
    pub status: String,
    /// Parsed node role from announce data.
    pub node_role: PeerRole,
    /// Capabilities extracted from announce (e.g. "hub", "api", "pages").
    pub capabilities: Vec<String>,
    /// Version string if announced (e.g. "0.13.43").
    pub version: Option<String>,
    /// Timestamp of last announce (Unix epoch seconds).
    pub last_announce: Option<i64>,
    /// Number of announces seen from this peer.
    pub announce_count: u32,
}

/// What kind of node a peer is, derived from its announce data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeerRole {
    /// Vanilla RNS peer — no Styrene capabilities detected.
    #[default]
    Rns,
    /// Styrene peer — announced with Styrene protocol extensions.
    Styrene,
    /// Styrene hub — announced with "hub" capability.
    Hub,
    /// NomadNet-compatible page host — announced with "pages" capability.
    PageHost,
}

/// Mesh status for the dashboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshStatusInfo {
    pub transport_active: bool,
    pub peer_count: u32,
    pub link_count: u32,
    pub interface_count: u32,
    pub propagation_enabled: bool,
    pub uptime: u64,
    pub version: String,
}

/// Stable primary route in the operator console.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AppRoute {
    #[default]
    Command,
    Network,
    Messages,
    Fleet,
    Propagation,
    Content,
    Lab,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ActivityEntry {
    pub timestamp: i64,
    pub severity: ActivitySeverity,
    pub kind: &'static str,
    pub summary: String,
    pub entity: Option<String>,
    pub correlation_id: Option<String>,
    pub provenance: String,
}

/// State for the page browser.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    pub host: String,
    pub path: String,
    pub authoritative: Option<styrene_ipc::types::PageContent>,
    pub loading: bool,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub retryable: bool,
}

impl PageView {
    pub fn loading(host: String, path: String) -> Self {
        Self {
            host,
            path,
            authoritative: None,
            loading: true,
            error: None,
            elapsed_ms: None,
            retryable: false,
        }
    }

    pub fn from_daemon(mut page: styrene_ipc::types::PageContent) -> Self {
        if let Some(failure) = &mut page.failure {
            failure.message = "daemon page request failed".into();
        }
        for warning in &mut page.parser_warnings {
            warning.message = "page parser warning".into();
        }
        let host = page.host_hash.clone();
        let path = page.request.native_path.clone();
        let error = page.failure.as_ref().map(|failure| failure.message.clone());
        let retryable = page.failure.as_ref().is_some_and(|failure| failure.retryable);
        let elapsed_ms = page.elapsed_ms;
        Self { host, path, authoritative: Some(page), loading: false, error, elapsed_ms, retryable }
    }

    pub fn failed_at(host: String, path: String, message: String, elapsed_ms: u64) -> Self {
        let mut page = Self::loading(host, path);
        page.loading = false;
        tracing::warn!(target: "dx::content", error_bytes = message.len(), "page request failed before daemon evidence");
        page.error = Some("page request failed before daemon evidence".into());
        page.elapsed_ms = Some(elapsed_ms);
        // An IPC failure has no daemon terminal observation from which to infer retryability.
        page.retryable = false;
        page
    }
}

/// A node in the network graph visualization.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub node_type: GraphNodeType,
    /// Capabilities from announce (shown in tooltip).
    pub capabilities: Vec<String>,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
}

/// Node type for the graph — determines shape, color, size, and click behavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphNodeType {
    /// The local node (center of the graph).
    Local,
    /// A transport interface (TCP, UDP, Serial).
    Interface { online: bool },
    /// Vanilla RNS peer.
    Rns { online: bool },
    /// Styrene peer (has Styrene protocol extensions).
    Styrene { online: bool },
    /// Styrene hub (relay, propagation, store-and-forward).
    Hub { online: bool },
    /// NomadNet-compatible page host.
    PageHost { online: bool },
}

impl GraphNodeType {
    pub fn is_online(&self) -> bool {
        match self {
            Self::Local => true,
            Self::Interface { online }
            | Self::Rns { online }
            | Self::Styrene { online }
            | Self::Hub { online }
            | Self::PageHost { online } => *online,
        }
    }
}

impl GraphNode {
    pub fn color(&self) -> &'static str {
        match &self.node_type {
            GraphNodeType::Local => "#58a6ff",                       // blue
            GraphNodeType::Interface { online: true } => "#3fb950",  // green
            GraphNodeType::Interface { online: false } => "#f85149", // red
            GraphNodeType::Hub { online: true } => "#bc8cff",        // purple
            GraphNodeType::Hub { online: false } => "#6e40aa",       // dim purple
            GraphNodeType::PageHost { online: true } => "#39d2c0",   // teal
            GraphNodeType::PageHost { online: false } => "#1a7f72",  // dim teal
            GraphNodeType::Styrene { online: true } => "#3fb950",    // green
            GraphNodeType::Styrene { online: false } => "#2ea043",   // dim green
            GraphNodeType::Rns { online: true } => "#d29922",        // amber
            GraphNodeType::Rns { online: false } => "#484f58",       // grey
        }
    }

    /// Base radius — small enough that 400 nodes don't overlap.
    pub fn radius(&self) -> f64 {
        match self.node_type {
            GraphNodeType::Local => 12.0,
            GraphNodeType::Interface { .. } => 8.0,
            GraphNodeType::Hub { .. } => 10.0,
            GraphNodeType::PageHost { .. } => 7.0,
            GraphNodeType::Styrene { .. } => 6.0,
            GraphNodeType::Rns { .. } => 5.0,
        }
    }

    pub fn border_color(&self) -> &'static str {
        match &self.node_type {
            GraphNodeType::Local => "#79c0ff",
            GraphNodeType::Interface { online: true } => "#56d364",
            GraphNodeType::Interface { online: false } => "#f85149",
            GraphNodeType::Hub { online: true } => "#d2a8ff",
            GraphNodeType::Hub { online: false } => "#6e40aa",
            GraphNodeType::PageHost { online: true } => "#56ead6",
            GraphNodeType::PageHost { online: false } => "#1a7f72",
            GraphNodeType::Styrene { online: true } => "#56d364",
            GraphNodeType::Styrene { online: false } => "#2ea043",
            GraphNodeType::Rns { online: true } => "#e3b341",
            GraphNodeType::Rns { online: false } => "#30363d",
        }
    }

    pub fn type_label(&self) -> &'static str {
        match &self.node_type {
            GraphNodeType::Local => "Local Node",
            GraphNodeType::Interface { online: true } => "Interface (connected)",
            GraphNodeType::Interface { online: false } => "Interface (disconnected)",
            GraphNodeType::Hub { online: true } => "Hub (online)",
            GraphNodeType::Hub { online: false } => "Hub (offline)",
            GraphNodeType::PageHost { online: true } => "Page Host (online)",
            GraphNodeType::PageHost { online: false } => "Page Host (offline)",
            GraphNodeType::Styrene { online: true } => "Styrene (online)",
            GraphNodeType::Styrene { online: false } => "Styrene (offline)",
            GraphNodeType::Rns { online: true } => "RNS Peer (online)",
            GraphNodeType::Rns { online: false } => "RNS Peer (offline)",
        }
    }

    /// Whether this node has a real display name (not just a hash prefix).
    pub fn name_is_set(&self) -> bool {
        // Hash-only labels are exactly 8 hex chars
        self.label.len() != 8 || self.label.chars().any(|c| !c.is_ascii_hexdigit())
    }

    /// SVG shape for this node type. Returns None for circle (default).
    /// Some(path_d) for custom shapes (diamond, hexagon, etc.).
    pub fn shape_path(&self, cx: f64, cy: f64) -> Option<String> {
        let r = self.radius();
        match &self.node_type {
            // Hexagon for local node
            GraphNodeType::Local => {
                let mut d = String::new();
                for i in 0..6 {
                    let angle =
                        std::f64::consts::FRAC_PI_3 * i as f64 - std::f64::consts::FRAC_PI_6;
                    let hx = cx + r * angle.cos();
                    let hy = cy + r * angle.sin();
                    if i == 0 {
                        d.push_str(&format!("M{hx},{hy}"));
                    } else {
                        d.push_str(&format!("L{hx},{hy}"));
                    }
                }
                d.push('Z');
                Some(d)
            }
            // Diamond for hub
            GraphNodeType::Hub { .. } => {
                let d = format!(
                    "M{},{} L{},{} L{},{} L{},{} Z",
                    cx,
                    cy - r, // top
                    cx + r,
                    cy, // right
                    cx,
                    cy + r, // bottom
                    cx - r,
                    cy, // left
                );
                Some(d)
            }
            // Rounded square for page hosts
            GraphNodeType::PageHost { .. } => {
                let half = r * 0.8;
                let cr = r * 0.25; // corner radius
                let d = format!(
                    "M{},{} L{},{} Q{},{} {},{} L{},{} Q{},{} {},{} L{},{} Q{},{} {},{} L{},{} Q{},{} {},{} Z",
                    cx - half + cr,
                    cy - half, // top-left after corner
                    cx + half - cr,
                    cy - half, // top-right before corner
                    cx + half,
                    cy - half,
                    cx + half,
                    cy - half + cr, // top-right corner
                    cx + half,
                    cy + half - cr, // bottom-right before corner
                    cx + half,
                    cy + half,
                    cx + half - cr,
                    cy + half, // bottom-right corner
                    cx - half + cr,
                    cy + half, // bottom-left before corner
                    cx - half,
                    cy + half,
                    cx - half,
                    cy + half - cr, // bottom-left corner
                    cx - half,
                    cy - half + cr, // top-left before corner
                    cx - half,
                    cy - half,
                    cx - half + cr,
                    cy - half, // top-left corner
                );
                Some(d)
            }
            // Small square for interfaces
            GraphNodeType::Interface { .. } => {
                let half = r * 0.85;
                let d = format!(
                    "M{},{} L{},{} L{},{} L{},{} Z",
                    cx - half,
                    cy - half,
                    cx + half,
                    cy - half,
                    cx + half,
                    cy + half,
                    cx - half,
                    cy + half,
                );
                Some(d)
            }
            // Circle for everything else
            _ => None,
        }
    }
}

/// An edge in the network graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub source: usize,
    pub target: usize,
    /// Hop count for this edge (0 = direct/unknown).
    pub hops: u8,
    pub kind: GraphEdgeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphEdgeKind {
    Interface,
    Route,
    Link,
    Association,
}

/// A path table entry for graph topology construction.
#[derive(Clone, Debug, PartialEq)]
pub struct PathEntry {
    pub destination_hash: String,
    pub hops: u8,
    pub next_hop: String,
    pub interface: String,
    pub expires: Option<i64>,
    pub observation: styrene_ipc::types::ObservationMetadata,
}

/// An announce event for the activity stream.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnounceEvent {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub timestamp: i64,
    pub node_role: PeerRole,
}

/// Exact typed interface projection retained for operator inspection.
pub type InterfaceInfo = styrene_ipc::types::InterfaceDetail;

/// An active link with telemetry.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkInfo {
    pub link_id: String,
    pub peer_hash: String,
    pub status: String,
    pub activity: styrene_ipc::types::LinkActivity,
    pub rtt_ms: Option<f64>,
    pub timestamp: i64,
    pub observation: styrene_ipc::types::ObservationMetadata,
}

/// A conversation summary for the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversationEntry {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message: Option<String>,
    pub last_timestamp: Option<i64>,
    pub unread_count: u32,
    pub message_count: u32,
    pub pinned: bool,
    pub muted: bool,
}

/// A chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub content: String,
    pub timestamp: i64,
    pub is_outgoing: bool,
    /// Delivery status: "pending", "delivered", "read", "failed", or empty.
    pub status: String,
    pub lifecycle: MessageLifecycle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageLifecycle {
    pub state: styrene_ipc::types::MessageLifecycleState,
    pub terminal_detail: Option<String>,
    pub requested_method: Option<String>,
    pub actual_method: Option<String>,
    pub fallback_reason: Option<String>,
    pub correlation_id: Option<String>,
    pub attempts: Vec<styrene_ipc::types::MessageAttemptInfo>,
    pub authentication: styrene_ipc::types::MessageAuthenticationState,
    pub stamp_state: styrene_ipc::types::MessageStampState,
    pub stamp_value: Option<u32>,
    pub stamp_cost: Option<u32>,
    pub evidence: Vec<styrene_ipc::types::MessageDeliveryEvidenceInfo>,
    pub attachments: Vec<styrene_ipc::types::AttachmentInfo>,
    pub propagation: Vec<styrene_ipc::types::MessagePropagationCorrelationInfo>,
}

impl From<styrene_ipc::types::MessageInfo> for ChatMessage {
    fn from(message: styrene_ipc::types::MessageInfo) -> Self {
        Self {
            id: message.id,
            source: message.source_hash,
            destination: message.destination_hash,
            content: message.content,
            timestamp: message.timestamp,
            is_outgoing: message.is_outgoing,
            status: message.status,
            lifecycle: MessageLifecycle {
                state: message.lifecycle_state,
                terminal_detail: message.terminal_detail,
                requested_method: message.requested_delivery_method,
                actual_method: message.actual_delivery_method,
                fallback_reason: message.fallback_reason,
                correlation_id: message.correlation_id,
                attempts: message.attempts,
                authentication: message.authentication_state,
                stamp_state: message.stamp_state,
                stamp_value: message.stamp_value,
                stamp_cost: message.stamp_cost,
                evidence: message.delivery_evidence,
                attachments: message.attachments,
                propagation: message.propagation_correlations,
            },
        }
    }
}

impl ChatMessage {
    #[cfg(test)]
    pub fn apply_status(&mut self, status: String) {
        self.status = status;
    }
}

// ── Styrene announce name parser ──────────────────────────────────────────

/// Parsed result from a Styrene-format display name.
#[derive(Clone, Debug, Default)]
pub struct ParsedAnnounceName {
    pub display_name: String,
    pub is_styrene: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub role: PeerRole,
}

/// Parse a raw announce name, handling both Styrene format and plain RNS names.
///
/// Styrene format: `"styrene:DISPLAY_NAME:VERSION:CAP1,CAP2,...:EXTRA:EXTRA:PLATFORM|N"`
/// Plain RNS format: `"Display Name"` (just a string)
pub fn parse_announce_name(raw: &str) -> ParsedAnnounceName {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedAnnounceName::default();
    }

    // Try to extract name from JSON service descriptors (e.g. RiftAgent)
    // Format: {"name": "hostname-riftagent", "services": [...]}
    if trimmed.starts_with('{') {
        let name = extract_json_name(trimmed);
        return ParsedAnnounceName { display_name: name, ..Default::default() };
    }

    // Check for Styrene prefix
    if !trimmed.starts_with("styrene:") {
        // Truncate plain names to something reasonable
        let display =
            if trimmed.len() > 32 { format!("{}...", &trimmed[..32]) } else { trimmed.to_string() };
        return ParsedAnnounceName { display_name: display, ..Default::default() };
    }

    // Split on ':'  — styrene:NAME:VERSION:CAPS:...:PLATFORM|N
    let parts: Vec<&str> = trimmed.splitn(8, ':').collect();

    let display_name = parts
        .get(1)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("Styrene Node")
        .to_string();

    let version = parts.get(2).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let capabilities: Vec<String> = parts
        .get(3)
        .map(|s| {
            s.split(',').map(|c| c.trim().to_ascii_lowercase()).filter(|c| !c.is_empty()).collect()
        })
        .unwrap_or_default();

    let role = if capabilities.iter().any(|c| c == "hub") {
        PeerRole::Hub
    } else if capabilities.iter().any(|c| c == "pages") {
        PeerRole::PageHost
    } else {
        PeerRole::Styrene
    };

    ParsedAnnounceName { display_name, is_styrene: true, version, capabilities, role }
}

/// Extract the "name" field from a JSON service descriptor.
/// Handles truncated JSON gracefully (announce data is often cut short).
fn extract_json_name(s: &str) -> String {
    // Simple substring extraction — avoid pulling in serde_json for this
    if let Some(start) = s.find("\"name\"") {
        let after_key = &s[start + 6..];
        // Skip whitespace and colon
        let after_colon = after_key.trim_start().strip_prefix(':').unwrap_or(after_key);
        let after_ws = after_colon.trim_start();
        // Extract quoted value
        if let Some(rest) = after_ws.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return rest[..end].to_string();
            }
            // Truncated — take what we have
            let truncated = &rest[..rest.len().min(32)];
            return truncated.to_string();
        }
    }
    // Fallback: truncate the raw string
    if s.len() > 24 { format!("{}...", &s[..24]) } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_lifecycle_preserves_authoritative_ipc_details() {
        let mut info = styrene_ipc::types::MessageInfo::default();
        info.id = "message".into();
        info.status = "delivered".into();
        info.requested_delivery_method = Some("opportunistic".into());
        info.actual_delivery_method = Some("direct".into());
        info.fallback_reason = Some("packet limit".into());
        info.correlation_id = Some("send-1".into());
        let mut attempt = styrene_ipc::types::MessageAttemptInfo::default();
        attempt.message_id = "message".into();
        attempt.number = 2;
        attempt.started_unix_ms = 100;
        attempt.deadline_unix_ms = 200;
        attempt.state = "delivered".into();
        info.attempts.push(attempt);
        info.read = true;
        let mut attachment = styrene_ipc::types::AttachmentInfo::default();
        attachment.name = "evidence.bin".into();
        attachment.size = 2048;
        info.attachments.push(attachment);

        let message = ChatMessage::from(info);
        assert_eq!(message.lifecycle.requested_method.as_deref(), Some("opportunistic"));
        assert_eq!(message.lifecycle.actual_method.as_deref(), Some("direct"));
        assert_eq!(message.lifecycle.fallback_reason.as_deref(), Some("packet limit"));
        assert_eq!(message.lifecycle.correlation_id.as_deref(), Some("send-1"));
        assert_eq!(message.lifecycle.attempts.len(), 1);
        assert_eq!(message.lifecycle.attempts[0].message_id, "message");
        assert_eq!(message.lifecycle.attempts[0].number, 2);
        assert_eq!(message.lifecycle.attempts[0].started_unix_ms, 100);
        assert_eq!(message.lifecycle.attempts[0].deadline_unix_ms, 200);
        assert_eq!(message.lifecycle.attempts[0].state, "delivered");
        assert!(message.lifecycle.propagation.is_empty());
        assert_eq!(message.lifecycle.attachments[0].size, 2048);
    }

    #[test]
    fn terminal_message_failure_updates_one_lifecycle() {
        let mut message = ChatMessage::from(styrene_ipc::types::MessageInfo::default());
        message.apply_status("failed".into());
        assert_eq!(message.status, "failed");
        assert!(message.lifecycle.terminal_detail.is_none());
    }

    #[test]
    fn page_failure_names_stage_and_leaves_loading_state() {
        let page =
            PageView::failed_at("peer".into(), "/".into(), "identity unavailable".into(), 42);
        assert!(!page.loading);
        assert!(!page.retryable);
        assert_eq!(page.elapsed_ms, Some(42));
        assert!(page.authoritative.is_none(), "client must not fabricate daemon stages");
    }

    #[test]
    fn completed_page_records_transfer_size_and_ui_stages() {
        let mut content = styrene_ipc::types::PageContent::default();
        content.source_bytes = b"# Page".to_vec();
        content.rendered_text = "Page".into();
        content.fetched_at = 10;
        content.transfer.received_bytes = 6;
        content.correlation_id = "page-1".into();
        content.elapsed_ms = Some(12);
        content.outcome = styrene_ipc::types::PageBrowseOutcome::Succeeded;
        let mut failure = styrene_ipc::types::PageBrowseFailure::default();
        failure.message = "token=structured-secret /private/key".into();
        content.failure = Some(failure);
        let mut warning = styrene_ipc::types::PageParserWarning::default();
        warning.message = "password=structured-secret".into();
        content.parser_warnings.push(warning);
        let mut stage = styrene_ipc::types::PageBrowseStage::default();
        stage.correlation_id = "page-1".into();
        stage.kind = styrene_ipc::types::PageBrowseStageKind::Transfer;
        stage.state = styrene_ipc::types::PageBrowseStageState::Succeeded;
        content.stages.push(stage);
        let page = PageView::from_daemon(content);
        let retained = page.authoritative.expect("authoritative daemon page");
        assert_eq!(retained.source_bytes, b"# Page");
        assert_eq!(retained.fetched_at, 10);
        assert_eq!(retained.stages.len(), 1);
        let rendered = format!("{retained:?}");
        assert!(!rendered.contains("structured-secret"));
        assert!(!rendered.contains("/private/key"));
    }
}
