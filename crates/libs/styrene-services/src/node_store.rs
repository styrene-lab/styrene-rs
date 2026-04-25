//! Persistent node store — SQLite-backed peer registry that survives restarts.
//!
//! Replaces the in-memory `HashMap<String, PeerRecord>` in DiscoveryService
//! with a durable store that tracks peer announces, device metadata,
//! connectivity history, and capabilities.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::ServiceError;

/// A discovered mesh node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Peer identity hash (hex, 32 chars).
    pub identity_hash: String,
    /// Display name (from announce app_data).
    pub display_name: Option<String>,
    /// Source of the display name ("announce", "manual", "contact").
    pub name_source: Option<String>,
    /// Unix timestamp of first discovery.
    pub first_seen: i64,
    /// Unix timestamp of most recent announce.
    pub last_seen: i64,
    /// Total announce count.
    pub announce_count: u64,
    /// Average signal quality (RSSI/SNR if available).
    pub signal_quality: Option<f64>,
    /// Device type label (e.g., "node", "hub", "gateway").
    pub device_type: Option<String>,
    /// Whether this node is explicitly blocked.
    pub blocked: bool,
    /// Whether this node is bookmarked/favorited.
    pub bookmarked: bool,
}

/// Persistent node registry backed by SQLite.
pub struct NodeStore {
    conn: Mutex<Connection>,
}

impl NodeStore {
    /// Open or create a node store at the given path.
    pub fn open(path: &str) -> Result<Self, ServiceError> {
        let conn = Connection::open(path)?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory() -> Result<Self, ServiceError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS nodes (
                identity_hash   TEXT PRIMARY KEY,
                display_name    TEXT,
                name_source     TEXT,
                first_seen      INTEGER NOT NULL,
                last_seen       INTEGER NOT NULL,
                announce_count  INTEGER NOT NULL DEFAULT 1,
                signal_quality  REAL,
                device_type     TEXT,
                blocked         INTEGER NOT NULL DEFAULT 0,
                bookmarked      INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_nodes_last_seen ON nodes(last_seen);
            ",
        )?;
        Ok(())
    }

    /// Upsert a node from an announce. Increments announce count, updates
    /// last_seen, and optionally updates display name.
    pub fn accept_announce(
        &self,
        identity_hash: &str,
        timestamp: i64,
        display_name: Option<&str>,
        name_source: Option<&str>,
        device_type: Option<&str>,
        signal_quality: Option<f64>,
    ) -> Result<Node, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;

        // Try to get existing node
        let existing: Option<(i64, u64)> = conn
            .query_row(
                "SELECT first_seen, announce_count FROM nodes WHERE identity_hash = ?1",
                params![identity_hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let (first_seen, announce_count) = match existing {
            Some((fs, ac)) => (fs, ac + 1),
            None => (timestamp, 1),
        };

        conn.execute(
            "INSERT INTO nodes (identity_hash, display_name, name_source, first_seen, last_seen,
                                announce_count, signal_quality, device_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(identity_hash) DO UPDATE SET
                display_name = COALESCE(?2, display_name),
                name_source = COALESCE(?3, name_source),
                last_seen = ?5,
                announce_count = ?6,
                signal_quality = COALESCE(?7, signal_quality),
                device_type = COALESCE(?8, device_type)",
            params![
                identity_hash,
                display_name,
                name_source,
                first_seen,
                timestamp,
                announce_count,
                signal_quality,
                device_type,
            ],
        )?;

        Ok(Node {
            identity_hash: identity_hash.to_string(),
            display_name: display_name.map(|s| s.to_string()),
            name_source: name_source.map(|s| s.to_string()),
            first_seen,
            last_seen: timestamp,
            announce_count,
            signal_quality,
            device_type: device_type.map(|s| s.to_string()),
            blocked: false,
            bookmarked: false,
        })
    }

    /// Get a node by identity hash.
    pub fn get(&self, identity_hash: &str) -> Result<Option<Node>, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.query_row(
            "SELECT identity_hash, display_name, name_source, first_seen, last_seen,
                    announce_count, signal_quality, device_type, blocked, bookmarked
             FROM nodes WHERE identity_hash = ?1",
            params![identity_hash],
            |row| {
                Ok(Node {
                    identity_hash: row.get(0)?,
                    display_name: row.get(1)?,
                    name_source: row.get(2)?,
                    first_seen: row.get(3)?,
                    last_seen: row.get(4)?,
                    announce_count: row.get(5)?,
                    signal_quality: row.get(6)?,
                    device_type: row.get(7)?,
                    blocked: row.get::<_, i32>(8)? != 0,
                    bookmarked: row.get::<_, i32>(9)? != 0,
                })
            },
        )
        .optional()
        .map_err(ServiceError::from)
    }

