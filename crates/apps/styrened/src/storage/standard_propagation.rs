use super::messages::MessagesStore;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const STANDARD_PROPAGATION_MIGRATION: &str = "2026-08-24-standard-lxmf-propagation-v10";
pub const STANDARD_PROPAGATION_OBSERVATION_MIGRATION: &str =
    "2026-08-25-standard-lxmf-propagation-observations-v11";
pub const STANDARD_PROPAGATION_CORRELATION_MIGRATION: &str =
    "2026-08-25-standard-lxmf-propagation-correlation-v12";
pub const TOMBSTONE_RETENTION_SECS: i64 = 180 * 24 * 60 * 60;
const FAILURE_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;
const ATTEMPT_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;
const MAX_FAILURES: usize = 4096;
const MAX_ATTEMPTS: usize = 4096;
const MAX_FAILURE_DETAIL: usize = 256;
type ExistingMaterializedJob = (String, Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

#[derive(Clone, Copy, Debug)]
pub struct StandardPropagationPolicy {
    pub queue_max_count: usize,
    pub queue_max_bytes: usize,
    pub expiry_secs: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationStats {
    pub queued_count: usize,
    pub stored_bytes: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StandardPropagationItem {
    pub transient_id: [u8; 32],
    pub destination: [u8; 16],
    pub lxmf_data: Vec<u8>,
    pub stamp: [u8; 32],
    pub stamp_value: u32,
    pub received_at: i64,
    pub expires_at: i64,
    pub stored_size: usize,
}

impl fmt::Debug for StandardPropagationItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StandardPropagationItem")
            .field("transient_id", &self.transient_id)
            .field("destination", &self.destination)
            .field("lxmf_data_len", &self.lxmf_data.len())
            .field("stamp", &"[REDACTED]")
            .field("stamp_value", &self.stamp_value)
            .field("received_at", &self.received_at)
            .field("expires_at", &self.expires_at)
            .field("stored_size", &self.stored_size)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationIngestOutcome {
    Accepted,
    CapacityRejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationAttemptStatus {
    Untracked,
    Partial([u8; 16]),
    Complete([u8; 16]),
}

impl StandardPropagationAttemptStatus {
    fn attempt_id(self) -> Option<[u8; 16]> {
        match self {
            Self::Untracked => None,
            Self::Partial(attempt_id) | Self::Complete(attempt_id) => Some(attempt_id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationProtocolStatus {
    Valid,
    Invalid,
}

pub struct StandardPropagationIngestRequest<'a> {
    pub items: &'a [StandardPropagationItem],
    pub source_peer: Option<[u8; 16]>,
    pub attempt: StandardPropagationAttemptStatus,
    pub protocol: StandardPropagationProtocolStatus,
    pub now: i64,
    pub policy: StandardPropagationPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationOfferComparison {
    pub wanted: Vec<[u8; 32]>,
    pub attempt_id: [u8; 16],
    pub capacity_rejected: bool,
}

pub struct StandardPropagationOfferRequest<'a> {
    pub peer: [u8; 16],
    pub offered: &'a [[u8; 32]],
    pub same_link_pending: &'a BTreeSet<[u8; 32]>,
    pub pending_elsewhere: &'a BTreeSet<[u8; 32]>,
    pub pending_count: usize,
    pub existing_attempt: Option<[u8; 16]>,
    pub request_id: [u8; 16],
    pub link_id: [u8; 16],
    pub now: i64,
    pub deadline: i64,
    pub policy: StandardPropagationPolicy,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StandardPropagationGetResult {
    pub inventory: Option<Vec<[u8; 32]>>,
    pub payloads: Vec<Vec<u8>>,
    pub attempt_id: [u8; 16],
}

impl fmt::Debug for StandardPropagationGetResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let payload_bytes = self.payloads.iter().map(Vec::len).sum::<usize>();
        formatter
            .debug_struct("StandardPropagationGetResult")
            .field("inventory_count", &self.inventory.as_ref().map(Vec::len))
            .field("payload_count", &self.payloads.len())
            .field("payload_bytes", &payload_bytes)
            .field("attempt_id", &self.attempt_id)
            .finish()
    }
}

pub struct StandardPropagationGetRequest<'a> {
    pub peer: [u8; 16],
    pub request_id: [u8; 16],
    pub recipient: [u8; 16],
    pub wants: Option<&'a [[u8; 32]]>,
    pub haves: Option<&'a [[u8; 32]]>,
    pub inventory: bool,
    pub response_limit: usize,
    pub now: i64,
    pub policy: StandardPropagationPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardPropagationGetOperation {
    Fetch,
    Download,
    Sync,
}

impl StandardPropagationGetOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Download => "download",
            Self::Sync => "sync",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationSelection {
    pub peer: Option<[u8; 16]>,
    pub mode: String,
    pub selected_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationPeer {
    pub identity_hash: [u8; 16],
    pub propagation_destination: Option<[u8; 16]>,
    pub configured: bool,
    pub enabled: bool,
    pub transfer_limit_kb: Option<usize>,
    pub sync_limit_kb: Option<usize>,
    pub stamp_cost: Option<u32>,
    pub stamp_flexibility: Option<u32>,
    pub peering_cost: Option<u32>,
    pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationCheckpoint {
    pub peer: [u8; 16],
    pub direction: String,
    pub completed_stage: String,
    pub cursor: Option<Vec<u8>>,
    pub digest: Option<[u8; 32]>,
    pub item_count: usize,
    pub byte_count: usize,
    pub last_attempt: Option<[u8; 16]>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationFailure {
    pub code: String,
    pub detail: Option<String>,
    pub occurred_at: i64,
    pub peer: Option<[u8; 16]>,
    pub transient_id: Option<[u8; 32]>,
    pub attempt_id: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationMessageLink {
    pub message_id: String,
    pub transient_id: [u8; 32],
    pub relation: String,
    pub attempt_id: Option<[u8; 16]>,
    pub peer: Option<[u8; 16]>,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StandardPropagationClientJob {
    pub message_id: String,
    pub transient_id: Option<[u8; 32]>,
    pub destination: [u8; 16],
    pub canonical_wire: Option<Vec<u8>>,
    pub lxmf_data: Option<Vec<u8>>,
    pub stamp: Option<[u8; 32]>,
    pub peer: [u8; 16],
    pub propagation_destination: [u8; 16],
    pub stamp_cost: u32,
    pub peering_cost: u32,
    pub correlation_id: [u8; 16],
    pub attempt_id: [u8; 16],
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl fmt::Debug for StandardPropagationClientJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StandardPropagationClientJob")
            .field("message_id", &self.message_id)
            .field("transient_id", &self.transient_id)
            .field("destination", &self.destination)
            .field("canonical_wire_len", &self.canonical_wire.as_ref().map(Vec::len))
            .field("lxmf_data_len", &self.lxmf_data.as_ref().map(Vec::len))
            .field("stamp", &self.stamp.as_ref().map(|_| "[REDACTED]"))
            .field("peer", &self.peer)
            .field("propagation_destination", &self.propagation_destination)
            .field("stamp_cost", &self.stamp_cost)
            .field("peering_cost", &self.peering_cost)
            .field("correlation_id", &self.correlation_id)
            .field("attempt_id", &self.attempt_id)
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

pub const STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT: usize = 128;
pub const STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT: usize = 256;
pub const STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT: usize = 128;
pub const STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StandardPropagationQueueObservation {
    pub queued_count: usize,
    pub queued_bytes: usize,
    pub acknowledged_count: usize,
    pub expired_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationPeerObservation {
    pub identity_hash: [u8; 16],
    pub propagation_destination: Option<[u8; 16]>,
    pub configured: bool,
    pub enabled: bool,
    pub transfer_limit_kb: Option<usize>,
    pub sync_limit_kb: Option<usize>,
    pub stamp_cost: Option<u32>,
    pub stamp_flexibility: Option<u32>,
    pub peering_cost: Option<u32>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub retry_at: Option<i64>,
    pub backoff_count: usize,
    pub offered_count: usize,
    pub wanted_count: usize,
    pub accepted_count: usize,
    pub accepted_bytes: usize,
    pub failure_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationAttemptObservation {
    pub attempt_id: [u8; 16],
    pub correlation_id: [u8; 16],
    pub peer: Option<[u8; 16]>,
    pub direction: String,
    pub stage: String,
    pub state: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub deadline_at: Option<i64>,
    pub offered_count: usize,
    pub wanted_count: usize,
    pub accepted_count: usize,
    pub accepted_bytes: usize,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationCheckpointObservation {
    pub peer: [u8; 16],
    pub direction: String,
    pub completed_stage: String,
    pub item_count: usize,
    pub byte_count: usize,
    pub last_attempt: Option<[u8; 16]>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardPropagationFailureObservation {
    pub code: String,
    pub occurred_at: i64,
    pub peer: Option<[u8; 16]>,
    pub attempt_id: Option<[u8; 16]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StandardPropagationObservation {
    pub observed_at: i64,
    pub queue: StandardPropagationQueueObservation,
    pub selection: Option<StandardPropagationSelection>,
    pub peers: Vec<StandardPropagationPeerObservation>,
    pub attempts: Vec<StandardPropagationAttemptObservation>,
    pub checkpoints: Vec<StandardPropagationCheckpointObservation>,
    pub failures: Vec<StandardPropagationFailureObservation>,
    pub peers_truncated: bool,
    pub attempts_truncated: bool,
    pub checkpoints_truncated: bool,
    pub failures_truncated: bool,
}

fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

fn to_i64(value: usize, field: &str) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| invalid(field))
}

fn to_usize(value: i64, field: &str) -> rusqlite::Result<usize> {
    usize::try_from(value).map_err(|_| invalid(field))
}

fn blob_array<const N: usize>(value: Vec<u8>, field: &str) -> rusqlite::Result<[u8; N]> {
    value.try_into().map_err(|_| invalid(field))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ColumnSpec {
    name: &'static str,
    data_type: &'static str,
    not_null: i64,
    primary_key: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct ColumnMetadata {
    name: String,
    data_type: String,
    not_null: i64,
    primary_key: i64,
}

#[derive(Clone, Copy)]
struct TableSpec {
    name: &'static str,
    columns: &'static [ColumnSpec],
    create_sql: &'static str,
}

macro_rules! column {
    ($name:literal, $type:literal, $not_null:literal, $pk:literal) => {
        ColumnSpec { name: $name, data_type: $type, not_null: $not_null, primary_key: $pk }
    };
}

const ITEM_COLUMNS: &[ColumnSpec] = &[
    column!("transient_id", "BLOB", 1, 1),
    column!("destination", "BLOB", 1, 0),
    column!("lxmf_data", "BLOB", 0, 0),
    column!("stamp", "BLOB", 0, 0),
    column!("stamp_value", "INTEGER", 1, 0),
    column!("received_at", "INTEGER", 1, 0),
    column!("expires_at", "INTEGER", 1, 0),
    column!("stored_size", "INTEGER", 1, 0),
    column!("state", "TEXT", 1, 0),
    column!("terminal_at", "INTEGER", 0, 0),
];
const PEER_COLUMNS: &[ColumnSpec] = &[
    column!("identity_hash", "BLOB", 1, 1),
    column!("propagation_destination", "BLOB", 0, 0),
    column!("origin", "TEXT", 1, 0),
    column!("enabled", "INTEGER", 1, 0),
    column!("transfer_limit_kb", "INTEGER", 0, 0),
    column!("sync_limit_kb", "INTEGER", 0, 0),
    column!("stamp_cost", "INTEGER", 0, 0),
    column!("stamp_flexibility", "INTEGER", 0, 0),
    column!("peering_cost", "INTEGER", 0, 0),
    column!("first_seen_at", "INTEGER", 1, 0),
    column!("last_seen_at", "INTEGER", 1, 0),
    column!("retry_at", "INTEGER", 0, 0),
    column!("backoff_count", "INTEGER", 1, 0),
    column!("offered_count", "INTEGER", 1, 0),
    column!("wanted_count", "INTEGER", 1, 0),
    column!("accepted_count", "INTEGER", 1, 0),
    column!("accepted_bytes", "INTEGER", 1, 0),
    column!("failure_count", "INTEGER", 1, 0),
];
const PEER_ITEM_COLUMNS: &[ColumnSpec] = &[
    column!("peer", "BLOB", 1, 1),
    column!("transient_id", "BLOB", 1, 2),
    column!("disposition", "TEXT", 1, 0),
    column!("updated_at", "INTEGER", 1, 0),
];
const SELECTION_COLUMNS: &[ColumnSpec] = &[
    column!("singleton", "INTEGER", 0, 1),
    column!("selected_peer", "BLOB", 0, 0),
    column!("mode", "TEXT", 1, 0),
    column!("selected_at", "INTEGER", 1, 0),
];
const ATTEMPT_COLUMNS: &[ColumnSpec] = &[
    column!("attempt_id", "BLOB", 1, 1),
    column!("correlation_id", "BLOB", 1, 0),
    column!("peer", "BLOB", 0, 0),
    column!("direction", "TEXT", 1, 0),
    column!("stage", "TEXT", 1, 0),
    column!("state", "TEXT", 1, 0),
    column!("started_at", "INTEGER", 1, 0),
    column!("updated_at", "INTEGER", 1, 0),
    column!("deadline_at", "INTEGER", 0, 0),
    column!("offered_count", "INTEGER", 1, 0),
    column!("wanted_count", "INTEGER", 1, 0),
    column!("accepted_count", "INTEGER", 1, 0),
    column!("accepted_bytes", "INTEGER", 1, 0),
    column!("failure_code", "TEXT", 0, 0),
    column!("failure_detail", "TEXT", 0, 0),
];
const ATTEMPT_ITEM_COLUMNS: &[ColumnSpec] = &[
    column!("attempt_id", "BLOB", 1, 1),
    column!("transient_id", "BLOB", 1, 2),
    column!("role", "TEXT", 1, 3),
];
const CHECKPOINT_COLUMNS: &[ColumnSpec] = &[
    column!("peer", "BLOB", 1, 1),
    column!("direction", "TEXT", 1, 2),
    column!("completed_stage", "TEXT", 1, 0),
    column!("cursor", "BLOB", 0, 0),
    column!("digest", "BLOB", 0, 0),
    column!("item_count", "INTEGER", 1, 0),
    column!("byte_count", "INTEGER", 1, 0),
    column!("last_attempt", "BLOB", 0, 0),
    column!("updated_at", "INTEGER", 1, 0),
];
const FAILURE_COLUMNS: &[ColumnSpec] = &[
    column!("failure_id", "INTEGER", 0, 1),
    column!("code", "TEXT", 1, 0),
    column!("detail", "TEXT", 0, 0),
    column!("occurred_at", "INTEGER", 1, 0),
    column!("peer", "BLOB", 0, 0),
    column!("transient_id", "BLOB", 0, 0),
    column!("attempt_id", "BLOB", 0, 0),
];

const ITEMS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_items (
    transient_id BLOB PRIMARY KEY CHECK(typeof(transient_id) = 'blob' AND length(transient_id) = 32),
    destination BLOB NOT NULL CHECK(typeof(destination) = 'blob' AND length(destination) = 16),
    lxmf_data BLOB CHECK(lxmf_data IS NULL OR (typeof(lxmf_data) = 'blob' AND length(lxmf_data) <= 4000000)),
    stamp BLOB CHECK(stamp IS NULL OR (typeof(stamp) = 'blob' AND length(stamp) = 32)),
    stamp_value INTEGER NOT NULL CHECK(typeof(stamp_value) = 'integer' AND stamp_value BETWEEN 0 AND 256),
    received_at INTEGER NOT NULL CHECK(typeof(received_at) = 'integer' AND received_at >= 0),
    expires_at INTEGER NOT NULL CHECK(typeof(expires_at) = 'integer' AND expires_at >= received_at),
    stored_size INTEGER NOT NULL CHECK(typeof(stored_size) = 'integer' AND stored_size >= 0 AND stored_size <= 4000000),
    state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('queued','acknowledged','expired')),
    terminal_at INTEGER CHECK(terminal_at IS NULL OR (typeof(terminal_at) = 'integer' AND terminal_at >= received_at)),
    CHECK((state = 'queued' AND lxmf_data IS NOT NULL AND stamp IS NOT NULL AND terminal_at IS NULL
           AND stored_size = length(lxmf_data) + length(stamp))
       OR (state IN ('acknowledged','expired') AND lxmf_data IS NULL AND stamp IS NULL
           AND stored_size = 0 AND terminal_at IS NOT NULL))
) STRICT";

const PEERS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_peers (
    identity_hash BLOB PRIMARY KEY CHECK(typeof(identity_hash) = 'blob' AND length(identity_hash) = 16),
    propagation_destination BLOB UNIQUE CHECK(propagation_destination IS NULL OR (typeof(propagation_destination) = 'blob' AND length(propagation_destination) = 16)),
    origin TEXT NOT NULL CHECK(typeof(origin) = 'text' AND origin IN ('configured','observed','both')),
    enabled INTEGER NOT NULL CHECK(typeof(enabled) = 'integer' AND enabled IN (0,1)),
    transfer_limit_kb INTEGER CHECK(transfer_limit_kb IS NULL OR (typeof(transfer_limit_kb) = 'integer' AND transfer_limit_kb >= 0)),
    sync_limit_kb INTEGER CHECK(sync_limit_kb IS NULL OR (typeof(sync_limit_kb) = 'integer' AND sync_limit_kb >= 0)),
    stamp_cost INTEGER CHECK(stamp_cost IS NULL OR (typeof(stamp_cost) = 'integer' AND stamp_cost BETWEEN 0 AND 254)),
    stamp_flexibility INTEGER CHECK(stamp_flexibility IS NULL OR (typeof(stamp_flexibility) = 'integer' AND stamp_flexibility BETWEEN 0 AND 254)),
    peering_cost INTEGER CHECK(peering_cost IS NULL OR (typeof(peering_cost) = 'integer' AND peering_cost BETWEEN 0 AND 254)),
    first_seen_at INTEGER NOT NULL CHECK(typeof(first_seen_at) = 'integer' AND first_seen_at >= 0),
    last_seen_at INTEGER NOT NULL CHECK(typeof(last_seen_at) = 'integer' AND last_seen_at >= first_seen_at),
    retry_at INTEGER CHECK(retry_at IS NULL OR (typeof(retry_at) = 'integer' AND retry_at >= 0)),
    backoff_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(backoff_count) = 'integer' AND backoff_count >= 0),
    offered_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(offered_count) = 'integer' AND offered_count >= 0),
    wanted_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(wanted_count) = 'integer' AND wanted_count >= 0),
    accepted_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(accepted_count) = 'integer' AND accepted_count >= 0),
    accepted_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(accepted_bytes) = 'integer' AND accepted_bytes >= 0),
    failure_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(failure_count) = 'integer' AND failure_count >= 0)
) STRICT";

const PEER_ITEMS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_peer_items (
    peer BLOB NOT NULL REFERENCES standard_lxmf_propagation_peers(identity_hash) ON DELETE CASCADE,
    transient_id BLOB NOT NULL REFERENCES standard_lxmf_propagation_items(transient_id) ON DELETE CASCADE,
    disposition TEXT NOT NULL CHECK(typeof(disposition) = 'text' AND disposition IN ('handled','unhandled')),
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= 0),
    PRIMARY KEY(peer, transient_id)
) STRICT";

const SELECTION_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_selection (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    selected_peer BLOB REFERENCES standard_lxmf_propagation_peers(identity_hash) ON DELETE SET NULL,
    mode TEXT NOT NULL CHECK(typeof(mode) = 'text' AND mode IN ('automatic','manual','disabled')),
    selected_at INTEGER NOT NULL CHECK(typeof(selected_at) = 'integer' AND selected_at >= 0)
) STRICT";

const ATTEMPTS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_attempts (
    attempt_id BLOB PRIMARY KEY CHECK(typeof(attempt_id) = 'blob' AND length(attempt_id) = 16),
    correlation_id BLOB NOT NULL CHECK(typeof(correlation_id) = 'blob' AND length(correlation_id) = 16),
    peer BLOB REFERENCES standard_lxmf_propagation_peers(identity_hash) ON DELETE SET NULL,
    direction TEXT NOT NULL CHECK(typeof(direction) = 'text' AND direction IN ('ingress','egress','sync')),
    stage TEXT NOT NULL CHECK(typeof(stage) = 'text' AND stage IN ('offer','transfer','get','complete')),
    state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('running','completed','failed','interrupted')),
    started_at INTEGER NOT NULL CHECK(typeof(started_at) = 'integer' AND started_at >= 0),
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= started_at),
    deadline_at INTEGER CHECK(deadline_at IS NULL OR (typeof(deadline_at) = 'integer' AND deadline_at >= started_at)),
    offered_count INTEGER NOT NULL CHECK(typeof(offered_count) = 'integer' AND offered_count >= 0 AND offered_count <= 4096),
    wanted_count INTEGER NOT NULL CHECK(typeof(wanted_count) = 'integer' AND wanted_count >= 0 AND wanted_count <= offered_count),
    accepted_count INTEGER NOT NULL DEFAULT 0 CHECK(typeof(accepted_count) = 'integer' AND accepted_count >= 0 AND accepted_count <= offered_count),
    accepted_bytes INTEGER NOT NULL DEFAULT 0 CHECK(typeof(accepted_bytes) = 'integer' AND accepted_bytes >= 0),
    failure_code TEXT CHECK(failure_code IS NULL OR (typeof(failure_code) = 'text' AND length(failure_code) BETWEEN 1 AND 64)),
    failure_detail TEXT CHECK(failure_detail IS NULL OR (typeof(failure_detail) = 'text' AND length(failure_detail) <= 256))
) STRICT";

const ATTEMPT_ITEMS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_attempt_items (
    attempt_id BLOB NOT NULL REFERENCES standard_lxmf_propagation_attempts(attempt_id) ON DELETE CASCADE,
    transient_id BLOB NOT NULL CHECK(typeof(transient_id) = 'blob' AND length(transient_id) = 32),
    role TEXT NOT NULL CHECK(typeof(role) = 'text' AND role IN ('offered','wanted','accepted')),
    PRIMARY KEY(attempt_id, transient_id, role)
) STRICT";

const CHECKPOINTS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_checkpoints (
    peer BLOB NOT NULL REFERENCES standard_lxmf_propagation_peers(identity_hash) ON DELETE CASCADE,
    direction TEXT NOT NULL CHECK(typeof(direction) = 'text' AND direction IN ('ingress','egress','sync')),
    completed_stage TEXT NOT NULL CHECK(typeof(completed_stage) = 'text' AND completed_stage IN ('offer','transfer','get','complete')),
    cursor BLOB CHECK(cursor IS NULL OR (typeof(cursor) = 'blob' AND length(cursor) <= 4096)),
    digest BLOB CHECK(digest IS NULL OR (typeof(digest) = 'blob' AND length(digest) = 32)),
    item_count INTEGER NOT NULL CHECK(typeof(item_count) = 'integer' AND item_count >= 0),
    byte_count INTEGER NOT NULL CHECK(typeof(byte_count) = 'integer' AND byte_count >= 0),
    last_attempt BLOB REFERENCES standard_lxmf_propagation_attempts(attempt_id) ON DELETE SET NULL,
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= 0),
    PRIMARY KEY(peer, direction)
) STRICT";

const FAILURES_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_failures (
    failure_id INTEGER PRIMARY KEY,
    code TEXT NOT NULL CHECK(typeof(code) = 'text' AND length(code) BETWEEN 1 AND 64),
    detail TEXT CHECK(detail IS NULL OR (typeof(detail) = 'text' AND length(detail) <= 256)),
    occurred_at INTEGER NOT NULL CHECK(typeof(occurred_at) = 'integer' AND occurred_at >= 0),
    peer BLOB REFERENCES standard_lxmf_propagation_peers(identity_hash) ON DELETE SET NULL,
    transient_id BLOB CHECK(transient_id IS NULL OR (typeof(transient_id) = 'blob' AND length(transient_id) = 32)),
    attempt_id BLOB REFERENCES standard_lxmf_propagation_attempts(attempt_id) ON DELETE SET NULL
) STRICT";

const INDEX_SQL: &str = "CREATE INDEX idx_standard_lxmf_propagation_items_destination
    ON standard_lxmf_propagation_items(destination, state, stored_size, transient_id);
CREATE INDEX idx_standard_lxmf_propagation_items_expiry
    ON standard_lxmf_propagation_items(state, expires_at);
CREATE INDEX idx_standard_lxmf_propagation_peer_items_disposition
    ON standard_lxmf_propagation_peer_items(peer, disposition, updated_at);
CREATE INDEX idx_standard_lxmf_propagation_attempts_state
    ON standard_lxmf_propagation_attempts(state, updated_at);
CREATE INDEX idx_standard_lxmf_propagation_attempts_peer
    ON standard_lxmf_propagation_attempts(peer, direction, updated_at);
CREATE INDEX idx_standard_lxmf_propagation_failures_time
    ON standard_lxmf_propagation_failures(occurred_at, failure_id);";

const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "standard_lxmf_propagation_items",
        columns: ITEM_COLUMNS,
        create_sql: ITEMS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_peers",
        columns: PEER_COLUMNS,
        create_sql: PEERS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_peer_items",
        columns: PEER_ITEM_COLUMNS,
        create_sql: PEER_ITEMS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_selection",
        columns: SELECTION_COLUMNS,
        create_sql: SELECTION_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_attempts",
        columns: ATTEMPT_COLUMNS,
        create_sql: ATTEMPTS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_attempt_items",
        columns: ATTEMPT_ITEM_COLUMNS,
        create_sql: ATTEMPT_ITEMS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_checkpoints",
        columns: CHECKPOINT_COLUMNS,
        create_sql: CHECKPOINTS_TABLE_SQL,
    },
    TableSpec {
        name: "standard_lxmf_propagation_failures",
        columns: FAILURE_COLUMNS,
        create_sql: FAILURES_TABLE_SQL,
    },
];

const EXPLICIT_INDEXES: &[(&str, &str, &[&str])] = &[
    (
        "idx_standard_lxmf_propagation_items_destination",
        "standard_lxmf_propagation_items",
        &["destination", "state", "stored_size", "transient_id"],
    ),
    (
        "idx_standard_lxmf_propagation_items_expiry",
        "standard_lxmf_propagation_items",
        &["state", "expires_at"],
    ),
    (
        "idx_standard_lxmf_propagation_peer_items_disposition",
        "standard_lxmf_propagation_peer_items",
        &["peer", "disposition", "updated_at"],
    ),
    (
        "idx_standard_lxmf_propagation_attempts_state",
        "standard_lxmf_propagation_attempts",
        &["state", "updated_at"],
    ),
    (
        "idx_standard_lxmf_propagation_attempts_peer",
        "standard_lxmf_propagation_attempts",
        &["peer", "direction", "updated_at"],
    ),
    (
        "idx_standard_lxmf_propagation_failures_time",
        "standard_lxmf_propagation_failures",
        &["occurred_at", "failure_id"],
    ),
];

const TABLE_INDEX_COUNTS: &[(&str, i64)] = &[
    ("standard_lxmf_propagation_items", 3),
    ("standard_lxmf_propagation_peers", 2),
    ("standard_lxmf_propagation_peer_items", 2),
    ("standard_lxmf_propagation_selection", 0),
    ("standard_lxmf_propagation_attempts", 3),
    ("standard_lxmf_propagation_attempt_items", 1),
    ("standard_lxmf_propagation_checkpoints", 1),
    ("standard_lxmf_propagation_failures", 1),
];

const TARGET_OBJECTS: &[&str] = &[
    "standard_lxmf_propagation_items",
    "standard_lxmf_propagation_peers",
    "standard_lxmf_propagation_peer_items",
    "standard_lxmf_propagation_selection",
    "standard_lxmf_propagation_attempts",
    "standard_lxmf_propagation_attempt_items",
    "standard_lxmf_propagation_checkpoints",
    "standard_lxmf_propagation_failures",
    "idx_standard_lxmf_propagation_items_destination",
    "idx_standard_lxmf_propagation_items_expiry",
    "idx_standard_lxmf_propagation_peer_items_disposition",
    "idx_standard_lxmf_propagation_attempts_state",
    "idx_standard_lxmf_propagation_attempts_peer",
    "idx_standard_lxmf_propagation_failures_time",
];

fn normalize_schema_sql(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\'' => {
                normalized.push(character);
                while let Some(quoted) = chars.next() {
                    normalized.push(quoted);
                    if quoted == '\'' {
                        if chars.peek() == Some(&'\'') {
                            normalized.push(chars.next().unwrap_or('\''));
                        } else {
                            break;
                        }
                    }
                }
            }
            '"' | '`' => {
                let delimiter = character;
                while let Some(quoted) = chars.next() {
                    if quoted == delimiter {
                        if chars.peek() == Some(&delimiter) {
                            normalized.push(delimiter);
                            chars.next();
                        } else {
                            break;
                        }
                    } else {
                        normalized.extend(quoted.to_lowercase());
                    }
                }
            }
            '[' => {
                for quoted in chars.by_ref() {
                    if quoted == ']' {
                        break;
                    }
                    normalized.extend(quoted.to_lowercase());
                }
            }
            ';' => {}
            character if character.is_whitespace() => {}
            character => normalized.extend(character.to_lowercase()),
        }
    }
    normalized
}

fn schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    for table in TABLES {
        let create_sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table.name],
                |row| row.get(0),
            )
            .optional()?;
        let expected_sql = normalize_schema_sql(table.create_sql);
        if create_sql.as_deref().map(normalize_schema_sql) != Some(expected_sql) {
            return Ok(false);
        }
        let strict: Option<i64> = conn
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND type = 'table' AND name = ?1",
                params![table.name],
                |row| row.get(0),
            )
            .optional()?;
        if strict != Some(1) {
            return Ok(false);
        }
        let mut statement = conn.prepare(
            "SELECT name, type, \"notnull\", pk FROM pragma_table_xinfo(?1)
             WHERE hidden = 0 ORDER BY cid",
        )?;
        let actual = statement
            .query_map(params![table.name], |row| {
                Ok(ColumnMetadata {
                    name: row.get(0)?,
                    data_type: row.get(1)?,
                    not_null: row.get(2)?,
                    primary_key: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual.len() != table.columns.len()
            || actual.iter().zip(table.columns).any(|(actual, expected)| {
                actual.name != expected.name
                    || actual.data_type != expected.data_type
                    || actual.not_null != expected.not_null
                    || actual.primary_key != expected.primary_key
            })
        {
            return Ok(false);
        }
    }

    let expected_foreign_keys = [
        (
            "standard_lxmf_propagation_attempt_items",
            "attempt_id",
            "standard_lxmf_propagation_attempts",
            "attempt_id",
            "CASCADE",
        ),
        (
            "standard_lxmf_propagation_attempts",
            "peer",
            "standard_lxmf_propagation_peers",
            "identity_hash",
            "SET NULL",
        ),
        (
            "standard_lxmf_propagation_checkpoints",
            "last_attempt",
            "standard_lxmf_propagation_attempts",
            "attempt_id",
            "SET NULL",
        ),
        (
            "standard_lxmf_propagation_checkpoints",
            "peer",
            "standard_lxmf_propagation_peers",
            "identity_hash",
            "CASCADE",
        ),
        (
            "standard_lxmf_propagation_failures",
            "attempt_id",
            "standard_lxmf_propagation_attempts",
            "attempt_id",
            "SET NULL",
        ),
        (
            "standard_lxmf_propagation_failures",
            "peer",
            "standard_lxmf_propagation_peers",
            "identity_hash",
            "SET NULL",
        ),
        (
            "standard_lxmf_propagation_peer_items",
            "peer",
            "standard_lxmf_propagation_peers",
            "identity_hash",
            "CASCADE",
        ),
        (
            "standard_lxmf_propagation_peer_items",
            "transient_id",
            "standard_lxmf_propagation_items",
            "transient_id",
            "CASCADE",
        ),
        (
            "standard_lxmf_propagation_selection",
            "selected_peer",
            "standard_lxmf_propagation_peers",
            "identity_hash",
            "SET NULL",
        ),
    ];
    let mut actual_foreign_keys: Vec<(String, String, String, String, String)> = Vec::new();
    for table in TABLES {
        let mut statement = conn.prepare(
            "SELECT \"from\", \"table\", \"to\", on_delete FROM pragma_foreign_key_list(?1)",
        )?;
        let rows = statement
            .query_map(params![table.name], |row| {
                Ok((table.name.to_string(), row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        actual_foreign_keys.extend(rows);
    }
    actual_foreign_keys.sort_unstable();
    let mut expected_foreign_keys = expected_foreign_keys
        .into_iter()
        .map(|values| {
            (
                values.0.to_string(),
                values.1.to_string(),
                values.2.to_string(),
                values.3.to_string(),
                values.4.to_string(),
            )
        })
        .collect::<Vec<_>>();
    expected_foreign_keys.sort_unstable();
    if actual_foreign_keys != expected_foreign_keys {
        return Ok(false);
    }

    for (index, table, expected_columns) in EXPLICIT_INDEXES {
        let index_metadata: Option<(String, i64, String, i64)> = conn
            .query_row(
                "SELECT m.tbl_name, l.\"unique\", l.origin, l.partial
                 FROM sqlite_master m JOIN pragma_index_list(m.tbl_name) l ON l.name = m.name
                 WHERE m.type = 'index' AND m.name = ?1",
                params![index],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if index_metadata
            .as_ref()
            .map(|values| (values.0.as_str(), values.1, values.2.as_str(), values.3))
            != Some((*table, 0, "c", 0))
        {
            return Ok(false);
        }
        let mut statement =
            conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
        let columns = statement
            .query_map(params![index], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns.iter().map(String::as_str).collect::<Vec<_>>() != *expected_columns {
            return Ok(false);
        }
    }
    let explicit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'index' AND name LIKE 'idx_standard_lxmf_propagation_%'",
        [],
        |row| row.get(0),
    )?;
    if explicit_count != EXPLICIT_INDEXES.len() as i64 {
        return Ok(false);
    }
    for (table, expected_count) in TABLE_INDEX_COUNTS {
        let actual_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_index_list(?1)", params![table], |row| {
                row.get(0)
            })?;
        if actual_count != *expected_count {
            return Ok(false);
        }
    }
    let trigger_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'trigger' AND (
             name LIKE 'standard_lxmf_propagation_%'
             OR tbl_name LIKE 'standard_lxmf_propagation_%'
         )",
        [],
        |row| row.get(0),
    )?;
    if trigger_count != 0 {
        return Ok(false);
    }
    let peer_destination_unique_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM pragma_index_list('standard_lxmf_propagation_peers') l
         WHERE l.\"unique\" = 1 AND l.origin = 'u' AND l.partial = 0
           AND (SELECT COUNT(*) FROM pragma_index_info(l.name)) = 1
           AND (SELECT name FROM pragma_index_info(l.name) WHERE seqno = 0)
               = 'propagation_destination'",
        [],
        |row| row.get(0),
    )?;
    if peer_destination_unique_count != 1 {
        return Ok(false);
    }
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Ok(false);
    }
    let foreign_key_failure: bool =
        conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)", [], |row| {
            row.get(0)
        })?;
    Ok(!foreign_key_failure)
}

