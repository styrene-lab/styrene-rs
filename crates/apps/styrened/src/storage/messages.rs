use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::Value as JsonValue;
use sha2::Digest;

pub const MAX_MESSAGE_QUERY_LIMIT: usize = styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT as usize;
pub const MAX_MESSAGE_PROJECTION_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_DRAFT_BYTES: usize = 64 * 1024;
pub const MAX_RETAINED_DRAFTS: usize = 128;
pub const MAX_DRAFT_AGGREGATE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CONTACT_ALIAS_BYTES: usize = 256;
pub const MAX_CONTACT_NOTES_BYTES: usize = 4096;
pub const MAX_SEARCH_QUERY_BYTES: usize = 1024;
pub const MAX_ATTACHMENT_BLOB_BYTES: usize = 768 * 1024;
pub const MAX_ATTACHMENT_BLOB_AGGREGATE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ATTACHMENT_BLOB_COUNT: usize = 4096;
pub const MAX_DELIVERY_EVIDENCE_PER_MESSAGE: usize = 32;
const DELIVERY_EVIDENCE_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

fn unix_time_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}
const MESSAGE_PROJECTION_OVERHEAD_BYTES: usize = 4096;
const CONVERSATION_SCHEMA_DDL: &str = "
    CREATE TABLE conversation_state (
        peer_hash TEXT NOT NULL PRIMARY KEY
            CHECK(length(peer_hash) = 32)
            CHECK(peer_hash = lower(peer_hash))
            CHECK(peer_hash NOT GLOB '*[^0-9a-f]*'),
        pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
        muted INTEGER NOT NULL DEFAULT 0 CHECK(muted IN (0, 1)),
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE conversation_drafts (
        peer_hash TEXT NOT NULL PRIMARY KEY
            CHECK(length(peer_hash) = 32)
            CHECK(peer_hash = lower(peer_hash))
            CHECK(peer_hash NOT GLOB '*[^0-9a-f]*'),
        content TEXT NOT NULL
            CHECK(typeof(content) = 'text')
            CHECK(length(CAST(content AS BLOB)) <= 65536),
        updated_at INTEGER NOT NULL
    );";
const PAGE_KEYS_SCHEMA_DDL: &str = "
    CREATE TABLE message_page_keys (
        message_id TEXT NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
        ingest_seq INTEGER PRIMARY KEY AUTOINCREMENT,
        sort_timestamp INTEGER NOT NULL,
        conversation_peer TEXT NOT NULL
    );";
const PAGE_METADATA_SCHEMA_DDL: &str = "
    CREATE TABLE message_page_metadata (
        singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
        store_id BLOB NOT NULL UNIQUE
            CHECK(typeof(store_id) = 'blob' AND length(store_id) = 16),
        conversation_epoch INTEGER NOT NULL DEFAULT 0
            CHECK(typeof(conversation_epoch) = 'integer'
                  AND conversation_epoch BETWEEN 0 AND 9223372036854775807),
        cursor_secret BLOB NOT NULL UNIQUE
            CHECK(typeof(cursor_secret) = 'blob' AND length(cursor_secret) = 32)
    );";
const MESSAGE_INSPECTION_MIGRATION: &str = "2026-08-25-authoritative-message-inspection-v13";
const CANONICAL_INSPECTION_TABLE_SQL: &str = "CREATE TABLE canonical_inbound_inspection (
    message_id TEXT PRIMARY KEY REFERENCES canonical_inbound_messages(message_id) ON DELETE CASCADE CHECK(typeof(message_id) = 'text'),
    stamp_target INTEGER CHECK(stamp_target IS NULL OR (typeof(stamp_target) = 'integer' AND stamp_target BETWEEN 0 AND 254))
) STRICT";
const OUTBOUND_INSPECTION_TABLE_SQL: &str = "CREATE TABLE outbound_message_inspection (
    message_id TEXT PRIMARY KEY REFERENCES outbound_routes(message_id) ON DELETE CASCADE CHECK(typeof(message_id) = 'text'),
    terminal_detail TEXT CHECK(terminal_detail IS NULL OR (typeof(terminal_detail) = 'text' AND length(CAST(terminal_detail AS BLOB)) <= 1024))
) STRICT";
const DELIVERY_EVIDENCE_TABLE_SQL: &str = "CREATE TABLE message_delivery_evidence (
    evidence_hash TEXT PRIMARY KEY CHECK(typeof(evidence_hash) = 'text' AND length(evidence_hash) = 64 AND evidence_hash = lower(evidence_hash) AND evidence_hash NOT GLOB '*[^0-9a-f]*'),
    message_id TEXT NOT NULL REFERENCES outbound_routes(message_id) ON DELETE CASCADE CHECK(typeof(message_id) = 'text'),
    kind TEXT NOT NULL CHECK(typeof(kind) = 'text' AND kind IN ('packet_receipt','resource_completion')),
    representation TEXT NOT NULL CHECK(typeof(representation) = 'text' AND representation IN ('packet','resource')),
    state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('tracked','completed','failed','cancelled')),
    outcome TEXT CHECK(outcome IS NULL OR (typeof(outcome) = 'text' AND length(CAST(outcome AS BLOB)) <= 1024)),
    attempt_number INTEGER NOT NULL CHECK(typeof(attempt_number) = 'integer' AND attempt_number BETWEEN 1 AND 32),
    correlation_id TEXT CHECK(correlation_id IS NULL OR (typeof(correlation_id) = 'text' AND length(CAST(correlation_id AS BLOB)) BETWEEN 1 AND 128)),
    observed_at INTEGER NOT NULL CHECK(typeof(observed_at) = 'integer' AND observed_at >= 0),
    terminal_at INTEGER CHECK(terminal_at IS NULL OR (typeof(terminal_at) = 'integer' AND terminal_at >= observed_at)),
    transferred_bytes INTEGER CHECK(transferred_bytes IS NULL OR (typeof(transferred_bytes) = 'integer' AND transferred_bytes >= 0)),
    total_bytes INTEGER CHECK(total_bytes IS NULL OR (typeof(total_bytes) = 'integer' AND total_bytes >= 0)),
    progress INTEGER CHECK(progress IS NULL OR (typeof(progress) = 'integer' AND progress BETWEEN 0 AND 100)),
    CHECK((kind = 'packet_receipt' AND representation = 'packet') OR (kind = 'resource_completion' AND representation = 'resource')),
    CHECK((state = 'tracked' AND terminal_at IS NULL) OR (state != 'tracked' AND terminal_at IS NOT NULL)),
    CHECK((representation = 'packet' AND transferred_bytes IS NULL AND total_bytes IS NULL AND progress IS NULL) OR (representation = 'resource' AND ((transferred_bytes IS NULL AND total_bytes IS NULL AND progress IS NULL) OR (transferred_bytes IS NOT NULL AND total_bytes IS NOT NULL AND progress IS NOT NULL AND transferred_bytes <= total_bytes))))
) STRICT";
const DELIVERY_EVIDENCE_INDEX_SQL: &str = "CREATE INDEX idx_message_delivery_evidence_message
    ON message_delivery_evidence(message_id, observed_at DESC, evidence_hash)";
const DELIVERY_EVIDENCE_RETENTION_INDEX_SQL: &str =
    "CREATE INDEX idx_message_delivery_evidence_terminal
    ON message_delivery_evidence(terminal_at) WHERE terminal_at IS NOT NULL";

fn normalize_local_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ").replace(" (", "(")
}

fn message_inspection_schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    for (kind, name, expected) in [
        ("table", "canonical_inbound_inspection", CANONICAL_INSPECTION_TABLE_SQL),
        ("table", "outbound_message_inspection", OUTBOUND_INSPECTION_TABLE_SQL),
        ("table", "message_delivery_evidence", DELIVERY_EVIDENCE_TABLE_SQL),
        ("index", "idx_message_delivery_evidence_message", DELIVERY_EVIDENCE_INDEX_SQL),
        ("index", "idx_message_delivery_evidence_terminal", DELIVERY_EVIDENCE_RETENTION_INDEX_SQL),
    ] {
        let sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()?;
        if sql.as_deref().map(normalize_local_schema_sql)
            != Some(normalize_local_schema_sql(expected))
        {
            return Ok(false);
        }
    }
    for table in
        ["canonical_inbound_inspection", "outbound_message_inspection", "message_delivery_evidence"]
    {
        let strict: Option<i64> = conn
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .optional()?;
        if strict != Some(1) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn ensure_message_inspection_schema(conn: &mut Connection) -> rusqlite::Result<()> {
    let applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
        params![MESSAGE_INSPECTION_MIGRATION],
        |row| row.get(0),
    )?;
    if applied {
        return message_inspection_schema_is_valid(conn)?.then_some(()).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(
                "v13 message inspection schema attestation failed".into(),
            )
        });
    }
    let collision: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE name IN (
                 'canonical_inbound_inspection','outbound_message_inspection',
                 'message_delivery_evidence','idx_message_delivery_evidence_message',
                 'idx_message_delivery_evidence_terminal'
             )
         )",
        [],
        |row| row.get(0),
    )?;
    if collision {
        return Err(rusqlite::Error::InvalidParameterName(
            "v13 message inspection schema object collision".into(),
        ));
    }
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(CANONICAL_INSPECTION_TABLE_SQL)?;
    transaction.execute_batch(OUTBOUND_INSPECTION_TABLE_SQL)?;
    transaction.execute(
        "INSERT INTO canonical_inbound_inspection (message_id, stamp_target)
         SELECT message_id, NULL FROM canonical_inbound_messages",
        [],
    )?;
    transaction.execute(
        "INSERT INTO outbound_message_inspection (message_id, terminal_detail)
         SELECT message_id, NULL FROM outbound_routes",
        [],
    )?;
    transaction.execute_batch(DELIVERY_EVIDENCE_TABLE_SQL)?;
    transaction.execute_batch(DELIVERY_EVIDENCE_INDEX_SQL)?;
    transaction.execute_batch(DELIVERY_EVIDENCE_RETENTION_INDEX_SQL)?;
    if !message_inspection_schema_is_valid(&transaction)? {
        return Err(rusqlite::Error::InvalidParameterName(
            "v13 message inspection schema validation failed".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO schema_migrations (id, applied_at) VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
        params![MESSAGE_INSPECTION_MIGRATION],
    )?;
    transaction.commit()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MessageRecord {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub title: String,
    pub content: String,
    pub timestamp: i64,
    pub direction: String,
    pub fields: Option<JsonValue>,
    pub receipt_status: Option<String>,
    /// Whether the message has been read by the local user.
    pub read: bool,
}

#[derive(Clone, PartialEq)]
pub struct CanonicalInboundRecord {
    pub message_id: String,
    pub source: [u8; 16],
    pub destination: [u8; 16],
    pub title: Vec<u8>,
    pub content: Vec<u8>,
    pub timestamp: f64,
    pub fields_msgpack: Option<Vec<u8>>,
    pub signature: Option<Vec<u8>>,
    pub stamp: Option<Vec<u8>>,
    pub wire: Vec<u8>,
    pub authentication_state: String,
    pub stamp_state: String,
    pub stamp_value: Option<u32>,
    pub stamp_target: Option<u32>,
}

impl std::fmt::Debug for CanonicalInboundRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalInboundRecord")
            .field("message_id", &self.message_id)
            .field("source", &hex::encode(self.source))
            .field("destination", &hex::encode(self.destination))
            .field("title_len", &self.title.len())
            .field("content_len", &self.content.len())
            .field("timestamp", &self.timestamp)
            .field("fields_msgpack_len", &self.fields_msgpack.as_ref().map(Vec::len))
            .field("signature_len", &self.signature.as_ref().map(Vec::len))
            .field("stamp_len", &self.stamp.as_ref().map(Vec::len))
            .field("wire_len", &self.wire.len())
            .field("authentication_state", &self.authentication_state)
            .field("stamp_state", &self.stamp_state)
            .field("stamp_value", &self.stamp_value)
            .field("stamp_target", &self.stamp_target)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LxmfTicketRecord {
    pub peer: String,
    pub ticket: Vec<u8>,
    pub expires_at: i64,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LxmfTicketOfferReservation {
    pub reservation_id: String,
    pub ticket: LxmfTicketRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LxmfStampPolicy {
    pub target_cost: u32,
    pub flexibility: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContactRecord {
    pub peer_hash: String,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Summary of a conversation with a peer.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ConversationSummary {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message_timestamp: Option<i64>,
    pub last_message_content: Option<String>,
    pub unread_count: u32,
    pub message_count: u32,
    pub pinned: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationDisposition {
    Applied,
    Unchanged,
    NotFound,
    TerminalConflict,
    Created,
    Updated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMutationOutcome {
    pub disposition: MutationDisposition,
    pub affected_count: u64,
    pub summary: Option<ConversationSummary>,
    pub terminal_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactMutationOutcome {
    pub disposition: MutationDisposition,
    pub affected_count: u64,
    pub contact: Option<ContactRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageMutationOutcome {
    pub disposition: MutationDisposition,
    pub affected_count: u64,
    pub terminal_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageSearchSnapshot {
    pub items: Vec<MessageProjectionSnapshot>,
    pub truncated: bool,
    pub matched_count: u64,
}

fn conversation_exists(
    transaction: &rusqlite::Transaction<'_>,
    peer_hash: &str,
) -> rusqlite::Result<bool> {
    transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages
         WHERE lower(source) = ?1 OR lower(destination) = ?1)",
        params![peer_hash],
        |row| row.get(0),
    )
}

fn conversation_summary(
    transaction: &rusqlite::Transaction<'_>,
    peer_hash: &str,
) -> rusqlite::Result<Option<ConversationSummary>> {
    transaction
        .query_row(
            "SELECT ?1,
                    (SELECT timestamp FROM messages
                     WHERE lower(source) = ?1 OR lower(destination) = ?1
                     ORDER BY timestamp DESC, id DESC LIMIT 1),
                    (SELECT content FROM messages
                     WHERE lower(source) = ?1 OR lower(destination) = ?1
                     ORDER BY timestamp DESC, id DESC LIMIT 1),
                    SUM(CASE WHEN direction = 'in' AND lower(source) = ?1
                                  AND COALESCE(read, 0) = 0 THEN 1 ELSE 0 END),
                    COUNT(*), COALESCE(s.pinned, 0), COALESCE(s.muted, 0)
             FROM messages LEFT JOIN conversation_state s ON s.peer_hash = ?1
             WHERE lower(source) = ?1 OR lower(destination) = ?1
             HAVING COUNT(*) > 0",
            params![peer_hash],
            |row| {
                Ok(ConversationSummary {
                    peer_hash: row.get(0)?,
                    peer_name: None,
                    last_message_timestamp: row.get(1)?,
                    last_message_content: row.get(2)?,
                    unread_count: row.get::<_, i64>(3)? as u32,
                    message_count: row.get::<_, i64>(4)? as u32,
                    pinned: row.get::<_, i64>(5)? != 0,
                    muted: row.get::<_, i64>(6)? != 0,
                })
            },
        )
        .optional()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationDraft {
    pub peer_hash: String,
    pub content: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnnounceRecord {
    pub id: String,
    pub peer: String,
    pub timestamp: i64,
    pub name: Option<String>,
    pub name_source: Option<String>,
    pub first_seen: i64,
    pub seen_count: u64,
    pub app_data_hex: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub rssi: Option<f64>,
    pub snr: Option<f64>,
    pub q: Option<f64>,
    pub stamp_cost: Option<u32>,
    pub stamp_cost_flexibility: Option<u32>,
    pub peering_cost: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropagationInventoryRecord {
    pub id: String,
    pub destination_hash: String,
    pub source_hash: Option<String>,
    pub received_at: i64,
    pub expires_at: i64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRouteRecord {
    pub message_id: String,
    pub requested_method: String,
    pub actual_method: String,
    pub representation: String,
    pub fallback_reason: Option<String>,
    pub correlation_id: String,
    pub retry_of: Option<String>,
    pub deadline_unix_ms: i64,
    pub state: String,
    pub attempt_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDeliveryEvidenceRecord {
    pub message_id: String,
    pub kind: String,
    pub evidence_hash: String,
    pub representation: String,
    pub state: String,
    pub outcome: Option<String>,
    pub attempt_number: Option<u32>,
    pub correlation_id: Option<String>,
    pub observed_at: i64,
    pub terminal_at: Option<i64>,
    pub transferred_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub progress: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundAttemptRecord {
    pub message_id: String,
    pub attempt_number: u32,
    pub started_unix_ms: i64,
    pub deadline_unix_ms: i64,
    pub state: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentBlobInput {
    pub wire_name: String,
    pub data: Vec<u8>,
    pub content_type: Option<String>,
    pub source: String,
}

impl std::fmt::Debug for AttachmentBlobInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentBlobInput")
            .field("wire_name", &self.wire_name)
            .field("byte_len", &self.data.len())
            .field("content_type", &self.content_type)
            .field("source", &self.source)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageAttachmentRecord {
    pub message_id: String,
    pub ordinal: u8,
    pub digest: [u8; 32],
    pub wire_name: String,
    pub byte_len: u64,
    pub content_type: Option<String>,
    pub availability: String,
    pub integrity: String,
    pub source: String,
    pub transfer_id: Option<String>,
    pub resource_hash: Option<Vec<u8>>,
    pub representation: Option<String>,
    pub direction: Option<String>,
    pub transfer_state: Option<String>,
    pub transferred: u64,
    pub total: u64,
    pub checksum_verified: bool,
    pub transfer_error: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentChunkRecord {
    pub attachment: MessageAttachmentRecord,
    pub data: Vec<u8>,
    pub next_offset: usize,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundAttachmentTransferEvidence {
    pub resource_hash: [u8; 32],
    pub transferred: u64,
    pub total: u64,
    pub checksum_verified: bool,
}

impl std::fmt::Debug for AttachmentChunkRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentChunkRecord")
            .field("attachment", &self.attachment)
            .field("byte_len", &self.data.len())
            .field("next_offset", &self.next_offset)
            .field("done", &self.done)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionSnapshot {
    pub message: MessageRecord,
    pub canonical: Option<CanonicalInboundRecord>,
    pub lifecycle: Option<(OutboundRouteRecord, Vec<OutboundAttemptRecord>)>,
}

#[derive(Debug)]
pub struct MessageProjectionPage {
    pub items: Vec<MessageProjectionSnapshot>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub struct ConversationPage {
    pub items: Vec<ConversationSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PageError {
    #[error("{0}")]
    InvalidCursor(String),
    #[error("cursor_stale")]
    CursorStale,
    #[error("{0}")]
    Internal(String),
    #[error(transparent)]
    Storage(#[from] rusqlite::Error),
}

/// Parse a message row from a SELECT that returns 10 columns:
/// id, source, destination, title, content, timestamp, direction, fields, receipt_status, read
fn parse_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    let fields_json: Option<String> = row.get(7)?;
    let fields = fields_json.as_ref().and_then(|v| serde_json::from_str(v).ok());
    Ok(MessageRecord {
        id: row.get(0)?,
        source: row.get(1)?,
        destination: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        timestamp: row.get(5)?,
        direction: row.get(6)?,
        fields,
        receipt_status: row.get(8)?,
        read: row.get::<_, i64>(9)? != 0,
    })
}

fn validated_message_limit(limit: usize) -> rusqlite::Result<i64> {
    if limit > MAX_MESSAGE_QUERY_LIMIT {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "message query limit {limit} exceeds maximum {MAX_MESSAGE_QUERY_LIMIT}"
        )));
    }
    i64::try_from(limit).map_err(|_| {
        rusqlite::Error::InvalidParameterName("message query limit exceeds SQLite range".into())
    })
}

fn validated_page_limit(limit: usize) -> Result<usize, PageError> {
    if !(1..=MAX_MESSAGE_QUERY_LIMIT).contains(&limit) {
        return Err(PageError::InvalidCursor(format!(
            "page limit must be between 1 and {MAX_MESSAGE_QUERY_LIMIT}"
        )));
    }
    Ok(limit)
}

fn canonical_peer_bytes(peer_hash: &str) -> Result<[u8; 16], PageError> {
    if peer_hash.len() != 32
        || !peer_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || peer_hash.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(PageError::InvalidCursor(
            "peer hash must be 32 lowercase hex characters".into(),
        ));
    }
    let bytes = hex::decode(peer_hash)
        .map_err(|_| PageError::InvalidCursor("peer hash is malformed".into()))?;
    bytes.try_into().map_err(|_| PageError::InvalidCursor("peer hash has the wrong length".into()))
}

#[derive(Clone, Copy)]
struct PageMetadata {
    store_id: [u8; 16],
    conversation_epoch: i64,
    cursor_secret: crate::cursor::CursorSecret,
}

fn page_metadata(conn: &Connection) -> rusqlite::Result<PageMetadata> {
    conn.query_row(
        "SELECT store_id, conversation_epoch, cursor_secret
         FROM message_page_metadata
         WHERE singleton = 1
           AND typeof(store_id) = 'blob' AND length(store_id) = 16
           AND typeof(conversation_epoch) = 'integer'
           AND conversation_epoch BETWEEN 0 AND 9223372036854775807
           AND typeof(cursor_secret) = 'blob' AND length(cursor_secret) = 32",
        [],
        |row| {
            let store_id: Vec<u8> = row.get(0)?;
            let store_id = store_id.try_into().map_err(|value: Vec<u8>| {
                rusqlite::Error::FromSqlConversionFailure(
                    value.len(),
                    rusqlite::types::Type::Blob,
                    "page store id must be 16 bytes".into(),
                )
            })?;
            let cursor_secret: Vec<u8> = row.get(2)?;
            let cursor_secret = cursor_secret.try_into().map_err(|value: Vec<u8>| {
                rusqlite::Error::FromSqlConversionFailure(
                    value.len(),
                    rusqlite::types::Type::Blob,
                    "page cursor secret must be 32 bytes".into(),
                )
            })?;
            Ok(PageMetadata { store_id, conversation_epoch: row.get(1)?, cursor_secret })
        },
    )
}

fn charge_projection_budget(used: &mut usize, additional: usize) -> rusqlite::Result<()> {
    *used = used.checked_add(additional).ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("message projection byte budget overflow".into())
    })?;
    if *used > MAX_MESSAGE_PROJECTION_BYTES {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "message projection exceeds {MAX_MESSAGE_PROJECTION_BYTES} byte budget"
        )));
    }
    Ok(())
}

fn charge_message_projection(used: &mut usize, message: &MessageRecord) -> rusqlite::Result<()> {
    let fields = message.fields.as_ref().map_or(0, |value| value.to_string().len());
    charge_projection_budget(
        used,
        MESSAGE_PROJECTION_OVERHEAD_BYTES
            .saturating_add(message.id.len())
            .saturating_add(message.source.len())
            .saturating_add(message.destination.len())
            .saturating_add(message.title.len())
            .saturating_add(message.content.len())
            .saturating_add(message.direction.len())
            .saturating_add(message.receipt_status.as_ref().map_or(0, String::len))
            .saturating_add(fields),
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn open_regular_file_no_follow(
    path: &std::path::Path,
    create_new: bool,
) -> rusqlite::Result<std::fs::File> {
    use std::os::unix::fs::PermissionsExt;

    let mut flags = rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOFOLLOW;
    if create_new {
        flags |= rustix::fs::OFlags::CREATE | rustix::fs::OFlags::EXCL;
    }
    let fd = rustix::fs::open(path, flags, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
        .map_err(|error| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::from_raw_os_error(
                error.raw_os_error(),
            )))
        })?;
    let file = std::fs::File::from(fd);
    if !file
        .metadata()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .is_file()
    {
        return Err(rusqlite::Error::InvalidPath(path.to_path_buf()));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(file)
}

#[cfg(unix)]
fn securely_create_database_file(
    path: &std::path::Path,
) -> rusqlite::Result<(std::path::PathBuf, std::fs::File)> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_name = path.file_name().ok_or_else(|| rusqlite::Error::InvalidPath(path.into()))?;
    let secured_path = parent
        .canonicalize()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .join(file_name);
    match open_regular_file_no_follow(&secured_path, true) {
        Ok(file) => Ok((secured_path, file)),
        Err(rusqlite::Error::ToSqlConversionFailure(error))
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            open_regular_file_no_follow(&secured_path, false).map(|file| (secured_path, file))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn securely_create_database_file(
    path: &std::path::Path,
) -> rusqlite::Result<(std::path::PathBuf, std::fs::File)> {
    let file = match std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        }
        Err(error) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(error))),
    };
    if !file
        .metadata()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .is_file()
    {
        return Err(rusqlite::Error::InvalidPath(path.to_path_buf()));
    }
    Ok((path.to_path_buf(), file))
}

#[cfg(unix)]
fn guarded_file_matches_path(
    path: &std::path::Path,
    guarded: &std::fs::File,
) -> rusqlite::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let guarded_metadata = guarded
        .metadata()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    if !path_metadata.is_file()
        || guarded_metadata.dev() != path_metadata.dev()
        || guarded_metadata.ino() != path_metadata.ino()
    {
        return Err(rusqlite::Error::InvalidPath(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn guarded_file_matches_path(
    path: &std::path::Path,
    _guarded: &std::fs::File,
) -> rusqlite::Result<()> {
    if std::fs::symlink_metadata(path)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
        .is_file()
    {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidPath(path.to_path_buf()))
    }
}

fn open_guarded_connection(
    path: &std::path::Path,
    guarded: &std::fs::File,
) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(path, flags)?;
    guarded_file_matches_path(path, guarded)?;
    Ok(connection)
}

#[cfg(unix)]
fn set_sqlite_sidecar_permissions(path: &std::path::Path) -> rusqlite::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        let sidecar = std::path::PathBuf::from(sidecar);
        match open_regular_file_no_follow(&sidecar, false) {
            Ok(_) => {}
            Err(rusqlite::Error::ToSqlConversionFailure(error))
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_sqlite_sidecar_permissions(_path: &std::path::Path) -> rusqlite::Result<()> {
    Ok(())
}

fn parse_outbound_route_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboundRouteRecord> {
    Ok(OutboundRouteRecord {
        message_id: row.get(0)?,
        requested_method: row.get(1)?,
        actual_method: row.get(2)?,
        representation: row.get(3)?,
        fallback_reason: row.get(4)?,
        correlation_id: row.get(5)?,
        retry_of: row.get(6)?,
        deadline_unix_ms: row.get(7)?,
        state: row.get(8)?,
        attempt_count: row.get(9)?,
    })
}

fn parse_canonical_inbound_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CanonicalInboundRecord> {
    let source: Vec<u8> = row.get(1)?;
    let destination: Vec<u8> = row.get(2)?;
    let invalid_hash = |length| {
        rusqlite::Error::FromSqlConversionFailure(
            length,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "canonical LXMF hash must be 16 bytes",
            )),
        )
    };
    let source = source.try_into().map_err(|value: Vec<u8>| invalid_hash(value.len()))?;
    let destination = destination.try_into().map_err(|value: Vec<u8>| invalid_hash(value.len()))?;
    Ok(CanonicalInboundRecord {
        message_id: row.get(0)?,
        source,
        destination,
        title: row.get(3)?,
        content: row.get(4)?,
        timestamp: row.get(5)?,
        fields_msgpack: row.get(6)?,
        signature: row.get(7)?,
        stamp: row.get(8)?,
        wire: row.get(9)?,
        authentication_state: row.get(10)?,
        stamp_state: row.get(11)?,
        stamp_value: row.get(12)?,
        stamp_target: row.get(13)?,
    })
}

fn parse_draft_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationDraft> {
    let content = match row.get_ref(1)? {
        rusqlite::types::ValueRef::Text(bytes) => {
            std::str::from_utf8(bytes).map(str::to_owned).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("conversation draft is not valid UTF-8: {error}"),
                    )),
                )
            })?
        }
        value => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                value.data_type(),
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "conversation draft content must be SQLite TEXT",
                )),
            ));
        }
    };
    Ok(ConversationDraft { peer_hash: row.get(0)?, content, updated_at: row.get(2)? })
}

#[derive(Clone)]
struct MigratedConversationState {
    peer_hash: String,
    pinned: i64,
    muted: i64,
    updated_at: i64,
}

fn table_has_columns(conn: &Connection, table: &str, required: &[&str]) -> rusqlite::Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
    Ok(required.iter().all(|column| columns.contains(*column)))
}

fn extract_conversation_rows(
    conn: &Connection,
) -> rusqlite::Result<(Vec<MigratedConversationState>, Vec<ConversationDraft>)> {
    let mut states = std::collections::BTreeMap::<
        String,
        ((i64, i64, i64, Vec<u8>), MigratedConversationState),
    >::new();
    if table_has_columns(
        conn,
        "conversation_state",
        &["peer_hash", "pinned", "muted", "updated_at"],
    )? {
        let mut statement = conn.prepare(
            "SELECT CAST(peer_hash AS BLOB), pinned, muted, updated_at
             FROM conversation_state
             WHERE typeof(peer_hash) = 'text'
               AND typeof(pinned) = 'integer' AND pinned IN (0, 1)
               AND typeof(muted) = 'integer' AND muted IN (0, 1)
               AND typeof(updated_at) = 'integer'",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let peer_bytes: Vec<u8> = row.get::<_, Option<Vec<u8>>>(0)?.unwrap_or_default();
            let Ok(peer) = std::str::from_utf8(&peer_bytes) else {
                continue;
            };
            if peer.len() != 32 || !peer.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let pinned: i64 = row.get(1)?;
            let muted: i64 = row.get(2)?;
            let updated_at: i64 = row.get(3)?;
            let canonical = peer.to_ascii_lowercase();
            let authority = (updated_at, pinned, muted, peer_bytes);
            let candidate = MigratedConversationState {
                peer_hash: canonical.clone(),
                pinned,
                muted,
                updated_at,
            };
            if states.get(&canonical).is_none_or(|(existing, _)| authority > *existing) {
                states.insert(canonical, (authority, candidate));
            }
        }
    }

    let mut drafts =
        std::collections::BTreeMap::<String, ((i64, Vec<u8>, Vec<u8>), ConversationDraft)>::new();
    if table_has_columns(conn, "conversation_drafts", &["peer_hash", "content", "updated_at"])? {
        let mut statement = conn.prepare(
            "SELECT CAST(peer_hash AS BLOB), CAST(content AS BLOB), updated_at
             FROM conversation_drafts
             WHERE typeof(peer_hash) = 'text'
               AND typeof(content) = 'text'
               AND length(CAST(content AS BLOB)) <= 65536
               AND typeof(updated_at) = 'integer'",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let peer_bytes: Vec<u8> = row.get(0)?;
            let content_bytes: Vec<u8> = row.get(1)?;
            let (Ok(peer), Ok(content)) =
                (std::str::from_utf8(&peer_bytes), std::str::from_utf8(&content_bytes))
            else {
                continue;
            };
            if peer.len() != 32 || !peer.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let updated_at: i64 = row.get(2)?;
            let canonical = peer.to_ascii_lowercase();
            let content = content.to_owned();
            let authority = (updated_at, content_bytes, peer_bytes);
            let candidate = ConversationDraft { peer_hash: canonical.clone(), content, updated_at };
            if drafts.get(&canonical).is_none_or(|(existing, _)| authority > *existing) {
                drafts.insert(canonical, (authority, candidate));
            }
        }
    }
    Ok((
        states.into_values().map(|(_, state)| state).collect(),
        drafts.into_values().map(|(_, draft)| draft).collect(),
    ))
}

fn rebuild_conversation_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversation_state
             (peer_hash, pinned, muted, updated_at);
         CREATE TABLE IF NOT EXISTS conversation_drafts
             (peer_hash, content, updated_at);",
    )?;
    let (states, drafts) = extract_conversation_rows(conn)?;
    conn.execute_batch(
        "DROP TABLE IF EXISTS conversation_state_v6_weak;
         DROP TABLE IF EXISTS conversation_drafts_v6_weak;
         ALTER TABLE conversation_state RENAME TO conversation_state_v6_weak;
         ALTER TABLE conversation_drafts RENAME TO conversation_drafts_v6_weak;",
    )?;
    conn.execute_batch(CONVERSATION_SCHEMA_DDL)?;
    for state in states {
        conn.execute(
            "INSERT INTO conversation_state (peer_hash, pinned, muted, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![state.peer_hash, state.pinned, state.muted, state.updated_at],
        )?;
    }
    for draft in drafts {
        conn.execute(
            "INSERT INTO conversation_drafts (peer_hash, content, updated_at)
             VALUES (?1, ?2, ?3)",
            params![draft.peer_hash, draft.content, draft.updated_at],
        )?;
    }
    conn.execute_batch(
        "DROP TABLE conversation_state_v6_weak;
         DROP TABLE conversation_drafts_v6_weak;",
    )?;
    Ok(())
}

fn conversation_schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    let state_columns = {
        let mut statement = conn.prepare("PRAGMA table_info(conversation_state)")?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns
    };
    let draft_columns = {
        let mut statement = conn.prepare("PRAGMA table_info(conversation_drafts)")?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns
    };
    let expected_state = vec![
        ("peer_hash".into(), "TEXT".into(), 1, 1),
        ("pinned".into(), "INTEGER".into(), 1, 0),
        ("muted".into(), "INTEGER".into(), 1, 0),
        ("updated_at".into(), "INTEGER".into(), 1, 0),
    ];
    let expected_drafts = vec![
        ("peer_hash".into(), "TEXT".into(), 1, 1),
        ("content".into(), "TEXT".into(), 1, 0),
        ("updated_at".into(), "INTEGER".into(), 1, 0),
    ];
    if state_columns != expected_state || draft_columns != expected_drafts {
        return Ok(false);
    }
    let state_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'conversation_state'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let draft_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'conversation_drafts'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(state_sql) = state_sql.map(|sql| sql.to_ascii_lowercase()) else {
        return Ok(false);
    };
    let Some(draft_sql) = draft_sql.map(|sql| sql.to_ascii_lowercase()) else {
        return Ok(false);
    };
    let has_peer_constraints = |sql: &str| {
        sql.contains("peer_hash text not null primary key")
            && sql.contains("length(peer_hash) = 32")
            && sql.contains("peer_hash = lower(peer_hash)")
            && sql.contains("peer_hash not glob '*[^0-9a-f]*'")
    };
    Ok(has_peer_constraints(&state_sql)
        && has_peer_constraints(&draft_sql)
        && state_sql.contains("pinned integer not null default 0")
        && state_sql.contains("muted integer not null default 0")
        && state_sql.contains("pinned in (0, 1)")
        && state_sql.contains("muted in (0, 1)")
        && state_sql.contains("updated_at integer not null")
        && draft_sql.contains("content text not null")
        && draft_sql.contains("typeof(content) = 'text'")
        && draft_sql.contains("length(cast(content as blob)) <= 65536")
        && draft_sql.contains("updated_at integer not null"))
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get(0),
    )
}