    /// List all nodes, ordered by last seen (most recent first).
    /// Optionally filter to only nodes seen within `since` seconds.
    pub fn list(&self, since_secs: Option<i64>) -> Result<Vec<Node>, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let cutoff = since_secs
            .map(|s| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
                    - s
            })
            .unwrap_or(0);

        let mut stmt = conn.prepare(
            "SELECT identity_hash, display_name, name_source, first_seen, last_seen,
                    announce_count, signal_quality, device_type, blocked, bookmarked
             FROM nodes WHERE last_seen >= ?1
             ORDER BY last_seen DESC",
        )?;

        let rows = stmt.query_map(params![cutoff], |row| {
            Ok(Node {
                identity_hash: row.get(0)?,
                display_name: row.get(1)?,
                name_source: row.get(2)?,
                first_seen: row.get(3)?,
                last_seen: row.get(4)?,
                announce_count: row.get(5)?,
                signal_quality: row.get(6)?,
                device_type: row.get(7)?,
                blocked: row.get::<_, i32>(8)? != 0,
                bookmarked: row.get::<_, i32>(9)? != 0,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(ServiceError::from)
    }

    /// Count all known nodes.
    pub fn count(&self) -> Result<u64, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Block or unblock a node.
    pub fn set_blocked(&self, identity_hash: &str, blocked: bool) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE nodes SET blocked = ?1 WHERE identity_hash = ?2",
            params![blocked as i32, identity_hash],
        )?;
        Ok(())
    }

    /// Bookmark or unbookmark a node.
    pub fn set_bookmarked(
        &self,
        identity_hash: &str,
        bookmarked: bool,
    ) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE nodes SET bookmarked = ?1 WHERE identity_hash = ?2",
            params![bookmarked as i32, identity_hash],
        )?;
        Ok(())
    }

    /// Delete a node from the store.
    pub fn remove(&self, identity_hash: &str) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM nodes WHERE identity_hash = ?1", params![identity_hash])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store() {
        let store = NodeStore::in_memory().unwrap();
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.list(None).unwrap().is_empty());
    }

    #[test]
    fn accept_announce_creates_node() {
        let store = NodeStore::in_memory().unwrap();
        let node = store
            .accept_announce("aaa", 1000, Some("Alice"), Some("announce"), None, None)
            .unwrap();

        assert_eq!(node.identity_hash, "aaa");
        assert_eq!(node.display_name.as_deref(), Some("Alice"));
        assert_eq!(node.announce_count, 1);
        assert_eq!(node.first_seen, 1000);
    }

    #[test]
    fn accept_announce_increments_count() {
        let store = NodeStore::in_memory().unwrap();
        store.accept_announce("aaa", 1000, Some("Alice"), None, None, None).unwrap();
        let node = store.accept_announce("aaa", 2000, None, None, None, None).unwrap();

        assert_eq!(node.announce_count, 2);
        assert_eq!(node.first_seen, 1000);
        assert_eq!(node.last_seen, 2000);
    }

    #[test]
    fn get_existing_node() {
        let store = NodeStore::in_memory().unwrap();
        store.accept_announce("aaa", 1000, Some("Alice"), None, None, None).unwrap();

        let node = store.get("aaa").unwrap().unwrap();
        assert_eq!(node.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let store = NodeStore::in_memory().unwrap();
        assert!(store.get("zzz").unwrap().is_none());
    }

    #[test]
    fn list_ordered_by_last_seen() {
        let store = NodeStore::in_memory().unwrap();
        store.accept_announce("aaa", 1000, Some("Alice"), None, None, None).unwrap();
        store.accept_announce("bbb", 2000, Some("Bob"), None, None, None).unwrap();

        let nodes = store.list(None).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].identity_hash, "bbb"); // most recent first
    }

    #[test]
    fn block_and_bookmark() {
        let store = NodeStore::in_memory().unwrap();
        store.accept_announce("aaa", 1000, None, None, None, None).unwrap();

        store.set_blocked("aaa", true).unwrap();
        store.set_bookmarked("aaa", true).unwrap();

        let node = store.get("aaa").unwrap().unwrap();
        assert!(node.blocked);
        assert!(node.bookmarked);
    }

    #[test]
    fn remove_node() {
        let store = NodeStore::in_memory().unwrap();
        store.accept_announce("aaa", 1000, None, None, None, None).unwrap();
        store.remove("aaa").unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }
}