const ATTEMPT_OPERATIONS_TABLE_SQL: &str =
    "CREATE TABLE standard_lxmf_propagation_attempt_operations (
        attempt_id BLOB PRIMARY KEY
            REFERENCES standard_lxmf_propagation_attempts(attempt_id) ON DELETE CASCADE
            CHECK(typeof(attempt_id) = 'blob' AND length(attempt_id) = 16),
        operation TEXT NOT NULL
            CHECK(typeof(operation) = 'text' AND operation IN ('fetch','download','sync'))
    ) STRICT";

const MESSAGE_LINKS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_message_links (
    transient_id BLOB NOT NULL CHECK(typeof(transient_id) = 'blob' AND length(transient_id) = 32),
    message_id TEXT NOT NULL CHECK(typeof(message_id) = 'text' AND length(message_id) = 64 AND message_id = lower(message_id) AND message_id NOT GLOB '*[^0-9a-f]*'),
    relation TEXT NOT NULL CHECK(typeof(relation) = 'text' AND relation IN ('outbound','inbound')),
    attempt_id BLOB CHECK(attempt_id IS NULL OR (typeof(attempt_id) = 'blob' AND length(attempt_id) = 16)),
    peer BLOB NOT NULL CHECK(typeof(peer) = 'blob' AND length(peer) = 16),
    state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('spooled','accepted','pending_ack','acknowledged','deleted')),
    created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= created_at),
    CHECK((relation = 'outbound' AND state IN ('spooled','accepted','deleted'))
       OR (relation = 'inbound' AND state IN ('pending_ack','acknowledged','deleted'))),
    PRIMARY KEY(transient_id, peer, relation)
) STRICT";

const CLIENT_JOBS_TABLE_SQL: &str = "CREATE TABLE standard_lxmf_propagation_client_jobs (
    message_id TEXT PRIMARY KEY CHECK(typeof(message_id) = 'text' AND length(message_id) = 64 AND message_id = lower(message_id) AND message_id NOT GLOB '*[^0-9a-f]*'),
    transient_id BLOB UNIQUE CHECK(transient_id IS NULL OR (typeof(transient_id) = 'blob' AND length(transient_id) = 32)),
    destination BLOB NOT NULL CHECK(typeof(destination) = 'blob' AND length(destination) = 16),
    canonical_wire BLOB CHECK(canonical_wire IS NULL OR (typeof(canonical_wire) = 'blob' AND length(canonical_wire) BETWEEN 113 AND 4000000)),
    lxmf_data BLOB CHECK(lxmf_data IS NULL OR (typeof(lxmf_data) = 'blob' AND length(lxmf_data) BETWEEN 113 AND 4000000)),
    stamp BLOB CHECK(stamp IS NULL OR (typeof(stamp) = 'blob' AND length(stamp) = 32)),
    peer BLOB NOT NULL CHECK(typeof(peer) = 'blob' AND length(peer) = 16),
    propagation_destination BLOB NOT NULL CHECK(typeof(propagation_destination) = 'blob' AND length(propagation_destination) = 16),
    stamp_cost INTEGER NOT NULL CHECK(typeof(stamp_cost) = 'integer' AND stamp_cost BETWEEN 0 AND 254),
    peering_cost INTEGER NOT NULL CHECK(typeof(peering_cost) = 'integer' AND peering_cost BETWEEN 0 AND 254),
    correlation_id BLOB NOT NULL CHECK(typeof(correlation_id) = 'blob' AND length(correlation_id) = 16),
    attempt_id BLOB NOT NULL CHECK(typeof(attempt_id) = 'blob' AND length(attempt_id) = 16),
    state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('preparing','spooled','uploading','accepted','failed')),
    created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= created_at),
    CHECK((state = 'preparing' AND canonical_wire IS NOT NULL AND transient_id IS NULL
           AND lxmf_data IS NULL AND stamp IS NULL AND substr(canonical_wire, 1, 16) = destination)
       OR (state IN ('spooled','uploading','accepted','failed') AND canonical_wire IS NULL
           AND transient_id IS NOT NULL AND lxmf_data IS NOT NULL AND stamp IS NOT NULL
           AND substr(lxmf_data, 1, 16) = destination))
) STRICT";

const CLIENT_ATTEMPT_ITEMS_TABLE_SQL: &str =
    "CREATE TABLE standard_lxmf_propagation_client_attempt_items (
        attempt_id BLOB NOT NULL CHECK(typeof(attempt_id) = 'blob' AND length(attempt_id) = 16),
        transient_id BLOB NOT NULL CHECK(typeof(transient_id) = 'blob' AND length(transient_id) = 32),
        role TEXT NOT NULL CHECK(typeof(role) = 'text' AND role IN ('inventory','offered','accepted','returned')),
        created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0),
        PRIMARY KEY(attempt_id, transient_id, role)
    ) STRICT";

const CORRELATION_INDEX_SQL: &str =
    "CREATE INDEX idx_stdprop_v12_links_message ON standard_lxmf_propagation_message_links(message_id, updated_at, transient_id);
     CREATE INDEX idx_stdprop_v12_links_pending_ack ON standard_lxmf_propagation_message_links(state, updated_at, transient_id);
     CREATE INDEX idx_stdprop_v12_jobs_state ON standard_lxmf_propagation_client_jobs(state, updated_at, message_id);
     CREATE INDEX idx_stdprop_v12_attempt_items_transient ON standard_lxmf_propagation_client_attempt_items(transient_id, role, attempt_id);";
const CORRELATION_INDEXES: &[(&str, &str)] = &[
    (
        "idx_stdprop_v12_links_message",
        "CREATE INDEX idx_stdprop_v12_links_message ON standard_lxmf_propagation_message_links(message_id, updated_at, transient_id)",
    ),
    (
        "idx_stdprop_v12_links_pending_ack",
        "CREATE INDEX idx_stdprop_v12_links_pending_ack ON standard_lxmf_propagation_message_links(state, updated_at, transient_id)",
    ),
    (
        "idx_stdprop_v12_jobs_state",
        "CREATE INDEX idx_stdprop_v12_jobs_state ON standard_lxmf_propagation_client_jobs(state, updated_at, message_id)",
    ),
    (
        "idx_stdprop_v12_attempt_items_transient",
        "CREATE INDEX idx_stdprop_v12_attempt_items_transient ON standard_lxmf_propagation_client_attempt_items(transient_id, role, attempt_id)",
    ),
];

const CORRELATION_TARGETS: &[&str] = &[
    "standard_lxmf_propagation_message_links",
    "standard_lxmf_propagation_client_jobs",
    "standard_lxmf_propagation_client_attempt_items",
    "idx_stdprop_v12_links_message",
    "idx_stdprop_v12_links_pending_ack",
    "idx_stdprop_v12_jobs_state",
    "idx_stdprop_v12_attempt_items_transient",
];

fn correlation_schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    for (name, expected) in [
        ("standard_lxmf_propagation_message_links", MESSAGE_LINKS_TABLE_SQL),
        ("standard_lxmf_propagation_client_jobs", CLIENT_JOBS_TABLE_SQL),
        ("standard_lxmf_propagation_client_attempt_items", CLIENT_ATTEMPT_ITEMS_TABLE_SQL),
    ] {
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.as_deref().map(normalize_schema_sql) != Some(normalize_schema_sql(expected)) {
            return Ok(false);
        }
        let strict: Option<i64> = conn
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        if strict != Some(1) {
            return Ok(false);
        }
        let foreign_keys: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list(?1)",
            params![name],
            |row| row.get(0),
        )?;
        if foreign_keys != 0 {
            return Ok(false);
        }
    }
    let indexes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_stdprop_v12_%'",
        [],
        |row| row.get(0),
    )?;
    if indexes != 4 {
        return Ok(false);
    }
    for (name, expected) in CORRELATION_INDEXES {
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.as_deref().map(normalize_schema_sql) != Some(normalize_schema_sql(expected)) {
            return Ok(false);
        }
    }
    let triggers: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND (name LIKE 'idx_stdprop_v12_%' OR tbl_name IN (?1, ?2, ?3))",
        params![CORRELATION_TARGETS[0], CORRELATION_TARGETS[1], CORRELATION_TARGETS[2]],
        |row| row.get(0),
    )?;
    Ok(triggers == 0)
}

fn ensure_correlation_schema(conn: &mut Connection) -> rusqlite::Result<()> {
    let marked: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
        params![STANDARD_PROPAGATION_CORRELATION_MIGRATION],
        |row| row.get(0),
    )?;
    if marked {
        return if correlation_schema_is_valid(conn)? {
            Ok(())
        } else {
            Err(invalid("v12 standard propagation marker present for malformed schema"))
        };
    }
    for object in CORRELATION_TARGETS {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            params![object],
            |row| row.get(0),
        )?;
        if exists {
            return Err(invalid("v12 standard propagation target object already exists"));
        }
    }
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(MESSAGE_LINKS_TABLE_SQL)?;
    transaction.execute_batch(CLIENT_JOBS_TABLE_SQL)?;
    transaction.execute_batch(CLIENT_ATTEMPT_ITEMS_TABLE_SQL)?;
    transaction.execute_batch(CORRELATION_INDEX_SQL)?;
    if !correlation_schema_is_valid(&transaction)? {
        return Err(invalid("v12 standard propagation schema validation failed"));
    }
    transaction.execute(
        "INSERT INTO schema_migrations(id, applied_at) VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
        params![STANDARD_PROPAGATION_CORRELATION_MIGRATION],
    )?;
    transaction.commit()
}