fn pagination_key_schema_is_valid(conn: &Connection) -> rusqlite::Result<bool> {
    if !table_exists(conn, "message_page_keys")?
        || !table_has_columns(
            conn,
            "message_page_keys",
            &["message_id", "ingest_seq", "sort_timestamp", "conversation_peer"],
        )?
    {
        return Ok(false);
    }
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'message_page_keys'",
        [],
        |row| row.get(0),
    )?;
    let sql = sql.to_ascii_lowercase();
    if !sql.contains("message_id text not null unique references messages(id) on delete cascade")
        || !sql.contains("ingest_seq integer primary key autoincrement")
        || !sql.contains("sort_timestamp integer not null")
        || !sql.contains("conversation_peer text not null")
    {
        return Ok(false);
    }
    let foreign_key: Option<(String, String, String)> = conn
        .query_row("PRAGMA foreign_key_list(message_page_keys)", [], |row| {
            Ok((row.get(2)?, row.get(3)?, row.get(6)?))
        })
        .optional()?;
    Ok(foreign_key == Some(("messages".into(), "message_id".into(), "CASCADE".into())))
}

fn pagination_keys_are_coherent(conn: &Connection) -> rusqlite::Result<bool> {
    let incoherent: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM messages m
             LEFT JOIN message_page_keys k ON k.message_id = m.id
             WHERE k.message_id IS NULL
                OR k.sort_timestamp <> m.timestamp
                OR k.conversation_peer <> CASE WHEN m.direction = 'out' THEN
                       CASE WHEN length(m.destination) = 32
                                  AND m.destination NOT GLOB '*[^0-9A-Fa-f]*'
                            THEN lower(m.destination) ELSE m.destination END
                   ELSE CASE WHEN length(m.source) = 32
                                  AND m.source NOT GLOB '*[^0-9A-Fa-f]*'
                            THEN lower(m.source) ELSE m.source END END
             UNION ALL
             SELECT 1 FROM message_page_keys k
             LEFT JOIN messages m ON m.id = k.message_id WHERE m.id IS NULL
         )",
        [],
        |row| row.get(0),
    )?;
    Ok(!incoherent)
}

fn load_existing_page_metadata(conn: &Connection) -> rusqlite::Result<Option<PageMetadata>> {
    if !table_exists(conn, "message_page_metadata")? {
        return Ok(None);
    }
    if !table_has_columns(
        conn,
        "message_page_metadata",
        &["singleton", "store_id", "conversation_epoch"],
    )? {
        return Err(rusqlite::Error::InvalidParameterName(
            "pagination metadata is missing required columns".into(),
        ));
    }
    let row_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM message_page_metadata", [], |row| row.get(0))?;
    let valid: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM message_page_metadata
             WHERE singleton = 1
               AND typeof(store_id) = 'blob' AND length(store_id) = 16
               AND typeof(conversation_epoch) = 'integer'
               AND conversation_epoch BETWEEN 0 AND 9223372036854775807
         )",
        [],
        |row| row.get(0),
    )?;
    if row_count != 1 || !valid {
        return Err(rusqlite::Error::InvalidParameterName(
            "pagination metadata has invalid types or ranges".into(),
        ));
    }
    let has_secret = table_has_columns(conn, "message_page_metadata", &["cursor_secret"])?;
    let (store_id, conversation_epoch): (Vec<u8>, i64) = conn.query_row(
        "SELECT store_id, conversation_epoch FROM message_page_metadata WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let store_id = store_id.try_into().map_err(|value: Vec<u8>| {
        rusqlite::Error::InvalidParameterName(format!(
            "pagination store id has invalid length {}",
            value.len()
        ))
    })?;
    let cursor_secret = if has_secret {
        let valid_secret: bool = conn.query_row(
            "SELECT typeof(cursor_secret) = 'blob' AND length(cursor_secret) = 32
             FROM message_page_metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if !valid_secret {
            return Err(rusqlite::Error::InvalidParameterName(
                "pagination cursor secret has invalid type or length".into(),
            ));
        }
        let secret: Vec<u8> = conn.query_row(
            "SELECT cursor_secret FROM message_page_metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        secret.try_into().map_err(|value: Vec<u8>| {
            rusqlite::Error::InvalidParameterName(format!(
                "pagination cursor secret has invalid length {}",
                value.len()
            ))
        })?
    } else {
        let mut secret = [0_u8; 32];
        OsRng.fill_bytes(&mut secret);
        secret
    };
    Ok(Some(PageMetadata { store_id, conversation_epoch, cursor_secret }))
}

fn random_page_metadata(conversation_epoch: i64) -> PageMetadata {
    let mut store_id = [0_u8; 16];
    let mut cursor_secret = [0_u8; 32];
    OsRng.fill_bytes(&mut store_id);
    OsRng.fill_bytes(&mut cursor_secret);
    PageMetadata { store_id, conversation_epoch, cursor_secret }
}

fn rebuild_page_keys(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS message_page_keys;")?;
    conn.execute_batch(PAGE_KEYS_SCHEMA_DDL)?;
    conn.execute_batch(
        "INSERT INTO message_page_keys (message_id, sort_timestamp, conversation_peer)
         SELECT id, timestamp,
                CASE WHEN direction = 'out' THEN
                    CASE WHEN length(destination) = 32
                              AND destination NOT GLOB '*[^0-9A-Fa-f]*'
                         THEN lower(destination) ELSE destination END
                ELSE CASE WHEN length(source) = 32
                               AND source NOT GLOB '*[^0-9A-Fa-f]*'
                          THEN lower(source) ELSE source END END
         FROM messages ORDER BY timestamp ASC, id ASC;",
    )
}

fn pagination_index_is_valid(
    conn: &Connection,
    name: &str,
    expected: &[(&str, bool)],
) -> rusqlite::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'index' AND name = ?1)",
        params![name],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    let mut statement = conn.prepare(&format!("PRAGMA index_xinfo({name})"))?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(5)? != 0,
            ))
        })?
        .filter_map(|row| match row {
            Ok((Some(column), descending, true)) => Some(Ok((column, descending))),
            Ok((_, _, false)) => None,
            Ok((None, _, true)) => Some(Err(rusqlite::Error::InvalidParameterName(
                "pagination index contains an expression".into(),
            ))),
            Err(error) => Some(Err(error)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(actual
        == expected
            .iter()
            .map(|(column, descending)| ((*column).to_string(), *descending))
            .collect::<Vec<_>>())
}

fn install_pagination_invariants(conn: &Connection) -> rusqlite::Result<()> {
    if !pagination_index_is_valid(
        conn,
        "idx_message_page_peer_order",
        &[("conversation_peer", false), ("sort_timestamp", true), ("ingest_seq", true)],
    )? {
        conn.execute_batch("DROP INDEX IF EXISTS idx_message_page_peer_order;")?;
    }
    if !pagination_index_is_valid(
        conn,
        "idx_message_page_snapshot_conversation",
        &[("ingest_seq", false), ("conversation_peer", false), ("sort_timestamp", true)],
    )? {
        conn.execute_batch("DROP INDEX IF EXISTS idx_message_page_snapshot_conversation;")?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_message_page_peer_order
             ON message_page_keys(conversation_peer, sort_timestamp DESC, ingest_seq DESC);
         CREATE INDEX IF NOT EXISTS idx_message_page_snapshot_conversation
             ON message_page_keys(ingest_seq, conversation_peer, sort_timestamp DESC);
         CREATE TRIGGER messages_page_key_after_insert
         AFTER INSERT ON messages
         BEGIN
             INSERT INTO message_page_keys (message_id, sort_timestamp, conversation_peer)
             VALUES (NEW.id, NEW.timestamp,
                 CASE WHEN NEW.direction = 'out' THEN
                     CASE WHEN length(NEW.destination) = 32
                               AND NEW.destination NOT GLOB '*[^0-9A-Fa-f]*'
                          THEN lower(NEW.destination) ELSE NEW.destination END
                 ELSE CASE WHEN length(NEW.source) = 32
                                AND NEW.source NOT GLOB '*[^0-9A-Fa-f]*'
                           THEN lower(NEW.source) ELSE NEW.source END END);
         END;
         CREATE TRIGGER messages_page_membership_immutable
         BEFORE UPDATE OF source, destination, timestamp, direction ON messages
         WHEN OLD.source <> NEW.source OR OLD.destination <> NEW.destination
              OR OLD.timestamp <> NEW.timestamp OR OLD.direction <> NEW.direction
         BEGIN
             SELECT RAISE(ABORT, 'message page membership and ordering are immutable');
         END;
         CREATE TRIGGER message_page_keys_immutable
         BEFORE UPDATE ON message_page_keys
         BEGIN
             SELECT RAISE(ABORT, 'message page keys are immutable');
         END;
         CREATE TRIGGER message_page_keys_delete_guard
         BEFORE DELETE ON message_page_keys
         WHEN EXISTS(SELECT 1 FROM messages WHERE id = OLD.message_id)
         BEGIN
             SELECT RAISE(ABORT, 'message page key requires parent deletion');
         END;
         CREATE TRIGGER message_page_metadata_secret_immutable
         BEFORE UPDATE OF store_id, cursor_secret ON message_page_metadata
         WHEN OLD.store_id <> NEW.store_id OR OLD.cursor_secret <> NEW.cursor_secret
         BEGIN
             SELECT RAISE(ABORT, 'pagination identity is immutable');
         END;
         CREATE TRIGGER messages_conversation_epoch_after_delete
         AFTER DELETE ON messages
         BEGIN
             SELECT CASE WHEN (SELECT conversation_epoch FROM message_page_metadata
                               WHERE singleton = 1) >= 9223372036854775807
                         THEN RAISE(ABORT, 'conversation epoch exhausted') END;
             UPDATE message_page_metadata SET conversation_epoch = conversation_epoch + 1
             WHERE singleton = 1;
         END;
         CREATE TRIGGER messages_conversation_epoch_after_read
         AFTER UPDATE OF read ON messages
         WHEN COALESCE(OLD.read, 0) <> COALESCE(NEW.read, 0)
         BEGIN
             SELECT CASE WHEN (SELECT conversation_epoch FROM message_page_metadata
                               WHERE singleton = 1) >= 9223372036854775807
                         THEN RAISE(ABORT, 'conversation epoch exhausted') END;
             UPDATE message_page_metadata SET conversation_epoch = conversation_epoch + 1
             WHERE singleton = 1;
         END;
         CREATE TRIGGER conversation_state_epoch_after_insert
         AFTER INSERT ON conversation_state WHEN NEW.pinned = 1
         BEGIN
             SELECT CASE WHEN (SELECT conversation_epoch FROM message_page_metadata
                               WHERE singleton = 1) >= 9223372036854775807
                         THEN RAISE(ABORT, 'conversation epoch exhausted') END;
             UPDATE message_page_metadata SET conversation_epoch = conversation_epoch + 1
             WHERE singleton = 1;
         END;
         CREATE TRIGGER conversation_state_epoch_after_pin
         AFTER UPDATE OF pinned ON conversation_state WHEN OLD.pinned <> NEW.pinned
         BEGIN
             SELECT CASE WHEN (SELECT conversation_epoch FROM message_page_metadata
                               WHERE singleton = 1) >= 9223372036854775807
                         THEN RAISE(ABORT, 'conversation epoch exhausted') END;
             UPDATE message_page_metadata SET conversation_epoch = conversation_epoch + 1
             WHERE singleton = 1;
         END;
         CREATE TRIGGER conversation_state_epoch_after_delete
         AFTER DELETE ON conversation_state WHEN OLD.pinned = 1
         BEGIN
             SELECT CASE WHEN (SELECT conversation_epoch FROM message_page_metadata
                               WHERE singleton = 1) >= 9223372036854775807
                         THEN RAISE(ABORT, 'conversation epoch exhausted') END;
             UPDATE message_page_metadata SET conversation_epoch = conversation_epoch + 1
             WHERE singleton = 1;
         END;",
    )
}

fn pagination_invariants_are_present(conn: &Connection) -> rusqlite::Result<bool> {
    let triggers: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name IN (
             'messages_page_key_after_insert',
             'messages_page_membership_immutable',
             'message_page_keys_immutable',
             'message_page_keys_delete_guard',
             'message_page_metadata_secret_immutable',
             'messages_conversation_epoch_after_delete',
             'messages_conversation_epoch_after_read',
             'conversation_state_epoch_after_insert',
             'conversation_state_epoch_after_pin',
             'conversation_state_epoch_after_delete')",
        [],
        |row| row.get(0),
    )?;
    let indexes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name IN (
             'idx_message_page_peer_order', 'idx_message_page_snapshot_conversation')",
        [],
        |row| row.get(0),
    )?;
    Ok(triggers == 10 && indexes == 2)
}

fn ensure_pagination_schema(conn: &mut Connection, migration_id: &str) -> rusqlite::Result<()> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "DROP TRIGGER IF EXISTS messages_page_key_after_insert;
         DROP TRIGGER IF EXISTS messages_page_membership_immutable;
         DROP TRIGGER IF EXISTS message_page_keys_immutable;
         DROP TRIGGER IF EXISTS message_page_keys_delete_guard;
         DROP TRIGGER IF EXISTS message_page_metadata_secret_immutable;
         DROP TRIGGER IF EXISTS messages_conversation_epoch_after_delete;
         DROP TRIGGER IF EXISTS messages_conversation_epoch_after_read;
         DROP TRIGGER IF EXISTS conversation_state_epoch_after_insert;
         DROP TRIGGER IF EXISTS conversation_state_epoch_after_pin;
         DROP TRIGGER IF EXISTS conversation_state_epoch_after_delete;",
    )?;
    let existing_metadata = load_existing_page_metadata(&transaction)?;
    let keys_valid = pagination_key_schema_is_valid(&transaction)?
        && pagination_keys_are_coherent(&transaction)?;
    let mut metadata = existing_metadata.unwrap_or_else(|| random_page_metadata(0));
    if !keys_valid {
        rebuild_page_keys(&transaction)?;
        metadata = random_page_metadata(metadata.conversation_epoch);
    }
    transaction.execute_batch("DROP TABLE IF EXISTS message_page_metadata;")?;
    transaction.execute_batch(PAGE_METADATA_SCHEMA_DDL)?;
    transaction.execute(
        "INSERT INTO message_page_metadata
             (singleton, store_id, conversation_epoch, cursor_secret) VALUES (1, ?1, ?2, ?3)",
        params![
            metadata.store_id.as_slice(),
            metadata.conversation_epoch,
            metadata.cursor_secret.as_slice()
        ],
    )?;
    install_pagination_invariants(&transaction)?;
    if !pagination_key_schema_is_valid(&transaction)?
        || !pagination_keys_are_coherent(&transaction)?
        || !pagination_invariants_are_present(&transaction)?
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "v8 pagination schema validation failed".into(),
        ));
    }
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (id, applied_at)
         VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
        params![migration_id],
    )?;
    transaction.commit()
}

fn load_projection_snapshots(
    transaction: &rusqlite::Transaction<'_>,
    messages: Vec<MessageRecord>,
    mut projection_bytes: usize,
) -> rusqlite::Result<Vec<MessageProjectionSnapshot>> {
    let mut snapshots = Vec::with_capacity(messages.len());
    for message in messages {
        let canonical = transaction
            .query_row(
                "SELECT c.message_id, c.source, c.destination, c.title, c.content, c.timestamp,
                        c.fields_msgpack, c.signature, c.stamp, c.wire, c.authentication_state,
                        c.stamp_state, c.stamp_value, i.stamp_target
                 FROM canonical_inbound_messages c
                 LEFT JOIN canonical_inbound_inspection i ON i.message_id = c.message_id
                 WHERE c.message_id = ?1",
                params![&message.id],
                parse_canonical_inbound_row,
            )
            .optional()?;
        if let Some(canonical) = canonical.as_ref() {
            charge_projection_budget(
                &mut projection_bytes,
                canonical
                    .title
                    .len()
                    .saturating_add(canonical.content.len())
                    .saturating_add(canonical.fields_msgpack.as_ref().map_or(0, Vec::len))
                    .saturating_add(canonical.signature.as_ref().map_or(0, Vec::len))
                    .saturating_add(canonical.stamp.as_ref().map_or(0, Vec::len))
                    .saturating_add(canonical.wire.len()),
            )?;
        }
        let route = transaction
            .query_row(
                "SELECT message_id, requested_method, actual_method, representation,
                        fallback_reason, correlation_id, retry_of, deadline_unix_ms,
                         state, attempt_count
                 FROM outbound_routes WHERE message_id = ?1",
                params![&message.id],
                parse_outbound_route_row,
            )
            .optional()?;
        let lifecycle = if let Some(route) = route {
            let mut statement = transaction.prepare(
                "SELECT a.message_id, a.attempt_number, a.started_unix_ms,
                        a.deadline_unix_ms, a.state
                 FROM outbound_attempts a
                 JOIN outbound_routes r ON r.message_id = a.message_id
                 WHERE r.correlation_id = ?1
                 ORDER BY a.attempt_number, a.started_unix_ms, a.message_id",
            )?;
            let attempts = statement
                .query_map(params![&route.correlation_id], |row| {
                    Ok(OutboundAttemptRecord {
                        message_id: row.get(0)?,
                        attempt_number: row.get(1)?,
                        started_unix_ms: row.get(2)?,
                        deadline_unix_ms: row.get(3)?,
                        state: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Some((route, attempts))
        } else {
            None
        };
        snapshots.push(MessageProjectionSnapshot { message, canonical, lifecycle });
    }
    Ok(snapshots)
}

fn detach_retry_children(
    transaction: &rusqlite::Transaction<'_>,
    message_ids: &[String],
) -> rusqlite::Result<()> {
    for message_id in message_ids {
        transaction.execute(
            "UPDATE outbound_routes SET retry_of = NULL WHERE retry_of = ?1",
            params![message_id],
        )?;
    }
    Ok(())
}

fn tombstone_standard_propagation_links(
    transaction: &rusqlite::Transaction<'_>,
    message_ids: &[String],
) -> rusqlite::Result<()> {
    let now: i64 =
        transaction
            .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |row| row.get(0))?;
    for message_id in message_ids {
        transaction.execute(
            "UPDATE standard_lxmf_propagation_message_links
             SET state = 'deleted', updated_at = MAX(updated_at, ?2)
             WHERE message_id = ?1",
            params![message_id, now],
        )?;
        transaction.execute(
            "DELETE FROM standard_lxmf_propagation_client_jobs WHERE message_id = ?1",
            params![message_id],
        )?;
    }
    Ok(())
}

pub struct MessagesStore {
    pub(super) conn: Connection,
}

fn validate_attachment_input(input: &AttachmentBlobInput) -> rusqlite::Result<()> {
    if !(1..=255).contains(&input.wire_name.len())
        || input.data.len() > MAX_ATTACHMENT_BLOB_BYTES
        || !matches!(input.source.as_str(), "canonical_binary" | "rust_integer_array" | "local")
        || input.content_type.as_ref().is_some_and(|value| {
            value.len() > 255
                || !value.bytes().all(|byte| byte.is_ascii() && !byte.is_ascii_control())
        })
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "invalid LXMF attachment metadata".into(),
        ));
    }
    Ok(())
}

pub(super) fn stage_attachment_blobs(
    transaction: &rusqlite::Transaction<'_>,
    message_id: &str,
    attachments: &[AttachmentBlobInput],
    now: i64,
) -> rusqlite::Result<()> {
    if attachments.len() > 8 {
        return Err(rusqlite::Error::InvalidParameterName("attachment count exceeds 8".into()));
    }
    let aggregate = attachments.iter().try_fold(0usize, |total, input| {
        validate_attachment_input(input)?;
        total.checked_add(input.data.len()).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("attachment aggregate overflow".into())
        })
    })?;
    if aggregate > MAX_ATTACHMENT_BLOB_BYTES {
        return Err(rusqlite::Error::InvalidParameterName(
            "message attachment aggregate exceeds 768 KiB".into(),
        ));
    }

    let existing_bytes: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(length(data)), 0) FROM attachment_blobs",
        [],
        |row| row.get(0),
    )?;
    let existing_count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM attachment_blobs", [], |row| row.get(0))?;
    let mut new_bytes = 0usize;
    let mut new_count = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    let mut digests = Vec::with_capacity(attachments.len());
    for input in attachments {
        let digest: [u8; 32] = sha2::Sha256::digest(&input.data).into();
        if seen.insert(digest) {
            let existing: Option<(i64, Vec<u8>)> = transaction
                .query_row(
                    "SELECT byte_len, data FROM attachment_blobs WHERE digest = ?1",
                    params![digest.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            match existing {
                Some((byte_len, data))
                    if byte_len == input.data.len() as i64 && data == input.data => {}
                Some(_) => {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "attachment digest/length conflict".into(),
                    ))
                }
                None => {
                    new_count += 1;
                    new_bytes += input.data.len();
                }
            }
        }
        digests.push(digest);
    }
    if existing_count.saturating_add(new_count as i64) > MAX_ATTACHMENT_BLOB_COUNT as i64
        || existing_bytes.saturating_add(new_bytes as i64)
            > MAX_ATTACHMENT_BLOB_AGGREGATE_BYTES as i64
    {
        return Err(rusqlite::Error::InvalidParameterName("attachment blob quota exceeded".into()));
    }

    for (ordinal, (input, digest)) in attachments.iter().zip(digests).enumerate() {
        transaction.execute(
            "INSERT OR IGNORE INTO attachment_blobs
             (digest, byte_len, data, state, created_at, verified_at)
             VALUES (?1, ?2, ?3, 'verified', ?4, ?4)",
            params![digest.as_slice(), input.data.len() as i64, &input.data, now],
        )?;
        transaction.execute(
            "INSERT INTO message_attachments
             (message_id, ordinal, digest, wire_name, content_type, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                message_id,
                ordinal as i64,
                digest.as_slice(),
                &input.wire_name,
                &input.content_type,
                &input.source
            ],
        )?;
    }
    Ok(())
}

fn gc_attachment_blobs(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<usize> {
    transaction.execute(
        "DELETE FROM attachment_blobs
         WHERE NOT EXISTS (SELECT 1 FROM message_attachments ma WHERE ma.digest = attachment_blobs.digest)",
        [],
    )
}

fn attachment_schema_is_valid(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<bool> {
    for (table, required) in [
        (
            "attachment_blobs",
            &[
                "typeof(digest) = 'blob'",
                "length(digest) = 32",
                "byte_len between 0 and 786432",
                "length(data) = byte_len",
                "typeof(state) = 'text'",
                "integrity_failed",
            ][..],
        ),
        (
            "message_attachments",
            &[
                "references messages(id) on delete cascade",
                "ordinal between 0 and 7",
                "references attachment_blobs(digest)",
                "typeof(resource_hash) = 'blob'",
                "between 1 and 255",
                "primary key(message_id, ordinal)",
            ][..],
        ),
        (
            "attachment_issues",
            &[
                "references messages(id) on delete cascade",
                "reason text not null",
                "length(cast(reason as blob)) between 1 and 1024",
            ][..],
        ),
        (
            "attachment_transfers",
            &[
                "references messages(id) on delete cascade",
                "typeof(representation) = 'text'",
                "representation in ('packet', 'resource')",
                "typeof(direction) = 'text'",
                "direction in ('inbound', 'outbound')",
                "typeof(resource_hash) = 'blob'",
                "typeof(transferred) = 'integer'",
                "typeof(total) = 'integer'",
                "typeof(checksum_verified) = 'integer'",
                "transferred <= total",
                "checksum_verified in (0, 1)",
            ][..],
        ),
    ] {
        let sql: Option<String> = transaction
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .optional()?;
        let Some(sql) = sql else { return Ok(false) };
        let normalized = sql.to_ascii_lowercase();
        if required.iter().any(|column| !normalized.contains(column)) {
            return Ok(false);
        }
    }
    let digest_index: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
         WHERE type = 'index' AND name = 'idx_message_attachments_digest'
           AND lower(sql) LIKE '%message_attachments(digest)%')",
        [],
        |row| row.get(0),
    )?;
    if !digest_index {
        return Ok(false);
    }
    Ok(true)
}

fn attachment_schema_has_expected_columns(
    transaction: &rusqlite::Transaction<'_>,
) -> rusqlite::Result<bool> {
    for (table, columns) in [
        (
            "attachment_blobs",
            &["digest", "byte_len", "data", "state", "created_at", "verified_at"][..],
        ),
        (
            "message_attachments",
            &[
                "message_id",
                "ordinal",
                "digest",
                "wire_name",
                "content_type",
                "transfer_id",
                "resource_hash",
                "source",
            ][..],
        ),
        ("attachment_issues", &["message_id", "reason", "created_at"][..]),
        (
            "attachment_transfers",
            &[
                "message_id",
                "transfer_id",
                "resource_hash",
                "representation",
                "direction",
                "state",
                "transferred",
                "total",
                "checksum_verified",
                "error",
                "updated_at",
            ][..],
        ),
    ] {
        let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
        if columns.iter().any(|column| !actual.contains(*column)) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn rebuild_attachment_schema_strict(
    transaction: &rusqlite::Transaction<'_>,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_message_attachments_digest;
         ALTER TABLE message_attachments RENAME TO message_attachments_v9_weak;
         ALTER TABLE attachment_issues RENAME TO attachment_issues_v9_weak;
         ALTER TABLE attachment_transfers RENAME TO attachment_transfers_v9_weak;
         ALTER TABLE attachment_blobs RENAME TO attachment_blobs_v9_weak;
         CREATE TABLE attachment_blobs (
             digest BLOB PRIMARY KEY CHECK(typeof(digest) = 'blob' AND length(digest) = 32),
             byte_len INTEGER NOT NULL CHECK(typeof(byte_len) = 'integer' AND byte_len BETWEEN 0 AND 786432),
             data BLOB NOT NULL CHECK(typeof(data) = 'blob' AND length(data) = byte_len),
             state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('verified', 'integrity_failed')),
             created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0),
             verified_at INTEGER CHECK(verified_at IS NULL OR (typeof(verified_at) = 'integer' AND verified_at >= 0)),
             CHECK((state = 'verified' AND verified_at IS NOT NULL) OR
                   (state = 'integrity_failed' AND verified_at IS NULL))
         );
         CREATE TABLE message_attachments (
             message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
                 CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
             ordinal INTEGER NOT NULL CHECK(typeof(ordinal) = 'integer' AND ordinal BETWEEN 0 AND 7),
             digest BLOB NOT NULL REFERENCES attachment_blobs(digest),
             wire_name TEXT NOT NULL CHECK(typeof(wire_name) = 'text' AND
                 length(CAST(wire_name AS BLOB)) BETWEEN 1 AND 255),
             content_type TEXT CHECK(content_type IS NULL OR
                 (typeof(content_type) = 'text' AND length(CAST(content_type AS BLOB)) <= 255 AND
                  content_type NOT GLOB '*[^ -~]*')),
             transfer_id TEXT CHECK(transfer_id IS NULL OR
                 (typeof(transfer_id) = 'text' AND length(transfer_id) BETWEEN 1 AND 128)),
             resource_hash BLOB CHECK(resource_hash IS NULL OR
                 (typeof(resource_hash) = 'blob' AND length(resource_hash) = 32)),
             source TEXT NOT NULL CHECK(typeof(source) = 'text' AND
                 source IN ('canonical_binary', 'rust_integer_array', 'local')),
             PRIMARY KEY(message_id, ordinal)
         );
         CREATE INDEX idx_message_attachments_digest ON message_attachments(digest);
         CREATE TABLE attachment_issues (
             message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
                 CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
             reason TEXT NOT NULL CHECK(typeof(reason) = 'text' AND
                 length(CAST(reason AS BLOB)) BETWEEN 1 AND 1024),
             created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0)
         );
         CREATE TABLE attachment_transfers (
             message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
                 CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
             transfer_id TEXT NOT NULL CHECK(typeof(transfer_id) = 'text' AND length(transfer_id) BETWEEN 1 AND 128),
             resource_hash BLOB CHECK(resource_hash IS NULL OR
                 (typeof(resource_hash) = 'blob' AND length(resource_hash) = 32)),
             representation TEXT NOT NULL CHECK(typeof(representation) = 'text' AND representation IN ('packet', 'resource')),
             direction TEXT NOT NULL CHECK(typeof(direction) = 'text' AND direction IN ('inbound', 'outbound')),
             state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('queued', 'transferring', 'completed', 'failed', 'cancelled')),
             transferred INTEGER NOT NULL CHECK(typeof(transferred) = 'integer' AND transferred >= 0),
             total INTEGER NOT NULL CHECK(typeof(total) = 'integer' AND total >= 0 AND transferred <= total),
             checksum_verified INTEGER NOT NULL CHECK(typeof(checksum_verified) = 'integer' AND checksum_verified IN (0, 1)),
             error TEXT CHECK(error IS NULL OR (typeof(error) = 'text' AND length(CAST(error AS BLOB)) <= 1024)),
             updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= 0)
         );
         INSERT INTO attachment_blobs
         SELECT digest, byte_len, data, state, created_at, verified_at
         FROM attachment_blobs_v9_weak
         WHERE typeof(digest) = 'blob' AND length(digest) = 32
           AND typeof(byte_len) = 'integer' AND byte_len BETWEEN 0 AND 786432
           AND typeof(data) = 'blob' AND length(data) = byte_len
           AND typeof(state) = 'text' AND state IN ('verified', 'integrity_failed')
           AND typeof(created_at) = 'integer' AND created_at >= 0
           AND (verified_at IS NULL OR (typeof(verified_at) = 'integer' AND verified_at >= 0))
           AND ((state = 'verified' AND verified_at IS NOT NULL) OR
                (state = 'integrity_failed' AND verified_at IS NULL));
         INSERT INTO message_attachments
         SELECT ma.message_id, ma.ordinal, ma.digest, ma.wire_name, ma.content_type,
                ma.transfer_id, ma.resource_hash, ma.source
         FROM message_attachments_v9_weak ma
         JOIN messages m ON m.id = ma.message_id
         JOIN attachment_blobs b ON b.digest = ma.digest
         WHERE typeof(ma.message_id) = 'text' AND length(ma.message_id) BETWEEN 1 AND 128
           AND typeof(ma.ordinal) = 'integer' AND ma.ordinal BETWEEN 0 AND 7
           AND typeof(ma.wire_name) = 'text' AND length(CAST(ma.wire_name AS BLOB)) BETWEEN 1 AND 255
           AND (ma.content_type IS NULL OR (typeof(ma.content_type) = 'text' AND
                length(CAST(ma.content_type AS BLOB)) <= 255 AND ma.content_type NOT GLOB '*[^ -~]*'))
           AND (ma.transfer_id IS NULL OR (typeof(ma.transfer_id) = 'text' AND length(ma.transfer_id) BETWEEN 1 AND 128))
           AND (ma.resource_hash IS NULL OR (typeof(ma.resource_hash) = 'blob' AND length(ma.resource_hash) = 32))
           AND typeof(ma.source) = 'text' AND ma.source IN ('canonical_binary', 'rust_integer_array', 'local');
         INSERT INTO attachment_issues
         SELECT i.message_id, i.reason, i.created_at FROM attachment_issues_v9_weak i
         JOIN messages m ON m.id = i.message_id
         WHERE typeof(i.message_id) = 'text' AND length(i.message_id) BETWEEN 1 AND 128
           AND typeof(i.reason) = 'text' AND length(CAST(i.reason AS BLOB)) BETWEEN 1 AND 1024
           AND typeof(i.created_at) = 'integer' AND i.created_at >= 0;
         INSERT INTO attachment_transfers
         SELECT t.message_id, t.transfer_id, t.resource_hash, t.representation, t.direction,
                t.state, t.transferred, t.total, t.checksum_verified, t.error, t.updated_at
         FROM attachment_transfers_v9_weak t JOIN messages m ON m.id = t.message_id
         WHERE typeof(t.message_id) = 'text' AND length(t.message_id) BETWEEN 1 AND 128
           AND typeof(t.transfer_id) = 'text' AND length(t.transfer_id) BETWEEN 1 AND 128
           AND (t.resource_hash IS NULL OR (typeof(t.resource_hash) = 'blob' AND length(t.resource_hash) = 32))
           AND typeof(t.representation) = 'text' AND t.representation IN ('packet', 'resource')
           AND typeof(t.direction) = 'text' AND t.direction IN ('inbound', 'outbound')
           AND typeof(t.state) = 'text' AND t.state IN ('queued', 'transferring', 'completed', 'failed', 'cancelled')
           AND typeof(t.transferred) = 'integer' AND t.transferred >= 0
           AND typeof(t.total) = 'integer' AND t.total >= 0 AND t.transferred <= t.total
           AND typeof(t.checksum_verified) = 'integer' AND t.checksum_verified IN (0, 1)
           AND (t.error IS NULL OR (typeof(t.error) = 'text' AND length(CAST(t.error AS BLOB)) <= 1024))
           AND typeof(t.updated_at) = 'integer' AND t.updated_at >= 0;
         DROP TABLE message_attachments_v9_weak;
         DROP TABLE attachment_issues_v9_weak;
         DROP TABLE attachment_transfers_v9_weak;
         DROP TABLE attachment_blobs_v9_weak;",
    )
}

