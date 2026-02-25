use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Type aliases ──────────────────────────────────────────────────────────────

/// Hex-encoded message identifier.
pub type MessageId = String;

/// Hex-encoded peer/destination hash.
pub type PeerHash = String;

/// Terminal session identifier.
pub type SessionId = String;

// ── Device discovery ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
}

// ── Identity ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct IdentityInfo {
    pub identity_hash: String,
    pub destination_hash: String,
    pub lxmf_destination_hash: String,
    pub display_name: String,
    pub icon: Option<String>,
    pub short_name: Option<String>,
}

// ── Daemon status ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct DaemonStatusInfo {
    pub uptime: u64,
    pub daemon_version: String,
    pub rns_initialized: bool,
    pub lxmf_initialized: bool,
    pub device_count: u32,
    pub interface_count: u32,
    pub hub_status: Option<String>,
    pub propagation_enabled: bool,
    pub transport_enabled: bool,
    pub active_links: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct InterfaceInfo {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConfigSnapshot {
    pub values: BTreeMap<String, serde_json::Value>,
}

// ── Messaging ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct MessageInfo {
    pub id: String,
    pub source_hash: String,
    pub destination_hash: String,
    pub timestamp: i64,
    pub content: String,
    pub title: Option<String>,
    pub status: String,
    pub is_outgoing: bool,
    pub delivery_method: Option<String>,
    pub read: bool,
    pub attachment_info: Option<AttachmentInfo>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct AttachmentInfo {
    pub name: String,
    pub content_type: String,
    pub size: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConversationInfo {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message_timestamp: Option<i64>,
    pub last_message_content: Option<String>,
    pub unread_count: u32,
    pub message_count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ContactInfo {
    pub peer_hash: String,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SendChatRequest {
    pub peer_hash: String,
    pub content: String,
    pub title: Option<String>,
    pub delivery_method: Option<String>,
    pub reply_to_hash: Option<String>,
    pub attachment: Option<Vec<u8>>,
    pub attachment_name: Option<String>,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct PathInfo {
    pub destination_hash: String,
    pub hops: Option<u32>,
    pub next_hop: Option<String>,
    pub interface: Option<String>,
    pub expires: Option<i64>,
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

// ── Events ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
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