fn observation_schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'standard_lxmf_propagation_attempt_operations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if sql.as_deref().map(normalize_schema_sql)
        != Some(normalize_schema_sql(ATTEMPT_OPERATIONS_TABLE_SQL))
    {
        return Ok(false);
    }
    let strict: Option<i64> = conn
        .query_row(
            "SELECT strict FROM pragma_table_list
             WHERE schema = 'main' AND type = 'table'
               AND name = 'standard_lxmf_propagation_attempt_operations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if strict != Some(1) {
        return Ok(false);
    }
    let foreign_key: Option<(String, String, String)> = conn
        .query_row(
            "SELECT \"from\", \"table\", on_delete
             FROM pragma_foreign_key_list('standard_lxmf_propagation_attempt_operations')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(foreign_key
        == Some((
            "attempt_id".into(),
            "standard_lxmf_propagation_attempts".into(),
            "CASCADE".into(),
        )))
}

fn ensure_observation_schema(conn: &mut Connection) -> rusqlite::Result<()> {
    let marker_present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
        params![STANDARD_PROPAGATION_OBSERVATION_MIGRATION],
        |row| row.get(0),
    )?;
    if marker_present {
        return if observation_schema_is_valid(conn)? {
            ensure_correlation_schema(conn)
        } else {
            Err(invalid("v11 standard propagation marker present for malformed schema"))
        };
    }
    let target_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
         WHERE name = 'standard_lxmf_propagation_attempt_operations')",
        [],
        |row| row.get(0),
    )?;
    if target_exists {
        return Err(invalid("v11 standard propagation target object already exists"));
    }
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(ATTEMPT_OPERATIONS_TABLE_SQL)?;
    if !observation_schema_is_valid(&transaction)? {
        return Err(invalid("v11 standard propagation schema validation failed"));
    }
    transaction.execute(
        "INSERT INTO schema_migrations(id, applied_at)
         VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
        params![STANDARD_PROPAGATION_OBSERVATION_MIGRATION],
    )?;
    transaction.commit()?;
    ensure_correlation_schema(conn)
}

pub(super) fn ensure_standard_propagation_schema(conn: &mut Connection) -> rusqlite::Result<()> {
    let marker_present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
        params![STANDARD_PROPAGATION_MIGRATION],
        |row| row.get(0),
    )?;
    if marker_present {
        if !schema_is_valid(conn)? {
            return Err(invalid("v10 standard propagation marker present for malformed schema"));
        }
        return ensure_observation_schema(conn);
    }

    for object in TARGET_OBJECTS {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            params![object],
            |row| row.get(0),
        )?;
        if exists {
            return Err(invalid("v10 standard propagation target object already exists"));
        }
    }

    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for table in TABLES {
        transaction.execute_batch(table.create_sql)?;
    }
    transaction.execute_batch(INDEX_SQL)?;
    if !schema_is_valid(&transaction)? {
        return Err(invalid("v10 standard propagation schema validation failed"));
    }
    transaction.execute(
        "INSERT INTO schema_migrations(id, applied_at) VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
        params![STANDARD_PROPAGATION_MIGRATION],
    )?;
    transaction.commit()?;
    ensure_observation_schema(conn)
}

fn expire_in_transaction(transaction: &Transaction<'_>, now: i64) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE standard_lxmf_propagation_items
         SET state = 'expired', lxmf_data = NULL, stamp = NULL, stored_size = 0, terminal_at = ?1
         WHERE state = 'queued' AND expires_at < ?1",
        params![now],
    )?;
    Ok(())
}

fn prune_in_transaction(transaction: &Transaction<'_>, now: i64) -> rusqlite::Result<()> {
    let tombstone_cutoff = now.saturating_sub(TOMBSTONE_RETENTION_SECS);
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_items
         WHERE state IN ('acknowledged','expired') AND terminal_at < ?1",
        params![tombstone_cutoff],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_client_jobs
         WHERE state IN ('accepted','failed') AND updated_at < ?1",
        params![tombstone_cutoff],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_message_links
         WHERE state IN ('accepted','acknowledged','deleted') AND updated_at < ?1",
        params![tombstone_cutoff],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_failures WHERE occurred_at < ?1",
        params![now.saturating_sub(FAILURE_RETENTION_SECS)],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_failures
         WHERE failure_id NOT IN (
             SELECT failure_id FROM standard_lxmf_propagation_failures
             ORDER BY occurred_at DESC, failure_id DESC LIMIT ?1
         )",
        params![to_i64(MAX_FAILURES, "failure retention")?],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_attempts
         WHERE state != 'running' AND updated_at < ?1",
        params![now.saturating_sub(ATTEMPT_RETENTION_SECS)],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_attempts
         WHERE state != 'running' AND attempt_id NOT IN (
             SELECT attempt_id FROM standard_lxmf_propagation_attempts
             ORDER BY updated_at DESC, attempt_id DESC LIMIT ?1
         )",
        params![to_i64(MAX_ATTEMPTS, "attempt retention")?],
    )?;
    Ok(())
}

fn reconcile_deadlines_in_transaction(
    transaction: &Transaction<'_>,
    now: i64,
) -> rusqlite::Result<usize> {
    let due = {
        let mut statement = transaction.prepare(
            "SELECT attempt_id, peer FROM standard_lxmf_propagation_attempts
             WHERE state = 'running' AND deadline_at IS NOT NULL AND deadline_at <= ?1
             ORDER BY deadline_at, attempt_id",
        )?;

        statement
            .query_map(params![now], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (attempt, peer) in &due {
        let attempt: [u8; 16] = blob_array(attempt.clone(), "deadline attempt")?;
        let peer = peer.clone().map(|value| blob_array(value, "deadline peer")).transpose()?;
        transaction.execute(
            "UPDATE standard_lxmf_propagation_attempts
             SET state = 'failed', updated_at = ?2, failure_code = 'deadline_elapsed',
                 failure_detail = NULL
             WHERE attempt_id = ?1 AND state = 'running'",
            params![attempt.as_slice(), now],
        )?;
        record_failure_in_transaction(
            transaction,
            "deadline_elapsed",
            None,
            now,
            peer.as_ref(),
            None,
            Some(&attempt),
        )?;
    }
    Ok(due.len())
}

fn stats_in_transaction(
    transaction: &Transaction<'_>,
) -> rusqlite::Result<StandardPropagationStats> {
    let (count, bytes): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(stored_size), 0)
         FROM standard_lxmf_propagation_items WHERE state = 'queued'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(StandardPropagationStats {
        queued_count: to_usize(count, "queued count")?,
        stored_bytes: to_usize(bytes, "queued bytes")?,
    })
}

fn record_failure_in_transaction(
    transaction: &Transaction<'_>,
    code: &str,
    detail: Option<&str>,
    now: i64,
    peer: Option<&[u8; 16]>,
    transient_id: Option<&[u8; 32]>,
    attempt_id: Option<&[u8; 16]>,
) -> rusqlite::Result<()> {
    if code.is_empty()
        || code.len() > 64
        || detail.is_some_and(|detail| detail.len() > MAX_FAILURE_DETAIL)
    {
        return Err(invalid("invalid standard propagation failure"));
    }
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_failures
         (code, detail, occurred_at, peer, transient_id, attempt_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            code,
            detail,
            now,
            peer.map(|value| value.as_slice()),
            transient_id.map(|value| value.as_slice()),
            attempt_id.map(|value| value.as_slice()),
        ],
    )?;
    transaction.execute(
        "DELETE FROM standard_lxmf_propagation_failures
         WHERE failure_id NOT IN (
             SELECT failure_id FROM standard_lxmf_propagation_failures
             ORDER BY occurred_at DESC, failure_id DESC LIMIT ?1
         )",
        params![to_i64(MAX_FAILURES, "failure retention")?],
    )?;
    if let Some(peer) = peer {
        transaction.execute(
            "UPDATE standard_lxmf_propagation_peers
             SET failure_count = failure_count + 1, last_seen_at = MAX(last_seen_at, ?2)
             WHERE identity_hash = ?1",
            params![peer.as_slice(), now],
        )?;
    }
    Ok(())
}

fn upsert_observed_peer(
    transaction: &Transaction<'_>,
    peer: &[u8; 16],
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_peers
         (identity_hash, origin, enabled, first_seen_at, last_seen_at)
         VALUES (?1, 'observed', 0, ?2, ?2)
         ON CONFLICT(identity_hash) DO UPDATE SET
             origin = CASE WHEN origin = 'configured' THEN 'both' ELSE origin END,
             last_seen_at = MAX(last_seen_at, excluded.last_seen_at)",
        params![peer.as_slice(), now],
    )?;
    let enabled: bool = transaction.query_row(
        "SELECT enabled FROM standard_lxmf_propagation_peers WHERE identity_hash = ?1",
        params![peer.as_slice()],
        |row| row.get(0),
    )?;
    if enabled {
        backfill_peer_items(transaction, peer, now)?;
    }
    Ok(())
}

fn backfill_peer_items(
    transaction: &Transaction<'_>,
    peer: &[u8; 16],
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_peer_items
         (peer, transient_id, disposition, updated_at)
         SELECT ?1, transient_id, 'unhandled', ?2
         FROM standard_lxmf_propagation_items WHERE state = 'queued'
         ON CONFLICT(peer, transient_id) DO NOTHING",
        params![peer.as_slice(), now],
    )?;
    Ok(())
}

pub(super) fn spool_outbound_in_transaction(
    transaction: &Transaction<'_>,
    job: &StandardPropagationClientJob,
) -> rusqlite::Result<()> {
    let valid_message_id = job.message_id.len() == 64
        && job
            .message_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid_message_id
        || job.created_at < 0
        || job.updated_at < job.created_at
        || job.stamp_cost > 254
        || job.peering_cost > 254
    {
        return Err(invalid("invalid outbound standard propagation job"));
    }
    if job.state == "preparing" {
        let canonical_wire = job
            .canonical_wire
            .as_deref()
            .ok_or_else(|| invalid("preparing propagation job has no canonical wire"))?;
        let wire = lxmf::WireMessage::unpack(canonical_wire)
            .map_err(|_| invalid("preparing propagation job has invalid canonical wire"))?;
        if job.transient_id.is_some()
            || job.lxmf_data.is_some()
            || job.stamp.is_some()
            || wire.destination != job.destination
            || lxmf::inbound_decode::outbound_message_id_hex(canonical_wire).as_deref()
                != Some(job.message_id.as_str())
        {
            return Err(invalid("invalid preparing standard propagation job"));
        }
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_client_jobs
             (message_id, transient_id, destination, canonical_wire, lxmf_data, stamp, peer,
              propagation_destination, stamp_cost, peering_cost, correlation_id, attempt_id, state,
              created_at, updated_at)
             VALUES (?1, NULL, ?2, ?3, NULL, NULL, ?4, ?5, ?6, ?7, ?8, ?9,
                     'preparing', ?10, ?11)",
            params![
                &job.message_id,
                job.destination.as_slice(),
                canonical_wire,
                job.peer.as_slice(),
                job.propagation_destination.as_slice(),
                i64::from(job.stamp_cost),
                i64::from(job.peering_cost),
                job.correlation_id.as_slice(),
                job.attempt_id.as_slice(),
                job.created_at,
                job.updated_at,
            ],
        )?;
        return Ok(());
    }
    let transient_id = job
        .transient_id
        .ok_or_else(|| invalid("materialized propagation job has no transient ID"))?;
    let lxmf_data = job
        .lxmf_data
        .as_deref()
        .ok_or_else(|| invalid("materialized propagation job has no ciphertext"))?;
    let stamp = job.stamp.ok_or_else(|| invalid("materialized propagation job has no stamp"))?;
    if job.canonical_wire.is_some()
        || job.message_id.len() != 64
        || !job
            .message_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || lxmf_data.len() < lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1
        || lxmf_data[..16] != job.destination
        || <[u8; 32]>::from(Sha256::digest(lxmf_data)) != transient_id
        || job.state != "spooled"
    {
        return Err(invalid("invalid outbound standard propagation spool"));
    }
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_client_jobs
          (message_id, transient_id, destination, canonical_wire, lxmf_data, stamp, peer,
           propagation_destination, stamp_cost, peering_cost, correlation_id, attempt_id, state,
           created_at, updated_at)
           VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                   'spooled', ?12, ?13)",
        params![
            &job.message_id,
            transient_id.as_slice(),
            job.destination.as_slice(),
            lxmf_data,
            stamp.as_slice(),
            job.peer.as_slice(),
            job.propagation_destination.as_slice(),
            i64::from(job.stamp_cost),
            i64::from(job.peering_cost),
            job.correlation_id.as_slice(),
            job.attempt_id.as_slice(),
            job.created_at,
            job.updated_at,
        ],
    )?;
    insert_materialized_outbound_children(transaction, job, transient_id)
}

fn insert_materialized_outbound_children(
    transaction: &Transaction<'_>,
    job: &StandardPropagationClientJob,
    transient_id: [u8; 32],
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_message_links
         (transient_id, message_id, relation, attempt_id, peer, state, created_at, updated_at)
         VALUES (?1, ?2, 'outbound', ?3, ?4, 'spooled', ?5, ?5)",
        params![
            transient_id.as_slice(),
            &job.message_id,
            job.attempt_id.as_slice(),
            job.peer.as_slice(),
            job.created_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_client_attempt_items
         (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, 'offered', ?3)",
        params![job.attempt_id.as_slice(), transient_id.as_slice(), job.created_at],
    )?;
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_attempts
         (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
          offered_count, wanted_count, accepted_count, accepted_bytes)
          VALUES (?1, ?2, ?3, 'egress', 'offer', 'running', ?4, ?4, 1, 0, 0, 0)",
        params![
            job.attempt_id.as_slice(),
            job.correlation_id.as_slice(),
            job.peer.as_slice(),
            job.created_at
        ],
    )?;
    Ok(())
}

pub(super) fn link_inbound_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &str,
    transient_id: [u8; 32],
    attempt_id: [u8; 16],
    peer: [u8; 16],
    now: i64,
) -> rusqlite::Result<()> {
    let message_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1 AND direction = 'in')",
        params![message_id],
        |row| row.get(0),
    )?;
    if !message_exists {
        return Err(invalid("inbound propagation link has no committed canonical message"));
    }
    let existing_message: Option<String> = transaction
        .query_row(
            "SELECT message_id FROM standard_lxmf_propagation_message_links
             WHERE transient_id = ?1 LIMIT 1",
            params![transient_id.as_slice()],
            |row| row.get(0),
        )
        .optional()?;
    if existing_message.as_deref().is_some_and(|existing| existing != message_id) {
        return Err(invalid("transient ID is already linked to another canonical message"));
    }
    transaction.execute(
        "INSERT INTO standard_lxmf_propagation_message_links
         (transient_id, message_id, relation, attempt_id, peer, state, created_at, updated_at)
         VALUES (?1, ?2, 'inbound', ?3, ?4, 'pending_ack', ?5, ?5)
         ON CONFLICT(transient_id, peer, relation) DO UPDATE SET
             attempt_id = COALESCE(attempt_id, excluded.attempt_id),
             updated_at = MAX(updated_at, excluded.updated_at)
         WHERE message_id = excluded.message_id AND relation = 'inbound'
           AND state IN ('pending_ack','acknowledged')",
        params![transient_id.as_slice(), message_id, attempt_id.as_slice(), peer.as_slice(), now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO standard_lxmf_propagation_client_attempt_items
         (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, 'returned', ?3)",
        params![attempt_id.as_slice(), transient_id.as_slice(), now],
    )?;
    Ok(())
}