fn ensure_attachment_schema(conn: &mut Connection, migration_id: &str) -> rusqlite::Result<()> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let applied: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
        params![migration_id],
        |row| row.get(0),
    )?;
    if !applied {
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS attachment_blobs (
                 digest BLOB PRIMARY KEY
                     CHECK(typeof(digest) = 'blob' AND length(digest) = 32),
                 byte_len INTEGER NOT NULL
                     CHECK(typeof(byte_len) = 'integer' AND byte_len BETWEEN 0 AND 786432),
                 data BLOB NOT NULL
                     CHECK(typeof(data) = 'blob' AND length(data) = byte_len),
                 state TEXT NOT NULL CHECK(typeof(state) = 'text' AND state IN ('verified', 'integrity_failed')),
                 created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0),
                 verified_at INTEGER CHECK(verified_at IS NULL OR (typeof(verified_at) = 'integer' AND verified_at >= 0)),
                 CHECK((state = 'verified' AND verified_at IS NOT NULL) OR
                       (state = 'integrity_failed' AND verified_at IS NULL))
             );
             CREATE TABLE IF NOT EXISTS message_attachments (
                 message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
                     CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
                 ordinal INTEGER NOT NULL CHECK(typeof(ordinal) = 'integer' AND ordinal BETWEEN 0 AND 7),
                 digest BLOB NOT NULL REFERENCES attachment_blobs(digest),
                 wire_name TEXT NOT NULL CHECK(typeof(wire_name) = 'text' AND
                     length(CAST(wire_name AS BLOB)) BETWEEN 1 AND 255),
                 content_type TEXT CHECK(content_type IS NULL OR
                     (typeof(content_type) = 'text' AND length(CAST(content_type AS BLOB)) <= 255 AND
                      content_type NOT GLOB '*[^ -~]*')),
                 transfer_id TEXT CHECK(transfer_id IS NULL OR
                     (typeof(transfer_id) = 'text' AND length(transfer_id) BETWEEN 1 AND 128)),
                 resource_hash BLOB CHECK(resource_hash IS NULL OR
                     (typeof(resource_hash) = 'blob' AND length(resource_hash) = 32)),
                 source TEXT NOT NULL CHECK(typeof(source) = 'text' AND
                     source IN ('canonical_binary', 'rust_integer_array', 'local')),
                 PRIMARY KEY(message_id, ordinal)
             );
             CREATE INDEX IF NOT EXISTS idx_message_attachments_digest ON message_attachments(digest);
             CREATE TABLE IF NOT EXISTS attachment_issues (
                 message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
                     CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
                 reason TEXT NOT NULL CHECK(typeof(reason) = 'text' AND
                     length(CAST(reason AS BLOB)) BETWEEN 1 AND 1024),
                 created_at INTEGER NOT NULL CHECK(typeof(created_at) = 'integer' AND created_at >= 0)
             );
             CREATE TABLE IF NOT EXISTS attachment_transfers (
                 message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
                     CHECK(typeof(message_id) = 'text' AND length(message_id) BETWEEN 1 AND 128),
                 transfer_id TEXT NOT NULL CHECK(typeof(transfer_id) = 'text' AND
                     length(transfer_id) BETWEEN 1 AND 128),
                 resource_hash BLOB CHECK(resource_hash IS NULL OR
                     (typeof(resource_hash) = 'blob' AND length(resource_hash) = 32)),
                 representation TEXT NOT NULL CHECK(typeof(representation) = 'text' AND
                     representation IN ('packet', 'resource')),
                 direction TEXT NOT NULL CHECK(typeof(direction) = 'text' AND
                     direction IN ('inbound', 'outbound')),
                 state TEXT NOT NULL CHECK(typeof(state) = 'text' AND
                     state IN ('queued', 'transferring', 'completed', 'failed', 'cancelled')),
                 transferred INTEGER NOT NULL CHECK(typeof(transferred) = 'integer' AND transferred >= 0),
                 total INTEGER NOT NULL CHECK(typeof(total) = 'integer' AND total >= 0 AND transferred <= total),
                 checksum_verified INTEGER NOT NULL CHECK(typeof(checksum_verified) = 'integer' AND
                     checksum_verified IN (0, 1)),
                 error TEXT CHECK(error IS NULL OR
                     (typeof(error) = 'text' AND length(CAST(error AS BLOB)) <= 1024)),
                 updated_at INTEGER NOT NULL CHECK(typeof(updated_at) = 'integer' AND updated_at >= 0)
             );",
        )?;
        if !attachment_schema_is_valid(&transaction)? {
            return Err(rusqlite::Error::InvalidParameterName(
                "v9 attachment schema validation failed".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO schema_migrations (id, applied_at)
             VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
            params![migration_id],
        )?;
    } else if !attachment_schema_is_valid(&transaction)? {
        if !attachment_schema_has_expected_columns(&transaction)? {
            return Err(rusqlite::Error::InvalidParameterName(
                "v9 attachment schema marker present for malformed schema".into(),
            ));
        }
        rebuild_attachment_schema_strict(&transaction)?;
        if !attachment_schema_is_valid(&transaction)? {
            return Err(rusqlite::Error::InvalidParameterName(
                "v9 attachment strict schema repair failed".into(),
            ));
        }
        transaction.execute(
            "INSERT OR IGNORE INTO schema_migrations (id, applied_at)
             VALUES ('2026-08-24-lxmf-inline-attachments-v9-strict-types',
                     CAST(strftime('%s','now') AS INTEGER))",
            [],
        )?;
    }

    transaction.execute(
        "UPDATE attachment_transfers SET state = 'failed', error = 'daemon_restarted'
         WHERE state IN ('queued', 'transferring')",
        [],
    )?;
    let invalid_references: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM message_attachments ma
             LEFT JOIN messages m ON m.id = ma.message_id
             LEFT JOIN attachment_blobs b ON b.digest = ma.digest
             WHERE m.id IS NULL OR b.digest IS NULL
         ) OR EXISTS(
             SELECT 1 FROM attachment_transfers t LEFT JOIN messages m ON m.id = t.message_id
             WHERE m.id IS NULL
         )",
        [],
        |row| row.get(0),
    )?;
    if invalid_references {
        return Err(rusqlite::Error::InvalidParameterName(
            "v9 attachment reference invariant failed".into(),
        ));
    }
    gc_attachment_blobs(&transaction)?;
    let (retained_count, retained_bytes): (i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(length(data)), 0) FROM attachment_blobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if retained_count > MAX_ATTACHMENT_BLOB_COUNT as i64
        || retained_bytes > MAX_ATTACHMENT_BLOB_AGGREGATE_BYTES as i64
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "v9 attachment quota invariant failed".into(),
        ));
    }
    let corrupt = {
        let mut statement = transaction.prepare(
            "SELECT digest, byte_len, data FROM attachment_blobs WHERE state = 'verified' ORDER BY digest",
        )?;
        let mut rows = statement.query([])?;
        let mut values = Vec::new();
        while let Some(row) = rows.next()? {
            let digest: Vec<u8> = row.get(0)?;
            let byte_len: i64 = row.get(1)?;
            let data: Vec<u8> = row.get(2)?;
            let actual: [u8; 32] = sha2::Sha256::digest(&data).into();
            if byte_len < 0
                || byte_len as usize != data.len()
                || digest.as_slice() != actual.as_slice()
            {
                values.push(digest);
            }
        }
        values
    };
    for digest in corrupt {
        transaction.execute(
            "UPDATE attachment_blobs SET state = 'integrity_failed', verified_at = NULL WHERE digest = ?1",
            params![digest],
        )?;
    }
    transaction.commit()
}

