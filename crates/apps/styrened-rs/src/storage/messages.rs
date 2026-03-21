use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContactRecord {
    pub peer_hash: String,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Summary of a conversation with a peer.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSummary {
    pub peer_hash: String,
    pub peer_name: Option<String>,
    pub last_message_timestamp: Option<i64>,
    pub last_message_content: Option<String>,
    pub unread_count: u32,
    pub message_count: u32,
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
    pub stamp_cost_flexibility: Option<u32>,
    pub peering_cost: Option<u32>,
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

pub struct MessagesStore {
    conn: Connection,
}

impl MessagesStore {
    const SDK_DOMAIN_SNAPSHOT_KEY: &'static str = "sdk_domains.v1";

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // WAL mode is required for concurrent readers (RpcDaemon + AppContext
        // hold separate connections to the same database file).
        conn.pragma_update(None, "journal_mode", "wal")?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    pub fn insert_message(&self, record: &MessageRecord) -> rusqlite::Result<()> {
        let fields_json =
            record.fields.as_ref().map(|value| serde_json::to_string(value).unwrap_or_default());
        self.conn.execute(
            "INSERT OR REPLACE INTO messages (id, source, destination, title, content, timestamp, direction, fields, receipt_status, read) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
        )?;
        Ok(())
    }

    pub fn list_messages(
        &self,
        limit: usize,
        before_ts: Option<i64>,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        let mut records = Vec::new();
        if let Some(ts) = before_ts {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE timestamp < ?1 ORDER BY timestamp DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![ts, limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages ORDER BY timestamp DESC LIMIT ?1",
            )?;
            let mut rows = stmt.query(params![limit as i64])?;
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
        stmt.query_row(params![message_id], parse_message_row)
            .optional()
    }

    /// List messages filtered by peer hash (source or destination matches).
    pub fn list_messages_for_peer(
        &self,
        peer_hash: &str,
        limit: usize,
        before_ts: Option<i64>,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        let mut records = Vec::new();
        if let Some(ts) = before_ts {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE (source = ?1 OR destination = ?1) AND timestamp < ?2 ORDER BY timestamp DESC LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![peer_hash, ts, limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE (source = ?1 OR destination = ?1) ORDER BY timestamp DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![peer_hash, limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        }
        Ok(records)
    }

    /// Mark all messages from a peer as read. Returns count of updated rows.
    pub fn mark_read(&self, peer_hash: &str) -> rusqlite::Result<u64> {
        let count = self.conn.execute(
            "UPDATE messages SET read = 1 WHERE (source = ?1 OR destination = ?1) AND COALESCE(read, 0) = 0",
            params![peer_hash],
        )?;
        Ok(count as u64)
    }

    /// Delete all messages in a conversation with a peer. Returns count.
    pub fn delete_conversation(&self, peer_hash: &str) -> rusqlite::Result<u64> {
        let count = self.conn.execute(
            "DELETE FROM messages WHERE source = ?1 OR destination = ?1",
            params![peer_hash],
        )?;
        Ok(count as u64)
    }

    /// Delete a single message by ID. Returns true if deleted.
    pub fn delete_message(&self, message_id: &str) -> rusqlite::Result<bool> {
        let count = self.conn.execute(
            "DELETE FROM messages WHERE id = ?1",
            params![message_id],
        )?;
        Ok(count > 0)
    }

    /// Search messages by content substring, optionally scoped to a peer.
    pub fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<Vec<MessageRecord>> {
        let pattern = format!("%{query}%");
        let mut records = Vec::new();
        if let Some(peer) = peer_hash {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE (source = ?1 OR destination = ?1) AND content LIKE ?2 ORDER BY timestamp DESC LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![peer, pattern, limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, source, destination, title, content, timestamp, direction, fields, receipt_status, COALESCE(read, 0) FROM messages WHERE content LIKE ?1 ORDER BY timestamp DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![pattern, limit as i64])?;
            while let Some(row) = rows.next()? {
                records.push(parse_message_row(row)?);
            }
        }
        Ok(records)
    }

    /// List conversation summaries grouped by peer.
    pub fn list_conversations(&self, unread_only: bool) -> rusqlite::Result<Vec<ConversationSummary>> {
        // For each unique peer (source or destination), aggregate message stats.
        // The peer is the "other side" — for outgoing messages it's destination, for incoming it's source.
        // Use a correlated subquery to get the content of the most recent message
        // per peer (MAX(content) would give lexicographic max, not most-recent).
        let base = "SELECT g.peer, g.last_ts, (SELECT m2.content FROM messages m2 WHERE (CASE WHEN m2.direction = 'out' THEN m2.destination ELSE m2.source END) = g.peer ORDER BY m2.timestamp DESC LIMIT 1) as last_content, g.unread, g.total FROM (SELECT CASE WHEN direction = 'out' THEN destination ELSE source END as peer, MAX(timestamp) as last_ts, SUM(CASE WHEN COALESCE(read, 0) = 0 THEN 1 ELSE 0 END) as unread, COUNT(*) as total FROM messages GROUP BY peer) g";
        let sql = if unread_only {
            format!("{base} WHERE g.unread > 0 ORDER BY g.last_ts DESC")
        } else {
            format!("{base} ORDER BY g.last_ts DESC")
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
            });
        }
        Ok(summaries)
    }

    // ── Contacts ────────────────────────────────────────────────────────

    /// Upsert a contact record. Returns the saved record.
    pub fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> rusqlite::Result<ContactRecord> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn.execute(
            "INSERT INTO contacts (peer_hash, alias, notes, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(peer_hash) DO UPDATE SET alias = ?2, notes = ?3, updated_at = ?4",
            params![peer_hash, alias, notes, now],
        )?;
        Ok(ContactRecord {
            peer_hash: peer_hash.to_string(),
            alias: alias.map(String::from),
            notes: notes.map(String::from),
            created_at: now,
            updated_at: now,
        })
    }

    /// Remove a contact by peer hash. Returns true if deleted.
    pub fn remove_contact(&self, peer_hash: &str) -> rusqlite::Result<bool> {
        let count = self.conn.execute(
            "DELETE FROM contacts WHERE peer_hash = ?1",
            params![peer_hash],
        )?;
        Ok(count > 0)
    }

    /// List all contacts.
    pub fn list_contacts(&self) -> rusqlite::Result<Vec<ContactRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_hash, alias, notes, created_at, updated_at FROM contacts ORDER BY updated_at DESC",
        )?;
        let mut rows = stmt.query([])?;
        let mut contacts = Vec::new();
        while let Some(row) = rows.next()? {
            contacts.push(ContactRecord {
                peer_hash: row.get(0)?,
                alias: row.get(1)?,
                notes: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
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
        let collect_ids = |query: &str, remaining: usize| -> rusqlite::Result<Vec<String>> {
            if remaining == 0 {
                return Ok(Vec::new());
            }
            let mut stmt = self.conn.prepare(query)?;
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
        for message_id in ids.iter() {
            self.conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        }
        Ok(ids)
    }

    pub fn update_receipt_status(&self, message_id: &str, status: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE messages SET receipt_status = ?1 WHERE id = ?2",
            params![status, message_id],
        )?;
        Ok(())
    }

    pub fn clear_messages(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM messages", [])?;
        Ok(())
    }

    pub fn insert_announce(&self, record: &AnnounceRecord) -> rusqlite::Result<()> {
        let capabilities_json = serde_json::to_string(&record.capabilities).unwrap_or_default();
        self.conn.execute(
            "INSERT OR REPLACE INTO announces (id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost_flexibility, peering_cost) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                record.stamp_cost_flexibility,
                record.peering_cost,
            ],
        )?;
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
                stamp_cost_flexibility: row.get(12)?,
                peering_cost: row.get(13)?,
            })
        };
        if let Some(ts) = before_ts {
            let query_with_id = "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost_flexibility, peering_cost FROM announces WHERE (timestamp < ?1 OR (timestamp = ?1 AND id < ?2)) ORDER BY timestamp DESC, id DESC LIMIT ?3";
            let query_without_id = "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost_flexibility, peering_cost FROM announces WHERE timestamp < ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2";
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
                "SELECT id, peer, timestamp, name, name_source, first_seen, seen_count, app_data_hex, capabilities, rssi, snr, q, stamp_cost_flexibility, peering_cost FROM announces ORDER BY timestamp DESC LIMIT ?1",
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

    fn init_schema(&self) -> rusqlite::Result<()> {
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
                stamp_cost_flexibility INTEGER,
                peering_cost INTEGER
            );
            CREATE TABLE IF NOT EXISTS sdk_domain_state (
                domain TEXT PRIMARY KEY,
                state_json TEXT NOT NULL
            );",
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
            );",
        )?;
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
        store.insert_message(&chat_message("m3", "bob", "me", 3)).expect("insert");

        let count = store.mark_read("alice").expect("mark_read");
        assert_eq!(count, 2);

        let m1 = store.get_message("m1").expect("get").expect("exists");
        assert!(m1.read);
        let m3 = store.get_message("m3").expect("get").expect("exists");
        assert!(!m3.read); // Bob's message unchanged
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
        store.set_contact("alice", Some("Alice B"), None).expect("update");
        let contacts = store.list_contacts().expect("list");
        assert_eq!(contacts[0].alias.as_deref(), Some("Alice B"));

        // Remove
        assert!(store.remove_contact("alice").expect("remove"));
        assert!(!store.remove_contact("alice").expect("remove again"));
        assert!(store.list_contacts().expect("list").is_empty());
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
}