impl MessagesStore {
    pub fn standard_propagation_upsert_peer(
        &mut self,
        peer: &StandardPropagationPeer,
    ) -> rusqlite::Result<()> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, peer.observed_at)?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_peers
             (identity_hash, propagation_destination, origin, enabled,
              transfer_limit_kb, sync_limit_kb, stamp_cost, stamp_flexibility, peering_cost,
              first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(identity_hash) DO UPDATE SET
                 propagation_destination = COALESCE(excluded.propagation_destination, propagation_destination),
                 origin = CASE
                     WHEN origin != excluded.origin THEN 'both'
                     ELSE origin
                 END,
                 enabled = excluded.enabled,
                 transfer_limit_kb = COALESCE(excluded.transfer_limit_kb, transfer_limit_kb),
                 sync_limit_kb = COALESCE(excluded.sync_limit_kb, sync_limit_kb),
                 stamp_cost = COALESCE(excluded.stamp_cost, stamp_cost),
                 stamp_flexibility = COALESCE(excluded.stamp_flexibility, stamp_flexibility),
                 peering_cost = COALESCE(excluded.peering_cost, peering_cost),
                 last_seen_at = MAX(last_seen_at, excluded.last_seen_at)",
            params![
                peer.identity_hash.as_slice(),
                peer.propagation_destination.map(|value| value.to_vec()),
                if peer.configured { "configured" } else { "observed" },
                i64::from(peer.enabled),
                peer.transfer_limit_kb
                    .map(|value| to_i64(value, "peer transfer limit"))
                    .transpose()?,
                peer.sync_limit_kb
                    .map(|value| to_i64(value, "peer sync limit"))
                    .transpose()?,
                peer.stamp_cost.map(i64::from),
                peer.stamp_flexibility.map(i64::from),
                peer.peering_cost.map(i64::from),
                peer.observed_at,
            ],
        )?;
        if peer.enabled {
            backfill_peer_items(&transaction, &peer.identity_hash, peer.observed_at)?;
        }
        transaction.commit()
    }

    pub fn standard_propagation_reconcile_startup(
        &mut self,
        now: i64,
        policy: StandardPropagationPolicy,
    ) -> rusqlite::Result<StandardPropagationStats> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        reconcile_deadlines_in_transaction(&transaction, now)?;
        transaction.execute(
            "UPDATE standard_lxmf_propagation_attempts
             SET state = 'interrupted', updated_at = ?1, failure_code = 'startup_interrupted'
             WHERE state = 'running'",
            params![now],
        )?;
        prune_in_transaction(&transaction, now)?;
        {
            let mut statement = transaction.prepare(
                "SELECT transient_id, destination, lxmf_data, stamp, stored_size
                 FROM standard_lxmf_propagation_items WHERE state = 'queued'",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let transient: [u8; 32] = blob_array(row.get(0)?, "queued transient")?;
                let destination: [u8; 16] = blob_array(row.get(1)?, "queued destination")?;
                let data: Vec<u8> = row.get(2)?;
                let stamp: Vec<u8> = row.get(3)?;
                let stored_size = to_usize(row.get(4)?, "queued stored size")?;
                let digest: [u8; 32] = Sha256::digest(&data).into();
                if data.len() < 16
                    || digest != transient
                    || data[..16] != destination
                    || stamp.len() != 32
                    || stored_size != data.len().saturating_add(stamp.len())
                {
                    return Err(invalid("standard propagation queued item invariant failed"));
                }
            }
        }
        let stats = stats_in_transaction(&transaction)?;
        if stats.queued_count > policy.queue_max_count
            || stats.stored_bytes > policy.queue_max_bytes
        {
            return Err(invalid("standard propagation queue exceeds configured capacity"));
        }
        transaction.commit()?;
        Ok(stats)
    }

    pub fn standard_propagation_reconcile_deadlines(
        &mut self,
        now: i64,
    ) -> rusqlite::Result<usize> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        let reconciled = reconcile_deadlines_in_transaction(&transaction, now)?;
        prune_in_transaction(&transaction, now)?;
        transaction.commit()?;
        Ok(reconciled)
    }

    pub fn standard_propagation_compare_offer(
        &mut self,
        request: StandardPropagationOfferRequest<'_>,
    ) -> rusqlite::Result<StandardPropagationOfferComparison> {
        let StandardPropagationOfferRequest {
            peer,
            offered,
            same_link_pending,
            pending_elsewhere,
            pending_count,
            existing_attempt,
            request_id,
            link_id,
            now,
            deadline,
            policy,
        } = request;
        if offered.len() > 1024 {
            return Err(invalid("standard propagation offer exceeds 1024 items"));
        }
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        reconcile_deadlines_in_transaction(&transaction, now)?;
        prune_in_transaction(&transaction, now)?;
        upsert_observed_peer(&transaction, &peer, now)?;
        let stats = stats_in_transaction(&transaction)?;
        let mut unknown = Vec::new();
        let mut wanted = Vec::new();
        for id in offered {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM standard_lxmf_propagation_items WHERE transient_id = ?1)",
                params![id.as_slice()],
                |row| row.get(0),
            )?;
            if exists || pending_elsewhere.contains(id) {
                continue;
            }
            if same_link_pending.contains(id) {
                wanted.push(*id);
            } else {
                unknown.push(*id);
            }
        }
        let available =
            policy.queue_max_count.saturating_sub(stats.queued_count.saturating_add(pending_count));
        let mut material = Vec::with_capacity(48 + offered.len().saturating_mul(32));
        material.extend_from_slice(&request_id);
        material.extend_from_slice(&link_id);
        material.extend_from_slice(&peer);
        for id in offered {
            material.extend_from_slice(id);
        }
        let digest = Sha256::digest(material);
        let mut generated_attempt = [0u8; 16];
        generated_attempt.copy_from_slice(&digest[..16]);
        let attempt_id = existing_attempt.unwrap_or(generated_attempt);
        let generated_attempt_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1)",
            params![generated_attempt.as_slice()],
            |row| row.get(0),
        )?;
        let attempt_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1)",
            params![attempt_id.as_slice()],
            |row| row.get(0),
        )?;
        let attempt_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM standard_lxmf_propagation_attempts",
            [],
            |row| row.get(0),
        )?;
        if !attempt_exists && to_usize(attempt_count, "attempt count")? >= MAX_ATTEMPTS {
            return Err(invalid("standard propagation attempt capacity exhausted"));
        }
        if !unknown.is_empty() && (available == 0 || stats.stored_bytes >= policy.queue_max_bytes) {
            if !generated_attempt_exists
                && to_usize(attempt_count, "attempt count")? >= MAX_ATTEMPTS
            {
                return Err(invalid("standard propagation attempt capacity exhausted"));
            }
            transaction.execute(
                "INSERT INTO standard_lxmf_propagation_attempts
                 (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
                  deadline_at, offered_count, wanted_count, failure_code)
                 VALUES (?1, ?2, ?3, 'ingress', 'offer', 'failed', ?4, ?4, ?5, ?6, 0, 'capacity')
                 ON CONFLICT(attempt_id) DO UPDATE SET state = 'failed', stage = 'offer',
                     updated_at = excluded.updated_at, failure_code = 'capacity', failure_detail = NULL",
                params![
                    generated_attempt.as_slice(),
                    request_id.as_slice(),
                    peer.as_slice(),
                    now,
                    deadline,
                    to_i64(offered.len(), "offered count")?,
                ],
            )?;
            transaction.execute(
                "DELETE FROM standard_lxmf_propagation_attempt_items WHERE attempt_id = ?1",
                params![generated_attempt.as_slice()],
            )?;
            for id in offered {
                transaction.execute(
                    "INSERT INTO standard_lxmf_propagation_attempt_items
                     (attempt_id, transient_id, role) VALUES (?1, ?2, 'offered')",
                    params![generated_attempt.as_slice(), id.as_slice()],
                )?;
            }
            record_failure_in_transaction(
                &transaction,
                "capacity",
                None,
                now,
                Some(&peer),
                None,
                Some(&generated_attempt),
            )?;
            transaction.execute(
                "UPDATE standard_lxmf_propagation_peers
                 SET offered_count = offered_count + ?2 WHERE identity_hash = ?1",
                params![peer.as_slice(), to_i64(offered.len(), "offered count")?],
            )?;
            transaction.commit()?;
            return Ok(StandardPropagationOfferComparison {
                wanted: Vec::new(),
                attempt_id: generated_attempt,
                capacity_rejected: true,
            });
        }
        wanted.extend(unknown.into_iter().take(available));
        let mut attempt_scope = same_link_pending.clone();
        attempt_scope.extend(wanted.iter().copied());
        let attempt_offered_count =
            if existing_attempt.is_some() { attempt_scope.len() } else { offered.len() };
        let attempt_wanted_count = attempt_scope.len();
        let (attempt_stage, attempt_state) =
            if attempt_scope.is_empty() { ("complete", "completed") } else { ("offer", "running") };
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempts
             (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
              deadline_at, offered_count, wanted_count)
             VALUES (?1, ?2, ?3, 'ingress', ?4, ?5, ?6, ?6, ?7, ?8, ?9)
             ON CONFLICT(attempt_id) DO UPDATE SET
                 updated_at = excluded.updated_at,
                 deadline_at = excluded.deadline_at,
                 offered_count = MAX(offered_count, excluded.offered_count),
                 wanted_count = MAX(wanted_count, excluded.wanted_count),
                 state = excluded.state, stage = excluded.stage,
                 failure_code = NULL, failure_detail = NULL",
            params![
                attempt_id.as_slice(),
                request_id.as_slice(),
                peer.as_slice(),
                attempt_stage,
                attempt_state,
                now,
                deadline,
                to_i64(attempt_offered_count, "offered count")?,
                to_i64(attempt_wanted_count, "wanted count")?,
            ],
        )?;
        for id in offered {
            transaction.execute(
                "INSERT OR IGNORE INTO standard_lxmf_propagation_attempt_items(attempt_id, transient_id, role)
                 VALUES (?1, ?2, 'offered')",
                params![attempt_id.as_slice(), id.as_slice()],
            )?;
        }
        for id in &attempt_scope {
            transaction.execute(
                "INSERT OR IGNORE INTO standard_lxmf_propagation_attempt_items(attempt_id, transient_id, role)
                 VALUES (?1, ?2, 'wanted')",
                params![attempt_id.as_slice(), id.as_slice()],
            )?;
        }
        transaction.execute(
            "UPDATE standard_lxmf_propagation_peers
             SET offered_count = offered_count + ?2, wanted_count = wanted_count + ?3
             WHERE identity_hash = ?1",
            params![
                peer.as_slice(),
                to_i64(offered.len(), "offered count")?,
                to_i64(wanted.len(), "wanted count")?,
            ],
        )?;
        transaction.commit()?;
        Ok(StandardPropagationOfferComparison { wanted, attempt_id, capacity_rejected: false })
    }

    pub fn standard_propagation_ingest_batch(
        &mut self,
        request: StandardPropagationIngestRequest<'_>,
    ) -> rusqlite::Result<StandardPropagationIngestOutcome> {
        let StandardPropagationIngestRequest { items, source_peer, attempt, protocol, now, policy } =
            request;
        if items.is_empty() || items.len() > 1024 {
            return Err(invalid("invalid standard propagation ingest batch"));
        }
        if !matches!(attempt, StandardPropagationAttemptStatus::Untracked) && source_peer.is_none()
        {
            return Err(invalid("tracked standard propagation ingest requires a source peer"));
        }
        if matches!(attempt, StandardPropagationAttemptStatus::Complete(_))
            && protocol != StandardPropagationProtocolStatus::Valid
        {
            return Err(invalid("invalid standard propagation batch cannot complete an attempt"));
        }
        let attempt_id = attempt.attempt_id();
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        prune_in_transaction(&transaction, now)?;
        let mut net_new = Vec::new();
        let mut queued_duplicates = BTreeSet::new();
        let mut net_stored_bytes = 0usize;
        let mut net_payload_bytes = 0usize;
        let mut batch_items = BTreeMap::new();
        for item in items {
            if item.received_at < 0
                || item.expires_at < item.received_at
                || item.stored_size != item.lxmf_data.len().saturating_add(item.stamp.len())
                || item.lxmf_data.len() < 16
                || item.lxmf_data[..16] != item.destination
                || <[u8; 32]>::from(Sha256::digest(&item.lxmf_data)) != item.transient_id
            {
                return Err(invalid("invalid standard propagation ingest item"));
            }
            if let Some(existing) = batch_items.insert(item.transient_id, item) {
                if existing.destination != item.destination
                    || existing.lxmf_data != item.lxmf_data
                    || existing.stamp != item.stamp
                {
                    return Err(invalid("conflicting duplicate propagation item"));
                }
                continue;
            }
            let state: Option<String> = transaction
                .query_row(
                    "SELECT state FROM standard_lxmf_propagation_items WHERE transient_id = ?1",
                    params![item.transient_id.as_slice()],
                    |row| row.get(0),
                )
                .optional()?;
            match state.as_deref() {
                None => {
                    net_stored_bytes = net_stored_bytes
                        .checked_add(item.stored_size)
                        .ok_or_else(|| invalid("standard propagation ingest byte overflow"))?;
                    net_payload_bytes = net_payload_bytes
                        .checked_add(item.lxmf_data.len())
                        .ok_or_else(|| invalid("standard propagation payload byte overflow"))?;
                    net_new.push(item);
                }
                Some("queued") => {
                    queued_duplicates.insert(item.transient_id);
                }
                Some("acknowledged" | "expired") => {}
                Some(_) => return Err(invalid("invalid stored standard propagation item state")),
            }
        }
        let stats = stats_in_transaction(&transaction)?;
        if stats.queued_count.saturating_add(net_new.len()) > policy.queue_max_count
            || stats.stored_bytes.saturating_add(net_stored_bytes) > policy.queue_max_bytes
        {
            if let Some(peer) = source_peer {
                upsert_observed_peer(&transaction, &peer, now)?;
                record_failure_in_transaction(
                    &transaction,
                    "capacity",
                    None,
                    now,
                    Some(&peer),
                    None,
                    attempt_id.as_ref(),
                )?;
            }
            if let Some(attempt_id) = attempt_id {
                transaction.execute(
                    "UPDATE standard_lxmf_propagation_attempts
                     SET state = 'failed', stage = 'transfer', updated_at = ?2,
                         failure_code = 'capacity' WHERE attempt_id = ?1",
                    params![attempt_id.as_slice(), now],
                )?;
            }
            transaction.commit()?;
            return Ok(StandardPropagationIngestOutcome::CapacityRejected);
        }
        let net_new_ids: BTreeSet<_> = net_new.iter().map(|item| item.transient_id).collect();
        for item in net_new {
            transaction.execute(
                "INSERT INTO standard_lxmf_propagation_items
                 (transient_id, destination, lxmf_data, stamp, stamp_value, received_at,
                  expires_at, stored_size, state, terminal_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', NULL)",
                params![
                    item.transient_id.as_slice(),
                    item.destination.as_slice(),
                    &item.lxmf_data,
                    item.stamp.as_slice(),
                    i64::from(item.stamp_value),
                    item.received_at,
                    item.expires_at,
                    to_i64(item.stored_size, "stored size")?,
                ],
            )?;
        }
        let accepted_items: Vec<_> = batch_items.into_values().collect();
        if let Some(peer) = source_peer {
            if !net_new_ids.is_empty() {
                upsert_observed_peer(&transaction, &peer, now)?;
            }
            for item in &accepted_items {
                if queued_duplicates.contains(&item.transient_id) {
                    transaction.execute(
                        "INSERT INTO standard_lxmf_propagation_peer_items
                         (peer, transient_id, disposition, updated_at)
                         VALUES (?1, ?2, 'handled', ?3)
                         ON CONFLICT(peer, transient_id) DO UPDATE SET
                             disposition = 'handled', updated_at = excluded.updated_at",
                        params![peer.as_slice(), item.transient_id.as_slice(), now],
                    )?;
                    continue;
                }
                if !net_new_ids.contains(&item.transient_id) {
                    continue;
                }
                transaction.execute(
                    "INSERT INTO standard_lxmf_propagation_peer_items
                     (peer, transient_id, disposition, updated_at)
                     VALUES (?1, ?2, 'handled', ?3)
                     ON CONFLICT(peer, transient_id) DO UPDATE SET
                         disposition = 'handled', updated_at = excluded.updated_at",
                    params![peer.as_slice(), item.transient_id.as_slice(), now],
                )?;
                transaction.execute(
                    "INSERT INTO standard_lxmf_propagation_peer_items
                     (peer, transient_id, disposition, updated_at)
                     SELECT identity_hash, ?1, 'unhandled', ?2
                     FROM standard_lxmf_propagation_peers
                     WHERE enabled = 1 AND identity_hash != ?3
                     ON CONFLICT(peer, transient_id) DO NOTHING",
                    params![item.transient_id.as_slice(), now, peer.as_slice()],
                )?;
            }
            if !net_new_ids.is_empty() {
                transaction.execute(
                    "UPDATE standard_lxmf_propagation_peers
                     SET accepted_count = accepted_count + ?2,
                         accepted_bytes = accepted_bytes + ?3,
                         last_seen_at = MAX(last_seen_at, ?4), backoff_count = 0, retry_at = NULL
                     WHERE identity_hash = ?1",
                    params![
                        peer.as_slice(),
                        to_i64(net_new_ids.len(), "accepted count",)?,
                        to_i64(net_payload_bytes, "accepted payload bytes")?,
                        now,
                    ],
                )?;
            }
            if let Some(attempt_id) = attempt_id {
                let changed = transaction.execute(
                    "UPDATE standard_lxmf_propagation_attempts
                     SET stage = 'transfer', updated_at = ?2,
                         failure_code = NULL, failure_detail = NULL
                     WHERE attempt_id = ?1 AND peer = ?3 AND state = 'running'",
                    params![attempt_id.as_slice(), now, peer.as_slice()],
                )?;
                if changed != 1 {
                    return Err(invalid("standard propagation attempt is not running for source"));
                }
                for item in &accepted_items {
                    transaction.execute(
                        "INSERT OR IGNORE INTO standard_lxmf_propagation_attempt_items
                         (attempt_id, transient_id, role) VALUES (?1, ?2, 'accepted')",
                        params![attempt_id.as_slice(), item.transient_id.as_slice()],
                    )?;
                }
                let accepted_count: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM standard_lxmf_propagation_attempt_items
                     WHERE attempt_id = ?1 AND role = 'accepted'",
                    params![attempt_id.as_slice()],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "UPDATE standard_lxmf_propagation_attempts
                     SET accepted_count = ?2, accepted_bytes = accepted_bytes + ?3
                     WHERE attempt_id = ?1",
                    params![
                        attempt_id.as_slice(),
                        accepted_count,
                        to_i64(net_payload_bytes, "accepted payload bytes")?,
                    ],
                )?;
                if protocol == StandardPropagationProtocolStatus::Invalid {
                    record_failure_in_transaction(
                        &transaction,
                        "invalid_stamp",
                        None,
                        now,
                        Some(&peer),
                        None,
                        Some(&attempt_id),
                    )?;
                }
                if matches!(attempt, StandardPropagationAttemptStatus::Complete(_)) {
                    let missing: bool = transaction.query_row(
                        "SELECT EXISTS(
                             SELECT 1 FROM standard_lxmf_propagation_attempt_items wanted
                             WHERE wanted.attempt_id = ?1 AND wanted.role = 'wanted'
                               AND NOT EXISTS(
                                   SELECT 1 FROM standard_lxmf_propagation_attempt_items accepted
                                   WHERE accepted.attempt_id = wanted.attempt_id
                                     AND accepted.transient_id = wanted.transient_id
                                     AND accepted.role = 'accepted'
                               )
                         )",
                        params![attempt_id.as_slice()],
                        |row| row.get(0),
                    )?;
                    if missing {
                        return Err(invalid("standard propagation completion has pending items"));
                    }
                    let accepted_bytes: i64 = transaction.query_row(
                        "SELECT accepted_bytes FROM standard_lxmf_propagation_attempts
                         WHERE attempt_id = ?1",
                        params![attempt_id.as_slice()],
                        |row| row.get(0),
                    )?;
                    transaction.execute(
                        "UPDATE standard_lxmf_propagation_attempts
                         SET state = 'completed', stage = 'complete', updated_at = ?2
                         WHERE attempt_id = ?1",
                        params![attempt_id.as_slice(), now],
                    )?;
                    transaction.execute(
                        "INSERT INTO standard_lxmf_propagation_checkpoints
                         (peer, direction, completed_stage, item_count, byte_count, last_attempt, updated_at)
                         VALUES (?1, 'ingress', 'complete', ?2, ?3, ?4, ?5)
                         ON CONFLICT(peer, direction) DO UPDATE SET
                             completed_stage = excluded.completed_stage,
                             item_count = excluded.item_count,
                             byte_count = excluded.byte_count,
                             last_attempt = excluded.last_attempt,
                             updated_at = excluded.updated_at",
                        params![
                            peer.as_slice(),
                            accepted_count,
                            accepted_bytes,
                            attempt_id.as_slice(),
                            now,
                        ],
                    )?;
                }
            }
        } else if attempt_id.is_some() {
            return Err(invalid("tracked standard propagation ingest requires source peer"));
        }
        if source_peer.is_none() {
            for item in &accepted_items {
                if !net_new_ids.contains(&item.transient_id) {
                    continue;
                }
                transaction.execute(
                    "INSERT INTO standard_lxmf_propagation_peer_items
                     (peer, transient_id, disposition, updated_at)
                     SELECT identity_hash, ?1, 'unhandled', ?2
                     FROM standard_lxmf_propagation_peers WHERE enabled = 1
                     ON CONFLICT(peer, transient_id) DO NOTHING",
                    params![item.transient_id.as_slice(), now],
                )?;
            }
        }
        transaction.commit()?;
        Ok(StandardPropagationIngestOutcome::Accepted)
    }

    pub fn standard_propagation_get(
        &mut self,
        request: StandardPropagationGetRequest<'_>,
    ) -> rusqlite::Result<StandardPropagationGetResult> {
        let StandardPropagationGetRequest {
            peer,
            request_id,
            recipient,
            wants,
            haves,
            inventory,
            response_limit,
            now,
            policy,
        } = request;
        let operation = if inventory {
            StandardPropagationGetOperation::Fetch
        } else if haves.is_some() {
            StandardPropagationGetOperation::Sync
        } else {
            StandardPropagationGetOperation::Download
        };
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        reconcile_deadlines_in_transaction(&transaction, now)?;
        prune_in_transaction(&transaction, now)?;
        upsert_observed_peer(&transaction, &peer, now)?;
        let mut offered_ids = BTreeSet::new();
        if let Some(wants) = wants {
            offered_ids.extend(wants.iter().copied());
        }
        if let Some(haves) = haves {
            offered_ids.extend(haves.iter().copied());
        }
        let mut accepted_ids = BTreeSet::new();
        let mut accepted_bytes = 0usize;
        if let Some(haves) = haves {
            let mut seen = BTreeSet::new();
            for id in haves.iter().filter(|id| seen.insert(**id)) {
                let changed = transaction.execute(
                    "UPDATE standard_lxmf_propagation_items
                     SET state = 'acknowledged', lxmf_data = NULL, stamp = NULL,
                          stored_size = 0, terminal_at = ?3
                     WHERE transient_id = ?1 AND destination = ?2 AND state = 'queued'",
                    params![id.as_slice(), recipient.as_slice(), now],
                )?;
                if changed == 1 {
                    accepted_ids.insert(*id);
                }
            }
        }
        let inventory_ids = if inventory {
            let mut statement = transaction.prepare(
                "SELECT transient_id FROM standard_lxmf_propagation_items
                 WHERE destination = ?1 AND state = 'queued'
                 ORDER BY stored_size, transient_id",
            )?;
            let rows = statement
                .query_map(params![recipient.as_slice()], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let bounded = rows
                .into_iter()
                .map(|value| blob_array(value, "inventory transient"))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for id in &bounded {
                accepted_ids.insert(*id);
            }
            offered_ids.extend(bounded.iter().copied());
            Some(bounded)
        } else {
            None
        };
        let mut payloads = Vec::new();
        let mut accounted = 24usize;
        let mut seen = BTreeSet::new();
        if let Some(wants) = wants {
            for id in wants {
                if !seen.insert(*id) {
                    continue;
                }
                let row: Option<(Vec<u8>, i64)> = transaction
                    .query_row(
                        "SELECT lxmf_data, stored_size FROM standard_lxmf_propagation_items
                         WHERE transient_id = ?1 AND destination = ?2 AND state = 'queued'",
                        params![id.as_slice(), recipient.as_slice()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((data, stored_size)) = row {
                    let item_size = to_usize(stored_size, "get stored size")?.saturating_add(16);
                    if accounted.saturating_add(item_size) <= response_limit {
                        accounted += item_size;
                        accepted_ids.insert(*id);
                        accepted_bytes = accepted_bytes.saturating_add(data.len());
                        payloads.push(data);
                    }
                }
            }
        }
        let stats = stats_in_transaction(&transaction)?;
        if stats.queued_count > policy.queue_max_count
            || stats.stored_bytes > policy.queue_max_bytes
        {
            return Err(invalid("standard propagation queue exceeds capacity during get"));
        }
        let mut material = Vec::with_capacity(48 + offered_ids.len().saturating_mul(32));
        material.extend_from_slice(&request_id);
        material.extend_from_slice(&peer);
        material.extend_from_slice(operation.as_str().as_bytes());
        for id in &offered_ids {
            material.extend_from_slice(id);
        }
        let digest = Sha256::digest(material);
        let mut attempt_id = [0u8; 16];
        attempt_id.copy_from_slice(&digest[..16]);
        let attempt_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1)",
            params![attempt_id.as_slice()],
            |row| row.get(0),
        )?;
        let attempt_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM standard_lxmf_propagation_attempts",
            [],
            |row| row.get(0),
        )?;
        if !attempt_exists && to_usize(attempt_count, "attempt count")? >= MAX_ATTEMPTS {
            return Err(invalid("standard propagation attempt capacity exhausted"));
        }
        let offered_count = offered_ids.len().max(accepted_ids.len());
        let wanted_count = if inventory { 0 } else { accepted_ids.len() };
        let direction =
            if operation == StandardPropagationGetOperation::Sync { "sync" } else { "egress" };
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempts
             (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
              deadline_at, offered_count, wanted_count, accepted_count, accepted_bytes)
             VALUES (?1, ?2, ?3, ?4, 'get', 'completed', ?5, ?5, NULL, ?6, ?7, ?8, ?9)
             ON CONFLICT(attempt_id) DO UPDATE SET
                 updated_at = excluded.updated_at, state = 'completed', stage = 'get',
                 offered_count = excluded.offered_count, wanted_count = excluded.wanted_count,
                 accepted_count = excluded.accepted_count,
                 accepted_bytes = excluded.accepted_bytes,
                 failure_code = NULL, failure_detail = NULL",
            params![
                attempt_id.as_slice(),
                request_id.as_slice(),
                peer.as_slice(),
                direction,
                now,
                to_i64(offered_count, "get offered count")?,
                to_i64(wanted_count, "get wanted count")?,
                to_i64(accepted_ids.len(), "get accepted count")?,
                to_i64(accepted_bytes, "get accepted bytes")?,
            ],
        )?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempt_operations(attempt_id, operation)
             VALUES (?1, ?2)
             ON CONFLICT(attempt_id) DO UPDATE SET operation = excluded.operation",
            params![attempt_id.as_slice(), operation.as_str()],
        )?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_checkpoints
             (peer, direction, completed_stage, item_count, byte_count, last_attempt, updated_at)
             VALUES (?1, ?2, 'get', ?3, ?4, ?5, ?6)
             ON CONFLICT(peer, direction) DO UPDATE SET
                 completed_stage = excluded.completed_stage,
                 item_count = excluded.item_count,
                 byte_count = excluded.byte_count,
                 last_attempt = excluded.last_attempt,
                 updated_at = excluded.updated_at",
            params![
                peer.as_slice(),
                direction,
                to_i64(accepted_ids.len(), "get checkpoint count")?,
                to_i64(accepted_bytes, "get checkpoint bytes")?,
                attempt_id.as_slice(),
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(StandardPropagationGetResult { inventory: inventory_ids, payloads, attempt_id })
    }

    pub fn standard_propagation_stats(
        &mut self,
        now: i64,
        policy: StandardPropagationPolicy,
    ) -> rusqlite::Result<StandardPropagationStats> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        let stats = stats_in_transaction(&transaction)?;
        if stats.queued_count > policy.queue_max_count
            || stats.stored_bytes > policy.queue_max_bytes
        {
            return Err(invalid("standard propagation queue exceeds capacity"));
        }
        transaction.commit()?;
        Ok(stats)
    }

    pub fn standard_propagation_observation(
        &mut self,
        now: i64,
        policy: StandardPropagationPolicy,
    ) -> rusqlite::Result<StandardPropagationObservation> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        reconcile_deadlines_in_transaction(&transaction, now)?;
        prune_in_transaction(&transaction, now)?;
        let queue = transaction.query_row(
            "SELECT
                 COALESCE(SUM(CASE WHEN state = 'queued' THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN state = 'queued' THEN stored_size ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN state = 'acknowledged' THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(CASE WHEN state = 'expired' THEN 1 ELSE 0 END), 0)
             FROM standard_lxmf_propagation_items",
            [],
            |row| {
                Ok(StandardPropagationQueueObservation {
                    queued_count: to_usize(row.get(0)?, "observation queued count")?,
                    queued_bytes: to_usize(row.get(1)?, "observation queued bytes")?,
                    acknowledged_count: to_usize(row.get(2)?, "observation acknowledged count")?,
                    expired_count: to_usize(row.get(3)?, "observation expired count")?,
                })
            },
        )?;
        if queue.queued_count > policy.queue_max_count
            || queue.queued_bytes > policy.queue_max_bytes
        {
            return Err(invalid("standard propagation observation exceeds configured capacity"));
        }

        let selection = transaction
            .query_row(
                "SELECT selected_peer, mode, selected_at
                 FROM standard_lxmf_propagation_selection WHERE singleton = 1",
                [],
                |row| {
                    let peer: Option<Vec<u8>> = row.get(0)?;
                    Ok(StandardPropagationSelection {
                        peer: peer
                            .map(|value| blob_array(value, "observation selected peer"))
                            .transpose()?,
                        mode: row.get(1)?,
                        selected_at: row.get(2)?,
                    })
                },
            )
            .optional()?;

        let mut peers = {
            let mut statement = transaction.prepare(
                "SELECT identity_hash, propagation_destination, origin, enabled,
                        transfer_limit_kb, sync_limit_kb, stamp_cost, stamp_flexibility,
                        peering_cost, first_seen_at, last_seen_at, retry_at, backoff_count,
                        offered_count, wanted_count, accepted_count, accepted_bytes, failure_count
                 FROM standard_lxmf_propagation_peers
                 ORDER BY CASE WHEN identity_hash = (
                         SELECT selected_peer FROM standard_lxmf_propagation_selection
                         WHERE singleton = 1
                     ) THEN 0 ELSE 1 END,
                     last_seen_at DESC, identity_hash ASC LIMIT ?1",
            )?;

            statement
                .query_map(
                    params![to_i64(
                        STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT + 1,
                        "observation peer limit"
                    )?],
                    |row| {
                        let destination: Option<Vec<u8>> = row.get(1)?;
                        let origin: String = row.get(2)?;
                        let transfer_limit: Option<i64> = row.get(4)?;
                        let sync_limit: Option<i64> = row.get(5)?;
                        let stamp_cost: Option<i64> = row.get(6)?;
                        let stamp_flexibility: Option<i64> = row.get(7)?;
                        let peering_cost: Option<i64> = row.get(8)?;
                        Ok(StandardPropagationPeerObservation {
                            identity_hash: blob_array(row.get(0)?, "observation peer")?,
                            propagation_destination: destination
                                .map(|value| blob_array(value, "observation peer destination"))
                                .transpose()?,
                            configured: matches!(origin.as_str(), "configured" | "both"),
                            enabled: row.get(3)?,
                            transfer_limit_kb: transfer_limit
                                .map(|value| to_usize(value, "observation transfer limit"))
                                .transpose()?,
                            sync_limit_kb: sync_limit
                                .map(|value| to_usize(value, "observation sync limit"))
                                .transpose()?,
                            stamp_cost: stamp_cost
                                .map(|value| {
                                    u32::try_from(value)
                                        .map_err(|_| invalid("observation stamp cost"))
                                })
                                .transpose()?,
                            stamp_flexibility: stamp_flexibility
                                .map(|value| {
                                    u32::try_from(value)
                                        .map_err(|_| invalid("observation stamp flexibility"))
                                })
                                .transpose()?,
                            peering_cost: peering_cost
                                .map(|value| {
                                    u32::try_from(value)
                                        .map_err(|_| invalid("observation peering cost"))
                                })
                                .transpose()?,
                            first_seen_at: row.get(9)?,
                            last_seen_at: row.get(10)?,
                            retry_at: row.get(11)?,
                            backoff_count: to_usize(row.get(12)?, "observation backoff count")?,
                            offered_count: to_usize(row.get(13)?, "observation offered count")?,
                            wanted_count: to_usize(row.get(14)?, "observation wanted count")?,
                            accepted_count: to_usize(row.get(15)?, "observation accepted count")?,
                            accepted_bytes: to_usize(row.get(16)?, "observation accepted bytes")?,
                            failure_count: to_usize(row.get(17)?, "observation failure count")?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let peers_truncated = peers.len() > STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT;
        peers.truncate(STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT);

        let mut attempts = {
            let mut statement = transaction.prepare(
                "SELECT a.attempt_id, a.correlation_id, a.peer, a.direction,
                        COALESCE(o.operation, a.stage), a.state, a.started_at,
                        updated_at, deadline_at, offered_count, wanted_count, accepted_count,
                        accepted_bytes, failure_code
                 FROM standard_lxmf_propagation_attempts a
                 LEFT JOIN standard_lxmf_propagation_attempt_operations o
                   ON o.attempt_id = a.attempt_id
                 ORDER BY a.updated_at DESC, a.attempt_id ASC LIMIT ?1",
            )?;

            statement
                .query_map(
                    params![to_i64(
                        STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT + 1,
                        "observation attempt limit"
                    )?],
                    |row| {
                        let peer: Option<Vec<u8>> = row.get(2)?;
                        Ok(StandardPropagationAttemptObservation {
                            attempt_id: blob_array(row.get(0)?, "observation attempt")?,
                            correlation_id: blob_array(row.get(1)?, "observation correlation")?,
                            peer: peer
                                .map(|value| blob_array(value, "observation attempt peer"))
                                .transpose()?,
                            direction: row.get(3)?,
                            stage: row.get(4)?,
                            state: row.get(5)?,
                            started_at: row.get(6)?,
                            updated_at: row.get(7)?,
                            deadline_at: row.get(8)?,
                            offered_count: to_usize(row.get(9)?, "observation attempt offered")?,
                            wanted_count: to_usize(row.get(10)?, "observation attempt wanted")?,
                            accepted_count: to_usize(row.get(11)?, "observation attempt accepted")?,
                            accepted_bytes: to_usize(row.get(12)?, "observation attempt bytes")?,
                            failure_code: row.get(13)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let attempts_truncated = attempts.len() > STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT;
        attempts.truncate(STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT);

        let mut checkpoints = {
            let mut statement = transaction.prepare(
                "SELECT c.peer, c.direction, COALESCE(o.operation, c.completed_stage),
                        c.item_count, c.byte_count, c.last_attempt, c.updated_at
                 FROM standard_lxmf_propagation_checkpoints c
                 LEFT JOIN standard_lxmf_propagation_attempt_operations o
                   ON o.attempt_id = c.last_attempt
                 ORDER BY c.updated_at DESC, c.peer ASC, c.direction ASC LIMIT ?1",
            )?;

            statement
                .query_map(
                    params![to_i64(
                        STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT + 1,
                        "observation checkpoint limit"
                    )?],
                    |row| {
                        let attempt: Option<Vec<u8>> = row.get(5)?;
                        Ok(StandardPropagationCheckpointObservation {
                            peer: blob_array(row.get(0)?, "observation checkpoint peer")?,
                            direction: row.get(1)?,
                            completed_stage: row.get(2)?,
                            item_count: to_usize(row.get(3)?, "observation checkpoint items")?,
                            byte_count: to_usize(row.get(4)?, "observation checkpoint bytes")?,
                            last_attempt: attempt
                                .map(|value| blob_array(value, "observation checkpoint attempt"))
                                .transpose()?,
                            updated_at: row.get(6)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let checkpoints_truncated =
            checkpoints.len() > STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT;
        checkpoints.truncate(STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT);

        let mut failures = {
            let mut statement = transaction.prepare(
                "SELECT code, occurred_at, peer, attempt_id
                 FROM standard_lxmf_propagation_failures
                 ORDER BY occurred_at DESC, failure_id DESC LIMIT ?1",
            )?;

            statement
                .query_map(
                    params![to_i64(
                        STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT + 1,
                        "observation failure limit"
                    )?],
                    |row| {
                        let peer: Option<Vec<u8>> = row.get(2)?;
                        let attempt: Option<Vec<u8>> = row.get(3)?;
                        Ok(StandardPropagationFailureObservation {
                            code: row.get(0)?,
                            occurred_at: row.get(1)?,
                            peer: peer
                                .map(|value| blob_array(value, "observation failure peer"))
                                .transpose()?,
                            attempt_id: attempt
                                .map(|value| blob_array(value, "observation failure attempt"))
                                .transpose()?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let failures_truncated = failures.len() > STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT;
        failures.truncate(STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT);

        transaction.commit()?;
        Ok(StandardPropagationObservation {
            observed_at: now,
            queue,
            selection,
            peers,
            attempts,
            checkpoints,
            failures,
            peers_truncated,
            attempts_truncated,
            checkpoints_truncated,
            failures_truncated,
        })
    }

    pub fn standard_propagation_snapshot(
        &mut self,
        now: i64,
        policy: StandardPropagationPolicy,
    ) -> rusqlite::Result<Vec<StandardPropagationItem>> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        let items = {
            let mut statement = transaction.prepare(
                "SELECT transient_id, destination, lxmf_data, stamp, stamp_value,
                        received_at, expires_at, stored_size
                 FROM standard_lxmf_propagation_items WHERE state = 'queued'
                 ORDER BY transient_id",
            )?;

            statement
                .query_map([], |row| {
                    let stamp_value: i64 = row.get(4)?;
                    Ok(StandardPropagationItem {
                        transient_id: blob_array(row.get(0)?, "snapshot transient")?,
                        destination: blob_array(row.get(1)?, "snapshot destination")?,
                        lxmf_data: row.get(2)?,
                        stamp: blob_array(row.get(3)?, "snapshot stamp")?,
                        stamp_value: u32::try_from(stamp_value)
                            .map_err(|_| invalid("snapshot stamp value"))?,
                        received_at: row.get(5)?,
                        expires_at: row.get(6)?,
                        stored_size: to_usize(row.get(7)?, "snapshot stored size")?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let stats = stats_in_transaction(&transaction)?;
        if stats.queued_count > policy.queue_max_count
            || stats.stored_bytes > policy.queue_max_bytes
        {
            return Err(invalid("standard propagation snapshot exceeds capacity"));
        }
        transaction.commit()?;
        Ok(items)
    }

    pub fn standard_propagation_set_selection(
        &mut self,
        peer: Option<[u8; 16]>,
        mode: &str,
        now: i64,
    ) -> rusqlite::Result<()> {
        if !matches!(mode, "automatic" | "manual" | "disabled") {
            return Err(invalid("invalid standard propagation selection mode"));
        }
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        if let Some(peer) = peer {
            upsert_observed_peer(&transaction, &peer, now)?;
        }
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_selection(singleton, selected_peer, mode, selected_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(singleton) DO UPDATE SET selected_peer = excluded.selected_peer,
                 mode = excluded.mode, selected_at = excluded.selected_at",
            params![peer.map(|value| value.to_vec()), mode, now],
        )?;
        transaction.commit()
    }

    pub fn standard_propagation_selection(
        &self,
    ) -> rusqlite::Result<Option<StandardPropagationSelection>> {
        self.conn
            .query_row(
                "SELECT selected_peer, mode, selected_at
                 FROM standard_lxmf_propagation_selection WHERE singleton = 1",
                [],
                |row| {
                    let peer: Option<Vec<u8>> = row.get(0)?;
                    Ok(StandardPropagationSelection {
                        peer: peer.map(|value| blob_array(value, "selected peer")).transpose()?,
                        mode: row.get(1)?,
                        selected_at: row.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn standard_propagation_checkpoint(
        &self,
        peer: [u8; 16],
        direction: &str,
    ) -> rusqlite::Result<Option<StandardPropagationCheckpoint>> {
        self.conn
            .query_row(
                "SELECT completed_stage, cursor, digest, item_count, byte_count,
                        last_attempt, updated_at
                 FROM standard_lxmf_propagation_checkpoints
                 WHERE peer = ?1 AND direction = ?2",
                params![peer.as_slice(), direction],
                |row| {
                    let digest: Option<Vec<u8>> = row.get(2)?;
                    let attempt: Option<Vec<u8>> = row.get(5)?;
                    Ok(StandardPropagationCheckpoint {
                        peer,
                        direction: direction.to_string(),
                        completed_stage: row.get(0)?,
                        cursor: row.get(1)?,
                        digest: digest
                            .map(|value| blob_array(value, "checkpoint digest"))
                            .transpose()?,
                        item_count: to_usize(row.get(3)?, "checkpoint item count")?,
                        byte_count: to_usize(row.get(4)?, "checkpoint byte count")?,
                        last_attempt: attempt
                            .map(|value| blob_array(value, "checkpoint attempt"))
                            .transpose()?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
    }

    pub fn standard_propagation_record_attempt_failure(
        &mut self,
        attempt_id: [u8; 16],
        peer: [u8; 16],
        code: &str,
        detail: Option<&str>,
        now: i64,
    ) -> rusqlite::Result<()> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        upsert_observed_peer(&transaction, &peer, now)?;
        let attempt: Option<(String, String, i64, i64)> = transaction
            .query_row(
                "SELECT direction, stage, accepted_count, accepted_bytes
                 FROM standard_lxmf_propagation_attempts
                 WHERE attempt_id = ?1 AND peer = ?2 AND state = 'running'",
                params![attempt_id.as_slice(), peer.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((direction, stage, accepted_count, accepted_bytes)) = attempt else {
            transaction.commit()?;
            return Ok(());
        };
        let changed = transaction.execute(
            "UPDATE standard_lxmf_propagation_attempts
             SET state = 'failed', stage = 'transfer', updated_at = ?2,
                 failure_code = ?3, failure_detail = NULL
             WHERE attempt_id = ?1 AND peer = ?4 AND state = 'running'",
            params![attempt_id.as_slice(), now, code, peer.as_slice()],
        )?;
        if changed != 1 {
            return Err(invalid("standard propagation attempt failure race"));
        }
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_checkpoints
             (peer, direction, completed_stage, item_count, byte_count, last_attempt, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(peer, direction) DO UPDATE SET
                 completed_stage = excluded.completed_stage,
                 item_count = excluded.item_count,
                 byte_count = excluded.byte_count,
                 last_attempt = excluded.last_attempt,
                 updated_at = excluded.updated_at
             WHERE excluded.updated_at >= standard_lxmf_propagation_checkpoints.updated_at",
            params![
                peer.as_slice(),
                direction,
                stage,
                accepted_count,
                accepted_bytes,
                attempt_id.as_slice(),
                now,
            ],
        )?;
        record_failure_in_transaction(
            &transaction,
            code,
            detail,
            now,
            Some(&peer),
            None,
            Some(&attempt_id),
        )?;
        prune_in_transaction(&transaction, now)?;
        transaction.commit()
    }

    pub fn standard_propagation_interrupt_attempt(
        &mut self,
        attempt_id: [u8; 16],
        now: i64,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_in_transaction(&transaction, now)?;
        let changed = transaction.execute(
            "UPDATE standard_lxmf_propagation_attempts
             SET state = 'interrupted', updated_at = ?2, failure_code = 'interrupted'
             WHERE attempt_id = ?1 AND state = 'running'",
            params![attempt_id.as_slice(), now],
        )?;
        transaction.commit()?;
        Ok(changed > 0)
    }

    pub fn standard_propagation_failures(
        &self,
        limit: usize,
    ) -> rusqlite::Result<Vec<StandardPropagationFailure>> {
        let limit = limit.min(MAX_FAILURES);
        let mut statement = self.conn.prepare(
            "SELECT code, detail, occurred_at, peer, transient_id, attempt_id
             FROM standard_lxmf_propagation_failures
             ORDER BY occurred_at DESC, failure_id DESC LIMIT ?1",
        )?;

        statement
            .query_map(params![to_i64(limit, "failure query limit")?], |row| {
                let peer: Option<Vec<u8>> = row.get(3)?;
                let transient: Option<Vec<u8>> = row.get(4)?;
                let attempt: Option<Vec<u8>> = row.get(5)?;
                Ok(StandardPropagationFailure {
                    code: row.get(0)?,
                    detail: row.get(1)?,
                    occurred_at: row.get(2)?,
                    peer: peer.map(|value| blob_array(value, "failure peer")).transpose()?,
                    transient_id: transient
                        .map(|value| blob_array(value, "failure transient"))
                        .transpose()?,
                    attempt_id: attempt
                        .map(|value| blob_array(value, "failure attempt"))
                        .transpose()?,
                })
            })?
            .collect()
    }

    pub fn standard_propagation_selected_peer(
        &self,
    ) -> rusqlite::Result<Option<StandardPropagationPeer>> {
        self.conn
            .query_row(
                "SELECT p.identity_hash, p.propagation_destination, p.origin, p.enabled,
                        p.transfer_limit_kb, p.sync_limit_kb, p.stamp_cost,
                        p.stamp_flexibility, p.peering_cost, p.last_seen_at
                 FROM standard_lxmf_propagation_selection s
                 JOIN standard_lxmf_propagation_peers p ON p.identity_hash = s.selected_peer
                 WHERE s.singleton = 1 AND s.mode != 'disabled' AND p.enabled = 1
                   AND p.propagation_destination IS NOT NULL",
                [],
                |row| {
                    let identity: Vec<u8> = row.get(0)?;
                    let destination: Vec<u8> = row.get(1)?;
                    let origin: String = row.get(2)?;
                    Ok(StandardPropagationPeer {
                        identity_hash: blob_array(identity, "selected peer identity")?,
                        propagation_destination: Some(blob_array(
                            destination,
                            "selected propagation destination",
                        )?),
                        configured: matches!(origin.as_str(), "configured" | "both"),
                        enabled: row.get(3)?,
                        transfer_limit_kb: row
                            .get::<_, Option<i64>>(4)?
                            .map(|value| to_usize(value, "selected transfer limit"))
                            .transpose()?,
                        sync_limit_kb: row
                            .get::<_, Option<i64>>(5)?
                            .map(|value| to_usize(value, "selected sync limit"))
                            .transpose()?,
                        stamp_cost: row
                            .get::<_, Option<i64>>(6)?
                            .map(|value| {
                                u32::try_from(value).map_err(|_| invalid("selected stamp cost"))
                            })
                            .transpose()?,
                        stamp_flexibility: row
                            .get::<_, Option<i64>>(7)?
                            .map(|value| {
                                u32::try_from(value)
                                    .map_err(|_| invalid("selected stamp flexibility"))
                            })
                            .transpose()?,
                        peering_cost: row
                            .get::<_, Option<i64>>(8)?
                            .map(|value| {
                                u32::try_from(value).map_err(|_| invalid("selected peering cost"))
                            })
                            .transpose()?,
                        observed_at: row.get(9)?,
                    })
                },
            )
            .optional()
    }

    pub fn standard_propagation_spool_outbound(
        &mut self,
        job: &StandardPropagationClientJob,
    ) -> rusqlite::Result<()> {
        if job.state != "spooled" || job.canonical_wire.is_some() {
            return Err(invalid("materialization requires a spooled propagation job"));
        }
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let route_active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN outbound_routes r ON r.message_id = m.id
             WHERE m.id = ?1 AND r.actual_method = 'propagated'
               AND r.state IN ('queued','sending'))",
            params![&job.message_id],
            |row| row.get(0),
        )?;
        if !route_active {
            return Err(invalid("outbound propagation route is not resumable"));
        }
        let existing: Option<ExistingMaterializedJob> = transaction
            .query_row(
                "SELECT state, transient_id, lxmf_data, stamp
                 FROM standard_lxmf_propagation_client_jobs WHERE message_id = ?1",
                params![&job.message_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((state, transient, data, stamp)) = existing else {
            return Err(invalid("outbound propagation preparation is missing"));
        };
        if state != "preparing" {
            if state == "spooled"
                && transient.as_deref() == job.transient_id.as_ref().map(<[u8; 32]>::as_slice)
                && data.as_deref() == job.lxmf_data.as_deref()
                && stamp.as_deref() == job.stamp.as_ref().map(<[u8; 32]>::as_slice)
            {
                transaction.commit()?;
                return Ok(());
            } else {
                return Err(invalid("conflicting outbound standard propagation spool"));
            }
        }
        let transient_id =
            job.transient_id.ok_or_else(|| invalid("materialized job transient is missing"))?;
        let lxmf_data = job
            .lxmf_data
            .as_deref()
            .ok_or_else(|| invalid("materialized job ciphertext is missing"))?;
        let stamp = job.stamp.ok_or_else(|| invalid("materialized job stamp is missing"))?;
        if lxmf_data.len() < lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1
            || lxmf_data[..16] != job.destination
            || <[u8; 32]>::from(Sha256::digest(lxmf_data)) != transient_id
        {
            return Err(invalid("invalid materialized standard propagation payload"));
        }
        let changed = transaction.execute(
            "UPDATE standard_lxmf_propagation_client_jobs
             SET transient_id = ?2, canonical_wire = NULL, lxmf_data = ?3, stamp = ?4,
                 state = 'spooled', updated_at = ?5
             WHERE message_id = ?1 AND state = 'preparing' AND destination = ?6
               AND peer = ?7 AND propagation_destination = ?8 AND stamp_cost = ?9
               AND peering_cost = ?10 AND correlation_id = ?11 AND attempt_id = ?12",
            params![
                &job.message_id,
                transient_id.as_slice(),
                lxmf_data,
                stamp.as_slice(),
                job.updated_at,
                job.destination.as_slice(),
                job.peer.as_slice(),
                job.propagation_destination.as_slice(),
                i64::from(job.stamp_cost),
                i64::from(job.peering_cost),
                job.correlation_id.as_slice(),
                job.attempt_id.as_slice(),
            ],
        )?;
        if changed != 1 {
            return Err(invalid(
                "persisted propagation preparation changed during materialization",
            ));
        }
        insert_materialized_outbound_children(&transaction, job, transient_id)?;
        transaction.commit()
    }

    pub fn standard_propagation_client_job(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Option<StandardPropagationClientJob>> {
        self.conn
            .query_row(
                "SELECT message_id, transient_id, destination, canonical_wire, lxmf_data, stamp, peer,
                        propagation_destination, stamp_cost, peering_cost, correlation_id,
                        attempt_id, state,
                        created_at, updated_at
                 FROM standard_lxmf_propagation_client_jobs WHERE message_id = ?1",
                params![message_id],
                |row| {
                    Ok(StandardPropagationClientJob {
                        message_id: row.get(0)?,
                        transient_id: row
                            .get::<_, Option<Vec<u8>>>(1)?
                            .map(|value| blob_array(value, "client job transient"))
                            .transpose()?,
                        destination: blob_array(row.get(2)?, "client job destination")?,
                        canonical_wire: row.get(3)?,
                        lxmf_data: row.get(4)?,
                        stamp: row
                            .get::<_, Option<Vec<u8>>>(5)?
                            .map(|value| blob_array(value, "client job stamp"))
                            .transpose()?,
                        peer: blob_array(row.get(6)?, "client job peer")?,
                        propagation_destination: blob_array(
                            row.get(7)?,
                            "client job propagation destination",
                        )?,
                        stamp_cost: u32::try_from(row.get::<_, i64>(8)?)
                            .map_err(|_| invalid("client job stamp cost"))?,
                        peering_cost: u32::try_from(row.get::<_, i64>(9)?)
                            .map_err(|_| invalid("client job peering cost"))?,
                        correlation_id: blob_array(row.get(10)?, "client job correlation")?,
                        attempt_id: blob_array(row.get(11)?, "client job attempt")?,
                        state: row.get(12)?,
                        created_at: row.get(13)?,
                        updated_at: row.get(14)?,
                    })
                },
            )
            .optional()
    }

    pub fn standard_propagation_recoverable_outbound_jobs(
        &self,
        now_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut statement = self.conn.prepare(
            "SELECT j.message_id, r.deadline_unix_ms
             FROM standard_lxmf_propagation_client_jobs j
             JOIN outbound_routes r ON r.message_id = j.message_id
              WHERE j.state IN ('preparing','spooled','uploading')
               AND r.state IN ('queued','sending') AND r.deadline_unix_ms > ?1
             ORDER BY j.updated_at, j.message_id LIMIT ?2",
        )?;

        statement
            .query_map(
                params![now_unix_ms, to_i64(limit.min(64), "recoverable job limit")?],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect()
    }

    pub fn standard_propagation_resume_outbound_attempt(
        &mut self,
        message_id: &str,
        now: i64,
        deadline: i64,
    ) -> rusqlite::Result<[u8; 16]> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, String) = transaction.query_row(
            "SELECT j.correlation_id, j.attempt_id, j.peer, j.transient_id, a.state
             FROM standard_lxmf_propagation_client_jobs j
             JOIN outbound_routes r ON r.message_id = j.message_id
             JOIN standard_lxmf_propagation_attempts a ON a.attempt_id = j.attempt_id
             WHERE j.message_id = ?1 AND j.state IN ('spooled','uploading')
               AND r.state IN ('queued','sending')",
            params![message_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        let correlation_id = blob_array::<16>(row.0, "outbound attempt correlation")?;
        let current_attempt = blob_array::<16>(row.1, "outbound current attempt")?;
        let peer = blob_array::<16>(row.2, "outbound attempt peer")?;
        let transient_id = blob_array::<32>(row.3, "outbound attempt transient")?;
        if row.4 == "running" {
            transaction.commit()?;
            return Ok(current_attempt);
        }
        let sequence: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM standard_lxmf_propagation_attempts WHERE correlation_id = ?1",
            params![correlation_id.as_slice()],
            |row| row.get(0),
        )?;
        let digest = Sha256::digest(
            [correlation_id.as_slice(), &sequence.saturating_add(1).to_be_bytes()].concat(),
        );
        let mut attempt_id = [0u8; 16];
        attempt_id.copy_from_slice(&digest[..16]);
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempts
             (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
              deadline_at, offered_count, wanted_count, accepted_count, accepted_bytes)
             VALUES (?1, ?2, ?3, 'egress', 'offer', 'running', ?4, ?4, ?5, 1, 0, 0, 0)",
            params![
                attempt_id.as_slice(),
                correlation_id.as_slice(),
                peer.as_slice(),
                now,
                deadline.max(now),
            ],
        )?;
        transaction.execute(
            "UPDATE standard_lxmf_propagation_client_jobs
             SET attempt_id = ?2, updated_at = ?3 WHERE message_id = ?1",
            params![message_id, attempt_id.as_slice(), now],
        )?;
        transaction.execute(
            "UPDATE standard_lxmf_propagation_message_links
             SET attempt_id = ?2, updated_at = ?3
             WHERE message_id = ?1 AND relation = 'outbound' AND state = 'spooled'",
            params![message_id, attempt_id.as_slice(), now],
        )?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_client_attempt_items
             (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, 'offered', ?3)",
            params![attempt_id.as_slice(), transient_id.as_slice(), now],
        )?;
        transaction.commit()?;
        Ok(attempt_id)
    }

    pub fn standard_propagation_mark_upload_accepted(
        &mut self,
        message_id: &str,
        now: i64,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE standard_lxmf_propagation_client_jobs SET state = 'accepted', updated_at = ?2
             WHERE message_id = ?1 AND state IN ('spooled','uploading')",
            params![message_id, now],
        )?;
        if changed > 0 {
            transaction.execute(
                "UPDATE standard_lxmf_propagation_message_links
                 SET state = 'accepted', updated_at = ?2
                 WHERE message_id = ?1 AND relation = 'outbound' AND state = 'spooled'",
                params![message_id, now],
            )?;
            transaction.execute(
                "UPDATE standard_lxmf_propagation_attempts
                 SET stage = 'complete', state = 'completed', updated_at = ?2,
                     wanted_count = 1, accepted_count = 1,
                     accepted_bytes = (SELECT length(lxmf_data)
                                       FROM standard_lxmf_propagation_client_jobs
                                       WHERE message_id = ?1),
                     failure_code = NULL, failure_detail = NULL
                 WHERE attempt_id = (SELECT attempt_id
                                     FROM standard_lxmf_propagation_client_jobs
                                     WHERE message_id = ?1) AND state = 'running'",
                params![message_id, now],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    pub fn standard_propagation_link_inbound(
        &mut self,
        message_id: &str,
        transient_id: [u8; 32],
        attempt_id: [u8; 16],
        peer: [u8; 16],
        now: i64,
    ) -> rusqlite::Result<()> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        link_inbound_in_transaction(&transaction, message_id, transient_id, attempt_id, peer, now)?;
        transaction.commit()
    }

    pub fn standard_propagation_pending_haves(
        &self,
        peer: [u8; 16],
        limit: usize,
    ) -> rusqlite::Result<Vec<[u8; 32]>> {
        let limit = limit.min(1024);
        let mut statement = self.conn.prepare(
            "SELECT transient_id FROM standard_lxmf_propagation_message_links
             WHERE relation = 'inbound' AND state = 'pending_ack' AND peer = ?1
             ORDER BY updated_at, transient_id LIMIT ?2",
        )?;

        statement
            .query_map(params![peer.as_slice(), to_i64(limit, "pending haves limit")?], |row| {
                blob_array(row.get(0)?, "pending have transient")
            })?
            .collect()
    }

    pub fn standard_propagation_mark_haves_acknowledged(
        &mut self,
        peer: [u8; 16],
        transient_ids: &[[u8; 32]],
        attempt_id: [u8; 16],
        now: i64,
    ) -> rusqlite::Result<usize> {
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changed = 0usize;
        for transient_id in transient_ids.iter().take(1024) {
            changed += transaction.execute(
                "UPDATE standard_lxmf_propagation_message_links
                 SET state = 'acknowledged', updated_at = ?2
                 WHERE transient_id = ?1 AND peer = ?3
                   AND relation = 'inbound' AND state = 'pending_ack'",
                params![transient_id.as_slice(), now, peer.as_slice()],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO standard_lxmf_propagation_client_attempt_items
                 (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, 'accepted', ?3)",
                params![attempt_id.as_slice(), transient_id.as_slice(), now],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn standard_propagation_begin_client_attempt(
        &mut self,
        attempt_id: [u8; 16],
        peer: [u8; 16],
        operation: StandardPropagationGetOperation,
        ids: &[[u8; 32]],
        role: &str,
        now: i64,
        deadline: i64,
    ) -> rusqlite::Result<()> {
        if ids.len() > 1024 || !matches!(role, "inventory" | "offered" | "accepted") {
            return Err(invalid("invalid standard propagation client attempt items"));
        }
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_observed_peer(&transaction, &peer, now)?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempts
             (attempt_id, correlation_id, peer, direction, stage, state, started_at, updated_at,
              deadline_at, offered_count, wanted_count, accepted_count, accepted_bytes)
             VALUES (?1, ?1, ?2, 'sync', 'get', 'running', ?3, ?3, ?4, ?5, 0, 0, 0)",
            params![
                attempt_id.as_slice(),
                peer.as_slice(),
                now,
                deadline,
                to_i64(ids.len(), "client attempt item count")?,
            ],
        )?;
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_attempt_operations(attempt_id, operation)
             VALUES (?1, ?2)",
            params![attempt_id.as_slice(), operation.as_str()],
        )?;
        for id in ids {
            transaction.execute(
                "INSERT INTO standard_lxmf_propagation_client_attempt_items
                 (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![attempt_id.as_slice(), id.as_slice(), role, now],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO standard_lxmf_propagation_attempt_items
                 (attempt_id, transient_id, role) VALUES (?1, ?2, 'offered')",
                params![attempt_id.as_slice(), id.as_slice()],
            )?;
        }
        transaction.commit()
    }

    pub fn standard_propagation_complete_client_attempt(
        &mut self,
        attempt_id: [u8; 16],
        peer: [u8; 16],
        ids: &[[u8; 32]],
        role: &str,
        accepted_bytes: usize,
        now: i64,
    ) -> rusqlite::Result<()> {
        if ids.len() > 1024 || !matches!(role, "inventory" | "accepted" | "returned") {
            return Err(invalid("invalid completed standard propagation client attempt"));
        }
        let transaction = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            transaction.execute(
                "INSERT OR IGNORE INTO standard_lxmf_propagation_client_attempt_items
                 (attempt_id, transient_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![attempt_id.as_slice(), id.as_slice(), role, now],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE standard_lxmf_propagation_attempts
             SET stage = 'complete', state = 'completed', updated_at = ?3,
                 offered_count = MAX(offered_count, ?4), accepted_count = ?4,
                 accepted_bytes = ?5, failure_code = NULL, failure_detail = NULL
             WHERE attempt_id = ?1 AND peer = ?2 AND state = 'running'",
            params![
                attempt_id.as_slice(),
                peer.as_slice(),
                now,
                to_i64(ids.len(), "completed client item count")?,
                to_i64(accepted_bytes, "completed client bytes")?,
            ],
        )?;
        if changed != 1 {
            return Err(invalid("standard propagation client attempt is not running"));
        }
        transaction.execute(
            "INSERT INTO standard_lxmf_propagation_checkpoints
             (peer, direction, completed_stage, item_count, byte_count, last_attempt, updated_at)
             VALUES (?1, 'sync', 'complete', ?2, ?3, ?4, ?5)
             ON CONFLICT(peer, direction) DO UPDATE SET completed_stage = 'complete',
                 item_count = excluded.item_count, byte_count = excluded.byte_count,
                 last_attempt = excluded.last_attempt, updated_at = excluded.updated_at",
            params![
                peer.as_slice(),
                to_i64(ids.len(), "client checkpoint items")?,
                to_i64(accepted_bytes, "client checkpoint bytes")?,
                attempt_id.as_slice(),
                now,
            ],
        )?;
        transaction.commit()
    }

    pub fn standard_propagation_links_for_message(
        &self,
        message_id: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<StandardPropagationMessageLink>> {
        let mut statement = self.conn.prepare(
            "SELECT message_id, transient_id, relation, attempt_id, peer, state, created_at, updated_at
             FROM standard_lxmf_propagation_message_links WHERE message_id = ?1
             ORDER BY updated_at DESC, transient_id LIMIT ?2",
        )?;

        statement
            .query_map(params![message_id, to_i64(limit.min(64), "message link limit")?], |row| {
                let attempt: Option<Vec<u8>> = row.get(3)?;
                let peer: Option<Vec<u8>> = row.get(4)?;
                Ok(StandardPropagationMessageLink {
                    message_id: row.get(0)?,
                    transient_id: blob_array(row.get(1)?, "message link transient")?,
                    relation: row.get(2)?,
                    attempt_id: attempt
                        .map(|value| blob_array(value, "message link attempt"))
                        .transpose()?,
                    peer: peer.map(|value| blob_array(value, "message link peer")).transpose()?,
                    state: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect()
    }

    pub fn standard_propagation_message_for_transient(
        &self,
        transient_id: [u8; 32],
    ) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT message_id FROM standard_lxmf_propagation_message_links WHERE transient_id = ?1",
                params![transient_id.as_slice()],
                |row| row.get(0),
            )
            .optional()
    }

    #[cfg(test)]
    pub(crate) fn standard_propagation_fail_inserts_for_test(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TEMP TRIGGER fail_standard_propagation_insert
             BEFORE INSERT ON standard_lxmf_propagation_items
             BEGIN SELECT RAISE(ABORT, 'injected standard propagation insert failure'); END;",
        )
    }

    #[cfg(test)]
    pub(crate) fn standard_propagation_fail_job_insert_for_test(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TEMP TRIGGER fail_standard_propagation_job_insert
             BEFORE INSERT ON standard_lxmf_propagation_client_jobs
             BEGIN SELECT RAISE(ABORT, 'injected propagation job insert failure'); END;",
        )
    }

    #[cfg(test)]
    pub(crate) fn standard_propagation_fail_observation_for_test(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "INSERT INTO standard_lxmf_propagation_failures(code, occurred_at)
             VALUES ('observation_failure', 0);
             CREATE TEMP TRIGGER fail_standard_propagation_observation
             BEFORE DELETE ON standard_lxmf_propagation_failures
             BEGIN SELECT RAISE(ABORT, 'injected observation failure'); END;",
        )
    }

    #[cfg(test)]
    pub(crate) fn standard_propagation_attempt_state_for_test(
        &self,
        attempt_id: [u8; 16],
    ) -> rusqlite::Result<String> {
        self.conn.query_row(
            "SELECT state FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1",
            params![attempt_id.as_slice()],
            |row| row.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn policy() -> StandardPropagationPolicy {
        StandardPropagationPolicy {
            queue_max_count: 4,
            queue_max_bytes: 16 * 1024 * 1024,
            expiry_secs: 30 * 24 * 60 * 60,
        }
    }

    fn item(destination: [u8; 16], fill: u8, now: i64) -> StandardPropagationItem {
        let mut data = vec![fill; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        data[..16].copy_from_slice(&destination);
        let transient_id = Sha256::digest(&data).into();
        StandardPropagationItem {
            transient_id,
            destination,
            stored_size: data.len() + 32,
            lxmf_data: data,
            stamp: [fill; 32],
            stamp_value: 0,
            received_at: now,
            expires_at: now + policy().expiry_secs,
        }
    }

    fn ingest(
        store: &mut MessagesStore,
        items: &[StandardPropagationItem],
        source_peer: Option<[u8; 16]>,
        attempt_id: Option<[u8; 16]>,
        now: i64,
        policy: StandardPropagationPolicy,
    ) -> rusqlite::Result<StandardPropagationIngestOutcome> {
        store.standard_propagation_ingest_batch(StandardPropagationIngestRequest {
            items,
            source_peer,
            attempt: attempt_id.map_or(
                StandardPropagationAttemptStatus::Untracked,
                StandardPropagationAttemptStatus::Complete,
            ),
            protocol: StandardPropagationProtocolStatus::Valid,
            now,
            policy,
        })
    }

    fn marked_schema_with_replacement(table_name: &str, replacement_sql: &str) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations(id TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
            )
            .unwrap();
        for table in TABLES {
            connection
                .execute_batch(if table.name == table_name {
                    replacement_sql
                } else {
                    table.create_sql
                })
                .unwrap();
        }
        connection.execute_batch(INDEX_SQL).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(id, applied_at) VALUES (?1, 0)",
                params![STANDARD_PROPAGATION_MIGRATION],
            )
            .unwrap();
        connection
    }

    #[test]
    fn v10_migration_is_idempotent_strict_and_marker_fails_closed() {
        let mut store = MessagesStore::in_memory().unwrap();
        ensure_standard_propagation_schema(&mut store.conn).unwrap();
        let sql: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'table' AND name = 'standard_lxmf_propagation_items'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sql.to_ascii_uppercase().contains(" STRICT"));
        assert!(
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_items
                 (transient_id, destination, stamp_value, received_at, expires_at,
                  stored_size, state, terminal_at)
                 VALUES (?1, ?2, 0, 0, 1, 0, 'bad', NULL)",
                    params![vec![0u8; 32], vec![0u8; 16]],
                )
                .is_err()
        );
        let mut corrupt_data = vec![0x22u8; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        corrupt_data[..16].copy_from_slice(&[0x23; 16]);
        store
            .conn
            .execute(
                "INSERT INTO standard_lxmf_propagation_items
                 (transient_id, destination, lxmf_data, stamp, stamp_value, received_at,
                  expires_at, stored_size, state, terminal_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 100, ?5, 'queued', NULL)",
                params![
                    [0x24u8; 32].as_slice(),
                    [0x23u8; 16].as_slice(),
                    corrupt_data,
                    [0x25u8; 32].as_slice(),
                    (lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1 + 32) as i64,
                ],
            )
            .unwrap();
        assert!(store.standard_propagation_reconcile_startup(0, policy()).is_err());

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE schema_migrations(id TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);
             CREATE TABLE standard_lxmf_propagation_items(transient_id BLOB PRIMARY KEY);
             INSERT INTO schema_migrations(id, applied_at)
             VALUES ('2026-08-24-standard-lxmf-propagation-v10', 0);",
        )
        .unwrap();
        assert!(ensure_standard_propagation_schema(&mut conn).is_err());

        let mut collision = Connection::open_in_memory().unwrap();
        collision
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations(id TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);
                 CREATE TABLE standard_lxmf_propagation_items(transient_id BLOB PRIMARY KEY);",
            )
            .unwrap();
        assert!(ensure_standard_propagation_schema(&mut collision).is_err());
        let marker_count: i64 = collision
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE id = ?1",
                params![STANDARD_PROPAGATION_MIGRATION],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 0);

        let mut unexpected_trigger = MessagesStore::in_memory().unwrap();
        unexpected_trigger
            .conn
            .execute_batch(
                "CREATE TRIGGER unexpected_propagation_trigger
                 AFTER INSERT ON standard_lxmf_propagation_items BEGIN SELECT 1; END;",
            )
            .unwrap();
        assert!(ensure_standard_propagation_schema(&mut unexpected_trigger.conn).is_err());

        let mut malformed_index = MessagesStore::in_memory().unwrap();
        malformed_index
            .conn
            .execute_batch(
                "DROP INDEX idx_standard_lxmf_propagation_items_expiry;
                 CREATE INDEX idx_standard_lxmf_propagation_items_expiry
                 ON standard_lxmf_propagation_items(expires_at, state);",
            )
            .unwrap();
        assert!(ensure_standard_propagation_schema(&mut malformed_index.conn).is_err());
    }

    #[test]
    fn v12_correlation_migration_is_strict_idempotent_and_collision_safe() {
        let mut store = MessagesStore::in_memory().unwrap();
        ensure_standard_propagation_schema(&mut store.conn).unwrap();
        assert!(correlation_schema_is_valid(&store.conn).unwrap());
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE id = ?1",
                    params![STANDARD_PROPAGATION_CORRELATION_MIGRATION],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert!(
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_message_links
                 (transient_id, message_id, relation, peer, state, created_at, updated_at)
                 VALUES (?1, ?2, 'inbound', ?3, 'spooled', 0, 0)",
                    params![[1u8; 32].as_slice(), "11".repeat(32), [2u8; 16].as_slice()],
                )
                .is_err()
        );

        let root = tempdir().unwrap();
        let path = root.path().join("v12-restart.db");
        drop(MessagesStore::open(&path).unwrap());
        let mut reopened = MessagesStore::open(&path).unwrap();
        ensure_standard_propagation_schema(&mut reopened.conn).unwrap();
        assert!(correlation_schema_is_valid(&reopened.conn).unwrap());

        store
            .conn
            .execute_batch(
                "DELETE FROM schema_migrations
                   WHERE id = '2026-08-25-standard-lxmf-propagation-correlation-v12';
                 DROP TABLE standard_lxmf_propagation_client_attempt_items;
                 DROP TABLE standard_lxmf_propagation_client_jobs;
                 DROP TABLE standard_lxmf_propagation_message_links;
                 CREATE TABLE standard_lxmf_propagation_message_links(transient_id BLOB PRIMARY KEY);",
            )
            .unwrap();
        assert!(ensure_standard_propagation_schema(&mut store.conn).is_err());
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE id = ?1",
                    params![STANDARD_PROPAGATION_CORRELATION_MIGRATION],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );

        let mut historical = Connection::open_in_memory().unwrap();
        historical
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations(id TEXT PRIMARY KEY, applied_at INTEGER NOT NULL);",
            )
            .unwrap();
        for table in TABLES {
            historical.execute_batch(table.create_sql).unwrap();
        }
        historical.execute_batch(INDEX_SQL).unwrap();
        historical.execute_batch(ATTEMPT_OPERATIONS_TABLE_SQL).unwrap();
        historical
            .execute(
                "INSERT INTO schema_migrations(id, applied_at) VALUES (?1, 1), (?2, 2)",
                params![STANDARD_PROPAGATION_MIGRATION, STANDARD_PROPAGATION_OBSERVATION_MIGRATION],
            )
            .unwrap();
        let active = item([0x91; 16], 0x92, 10);
        historical
            .execute(
                "INSERT INTO standard_lxmf_propagation_items
                 (transient_id, destination, lxmf_data, stamp, stamp_value, received_at,
                  expires_at, stored_size, state, terminal_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 10, ?5, ?6, 'queued', NULL)",
                params![
                    active.transient_id.as_slice(),
                    active.destination.as_slice(),
                    &active.lxmf_data,
                    active.stamp.as_slice(),
                    active.expires_at,
                    i64::try_from(active.stored_size).unwrap(),
                ],
            )
            .unwrap();
        ensure_standard_propagation_schema(&mut historical).unwrap();
        assert!(correlation_schema_is_valid(&historical).unwrap());
        let preserved: (Vec<u8>, Vec<u8>, String) = historical
            .query_row(
                "SELECT lxmf_data, stamp, state FROM standard_lxmf_propagation_items
                 WHERE transient_id = ?1",
                params![active.transient_id.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(preserved, (active.lxmf_data, active.stamp.to_vec(), "queued".into()));
    }

    #[test]
    fn message_transient_relationships_are_global_idempotent_and_tombstoned() {
        use crate::storage::messages::{MessageRecord, OutboundRouteRecord};

        let mut store = MessagesStore::in_memory().unwrap();
        let outbound_id = "31".repeat(32);
        let outbound = MessageRecord {
            id: outbound_id.clone(),
            source: "11".repeat(16),
            destination: "22".repeat(16),
            title: String::new(),
            content: "outbound".into(),
            timestamp: 1,
            direction: "out".into(),
            fields: None,
            receipt_status: Some("queued".into()),
            read: true,
        };
        let route = OutboundRouteRecord {
            message_id: outbound_id.clone(),
            requested_method: "propagated".into(),
            actual_method: "propagated".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: outbound_id.clone(),
            retry_of: None,
            deadline_unix_ms: 10_000,
            state: "queued".into(),
            attempt_count: 0,
        };
        let mut data = vec![0x44; lxmf::propagation::MIN_PROPAGATED_LXMF_BYTES + 1];
        data[..16].copy_from_slice(&[0x22; 16]);
        let transient_id: [u8; 32] = Sha256::digest(&data).into();
        let job = StandardPropagationClientJob {
            message_id: outbound_id.clone(),
            transient_id: Some(transient_id),
            destination: [0x22; 16],
            canonical_wire: None,
            lxmf_data: Some(data.clone()),
            stamp: Some([0x55; 32]),
            peer: [0x66; 16],
            propagation_destination: [0x77; 16],
            stamp_cost: 0,
            peering_cost: 18,
            correlation_id: [0x88; 16],
            attempt_id: [0x88; 16],
            state: "spooled".into(),
            created_at: 1,
            updated_at: 1,
        };
        store
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash: job.peer,
                propagation_destination: Some(job.propagation_destination),
                configured: true,
                enabled: true,
                transfer_limit_kb: None,
                sync_limit_kb: None,
                stamp_cost: Some(0),
                stamp_flexibility: Some(0),
                peering_cost: Some(0),
                observed_at: 1,
            })
            .unwrap();
        store
            .insert_outbound_message_with_attachments_and_propagation(
                &outbound,
                &route,
                None,
                &[],
                data.len(),
                Some(&job),
            )
            .unwrap();
        assert_eq!(store.standard_propagation_client_job(&outbound_id).unwrap(), Some(job));
        assert!(store.standard_propagation_mark_upload_accepted(&outbound_id, 2).unwrap());
        assert_eq!(store.outbound_route(&outbound_id).unwrap().unwrap().state, "queued");
        store
            .conn
            .execute(
                "UPDATE outbound_routes SET state = 'sent' WHERE message_id = ?1",
                params![&outbound_id],
            )
            .unwrap();
        assert!(store.delete_message(&outbound_id).unwrap());
        assert!(store.standard_propagation_client_job(&outbound_id).unwrap().is_none());
        assert_eq!(
            store.standard_propagation_links_for_message(&outbound_id, 64).unwrap()[0].state,
            "deleted"
        );

        let inbound_id = "41".repeat(32);
        let second_id = "42".repeat(32);
        for id in [&inbound_id, &second_id] {
            store
                .insert_message_if_absent(&MessageRecord {
                    id: id.clone(),
                    source: "aa".repeat(16),
                    destination: "bb".repeat(16),
                    title: String::new(),
                    content: "inbound".into(),
                    timestamp: 2,
                    direction: "in".into(),
                    fields: None,
                    receipt_status: None,
                    read: false,
                })
                .unwrap();
        }
        store.standard_propagation_link_inbound(&inbound_id, [1; 32], [3; 16], [4; 16], 2).unwrap();
        store.standard_propagation_link_inbound(&inbound_id, [2; 32], [3; 16], [4; 16], 3).unwrap();
        store.standard_propagation_link_inbound(&inbound_id, [2; 32], [3; 16], [4; 16], 4).unwrap();
        store.standard_propagation_link_inbound(&inbound_id, [1; 32], [6; 16], [7; 16], 4).unwrap();
        assert_eq!(store.standard_propagation_links_for_message(&inbound_id, 64).unwrap().len(), 3);
        assert!(
            store
                .standard_propagation_link_inbound(&second_id, [1; 32], [5; 16], [4; 16], 5)
                .is_err()
        );
        assert!(store.delete_message(&inbound_id).unwrap());
        assert!(
            store
                .standard_propagation_links_for_message(&inbound_id, 64)
                .unwrap()
                .iter()
                .all(|link| link.state == "deleted")
        );

        let opaque = item([0x90; 16], 0x91, 10);
        let opaque_id = opaque.transient_id;
        assert_eq!(
            ingest(&mut store, &[opaque], None, None, 10, policy()).unwrap(),
            StandardPropagationIngestOutcome::Accepted
        );
        assert_eq!(store.standard_propagation_message_for_transient(opaque_id).unwrap(), None);
    }

    #[test]
    fn pending_haves_survive_restart_and_are_peer_scoped() {
        use crate::storage::messages::MessageRecord;

        let root = tempdir().unwrap();
        let path = root.path().join("pending-haves.db");
        let message_id = "51".repeat(32);
        let transient_id = [0x52; 32];
        let peer = [0x53; 16];
        let other_peer = [0x59; 16];
        {
            let mut store = MessagesStore::open(&path).unwrap();
            store
                .insert_message_if_absent(&MessageRecord {
                    id: message_id.clone(),
                    source: "54".repeat(16),
                    destination: "55".repeat(16),
                    title: String::new(),
                    content: "pending".into(),
                    timestamp: 1,
                    direction: "in".into(),
                    fields: None,
                    receipt_status: None,
                    read: false,
                })
                .unwrap();
            store
                .standard_propagation_link_inbound(&message_id, transient_id, [0x56; 16], peer, 1)
                .unwrap();
            store
                .standard_propagation_link_inbound(
                    &message_id,
                    transient_id,
                    [0x5a; 16],
                    other_peer,
                    1,
                )
                .unwrap();
        }
        let mut reopened = MessagesStore::open(&path).unwrap();
        assert_eq!(
            reopened.standard_propagation_pending_haves(peer, 64).unwrap(),
            vec![transient_id]
        );
        assert!(reopened.standard_propagation_pending_haves([0x57; 16], 64).unwrap().is_empty());
        assert_eq!(
            reopened
                .standard_propagation_mark_haves_acknowledged(peer, &[transient_id], [0x58; 16], 2,)
                .unwrap(),
            1
        );
        assert!(reopened.standard_propagation_pending_haves(peer, 64).unwrap().is_empty());
        assert_eq!(
            reopened.standard_propagation_pending_haves(other_peer, 64).unwrap(),
            vec![transient_id]
        );
    }

    #[test]
    fn v10_marked_schema_rejects_missing_defaults_and_weakened_checks() {
        assert_eq!(
            normalize_schema_sql(
                "CREATE TABLE \"Example\" (\"Value\" INTEGER CHECK(typeof(\"Value\") = 'integer')) STRICT;"
            ),
            normalize_schema_sql(
                "create table example(value integer check(TYPEOF(value)='integer')) strict"
            )
        );

        let missing_default = PEERS_TABLE_SQL.replacen(
            "backoff_count INTEGER NOT NULL DEFAULT 0",
            "backoff_count INTEGER NOT NULL",
            1,
        );
        let mut missing_default =
            marked_schema_with_replacement("standard_lxmf_propagation_peers", &missing_default);
        assert!(ensure_standard_propagation_schema(&mut missing_default).is_err());

        let weakened_state = ITEMS_TABLE_SQL.replacen(
            "state IN ('queued','acknowledged','expired')",
            "state IN ('queued','acknowledged','expired','invalid')",
            1,
        );
        let mut weakened_state =
            marked_schema_with_replacement("standard_lxmf_propagation_items", &weakened_state);
        assert!(ensure_standard_propagation_schema(&mut weakened_state).is_err());

        let weakened_length =
            ITEMS_TABLE_SQL.replacen("length(transient_id) = 32", "length(transient_id) >= 1", 1);
        let mut weakened_length =
            marked_schema_with_replacement("standard_lxmf_propagation_items", &weakened_length);
        assert!(ensure_standard_propagation_schema(&mut weakened_length).is_err());
    }

    #[test]
    fn ingest_capacity_and_storage_failure_are_atomic() {
        let mut store = MessagesStore::in_memory().unwrap();
        let first = item([1; 16], 1, 10);
        let second = item([2; 16], 2, 10);
        let limited = StandardPropagationPolicy { queue_max_count: 1, ..policy() };
        assert_eq!(
            ingest(&mut store, &[first.clone(), second], None, None, 10, limited).unwrap(),
            StandardPropagationIngestOutcome::CapacityRejected
        );
        assert_eq!(
            store.standard_propagation_stats(10, limited).unwrap(),
            StandardPropagationStats { queued_count: 0, stored_bytes: 0 }
        );

        let mut overflow = MessagesStore::in_memory().unwrap();
        ingest(
            &mut overflow,
            &[item([3; 16], 3, 10), item([4; 16], 4, 10)],
            None,
            None,
            10,
            policy(),
        )
        .unwrap();
        assert!(overflow.standard_propagation_reconcile_startup(10, limited).is_err());

        let mut duplicates = MessagesStore::in_memory().unwrap();
        assert_eq!(
            ingest(&mut duplicates, &[first.clone(), first.clone()], None, None, 10, limited,)
                .unwrap(),
            StandardPropagationIngestOutcome::Accepted
        );
        assert_eq!(duplicates.standard_propagation_stats(10, limited).unwrap().queued_count, 1);

        let mut identified = MessagesStore::in_memory().unwrap();
        let identified_items = [item([5; 16], 5, 10), item([6; 16], 6, 10)];
        let comparison = identified
            .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                peer: [7; 16],
                offered: &[identified_items[0].transient_id, identified_items[1].transient_id],
                same_link_pending: &BTreeSet::new(),
                pending_elsewhere: &BTreeSet::new(),
                pending_count: 0,
                existing_attempt: None,
                request_id: [8; 16],
                link_id: [9; 16],
                now: 10,
                deadline: 100,
                policy: policy(),
            })
            .unwrap();
        assert_eq!(
            ingest(
                &mut identified,
                &identified_items,
                Some([7; 16]),
                Some(comparison.attempt_id),
                11,
                limited,
            )
            .unwrap(),
            StandardPropagationIngestOutcome::CapacityRejected
        );
        let attempt_state: String = identified
            .conn
            .query_row(
                "SELECT state FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1",
                params![comparison.attempt_id.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempt_state, "failed");
        assert_eq!(identified.standard_propagation_failures(10).unwrap().len(), 1);

        store.standard_propagation_fail_inserts_for_test().unwrap();
        assert!(ingest(&mut store, &[first], None, None, 10, limited).is_err());
        assert_eq!(
            store.standard_propagation_stats(10, limited).unwrap(),
            StandardPropagationStats { queued_count: 0, stored_bytes: 0 }
        );
    }

    #[test]
    fn queue_tombstone_and_acknowledgement_survive_reopens_without_legacy_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("messages.db");
        let queued = item([3; 16], 3, 10);
        let id = queued.transient_id;
        {
            let mut store = MessagesStore::open(&path).unwrap();
            assert_eq!(
                store
                    .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                        items: std::slice::from_ref(&queued),
                        source_peer: None,
                        attempt: StandardPropagationAttemptStatus::Untracked,
                        protocol: StandardPropagationProtocolStatus::Valid,
                        now: 10,
                        policy: policy(),
                    })
                    .unwrap(),
                StandardPropagationIngestOutcome::Accepted
            );
        }
        {
            let mut store = MessagesStore::open(&path).unwrap();
            assert_eq!(
                store.standard_propagation_snapshot(11, policy()).unwrap(),
                vec![queued.clone()]
            );
            assert_eq!(
                store
                    .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                        items: std::slice::from_ref(&queued),
                        source_peer: None,
                        attempt: StandardPropagationAttemptStatus::Untracked,
                        protocol: StandardPropagationProtocolStatus::Valid,
                        now: 11,
                        policy: policy(),
                    })
                    .unwrap(),
                StandardPropagationIngestOutcome::Accepted
            );
            let fetched = store
                .standard_propagation_get(StandardPropagationGetRequest {
                    peer: [0x31; 16],
                    request_id: [0x32; 16],
                    recipient: [3; 16],
                    wants: Some(&[id]),
                    haves: None,
                    inventory: false,
                    response_limit: usize::MAX,
                    now: 11,
                    policy: policy(),
                })
                .unwrap();
            assert_eq!(fetched.payloads, vec![queued.lxmf_data.clone()]);
            store
                .standard_propagation_get(StandardPropagationGetRequest {
                    peer: [0x31; 16],
                    request_id: [0x33; 16],
                    recipient: [3; 16],
                    wants: None,
                    haves: Some(&[id]),
                    inventory: false,
                    response_limit: usize::MAX,
                    now: 12,
                    policy: policy(),
                })
                .unwrap();
        }
        {
            let mut store = MessagesStore::open(&path).unwrap();
            assert!(store.standard_propagation_snapshot(13, policy()).unwrap().is_empty());
            assert_eq!(
                ingest(&mut store, &[queued], None, None, 13, policy()).unwrap(),
                StandardPropagationIngestOutcome::Accepted
            );
            assert!(store.standard_propagation_snapshot(13, policy()).unwrap().is_empty());
            let legacy_count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM propagation_store", [], |row| row.get(0))
                .unwrap();
            assert_eq!(legacy_count, 0);
            store.standard_propagation_stats(12 + TOMBSTONE_RETENTION_SECS, policy()).unwrap();
            let retained: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM standard_lxmf_propagation_items
                     WHERE transient_id = ?1",
                    params![id.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(retained, 1);
            store
                .standard_propagation_reconcile_startup(13 + TOMBSTONE_RETENTION_SECS, policy())
                .unwrap();
            let pruned: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM standard_lxmf_propagation_items
                     WHERE transient_id = ?1",
                    params![id.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(pruned, 0);
        }
    }

    #[test]
    fn attempts_checkpoint_selection_failures_and_reconcile_are_durable_and_bounded() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("messages.db");
        let peer = [4; 16];
        let other_peer = [5; 16];
        let queued = item([6; 16], 6, 20);
        let attempt;
        let interrupted;
        {
            let mut store = MessagesStore::open(&path).unwrap();
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: other_peer,
                    propagation_destination: Some([7; 16]),
                    configured: true,
                    enabled: true,
                    transfer_limit_kb: Some(256),
                    sync_limit_kb: Some(4000),
                    stamp_cost: Some(16),
                    stamp_flexibility: Some(3),
                    peering_cost: Some(18),
                    observed_at: 20,
                })
                .unwrap();
            let comparison = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer,
                    offered: &[queued.transient_id],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [8; 16],
                    link_id: [9; 16],
                    now: 20,
                    deadline: 200,
                    policy: policy(),
                })
                .unwrap();
            attempt = comparison.attempt_id;
            assert_eq!(comparison.wanted, vec![queued.transient_id]);
            assert_eq!(
                ingest(
                    &mut store,
                    std::slice::from_ref(&queued),
                    Some(peer),
                    Some(attempt),
                    21,
                    policy(),
                )
                .unwrap(),
                StandardPropagationIngestOutcome::Accepted
            );
            store.standard_propagation_set_selection(Some(peer), "manual", 21).unwrap();
            interrupted = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer,
                    offered: &[[10; 32]],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [11; 16],
                    link_id: [12; 16],
                    now: 22,
                    deadline: 202,
                    policy: policy(),
                })
                .unwrap()
                .attempt_id;
            for index in 0..(MAX_FAILURES + 4) {
                store
                    .conn
                    .execute(
                        "INSERT INTO standard_lxmf_propagation_failures
                         (code, occurred_at, peer) VALUES ('bounded', ?1, ?2)",
                        params![index as i64, peer.as_slice()],
                    )
                    .unwrap();
            }
        }
        {
            let mut store = MessagesStore::open(&path).unwrap();
            store.standard_propagation_reconcile_startup(30, policy()).unwrap();
            let state: String = store
                .conn
                .query_row(
                    "SELECT state FROM standard_lxmf_propagation_attempts WHERE attempt_id = ?1",
                    params![interrupted.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, "interrupted");
            assert_eq!(
                store.standard_propagation_selection().unwrap(),
                Some(StandardPropagationSelection {
                    peer: Some(peer),
                    mode: "manual".into(),
                    selected_at: 21,
                })
            );
            let checkpoint =
                store.standard_propagation_checkpoint(peer, "ingress").unwrap().unwrap();
            assert_eq!(checkpoint.last_attempt, Some(attempt));
            assert_eq!(checkpoint.completed_stage, "complete");
            let disposition: String = store
                .conn
                .query_row(
                    "SELECT disposition FROM standard_lxmf_propagation_peer_items
                     WHERE peer = ?1 AND transient_id = ?2",
                    params![other_peer.as_slice(), queued.transient_id.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(disposition, "unhandled");
            assert!(
                store.standard_propagation_failures(MAX_FAILURES + 100).unwrap().len()
                    <= MAX_FAILURES
            );
        }
    }

    #[test]
    fn peer_backfill_and_terminal_duplicate_semantics_survive_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("messages.db");
        let first = item([0x31; 16], 0x31, 10);
        let offered = item([0x32; 16], 0x32, 11);
        let queued_race = item([0x34; 16], 0x34, 12);
        let terminal = item([0x33; 16], 0x33, 12);
        let first_peer = [0x41; 16];
        let second_peer = [0x42; 16];
        let source_peer = [0x43; 16];
        let terminal_attempt;
        {
            let mut store = MessagesStore::open(&path).unwrap();
            ingest(&mut store, std::slice::from_ref(&first), None, None, 10, policy()).unwrap();
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: first_peer,
                    propagation_destination: None,
                    configured: true,
                    enabled: true,
                    transfer_limit_kb: None,
                    sync_limit_kb: None,
                    stamp_cost: None,
                    stamp_flexibility: None,
                    peering_cost: None,
                    observed_at: 10,
                })
                .unwrap();
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: second_peer,
                    propagation_destination: None,
                    configured: true,
                    enabled: false,
                    transfer_limit_kb: None,
                    sync_limit_kb: None,
                    stamp_cost: None,
                    stamp_flexibility: None,
                    peering_cost: None,
                    observed_at: 10,
                })
                .unwrap();
            let disabled_count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM standard_lxmf_propagation_peer_items
                     WHERE peer = ?1",
                    params![second_peer.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(disabled_count, 0);
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: second_peer,
                    propagation_destination: None,
                    configured: true,
                    enabled: true,
                    transfer_limit_kb: None,
                    sync_limit_kb: None,
                    stamp_cost: None,
                    stamp_flexibility: None,
                    peering_cost: None,
                    observed_at: 11,
                })
                .unwrap();
            ingest(
                &mut store,
                std::slice::from_ref(&offered),
                Some(first_peer),
                None,
                11,
                policy(),
            )
            .unwrap();
            let dispositions = |store: &MessagesStore, id: [u8; 32]| {
                let mut statement = store
                    .conn
                    .prepare(
                        "SELECT peer, disposition FROM standard_lxmf_propagation_peer_items
                         WHERE transient_id = ?1 ORDER BY peer",
                    )
                    .unwrap();
                statement
                    .query_map(params![id.as_slice()], |row| {
                        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            };
            assert_eq!(
                dispositions(&store, first.transient_id),
                vec![
                    (first_peer.to_vec(), "unhandled".into()),
                    (second_peer.to_vec(), "unhandled".into()),
                ]
            );
            assert_eq!(
                dispositions(&store, offered.transient_id),
                vec![
                    (first_peer.to_vec(), "handled".into()),
                    (second_peer.to_vec(), "unhandled".into()),
                ]
            );

            let queued_attempt = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer: source_peer,
                    offered: &[queued_race.transient_id],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [0x53; 16],
                    link_id: [0x54; 16],
                    now: 12,
                    deadline: 100,
                    policy: policy(),
                })
                .unwrap()
                .attempt_id;
            ingest(&mut store, std::slice::from_ref(&queued_race), None, None, 12, policy())
                .unwrap();
            let other_dispositions_before: Vec<_> = dispositions(&store, queued_race.transient_id)
                .into_iter()
                .filter(|(peer, _)| peer.as_slice() != source_peer)
                .collect();
            let source_counters_before: (i64, i64) = store
                .conn
                .query_row(
                    "SELECT accepted_count, accepted_bytes
                     FROM standard_lxmf_propagation_peers WHERE identity_hash = ?1",
                    params![source_peer.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            store
                .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                    items: std::slice::from_ref(&queued_race),
                    source_peer: Some(source_peer),
                    attempt: StandardPropagationAttemptStatus::Complete(queued_attempt),
                    protocol: StandardPropagationProtocolStatus::Valid,
                    now: 13,
                    policy: policy(),
                })
                .unwrap();
            let queued_dispositions = dispositions(&store, queued_race.transient_id);
            assert!(
                queued_dispositions
                    .iter()
                    .any(|(peer, disposition)| peer.as_slice() == source_peer
                        && disposition == "handled")
            );
            assert_eq!(
                queued_dispositions
                    .into_iter()
                    .filter(|(peer, _)| peer.as_slice() != source_peer)
                    .collect::<Vec<_>>(),
                other_dispositions_before
            );
            let source_counters_after: (i64, i64) = store
                .conn
                .query_row(
                    "SELECT accepted_count, accepted_bytes
                     FROM standard_lxmf_propagation_peers WHERE identity_hash = ?1",
                    params![source_peer.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(source_counters_after, source_counters_before);
            let queued_checkpoint =
                store.standard_propagation_checkpoint(source_peer, "ingress").unwrap().unwrap();
            assert_eq!(queued_checkpoint.last_attempt, Some(queued_attempt));
            assert_eq!(queued_checkpoint.item_count, 1);
            assert_eq!(queued_checkpoint.byte_count, 0);

            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: source_peer,
                    propagation_destination: Some([0x44; 16]),
                    configured: false,
                    enabled: true,
                    transfer_limit_kb: None,
                    sync_limit_kb: None,
                    stamp_cost: None,
                    stamp_flexibility: None,
                    peering_cost: None,
                    observed_at: 13,
                })
                .unwrap();

            terminal_attempt = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer: source_peer,
                    offered: &[terminal.transient_id],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [0x51; 16],
                    link_id: [0x52; 16],
                    now: 14,
                    deadline: 100,
                    policy: policy(),
                })
                .unwrap()
                .attempt_id;
            ingest(&mut store, std::slice::from_ref(&terminal), None, None, 14, policy()).unwrap();
            store
                .standard_propagation_get(StandardPropagationGetRequest {
                    peer: [0x53; 16],
                    request_id: [0x54; 16],
                    recipient: terminal.destination,
                    wants: None,
                    haves: Some(&[terminal.transient_id]),
                    inventory: false,
                    response_limit: usize::MAX,
                    now: 15,
                    policy: policy(),
                })
                .unwrap();
            let before: (i64, i64, String) = store
                .conn
                .query_row(
                    "SELECT p.accepted_count, p.accepted_bytes, pi.disposition
                     FROM standard_lxmf_propagation_peers p
                     JOIN standard_lxmf_propagation_peer_items pi ON pi.peer = p.identity_hash
                     WHERE p.identity_hash = ?1 AND pi.transient_id = ?2",
                    params![source_peer.as_slice(), terminal.transient_id.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(before.2, "unhandled");
            store
                .standard_propagation_ingest_batch(StandardPropagationIngestRequest {
                    items: std::slice::from_ref(&terminal),
                    source_peer: Some(source_peer),
                    attempt: StandardPropagationAttemptStatus::Complete(terminal_attempt),
                    protocol: StandardPropagationProtocolStatus::Valid,
                    now: 16,
                    policy: policy(),
                })
                .unwrap();
            let after: (i64, i64, String) = store
                .conn
                .query_row(
                    "SELECT p.accepted_count, p.accepted_bytes, pi.disposition
                     FROM standard_lxmf_propagation_peers p
                     JOIN standard_lxmf_propagation_peer_items pi ON pi.peer = p.identity_hash
                     WHERE p.identity_hash = ?1 AND pi.transient_id = ?2",
                    params![source_peer.as_slice(), terminal.transient_id.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(after, before);
        }
        {
            let mut store = MessagesStore::open(&path).unwrap();
            assert!(
                store
                    .standard_propagation_snapshot(17, policy())
                    .unwrap()
                    .iter()
                    .all(|item| { item.transient_id != terminal.transient_id })
            );
            let checkpoint =
                store.standard_propagation_checkpoint(source_peer, "ingress").unwrap().unwrap();
            assert_eq!(checkpoint.last_attempt, Some(terminal_attempt));
            assert_eq!(checkpoint.item_count, 1);
            assert_eq!(checkpoint.byte_count, 0);
            let observation = store.standard_propagation_observation(17, policy()).unwrap();
            assert_eq!(observation.queue.acknowledged_count, 1);
            assert_eq!(observation.queue.expired_count, 0);
            let disposition: String = store
                .conn
                .query_row(
                    "SELECT disposition FROM standard_lxmf_propagation_peer_items
                     WHERE peer = ?1 AND transient_id = ?2",
                    params![source_peer.as_slice(), terminal.transient_id.as_slice()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(disposition, "unhandled");
        }
    }

    #[test]
    fn observation_projects_durable_attempts_checkpoints_and_failures_after_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("messages.db");
        let peer = [0x61; 16];
        let completed_item = item([0x62; 16], 0x62, 20);
        let completed_attempt;
        let running_attempt;
        {
            let mut store = MessagesStore::open(&path).unwrap();
            store
                .standard_propagation_upsert_peer(&StandardPropagationPeer {
                    identity_hash: peer,
                    propagation_destination: Some([0x63; 16]),
                    configured: true,
                    enabled: true,
                    transfer_limit_kb: Some(256),
                    sync_limit_kb: Some(4000),
                    stamp_cost: Some(16),
                    stamp_flexibility: Some(3),
                    peering_cost: Some(18),
                    observed_at: 20,
                })
                .unwrap();
            completed_attempt = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer,
                    offered: &[completed_item.transient_id],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [0x64; 16],
                    link_id: [0x65; 16],
                    now: 20,
                    deadline: 120,
                    policy: policy(),
                })
                .unwrap()
                .attempt_id;
            ingest(
                &mut store,
                std::slice::from_ref(&completed_item),
                Some(peer),
                Some(completed_attempt),
                21,
                policy(),
            )
            .unwrap();
            running_attempt = store
                .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                    peer,
                    offered: &[[0x66; 32]],
                    same_link_pending: &BTreeSet::new(),
                    pending_elsewhere: &BTreeSet::new(),
                    pending_count: 0,
                    existing_attempt: None,
                    request_id: [0x67; 16],
                    link_id: [0x68; 16],
                    now: 22,
                    deadline: 122,
                    policy: policy(),
                })
                .unwrap()
                .attempt_id;
            store
                .standard_propagation_record_attempt_failure(
                    running_attempt,
                    peer,
                    "temporary_link",
                    Some("not projected"),
                    23,
                )
                .unwrap();
            store.standard_propagation_set_selection(Some(peer), "manual", 24).unwrap();
        }
        {
            let mut store = MessagesStore::open(&path).unwrap();
            let observation = store.standard_propagation_observation(25, policy()).unwrap();
            assert_eq!(observation.observed_at, 25);
            assert_eq!(observation.queue.queued_count, 1);
            assert_eq!(observation.peers.len(), 1);
            assert!(observation.peers[0].configured);
            assert_eq!(observation.peers[0].propagation_destination, Some([0x63; 16]));
            assert_eq!(observation.selection.unwrap().peer, Some(peer));
            assert_eq!(observation.attempts.len(), 2);
            assert_eq!(observation.attempts[0].attempt_id, running_attempt);
            assert_eq!(observation.attempts[0].stage, "transfer");
            assert_eq!(observation.attempts[0].state, "failed");
            assert_eq!(observation.attempts[0].failure_code.as_deref(), Some("temporary_link"));
            assert_eq!(observation.attempts[1].attempt_id, completed_attempt);
            assert_eq!(observation.attempts[1].state, "completed");
            assert_eq!(observation.checkpoints.len(), 1);
            assert_eq!(observation.checkpoints[0].last_attempt, Some(running_attempt));
            assert_eq!(observation.failures.len(), 1);
            assert_eq!(observation.failures[0].code, "temporary_link");
            assert_eq!(observation.failures[0].attempt_id, Some(running_attempt));
        }
    }

    #[test]
    fn observation_bounds_and_orders_newest_records_deterministically() {
        assert_eq!(
            STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT,
            styrene_ipc::types::MAX_STANDARD_PROPAGATION_PEERS
        );
        assert_eq!(
            STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT,
            styrene_ipc::types::MAX_STANDARD_PROPAGATION_ATTEMPTS
        );
        assert_eq!(
            STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT,
            styrene_ipc::types::MAX_STANDARD_PROPAGATION_CHECKPOINTS
        );
        assert_eq!(
            STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT,
            styrene_ipc::types::MAX_STANDARD_PROPAGATION_FAILURES
        );
        let mut store = MessagesStore::in_memory().unwrap();
        for index in 0..(STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT + 2) {
            let mut peer = [0u8; 16];
            peer[..8].copy_from_slice(&(index as u64).to_be_bytes());
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_peers
                     (identity_hash, origin, enabled, first_seen_at, last_seen_at)
                     VALUES (?1, 'observed', 1, ?2, ?2)",
                    params![peer.as_slice(), index as i64],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_checkpoints
                     (peer, direction, completed_stage, item_count, byte_count, updated_at)
                     VALUES (?1, 'ingress', 'complete', 0, 0, ?2)",
                    params![peer.as_slice(), index as i64],
                )
                .unwrap();
        }
        for index in 0..(STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT + 1) {
            let attempt_id = (index as u128).to_be_bytes();
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_attempts
                     (attempt_id, correlation_id, direction, stage, state, started_at,
                      updated_at, offered_count, wanted_count)
                     VALUES (?1, ?1, 'ingress', 'offer', 'running', ?2, ?2, 0, 0)",
                    params![attempt_id.as_slice(), index as i64],
                )
                .unwrap();
        }
        for index in 0..(STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT + 2) {
            store
                .conn
                .execute(
                    "INSERT INTO standard_lxmf_propagation_failures(code, occurred_at)
                     VALUES ('bounded', ?1)",
                    params![index as i64],
                )
                .unwrap();
        }

        let first = store.standard_propagation_observation(200, policy()).unwrap();
        let second = store.standard_propagation_observation(200, policy()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.peers.len(), STANDARD_PROPAGATION_OBSERVATION_PEER_LIMIT);
        assert_eq!(first.attempts.len(), STANDARD_PROPAGATION_OBSERVATION_ATTEMPT_LIMIT);
        assert_eq!(first.checkpoints.len(), STANDARD_PROPAGATION_OBSERVATION_CHECKPOINT_LIMIT);
        assert_eq!(first.failures.len(), STANDARD_PROPAGATION_OBSERVATION_FAILURE_LIMIT);
        assert!(first.peers_truncated);
        assert!(first.attempts_truncated);
        assert!(first.checkpoints_truncated);
        assert!(first.failures_truncated);
        assert_eq!(first.peers[0].last_seen_at, 129);
        assert_eq!(first.attempts[0].updated_at, 256);
        assert_eq!(first.checkpoints[0].updated_at, 129);
        assert_eq!(first.failures[0].occurred_at, 129);
    }

    #[test]
    fn get_operations_and_exact_deadlines_are_durable_observations() {
        let mut store = MessagesStore::in_memory().unwrap();
        let peer = [0xd1; 16];
        let queued = item([0xd2; 16], 0xd3, 10);
        let transient_id = queued.transient_id;
        ingest(&mut store, std::slice::from_ref(&queued), None, None, 10, policy()).unwrap();

        let fetched = store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer,
                request_id: [0xd4; 16],
                recipient: queued.destination,
                wants: None,
                haves: None,
                inventory: true,
                response_limit: usize::MAX,
                now: 11,
                policy: policy(),
            })
            .unwrap();
        assert_eq!(fetched.inventory, Some(vec![transient_id]));
        let downloaded = store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer,
                request_id: [0xd5; 16],
                recipient: queued.destination,
                wants: Some(&[transient_id]),
                haves: None,
                inventory: false,
                response_limit: usize::MAX,
                now: 12,
                policy: policy(),
            })
            .unwrap();
        assert_eq!(downloaded.payloads, vec![queued.lxmf_data.clone()]);
        store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer,
                request_id: [0xd6; 16],
                recipient: queued.destination,
                wants: None,
                haves: Some(&[transient_id]),
                inventory: false,
                response_limit: usize::MAX,
                now: 13,
                policy: policy(),
            })
            .unwrap();

        let deadline_attempt = store
            .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                peer,
                offered: &[[0xd7; 32]],
                same_link_pending: &BTreeSet::new(),
                pending_elsewhere: &BTreeSet::new(),
                pending_count: 0,
                existing_attempt: None,
                request_id: [0xd8; 16],
                link_id: [0xd9; 16],
                now: 20,
                deadline: 50,
                policy: policy(),
            })
            .unwrap()
            .attempt_id;
        assert_eq!(store.standard_propagation_reconcile_deadlines(49).unwrap(), 0);
        assert_eq!(store.standard_propagation_reconcile_deadlines(50).unwrap(), 1);
        assert_eq!(store.standard_propagation_reconcile_deadlines(50).unwrap(), 0);

        let observation = store.standard_propagation_observation(50, policy()).unwrap();
        for stage in ["fetch", "download", "sync"] {
            let attempt =
                observation.attempts.iter().find(|attempt| attempt.stage == stage).unwrap();
            assert_eq!(attempt.peer, Some(peer));
            assert_eq!(attempt.state, "completed");
            assert_eq!(attempt.accepted_count, 1);
            assert_eq!(
                attempt.accepted_bytes,
                if stage == "download" { queued.lxmf_data.len() } else { 0 }
            );
        }
        let deadline = observation
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == deadline_attempt)
            .unwrap();
        assert_eq!(deadline.state, "failed");
        assert_eq!(deadline.failure_code.as_deref(), Some("deadline_elapsed"));
        assert_eq!(
            observation
                .failures
                .iter()
                .filter(|failure| failure.attempt_id == Some(deadline_attempt))
                .count(),
            1
        );
    }

    #[test]
    fn startup_reconciliation_uses_exact_deadline_before_interrupting_running_attempts() {
        let mut store = MessagesStore::in_memory().unwrap();
        let peer = [0xe1; 16];
        let mut attempts = Vec::new();
        for (index, deadline) in [99, 100, 101].into_iter().enumerate() {
            attempts.push(
                store
                    .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                        peer,
                        offered: &[[index as u8 + 1; 32]],
                        same_link_pending: &BTreeSet::new(),
                        pending_elsewhere: &BTreeSet::new(),
                        pending_count: 0,
                        existing_attempt: None,
                        request_id: [index as u8 + 4; 16],
                        link_id: [index as u8 + 8; 16],
                        now: 10,
                        deadline,
                        policy: policy(),
                    })
                    .unwrap()
                    .attempt_id,
            );
        }
        let no_deadline = [0xee; 16];
        store
            .conn
            .execute(
                "INSERT INTO standard_lxmf_propagation_attempts
                 (attempt_id, correlation_id, peer, direction, stage, state, started_at,
                  updated_at, deadline_at, offered_count, wanted_count)
                 VALUES (?1, ?1, ?2, 'ingress', 'offer', 'running', 10, 10, NULL, 1, 1)",
                params![no_deadline.as_slice(), peer.as_slice()],
            )
            .unwrap();

        store.standard_propagation_reconcile_startup(100, policy()).unwrap();
        let observation = store.standard_propagation_observation(100, policy()).unwrap();
        for attempt in &attempts[..2] {
            let attempt =
                observation.attempts.iter().find(|item| item.attempt_id == *attempt).unwrap();
            assert_eq!(attempt.state, "failed");
            assert_eq!(attempt.failure_code.as_deref(), Some("deadline_elapsed"));
        }
        for attempt in [attempts[2], no_deadline] {
            let attempt =
                observation.attempts.iter().find(|item| item.attempt_id == attempt).unwrap();
            assert_eq!(attempt.state, "interrupted");
            assert_eq!(attempt.failure_code.as_deref(), Some("startup_interrupted"));
        }
        assert_eq!(observation.failures.len(), 2);
    }

    #[test]
    fn request_observation_is_inactive_until_announce_and_does_not_disable_existing_peer() {
        let mut store = MessagesStore::in_memory().unwrap();
        let peer = [0xf1; 16];
        store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer,
                request_id: [0xf2; 16],
                recipient: [0xf3; 16],
                wants: None,
                haves: None,
                inventory: true,
                response_limit: usize::MAX,
                now: 10,
                policy: policy(),
            })
            .unwrap();
        let observed = store.standard_propagation_observation(10, policy()).unwrap();
        assert!(!observed.peers[0].enabled);

        store
            .standard_propagation_upsert_peer(&StandardPropagationPeer {
                identity_hash: peer,
                propagation_destination: Some([0xf4; 16]),
                configured: false,
                enabled: true,
                transfer_limit_kb: None,
                sync_limit_kb: None,
                stamp_cost: None,
                stamp_flexibility: None,
                peering_cost: None,
                observed_at: 11,
            })
            .unwrap();
        store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer,
                request_id: [0xf5; 16],
                recipient: [0xf3; 16],
                wants: None,
                haves: None,
                inventory: true,
                response_limit: usize::MAX,
                now: 12,
                policy: policy(),
            })
            .unwrap();
        assert!(store.standard_propagation_observation(12, policy()).unwrap().peers[0].enabled);
    }

    #[test]
    fn ingress_observation_bytes_exclude_persisted_stamp_bytes() {
        let mut store = MessagesStore::in_memory().unwrap();
        let peer = [0x91; 16];
        let received = item([0x92; 16], 0x93, 10);
        ingest(&mut store, std::slice::from_ref(&received), Some(peer), None, 10, policy())
            .unwrap();

        let observation = store.standard_propagation_observation(10, policy()).unwrap();
        assert_eq!(observation.queue.queued_bytes, received.stored_size);
        let observed_peer =
            observation.peers.iter().find(|item| item.identity_hash == peer).unwrap();
        assert_eq!(observed_peer.accepted_count, 1);
        assert_eq!(observed_peer.accepted_bytes, received.lxmf_data.len());
    }

    #[test]
    fn mixed_haves_and_wants_count_items_but_only_wants_count_payload_bytes() {
        let mut store = MessagesStore::in_memory().unwrap();
        let acknowledged = item([0xa1; 16], 0xa2, 10);
        let returned = item(acknowledged.destination, 0xa3, 10);
        ingest(&mut store, &[acknowledged.clone(), returned.clone()], None, None, 10, policy())
            .unwrap();
        let result = store
            .standard_propagation_get(StandardPropagationGetRequest {
                peer: [0xa4; 16],
                request_id: [0xa5; 16],
                recipient: acknowledged.destination,
                wants: Some(&[returned.transient_id]),
                haves: Some(&[acknowledged.transient_id]),
                inventory: false,
                response_limit: usize::MAX,
                now: 11,
                policy: policy(),
            })
            .unwrap();
        assert_eq!(result.payloads, vec![returned.lxmf_data.clone()]);
        let observation = store.standard_propagation_observation(11, policy()).unwrap();
        let attempt =
            observation.attempts.iter().find(|item| item.attempt_id == result.attempt_id).unwrap();
        assert_eq!(attempt.accepted_count, 2);
        assert_eq!(attempt.accepted_bytes, returned.lxmf_data.len());
        let checkpoint =
            store.standard_propagation_checkpoint([0xa4; 16], "sync").unwrap().unwrap();
        assert_eq!(checkpoint.item_count, 2);
        assert_eq!(checkpoint.byte_count, returned.lxmf_data.len());
    }

    #[test]
    fn attempt_failure_terminalizes_once_and_redacted_debug_excludes_payload_secrets() {
        let mut store = MessagesStore::in_memory().unwrap();
        let peer = [0xb1; 16];
        let comparison = store
            .standard_propagation_compare_offer(StandardPropagationOfferRequest {
                peer,
                offered: &[[0xb2; 32]],
                same_link_pending: &BTreeSet::new(),
                pending_elsewhere: &BTreeSet::new(),
                pending_count: 0,
                existing_attempt: None,
                request_id: [0xb3; 16],
                link_id: [0xb4; 16],
                now: 10,
                deadline: 20,
                policy: policy(),
            })
            .unwrap();
        store
            .standard_propagation_record_attempt_failure(
                comparison.attempt_id,
                peer,
                "malformed_transfer",
                None,
                11,
            )
            .unwrap();
        store.standard_propagation_reconcile_deadlines(20).unwrap();
        store
            .standard_propagation_record_attempt_failure(
                comparison.attempt_id,
                peer,
                "storage",
                None,
                21,
            )
            .unwrap();
        let observation = store.standard_propagation_observation(21, policy()).unwrap();
        let attempt = observation
            .attempts
            .iter()
            .find(|item| item.attempt_id == comparison.attempt_id)
            .unwrap();
        assert_eq!(attempt.state, "failed");
        assert_eq!(attempt.failure_code.as_deref(), Some("malformed_transfer"));
        assert_eq!(observation.failures.len(), 1);
        assert_eq!(observation.failures[0].code, "malformed_transfer");
        assert_eq!(observation.checkpoints[0].completed_stage, "offer");

        let secret = b"SENTINEL_LXMF_SECRET".to_vec();
        let mut redacted_item = item([0xb5; 16], 0xb6, 10);
        redacted_item.lxmf_data = secret.clone();
        redacted_item.stamp = *b"SENTINEL_STAMP_SECRET_1234567890";
        let item_debug = format!("{redacted_item:?}");
        assert!(!item_debug.contains("SENTINEL"));
        assert!(item_debug.contains("lxmf_data_len"));
        let result = StandardPropagationGetResult {
            inventory: None,
            payloads: vec![secret],
            attempt_id: [0xb7; 16],
        };
        let result_debug = format!("{result:?}");
        assert!(!result_debug.contains("SENTINEL"));
        assert!(result_debug.contains("payload_bytes"));
    }
}