impl MessagesStore {
    #[cfg(test)]
    pub(crate) fn fail_message_deletes_for_test(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TEMP TRIGGER fail_message_delete
             BEFORE DELETE ON messages
             BEGIN SELECT RAISE(ABORT, 'injected message delete failure'); END;",
        )
    }

    const SDK_DOMAIN_SNAPSHOT_KEY: &'static str = "sdk_domains.v1";

    fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;
        Ok(())
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_connection(&conn)?;
        let mut store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let (secured_path, guarded) = securely_create_database_file(path)?;
        let conn = open_guarded_connection(&secured_path, &guarded)?;
        // WAL permits concurrent readers while the busy timeout absorbs brief
        // writer contention between daemon workers and compatibility paths.
        Self::configure_connection(&conn)?;
        let mut store = Self { conn };
        store.init_schema()?;
        guarded_file_matches_path(&secured_path, &guarded)?;
        set_sqlite_sidecar_permissions(&secured_path)?;
        Ok(store)
    }

    pub fn insert_message(&self, record: &MessageRecord) -> rusqlite::Result<()> {
        self.upsert_message(record).map(|_| ())
    }

    /// Insert an inbound message without replacing an existing immutable LXMF
    /// record. Returns `true` only for the first accepted delivery.
    pub fn insert_message_if_absent(&self, record: &MessageRecord) -> rusqlite::Result<bool> {
        self.insert_message_if_missing(record).map(|changed| changed > 0)
    }

    pub fn insert_canonical_inbound_if_absent(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
    ) -> rusqlite::Result<bool> {
        self.insert_canonical_inbound_with_attachments(projection, canonical, &[], None)
    }

    pub fn insert_canonical_inbound_with_attachments(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
        attachments: &[AttachmentBlobInput],
        attachment_issue: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "INSERT OR IGNORE INTO messages (id, source, destination, title, content, timestamp, direction, fields, receipt_status, read) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'in', NULL, NULL, 0)",
            params![
                &projection.id,
                &projection.source,
                &projection.destination,
                &projection.title,
                &projection.content,
                projection.timestamp,
            ],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO canonical_inbound_messages
             (message_id, source, destination, title, content, timestamp, fields_msgpack,
               signature, stamp, wire, authentication_state, stamp_state, stamp_value)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                &canonical.message_id,
                canonical.source.as_slice(),
                canonical.destination.as_slice(),
                &canonical.title,
                &canonical.content,
                canonical.timestamp,
                &canonical.fields_msgpack,
                &canonical.signature,
                &canonical.stamp,
                &canonical.wire,
                &canonical.authentication_state,
                &canonical.stamp_state,
                canonical.stamp_value,
            ],
        )?;
        transaction.execute(
            "INSERT INTO canonical_inbound_inspection (message_id, stamp_target) VALUES (?1, ?2)",
            params![&canonical.message_id, canonical.stamp_target],
        )?;
        if let Some(issue) = attachment_issue {
            transaction.execute(
                "INSERT INTO attachment_issues (message_id, reason, created_at)
                 VALUES (?1, ?2, CAST(strftime('%s','now') AS INTEGER))",
                params![&projection.id, issue],
            )?;
        } else {
            stage_attachment_blobs(
                &transaction,
                &projection.id,
                attachments,
                projection.timestamp.max(0),
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn list_message_attachments(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Vec<MessageAttachmentRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT ma.message_id, ma.ordinal, ma.digest, ma.wire_name, b.byte_len,
                    ma.content_type, b.state, ma.source, COALESCE(ma.transfer_id, t.transfer_id),
                    COALESCE(ma.resource_hash, t.resource_hash),
                    t.representation, t.direction, t.state, COALESCE(t.transferred, 0),
                    COALESCE(t.total, b.byte_len), COALESCE(t.checksum_verified, 0), t.error
             FROM message_attachments ma
             JOIN attachment_blobs b ON b.digest = ma.digest
             LEFT JOIN attachment_transfers t ON t.message_id = ma.message_id
             WHERE ma.message_id = ?1 ORDER BY ma.ordinal",
        )?;
        let mut records: Vec<MessageAttachmentRecord> = statement
            .query_map(params![message_id], |row| {
                let digest: Vec<u8> = row.get(2)?;
                let digest: [u8; 32] = digest.try_into().map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        2,
                        "digest".into(),
                        rusqlite::types::Type::Blob,
                    )
                })?;
                let state: String = row.get(6)?;
                Ok(MessageAttachmentRecord {
                    message_id: row.get(0)?,
                    ordinal: row.get::<_, i64>(1)? as u8,
                    digest,
                    wire_name: row.get(3)?,
                    byte_len: row.get::<_, i64>(4)?.max(0) as u64,
                    content_type: row.get(5)?,
                    availability: if state == "verified" { "available" } else { "unavailable" }
                        .into(),
                    integrity: state,
                    source: row.get(7)?,
                    transfer_id: row.get(8)?,
                    resource_hash: row.get(9)?,
                    representation: row.get(10)?,
                    direction: row.get(11)?,
                    transfer_state: row.get(12)?,
                    transferred: row.get::<_, i64>(13)?.max(0) as u64,
                    total: row.get::<_, i64>(14)?.max(0) as u64,
                    checksum_verified: row.get::<_, i64>(15)? == 1,
                    transfer_error: row.get(16)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if records.is_empty() {
            let issue: Option<String> = self
                .conn
                .query_row(
                    "SELECT reason FROM attachment_issues WHERE message_id = ?1",
                    params![message_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(issue) = issue {
                records.push(MessageAttachmentRecord {
                    message_id: message_id.into(),
                    ordinal: 0,
                    digest: [0; 32],
                    wire_name: "invalid attachment field".into(),
                    byte_len: 0,
                    content_type: None,
                    availability: "unavailable".into(),
                    integrity: "invalid".into(),
                    source: "issue".into(),
                    transfer_id: None,
                    resource_hash: None,
                    representation: None,
                    direction: None,
                    transfer_state: None,
                    transferred: 0,
                    total: 0,
                    checksum_verified: false,
                    transfer_error: Some(issue),
                });
            }
        }
        Ok(records)
    }

    pub fn attachment_blob_usage(&self) -> rusqlite::Result<(u64, u64)> {
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(data)), 0) FROM attachment_blobs",
            [],
            |row| Ok((row.get::<_, i64>(0)?.max(0) as u64, row.get::<_, i64>(1)?.max(0) as u64)),
        )
    }

    pub fn query_attachment_chunk(
        &self,
        message_id: &str,
        ordinal: u8,
        offset: usize,
        max_bytes: usize,
    ) -> rusqlite::Result<Option<AttachmentChunkRecord>> {
        if ordinal > 7 || max_bytes == 0 || max_bytes > 256 * 1024 {
            return Err(rusqlite::Error::InvalidParameterName(
                "invalid attachment chunk range".into(),
            ));
        }
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let row: Option<(Vec<u8>, i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT b.digest, b.byte_len, b.data FROM message_attachments ma
                 JOIN attachment_blobs b ON b.digest = ma.digest
                 WHERE ma.message_id = ?1 AND ma.ordinal = ?2 AND b.state = 'verified'",
                params![message_id, ordinal as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((digest, byte_len, bytes)) = row else { return Ok(None) };
        let actual: [u8; 32] = sha2::Sha256::digest(&bytes).into();
        if byte_len < 0 || byte_len as usize != bytes.len() || digest.as_slice() != actual {
            transaction.execute(
                "UPDATE attachment_blobs SET state = 'integrity_failed', verified_at = NULL
                 WHERE digest = ?1",
                params![digest],
            )?;
            transaction.commit()?;
            return Ok(None);
        }
        if offset > bytes.len() {
            return Err(rusqlite::Error::InvalidParameterName(
                "attachment offset exceeds size".into(),
            ));
        }
        let end = offset.saturating_add(max_bytes).min(bytes.len());
        let data = bytes[offset..end].to_vec();
        transaction.commit()?;
        let info = self
            .list_message_attachments(message_id)?
            .into_iter()
            .find(|record| record.ordinal == ordinal)
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
        Ok(Some(AttachmentChunkRecord {
            attachment: info,
            data,
            next_offset: end,
            done: end == bytes.len(),
        }))
    }

    pub fn update_attachment_transfer_progress(
        &self,
        resource_hash: &[u8; 32],
        transferred: u64,
        total: u64,
    ) -> rusqlite::Result<bool> {
        let transferred = i64::try_from(transferred).unwrap_or(i64::MAX);
        let total = i64::try_from(total).unwrap_or(i64::MAX);
        if transferred > total {
            return Err(rusqlite::Error::InvalidParameterName(
                "attachment transfer progress exceeds total".into(),
            ));
        }
        Ok(self.conn.execute(
            "UPDATE attachment_transfers SET state = 'transferring', transferred = ?2,
                 total = ?3, updated_at = CAST(strftime('%s','now') AS INTEGER)
             WHERE resource_hash = ?1 AND state = 'transferring'",
            params![resource_hash.as_slice(), transferred, total],
        )? > 0)
    }

    pub fn canonical_inbound(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Option<CanonicalInboundRecord>> {
        self.conn
            .query_row(
                "SELECT c.message_id, c.source, c.destination, c.title, c.content, c.timestamp,
                        c.fields_msgpack, c.signature, c.stamp, c.wire, c.authentication_state,
                        c.stamp_state, c.stamp_value, i.stamp_target
                 FROM canonical_inbound_messages c
                 LEFT JOIN canonical_inbound_inspection i ON i.message_id = c.message_id
                 WHERE c.message_id = ?1",
                params![message_id],
                parse_canonical_inbound_row,
            )
            .optional()
    }

    pub fn unknown_identity_messages(
        &self,
        source: &[u8; 16],
        limit: usize,
    ) -> rusqlite::Result<Vec<CanonicalInboundRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT c.message_id, c.source, c.destination, c.title, c.content, c.timestamp,
                    c.fields_msgpack, c.signature, c.stamp, c.wire, c.authentication_state,
                    c.stamp_state, c.stamp_value, i.stamp_target
             FROM canonical_inbound_messages c
             LEFT JOIN canonical_inbound_inspection i ON i.message_id = c.message_id
             WHERE c.source = ?1 AND c.authentication_state = 'unknown_identity'
             ORDER BY c.rowid LIMIT ?2",
        )?;
        let records = statement
            .query_map(
                params![source.as_slice(), limit.min(1024) as i64],
                parse_canonical_inbound_row,
            )?
            .collect();
        records
    }

    pub fn update_authentication_state(
        &self,
        message_id: &str,
        state: &str,
    ) -> rusqlite::Result<bool> {
        if !matches!(state, "verified" | "invalid") {
            return Ok(false);
        }
        Ok(self.conn.execute(
            "UPDATE canonical_inbound_messages SET authentication_state = ?2
             WHERE message_id = ?1 AND authentication_state = 'unknown_identity'",
            params![message_id, state],
        )? > 0)
    }

    pub fn lxmf_stamp_policy(&self) -> rusqlite::Result<LxmfStampPolicy> {
        self.conn.query_row(
            "SELECT target_cost, flexibility FROM lxmf_stamp_policy WHERE singleton = 1",
            [],
            |row| Ok(LxmfStampPolicy { target_cost: row.get(0)?, flexibility: row.get(1)? }),
        )
    }

    pub fn set_lxmf_stamp_policy(&self, policy: LxmfStampPolicy) -> rusqlite::Result<()> {
        if policy.target_cost > lxmf::stamps::MAX_STAMP_COST
            || policy.flexibility > lxmf::stamps::MAX_STAMP_COST
        {
            return Err(rusqlite::Error::InvalidParameterName("stamp cost exceeds limit".into()));
        }
        self.conn.execute(
            "UPDATE lxmf_stamp_policy SET target_cost = ?1, flexibility = ?2 WHERE singleton = 1",
            params![policy.target_cost, policy.flexibility],
        )?;
        Ok(())
    }

    pub fn learn_peer_stamp_cost(
        &self,
        peer: &str,
        cost: u32,
        observed_at: i64,
    ) -> rusqlite::Result<()> {
        if peer.len() > 128 || cost > lxmf::stamps::MAX_STAMP_COST {
            return Err(rusqlite::Error::InvalidParameterName("invalid peer stamp cost".into()));
        }
        self.conn.execute(
            "INSERT INTO lxmf_peer_costs (peer, stamp_cost, observed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(peer) DO UPDATE SET stamp_cost = excluded.stamp_cost,
                 observed_at = excluded.observed_at WHERE excluded.observed_at >= lxmf_peer_costs.observed_at",
            params![peer, cost, observed_at],
        )?;
        Ok(())
    }

    pub fn peer_stamp_cost(&self, peer: &str) -> rusqlite::Result<Option<u32>> {
        self.conn
            .query_row(
                "SELECT stamp_cost FROM lxmf_peer_costs WHERE peer = ?1",
                params![peer],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn upsert_lxmf_ticket(&self, record: &LxmfTicketRecord) -> rusqlite::Result<()> {
        if record.peer.len() > 128
            || record.ticket.len() != lxmf::stamps::TICKET_LENGTH
            || !matches!(record.direction.as_str(), "issued" | "received")
        {
            return Err(rusqlite::Error::InvalidParameterName("invalid LXMF ticket".into()));
        }
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO lxmf_tickets (peer, ticket, expires_at, direction) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(peer, direction, ticket) DO UPDATE SET expires_at = excluded.expires_at",
            params![&record.peer, &record.ticket, record.expires_at, &record.direction],
        )?;
        transaction.execute(
            "DELETE FROM lxmf_tickets WHERE rowid IN (
                 SELECT rowid FROM lxmf_tickets ORDER BY expires_at DESC LIMIT -1 OFFSET 2048
             )",
            [],
        )?;
        transaction.commit()
    }

    pub fn active_lxmf_ticket(
        &self,
        peer: &str,
        direction: &str,
        now: i64,
    ) -> rusqlite::Result<Option<LxmfTicketRecord>> {
        self.conn
            .query_row(
                "SELECT peer, ticket, expires_at, direction FROM lxmf_tickets
                 WHERE peer = ?1 AND direction = ?2 AND expires_at > ?3
                 ORDER BY expires_at DESC LIMIT 1",
                params![peer, direction, now],
                |row| {
                    Ok(LxmfTicketRecord {
                        peer: row.get(0)?,
                        ticket: row.get(1)?,
                        expires_at: row.get(2)?,
                        direction: row.get(3)?,
                    })
                },
            )
            .optional()
    }

    pub fn expire_lxmf_tickets(&self, now: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM lxmf_tickets
             WHERE expires_at + CASE WHEN direction = 'issued' THEN ?2 ELSE 0 END <= ?1",
            params![now, lxmf::stamps::TICKET_GRACE_SECS],
        )
    }

    pub fn issued_lxmf_tickets(&self, peer: &str, now: i64) -> rusqlite::Result<Vec<Vec<u8>>> {
        let mut statement = self.conn.prepare(
            "SELECT ticket FROM lxmf_tickets WHERE peer = ?1 AND direction = 'issued'
             AND expires_at > ?2 ORDER BY expires_at DESC LIMIT 8",
        )?;
        let records = statement.query_map(params![peer, now], |row| row.get(0))?.collect();
        records
    }

    fn upsert_message(&self, record: &MessageRecord) -> rusqlite::Result<usize> {
        let fields_json =
            record.fields.as_ref().map(|value| serde_json::to_string(value).unwrap_or_default());
        self.conn.execute(
            "INSERT INTO messages
             (id, source, destination, title, content, timestamp, direction, fields,
              receipt_status, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 content = excluded.content,
                 fields = excluded.fields,
                 receipt_status = excluded.receipt_status,
                 read = excluded.read",
            params![
                &record.id,
                &record.source,
                &record.destination,
                &record.title,
                &record.content,
                record.timestamp,
                &record.direction,
                fields_json,
                &record.receipt_status,
                record.read as i64,
            ],
        )
    }

    fn insert_message_if_missing(&self, record: &MessageRecord) -> rusqlite::Result<usize> {
        let fields_json =
            record.fields.as_ref().map(|value| serde_json::to_string(value).unwrap_or_default());
        self.conn.execute(
            "INSERT OR IGNORE INTO messages
             (id, source, destination, title, content, timestamp, direction, fields,
              receipt_status, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &record.id,
                &record.source,
                &record.destination,
                &record.title,
                &record.content,
                record.timestamp,
                &record.direction,
                fields_json,
                &record.receipt_status,
                record.read as i64,
            ],
        )
    }

    pub fn insert_outbound_message(
        &self,
        message: &MessageRecord,
        route: &OutboundRouteRecord,
    ) -> rusqlite::Result<()> {
        self.insert_outbound_message_with_ticket_offer(message, route, None)
    }

    pub fn insert_outbound_message_with_ticket_offer(
        &self,
        message: &MessageRecord,
        route: &OutboundRouteRecord,
        reservation: Option<&LxmfTicketOfferReservation>,
    ) -> rusqlite::Result<()> {
        self.insert_outbound_message_with_attachments(message, route, reservation, &[], 0)
    }

    pub fn insert_outbound_message_with_attachments(
        &self,
        message: &MessageRecord,
        route: &OutboundRouteRecord,
        reservation: Option<&LxmfTicketOfferReservation>,
        attachments: &[AttachmentBlobInput],
        transfer_total: usize,
    ) -> rusqlite::Result<()> {
        self.insert_outbound_message_with_attachments_and_propagation(
            message,
            route,
            reservation,
            attachments,
            transfer_total,
            None,
        )
    }

    pub fn insert_outbound_message_with_attachments_and_propagation(
        &self,
        message: &MessageRecord,
        route: &OutboundRouteRecord,
        reservation: Option<&LxmfTicketOfferReservation>,
        attachments: &[AttachmentBlobInput],
        transfer_total: usize,
        propagation: Option<&crate::storage::standard_propagation::StandardPropagationClientJob>,
    ) -> rusqlite::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let fields_json =
            message.fields.as_ref().map(|value| serde_json::to_string(value).unwrap_or_default());
        transaction.execute(
            "INSERT INTO messages (id, source, destination, title, content, timestamp, direction, fields, receipt_status, read) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &message.id,
                &message.source,
                &message.destination,
                &message.title,
                &message.content,
                message.timestamp,
                &message.direction,
                fields_json,
                &message.receipt_status,
                message.read as i64,
            ],
        )?;
        stage_attachment_blobs(&transaction, &message.id, attachments, message.timestamp.max(0))?;
        if !attachments.is_empty() {
            transaction.execute(
                "INSERT INTO attachment_transfers
                 (message_id, transfer_id, representation, direction, state, transferred, total,
                  checksum_verified, error, updated_at)
                 VALUES (?1, ?2, ?3, 'outbound', 'queued', 0, ?4, 0, NULL, ?5)",
                params![
                    &message.id,
                    &route.correlation_id,
                    &route.representation,
                    i64::try_from(transfer_total).unwrap_or(i64::MAX),
                    message.timestamp.max(0),
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO outbound_routes (message_id, requested_method, actual_method, representation, fallback_reason, correlation_id, retry_of, deadline_unix_ms, state, attempt_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &route.message_id,
                &route.requested_method,
                &route.actual_method,
                &route.representation,
                &route.fallback_reason,
                &route.correlation_id,
                &route.retry_of,
                route.deadline_unix_ms,
                &route.state,
                route.attempt_count,
            ],
        )?;
        transaction.execute(
            "INSERT INTO outbound_message_inspection (message_id, terminal_detail) VALUES (?1, NULL)",
            params![&route.message_id],
        )?;
        if let Some(propagation) = propagation {
            if route.actual_method != "propagated" || propagation.message_id != message.id {
                return Err(rusqlite::Error::InvalidParameterName(
                    "propagation spool does not match outbound route".into(),
                ));
            }
            crate::storage::standard_propagation::spool_outbound_in_transaction(
                &transaction,
                propagation,
            )?;
        }
        if let Some(reservation) = reservation {
            let ticket = &reservation.ticket;
            if ticket.peer.len() > 128
                || ticket.ticket.len() != lxmf::stamps::TICKET_LENGTH
                || ticket.direction != "issued"
            {
                return Err(rusqlite::Error::InvalidParameterName(
                    "invalid outbound LXMF ticket offer".into(),
                ));
            }
            let released = transaction.execute(
                "DELETE FROM lxmf_ticket_offer_reservations
                 WHERE peer = ?1 AND reservation_id = ?2",
                params![&ticket.peer, &reservation.reservation_id],
            )?;
            if released != 1 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "LXMF ticket offer reservation is missing or stale".into(),
                ));
            }
            transaction.execute(
                "INSERT INTO outbound_ticket_offers
                 (message_id, peer, ticket, expires_at, delivered_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                params![&message.id, &ticket.peer, &ticket.ticket, ticket.expires_at],
            )?;
        }
        transaction.commit()
    }

    pub fn outbound_route(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Option<OutboundRouteRecord>> {
        self.conn
            .query_row(
                "SELECT message_id, requested_method, actual_method, representation, fallback_reason, correlation_id, retry_of, deadline_unix_ms, state, attempt_count FROM outbound_routes WHERE message_id = ?1",
                params![message_id],
                parse_outbound_route_row,
            )
            .optional()
    }

    pub fn outbound_routes(&self) -> rusqlite::Result<Vec<OutboundRouteRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT message_id, requested_method, actual_method, representation, fallback_reason, correlation_id, retry_of, deadline_unix_ms, state, attempt_count FROM outbound_routes ORDER BY rowid",
        )?;
        let records = statement.query_map([], parse_outbound_route_row)?.collect();
        records
    }

    pub fn outbound_retry_for(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Option<OutboundRouteRecord>> {
        self.conn
            .query_row(
                "SELECT message_id, requested_method, actual_method, representation, fallback_reason, correlation_id, retry_of, deadline_unix_ms, state, attempt_count FROM outbound_routes WHERE retry_of = ?1",
                params![message_id],
                parse_outbound_route_row,
            )
            .optional()
    }

    pub fn track_outbound_evidence(
        &self,
        evidence_id: &str,
        message_id: &str,
        kind: &str,
    ) -> rusqlite::Result<bool> {
        if evidence_id.len() != 64
            || evidence_id
                .bytes()
                .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
            || !matches!(kind, "packet" | "resource")
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "delivery evidence must be a 32-byte lowercase hexadecimal hash".into(),
            ));
        }
        let transaction = self.conn.unchecked_transaction()?;
        let count = transaction.execute(
            "INSERT INTO outbound_evidence (evidence_id, message_id, kind)
             SELECT ?1, ?2, ?3 WHERE EXISTS (
                  SELECT 1 FROM outbound_routes WHERE message_id = ?2
                    AND representation = ?3
                    AND attempt_count BETWEEN 1 AND 32
                    AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')
             ) ON CONFLICT(evidence_id) DO NOTHING",
            params![evidence_id, message_id, kind],
        )?;
        if count > 0 {
            let evidence_kind =
                if kind == "packet" { "packet_receipt" } else { "resource_completion" };
            transaction.execute(
                "INSERT INTO message_delivery_evidence
                 (evidence_hash, message_id, kind, representation, state, outcome,
                   attempt_number, correlation_id, observed_at, terminal_at,
                   transferred_bytes, total_bytes, progress)
                  SELECT ?1, r.message_id, ?4, r.representation, 'tracked', NULL,
                         r.attempt_count, r.correlation_id,
                         CAST(strftime('%s','now') AS INTEGER), NULL, NULL, NULL, NULL
                 FROM outbound_routes r WHERE r.message_id = ?2 AND r.representation = ?3",
                params![evidence_id, message_id, kind, evidence_kind],
            )?;
            transaction.execute(
                "DELETE FROM outbound_evidence
                 WHERE evidence_id IN (
                     SELECT evidence_hash FROM message_delivery_evidence WHERE message_id = ?1
                     ORDER BY observed_at DESC, evidence_hash DESC LIMIT -1 OFFSET ?2
                 )",
                params![message_id, MAX_DELIVERY_EVIDENCE_PER_MESSAGE as i64],
            )?;
            transaction.execute(
                "DELETE FROM message_delivery_evidence
                 WHERE message_id = ?1 AND evidence_hash IN (
                     SELECT evidence_hash FROM message_delivery_evidence WHERE message_id = ?1
                     ORDER BY observed_at DESC, evidence_hash DESC LIMIT -1 OFFSET ?2
                 )",
                params![message_id, MAX_DELIVERY_EVIDENCE_PER_MESSAGE as i64],
            )?;
            transaction.execute(
                "DELETE FROM outbound_evidence
                 WHERE evidence_id IN (
                     SELECT evidence_hash FROM message_delivery_evidence
                     WHERE terminal_at IS NOT NULL
                       AND terminal_at <= CAST(strftime('%s','now') AS INTEGER) - ?1
                 )",
                params![DELIVERY_EVIDENCE_RETENTION_SECS],
            )?;
            transaction.execute(
                "DELETE FROM message_delivery_evidence
                 WHERE terminal_at IS NOT NULL AND terminal_at <= CAST(strftime('%s','now') AS INTEGER) - ?1",
                params![DELIVERY_EVIDENCE_RETENTION_SECS],
            )?;
        }
        let has_attachment_transfer: bool = count > 0
            && kind == "resource"
            && transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM attachment_transfers WHERE message_id = ?1)",
                params![message_id],
                |row| row.get(0),
            )?;
        if has_attachment_transfer {
            let resource_hash = hex::decode(evidence_id).map_err(|_| {
                rusqlite::Error::InvalidParameterName("resource evidence is not hex".into())
            })?;
            if resource_hash.len() != 32 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "resource evidence hash must be 32 bytes".into(),
                ));
            }
            transaction.execute(
                "UPDATE attachment_transfers SET resource_hash = ?2 WHERE message_id = ?1",
                params![message_id, &resource_hash],
            )?;
            transaction.execute(
                "UPDATE message_attachments SET resource_hash = ?2, transfer_id = (
                     SELECT transfer_id FROM attachment_transfers WHERE message_id = ?1
                 ) WHERE message_id = ?1",
                params![message_id, &resource_hash],
            )?;
        }
        transaction.commit()?;
        Ok(count > 0)
    }

    pub fn outbound_evidence(
        &self,
        evidence_id: &str,
    ) -> rusqlite::Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT message_id, kind FROM outbound_evidence WHERE evidence_id = ?1",
                params![evidence_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    pub fn outbound_evidence_for_message(
        &self,
        message_id: &str,
        kind: &str,
    ) -> rusqlite::Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT evidence_id, kind FROM outbound_evidence WHERE message_id = ?1 AND kind = ?2 ORDER BY rowid DESC LIMIT 1",
                params![message_id, kind],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    pub fn update_delivery_evidence_progress(
        &self,
        evidence_hash: &str,
        transferred: u64,
        total: u64,
    ) -> rusqlite::Result<bool> {
        if transferred > total {
            return Err(rusqlite::Error::InvalidParameterName(
                "resource evidence progress exceeds total".into(),
            ));
        }
        let transferred = i64::try_from(transferred).map_err(|_| {
            rusqlite::Error::InvalidParameterName(
                "resource evidence progress exceeds SQLite range".into(),
            )
        })?;
        let total = i64::try_from(total).map_err(|_| {
            rusqlite::Error::InvalidParameterName(
                "resource evidence total exceeds SQLite range".into(),
            )
        })?;
        let progress = if total == 0 {
            0
        } else {
            ((transferred as u128 * 100) / total as u128).min(100) as i64
        };
        Ok(self.conn.execute(
            "UPDATE message_delivery_evidence
             SET transferred_bytes = ?2, total_bytes = ?3, progress = ?4
             WHERE evidence_hash = ?1 AND kind = 'resource_completion'
               AND representation = 'resource' AND state = 'tracked'",
            params![evidence_hash, transferred, total, progress],
        )? > 0)
    }

    pub fn due_outbound_resource_evidence(
        &self,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT r.message_id, e.evidence_id
             FROM outbound_routes r
             JOIN outbound_evidence e ON e.message_id = r.message_id AND e.kind = 'resource'
             WHERE r.deadline_unix_ms <= ?1
               AND r.state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')
             ORDER BY r.message_id, e.rowid DESC",
        )?;
        let records = statement
            .query_map(params![now_unix_ms], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect();
        records
    }

    pub fn begin_outbound_attempt(
        &self,
        attempt: &OutboundAttemptRecord,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE outbound_routes SET state = 'sending', attempt_count = ?2 WHERE message_id = ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')",
            params![&attempt.message_id, attempt.attempt_number],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO outbound_attempts (message_id, attempt_number, started_unix_ms, deadline_unix_ms, state) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &attempt.message_id,
                attempt.attempt_number,
                attempt.started_unix_ms,
                attempt.deadline_unix_ms,
                &attempt.state,
            ],
        )?;
        transaction.execute(
            "UPDATE attachment_transfers SET state = 'transferring', updated_at = ?2
             WHERE message_id = ?1 AND state = 'queued'",
            params![&attempt.message_id, attempt.started_unix_ms / 1000],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn finish_outbound(
        &self,
        message_id: &str,
        state: &str,
        receipt_status: &str,
    ) -> rusqlite::Result<bool> {
        self.finish_outbound_with_detail(message_id, state, receipt_status, None)
    }

    pub fn finish_outbound_with_detail(
        &self,
        message_id: &str,
        state: &str,
        receipt_status: &str,
        terminal_detail: Option<&str>,
    ) -> rusqlite::Result<bool> {
        self.finish_outbound_with_detail_and_evidence(
            message_id,
            state,
            receipt_status,
            terminal_detail,
            None,
        )
    }

    pub fn finish_outbound_with_exact_evidence(
        &self,
        message_id: &str,
        state: &str,
        receipt_status: &str,
        terminal_detail: Option<&str>,
        evidence_hash: &str,
        evidence_kind: &str,
    ) -> rusqlite::Result<bool> {
        self.finish_outbound_with_detail_and_evidence(
            message_id,
            state,
            receipt_status,
            terminal_detail,
            Some((evidence_hash, evidence_kind)),
        )
    }

    fn finish_outbound_with_detail_and_evidence(
        &self,
        message_id: &str,
        state: &str,
        receipt_status: &str,
        terminal_detail: Option<&str>,
        exact_evidence: Option<(&str, &str)>,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        if let Some((evidence_hash, evidence_kind)) = exact_evidence {
            let stored_kind = match evidence_kind {
                "packet" => "packet_receipt",
                "resource" => "resource_completion",
                _ => return Ok(false),
            };
            let exact: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM message_delivery_evidence e
                    JOIN outbound_evidence o ON o.evidence_id = e.evidence_hash
                    JOIN outbound_routes r ON r.message_id = e.message_id
                    JOIN outbound_attempts a ON a.message_id = e.message_id
                        AND a.attempt_number = e.attempt_number
                    WHERE e.evidence_hash = ?1 AND e.message_id = ?2
                      AND e.kind = ?3 AND e.representation = ?4 AND e.state = 'tracked'
                      AND o.message_id = e.message_id AND o.kind = ?4
                      AND r.representation = ?4
                )",
                params![evidence_hash, message_id, stored_kind, evidence_kind],
                |row| row.get(0),
            )?;
            if !exact {
                return Ok(false);
            }
        }
        let changed = if state == "sent" {
            transaction.execute(
                "UPDATE outbound_routes SET state = ?2 WHERE message_id = ?1 AND (
                    state IN ('queued', 'sending') OR (
                        state = 'sent' AND COALESCE((SELECT receipt_status FROM messages WHERE id = ?1), '') != ?3
                    )
                )",
                params![message_id, state, receipt_status],
            )?
        } else {
            transaction.execute(
                "UPDATE outbound_routes SET state = ?2 WHERE message_id = ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')",
                params![message_id, state],
            )?
        };
        if changed == 0 {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE messages SET receipt_status = ?2 WHERE id = ?1",
            params![message_id, receipt_status],
        )?;
        transaction.execute(
            "UPDATE outbound_message_inspection SET terminal_detail = ?2 WHERE message_id = ?1",
            params![message_id, if state == "sent" { None } else { terminal_detail }],
        )?;
        transaction.execute(
            "UPDATE outbound_attempts SET state = ?2 WHERE message_id = ?1 AND attempt_number = (SELECT MAX(attempt_number) FROM outbound_attempts WHERE message_id = ?1)",
            params![message_id, state],
        )?;
        let transfer_state = match state {
            "delivered" => Some(("completed", None)),
            "failed" | "expired" | "rejected" => Some(("failed", Some(receipt_status))),
            "cancelled" => Some(("cancelled", Some(receipt_status))),
            _ => None,
        };
        if let Some((transfer_state, error)) = transfer_state {
            transaction.execute(
                "UPDATE attachment_transfers
                 SET state = ?2,
                     transferred = CASE WHEN ?2 = 'completed' THEN total ELSE transferred END,
                     checksum_verified = CASE WHEN ?2 = 'completed' THEN 1 ELSE checksum_verified END,
                     error = ?3,
                     updated_at = CAST(strftime('%s','now') AS INTEGER)
                 WHERE message_id = ?1 AND state NOT IN ('completed', 'failed', 'cancelled')",
                params![message_id, transfer_state, error],
            )?;
        } else if state == "sent" {
            transaction.execute(
                "UPDATE attachment_transfers
                 SET state = 'completed', transferred = total, checksum_verified = 1,
                     updated_at = CAST(strftime('%s','now') AS INTEGER)
                 WHERE message_id = ?1 AND representation = 'packet'
                   AND state NOT IN ('completed', 'failed', 'cancelled')",
                params![message_id],
            )?;
        }
        if matches!(state, "delivered" | "failed" | "cancelled" | "expired" | "rejected") {
            let now = unix_time_secs();
            if let Some((evidence_hash, evidence_kind)) = exact_evidence {
                let stored_kind = if evidence_kind == "packet" {
                    "packet_receipt"
                } else {
                    "resource_completion"
                };
                let exact_state = match state {
                    "delivered" => "completed",
                    "cancelled" => "cancelled",
                    _ => "failed",
                };
                transaction.execute(
                    "UPDATE message_delivery_evidence
                     SET state = ?4, outcome = ?5, terminal_at = ?6,
                         transferred_bytes = CASE WHEN representation = 'resource' AND ?4 = 'completed' THEN COALESCE(total_bytes, transferred_bytes) ELSE transferred_bytes END,
                         progress = CASE WHEN representation = 'resource' AND ?4 = 'completed' AND total_bytes IS NOT NULL THEN 100 ELSE progress END
                     WHERE evidence_hash = ?1 AND message_id = ?2 AND kind = ?3 AND state = 'tracked'",
                    params![evidence_hash, message_id, stored_kind, exact_state, terminal_detail, now],
                )?;
                transaction.execute(
                    "UPDATE message_delivery_evidence
                     SET state = CASE WHEN ?2 = 'cancelled' THEN 'cancelled' ELSE 'failed' END,
                         outcome = 'message terminalized by different exact evidence', terminal_at = ?3
                     WHERE message_id = ?1 AND evidence_hash != ?4 AND state = 'tracked'",
                    params![message_id, state, now, evidence_hash],
                )?;
            } else {
                let evidence_state = if state == "cancelled" { "cancelled" } else { "failed" };
                transaction.execute(
                    "UPDATE message_delivery_evidence
                     SET state = ?2, outcome = ?3, terminal_at = ?4
                     WHERE message_id = ?1 AND state = 'tracked'",
                    params![message_id, evidence_state, terminal_detail, now],
                )?;
            }
            transaction.execute(
                "DELETE FROM outbound_evidence WHERE message_id = ?1",
                params![message_id],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn message_delivery_evidence(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Vec<MessageDeliveryEvidenceRecord>> {
        self.prune_delivery_evidence(unix_time_secs())?;
        let mut statement = self.conn.prepare(
            "SELECT message_id, kind, evidence_hash, representation, state, outcome,
                    attempt_number, correlation_id, observed_at, terminal_at,
                    transferred_bytes, total_bytes, progress
             FROM message_delivery_evidence WHERE message_id = ?1
             ORDER BY observed_at, evidence_hash LIMIT ?2",
        )?;
        let records = statement
            .query_map(params![message_id, MAX_DELIVERY_EVIDENCE_PER_MESSAGE as i64], |row| {
                Ok(MessageDeliveryEvidenceRecord {
                    message_id: row.get(0)?,
                    kind: row.get(1)?,
                    evidence_hash: row.get(2)?,
                    representation: row.get(3)?,
                    state: row.get(4)?,
                    outcome: row.get(5)?,
                    attempt_number: row.get(6)?,
                    correlation_id: row.get(7)?,
                    observed_at: row.get(8)?,
                    terminal_at: row.get(9)?,
                    transferred_bytes: row.get(10)?,
                    total_bytes: row.get(11)?,
                    progress: row.get(12)?,
                })
            })?
            .collect();
        records
    }

    pub fn prune_delivery_evidence(&self, now: i64) -> rusqlite::Result<usize> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM outbound_evidence
             WHERE evidence_id IN (
                 SELECT evidence_hash FROM message_delivery_evidence
                 WHERE terminal_at IS NOT NULL AND terminal_at <= ?1 - ?2
             )",
            params![now.max(0), DELIVERY_EVIDENCE_RETENTION_SECS],
        )?;
        let removed = transaction.execute(
            "DELETE FROM message_delivery_evidence
             WHERE terminal_at IS NOT NULL AND terminal_at <= ?1 - ?2",
            params![now.max(0), DELIVERY_EVIDENCE_RETENTION_SECS],
        )?;
        transaction.commit()?;
        Ok(removed)
    }

    pub fn outbound_terminal_detail(&self, message_id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT terminal_detail FROM outbound_message_inspection WHERE message_id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    pub fn outbound_attempts(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<Vec<OutboundAttemptRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT message_id, attempt_number, started_unix_ms, deadline_unix_ms, state FROM outbound_attempts WHERE message_id = ?1 ORDER BY attempt_number",
        )?;
        let records = statement
            .query_map(params![message_id], |row| {
                Ok(OutboundAttemptRecord {
                    message_id: row.get(0)?,
                    attempt_number: row.get(1)?,
                    started_unix_ms: row.get(2)?,
                    deadline_unix_ms: row.get(3)?,
                    state: row.get(4)?,
                })
            })?
            .collect();
        records
    }

    pub fn outbound_attempts_for_correlation(
        &self,
        correlation_id: &str,
    ) -> rusqlite::Result<Vec<OutboundAttemptRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT a.message_id, a.attempt_number, a.started_unix_ms, a.deadline_unix_ms, a.state
             FROM outbound_attempts a
             JOIN outbound_routes r ON r.message_id = a.message_id
             WHERE r.correlation_id = ?1
             ORDER BY a.attempt_number, a.started_unix_ms, a.message_id",
        )?;
        let records = statement
            .query_map(params![correlation_id], |row| {
                Ok(OutboundAttemptRecord {
                    message_id: row.get(0)?,
                    attempt_number: row.get(1)?,
                    started_unix_ms: row.get(2)?,
                    deadline_unix_ms: row.get(3)?,
                    state: row.get(4)?,
                })
            })?
            .collect();
        records
    }

    pub fn reconcile_outbound_startup(&self, now_unix_ms: i64) -> rusqlite::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM outbound_evidence
             WHERE evidence_id IN (
                 SELECT evidence_hash FROM message_delivery_evidence
                 WHERE terminal_at IS NOT NULL AND terminal_at <= ?1 - ?2
             )",
            params![now_unix_ms.max(0) / 1000, DELIVERY_EVIDENCE_RETENTION_SECS],
        )?;
        transaction.execute(
            "DELETE FROM message_delivery_evidence
             WHERE terminal_at IS NOT NULL AND terminal_at <= ?1 - ?2",
            params![now_unix_ms.max(0) / 1000, DELIVERY_EVIDENCE_RETENTION_SECS],
        )?;
        transaction.execute(
            "UPDATE outbound_attempts SET state = 'expired' WHERE message_id IN (
                SELECT message_id FROM outbound_routes
                WHERE deadline_unix_ms <= ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')
            )",
            params![now_unix_ms],
        )?;
        transaction.execute(
            "UPDATE messages SET receipt_status = 'expired' WHERE id IN (
                SELECT message_id FROM outbound_routes
                WHERE deadline_unix_ms <= ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')
            )",
            params![now_unix_ms],
        )?;
        transaction.execute(
            "UPDATE outbound_routes SET state = 'expired' WHERE deadline_unix_ms <= ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')",
            params![now_unix_ms],
        )?;
        transaction.execute(
            "UPDATE outbound_attempts SET state = 'interrupted' WHERE message_id IN (
                SELECT message_id FROM outbound_routes WHERE state = 'sending' AND deadline_unix_ms > ?1
            ) AND state = 'sending'",
            params![now_unix_ms],
        )?;
        transaction.execute(
            "UPDATE messages SET receipt_status = 'queued: recovered' WHERE id IN (
                SELECT message_id FROM outbound_routes WHERE state = 'sending' AND deadline_unix_ms > ?1
            )",
            params![now_unix_ms],
        )?;
        transaction.execute(
            "UPDATE outbound_routes SET state = 'queued' WHERE state = 'sending' AND deadline_unix_ms > ?1",
            params![now_unix_ms],
        )?;
        transaction.commit()
    }

    pub fn expire_outbound_routes(&self, now_unix_ms: i64) -> rusqlite::Result<Vec<String>> {
        let transaction = self.conn.unchecked_transaction()?;
        let mut statement = transaction.prepare(
            "SELECT message_id FROM outbound_routes WHERE deadline_unix_ms <= ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected') ORDER BY message_id",
        )?;
        let ids: Vec<String> = statement
            .query_map(params![now_unix_ms], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);
        for message_id in &ids {
            transaction.execute(
                "UPDATE outbound_routes SET state = 'expired' WHERE message_id = ?1",
                params![message_id],
            )?;
            transaction.execute(
                "UPDATE outbound_attempts SET state = 'expired' WHERE message_id = ?1 AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired')",
                params![message_id],
            )?;
            transaction.execute(
                "UPDATE messages SET receipt_status = 'expired' WHERE id = ?1",
                params![message_id],
            )?;
        }
        transaction.commit()?;
        Ok(ids)
    }

    pub fn list_messages(
        &self,
        limit: usize,
        before_ts: Option<i64>,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        let limit = validated_message_limit(limit)?;
        let mut records = Vec::new();
        if let Some(ts) = before_ts {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE timestamp < ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![ts, limit])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages ORDER BY timestamp DESC, id DESC LIMIT ?1",
            )?;
            let mut rows = stmt.query(params![limit])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        }
        Ok(records)
    }

    pub fn get_message(&self, message_id: &str) -> rusqlite::Result<Option<MessageRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE id = ?1 LIMIT 1",
        )?;
        stmt.query_row(params![message_id], parse_message_row).optional()
    }

    /// List messages filtered by peer hash (source or destination matches).
    pub fn list_messages_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        let limit = validated_message_limit(limit)?;
        let mut records = Vec::new();
        if let Some(ts) = before_ts {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE (lower(source) = ?1 OR lower(destination) = ?1) AND timestamp < ?2 ORDER BY timestamp DESC, id DESC LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![peer_hash, ts, limit])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE lower(source) = ?1 OR lower(destination) = ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![peer_hash, limit])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        }
        Ok(records)
    }

    pub fn message_projection_snapshot_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> rusqlite::Result<Vec<MessageProjectionSnapshot>> {
        let limit = validated_message_limit(limit)?;
        let transaction = self.conn.unchecked_transaction()?;
        let mut messages = Vec::new();
        let mut projection_bytes = 0usize;
        if let Some(timestamp) = before_ts {
            let mut statement = transaction.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction,
                        fields, receipt_status, COALESCE(read, 0)
                 FROM messages WHERE (lower(source) = ?1 OR lower(destination) = ?1) AND timestamp < ?2
                 ORDER BY timestamp DESC, id DESC LIMIT ?3",
            )?;
            let mut rows = statement.query(params![peer_hash, timestamp, limit])?;
            while let Some(row) = rows.next()? {
                let message = parse_message_row(row)?;
                charge_message_projection(&mut projection_bytes, &message)?;
                messages.push(message);
            }
        } else {
            let mut statement = transaction.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction,
                        fields, receipt_status, COALESCE(read, 0)
                 FROM messages WHERE lower(source) = ?1 OR lower(destination) = ?1
                 ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let mut rows = statement.query(params![peer_hash, limit])?;
            while let Some(row) = rows.next()? {
                let message = parse_message_row(row)?;
                charge_message_projection(&mut projection_bytes, &message)?;
                messages.push(message);
            }
        }
        let snapshots = load_projection_snapshots(&transaction, messages, projection_bytes)?;
        transaction.commit()?;
        Ok(snapshots)
    }

    pub fn message_projection_page_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<MessageProjectionPage, PageError> {
        let limit = validated_page_limit(limit)?;
        let peer = canonical_peer_bytes(peer_hash)?;
        let transaction = self.conn.unchecked_transaction()?;
        let metadata = page_metadata(&transaction)?;
        let decoded = cursor
            .map(|cursor| crate::cursor::MessageCursor::decode(cursor, &metadata.cursor_secret))
            .transpose()
            .map_err(|error| PageError::InvalidCursor(error.to_string()))?;
        if let Some(cursor) = decoded.as_ref() {
            if cursor.store_id != metadata.store_id || cursor.peer != peer {
                return Err(PageError::InvalidCursor("cursor scope does not match query".into()));
            }
        }
        let snapshot_seq = match decoded.as_ref() {
            Some(cursor) => cursor.snapshot_seq,
            None => transaction.query_row(
                "SELECT COALESCE(MAX(ingest_seq), 0) FROM message_page_keys",
                [],
                |row| row.get(0),
            )?,
        };
        let fetch_limit = limit + 1;
        let mut messages = Vec::with_capacity(fetch_limit);
        let mut boundaries = Vec::with_capacity(fetch_limit);
        let mut projection_bytes = 0usize;
        let sql = if decoded.is_some() {
            "SELECT m.id, m.source, m.destination, m.title, m.content, m.timestamp,
                    m.direction, m.fields, m.receipt_status, COALESCE(m.read, 0),
                    k.sort_timestamp, k.ingest_seq
             FROM message_page_keys k JOIN messages m ON m.id = k.message_id
             WHERE k.conversation_peer = ?1 AND k.ingest_seq <= ?2
               AND (k.sort_timestamp < ?3 OR
                    (k.sort_timestamp = ?3 AND k.ingest_seq < ?4))
             ORDER BY k.sort_timestamp DESC, k.ingest_seq DESC LIMIT ?5"
        } else {
            "SELECT m.id, m.source, m.destination, m.title, m.content, m.timestamp,
                    m.direction, m.fields, m.receipt_status, COALESCE(m.read, 0),
                    k.sort_timestamp, k.ingest_seq
             FROM message_page_keys k JOIN messages m ON m.id = k.message_id
             WHERE k.conversation_peer = ?1 AND k.ingest_seq <= ?2
             ORDER BY k.sort_timestamp DESC, k.ingest_seq DESC LIMIT ?5"
        };
        let mut statement = transaction.prepare(sql)?;
        let boundary = decoded.as_ref().map(|cursor| (cursor.sort_timestamp, cursor.ingest_seq));
        let mut rows = statement.query(params![
            peer_hash,
            snapshot_seq,
            boundary.map(|value| value.0),
            boundary.map(|value| value.1),
            i64::try_from(fetch_limit)
                .map_err(|_| PageError::InvalidCursor("limit overflow".into()))?,
        ])?;
        while let Some(row) = rows.next()? {
            let message = parse_message_row(row)?;
            charge_message_projection(&mut projection_bytes, &message)?;
            boundaries.push((row.get::<_, i64>(10)?, row.get::<_, i64>(11)?));
            messages.push(message);
        }
        drop(rows);
        drop(statement);
        let has_more = messages.len() > limit;
        if has_more {
            messages.pop();
            boundaries.pop();
        }
        let next_cursor = has_more.then(|| {
            let (sort_timestamp, ingest_seq) = boundaries[boundaries.len() - 1];
            crate::cursor::MessageCursor {
                store_id: metadata.store_id,
                snapshot_seq,
                peer,
                sort_timestamp,
                ingest_seq,
            }
            .encode(&metadata.cursor_secret)
        });
        let items = load_projection_snapshots(&transaction, messages, projection_bytes)?;
        transaction.commit()?;
        Ok(MessageProjectionPage { items, next_cursor })
    }

    pub fn mark_read_outcome(
        &self,
        peer_hash: &str,
    ) -> rusqlite::Result<ConversationMutationOutcome> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if !conversation_exists(&transaction, peer_hash)? {
            return Ok(ConversationMutationOutcome {
                disposition: MutationDisposition::NotFound,
                affected_count: 0,
                summary: None,
                terminal_state: None,
            });
        }
        let count = transaction.execute(
            "UPDATE messages SET read = 1
             WHERE direction = 'in' AND lower(source) = ?1 AND COALESCE(read, 0) = 0",
            params![peer_hash],
        )?;
        let summary = conversation_summary(&transaction, peer_hash)?;
        transaction.commit()?;
        Ok(ConversationMutationOutcome {
            disposition: if count == 0 {
                MutationDisposition::Unchanged
            } else {
                MutationDisposition::Applied
            },
            affected_count: count as u64,
            summary,
            terminal_state: None,
        })
    }

    /// Mark unread inbound messages from a peer as read. Returns the updated row count.
    pub fn mark_read(&self, peer_hash: &str) -> rusqlite::Result<u64> {
        Ok(self.mark_read_outcome(peer_hash)?.affected_count)
    }

    /// Delete all messages in a conversation with a peer. Returns count.
    pub fn delete_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> rusqlite::Result<ConversationMutationOutcome> {
        self.delete_conversation_outcome_with_ids(peer_hash).map(|(outcome, _)| outcome)
    }

    pub(crate) fn delete_conversation_outcome_with_ids(
        &self,
        peer_hash: &str,
    ) -> rusqlite::Result<(ConversationMutationOutcome, Vec<String>)> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let message_ids = {
            let mut statement = transaction.prepare(
                "SELECT id FROM messages
                 WHERE lower(source) = ?1 OR lower(destination) = ?1
                 ORDER BY id",
            )?;
            let ids = statement
                .query_map(params![peer_hash], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            ids
        };
        if message_ids.is_empty() {
            return Ok((
                ConversationMutationOutcome {
                    disposition: MutationDisposition::NotFound,
                    affected_count: 0,
                    summary: None,
                    terminal_state: None,
                },
                message_ids,
            ));
        }
        let active_state: Option<String> = transaction
            .query_row(
                "SELECT state FROM outbound_routes WHERE message_id IN (
                     SELECT id FROM messages WHERE lower(source) = ?1 OR lower(destination) = ?1
                  ) AND state NOT IN ('delivered', 'failed', 'cancelled', 'expired', 'rejected')
                    AND NOT (state = 'sent' AND actual_method = 'propagated')
                 ORDER BY message_id LIMIT 1",
                params![peer_hash],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(state) = active_state {
            return Ok((
                ConversationMutationOutcome {
                    disposition: MutationDisposition::TerminalConflict,
                    affected_count: 0,
                    summary: conversation_summary(&transaction, peer_hash)?,
                    terminal_state: Some(state),
                },
                Vec::new(),
            ));
        }
        detach_retry_children(&transaction, &message_ids)?;
        tombstone_standard_propagation_links(&transaction, &message_ids)?;
        let count = transaction.execute(
            "DELETE FROM messages WHERE lower(source) = ?1 OR lower(destination) = ?1",
            params![peer_hash],
        )?;
        gc_attachment_blobs(&transaction)?;
        transaction
            .execute("DELETE FROM conversation_state WHERE peer_hash = ?1", params![peer_hash])?;
        transaction
            .execute("DELETE FROM conversation_drafts WHERE peer_hash = ?1", params![peer_hash])?;
        transaction.commit()?;
        Ok((
            ConversationMutationOutcome {
                disposition: MutationDisposition::Applied,
                affected_count: count as u64,
                summary: None,
                terminal_state: None,
            },
            message_ids,
        ))
    }

    pub fn delete_conversation(&self, peer_hash: &str) -> rusqlite::Result<u64> {
        Ok(self.delete_conversation_outcome(peer_hash)?.affected_count)
    }

    /// Delete a single message by ID. Returns true if deleted.
    pub fn delete_message_outcome(
        &self,
        message_id: &str,
    ) -> rusqlite::Result<MessageMutationOutcome> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let state: Option<(Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT r.state, r.actual_method
                 FROM messages m LEFT JOIN outbound_routes r ON r.message_id = m.id
                 WHERE m.id = ?1",
                params![message_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((state, actual_method)) = state else {
            return Ok(MessageMutationOutcome {
                disposition: MutationDisposition::NotFound,
                affected_count: 0,
                terminal_state: None,
            });
        };
        if state.as_deref().is_some_and(|state| {
            !matches!(state, "delivered" | "failed" | "cancelled" | "expired" | "rejected")
                && !(state == "sent" && actual_method.as_deref() == Some("propagated"))
        }) {
            return Ok(MessageMutationOutcome {
                disposition: MutationDisposition::TerminalConflict,
                affected_count: 0,
                terminal_state: state,
            });
        }
        detach_retry_children(&transaction, &[message_id.to_string()])?;
        tombstone_standard_propagation_links(&transaction, &[message_id.to_string()])?;
        let count =
            transaction.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        gc_attachment_blobs(&transaction)?;
        transaction.commit()?;
        Ok(MessageMutationOutcome {
            disposition: MutationDisposition::Applied,
            affected_count: count as u64,
            terminal_state: None,
        })
    }

    pub fn delete_message(&self, message_id: &str) -> rusqlite::Result<bool> {
        Ok(self.delete_message_outcome(message_id)?.disposition == MutationDisposition::Applied)
    }

    /// Search messages by content substring, optionally scoped to a peer.
    pub fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        Ok(self
            .search_message_projection_outcome(query, peer_hash, limit)?
            .items
            .into_iter()
            .map(|snapshot| snapshot.message)
            .collect())
    }

    pub fn search_message_projection_snapshot(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<Vec<MessageProjectionSnapshot>> {
        Ok(self.search_message_projection_outcome(query, peer_hash, limit)?.items)
    }

    pub fn search_message_projection_outcome(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<MessageSearchSnapshot> {
        if !(1..=MAX_MESSAGE_QUERY_LIMIT).contains(&limit) {
            return Err(rusqlite::Error::InvalidParameterName(
                "search limit must be between 1 and 256".into(),
            ));
        }
        let requested_limit = limit;
        let limit = validated_message_limit(limit)?;
        if query.is_empty() || query.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "search query must be 1..={MAX_SEARCH_QUERY_BYTES} UTF-8 bytes"
            )));
        }
        let transaction = self.conn.unchecked_transaction()?;
        let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let matched_count: i64 = if let Some(peer) = peer_hash {
            transaction.query_row(
                "SELECT COUNT(*) FROM messages WHERE
                 (lower(source) = ?1 OR lower(destination) = ?1)
                 AND content LIKE ?2 ESCAPE '\\'",
                params![peer, pattern],
                |row| row.get(0),
            )?
        } else {
            transaction.query_row(
                "SELECT COUNT(*) FROM messages WHERE content LIKE ?1 ESCAPE '\\'",
                params![pattern],
                |row| row.get(0),
            )?
        };
        let fetch_limit = limit + 1;
        let mut messages = Vec::new();
        let mut projection_bytes = 0usize;
        if let Some(peer) = peer_hash {
            let mut statement = transaction.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction,
                        fields, receipt_status, COALESCE(read, 0)
                 FROM messages
                   WHERE (lower(source) = ?1 OR lower(destination) = ?1)
                     AND content LIKE ?2 ESCAPE '\\'
                   ORDER BY timestamp DESC, id DESC LIMIT ?3",
            )?;
            let mut rows = statement.query(params![peer, pattern, fetch_limit])?;
            while let Some(row) = rows.next()? {
                let message = parse_message_row(row)?;
                charge_message_projection(&mut projection_bytes, &message)?;
                messages.push(message);
            }
        } else {
            let mut statement = transaction.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction,
                        fields, receipt_status, COALESCE(read, 0)
                 FROM messages WHERE content LIKE ?1 ESCAPE '\\'
                   ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let mut rows = statement.query(params![pattern, fetch_limit])?;
            while let Some(row) = rows.next()? {
                let message = parse_message_row(row)?;
                charge_message_projection(&mut projection_bytes, &message)?;
                messages.push(message);
            }
        }
        let truncated = messages.len() > requested_limit;
        if truncated {
            messages.pop();
        }
        let snapshots = load_projection_snapshots(&transaction, messages, projection_bytes)?;
        transaction.commit()?;
        Ok(MessageSearchSnapshot {
            items: snapshots,
            truncated,
            matched_count: matched_count.max(0) as u64,
        })
    }

    /// List conversation summaries grouped by peer.
    pub fn list_conversations(
        &self,
        unread_only: bool,
    ) -> rusqlite::Result<Vec<ConversationSummary>> {
        let base = "SELECT g.peer, g.last_ts,
                    (SELECT m2.content FROM messages m2
                     WHERE (CASE WHEN m2.direction = 'out' THEN
                                CASE WHEN length(m2.destination) = 32
                                          AND m2.destination NOT GLOB '*[^0-9A-Fa-f]*'
                                     THEN lower(m2.destination) ELSE m2.destination END
                            ELSE CASE WHEN length(m2.source) = 32
                                          AND m2.source NOT GLOB '*[^0-9A-Fa-f]*'
                                     THEN lower(m2.source) ELSE m2.source END END) = g.peer
                     ORDER BY m2.timestamp DESC, m2.id DESC LIMIT 1) AS last_content,
                    g.unread, g.total, COALESCE(s.pinned, 0), COALESCE(s.muted, 0)
                    FROM (
                        SELECT CASE WHEN direction = 'out' THEN
                                    CASE WHEN length(destination) = 32
                                              AND destination NOT GLOB '*[^0-9A-Fa-f]*'
                                         THEN lower(destination) ELSE destination END
                                    ELSE CASE WHEN length(source) = 32
                                                   AND source NOT GLOB '*[^0-9A-Fa-f]*'
                                              THEN lower(source) ELSE source END END AS peer,
                               MAX(timestamp) AS last_ts,
                               SUM(CASE WHEN direction = 'in' AND COALESCE(read, 0) = 0 THEN 1 ELSE 0 END) AS unread,
                               COUNT(*) AS total
                        FROM messages GROUP BY peer
                    ) g
                    LEFT JOIN conversation_state s ON s.peer_hash = g.peer";
        let sql = if unread_only {
            format!(
                "{base} WHERE g.unread > 0
                 ORDER BY COALESCE(s.pinned, 0) DESC, g.last_ts DESC, g.peer ASC"
            )
        } else {
            format!("{base} ORDER BY COALESCE(s.pinned, 0) DESC, g.last_ts DESC, g.peer ASC")
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut summaries = Vec::new();
        while let Some(row) = rows.next()? {
            summaries.push(ConversationSummary {
                peer_hash: row.get(0)?,
                peer_name: None, // Resolved at service layer via announces
                last_message_timestamp: row.get(1)?,
                last_message_content: row.get(2)?,
                unread_count: row.get::<_, i64>(3)? as u32,
                message_count: row.get::<_, i64>(4)? as u32,
                pinned: row.get::<_, i64>(5)? != 0,
                muted: row.get::<_, i64>(6)? != 0,
            });
        }
        Ok(summaries)
    }

    pub fn conversation_page(
        &self,
        unread_only: bool,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<ConversationPage, PageError> {
        let limit = validated_page_limit(limit)?;
        let transaction = self.conn.unchecked_transaction()?;
        let metadata = page_metadata(&transaction)?;
        let decoded = cursor
            .map(|cursor| {
                crate::cursor::ConversationCursor::decode(cursor, &metadata.cursor_secret)
            })
            .transpose()
            .map_err(|error| PageError::InvalidCursor(error.to_string()))?;
        if let Some(cursor) = decoded.as_ref() {
            if cursor.store_id != metadata.store_id || cursor.unread_only != unread_only {
                return Err(PageError::InvalidCursor("cursor scope does not match query".into()));
            }
            if cursor.conversation_epoch != metadata.conversation_epoch {
                return Err(PageError::CursorStale);
            }
        }
        let snapshot_seq = match decoded.as_ref() {
            Some(cursor) => cursor.snapshot_seq,
            None => transaction.query_row(
                "SELECT COALESCE(MAX(ingest_seq), 0) FROM message_page_keys",
                [],
                |row| row.get(0),
            )?,
        };
        let fetch_limit = limit + 1;
        let boundary = decoded.as_ref().map(|cursor| {
            (
                i64::from(cursor.pinned),
                cursor.last_sort_timestamp,
                cursor.last_ingest_seq,
                hex::encode(cursor.peer),
            )
        });
        // Conversation summaries necessarily aggregate every retained key in the fixed
        // snapshot. The v8 covering indexes keep that bounded to key/message reads and
        // avoid OFFSET or repeated full message payload scans; the query-plan test pins it.
        let sql = "WITH eligible AS (
                SELECT k.message_id, k.ingest_seq, k.sort_timestamp, k.conversation_peer,
                       m.content, m.direction, COALESCE(m.read, 0) AS is_read
                FROM message_page_keys k JOIN messages m ON m.id = k.message_id
                WHERE k.ingest_seq <= ?1
                  AND length(k.conversation_peer) = 32
                  AND k.conversation_peer NOT GLOB '*[^0-9a-f]*'
            ), peers AS (
                SELECT conversation_peer AS peer,
                       SUM(CASE WHEN direction = 'in' AND is_read = 0 THEN 1 ELSE 0 END) AS unread,
                       COUNT(*) AS total
                FROM eligible GROUP BY conversation_peer
            ), summaries AS (
                SELECT p.peer,
                       (SELECT e.sort_timestamp FROM eligible e WHERE e.conversation_peer = p.peer
                        ORDER BY e.sort_timestamp DESC, e.ingest_seq DESC LIMIT 1) AS last_ts,
                       (SELECT e.ingest_seq FROM eligible e WHERE e.conversation_peer = p.peer
                        ORDER BY e.sort_timestamp DESC, e.ingest_seq DESC LIMIT 1) AS last_seq,
                       (SELECT e.content FROM eligible e WHERE e.conversation_peer = p.peer
                        ORDER BY e.sort_timestamp DESC, e.ingest_seq DESC LIMIT 1) AS last_content,
                       p.unread, p.total, COALESCE(s.pinned, 0) AS pinned,
                       COALESCE(s.muted, 0) AS muted
                FROM peers p LEFT JOIN conversation_state s ON s.peer_hash = p.peer
            )
            SELECT peer, last_ts, last_content, unread, total, pinned, muted, last_seq
            FROM summaries
            WHERE (?2 = 0 OR unread > 0)
              AND (?3 IS NULL OR pinned < ?3
                   OR (pinned = ?3 AND last_ts < ?4)
                   OR (pinned = ?3 AND last_ts = ?4 AND last_seq < ?5)
                   OR (pinned = ?3 AND last_ts = ?4 AND last_seq = ?5 AND peer > ?6))
            ORDER BY pinned DESC, last_ts DESC, last_seq DESC, peer ASC LIMIT ?7";
        let mut statement = transaction.prepare(sql)?;
        let mut rows = statement.query(params![
            snapshot_seq,
            i64::from(unread_only),
            boundary.as_ref().map(|value| value.0),
            boundary.as_ref().map(|value| value.1),
            boundary.as_ref().map(|value| value.2),
            boundary.as_ref().map(|value| value.3.as_str()),
            i64::try_from(fetch_limit)
                .map_err(|_| PageError::InvalidCursor("limit overflow".into()))?,
        ])?;
        let mut items = Vec::with_capacity(fetch_limit);
        let mut keys = Vec::with_capacity(fetch_limit);
        while let Some(row) = rows.next()? {
            let peer_hash: String = row.get(0)?;
            let last_sort_timestamp: i64 = row.get(1)?;
            let pinned = row.get::<_, i64>(5)? != 0;
            keys.push((pinned, last_sort_timestamp, row.get::<_, i64>(7)?, peer_hash.clone()));
            items.push(ConversationSummary {
                peer_hash,
                peer_name: None,
                last_message_timestamp: Some(last_sort_timestamp),
                last_message_content: row.get(2)?,
                unread_count: row.get::<_, i64>(3)? as u32,
                message_count: row.get::<_, i64>(4)? as u32,
                pinned,
                muted: row.get::<_, i64>(6)? != 0,
            });
        }
        drop(rows);
        drop(statement);
        let has_more = items.len() > limit;
        if has_more {
            items.pop();
            keys.pop();
        }
        let next_cursor = has_more
            .then(|| {
                let (pinned, last_sort_timestamp, last_ingest_seq, peer_hash) =
                    &keys[keys.len() - 1];
                let peer = canonical_peer_bytes(peer_hash)?;
                Ok::<_, PageError>(
                    crate::cursor::ConversationCursor {
                        store_id: metadata.store_id,
                        snapshot_seq,
                        conversation_epoch: metadata.conversation_epoch,
                        unread_only,
                        pinned: *pinned,
                        last_sort_timestamp: *last_sort_timestamp,
                        last_ingest_seq: *last_ingest_seq,
                        peer,
                    }
                    .encode(&metadata.cursor_secret),
                )
            })
            .transpose()?;
        transaction.commit()?;
        Ok(ConversationPage { items, next_cursor })
    }

    pub fn set_conversation_pinned(&self, peer_hash: &str, pinned: bool) -> rusqlite::Result<bool> {
        Ok(self.set_conversation_flag_outcome(peer_hash, "pinned", pinned)?.disposition
            == MutationDisposition::Applied)
    }

    pub fn set_conversation_muted(&self, peer_hash: &str, muted: bool) -> rusqlite::Result<bool> {
        Ok(self.set_conversation_flag_outcome(peer_hash, "muted", muted)?.disposition
            == MutationDisposition::Applied)
    }

    pub fn set_conversation_flag_outcome(
        &self,
        peer_hash: &str,
        column: &str,
        value: bool,
    ) -> rusqlite::Result<ConversationMutationOutcome> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if !conversation_exists(&transaction, peer_hash)? {
            return Ok(ConversationMutationOutcome {
                disposition: MutationDisposition::NotFound,
                affected_count: 0,
                summary: None,
                terminal_state: None,
            });
        }
        let (pinned, muted): (bool, bool) = transaction
            .query_row(
                "SELECT pinned, muted FROM conversation_state WHERE peer_hash = ?1",
                params![peer_hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((false, false));
        let current = if column == "pinned" { pinned } else { muted };
        if current == value {
            return Ok(ConversationMutationOutcome {
                disposition: MutationDisposition::Unchanged,
                affected_count: 0,
                summary: conversation_summary(&transaction, peer_hash)?,
                terminal_state: None,
            });
        }
        let now = unix_now();
        let sql = match column {
            "pinned" => {
                "INSERT INTO conversation_state (peer_hash, pinned, muted, updated_at)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT(peer_hash) DO UPDATE SET pinned = excluded.pinned,
                     updated_at = excluded.updated_at"
            }
            "muted" => {
                "INSERT INTO conversation_state (peer_hash, pinned, muted, updated_at)
                 VALUES (?1, 0, ?2, ?3)
                 ON CONFLICT(peer_hash) DO UPDATE SET muted = excluded.muted,
                     updated_at = excluded.updated_at"
            }
            _ => return Err(rusqlite::Error::InvalidParameterName(column.to_string())),
        };
        transaction.execute(sql, params![peer_hash, value, now])?;
        let summary = conversation_summary(&transaction, peer_hash)?;
        transaction.commit()?;
        Ok(ConversationMutationOutcome {
            disposition: MutationDisposition::Applied,
            affected_count: 1,
            summary,
            terminal_state: None,
        })
    }

    pub fn set_draft(&self, peer_hash: &str, content: &str) -> rusqlite::Result<ConversationDraft> {
        let content_bytes = content.len();
        if content_bytes > MAX_DRAFT_BYTES {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "draft exceeds {MAX_DRAFT_BYTES} UTF-8 bytes"
            )));
        }

        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let (other_count, other_bytes): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(CAST(content AS BLOB))), 0)
             FROM conversation_drafts WHERE peer_hash <> ?1",
            params![peer_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if other_count >= MAX_RETAINED_DRAFTS as i64 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "draft count exceeds {MAX_RETAINED_DRAFTS}"
            )));
        }
        let aggregate =
            usize::try_from(other_bytes).unwrap_or(usize::MAX).saturating_add(content_bytes);
        if aggregate > MAX_DRAFT_AGGREGATE_BYTES {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "draft storage exceeds {MAX_DRAFT_AGGREGATE_BYTES} UTF-8 bytes"
            )));
        }
        let updated_at = unix_now();
        transaction.execute(
            "INSERT INTO conversation_drafts (peer_hash, content, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(peer_hash) DO UPDATE SET content = excluded.content,
                 updated_at = excluded.updated_at",
            params![peer_hash, content, updated_at],
        )?;
        transaction.commit()?;
        Ok(ConversationDraft {
            peer_hash: peer_hash.to_string(),
            content: content.to_string(),
            updated_at,
        })
    }

    pub fn draft(&self, peer_hash: &str) -> rusqlite::Result<Option<ConversationDraft>> {
        self.conn
            .query_row(
                "SELECT peer_hash, content, updated_at FROM conversation_drafts WHERE peer_hash = ?1",
                params![peer_hash],
                parse_draft_row,
            )
            .optional()
    }

    pub fn clear_draft(&self, peer_hash: &str) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM conversation_drafts WHERE peer_hash = ?1", params![peer_hash])?
            > 0)
    }

    // ── Contacts ────────────────────────────────────────────────────────

    /// Upsert a contact record. Returns the saved record.
    pub fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> rusqlite::Result<ContactRecord> {
        self.set_contact_outcome(peer_hash, alias, notes)?
            .contact
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    pub fn set_contact_outcome(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> rusqlite::Result<ContactMutationOutcome> {
        if alias.is_some_and(|value| value.len() > MAX_CONTACT_ALIAS_BYTES) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "contact alias exceeds {MAX_CONTACT_ALIAS_BYTES} UTF-8 bytes"
            )));
        }
        if notes.is_some_and(|value| value.len() > MAX_CONTACT_NOTES_BYTES) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "contact notes exceed {MAX_CONTACT_NOTES_BYTES} UTF-8 bytes"
            )));
        }
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT peer_hash, alias, notes, created_at, updated_at FROM contacts
                 WHERE peer_hash = ?1",
                params![peer_hash],
                |row| {
                    Ok(ContactRecord {
                        peer_hash: row.get(0)?,
                        alias: row.get(1)?,
                        notes: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        if existing.as_ref().is_some_and(|contact| {
            contact.alias.as_deref() == alias && contact.notes.as_deref() == notes
        }) {
            return Ok(ContactMutationOutcome {
                disposition: MutationDisposition::Unchanged,
                affected_count: 0,
                contact: existing,
            });
        }
        let now = unix_now();
        transaction.execute(
            "INSERT INTO contacts (peer_hash, alias, notes, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(peer_hash) DO UPDATE SET alias = ?2, notes = ?3,
                 updated_at = MAX(?4, contacts.updated_at + 1)",
            params![peer_hash, alias, notes, now],
        )?;
        let contact = transaction.query_row(
            "SELECT peer_hash, alias, notes, created_at, updated_at
             FROM contacts WHERE peer_hash = ?1",
            params![peer_hash],
            |row| {
                Ok(ContactRecord {
                    peer_hash: row.get(0)?,
                    alias: row.get(1)?,
                    notes: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )?;
        transaction.commit()?;
        Ok(ContactMutationOutcome {
            disposition: if existing.is_some() {
                MutationDisposition::Updated
            } else {
                MutationDisposition::Created
            },
            affected_count: 1,
            contact: Some(contact),
        })
    }

    /// Remove a contact by peer hash. Returns true if deleted.
    pub fn remove_contact(&self, peer_hash: &str) -> rusqlite::Result<bool> {
        Ok(self.remove_contact_outcome(peer_hash)?.disposition == MutationDisposition::Applied)
    }

    pub fn remove_contact_outcome(
        &self,
        peer_hash: &str,
    ) -> rusqlite::Result<ContactMutationOutcome> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let count =
            transaction.execute("DELETE FROM contacts WHERE peer_hash = ?1", params![peer_hash])?;
        transaction.commit()?;
        Ok(ContactMutationOutcome {
            disposition: if count == 0 {
                MutationDisposition::NotFound
            } else {
                MutationDisposition::Applied
            },
            affected_count: count as u64,
            contact: None,
        })
    }

    /// List all contacts.
    pub fn list_contacts(&self) -> rusqlite::Result<Vec<ContactRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT CAST(peer_hash AS BLOB), typeof(peer_hash),
                    CAST(alias AS BLOB), typeof(alias),
                    CAST(notes AS BLOB), typeof(notes),
                    CASE WHEN typeof(created_at) = 'integer' AND created_at >= 0
                         THEN created_at ELSE 0 END,
                    CASE WHEN typeof(updated_at) = 'integer' AND updated_at >= 0
                         THEN updated_at ELSE 0 END
             FROM contacts
             ORDER BY CASE WHEN typeof(updated_at) = 'integer' AND updated_at >= 0
                           THEN updated_at ELSE 0 END DESC,
                      hex(CAST(peer_hash AS BLOB)) ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut contacts = Vec::new();
        while let Some(row) = rows.next()? {
            let peer_bytes: Vec<u8> = row.get(0)?;
            let peer_type: String = row.get(1)?;
            let peer_hash = if peer_type == "text" {
                std::str::from_utf8(&peer_bytes)
                    .map(str::to_owned)
                    .unwrap_or_else(|_| format!("legacy-invalid-utf8:{}", hex::encode(&peer_bytes)))
            } else {
                format!("legacy-{peer_type}:{}", hex::encode(&peer_bytes))
            };
            let safe_text = |bytes: Option<Vec<u8>>, value_type: String| {
                if value_type == "text" {
                    bytes.and_then(|bytes| String::from_utf8(bytes).ok())
                } else {
                    None
                }
            };
            contacts.push(ContactRecord {
                peer_hash,
                alias: safe_text(row.get(2)?, row.get(3)?),
                notes: safe_text(row.get(4)?, row.get(5)?),
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            });
        }
        Ok(contacts)
    }

    pub fn count_message_buckets(&self) -> rusqlite::Result<(u64, u64)> {
        let mut stmt = self.conn.prepare(
            "SELECT
                COALESCE(SUM(CASE
                    WHEN receipt_status IS NULL OR TRIM(receipt_status) = '' THEN 1
                    ELSE 0
                END), 0) AS queued_count,
                COALESCE(SUM(CASE
                    WHEN receipt_status IS NOT NULL
                        AND TRIM(receipt_status) <> ''
                        AND LOWER(receipt_status) NOT LIKE 'sent%'
                        AND LOWER(receipt_status) NOT IN ('cancelled', 'delivered', 'failed', 'expired', 'rejected')
                    THEN 1
                    ELSE 0
                END), 0) AS in_flight_count
             FROM messages",
        )?;
        let (queued, in_flight): (i64, i64) =
            stmt.query_row([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok((queued.max(0) as u64, in_flight.max(0) as u64))
    }

    pub fn count_outbound_messages(&self) -> rusqlite::Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE direction = 'out'",
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    pub fn expire_outbound_messages_before(&self, cutoff_ts: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id
             FROM messages
             WHERE direction = 'out'
               AND timestamp < ?1
               AND (
                    receipt_status IS NULL
                    OR TRIM(receipt_status) = ''
                    OR (
                        LOWER(receipt_status) NOT LIKE 'sent%'
                        AND LOWER(receipt_status) NOT IN ('cancelled', 'delivered', 'failed', 'expired', 'rejected')
                    )
               )
             ORDER BY timestamp ASC, id ASC",
        )?;
        let mut rows = stmt.query(params![cutoff_ts])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            ids.push(row.get::<_, String>(0)?);
        }
        for message_id in ids.iter() {
            self.conn.execute(
                "UPDATE messages SET receipt_status = 'expired' WHERE id = ?1",
                params![message_id],
            )?;
        }
        Ok(ids)
    }

    pub fn prune_outbound_messages(
        &self,
        count: usize,
        eviction_priority: &str,
    ) -> rusqlite::Result<Vec<String>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let collect_ids = |query: &str, remaining: usize| -> rusqlite::Result<Vec<String>> {
            if remaining == 0 {
                return Ok(Vec::new());
            }
            let mut stmt = transaction.prepare(query)?;
            let mut rows = stmt.query(params![remaining as i64])?;
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(row.get::<_, String>(0)?);
            }
            Ok(ids)
        };

        let normalized = eviction_priority.trim().to_ascii_lowercase();
        let mut ids = if normalized == "terminal_first" {
            let mut selected = collect_ids(
                "SELECT id
                 FROM messages
                 WHERE direction = 'out'
                   AND receipt_status IS NOT NULL
                   AND TRIM(receipt_status) <> ''
                   AND (
                        LOWER(receipt_status) LIKE 'sent%'
                        OR LOWER(receipt_status) IN ('cancelled', 'delivered', 'failed', 'expired', 'rejected')
                   )
                 ORDER BY timestamp ASC, id ASC
                 LIMIT ?1",
                count,
            )?;
            let remaining = count.saturating_sub(selected.len());
            if remaining > 0 {
                let mut non_terminal = collect_ids(
                    "SELECT id
                     FROM messages
                     WHERE direction = 'out'
                       AND (
                            receipt_status IS NULL
                            OR TRIM(receipt_status) = ''
                            OR (
                                LOWER(receipt_status) NOT LIKE 'sent%'
                                AND LOWER(receipt_status) NOT IN ('cancelled', 'delivered', 'failed', 'expired', 'rejected')
                            )
                       )
                     ORDER BY timestamp ASC, id ASC
                     LIMIT ?1",
                    remaining,
                )?;
                selected.append(&mut non_terminal);
            }
            selected
        } else {
            collect_ids(
                "SELECT id
                 FROM messages
                 WHERE direction = 'out'
                 ORDER BY timestamp ASC, id ASC
                 LIMIT ?1",
                count,
            )?
        };

        ids.sort();
        ids.dedup();
        detach_retry_children(&transaction, &ids)?;
        tombstone_standard_propagation_links(&transaction, &ids)?;
        for message_id in ids.iter() {
            transaction.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        }
        gc_attachment_blobs(&transaction)?;
        transaction.commit()?;
        Ok(ids)
    }

    pub fn update_receipt_status(&self, message_id: &str, status: &str) -> rusqlite::Result<bool> {
        let count = self.conn.execute(
            "UPDATE messages SET receipt_status = ?1 WHERE id = ?2",
            params![status, message_id],
        )?;
        Ok(count > 0)
    }

    pub fn clear_messages(&self) -> rusqlite::Result<()> {
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM messages", [])?;
        gc_attachment_blobs(&transaction)?;
        transaction.commit()
    }

    pub fn insert_announce(&self, record: &AnnounceRecord) -> rusqlite::Result<()> {
        let capabilities_json = serde_json::to_string(&record.capabilities).unwrap_or_default();
        self.conn.execute(
            "INSERT OR REPLACE INTO announces (id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost, stamp_cost_flexibility, peering_cost) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                &record.id,
                &record.peer,
                record.timestamp,
                &record.name,
                &record.name_source,
                record.first_seen,
                record.seen_count as i64,
                &record.app_data_hex,
                capabilities_json,
                record.rssi,
                record.snr,
                record.q,
                record.stamp_cost,
                record.stamp_cost_flexibility,
                record.peering_cost,
            ],
        )?;
        if let Some(cost) = record.stamp_cost {
            self.learn_peer_stamp_cost(&record.peer, cost, record.timestamp)?;
        }
        Ok(())
    }

    pub fn list_announces(
        &self,
        limit: usize,
        before_ts: Option<i64>,
        before_id: Option<&str>,
    ) -> rusqlite::Result<Vec<AnnounceRecord>> {
        let mut records = Vec::new();
        let parse_row = |row: &rusqlite::Row| -> rusqlite::Result<AnnounceRecord> {
            let capabilities_json: Option<String> = row.get(8)?;
            let capabilities = capabilities_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
                .unwrap_or_default();
            let seen_count: i64 = row.get(6)?;
            Ok(AnnounceRecord {
                id: row.get(0)?,
                peer: row.get(1)?,
                timestamp: row.get(2)?,
                name: row.get(3)?,
                name_source: row.get(4)?,
                first_seen: row.get(5)?,
                seen_count: seen_count.max(0) as u64,
                app_data_hex: row.get(7)?,
                capabilities,
                rssi: row.get(9)?,
                snr: row.get(10)?,
                q: row.get(11)?,
                stamp_cost: row.get(12)?,
                stamp_cost_flexibility: row.get(13)?,
                peering_cost: row.get(14)?,
            })
        };
        if let Some(ts) = before_ts {
            let query_with_id = "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost, stamp_cost_flexibility, peering_cost FROM announces WHERE (timestamp < ?1 OR (timestamp = ?1 AND id < ?2)) ORDER BY timestamp DESC, id DESC LIMIT ?3";
            let query_without_id = "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost, stamp_cost_flexibility, peering_cost FROM announces WHERE timestamp < ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2";
            if let Some(ann_id) = before_id {
                let mut stmt = self.conn.prepare(query_with_id)?;
                let mut rows = stmt.query(params![ts, ann_id, limit as i64])?;
                while let Some(row) = rows.next()? {
                    records.push(parse_row(row)?);
                }
            } else {
                let mut stmt = self.conn.prepare(query_without_id)?;
                let mut rows = stmt.query(params![ts, limit as i64])?;
                while let Some(row) = rows.next()? {
                    records.push(parse_row(row)?);
                }
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost, stamp_cost_flexibility, peering_cost FROM announces ORDER BY timestamp DESC LIMIT ?1",
            )?;
            let mut rows = stmt.query(params![limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_row(row)?);
            }
        }
        Ok(records)
    }

    pub fn clear_announces(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM announces", [])?;
        Ok(())
    }

    pub fn put_sdk_domain_snapshot(&self, snapshot: &JsonValue) -> rusqlite::Result<()> {
        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        self.conn.execute(
            "INSERT INTO sdk_domain_state (domain, state_json) VALUES (?1, ?2)
             ON CONFLICT(domain) DO UPDATE SET state_json = excluded.state_json",
            params![Self::SDK_DOMAIN_SNAPSHOT_KEY, snapshot_json],
        )?;
        Ok(())
    }

    pub fn get_sdk_domain_snapshot(&self) -> rusqlite::Result<Option<JsonValue>> {
        let snapshot_json: Option<String> = self
            .conn
            .query_row(
                "SELECT state_json FROM sdk_domain_state WHERE domain = ?1 LIMIT 1",
                params![Self::SDK_DOMAIN_SNAPSHOT_KEY],
                |row| row.get(0),
            )
            .optional()?;
        let Some(snapshot_json) = snapshot_json else {
            return Ok(None);
        };
        let parsed = serde_json::from_str(snapshot_json.as_str()).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })?;
        Ok(Some(parsed))
    }

    pub fn clear_sdk_domain_snapshot(&self) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM sdk_domain_state WHERE domain = ?1",
            params![Self::SDK_DOMAIN_SNAPSHOT_KEY],
        )?;
        Ok(())
    }

    fn init_schema(&mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                destination TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                direction TEXT NOT NULL,
                fields TEXT,
                receipt_status TEXT
            );
            CREATE TABLE IF NOT EXISTS announces (
                id TEXT PRIMARY KEY,
                peer TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                name TEXT,
                name_source TEXT,
                first_seen INTEGER NOT NULL,
                seen_count INTEGER NOT NULL,
                app_data_hex TEXT,
                capabilities TEXT,
                rssi REAL,
                snr REAL,
                q REAL,
                stamp_cost INTEGER,
                stamp_cost_flexibility INTEGER,
                peering_cost INTEGER
            );
            CREATE TABLE IF NOT EXISTS sdk_domain_state (
                domain TEXT PRIMARY KEY,
                state_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS propagation_store (
                id TEXT PRIMARY KEY,
                dest_hash TEXT NOT NULL,
                lxmf_bytes BLOB NOT NULL,
                source_hash TEXT,
                received_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_prop_dest ON propagation_store(dest_hash);
            CREATE INDEX IF NOT EXISTS idx_prop_expires ON propagation_store(expires_at);",
        )?;
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN title TEXT", []);
        let _ = self.conn.execute("UPDATE messages SET title = '' WHERE title IS NULL", []);
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN fields TEXT", []);
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN receipt_status TEXT", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN name TEXT", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN name_source TEXT", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN first_seen INTEGER", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN seen_count INTEGER", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN app_data_hex TEXT", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN capabilities TEXT", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN rssi REAL", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN snr REAL", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN q REAL", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN stamp_cost INTEGER", []);
        let _ = self
            .conn
            .execute("ALTER TABLE announces ADD COLUMN stamp_cost_flexibility INTEGER", []);
        let _ = self.conn.execute("ALTER TABLE announces ADD COLUMN peering_cost INTEGER", []);
        // v2 migrations: read column + contacts table
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN read INTEGER DEFAULT 0", []);
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                peer_hash TEXT PRIMARY KEY,
                alias TEXT,
                notes TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS blocked_peers (
                identity_hash TEXT PRIMARY KEY,
                blocked_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS rbac_roster (
                identity_hash TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                grants TEXT NOT NULL DEFAULT ''
            );",
        )?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                id TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )?;
        let router_migration = "2026-08-22-authoritative-lxmf-router-v1";
        let migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![router_migration],
            |row| row.get(0),
        )?;
        if !migration_applied {
            self.conn.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS outbound_routes (
                    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                    requested_method TEXT NOT NULL,
                    actual_method TEXT NOT NULL,
                    representation TEXT NOT NULL,
                    fallback_reason TEXT,
                    correlation_id TEXT NOT NULL,
                    deadline_unix_ms INTEGER NOT NULL,
                    state TEXT NOT NULL,
                    attempt_count INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS outbound_attempts (
                    message_id TEXT NOT NULL REFERENCES outbound_routes(message_id) ON DELETE CASCADE,
                    attempt_number INTEGER NOT NULL,
                    started_unix_ms INTEGER NOT NULL,
                    deadline_unix_ms INTEGER NOT NULL,
                    state TEXT NOT NULL,
                    PRIMARY KEY (message_id, attempt_number)
                );
                CREATE INDEX IF NOT EXISTS idx_outbound_routes_correlation ON outbound_routes(correlation_id);
                INSERT INTO schema_migrations (id, applied_at) VALUES ('2026-08-22-authoritative-lxmf-router-v1', CAST(strftime('%s','now') AS INTEGER));
                COMMIT;",
            )?;
        }
        let receipt_migration = "2026-08-22-authoritative-lxmf-receipts-v2";
        let receipt_migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![receipt_migration],
            |row| row.get(0),
        )?;
        if !receipt_migration_applied {
            self.conn.execute_batch(
                "BEGIN IMMEDIATE;
                ALTER TABLE outbound_routes ADD COLUMN retry_of TEXT REFERENCES outbound_routes(message_id);
                CREATE UNIQUE INDEX IF NOT EXISTS idx_outbound_routes_retry_of
                    ON outbound_routes(retry_of) WHERE retry_of IS NOT NULL;
                CREATE TABLE IF NOT EXISTS outbound_evidence (
                    evidence_id TEXT PRIMARY KEY,
                    message_id TEXT NOT NULL REFERENCES outbound_routes(message_id) ON DELETE CASCADE,
                    kind TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_outbound_evidence_message
                    ON outbound_evidence(message_id);
                INSERT INTO schema_migrations (id, applied_at) VALUES ('2026-08-22-authoritative-lxmf-receipts-v2', CAST(strftime('%s','now') AS INTEGER));
                COMMIT;",
            )?;
        }
        let canonical_migration = "2026-08-22-canonical-lxmf-inbound-v3";
        let canonical_migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![canonical_migration],
            |row| row.get(0),
        )?;
        if !canonical_migration_applied {
            self.conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE canonical_inbound_messages (
                     message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                     source BLOB NOT NULL CHECK(length(source) = 16),
                     destination BLOB NOT NULL CHECK(length(destination) = 16),
                     title BLOB NOT NULL CHECK(length(title) <= 1048576),
                     content BLOB NOT NULL CHECK(length(content) <= 1048576),
                     timestamp REAL NOT NULL,
                     fields_msgpack BLOB CHECK(fields_msgpack IS NULL OR length(fields_msgpack) <= 1048576),
                     signature BLOB CHECK(signature IS NULL OR length(signature) = 64),
                     stamp BLOB CHECK(stamp IS NULL OR length(stamp) <= 32),
                     wire BLOB NOT NULL CHECK(length(wire) <= 4194304),
                     authentication_state TEXT NOT NULL CHECK(authentication_state IN ('verified','invalid','unknown_identity','not_applicable')),
                     stamp_state TEXT NOT NULL CHECK(stamp_state IN ('verified','invalid','unknown','not_applicable')),
                     stamp_value INTEGER
                 );
                 CREATE INDEX idx_canonical_inbound_unknown
                     ON canonical_inbound_messages(source, authentication_state);
                 CREATE TABLE lxmf_stamp_policy (
                     singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                     target_cost INTEGER NOT NULL CHECK(target_cost BETWEEN 0 AND 64),
                     flexibility INTEGER NOT NULL CHECK(flexibility BETWEEN 0 AND 64)
                 );
                 INSERT INTO lxmf_stamp_policy(singleton, target_cost, flexibility) VALUES (1, 0, 0);
                 CREATE TABLE lxmf_peer_costs (
                     peer TEXT PRIMARY KEY CHECK(length(peer) <= 128),
                     stamp_cost INTEGER NOT NULL CHECK(stamp_cost BETWEEN 0 AND 64),
                     observed_at INTEGER NOT NULL
                 );
                 CREATE TABLE lxmf_tickets (
                     peer TEXT NOT NULL CHECK(length(peer) <= 128),
                     ticket BLOB NOT NULL CHECK(length(ticket) = 16),
                     expires_at INTEGER NOT NULL,
                     direction TEXT NOT NULL CHECK(direction IN ('issued','received')),
                     PRIMARY KEY(peer, direction, ticket)
                 );
                 CREATE INDEX idx_lxmf_tickets_expiry ON lxmf_tickets(expires_at);
                 INSERT INTO schema_migrations (id, applied_at) VALUES ('2026-08-22-canonical-lxmf-inbound-v3', CAST(strftime('%s','now') AS INTEGER));
                 COMMIT;",
            )?;
        }
        let fidelity_migration = "2026-08-23-canonical-lxmf-fidelity-v4";
        let fidelity_migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![fidelity_migration],
            |row| row.get(0),
        )?;
        if !fidelity_migration_applied {
            let transaction = self.conn.unchecked_transaction()?;
            transaction.execute_batch(
                "ALTER TABLE lxmf_stamp_policy RENAME TO lxmf_stamp_policy_v3;
                 CREATE TABLE lxmf_stamp_policy (
                     singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                     target_cost INTEGER NOT NULL CHECK(target_cost BETWEEN 0 AND 254),
                     flexibility INTEGER NOT NULL CHECK(flexibility BETWEEN 0 AND 254)
                 );
                 INSERT INTO lxmf_stamp_policy SELECT singleton, target_cost, flexibility
                     FROM lxmf_stamp_policy_v3;
                 DROP TABLE lxmf_stamp_policy_v3;
                 ALTER TABLE lxmf_peer_costs RENAME TO lxmf_peer_costs_v3;
                 CREATE TABLE lxmf_peer_costs (
                     peer TEXT PRIMARY KEY CHECK(length(peer) <= 128),
                     stamp_cost INTEGER NOT NULL CHECK(stamp_cost BETWEEN 0 AND 254),
                     observed_at INTEGER NOT NULL
                 );
                 INSERT INTO lxmf_peer_costs SELECT peer, stamp_cost, observed_at
                     FROM lxmf_peer_costs_v3;
                 DROP TABLE lxmf_peer_costs_v3;
                 CREATE TABLE lxmf_ticket_deliveries (
                     peer TEXT PRIMARY KEY CHECK(length(peer) <= 128),
                     last_delivered_at INTEGER NOT NULL
                 );
                 CREATE TABLE outbound_ticket_offers (
                     message_id TEXT PRIMARY KEY REFERENCES outbound_routes(message_id) ON DELETE CASCADE,
                     peer TEXT NOT NULL CHECK(length(peer) <= 128),
                     ticket BLOB NOT NULL CHECK(length(ticket) = 16),
                      expires_at INTEGER NOT NULL,
                       delivered_at INTEGER
                  );
                  CREATE TABLE lxmf_ticket_offer_reservations (
                      peer TEXT PRIMARY KEY CHECK(length(peer) <= 128),
                      reservation_id TEXT NOT NULL UNIQUE CHECK(length(reservation_id) <= 128),
                      reserved_at INTEGER NOT NULL
                  );",
            )?;
            let legacy_rows = {
                let mut statement = transaction
                    .prepare("SELECT message_id, wire FROM canonical_inbound_messages")?;
                let rows = statement
                    .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            for (message_id, wire) in legacy_rows {
                match lxmf::inbound_decode::decode_inbound_message(
                    [0; 16],
                    &wire,
                    lxmf::inbound_decode::InboundPayloadMode::FullWire,
                ) {
                    Ok(decoded) if decoded.id == message_id => {
                        transaction.execute(
                            "UPDATE canonical_inbound_messages SET
                                 source = ?2, destination = ?3, title = ?4, content = ?5,
                                 timestamp = ?6, fields_msgpack = ?7, signature = ?8,
                                 stamp = ?9, wire = ?10
                             WHERE message_id = ?1",
                            params![
                                message_id,
                                decoded.source.as_slice(),
                                decoded.destination.as_slice(),
                                decoded.title,
                                decoded.content,
                                decoded.timestamp,
                                decoded.fields_msgpack,
                                decoded.signature.map(|value| value.to_vec()),
                                decoded.stamp,
                                decoded.wire,
                            ],
                        )?;
                    }
                    Ok(_) | Err(_) => {
                        transaction.execute(
                            "UPDATE canonical_inbound_messages
                             SET authentication_state = 'invalid'
                             WHERE message_id = ?1",
                            params![message_id],
                        )?;
                    }
                }
            }
            transaction.execute(
                "INSERT INTO schema_migrations (id, applied_at) VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
                params![fidelity_migration],
            )?;
            transaction.commit()?;
        }
        let reservation_migration = "2026-08-23-lxmf-ticket-offer-reservations-v5";
        let reservation_migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![reservation_migration],
            |row| row.get(0),
        )?;
        if !reservation_migration_applied {
            self.conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS lxmf_ticket_offer_reservations (
                     peer TEXT PRIMARY KEY CHECK(length(peer) <= 128),
                     reservation_id TEXT NOT NULL UNIQUE CHECK(length(reservation_id) <= 128),
                     reserved_at INTEGER NOT NULL
                 );
                 INSERT INTO schema_migrations (id, applied_at) VALUES
                     ('2026-08-23-lxmf-ticket-offer-reservations-v5', CAST(strftime('%s','now') AS INTEGER));
                 COMMIT;",
            )?;
        }
        let conversation_migration = "2026-08-23-conversation-state-drafts-v6";
        let conversation_migration_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![conversation_migration],
            |row| row.get(0),
        )?;
        if !conversation_migration_applied {
            let transaction =
                self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let migration_applied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
                params![conversation_migration],
                |row| row.get(0),
            )?;
            if !migration_applied {
                rebuild_conversation_schema(&transaction)?;
                if !conversation_schema_is_valid(&transaction)? {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "v6 conversation schema validation failed".into(),
                    ));
                }

                let valid_contacts = {
                    let mut statement = transaction.prepare(
                        "SELECT peer_hash, alias, notes, created_at, updated_at
                         FROM contacts
                         WHERE typeof(peer_hash) = 'text'
                           AND length(peer_hash) = 32
                           AND peer_hash NOT GLOB '*[^0-9A-Fa-f]*'
                           AND typeof(alias) IN ('null', 'text')
                           AND typeof(notes) IN ('null', 'text')
                           AND typeof(created_at) = 'integer' AND created_at >= 0
                           AND typeof(updated_at) = 'integer' AND updated_at >= 0
                         ORDER BY lower(peer_hash), updated_at DESC,
                                  created_at DESC, peer_hash DESC",
                    )?;
                    let contacts = statement
                        .query_map([], |row| {
                            Ok(ContactRecord {
                                peer_hash: row.get(0)?,
                                alias: row.get(1)?,
                                notes: row.get(2)?,
                                created_at: row.get(3)?,
                                updated_at: row.get(4)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    contacts
                };
                let mut merged = std::collections::BTreeMap::<String, ContactRecord>::new();
                for contact in valid_contacts {
                    let canonical = contact.peer_hash.to_ascii_lowercase();
                    if let Some(existing) = merged.get_mut(&canonical) {
                        existing.created_at = existing.created_at.min(contact.created_at);
                        existing.updated_at = existing.updated_at.max(contact.updated_at);
                    } else {
                        merged.insert(
                            canonical.clone(),
                            ContactRecord { peer_hash: canonical, ..contact },
                        );
                    }
                }
                transaction.execute(
                    "DELETE FROM contacts
                     WHERE typeof(peer_hash) = 'text'
                       AND length(peer_hash) = 32
                       AND peer_hash NOT GLOB '*[^0-9A-Fa-f]*'
                       AND typeof(alias) IN ('null', 'text')
                       AND typeof(notes) IN ('null', 'text')
                       AND typeof(created_at) = 'integer' AND created_at >= 0
                       AND typeof(updated_at) = 'integer' AND updated_at >= 0",
                    [],
                )?;
                for contact in merged.into_values() {
                    transaction.execute(
                        "INSERT INTO contacts
                         (peer_hash, alias, notes, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            contact.peer_hash,
                            contact.alias,
                            contact.notes,
                            contact.created_at,
                            contact.updated_at,
                        ],
                    )?;
                }
                transaction.execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_contacts_canonical_peer
                     ON contacts(lower(peer_hash))
                     WHERE typeof(peer_hash) = 'text'
                       AND length(peer_hash) = 32
                       AND peer_hash NOT GLOB '*[^0-9A-Fa-f]*'
                       AND typeof(alias) IN ('null', 'text')
                       AND typeof(notes) IN ('null', 'text')
                       AND typeof(created_at) = 'integer' AND created_at >= 0
                       AND typeof(updated_at) = 'integer' AND updated_at >= 0;",
                )?;
                transaction.execute(
                    "INSERT INTO schema_migrations (id, applied_at)
                  VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
                    params![conversation_migration],
                )?;
            }
            transaction.commit()?;
        }
        let schema_hardening_migration = "2026-08-23-conversation-schema-hardening-v7";
        let schema_hardening_applied: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
            params![schema_hardening_migration],
            |row| row.get(0),
        )?;
        if !schema_hardening_applied {
            let transaction =
                self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let migration_applied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE id = ?1)",
                params![schema_hardening_migration],
                |row| row.get(0),
            )?;
            if !migration_applied {
                if !conversation_schema_is_valid(&transaction)? {
                    rebuild_conversation_schema(&transaction)?;
                }
                if !conversation_schema_is_valid(&transaction)? {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "v7 conversation schema validation failed".into(),
                    ));
                }
                transaction.execute(
                    "INSERT INTO schema_migrations (id, applied_at)
                     VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
                    params![schema_hardening_migration],
                )?;
            }
            transaction.commit()?;
        }
        let pagination_migration = "2026-08-23-stable-message-pagination-v8";
        ensure_pagination_schema(&mut self.conn, pagination_migration)?;
        let attachment_migration = "2026-08-24-lxmf-inline-attachments-v9";
        ensure_attachment_schema(&mut self.conn, attachment_migration)?;
        super::standard_propagation::ensure_standard_propagation_schema(&mut self.conn)?;
        ensure_message_inspection_schema(&mut self.conn)?;
        self.prune_delivery_evidence(unix_time_secs())?;
        Ok(())
    }

    // ── Blocklist ────────────────────────────────────────────────────────

    /// Block a peer. Returns true if newly blocked, false if already blocked.
    pub fn block_peer(&self, identity_hash: &str) -> rusqlite::Result<bool> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO blocked_peers (identity_hash, blocked_at) VALUES (?1, ?2)",
            rusqlite::params![identity_hash, now],
        )?;
        Ok(changed > 0)
    }

    /// Unblock a peer. Returns true if was blocked, false if wasn't.
    pub fn unblock_peer(&self, identity_hash: &str) -> rusqlite::Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM blocked_peers WHERE identity_hash = ?1",
            rusqlite::params![identity_hash],
        )?;
        Ok(changed > 0)
    }

    /// List all blocked peer identity hashes.
    pub fn blocked_peers(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt =
            self.conn.prepare("SELECT identity_hash FROM blocked_peers ORDER BY blocked_at")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Check if a peer is blocked.
    pub fn is_blocked(&self, identity_hash: &str) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM blocked_peers WHERE identity_hash = ?1",
            rusqlite::params![identity_hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    // ── RBAC Roster ─────────────────────────────────────────────────────

    /// Load all RBAC roster entries from the database.
    pub fn load_rbac_roster(&self) -> rusqlite::Result<Vec<styrene_rbac::RosterEntry>> {
        let mut stmt =
            self.conn.prepare("SELECT identity_hash, role, label, grants FROM rbac_roster")?;
        let rows = stmt.query_map([], |row| {
            let hash: String = row.get(0)?;
            let role_str: String = row.get(1)?;
            let label: String = row.get(2)?;
            let grants_csv: String = row.get(3)?;
            Ok((hash, role_str, label, grants_csv))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (hash, role_str, label, grants_csv) = row?;
            let role = styrene_rbac::Role::from_name(&role_str).unwrap_or(styrene_rbac::Role::Peer);
            let grants: Vec<String> = if grants_csv.is_empty() {
                Vec::new()
            } else if grants_csv.starts_with('[') {
                // JSON array format (current)
                serde_json::from_str(&grants_csv).unwrap_or_default()
            } else {
                // Legacy CSV format (migration path)
                grants_csv.split(',').map(|s| s.trim().to_string()).collect()
            };
            entries.push(
                styrene_rbac::RosterEntry::new(hash, role).with_label(label).with_grants(grants),
            );
        }
        Ok(entries)
    }

    /// Insert or replace an RBAC roster entry.
    ///
    /// Identity hash is normalized to lowercase before storage to ensure
    /// consistent lookups (DELETE by hash must match INSERT case).
    /// Grants are stored as a JSON array for safe round-tripping.
    pub fn upsert_rbac_entry(&self, entry: &styrene_rbac::RosterEntry) -> rusqlite::Result<()> {
        let normalized_hash = entry.identity_hash.to_ascii_lowercase();
        let grants_json = serde_json::to_string(entry.grants()).unwrap_or_default();
        self.conn.execute(
            "INSERT OR REPLACE INTO rbac_roster (identity_hash, role, label, grants) VALUES (?1, ?2, ?3, ?4)",
            params![normalized_hash, entry.role.as_str(), entry.label, grants_json],
        )?;
        Ok(())
    }

    /// Remove an RBAC roster entry by identity hash.
    ///
    /// Identity hash is normalized to lowercase to match stored format.
    pub fn remove_rbac_entry(&self, identity_hash: &str) -> rusqlite::Result<bool> {
        let normalized = identity_hash.to_ascii_lowercase();
        let changed = self
            .conn
            .execute("DELETE FROM rbac_roster WHERE identity_hash = ?1", params![normalized])?;
        Ok(changed > 0)
    }

    /// Store an LXMF propagation packet for later delivery.
    ///
    /// Returns `Ok(false)` when the packet is already stored (deduplicated by
    /// deterministic packet id).
    pub fn propagation_ingest(
        &self,
        dest_hash: &str,
        lxmf_bytes: &[u8],
        source_hash: Option<&str>,
        max_age_secs: u64,
    ) -> rusqlite::Result<bool> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let expires_at = now.saturating_add(max_age_secs.min(i64::MAX as u64) as i64);
        let digest = sha2::Sha256::digest(lxmf_bytes);
        let packet_id = hex::encode(&digest[..16]);
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO propagation_store
             (id, dest_hash, lxmf_bytes, source_hash, received_at, expires_at, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                packet_id,
                dest_hash,
                lxmf_bytes,
                source_hash,
                now,
                expires_at,
                lxmf_bytes.len() as i64,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Fetch raw propagation packets queued for a destination.
    pub fn propagation_fetch(&self, dest_hash: &str) -> rusqlite::Result<Vec<Vec<u8>>> {
        let mut stmt = self.conn.prepare(
            "SELECT lxmf_bytes FROM propagation_store
             WHERE dest_hash = ?1
             ORDER BY received_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![dest_hash], |row| row.get(0))?;
        rows.collect()
    }

    /// Fetch propagation packet ids with payloads for delivery/deletion.
    pub fn propagation_fetch_with_ids(
        &self,
        dest_hash: &str,
    ) -> rusqlite::Result<Vec<(String, Vec<u8>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, lxmf_bytes FROM propagation_store
             WHERE dest_hash = ?1
             ORDER BY received_at ASC",
        )?;
        let rows =
            stmt.query_map(rusqlite::params![dest_hash], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Delete expired propagation packets.
    pub fn propagation_expire(&self) -> rusqlite::Result<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let changed = self.conn.execute(
            "DELETE FROM propagation_store WHERE expires_at <= ?1",
            rusqlite::params![now],
        )?;
        Ok(changed)
    }

    /// Count queued propagation packets and their total stored size.
    pub fn propagation_stats(&self) -> rusqlite::Result<(usize, u64)> {
        let (count, total): (i64, Option<i64>) = self.conn.query_row(
            "SELECT COUNT(*), SUM(size_bytes) FROM propagation_store",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((count.max(0) as usize, total.unwrap_or(0).max(0) as u64))
    }

    pub fn propagation_inventory(
        &self,
        limit: usize,
        after: Option<(i64, &str)>,
    ) -> rusqlite::Result<Vec<PropagationInventoryRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, dest_hash, source_hash, received_at, expires_at, size_bytes
             FROM propagation_store
             WHERE (?2 IS NULL OR received_at > ?2 OR (received_at = ?2 AND id > ?3))
             ORDER BY received_at ASC, id ASC LIMIT ?1",
        )?;
        let (after_timestamp, after_id) =
            after.map(|(timestamp, id)| (Some(timestamp), Some(id))).unwrap_or((None, None));
        let rows = statement
            .query_map(
                params![i64::try_from(limit).unwrap_or(i64::MAX), after_timestamp, after_id],
                |row| {
                    Ok(PropagationInventoryRecord {
                        id: row.get(0)?,
                        destination_hash: row.get(1)?,
                        source_hash: row.get(2)?,
                        received_at: row.get(3)?,
                        expires_at: row.get(4)?,
                        size_bytes: row.get(5)?,
                    })
                },
            )?
            .collect();
        rows
    }

    /// Delete propagation packets by id after successful delivery.
    pub fn propagation_delete(&self, ids: &[String]) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare("DELETE FROM propagation_store WHERE id = ?1")?;
        for id in ids {
            stmt.execute(rusqlite::params![id])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn outbound_message(id: &str, timestamp: i64, receipt_status: Option<&str>) -> MessageRecord {
        MessageRecord {
            id: id.to_string(),
            source: "src".to_string(),
            destination: "dst".to_string(),
            title: "title".to_string(),
            content: "body".to_string(),
            timestamp,
            direction: "out".to_string(),
            fields: None,
            receipt_status: receipt_status.map(ToString::to_string),
            read: false,
        }
    }

    fn peer_message(id: &str, peer: &str, timestamp: i64, incoming: bool) -> MessageRecord {
        MessageRecord {
            id: id.into(),
            source: if incoming { peer.into() } else { "11111111111111111111111111111111".into() },
            destination: if incoming {
                "11111111111111111111111111111111".into()
            } else {
                peer.into()
            },
            title: String::new(),
            content: id.into(),
            timestamp,
            direction: if incoming { "in".into() } else { "out".into() },
            fields: None,
            receipt_status: None,
            read: !incoming,
        }
    }

    fn collect_message_pages(store: &MessagesStore, peer: &str, limit: usize) -> Vec<String> {
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page =
                store.message_projection_page_for_peer(peer, limit, cursor.as_deref()).unwrap();
            ids.extend(page.items.into_iter().map(|item| item.message.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                return ids;
            }
        }
    }

    #[test]
    fn v8_backfills_deterministically_and_trigger_covers_compatibility_writers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE messages (
                         id TEXT PRIMARY KEY, source TEXT NOT NULL, destination TEXT NOT NULL,
                         title TEXT NOT NULL, content TEXT NOT NULL, timestamp INTEGER NOT NULL,
                         direction TEXT NOT NULL, fields TEXT, receipt_status TEXT, read INTEGER
                     );
                     INSERT INTO messages VALUES
                         ('b', 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 'local', '', 'b', 9, 'in', NULL, NULL, 0),
                         ('a', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'local', '', 'a', 9, 'in', NULL, NULL, 0);",
                )
                .unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        let keys: Vec<(String, i64, String)> = store
            .conn
            .prepare(
                "SELECT message_id, ingest_seq, conversation_peer
                 FROM message_page_keys ORDER BY ingest_seq",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(keys, vec![("a".into(), 1, "a".repeat(32)), ("b".into(), 2, "a".repeat(32))]);

        store
            .conn
            .execute(
                "INSERT INTO messages VALUES
                 ('compat', 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', 'local', '', '', 1, 'in', NULL, NULL, 0)",
                [],
            )
            .unwrap();
        let key: (i64, String) = store
            .conn
            .query_row(
                "SELECT sort_timestamp, conversation_peer FROM message_page_keys
                 WHERE message_id = 'compat'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, (1, "a".repeat(32)));
        store.insert_message(&peer_message("compat", &"a".repeat(32), 999, true)).unwrap();
        let unchanged: (i64, i64) = store
            .conn
            .query_row(
                "SELECT sort_timestamp, ingest_seq FROM message_page_keys
                 WHERE message_id = 'compat'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(unchanged, (1, 3));
        let plan = store
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT message_id FROM message_page_keys
                 WHERE conversation_peer = ?1 AND ingest_seq <= ?2
                 ORDER BY sort_timestamp DESC, ingest_seq DESC LIMIT 10",
            )
            .unwrap()
            .query_map(params!["a".repeat(32), i64::MAX], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(plan.iter().any(|detail| detail.contains("idx_message_page_peer_order")));
        let aggregation_plan = store
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT conversation_peer, COUNT(*)
                 FROM message_page_keys WHERE ingest_seq <= ?1 GROUP BY conversation_peer",
            )
            .unwrap()
            .query_map(params![i64::MAX], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            aggregation_plan.iter().any(|detail| {
                detail.contains("INTEGER PRIMARY KEY")
                    || detail.contains("idx_message_page_snapshot_conversation")
                    || detail.contains("COVERING INDEX idx_message_page_peer_order")
            }),
            "{aggregation_plan:?}"
        );
    }

    #[test]
    fn marker_present_v8_repairs_post_v6_v7_triggers_and_pin_stales_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        let peers = ["11111111111111111111111111111111", "22222222222222222222222222222222"];
        let cursor = {
            let store = MessagesStore::open(&path).unwrap();
            for (index, peer) in peers.iter().enumerate() {
                store
                    .insert_message(&peer_message(&format!("repair-{index}"), peer, 10, true))
                    .unwrap();
            }
            let cursor = store.conversation_page(false, 1, None).unwrap().next_cursor.unwrap();
            store
                .conn
                .execute(
                    "DELETE FROM schema_migrations WHERE id IN (
                         '2026-08-23-conversation-state-drafts-v6',
                         '2026-08-23-conversation-schema-hardening-v7')",
                    [],
                )
                .unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TRIGGER messages_page_key_after_insert;
                     DROP INDEX idx_message_page_peer_order;",
                )
                .unwrap();
            assert!(store
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations
                     WHERE id = '2026-08-23-stable-message-pagination-v8')",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap());
            cursor
        };
        let store = MessagesStore::open(&path).unwrap();
        store.insert_message(&peer_message("repair-new", peers[0], 11, true)).unwrap();
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM message_page_keys WHERE message_id = 'repair-new'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let invariant_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name IN (
                     'messages_page_key_after_insert',
                     'messages_page_membership_immutable',
                     'message_page_keys_immutable',
                     'message_page_keys_delete_guard',
                     'message_page_metadata_secret_immutable',
                     'messages_conversation_epoch_after_delete',
                     'messages_conversation_epoch_after_read',
                     'conversation_state_epoch_after_insert',
                     'conversation_state_epoch_after_pin',
                     'conversation_state_epoch_after_delete')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invariant_count, 10);
        let index_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name IN (
                     'idx_message_page_peer_order',
                     'idx_message_page_snapshot_conversation')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 2);
        store.set_conversation_pinned(peers[0], true).unwrap();
        assert!(matches!(
            store.conversation_page(false, 1, Some(&cursor)),
            Err(PageError::CursorStale)
        ));
    }

    #[test]
    fn message_membership_is_immutable_for_upsert_and_compatibility_sql() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        let store = MessagesStore::open(&path).unwrap();
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let original = peer_message("immutable", peer, 1, true);
        store.insert_message(&original).unwrap();

        let mut conflict = peer_message("immutable", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", 99, false);
        conflict.title = "updated".into();
        conflict.content = "mutable content".into();
        conflict.receipt_status = Some("delivered".into());
        conflict.read = true;
        store.insert_message(&conflict).unwrap();
        let authoritative = store.get_message("immutable").unwrap().unwrap();
        assert_eq!(
            (
                authoritative.source.as_str(),
                authoritative.destination.as_str(),
                authoritative.timestamp,
                authoritative.direction.as_str()
            ),
            (
                original.source.as_str(),
                original.destination.as_str(),
                original.timestamp,
                original.direction.as_str()
            )
        );
        assert_eq!(authoritative.content, "mutable content");
        assert_eq!(authoritative.receipt_status.as_deref(), Some("delivered"));

        let compatibility = Connection::open(&path).unwrap();
        compatibility.busy_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert!(compatibility
            .execute(
                "UPDATE messages SET source = ?2, timestamp = 500 WHERE id = ?1",
                params!["immutable", "cccccccccccccccccccccccccccccccc"],
            )
            .is_err());
        assert_eq!(
            compatibility
                .execute(
                    "UPDATE messages SET receipt_status = 'read', read = 1 WHERE id = 'immutable'",
                    [],
                )
                .unwrap(),
            1
        );
        let coherent: bool = compatibility
            .query_row(
                "SELECT k.sort_timestamp = m.timestamp AND k.conversation_peer = lower(m.source)
                 FROM messages m JOIN message_page_keys k ON k.message_id = m.id
                 WHERE m.id = 'immutable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(coherent);
    }

    #[test]
    fn compatibility_connection_cannot_orphan_page_key_but_parent_cascade_works() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        let store = MessagesStore::open(&path).unwrap();
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        store.insert_message(&peer_message("guarded", peer, 1, true)).unwrap();
        let compatibility = Connection::open(&path).unwrap();
        compatibility.pragma_update(None, "foreign_keys", "ON").unwrap();
        assert!(compatibility
            .execute("DELETE FROM message_page_keys WHERE message_id = 'guarded'", [])
            .is_err());
        assert_eq!(
            compatibility.execute("DELETE FROM messages WHERE id = 'guarded'", []).unwrap(),
            1
        );
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM message_page_keys WHERE message_id = 'guarded'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn old_v8_metadata_upgrades_secret_transactionally_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let old_store_id = {
            let store = MessagesStore::open(&path).unwrap();
            store.insert_message(&peer_message("a", peer, 1, true)).unwrap();
            store.insert_message(&peer_message("b", peer, 2, true)).unwrap();
            let metadata = page_metadata(&store.conn).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TRIGGER message_page_metadata_secret_immutable;
                     ALTER TABLE message_page_metadata RENAME TO message_page_metadata_v8_auth;
                     CREATE TABLE message_page_metadata (
                         singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                         store_id BLOB NOT NULL UNIQUE CHECK(length(store_id) = 16),
                         conversation_epoch INTEGER NOT NULL DEFAULT 0 CHECK(conversation_epoch >= 0)
                     );
                     INSERT INTO message_page_metadata
                         SELECT singleton, store_id, conversation_epoch
                         FROM message_page_metadata_v8_auth;
                     DROP TABLE message_page_metadata_v8_auth;",
                )
                .unwrap();
            metadata.store_id
        };
        let cursor = {
            let store = MessagesStore::open(&path).unwrap();
            let metadata = page_metadata(&store.conn).unwrap();
            assert_eq!(metadata.store_id, old_store_id);
            store.message_projection_page_for_peer(peer, 1, None).unwrap().next_cursor.unwrap()
        };
        let store = MessagesStore::open(&path).unwrap();
        assert_eq!(
            store.message_projection_page_for_peer(peer, 1, Some(&cursor)).unwrap().items.len(),
            1
        );
    }

    #[test]
    fn conversation_epoch_exhaustion_fails_mutations_without_partial_effects() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        store.insert_message(&peer_message("epoch", peer, 1, true)).unwrap();
        store.set_conversation_pinned(peer, false).unwrap();
        store
            .conn
            .execute(
                "UPDATE message_page_metadata SET conversation_epoch = ?1 WHERE singleton = 1",
                params![i64::MAX],
            )
            .unwrap();
        assert!(store.set_conversation_pinned(peer, true).is_err());
        assert!(store.mark_read(peer).is_err());
        assert!(store.delete_message("epoch").is_err());
        assert!(!store.list_conversations(false).unwrap()[0].pinned);
        assert!(!store.get_message("epoch").unwrap().unwrap().read);
    }

    #[test]
    fn open_rejects_malformed_pagination_metadata_types() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TRIGGER message_page_metadata_secret_immutable;
                     ALTER TABLE message_page_metadata RENAME TO message_page_metadata_valid;
                     CREATE TABLE message_page_metadata (
                         singleton, store_id, conversation_epoch, cursor_secret
                     );
                     INSERT INTO message_page_metadata
                         SELECT singleton, store_id, 1.5, cursor_secret
                         FROM message_page_metadata_valid;
                     DROP TABLE message_page_metadata_valid;",
                )
                .unwrap();
        }
        assert!(MessagesStore::open(&path).is_err());
    }

    #[test]
    fn message_pages_complete_equal_timestamps_and_exclude_later_inserts() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for id in ["a", "b", "c", "d", "e", "f"] {
            store.insert_message(&peer_message(id, peer, 10, true)).unwrap();
        }
        let first = store.message_projection_page_for_peer(peer, 2, None).unwrap();
        assert_eq!(
            first.items.iter().map(|item| item.message.id.as_str()).collect::<Vec<_>>(),
            vec!["f", "e"]
        );
        store.insert_message(&peer_message("newer", peer, 20, true)).unwrap();
        store.insert_message(&peer_message("backdated", peer, 1, true)).unwrap();
        let second = store
            .message_projection_page_for_peer(peer, 256, first.next_cursor.as_deref())
            .unwrap();
        assert_eq!(
            second.items.into_iter().map(|item| item.message.id).collect::<Vec<_>>(),
            vec!["d", "c", "b", "a"]
        );
        assert_eq!(collect_message_pages(&store, peer, 2).len(), 8);
    }

    #[test]
    fn message_cursor_survives_restart_and_deleted_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("messages.db");
        let peer = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let cursor = {
            let store = MessagesStore::open(&path).unwrap();
            for id in ["a", "b", "c", "d"] {
                store.insert_message(&peer_message(id, peer, 5, false)).unwrap();
            }
            let page = store.message_projection_page_for_peer(peer, 2, None).unwrap();
            assert_eq!(page.items[1].message.id, "c");
            store.delete_message("c").unwrap();
            page.next_cursor.unwrap()
        };
        let store = MessagesStore::open(&path).unwrap();
        let page = store.message_projection_page_for_peer(peer, 10, Some(&cursor)).unwrap();
        assert_eq!(
            page.items.into_iter().map(|item| item.message.id).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert!(matches!(
            store.message_projection_page_for_peer(
                "cccccccccccccccccccccccccccccccc",
                10,
                Some(&cursor)
            ),
            Err(PageError::InvalidCursor(_))
        ));
        let other = MessagesStore::in_memory().unwrap();
        assert!(matches!(
            other.message_projection_page_for_peer(peer, 10, Some(&cursor)),
            Err(PageError::InvalidCursor(_))
        ));
    }

    #[test]
    fn conversation_pages_use_complete_order_and_mutations_stale_cursor() {
        let store = MessagesStore::in_memory().unwrap();
        let peers = [
            "11111111111111111111111111111111",
            "22222222222222222222222222222222",
            "33333333333333333333333333333333",
        ];
        for (index, peer) in peers.iter().enumerate() {
            store.insert_message(&peer_message(&format!("m{index}"), peer, 10, true)).unwrap();
        }
        store.set_conversation_pinned(peers[1], true).unwrap();
        let first = store.conversation_page(false, 1, None).unwrap();
        assert_eq!(first.items[0].peer_hash, peers[1]);
        let cursor = first.next_cursor.unwrap();
        let remaining = store.conversation_page(false, 10, Some(&cursor)).unwrap();
        assert_eq!(
            remaining.items.into_iter().map(|item| item.peer_hash).collect::<Vec<_>>(),
            vec![peers[2].to_string(), peers[0].to_string()]
        );

        let cursor = store.conversation_page(true, 1, None).unwrap().next_cursor.unwrap();
        assert!(matches!(
            store.conversation_page(false, 1, Some(&cursor)),
            Err(PageError::InvalidCursor(_))
        ));
        store.mark_read(peers[0]).unwrap();
        assert!(matches!(
            store.conversation_page(true, 1, Some(&cursor)),
            Err(PageError::CursorStale)
        ));
        let cursor = store.conversation_page(false, 1, None).unwrap().next_cursor.unwrap();
        store.set_conversation_pinned(peers[0], true).unwrap();
        assert!(matches!(
            store.conversation_page(false, 1, Some(&cursor)),
            Err(PageError::CursorStale)
        ));
        let cursor = store.conversation_page(false, 1, None).unwrap().next_cursor.unwrap();
        store.delete_message("m2").unwrap();
        assert!(matches!(
            store.conversation_page(false, 1, Some(&cursor)),
            Err(PageError::CursorStale)
        ));
    }

    fn outbound_route(id: &str, retry_of: Option<&str>) -> OutboundRouteRecord {
        OutboundRouteRecord {
            message_id: id.into(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "retry-chain".into(),
            retry_of: retry_of.map(str::to_owned),
            deadline_unix_ms: 100,
            state: "queued".into(),
            attempt_count: 0,
        }
    }

    fn add_attempt(store: &MessagesStore, message_id: &str) {
        store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: message_id.into(),
                attempt_number: 1,
                started_unix_ms: 1,
                deadline_unix_ms: 100,
                state: "delivered".into(),
            })
            .unwrap();
    }

    #[test]
    fn concurrent_connections_wait_for_brief_writer_contention() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("messages.db");
        let first = MessagesStore::open(&path).unwrap();
        let second = MessagesStore::open(&path).unwrap();

        first.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            first.conn.execute_batch("COMMIT").unwrap();
        });

        let record = outbound_message("contended", 1, Some("sending"));
        second.insert_message(&record).expect("busy timeout should permit retry");
        releaser.join().unwrap();
        assert!(second.get_message("contended").unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn database_open_rejects_symlinks_without_chmodding_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.db");
        let link = temp.path().join("messages.db");
        std::fs::write(&target, []).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
        symlink(&target, &link).unwrap();

        assert!(MessagesStore::open(&link).is_err());
        assert_eq!(std::fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o666);
    }

    #[cfg(unix)]
    #[test]
    fn database_open_never_chmods_symlinked_sidecar_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("messages.db");
        let target = temp.path().join("sidecar-target");
        drop(Connection::open(&path).unwrap());
        std::fs::write(&target, []).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
        let mut wal = path.as_os_str().to_os_string();
        wal.push("-wal");
        symlink(&target, std::path::PathBuf::from(wal)).unwrap();

        assert!(MessagesStore::open(&path).is_err());
        assert_eq!(std::fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o666);
    }

    #[cfg(unix)]
    #[test]
    fn guarded_open_rejects_regular_file_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("messages.db");
        let displaced = temp.path().join("displaced.db");
        let (secured_path, guarded) = securely_create_database_file(&path).unwrap();
        std::fs::rename(&secured_path, &displaced).unwrap();
        std::fs::write(&secured_path, []).unwrap();

        assert!(open_guarded_connection(&secured_path, &guarded).is_err());
    }

    #[test]
    fn database_open_rejects_non_regular_path() {
        let temp = tempfile::tempdir().unwrap();
        assert!(MessagesStore::open(temp.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "spawns a child process to isolate a permissive umask from the offline test runner"]
    fn new_database_file_is_private_under_permissive_child_umask() {
        use std::os::unix::fs::PermissionsExt;

        if let Some(path) = std::env::var_os("STYRENE_PRIVATE_DB_CHILD") {
            let path = std::path::PathBuf::from(path);
            assert!(!path.exists());
            let _store = MessagesStore::open(&path).unwrap();
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("new-private.db");
        let executable = std::env::current_exe().unwrap();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(
                "umask 000; exec \"$1\" --exact \
                 storage::messages::tests::new_database_file_is_private_under_permissive_child_umask \
                 --ignored --nocapture",
            )
            .arg("sh")
            .arg(executable)
            .env("STYRENE_PRIVATE_DB_CHILD", &path)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn router_migration_and_records_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("messages.db");
        {
            let legacy = Connection::open(&path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE messages (
                        id TEXT PRIMARY KEY, source TEXT NOT NULL, destination TEXT NOT NULL,
                        title TEXT NOT NULL, content TEXT NOT NULL, timestamp INTEGER NOT NULL,
                        direction TEXT NOT NULL, fields TEXT, receipt_status TEXT, read INTEGER DEFAULT 0
                    );",
                )
                .unwrap();
        }
        let route = OutboundRouteRecord {
            message_id: "routed".into(),
            requested_method: "opportunistic".into(),
            actual_method: "direct".into(),
            representation: "resource".into(),
            fallback_reason: Some("encoded payload exceeds packet limit".into()),
            correlation_id: "correlation".into(),
            retry_of: None,
            deadline_unix_ms: 42_000,
            state: "queued".into(),
            attempt_count: 0,
        };
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .insert_outbound_message(&outbound_message("routed", 1, Some("queued")), &route)
                .unwrap();
            assert!(store
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: "routed".into(),
                    attempt_number: 1,
                    started_unix_ms: 10_000,
                    deadline_unix_ms: 42_000,
                    state: "sending".into(),
                })
                .unwrap());
        }

        let reopened = MessagesStore::open(&path).unwrap();
        let persisted = reopened.outbound_route("routed").unwrap().unwrap();
        assert_eq!(
            persisted,
            OutboundRouteRecord { attempt_count: 1, state: "sending".into(), ..route }
        );
        assert_eq!(reopened.outbound_attempts("routed").unwrap().len(), 1);
    }

    #[test]
    fn foreign_keys_are_enabled_and_message_delete_cascades_router_state() {
        let store = MessagesStore::in_memory().unwrap();
        let foreign_keys: i64 =
            store.conn.pragma_query_value(None, "foreign_keys", |row| row.get(0)).unwrap();
        assert_eq!(foreign_keys, 1);
        let route = OutboundRouteRecord {
            message_id: "cascade".into(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "cascade".into(),
            retry_of: None,
            deadline_unix_ms: 42_000,
            state: "queued".into(),
            attempt_count: 0,
        };
        store
            .insert_outbound_message(&outbound_message("cascade", 1, Some("queued")), &route)
            .unwrap();
        store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: "cascade".into(),
                attempt_number: 1,
                started_unix_ms: 1,
                deadline_unix_ms: 42_000,
                state: "sending".into(),
            })
            .unwrap();

        assert!(store.finish_outbound("cascade", "delivered", "delivered").unwrap());
        assert!(store.delete_message("cascade").unwrap());

        assert!(store.outbound_route("cascade").unwrap().is_none());
        assert!(store.outbound_attempts("cascade").unwrap().is_empty());
    }

    #[test]
    fn message_upsert_preserves_lifecycle_and_canonical_children() {
        let store = MessagesStore::in_memory().unwrap();
        let route = OutboundRouteRecord {
            message_id: "upsert-route".into(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "upsert-route".into(),
            retry_of: None,
            deadline_unix_ms: 100,
            state: "sending".into(),
            attempt_count: 0,
        };
        store
            .insert_outbound_message(&outbound_message("upsert-route", 1, Some("sending")), &route)
            .unwrap();
        store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: "upsert-route".into(),
                attempt_number: 1,
                started_unix_ms: 1,
                deadline_unix_ms: 100,
                state: "sending".into(),
            })
            .unwrap();
        let mut updated = outbound_message("upsert-route", 2, Some("delivered"));
        updated.content = "updated without replacement".into();
        store.insert_message(&updated).unwrap();
        assert!(store.outbound_route("upsert-route").unwrap().is_some());
        assert_eq!(store.outbound_attempts("upsert-route").unwrap().len(), 1);

        let projection = chat_message("upsert-canonical", "aa", "me", 1);
        let canonical = CanonicalInboundRecord {
            message_id: projection.id.clone(),
            source: [1; 16],
            destination: [2; 16],
            title: Vec::new(),
            content: b"canonical".to_vec(),
            timestamp: 1.0,
            fields_msgpack: None,
            signature: None,
            stamp: None,
            wire: vec![3; 64],
            authentication_state: "unknown_identity".into(),
            stamp_state: "unknown".into(),
            stamp_value: None,
            stamp_target: None,
        };
        store.insert_canonical_inbound_if_absent(&projection, &canonical).unwrap();
        let mut updated_projection = projection;
        updated_projection.content = "new projection".into();
        store.insert_message(&updated_projection).unwrap();
        assert_eq!(store.canonical_inbound("upsert-canonical").unwrap(), Some(canonical));
    }

    #[test]
    fn sdk_domain_snapshot_roundtrip() {
        let store = MessagesStore::in_memory().expect("in-memory store");
        let initial = store.get_sdk_domain_snapshot().expect("query snapshot");
        assert!(initial.is_none(), "snapshot should be absent before first write");

        let snapshot = json!({
            "topics": [{ "topic_id": "topic-1" }],
            "attachments": [],
            "markers": [],
        });
        store.put_sdk_domain_snapshot(&snapshot).expect("persist snapshot");

        let loaded = store.get_sdk_domain_snapshot().expect("load snapshot");
        assert_eq!(loaded, Some(snapshot));
    }

    #[test]
    fn sdk_domain_snapshot_clear_removes_record() {
        let store = MessagesStore::in_memory().expect("in-memory store");
        store
            .put_sdk_domain_snapshot(&json!({ "voice_sessions": [{ "session_id": "voice-1" }] }))
            .expect("persist snapshot");
        store.clear_sdk_domain_snapshot().expect("clear snapshot");
        let loaded = store.get_sdk_domain_snapshot().expect("load snapshot");
        assert!(loaded.is_none(), "snapshot should be removed after clear");
    }

    #[test]
    fn expire_outbound_messages_marks_non_terminal_records() {
        let store = MessagesStore::in_memory().expect("in-memory store");
        store
            .insert_message(&outbound_message("out-non-terminal", 10, None))
            .expect("insert non-terminal");
        store
            .insert_message(&outbound_message("out-terminal", 10, Some("delivered")))
            .expect("insert terminal");
        let expired = store.expire_outbound_messages_before(11).expect("expire outbound");
        assert_eq!(expired, vec!["out-non-terminal".to_string()]);
        let non_terminal = store
            .get_message("out-non-terminal")
            .expect("load non-terminal")
            .expect("non-terminal exists");
        assert_eq!(non_terminal.receipt_status.as_deref(), Some("expired"));
        let terminal =
            store.get_message("out-terminal").expect("load terminal").expect("terminal exists");
        assert_eq!(terminal.receipt_status.as_deref(), Some("delivered"));
    }

    #[test]
    fn prune_outbound_messages_terminal_first_prefers_terminal_records() {
        let store = MessagesStore::in_memory().expect("in-memory store");
        store
            .insert_message(&outbound_message("msg-terminal-old", 1, Some("sent: direct")))
            .expect("insert terminal old");
        store
            .insert_message(&outbound_message("msg-non-terminal", 2, None))
            .expect("insert non-terminal");
        store
            .insert_message(&outbound_message("msg-terminal-new", 3, Some("delivered")))
            .expect("insert terminal new");

        let pruned = store.prune_outbound_messages(2, "terminal_first").expect("prune outbound");
        assert_eq!(pruned.len(), 2);
        assert!(pruned.iter().any(|id| id == "msg-terminal-old"));
        assert!(pruned.iter().any(|id| id == "msg-terminal-new"));
        assert!(
            store.get_message("msg-non-terminal").expect("load non-terminal").is_some(),
            "non-terminal record should remain when terminal records satisfy prune count"
        );
    }

    #[test]
    fn deleting_retry_original_detaches_and_preserves_retry_attempts() {
        let store = MessagesStore::in_memory().unwrap();
        store
            .insert_outbound_message(
                &outbound_message("original-delete", 1, Some("delivered")),
                &outbound_route("original-delete", None),
            )
            .unwrap();
        store
            .insert_outbound_message(
                &outbound_message("retry-delete", 2, Some("delivered")),
                &outbound_route("retry-delete", Some("original-delete")),
            )
            .unwrap();
        add_attempt(&store, "original-delete");
        add_attempt(&store, "retry-delete");
        assert!(store.finish_outbound("original-delete", "delivered", "delivered").unwrap());

        assert!(store.delete_message("original-delete").unwrap());
        assert!(store.outbound_route("original-delete").unwrap().is_none());
        assert!(store.outbound_attempts("original-delete").unwrap().is_empty());
        assert_eq!(store.outbound_route("retry-delete").unwrap().unwrap().retry_of, None);
        assert_eq!(store.outbound_attempts("retry-delete").unwrap().len(), 1);
    }

    #[test]
    fn pruning_retry_original_is_atomic_and_preserves_retry_attempts() {
        let store = MessagesStore::in_memory().unwrap();
        store
            .insert_outbound_message(
                &outbound_message("original-prune", 1, Some("delivered")),
                &outbound_route("original-prune", None),
            )
            .unwrap();
        store
            .insert_outbound_message(
                &outbound_message("retry-prune", 2, Some("delivered")),
                &outbound_route("retry-prune", Some("original-prune")),
            )
            .unwrap();
        add_attempt(&store, "original-prune");
        add_attempt(&store, "retry-prune");

        assert_eq!(
            store.prune_outbound_messages(1, "oldest").unwrap(),
            vec!["original-prune".to_string()]
        );
        assert_eq!(store.outbound_route("retry-prune").unwrap().unwrap().retry_of, None);
        assert_eq!(store.outbound_attempts("retry-prune").unwrap().len(), 1);
    }

    #[test]
    fn conversation_delete_detaches_retry_outside_conversation() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "e1".repeat(16);
        let survivor = "e2".repeat(16);
        let mut original = outbound_message("original-conversation", 1, Some("delivered"));
        original.destination = peer.clone();
        let mut retry = outbound_message("retry-conversation", 2, Some("delivered"));
        retry.destination = survivor;
        store.insert_outbound_message(&original, &outbound_route(&original.id, None)).unwrap();
        store
            .insert_outbound_message(&retry, &outbound_route(&retry.id, Some(&original.id)))
            .unwrap();
        add_attempt(&store, &retry.id);
        assert!(store.finish_outbound(&original.id, "delivered", "delivered").unwrap());

        assert_eq!(store.delete_conversation(&peer).unwrap(), 1);
        assert_eq!(store.outbound_route(&retry.id).unwrap().unwrap().retry_of, None);
        assert_eq!(store.outbound_attempts(&retry.id).unwrap().len(), 1);
    }

    // ── New store method tests ──────────────────────────────────────────

    fn chat_message(id: &str, source: &str, dest: &str, ts: i64) -> MessageRecord {
        MessageRecord {
            id: id.to_string(),
            source: source.to_string(),
            destination: dest.to_string(),
            title: String::new(),
            content: format!("message {id}"),
            timestamp: ts,
            direction: if source == "me" { "out".to_string() } else { "in".to_string() },
            fields: None,
            receipt_status: None,
            read: false,
        }
    }

    #[test]
    fn mark_read_updates_unread_messages() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        store.insert_message(&chat_message("m2", "alice", "me", 2)).expect("insert");
        store.insert_message(&chat_message("out", "me", "alice", 2)).expect("insert");
        store.insert_message(&chat_message("m3", "bob", "me", 3)).expect("insert");

        let count = store.mark_read("alice").expect("mark_read");
        assert_eq!(count, 2);

        let m1 = store.get_message("m1").expect("get").expect("exists");
        assert!(m1.read);
        let m3 = store.get_message("m3").expect("get").expect("exists");
        assert!(!m3.read); // Bob's message unchanged
        let outbound = store.get_message("out").expect("get").expect("exists");
        assert!(!outbound.read, "mark_read must not rewrite outbound rows");
    }

    #[test]
    fn delete_conversation_removes_all_peer_messages() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        store.insert_message(&chat_message("m2", "me", "alice", 2)).expect("insert");
        store.insert_message(&chat_message("m3", "bob", "me", 3)).expect("insert");

        let count = store.delete_conversation("alice").expect("delete");
        assert_eq!(count, 2);
        assert!(store.get_message("m1").expect("get").is_none());
        assert!(store.get_message("m2").expect("get").is_none());
        assert!(store.get_message("m3").expect("get").is_some());
    }

    #[test]
    fn delete_message_removes_single_record() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        assert!(store.delete_message("m1").expect("delete"));
        assert!(!store.delete_message("m1").expect("delete again"));
    }

    #[test]
    fn search_messages_finds_by_content() {
        let store = MessagesStore::in_memory().expect("store");
        let mut msg = chat_message("m1", "alice", "me", 1);
        msg.content = "hello world".to_string();
        store.insert_message(&msg).expect("insert");
        let mut msg2 = chat_message("m2", "bob", "me", 2);
        msg2.content = "goodbye".to_string();
        store.insert_message(&msg2).expect("insert");

        let results = store.search_messages("hello", None, 10).expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");

        let scoped = store.search_messages("hello", Some("bob"), 10).expect("search");
        assert_eq!(scoped.len(), 0);
    }

    #[test]
    fn list_conversations_groups_by_peer() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        store.insert_message(&chat_message("m2", "me", "alice", 2)).expect("insert");
        store.insert_message(&chat_message("m3", "bob", "me", 3)).expect("insert");

        let convos = store.list_conversations(false).expect("list");
        assert_eq!(convos.len(), 2);
        // Most recent first
        assert_eq!(convos[0].peer_hash, "bob");
        assert_eq!(convos[0].message_count, 1);
        assert_eq!(convos[1].peer_hash, "alice");
        assert_eq!(convos[1].message_count, 2);
    }

    #[test]
    fn list_conversations_last_content_is_most_recent() {
        let store = MessagesStore::in_memory().expect("store");
        let mut m1 = chat_message("m1", "alice", "me", 1);
        m1.content = "zzz first".to_string(); // Lexicographically greater
        store.insert_message(&m1).expect("insert");
        let mut m2 = chat_message("m2", "me", "alice", 2);
        m2.content = "aaa second".to_string(); // Lexicographically smaller but more recent
        store.insert_message(&m2).expect("insert");

        let convos = store.list_conversations(false).expect("list");
        assert_eq!(convos.len(), 1);
        assert_eq!(
            convos[0].last_message_content.as_deref(),
            Some("aaa second"),
            "should return most recent content, not lexicographic max"
        );
    }

    #[test]
    fn list_conversations_unread_only() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        store.mark_read("alice").expect("mark");
        store.insert_message(&chat_message("m2", "bob", "me", 2)).expect("insert");

        let convos = store.list_conversations(true).expect("list");
        assert_eq!(convos.len(), 1);
        assert_eq!(convos[0].peer_hash, "bob");
    }

    #[test]
    fn conversation_flags_are_independent_and_order_ties_are_stable() {
        let store = MessagesStore::in_memory().expect("store");
        let alice = "aa".repeat(16);
        let bob = "bb".repeat(16);
        let charlie = "cc".repeat(16);
        store.insert_message(&chat_message("z-last", &alice, "me", 10)).unwrap();
        store.insert_message(&chat_message("a-last", &alice, "me", 10)).unwrap();
        store.insert_message(&chat_message("bob", &bob, "me", 10)).unwrap();
        store.insert_message(&chat_message("charlie", &charlie, "me", 10)).unwrap();

        store.set_conversation_pinned(&charlie, true).unwrap();
        store.set_conversation_muted(&alice, true).unwrap();
        store.set_conversation_pinned(&alice, true).unwrap();
        store.set_conversation_pinned(&alice, false).unwrap();

        let conversations = store.list_conversations(false).unwrap();
        assert_eq!(
            conversations.iter().map(|item| item.peer_hash.as_str()).collect::<Vec<_>>(),
            vec![charlie.as_str(), alice.as_str(), bob.as_str()]
        );
        assert!(conversations[0].pinned);
        assert!(!conversations[0].muted);
        assert!(!conversations[1].pinned);
        assert!(conversations[1].muted);
        assert_eq!(conversations[1].last_message_content.as_deref(), Some("message z-last"));
        assert_eq!(conversations[1].unread_count, 2);
    }

    #[test]
    fn uppercase_historical_messages_join_canonical_state_and_queries() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "ab".repeat(16);
        let uppercase = peer.to_ascii_uppercase();
        store.insert_message(&chat_message("upper", &uppercase, "me", 10)).unwrap();
        store.set_conversation_pinned(&peer, true).unwrap();
        store.set_conversation_muted(&peer, true).unwrap();

        let summary = store.list_conversations(false).unwrap().remove(0);
        assert_eq!(summary.peer_hash, peer);
        assert!(summary.pinned);
        assert!(summary.muted);
        assert_eq!(store.list_messages_for_peer(&summary.peer_hash, 10, None).unwrap().len(), 1);
        assert_eq!(
            store.search_messages("message", Some(&summary.peer_hash), 10).unwrap().len(),
            1
        );
        assert_eq!(store.mark_read(&summary.peer_hash).unwrap(), 1);
    }

    #[test]
    fn drafts_are_bounded_replaceable_deletable_and_do_not_leak_into_messages() {
        let store = MessagesStore::in_memory().expect("store");
        let peer = "01".repeat(16);
        let draft = store.set_draft(&peer, "private draft").unwrap();
        assert_eq!(draft.content, "private draft");
        let replaced = store.set_draft(&peer, "replacement").unwrap();
        assert_eq!(store.draft(&peer).unwrap(), Some(replaced));
        assert!(store.search_messages("replacement", None, 10).unwrap().is_empty());
        assert!(store.list_conversations(false).unwrap().is_empty());
        assert_eq!(store.count_message_buckets().unwrap(), (0, 0));
        assert!(store.clear_draft(&peer).unwrap());
        assert!(store.draft(&peer).unwrap().is_none());

        assert!(store.set_draft(&peer, &"é".repeat(MAX_DRAFT_BYTES / 2 + 1)).is_err());
        for index in 0..MAX_RETAINED_DRAFTS {
            store.set_draft(&format!("{index:032x}"), "x").unwrap();
        }
        store.set_draft(&format!("{:032x}", 0), "still allowed").unwrap();
        assert!(store.set_draft(&"ff".repeat(16), "capacity failure").is_err());
        assert_eq!(store.draft(&format!("{:032x}", 0)).unwrap().unwrap().content, "still allowed");
    }

    #[test]
    fn draft_aggregate_failure_preserves_existing_value() {
        let store = MessagesStore::in_memory().expect("store");
        let full = "x".repeat(MAX_DRAFT_BYTES);
        for index in 0..(MAX_DRAFT_AGGREGATE_BYTES / MAX_DRAFT_BYTES) {
            store.set_draft(&format!("{index:032x}"), &full).unwrap();
        }
        let peer = format!("{:032x}", 0);
        store.set_draft(&peer, "replacement").unwrap();
        assert!(store.set_draft(&"fe".repeat(16), &full).is_err());
        assert_eq!(store.draft(&peer).unwrap().unwrap().content, "replacement");
    }

    #[test]
    fn concurrent_file_backed_draft_replacements_serialize() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("concurrent-drafts.db");
        let peer = "82".repeat(16);
        drop(MessagesStore::open(&path).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let replace = |content: &'static str| {
            let path = path.clone();
            let peer = peer.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = MessagesStore::open(&path).unwrap();
                barrier.wait();
                store.set_draft(&peer, content)
            })
        };
        let first = replace("first");
        let second = replace("second");
        barrier.wait();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        let content = MessagesStore::open(&path).unwrap().draft(&peer).unwrap().unwrap().content;
        assert!(["first", "second"].contains(&content.as_str()));
    }

    #[test]
    fn draft_schema_rejects_blob_and_legacy_blob_read_is_explicit_error() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "91".repeat(16);
        assert!(store
            .conn
            .execute(
                "INSERT INTO conversation_drafts (peer_hash, content, updated_at)
                 VALUES (?1, ?2, 1)",
                params![peer, vec![0xff_u8, 0xfe]],
            )
            .is_err());

        store.conn.pragma_update(None, "ignore_check_constraints", "ON").unwrap();
        store
            .conn
            .execute(
                "INSERT INTO conversation_drafts (peer_hash, content, updated_at)
                 VALUES (?1, ?2, 1)",
                params![peer, vec![0xff_u8, 0xfe]],
            )
            .unwrap();
        let error = store.draft(&peer).unwrap_err();
        assert!(error.to_string().contains("must be SQLite TEXT"), "{error}");
    }

    #[test]
    fn deleting_conversation_removes_state_and_draft() {
        let store = MessagesStore::in_memory().expect("store");
        let peer = "12".repeat(16);
        store.insert_message(&chat_message("old", &peer, "me", 1)).unwrap();
        store.set_conversation_pinned(&peer, true).unwrap();
        store.set_conversation_muted(&peer, true).unwrap();
        store.set_draft(&peer, "draft").unwrap();
        assert_eq!(store.delete_conversation(&peer).unwrap(), 1);
        assert!(store.draft(&peer).unwrap().is_none());

        store.insert_message(&chat_message("new", &peer, "me", 2)).unwrap();
        let summary = store.list_conversations(false).unwrap().remove(0);
        assert!(!summary.pinned);
        assert!(!summary.muted);
    }

    #[test]
    fn contacts_crud() {
        let store = MessagesStore::in_memory().expect("store");

        // Create
        let contact = store.set_contact("alice", Some("Alice"), Some("friend")).expect("set");
        assert_eq!(contact.peer_hash, "alice");
        assert_eq!(contact.alias.as_deref(), Some("Alice"));

        // List
        let contacts = store.list_contacts().expect("list");
        assert_eq!(contacts.len(), 1);

        // Update
        let updated = store.set_contact("alice", Some("Alice B"), None).expect("update");
        let contacts = store.list_contacts().expect("list");
        assert_eq!(contacts[0].alias.as_deref(), Some("Alice B"));
        assert_eq!(updated.created_at, contact.created_at);
        assert!(updated.updated_at > contact.updated_at);

        // Remove
        assert!(store.remove_contact("alice").expect("remove"));
        assert!(!store.remove_contact("alice").expect("remove again"));
        assert!(store.list_contacts().expect("list").is_empty());
    }

    #[test]
    fn contact_outcomes_preserve_identical_timestamps_and_enforce_utf8_byte_limits() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "a1".repeat(16);
        let created = store.set_contact_outcome(&peer, Some("Alice"), Some("notes")).unwrap();
        assert_eq!(created.disposition, MutationDisposition::Created);
        let original = created.contact.unwrap();
        let unchanged = store.set_contact_outcome(&peer, Some("Alice"), Some("notes")).unwrap();
        assert_eq!(unchanged.disposition, MutationDisposition::Unchanged);
        assert_eq!(unchanged.contact.unwrap(), original);
        let updated = store.set_contact_outcome(&peer, Some("Alice 2"), Some("notes")).unwrap();
        assert_eq!(updated.disposition, MutationDisposition::Updated);
        assert!(updated.contact.unwrap().updated_at > original.updated_at);
        assert!(store.set_contact_outcome(&peer, Some(&"é".repeat(129)), None).is_err());
        assert!(store.set_contact_outcome(&peer, None, Some(&"é".repeat(2049))).is_err());
    }

    #[test]
    fn mark_read_and_flags_are_authoritative_and_do_not_create_ghosts() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "b1".repeat(16);
        assert_eq!(
            store.mark_read_outcome(&peer).unwrap().disposition,
            MutationDisposition::NotFound
        );
        assert_eq!(
            store.set_conversation_flag_outcome(&peer, "pinned", true).unwrap().disposition,
            MutationDisposition::NotFound
        );
        let state_rows: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM conversation_state", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state_rows, 0);

        store.insert_message(&chat_message("unread", &peer, "me", 1)).unwrap();
        let applied = store.mark_read_outcome(&peer).unwrap();
        assert_eq!(applied.disposition, MutationDisposition::Applied);
        assert_eq!(applied.affected_count, 1);
        assert_eq!(applied.summary.unwrap().unread_count, 0);
        assert_eq!(
            store.mark_read_outcome(&peer).unwrap().disposition,
            MutationDisposition::Unchanged
        );
        let pinned = store.set_conversation_flag_outcome(&peer, "pinned", true).unwrap();
        assert!(pinned.summary.unwrap().pinned);
        let muted = store.set_conversation_flag_outcome(&peer, "muted", true).unwrap();
        let summary = muted.summary.unwrap();
        assert!(summary.pinned && summary.muted);
    }

    #[test]
    fn search_is_literal_bounded_ordered_and_reports_truncation() {
        let store = MessagesStore::in_memory().unwrap();
        let peer = "c1".repeat(16);
        for (id, content, timestamp) in [
            ("a", "literal 100%_\\ match", 2),
            ("b", "literal 100%_\\ match", 2),
            ("c", "literal 100xx match", 3),
        ] {
            let mut message = chat_message(id, &peer, "me", timestamp);
            message.content = content.into();
            store.insert_message(&message).unwrap();
        }
        let result = store.search_message_projection_outcome("100%_\\", None, 1).unwrap();
        assert_eq!(result.matched_count, 2);
        assert!(result.truncated);
        assert_eq!(result.items[0].message.id, "b");
        assert!(store.search_message_projection_outcome("", None, 1).is_err());
        assert!(store
            .search_message_projection_outcome(&"x".repeat(MAX_SEARCH_QUERY_BYTES + 1), None, 1)
            .is_err());
        assert!(store.search_message_projection_outcome("x", None, 0).is_err());
    }

    #[test]
    fn active_outbound_delete_conflicts_then_terminal_delete_is_idempotent() {
        let store = MessagesStore::in_memory().unwrap();
        store
            .insert_outbound_message(
                &outbound_message("active-delete", 1, Some("queued")),
                &outbound_route("active-delete", None),
            )
            .unwrap();
        let conflict = store.delete_message_outcome("active-delete").unwrap();
        assert_eq!(conflict.disposition, MutationDisposition::TerminalConflict);
        assert!(store.get_message("active-delete").unwrap().is_some());
        assert!(store.finish_outbound("active-delete", "failed", "failed").unwrap());
        assert_eq!(
            store.delete_message_outcome("active-delete").unwrap().disposition,
            MutationDisposition::Applied
        );
        assert_eq!(
            store.delete_message_outcome("active-delete").unwrap().disposition,
            MutationDisposition::NotFound
        );
    }

    #[test]
    fn v6_merges_valid_case_duplicate_contacts_and_preserves_invalid_rows() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("contact-migration.db");
        let peer = "ab".repeat(16);
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP INDEX idx_contacts_canonical_peer;
                     DROP TABLE conversation_drafts;
                     DROP TABLE conversation_state;
                     DELETE FROM schema_migrations
                     WHERE id = '2026-08-23-conversation-state-drafts-v6';",
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO contacts VALUES (?1, 'old', 'old notes', 5, 10)",
                    params![peer],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO contacts VALUES (?1, 'new', NULL, 7, 20)",
                    params![peer.to_ascii_uppercase()],
                )
                .unwrap();
            store
                .conn
                .execute("INSERT INTO contacts VALUES ('bad!', 'invalid', NULL, 1, 30)", [])
                .unwrap();
            store
                .conn
                .execute("INSERT INTO contacts VALUES ('BAD!', 'also invalid', NULL, 2, 31)", [])
                .unwrap();
        }

        let store = MessagesStore::open(&path).unwrap();
        let contacts = store.list_contacts().unwrap();
        let canonical = contacts.iter().find(|contact| contact.peer_hash == peer).unwrap();
        assert_eq!(canonical.alias.as_deref(), Some("new"));
        assert_eq!(canonical.notes, None);
        assert_eq!(canonical.created_at, 5);
        assert_eq!(canonical.updated_at, 20);
        assert!(contacts.iter().any(|contact| contact.peer_hash == "bad!"));
        assert!(contacts.iter().any(|contact| contact.peer_hash == "BAD!"));
        assert!(store
            .conn
            .execute(
                "INSERT INTO contacts VALUES (?1, NULL, NULL, 1, 1)",
                params![peer.to_ascii_uppercase()],
            )
            .is_err());
    }

    #[test]
    fn v6_skips_malformed_contact_types_without_blocking_startup() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("malformed-contacts.db");
        let blob_alias_peer = "c1".repeat(16);
        let negative_time_peer = "c2".repeat(16);
        let normal_peer = "c3".repeat(16);
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP INDEX idx_contacts_canonical_peer;
                     DROP TABLE conversation_drafts;
                     DROP TABLE conversation_state;
                     DELETE FROM schema_migrations
                     WHERE id IN (
                         '2026-08-23-conversation-state-drafts-v6',
                         '2026-08-23-conversation-schema-hardening-v7'
                     );",
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO contacts VALUES (?1, ?2, NULL, 1, 2)",
                    params![blob_alias_peer, vec![0xff_u8]],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO contacts VALUES (?1, 'Normal', 'safe', 3, 4)",
                    params![normal_peer],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO contacts VALUES (?1, 'negative', NULL, -1, 2)",
                    params![negative_time_peer],
                )
                .unwrap();
        }

        let store = MessagesStore::open(&path).unwrap();
        let blob_type: String = store
            .conn
            .query_row(
                "SELECT typeof(alias) FROM contacts WHERE peer_hash = ?1",
                params![blob_alias_peer],
                |row| row.get(0),
            )
            .unwrap();
        let created_at: i64 = store
            .conn
            .query_row(
                "SELECT created_at FROM contacts WHERE peer_hash = ?1",
                params![negative_time_peer],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(blob_type, "blob");
        assert_eq!(created_at, -1);
        let contacts = store.list_contacts().unwrap();
        let blob_alias =
            contacts.iter().find(|contact| contact.peer_hash == blob_alias_peer).unwrap();
        assert_eq!(blob_alias.alias, None);
        assert_eq!(blob_alias.created_at, 1);
        let negative =
            contacts.iter().find(|contact| contact.peer_hash == negative_time_peer).unwrap();
        assert_eq!(negative.alias.as_deref(), Some("negative"));
        assert_eq!(negative.created_at, 0);
        let normal = contacts.iter().find(|contact| contact.peer_hash == normal_peer).unwrap();
        assert_eq!(normal.alias.as_deref(), Some("Normal"));
        assert_eq!(normal.notes.as_deref(), Some("safe"));
    }

    #[test]
    fn absent_v6_marker_rebuilds_partial_tables_with_exact_constraints() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("partial-v6.db");
        let peer = "d0".repeat(16);
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TABLE conversation_drafts;
                     DROP TABLE conversation_state;
                     CREATE TABLE conversation_state (
                         peer_hash TEXT PRIMARY KEY, pinned, muted, updated_at
                     );
                     CREATE TABLE conversation_drafts (
                         peer_hash TEXT PRIMARY KEY, content, updated_at
                     );
                     DELETE FROM schema_migrations
                     WHERE id IN (
                         '2026-08-23-conversation-state-drafts-v6',
                         '2026-08-23-conversation-schema-hardening-v7'
                     );",
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO conversation_state VALUES (?1, 1, 0, 10)",
                    params![peer.to_ascii_uppercase()],
                )
                .unwrap();
            store
                .conn
                .execute("INSERT INTO conversation_state VALUES (?1, 0, 1, 20)", params![peer])
                .unwrap();
            store
                .conn
                .execute("INSERT INTO conversation_state VALUES ('invalid', 2, 0, 30)", [])
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO conversation_drafts VALUES (?1, 'old', 10)",
                    params![peer.to_ascii_uppercase()],
                )
                .unwrap();
            store
                .conn
                .execute("INSERT INTO conversation_drafts VALUES (?1, 'latest', 20)", params![peer])
                .unwrap();
            store
                .conn
                .execute("INSERT INTO conversation_drafts VALUES ('weak', NULL, NULL)", [])
                .unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        assert!(conversation_schema_is_valid(&store.conn).unwrap());
        let state: (i64, i64, i64) = store
            .conn
            .query_row(
                "SELECT pinned, muted, updated_at FROM conversation_state WHERE peer_hash = ?1",
                params![peer],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (0, 1, 20));
        assert_eq!(store.draft(&peer).unwrap().unwrap().content, "latest");
        assert!(store
            .conn
            .execute("INSERT INTO conversation_drafts VALUES (NULL, 'draft', 1)", [],)
            .is_err());
        assert!(store
            .conn
            .execute(
                "INSERT INTO conversation_drafts VALUES (?1, NULL, 1)",
                params!["d1".repeat(16)],
            )
            .is_err());
        assert!(store
            .conn
            .execute(
                "INSERT INTO conversation_drafts VALUES (?1, ?2, 1)",
                params!["d2".repeat(16), "x".repeat(MAX_DRAFT_BYTES + 1)],
            )
            .is_err());
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM conversation_drafts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn v7_repairs_marker_present_weak_schema() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("weak-v7.db");
        let peer = "d4".repeat(16);
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP TABLE conversation_drafts;
                     DROP TABLE conversation_state;
                     CREATE TABLE conversation_state (
                         peer_hash TEXT PRIMARY KEY, pinned, muted, updated_at
                     );
                     CREATE TABLE conversation_drafts (
                         peer_hash TEXT PRIMARY KEY, content, updated_at
                     );
                     DELETE FROM schema_migrations
                     WHERE id = '2026-08-23-conversation-schema-hardening-v7';",
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO conversation_state VALUES (?1, 1, 1, 40)",
                    params![peer.to_ascii_uppercase()],
                )
                .unwrap();
            store
                .conn
                .execute("INSERT INTO conversation_state VALUES ('invalid', 3, 0, 50)", [])
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO conversation_drafts VALUES (?1, 'preserved', 40)",
                    params![peer.to_ascii_uppercase()],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO conversation_drafts VALUES ('invalid', ?1, 50)",
                    params![vec![0xff_u8]],
                )
                .unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        assert!(conversation_schema_is_valid(&store.conn).unwrap());
        let state: (i64, i64) = store
            .conn
            .query_row(
                "SELECT pinned, muted FROM conversation_state WHERE peer_hash = ?1",
                params![peer],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, (1, 1));
        assert_eq!(store.draft(&peer).unwrap().unwrap().content, "preserved");
        let marker: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations
                 WHERE id = '2026-08-23-conversation-schema-hardening-v7')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(marker);
    }

    #[test]
    fn v7_repairs_each_missing_peer_constraint() {
        let variants = [
            (
                "not-null",
                "peer_hash TEXT PRIMARY KEY
                 CHECK(length(peer_hash) = 32)
                 CHECK(peer_hash = lower(peer_hash))
                 CHECK(peer_hash NOT GLOB '*[^0-9a-f]*')",
            ),
            (
                "length",
                "peer_hash TEXT NOT NULL PRIMARY KEY
                 CHECK(peer_hash = lower(peer_hash))
                 CHECK(peer_hash NOT GLOB '*[^0-9a-f]*')",
            ),
            (
                "lowercase",
                "peer_hash TEXT NOT NULL PRIMARY KEY
                 CHECK(length(peer_hash) = 32)
                 CHECK(peer_hash NOT GLOB '*[^0-9a-f]*')",
            ),
            (
                "hex",
                "peer_hash TEXT NOT NULL PRIMARY KEY
                 CHECK(length(peer_hash) = 32)
                 CHECK(peer_hash = lower(peer_hash))",
            ),
        ];
        for (name, peer_column) in variants {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(format!("weak-{name}.db"));
            {
                let store = MessagesStore::open(&path).unwrap();
                store
                    .conn
                    .execute_batch(&format!(
                        "DROP TABLE conversation_drafts;
                         DROP TABLE conversation_state;
                         CREATE TABLE conversation_state (
                             {peer_column},
                             pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
                             muted INTEGER NOT NULL DEFAULT 0 CHECK(muted IN (0, 1)),
                             updated_at INTEGER NOT NULL
                         );
                         CREATE TABLE conversation_drafts (
                             {peer_column},
                             content TEXT NOT NULL
                                 CHECK(typeof(content) = 'text')
                                 CHECK(length(CAST(content AS BLOB)) <= 65536),
                             updated_at INTEGER NOT NULL
                         );
                         DELETE FROM schema_migrations
                         WHERE id = '2026-08-23-conversation-schema-hardening-v7';"
                    ))
                    .unwrap();
            }
            let store = MessagesStore::open(&path).unwrap();
            assert!(conversation_schema_is_valid(&store.conn).unwrap(), "variant {name}");
        }
    }

    #[test]
    fn concurrent_v6_open_rechecks_marker_inside_immediate_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("concurrent-v6.db");
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .conn
                .execute_batch(
                    "DROP INDEX idx_contacts_canonical_peer;
                     DROP TABLE conversation_drafts;
                     DROP TABLE conversation_state;
                     CREATE TABLE conversation_state (
                         peer_hash TEXT PRIMARY KEY, pinned INTEGER NOT NULL,
                         muted INTEGER NOT NULL, updated_at INTEGER NOT NULL
                     );
                     DELETE FROM schema_migrations
                     WHERE id = '2026-08-23-conversation-state-drafts-v6';",
                )
                .unwrap();
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let open = || {
            let barrier = barrier.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                barrier.wait();
                MessagesStore::open(&path)
            })
        };
        let first = open();
        let second = open();
        barrier.wait();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());

        let store = MessagesStore::open(&path).unwrap();
        let marker_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations
                 WHERE id = '2026-08-23-conversation-state-drafts-v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 1);
        let drafts_exist: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                 WHERE type = 'table' AND name = 'conversation_drafts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(drafts_exist);
    }

    #[test]
    fn conversation_migration_preserves_authoritative_records_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pre-conversation-v6.db");
        let peer = "34".repeat(16);
        let route = OutboundRouteRecord {
            message_id: "outbound".into(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "packet".into(),
            fallback_reason: None,
            correlation_id: "migration-attempt".into(),
            retry_of: None,
            deadline_unix_ms: 100,
            state: "sending".into(),
            attempt_count: 0,
        };
        {
            let store = MessagesStore::open(&path).unwrap();
            let mut inbound = chat_message("inbound", &peer, "me", 1);
            inbound.read = true;
            store.insert_message(&inbound).unwrap();
            store
                .insert_outbound_message(&outbound_message("outbound", 2, Some("sending")), &route)
                .unwrap();
            store
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: "outbound".into(),
                    attempt_number: 1,
                    started_unix_ms: 10,
                    deadline_unix_ms: 100,
                    state: "sending".into(),
                })
                .unwrap();
            store.set_contact(&peer, Some("Peer"), Some("preserve")).unwrap();
            store
                .insert_canonical_inbound_if_absent(
                    &chat_message("canonical", &peer, "me", 3),
                    &CanonicalInboundRecord {
                        message_id: "canonical".into(),
                        source: [3; 16],
                        destination: [4; 16],
                        title: b"title".to_vec(),
                        content: b"content".to_vec(),
                        timestamp: 3.5,
                        fields_msgpack: None,
                        signature: None,
                        stamp: None,
                        wire: vec![5; 64],
                        authentication_state: "unknown_identity".into(),
                        stamp_state: "unknown".into(),
                        stamp_value: None,
                        stamp_target: None,
                    },
                )
                .unwrap();
            store.propagation_ingest(&peer, b"route-like-payload", Some(&peer), 60).unwrap();
            store
                .upsert_lxmf_ticket(&LxmfTicketRecord {
                    peer: peer.clone(),
                    ticket: vec![7; lxmf::stamps::TICKET_LENGTH],
                    expires_at: i64::MAX,
                    direction: "received".into(),
                })
                .unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE conversation_drafts;
                 DROP TABLE conversation_state;
                 DELETE FROM schema_migrations
                 WHERE id = '2026-08-23-conversation-state-drafts-v6';",
            )
            .unwrap();
        }

        {
            let store = MessagesStore::open(&path).unwrap();
            assert!(store.get_message("inbound").unwrap().unwrap().read);
            assert_eq!(store.list_contacts().unwrap()[0].alias.as_deref(), Some("Peer"));
            assert_eq!(store.outbound_attempts("outbound").unwrap().len(), 1);
            assert_eq!(store.canonical_inbound("canonical").unwrap().unwrap().timestamp, 3.5);
            assert_eq!(store.propagation_stats().unwrap().0, 1);
            assert!(store.active_lxmf_ticket(&peer, "received", 1).unwrap().is_some());
            let marker_count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations
                 WHERE id = '2026-08-23-conversation-state-drafts-v6'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(marker_count, 1);
        }
        let reopened = MessagesStore::open(&path).unwrap();
        let marker_count: i64 = reopened
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations
             WHERE id = '2026-08-23-conversation-state-drafts-v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 1);
        assert_eq!(reopened.outbound_attempts("outbound").unwrap().len(), 1);
    }

    #[test]
    fn file_backed_conversation_projection_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("aggregate.db");
        let peer = "56".repeat(16);
        {
            let store = MessagesStore::open(&path).unwrap();
            store.insert_message(&chat_message("in", &peer, "me", 1)).unwrap();
            store.mark_read(&peer).unwrap();
            store.set_contact(&peer, Some("Restart"), None).unwrap();
            store.set_conversation_pinned(&peer, true).unwrap();
            store.set_conversation_muted(&peer, true).unwrap();
            store.set_draft(&peer, "resume here").unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        let summary = store.list_conversations(false).unwrap().remove(0);
        assert_eq!(summary.unread_count, 0);
        assert!(summary.pinned);
        assert!(summary.muted);
        assert_eq!(store.list_contacts().unwrap()[0].alias.as_deref(), Some("Restart"));
        assert_eq!(store.draft(&peer).unwrap().unwrap().content, "resume here");
    }

    #[cfg(unix)]
    #[test]
    fn file_backed_store_enforces_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("permissions.db");
        std::fs::write(&path, []).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        let store = MessagesStore::open(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        store.insert_message(&outbound_message("permissions", 1, None)).unwrap();
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            if sidecar.exists() {
                assert_eq!(std::fs::metadata(sidecar).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }
    }

    #[test]
    fn list_messages_for_peer_filters_correctly() {
        let store = MessagesStore::in_memory().expect("store");
        store.insert_message(&chat_message("m1", "alice", "me", 1)).expect("insert");
        store.insert_message(&chat_message("m2", "me", "alice", 2)).expect("insert");
        store.insert_message(&chat_message("m3", "bob", "me", 3)).expect("insert");

        let msgs = store.list_messages_for_peer("alice", 10, None).expect("list");
        assert_eq!(msgs.len(), 2);
        // Most recent first
        assert_eq!(msgs[0].id, "m2");
        assert_eq!(msgs[1].id, "m1");
    }

    #[test]
    fn list_messages_for_peer_orders_equal_timestamps_deterministically() {
        let store = MessagesStore::in_memory().expect("store");
        for index in 1..=100 {
            store
                .insert_message(&chat_message(&format!("m{index:03}"), "alice", "me", 42))
                .expect("insert");
        }

        let msgs = store.list_messages_for_peer("alice", 100, None).expect("list");
        let ids: Vec<_> = msgs.iter().map(|message| message.id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"m100"));
        assert_eq!(ids.last(), Some(&"m001"));
        assert_eq!(ids.len(), 100);

        let snapshots =
            store.message_projection_snapshot_for_peer("alice", 100, None).expect("snapshot");
        let snapshot_ids: Vec<_> =
            snapshots.iter().map(|snapshot| snapshot.message.id.as_str()).collect();
        assert_eq!(snapshot_ids, ids);
    }

    // ── Blocklist tests ─────────────────────────────────────────────────

    #[test]
    fn block_and_unblock_peer() {
        let store = MessagesStore::in_memory().expect("store");
        assert!(!store.is_blocked("abc123").unwrap());
        assert!(store.block_peer("abc123").unwrap()); // newly blocked
        assert!(store.is_blocked("abc123").unwrap());
        assert!(!store.block_peer("abc123").unwrap()); // already blocked
        assert!(store.unblock_peer("abc123").unwrap()); // was blocked
        assert!(!store.is_blocked("abc123").unwrap());
        assert!(!store.unblock_peer("abc123").unwrap()); // wasn't blocked
    }

    #[test]
    fn blocked_peers_list() {
        let store = MessagesStore::in_memory().expect("store");
        store.block_peer("peer_a").unwrap();
        store.block_peer("peer_b").unwrap();
        store.block_peer("peer_c").unwrap();
        let blocked = store.blocked_peers().unwrap();
        assert_eq!(blocked.len(), 3);
        assert!(blocked.contains(&"peer_a".to_string()));
        assert!(blocked.contains(&"peer_b".to_string()));
        assert!(blocked.contains(&"peer_c".to_string()));
        store.unblock_peer("peer_b").unwrap();
        let blocked = store.blocked_peers().unwrap();
        assert_eq!(blocked.len(), 2);
        assert!(!blocked.contains(&"peer_b".to_string()));
    }

    #[test]
    fn propagation_ingest_and_fetch_roundtrip() {
        let store = MessagesStore::in_memory().expect("store");
        let payload = b"lxmf-packet-1";
        assert!(store.propagation_ingest("dest-a", payload, Some("src-a"), 60).expect("ingest"));
        let fetched = store.propagation_fetch("dest-a").expect("fetch");
        assert_eq!(fetched, vec![payload.to_vec()]);
    }

    #[test]
    fn propagation_deduplicates_identical_packets() {
        let store = MessagesStore::in_memory().expect("store");
        let payload = b"same-packet";
        assert!(store.propagation_ingest("dest-a", payload, None, 60).expect("first ingest"));
        assert!(!store.propagation_ingest("dest-a", payload, None, 60).expect("second ingest"));
        let stats = store.propagation_stats().expect("stats");
        assert_eq!(stats.0, 1);
    }

    #[test]
    fn propagation_expire_removes_stale_packets() {
        let store = MessagesStore::in_memory().expect("store");
        store.propagation_ingest("dest-a", b"stale-packet", None, 0).expect("ingest");
        let deleted = store.propagation_expire().expect("expire");
        assert_eq!(deleted, 1);
        assert!(store.propagation_fetch("dest-a").expect("fetch").is_empty());
    }

    #[test]
    fn propagation_delete_removes_delivered_packets() {
        let store = MessagesStore::in_memory().expect("store");
        store.propagation_ingest("dest-a", b"packet-1", None, 60).expect("ingest one");
        store.propagation_ingest("dest-a", b"packet-2", None, 60).expect("ingest two");
        let fetched = store.propagation_fetch_with_ids("dest-a").expect("fetch with ids");
        let ids: Vec<String> = fetched.into_iter().map(|(id, _)| id).collect();
        store.propagation_delete(&ids).expect("delete");
        let stats = store.propagation_stats().expect("stats");
        assert_eq!(stats, (0, 0));
    }

    #[test]
    fn canonical_lxmf_migration_policy_tickets_and_cost_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("canonical-lxmf.sqlite");
        let projection = chat_message("canonical", "peer", "local", 10);
        let canonical = CanonicalInboundRecord {
            message_id: projection.id.clone(),
            source: [1; 16],
            destination: [2; 16],
            title: vec![0xfe],
            content: vec![0xff, 0x00],
            timestamp: 10.75,
            fields_msgpack: Some(rmp_serde::to_vec(&rmpv::Value::Binary(vec![0x80])).unwrap()),
            signature: Some(vec![3; 64]),
            stamp: Some(vec![4; 8]),
            wire: vec![5; 128],
            authentication_state: "unknown_identity".into(),
            stamp_state: "verified".into(),
            stamp_value: Some(8),
            stamp_target: Some(8),
        };
        {
            let store = MessagesStore::open(&path).unwrap();
            assert!(store.insert_canonical_inbound_if_absent(&projection, &canonical).unwrap());
            store
                .set_lxmf_stamp_policy(LxmfStampPolicy { target_cost: 8, flexibility: 2 })
                .unwrap();
            store.learn_peer_stamp_cost("peer", 7, 100).unwrap();
            store
                .upsert_lxmf_ticket(&LxmfTicketRecord {
                    peer: "peer".into(),
                    ticket: vec![6; lxmf::stamps::TICKET_LENGTH],
                    expires_at: 200,
                    direction: "received".into(),
                })
                .unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        assert_eq!(store.canonical_inbound("canonical").unwrap(), Some(canonical));
        assert_eq!(
            store.lxmf_stamp_policy().unwrap(),
            LxmfStampPolicy { target_cost: 8, flexibility: 2 }
        );
        assert_eq!(store.peer_stamp_cost("peer").unwrap(), Some(7));
        assert!(store.active_lxmf_ticket("peer", "received", 199).unwrap().is_some());
        assert_eq!(store.expire_lxmf_tickets(200).unwrap(), 1);
        assert!(store.active_lxmf_ticket("peer", "received", 200).unwrap().is_none());
    }

    #[test]
    fn v9_attachment_blobs_deduplicate_verify_and_gc_shared_content() {
        let store = MessagesStore::in_memory().expect("store");
        let attachment = AttachmentBlobInput {
            wire_name: "empty.bin".into(),
            data: Vec::new(),
            content_type: Some("application/octet-stream".into()),
            source: "canonical_binary".into(),
        };
        for id in ["attachment-a", "attachment-b"] {
            let projection = chat_message(id, "alice", "me", 10);
            let canonical = CanonicalInboundRecord {
                message_id: id.into(),
                source: [1; 16],
                destination: [2; 16],
                title: Vec::new(),
                content: Vec::new(),
                timestamp: 10.0,
                fields_msgpack: None,
                signature: Some(vec![3; 64]),
                stamp: None,
                wire: vec![4; 128],
                authentication_state: "verified".into(),
                stamp_state: "not_applicable".into(),
                stamp_value: None,
                stamp_target: None,
            };
            assert!(store
                .insert_canonical_inbound_with_attachments(
                    &projection,
                    &canonical,
                    std::slice::from_ref(&attachment),
                    None,
                )
                .expect("insert attachment"));
        }
        let blob_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM attachment_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(blob_count, 1);
        let info = store.list_message_attachments("attachment-a").unwrap();
        assert_eq!(
            hex::encode(info[0].digest),
            sha2::Sha256::digest([]).iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        );
        let chunk = store
            .query_attachment_chunk("attachment-a", 0, 0, 1)
            .unwrap()
            .expect("empty attachment");
        assert!(chunk.data.is_empty());
        assert_eq!(chunk.next_offset, 0);
        assert!(chunk.done);

        assert!(store.delete_message("attachment-a").unwrap());
        let blob_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM attachment_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(blob_count, 1);
        assert!(store.delete_message("attachment-b").unwrap());
        let blob_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM attachment_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(blob_count, 0);
    }

    #[test]
    fn attachment_integrity_failure_exposes_no_bytes() {
        let store = MessagesStore::in_memory().expect("store");
        let projection = chat_message("corrupt-attachment", "alice", "me", 10);
        let canonical = CanonicalInboundRecord {
            message_id: projection.id.clone(),
            source: [1; 16],
            destination: [2; 16],
            title: Vec::new(),
            content: Vec::new(),
            timestamp: 10.0,
            fields_msgpack: None,
            signature: Some(vec![3; 64]),
            stamp: None,
            wire: vec![4; 128],
            authentication_state: "verified".into(),
            stamp_state: "not_applicable".into(),
            stamp_value: None,
            stamp_target: None,
        };
        store
            .insert_canonical_inbound_with_attachments(
                &projection,
                &canonical,
                &[AttachmentBlobInput {
                    wire_name: "value.bin".into(),
                    data: vec![1, 2, 3],
                    content_type: None,
                    source: "canonical_binary".into(),
                }],
                None,
            )
            .unwrap();
        store.conn.execute("UPDATE attachment_blobs SET data = x'000000'", []).unwrap();
        assert!(store.query_attachment_chunk("corrupt-attachment", 0, 0, 3).unwrap().is_none());
        let state: String = store
            .conn
            .query_row("SELECT state FROM attachment_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state, "integrity_failed");
    }

    #[test]
    fn transfer_checksum_projection_uses_persisted_terminal_verification_only() {
        let store = MessagesStore::in_memory().unwrap();
        let message = outbound_message("checksum-transfer", 1, Some("queued"));
        let route = OutboundRouteRecord {
            message_id: message.id.clone(),
            requested_method: "direct".into(),
            actual_method: "direct".into(),
            representation: "resource".into(),
            fallback_reason: None,
            correlation_id: "checksum-transfer".into(),
            retry_of: None,
            deadline_unix_ms: 10,
            state: "queued".into(),
            attempt_count: 0,
        };
        store
            .insert_outbound_message_with_attachments(
                &message,
                &route,
                None,
                &[AttachmentBlobInput {
                    wire_name: "checksum.bin".into(),
                    data: vec![1, 2, 3],
                    content_type: None,
                    source: "local".into(),
                }],
                128,
            )
            .unwrap();
        assert!(!store.list_message_attachments(&message.id).unwrap()[0].checksum_verified);
        assert!(store.finish_outbound(&message.id, "failed", "failed: integrity").unwrap());
        let failed = store.list_message_attachments(&message.id).unwrap();
        assert_eq!(failed[0].transfer_state.as_deref(), Some("failed"));
        assert!(!failed[0].checksum_verified);
    }

    #[test]
    fn integrity_failed_referenced_blobs_still_consume_distinct_blob_quota() {
        let store = MessagesStore::in_memory().expect("store");
        let transaction = store.conn.unchecked_transaction().unwrap();
        for index in 0..MAX_ATTACHMENT_BLOB_COUNT {
            let message_id = format!("quota-{index}");
            let mut digest = [0u8; 32];
            digest[..8].copy_from_slice(&(index as u64).to_be_bytes());
            transaction
                .execute(
                    "INSERT INTO messages
                     (id, source, destination, title, content, timestamp, direction, fields,
                      receipt_status, read)
                     VALUES (?1, 'source', 'destination', '', '', 1, 'in', NULL, NULL, 0)",
                    params![&message_id],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO attachment_blobs
                     (digest, byte_len, data, state, created_at, verified_at)
                     VALUES (?1, 0, x'', 'integrity_failed', 1, NULL)",
                    params![digest.as_slice()],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO message_attachments
                     (message_id, ordinal, digest, wire_name, content_type, source)
                     VALUES (?1, 0, ?2, 'failed.bin', NULL, 'local')",
                    params![&message_id, digest.as_slice()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();

        let projection = chat_message("quota-new", "alice", "me", 10);
        let canonical = CanonicalInboundRecord {
            message_id: projection.id.clone(),
            source: [1; 16],
            destination: [2; 16],
            title: Vec::new(),
            content: Vec::new(),
            timestamp: 10.0,
            fields_msgpack: None,
            signature: Some(vec![3; 64]),
            stamp: None,
            wire: vec![4; 128],
            authentication_state: "verified".into(),
            stamp_state: "not_applicable".into(),
            stamp_value: None,
            stamp_target: None,
        };
        let error = store
            .insert_canonical_inbound_with_attachments(
                &projection,
                &canonical,
                &[AttachmentBlobInput {
                    wire_name: "new.bin".into(),
                    data: vec![1],
                    content_type: None,
                    source: "local".into(),
                }],
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains("quota"));
        assert!(store.get_message("quota-new").unwrap().is_none());
    }

    #[test]
    fn v9_restart_fails_nonterminal_transfer_and_repairs_corrupt_and_orphan_blobs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("attachments-v9.sqlite");
        {
            let store = MessagesStore::open(&path).unwrap();
            let projection = chat_message("restart-attachment", "alice", "me", 10);
            let canonical = CanonicalInboundRecord {
                message_id: projection.id.clone(),
                source: [1; 16],
                destination: [2; 16],
                title: Vec::new(),
                content: Vec::new(),
                timestamp: 10.0,
                fields_msgpack: None,
                signature: Some(vec![3; 64]),
                stamp: None,
                wire: vec![4; 128],
                authentication_state: "verified".into(),
                stamp_state: "not_applicable".into(),
                stamp_value: None,
                stamp_target: None,
            };
            store
                .insert_canonical_with_attachments_and_ticket(
                    &projection,
                    &canonical,
                    None,
                    &[AttachmentBlobInput {
                        wire_name: "restart.bin".into(),
                        data: vec![1, 2, 3],
                        content_type: None,
                        source: "canonical_binary".into(),
                    }],
                    None,
                )
                .unwrap();
            store
                .conn
                .execute(
                    "UPDATE attachment_transfers SET state = 'transferring', transferred = 1,
                     checksum_verified = 0",
                    [],
                )
                .unwrap();
            store.conn.execute("UPDATE attachment_blobs SET data = x'000000'", []).unwrap();
            let orphan = [0x55_u8; 32];
            store
                .conn
                .execute(
                    "INSERT INTO attachment_blobs
                     (digest, byte_len, data, state, created_at, verified_at)
                     VALUES (?1, 0, x'', 'verified', 10, 10)",
                    params![orphan.as_slice()],
                )
                .unwrap();
        }
        let store = MessagesStore::open(&path).unwrap();
        let (state, error): (String, Option<String>) = store
            .conn
            .query_row("SELECT state, error FROM attachment_transfers", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(state, "failed");
        assert_eq!(error.as_deref(), Some("daemon_restarted"));
        let states: Vec<String> = store
            .conn
            .prepare("SELECT state FROM attachment_blobs")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(states, ["integrity_failed"]);
    }

    #[test]
    fn v9_marker_with_malformed_schema_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("malformed-v9.sqlite");
        drop(MessagesStore::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE message_attachments;
                 CREATE TABLE message_attachments (message_id TEXT PRIMARY KEY);",
            )
            .unwrap();
        drop(connection);
        let error = MessagesStore::open(&path).err().expect("malformed v9 must fail");
        assert!(error.to_string().contains("malformed schema"));
    }

    #[test]
    fn v9_strict_types_reject_text_hash_blob_ids_and_real_counters() {
        let store = MessagesStore::in_memory().unwrap();
        let message = chat_message("strict-types", "alice", "me", 1);
        store.insert_message(&message).unwrap();
        assert!(store
            .conn
            .execute(
                "INSERT INTO attachment_transfers
                 (message_id, transfer_id, resource_hash, representation, direction, state,
                  transferred, total, checksum_verified, error, updated_at)
                 VALUES (?1, 'transfer', ?2, 'resource', 'outbound', 'transferring',
                         0, 1, 0, NULL, 1)",
                params![&message.id, "0".repeat(32)],
            )
            .is_err());
        assert!(store
            .conn
            .execute(
                "INSERT INTO attachment_issues (message_id, reason, created_at)
                 VALUES (?1, 'invalid', 1)",
                params![vec![0x61_u8; 8]],
            )
            .is_err());
        assert!(store
            .conn
            .execute(
                "INSERT INTO attachment_transfers
                 (message_id, transfer_id, representation, direction, state, transferred,
                  total, checksum_verified, error, updated_at)
                 VALUES (?1, 'transfer', 'resource', 'outbound', 'transferring',
                         0.5, 1.0, 0, NULL, 1.0)",
                params![&message.id],
            )
            .is_err());
    }

    #[test]
    fn v13_migrates_v12_additively_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("message-inspection-v13.sqlite");
        drop(MessagesStore::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "DROP INDEX idx_message_delivery_evidence_terminal;
                 DROP INDEX idx_message_delivery_evidence_message;
                 DROP TABLE message_delivery_evidence;
                 DROP TABLE canonical_inbound_inspection;
                 DROP TABLE outbound_message_inspection;
                 DELETE FROM schema_migrations
                  WHERE id = '2026-08-25-authoritative-message-inspection-v13';
                 INSERT INTO messages
                  (id, source, destination, title, content, timestamp, direction, fields,
                   receipt_status, read)
                  VALUES ('v12-message', 'source', 'destination', '', 'preserved', 7,
                          'in', NULL, NULL, 0);",
            )
            .unwrap();
        drop(connection);

        for _ in 0..2 {
            let store = MessagesStore::open(&path).unwrap();
            assert_eq!(store.get_message("v12-message").unwrap().unwrap().content, "preserved");
            assert!(message_inspection_schema_is_valid(&store.conn).unwrap());
            let markers: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE id = ?1",
                    params![MESSAGE_INSPECTION_MIGRATION],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(markers, 1);
        }
    }

    #[test]
    fn v13_stamp_target_and_terminal_evidence_survive_restart_and_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("message-evidence-v13.sqlite");
        let message_id = "ab".repeat(32);
        {
            let store = MessagesStore::open(&path).unwrap();
            let projection = MessageRecord {
                id: message_id.clone(),
                source: "11".repeat(16),
                destination: "22".repeat(16),
                title: String::new(),
                content: "stamp target".into(),
                timestamp: 1,
                direction: "in".into(),
                fields: None,
                receipt_status: None,
                read: false,
            };
            let canonical = CanonicalInboundRecord {
                message_id: message_id.clone(),
                source: [0x11; 16],
                destination: [0x22; 16],
                title: Vec::new(),
                content: b"stamp target".to_vec(),
                timestamp: 1.0,
                fields_msgpack: None,
                signature: None,
                stamp: None,
                wire: vec![0; 128],
                authentication_state: "unknown_identity".into(),
                stamp_state: "not_applicable".into(),
                stamp_value: None,
                stamp_target: Some(17),
            };
            store.insert_canonical_inbound_if_absent(&projection, &canonical).unwrap();
            assert!(store.update_authentication_state(&message_id, "verified").unwrap());
        }
        let store = MessagesStore::open(&path).unwrap();
        let canonical = store.canonical_inbound(&message_id).unwrap().unwrap();
        assert_eq!(canonical.stamp_target, Some(17));
        assert_eq!(canonical.authentication_state, "verified");

        let outbound_id = "cd".repeat(32);
        let message = outbound_message(&outbound_id, 2, Some("queued"));
        store.insert_outbound_message(&message, &outbound_route(&outbound_id, None)).unwrap();
        assert!(store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: outbound_id.clone(),
                attempt_number: 1,
                started_unix_ms: 1,
                deadline_unix_ms: i64::MAX,
                state: "sending".into(),
            })
            .unwrap());
        assert!(!store
            .track_outbound_evidence(&"ef".repeat(32), &outbound_id, "resource")
            .unwrap());
        for index in 0..20_u64 {
            assert!(store
                .track_outbound_evidence(&format!("{index:064x}"), &outbound_id, "packet")
                .unwrap());
        }
        assert!(store
            .begin_outbound_attempt(&OutboundAttemptRecord {
                message_id: outbound_id.clone(),
                attempt_number: 2,
                started_unix_ms: 2,
                deadline_unix_ms: i64::MAX,
                state: "sending".into(),
            })
            .unwrap());
        for index in 20..40_u64 {
            assert!(store
                .track_outbound_evidence(&format!("{index:064x}"), &outbound_id, "packet")
                .unwrap());
        }
        assert_eq!(store.message_delivery_evidence(&outbound_id).unwrap().len(), 32);
        let correlations: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM outbound_evidence WHERE message_id = ?1",
                params![outbound_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(correlations, 32);
        assert!(store
            .finish_outbound_with_exact_evidence(
                &outbound_id,
                "delivered",
                "delivered: packet-receipt",
                Some("authenticated packet receipt"),
                &format!("{:064x}", 39),
                "packet",
            )
            .unwrap());
        drop(store);

        let store = MessagesStore::open(&path).unwrap();
        let evidence = store.message_delivery_evidence(&outbound_id).unwrap();
        assert_eq!(evidence.len(), 32);
        assert!(evidence.iter().all(|item| item.kind == "packet_receipt"));
        assert_eq!(evidence.iter().filter(|item| item.state == "completed").count(), 1);
        assert_eq!(evidence.iter().filter(|item| item.state == "failed").count(), 31);
        let completed = evidence.iter().find(|item| item.state == "completed").unwrap();
        assert_eq!(completed.evidence_hash, format!("{:064x}", 39));
        assert_eq!(completed.attempt_number, Some(2));
        assert!(evidence
            .iter()
            .any(|item| item.attempt_number == Some(1) && item.state == "failed"));
        assert_eq!(completed.outcome.as_deref(), Some("authenticated packet receipt"));
        assert_eq!(
            store.outbound_terminal_detail(&outbound_id).unwrap().as_deref(),
            Some("authenticated packet receipt")
        );
    }

    #[test]
    fn v13_marker_rejects_schema_type_or_check_collision() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("message-inspection-collision.sqlite");
        let store = MessagesStore::open(&path).unwrap();
        store
            .conn
            .execute(
                "DELETE FROM schema_migrations WHERE id = ?1",
                params![MESSAGE_INSPECTION_MIGRATION],
            )
            .unwrap();
        drop(store);
        let error = match MessagesStore::open(&path) {
            Ok(_) => panic!("v13 collision unexpectedly opened"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("collision"));
    }

    #[test]
    fn canonical_debug_never_exposes_wire_or_payload_bytes() {
        let secret = b"TOP_SECRET_CANONICAL".to_vec();
        let record = CanonicalInboundRecord {
            message_id: "safe-id".into(),
            source: [0x11; 16],
            destination: [0x22; 16],
            title: secret.clone(),
            content: secret.clone(),
            timestamp: 1.0,
            fields_msgpack: Some(secret.clone()),
            signature: Some(secret.clone()),
            stamp: Some(secret.clone()),
            wire: secret.clone(),
            authentication_state: "verified".into(),
            stamp_state: "valid".into(),
            stamp_value: Some(1),
            stamp_target: Some(2),
        };
        let debug = format!("{record:?}");
        assert!(!debug.contains("TOP_SECRET_CANONICAL"));
        assert!(debug.contains("content_len"));
        assert!(debug.contains("wire_len"));
    }

    #[test]
    fn v13_plain_resource_progress_survives_restart_and_packet_progress_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("resource-progress-v13.sqlite");
        let message_id = "12".repeat(32);
        let packet_message_id = "23".repeat(32);
        let resource_hash = "34".repeat(32);
        let packet_hash = "56".repeat(32);
        {
            let store = MessagesStore::open(&path).unwrap();
            let mut resource_route = outbound_route(&message_id, None);
            resource_route.representation = "resource".into();
            store
                .insert_outbound_message(
                    &outbound_message(&message_id, 1, Some("queued")),
                    &resource_route,
                )
                .unwrap();
            store
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: message_id.clone(),
                    attempt_number: 1,
                    started_unix_ms: 1,
                    deadline_unix_ms: i64::MAX,
                    state: "sending".into(),
                })
                .unwrap();
            assert!(store
                .track_outbound_evidence(&resource_hash, &message_id, "resource")
                .unwrap());
            assert!(store.update_delivery_evidence_progress(&resource_hash, 25, 100).unwrap());
            store
                .insert_outbound_message(
                    &outbound_message(&packet_message_id, 2, Some("queued")),
                    &outbound_route(&packet_message_id, None),
                )
                .unwrap();
            store
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: packet_message_id.clone(),
                    attempt_number: 1,
                    started_unix_ms: 1,
                    deadline_unix_ms: i64::MAX,
                    state: "sending".into(),
                })
                .unwrap();
            assert!(store
                .track_outbound_evidence(&packet_hash, &packet_message_id, "packet")
                .unwrap());
            assert!(!store.update_delivery_evidence_progress(&packet_hash, 1, 2).unwrap());
        }

        let store = MessagesStore::open(&path).unwrap();
        let evidence = store.message_delivery_evidence(&message_id).unwrap();
        let resource = evidence.iter().find(|item| item.evidence_hash == resource_hash).unwrap();
        assert_eq!(resource.transferred_bytes, Some(25));
        assert_eq!(resource.total_bytes, Some(100));
        assert_eq!(resource.progress, Some(25));
        let packet = store
            .message_delivery_evidence(&packet_message_id)
            .unwrap()
            .into_iter()
            .find(|item| item.evidence_hash == packet_hash)
            .unwrap();
        assert_eq!(packet.transferred_bytes, None);
        assert_eq!(packet.total_bytes, None);
        assert_eq!(packet.progress, None);
    }

    #[test]
    fn v13_terminal_evidence_retention_is_inclusive_and_durable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("evidence-retention-v13.sqlite");
        let message_id = "78".repeat(32);
        let now = unix_time_secs();
        let cutoff = now - DELIVERY_EVIDENCE_RETENTION_SECS;
        let hashes = ["90".repeat(32), "91".repeat(32), "92".repeat(32), "93".repeat(32)];
        {
            let store = MessagesStore::open(&path).unwrap();
            store
                .insert_outbound_message(
                    &outbound_message(&message_id, 1, Some("queued")),
                    &outbound_route(&message_id, None),
                )
                .unwrap();
            store
                .begin_outbound_attempt(&OutboundAttemptRecord {
                    message_id: message_id.clone(),
                    attempt_number: 1,
                    started_unix_ms: 1,
                    deadline_unix_ms: i64::MAX,
                    state: "sending".into(),
                })
                .unwrap();
            for hash in &hashes {
                assert!(store.track_outbound_evidence(hash, &message_id, "packet").unwrap());
            }
            for (hash, terminal_at) in [
                (&hashes[0], Some(cutoff - 1)),
                (&hashes[1], Some(cutoff)),
                (&hashes[2], Some(cutoff + 1)),
                (&hashes[3], None),
            ] {
                store
                    .conn
                    .execute(
                        "UPDATE message_delivery_evidence
                         SET state = CASE WHEN ?2 IS NULL THEN 'tracked' ELSE 'failed' END,
                             observed_at = COALESCE(?2, observed_at), terminal_at = ?2
                         WHERE evidence_hash = ?1",
                        params![hash, terminal_at],
                    )
                    .unwrap();
            }
            assert_eq!(store.prune_delivery_evidence(now).unwrap(), 2);
        }

        let connection = Connection::open(&path).unwrap();
        let remaining: Vec<String> = connection
            .prepare(
                "SELECT evidence_hash FROM message_delivery_evidence
                 WHERE message_id = ?1 ORDER BY evidence_hash",
            )
            .unwrap()
            .query_map(params![message_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(remaining, vec![hashes[2].clone(), hashes[3].clone()]);
        let correlations: Vec<String> = connection
            .prepare(
                "SELECT evidence_id FROM outbound_evidence
                 WHERE message_id = ?1 ORDER BY evidence_id",
            )
            .unwrap()
            .query_map(params![message_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(correlations, vec![hashes[2].clone(), hashes[3].clone()]);
    }

    #[test]
    fn v13_schema_attestation_rejects_a_weakened_evidence_check() {
        let store = MessagesStore::in_memory().unwrap();
        store
            .conn
            .execute_batch(
                "DROP INDEX idx_message_delivery_evidence_terminal;
                 DROP INDEX idx_message_delivery_evidence_message;
                 DROP TABLE message_delivery_evidence;",
            )
            .unwrap();
        let weakened = DELIVERY_EVIDENCE_TABLE_SQL.replace(
            "CHECK((representation = 'packet' AND transferred_bytes IS NULL AND total_bytes IS NULL AND progress IS NULL) OR (representation = 'resource' AND ((transferred_bytes IS NULL AND total_bytes IS NULL AND progress IS NULL) OR (transferred_bytes IS NOT NULL AND total_bytes IS NOT NULL AND progress IS NOT NULL AND transferred_bytes <= total_bytes))))",
            "CHECK(1)",
        );
        assert_ne!(weakened, DELIVERY_EVIDENCE_TABLE_SQL);
        store.conn.execute_batch(&weakened).unwrap();
        store.conn.execute_batch(DELIVERY_EVIDENCE_INDEX_SQL).unwrap();
        store.conn.execute_batch(DELIVERY_EVIDENCE_RETENTION_INDEX_SQL).unwrap();
        assert!(!message_inspection_schema_is_valid(&store.conn).unwrap());
    }
}
