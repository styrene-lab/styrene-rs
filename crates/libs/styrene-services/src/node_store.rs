//! Persistent node store — SQLite-backed peer registry that survives restarts.
//!
//! Replaces the in-memory `HashMap<String, PeerRecord>` in DiscoveryService
//! with a durable store that tracks peer announces, device metadata,
//! connectivity history, and capabilities.

use rusqlite::{Connection, OptionalExtension, params};
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

    /// Upsert a node keyed by its announced destination hash.
    pub fn accept_announce(
        &self,
        destination_hash: &str,
        timestamp: i64,
        display_name: Option<&str>,
        name_source: Option<&str>,
        device_type: Option<&str>,
        signal_quality: Option<f64>,
    ) -> Result<Node, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;

        conn.execute(
            "INSERT INTO nodes (identity_hash, display_name, name_source, first_seen, last_seen,
                                announce_count, signal_quality, device_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(identity_hash) DO UPDATE SET
                display_name = CASE WHEN excluded.last_seen >= nodes.last_seen
                    THEN COALESCE(excluded.display_name, nodes.display_name)
                    ELSE nodes.display_name END,
                name_source = CASE WHEN excluded.last_seen >= nodes.last_seen
                    THEN COALESCE(excluded.name_source, nodes.name_source)
                    ELSE nodes.name_source END,
                first_seen = MIN(nodes.first_seen, excluded.first_seen),
                last_seen = MAX(nodes.last_seen, excluded.last_seen),
                announce_count = nodes.announce_count + 1,
                signal_quality = CASE WHEN excluded.last_seen >= nodes.last_seen
                    THEN COALESCE(excluded.signal_quality, nodes.signal_quality)
                    ELSE nodes.signal_quality END,
                device_type = CASE WHEN excluded.last_seen >= nodes.last_seen
                    THEN COALESCE(excluded.device_type, nodes.device_type)
                    ELSE nodes.device_type END",
            params![
                destination_hash,
                display_name,
                name_source,
                timestamp,
                timestamp,
                1_u64,
                signal_quality,
                device_type,
            ],
        )?;

        conn.query_row(
            "SELECT identity_hash, display_name, name_source, first_seen, last_seen,
                    announce_count, signal_quality, device_type, blocked, bookmarked
             FROM nodes WHERE identity_hash = ?1",
            params![destination_hash],
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
        .map_err(ServiceError::from)
    }

    /// Get a node by identity hash.
    /// Copy the committed node state into a fresh database file through
    /// SQLite's online backup API, holding the connection for the duration.
    pub fn backup_to(&self, destination: &std::path::Path) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let mut target =
            Connection::open(destination).map_err(|e| ServiceError::Storage(e.to_string()))?;
        let backup = rusqlite::backup::Backup::new(&conn, &mut target)
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        backup
            .run_to_completion(64, std::time::Duration::from_millis(5), None)
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        drop(backup);
        target
            .pragma_update(None, "journal_mode", "delete")
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        Ok(())
    }

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
    fn same_destination_upserts_one_node_without_regressing_freshness() {
        let store = NodeStore::in_memory().unwrap();
        store
            .accept_announce(
                "destination",
                2_000,
                Some("New Name"),
                Some("canonical_announce"),
                Some("lxmf.delivery"),
                Some(-40.0),
            )
            .unwrap();

        let merged = store
            .accept_announce(
                "destination",
                1_000,
                Some("Stale Name"),
                Some("legacy"),
                Some("unknown"),
                Some(-90.0),
            )
            .unwrap();

        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(merged.first_seen, 1_000);
        assert_eq!(merged.last_seen, 2_000);
        assert_eq!(merged.announce_count, 2);
        assert_eq!(merged.display_name.as_deref(), Some("New Name"));
        assert_eq!(merged.name_source.as_deref(), Some("canonical_announce"));
        assert_eq!(merged.device_type.as_deref(), Some("lxmf.delivery"));
        assert_eq!(merged.signal_quality, Some(-40.0));
        assert_eq!(store.get("destination").unwrap().unwrap().last_seen, 2_000);
    }

    #[test]
    fn newer_announce_without_name_returns_preserved_persisted_metadata() {
        let store = NodeStore::in_memory().unwrap();
        store
            .accept_announce(
                "destination",
                1_000,
                Some("Preserved Name"),
                Some("canonical_announce"),
                Some("lxmf.delivery"),
                None,
            )
            .unwrap();

        let merged = store.accept_announce("destination", 2_000, None, None, None, None).unwrap();

        assert_eq!(merged.display_name.as_deref(), Some("Preserved Name"));
        assert_eq!(merged.name_source.as_deref(), Some("canonical_announce"));
        assert_eq!(merged.device_type.as_deref(), Some("lxmf.delivery"));
        assert_eq!(merged.last_seen, 2_000);
        assert_eq!(merged.announce_count, 2);
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
