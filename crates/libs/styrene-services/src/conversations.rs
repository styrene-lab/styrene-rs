//! Conversation service — thread messages by peer, track unread counts.
//!
//! Provides a conversation-oriented view over the flat message store.
//! Each conversation is keyed by a peer identity hash and aggregates
//! messages, unread counts, last activity timestamps, and display names.
//!
//! ## Storage
//!
//! Uses SQLite directly (not the daemon's `MessagesStore`) so this crate
//! remains independent of the app layer. Tables are created on first use.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::ServiceError;

/// Summary of a conversation with a single peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Peer identity hash (hex, 32 chars).
    pub peer_hash: String,
    /// Display name (from announce or manual assignment).
    pub display_name: Option<String>,
    /// Total message count (inbound + outbound).
    pub message_count: u64,
    /// Unread inbound message count.
    pub unread_count: u64,
    /// Unix timestamp of most recent message.
    pub last_activity: i64,
    /// Preview of the most recent message (truncated).
    pub last_message_preview: Option<String>,
    /// Whether this conversation is pinned.
    pub pinned: bool,
    /// Whether this conversation is muted.
    pub muted: bool,
}

/// A single message within a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// Unique message ID.
    pub id: String,
    /// Peer identity hash.
    pub peer_hash: String,
    /// Whether this message is outbound (true) or inbound (false).
    pub outbound: bool,
    /// Message content (plaintext).
    pub content: String,
    /// Unix timestamp.
    pub timestamp: i64,
    /// Whether this message has been read.
    pub read: bool,
    /// Delivery status: "pending", "delivered", "failed".
    pub delivery_status: String,
}

/// Manages conversation state in SQLite.
pub struct ConversationStore {
    conn: Mutex<Connection>,
}

impl ConversationStore {
    /// Open or create a conversation store at the given path.
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
            CREATE TABLE IF NOT EXISTS messages (
                id          TEXT PRIMARY KEY,
                peer_hash   TEXT NOT NULL,
                outbound    INTEGER NOT NULL DEFAULT 0,
                content     TEXT NOT NULL,
                timestamp   INTEGER NOT NULL,
                read        INTEGER NOT NULL DEFAULT 0,
                delivery_status TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE INDEX IF NOT EXISTS idx_messages_peer ON messages(peer_hash, timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_unread ON messages(peer_hash, read) WHERE read = 0;

            CREATE TABLE IF NOT EXISTS conversation_meta (
                peer_hash       TEXT PRIMARY KEY,
                display_name    TEXT,
                pinned          INTEGER NOT NULL DEFAULT 0,
                muted           INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;
        Ok(())
    }

    /// Insert a message into the conversation store.
    pub fn insert_message(&self, msg: &ConversationMessage) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO messages (id, peer_hash, outbound, content, timestamp, read, delivery_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.id,
                msg.peer_hash,
                msg.outbound as i32,
                msg.content,
                msg.timestamp,
                msg.read as i32,
                msg.delivery_status,
            ],
        )?;
        Ok(())
    }

    /// List all conversations, ordered by last activity (most recent first).
    pub fn list_conversations(&self) -> Result<Vec<Conversation>, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT
                m.peer_hash,
                cm.display_name,
                COUNT(*) as message_count,
                SUM(CASE WHEN m.read = 0 AND m.outbound = 0 THEN 1 ELSE 0 END) as unread_count,
                MAX(m.timestamp) as last_activity,
                COALESCE(cm.pinned, 0) as pinned,
                COALESCE(cm.muted, 0) as muted
             FROM messages m
             LEFT JOIN conversation_meta cm ON m.peer_hash = cm.peer_hash
             GROUP BY m.peer_hash
             ORDER BY COALESCE(cm.pinned, 0) DESC, MAX(m.timestamp) DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(Conversation {
                peer_hash: row.get(0)?,
                display_name: row.get(1)?,
                message_count: row.get(2)?,
                unread_count: row.get(3)?,
                last_activity: row.get(4)?,
                last_message_preview: None, // filled separately
                pinned: row.get::<_, i32>(5)? != 0,
                muted: row.get::<_, i32>(6)? != 0,
            })
        })?;

        let mut conversations: Vec<Conversation> = rows.collect::<Result<_, _>>()?;

        // Fill last message preview
        for conv in &mut conversations {
            conv.last_message_preview = self.last_message_preview(&conn, &conv.peer_hash)?;
        }

        Ok(conversations)
    }

    fn last_message_preview(
        &self,
        conn: &Connection,
        peer_hash: &str,
    ) -> Result<Option<String>, ServiceError> {
        let preview: Option<String> = conn
            .query_row(
                "SELECT content FROM messages WHERE peer_hash = ?1 ORDER BY timestamp DESC LIMIT 1",
                params![peer_hash],
                |row| row.get(0),
            )
            .optional()?;

        Ok(preview.map(|s| if s.len() > 100 { format!("{}...", &s[..97]) } else { s }))
    }

    /// Get all messages for a conversation, ordered by timestamp.
    pub fn get_messages(
        &self,
        peer_hash: &str,
        limit: Option<u32>,
        before_timestamp: Option<i64>,
    ) -> Result<Vec<ConversationMessage>, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let (sql, limit_val, before_val);

        if let Some(before) = before_timestamp {
            sql = "SELECT id, peer_hash, outbound, content, timestamp, read, delivery_status
                   FROM messages WHERE peer_hash = ?1 AND timestamp < ?2
                   ORDER BY timestamp DESC LIMIT ?3";
            before_val = before;
            limit_val = limit.unwrap_or(50) as i64;
        } else {
            sql = "SELECT id, peer_hash, outbound, content, timestamp, read, delivery_status
                   FROM messages WHERE peer_hash = ?1 AND timestamp < ?2
                   ORDER BY timestamp DESC LIMIT ?3";
            before_val = i64::MAX;
            limit_val = limit.unwrap_or(50) as i64;
        }

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![peer_hash, before_val, limit_val], |row| {
            Ok(ConversationMessage {
                id: row.get(0)?,
                peer_hash: row.get(1)?,
                outbound: row.get::<_, i32>(2)? != 0,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                read: row.get::<_, i32>(5)? != 0,
                delivery_status: row.get(6)?,
            })
        })?;

        let mut messages: Vec<ConversationMessage> = rows.collect::<Result<_, _>>()?;
        messages.reverse(); // chronological order
        Ok(messages)
    }

    /// Mark all inbound messages from a peer as read.
    pub fn mark_read(&self, peer_hash: &str) -> Result<u64, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let changed = conn.execute(
            "UPDATE messages SET read = 1 WHERE peer_hash = ?1 AND outbound = 0 AND read = 0",
            params![peer_hash],
        )?;
        Ok(changed as u64)
    }

    /// Get unread count for a specific peer.
    pub fn unread_count(&self, peer_hash: &str) -> Result<u64, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE peer_hash = ?1 AND outbound = 0 AND read = 0",
            params![peer_hash],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Get total unread count across all conversations.
    pub fn total_unread(&self) -> Result<u64, ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE outbound = 0 AND read = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Set display name for a peer.
    pub fn set_display_name(&self, peer_hash: &str, name: &str) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO conversation_meta (peer_hash, display_name) VALUES (?1, ?2)
             ON CONFLICT(peer_hash) DO UPDATE SET display_name = ?2",
            params![peer_hash, name],
        )?;
        Ok(())
    }

    /// Pin or unpin a conversation.
    pub fn set_pinned(&self, peer_hash: &str, pinned: bool) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "INSERT INTO conversation_meta (peer_hash, pinned) VALUES (?1, ?2)
             ON CONFLICT(peer_hash) DO UPDATE SET pinned = ?2",
            params![peer_hash, pinned as i32],
        )?;
        Ok(())
    }

    /// Update delivery status for a message.
    pub fn update_delivery_status(
        &self,
        message_id: &str,
        status: &str,
    ) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE messages SET delivery_status = ?1 WHERE id = ?2",
            params![status, message_id],
        )?;
        Ok(())
    }

    /// Delete a conversation and all its messages.
    pub fn delete_conversation(&self, peer_hash: &str) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|e| ServiceError::Storage(e.to_string()))?;
        conn.execute("DELETE FROM messages WHERE peer_hash = ?1", params![peer_hash])?;
        conn.execute("DELETE FROM conversation_meta WHERE peer_hash = ?1", params![peer_hash])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(peer: &str, content: &str, outbound: bool, ts: i64) -> ConversationMessage {
        ConversationMessage {
            id: format!("{peer}-{ts}"),
            peer_hash: peer.to_string(),
            outbound,
            content: content.to_string(),
            timestamp: ts,
            read: false,
            delivery_status: "delivered".to_string(),
        }
    }

    #[test]
    fn empty_store_lists_no_conversations() {
        let store = ConversationStore::in_memory().unwrap();
        let convos = store.list_conversations().unwrap();
        assert!(convos.is_empty());
    }

    #[test]
    fn insert_and_list_conversations() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "hello", false, 100)).unwrap();
        store.insert_message(&test_message("bbb", "hey", false, 200)).unwrap();

        let convos = store.list_conversations().unwrap();
        assert_eq!(convos.len(), 2);
        // Most recent first
        assert_eq!(convos[0].peer_hash, "bbb");
        assert_eq!(convos[1].peer_hash, "aaa");
    }

    #[test]
    fn unread_counts() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "msg1", false, 100)).unwrap();
        store.insert_message(&test_message("aaa", "msg2", false, 200)).unwrap();
        store.insert_message(&test_message("aaa", "reply", true, 300)).unwrap();

        assert_eq!(store.unread_count("aaa").unwrap(), 2);
        assert_eq!(store.total_unread().unwrap(), 2);

        let marked = store.mark_read("aaa").unwrap();
        assert_eq!(marked, 2);
        assert_eq!(store.unread_count("aaa").unwrap(), 0);
    }

    #[test]
    fn get_messages_chronological() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "first", false, 100)).unwrap();
        store.insert_message(&test_message("aaa", "second", true, 200)).unwrap();
        store.insert_message(&test_message("aaa", "third", false, 300)).unwrap();

        let msgs = store.get_messages("aaa", None, None).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[2].content, "third");
    }

    #[test]
    fn get_messages_with_pagination() {
        let store = ConversationStore::in_memory().unwrap();
        for i in 0..10 {
            store.insert_message(&test_message("aaa", &format!("msg{i}"), false, i * 100)).unwrap();
        }

        let page1 = store.get_messages("aaa", Some(3), None).unwrap();
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[2].content, "msg9"); // most recent

        let page2 = store.get_messages("aaa", Some(3), Some(page1[0].timestamp)).unwrap();
        assert_eq!(page2.len(), 3);
    }

    #[test]
    fn pinned_conversations_sort_first() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "old", false, 100)).unwrap();
        store.insert_message(&test_message("bbb", "new", false, 200)).unwrap();

        store.set_pinned("aaa", true).unwrap();

        let convos = store.list_conversations().unwrap();
        assert_eq!(convos[0].peer_hash, "aaa"); // pinned, even though older
        assert!(convos[0].pinned);
    }

    #[test]
    fn display_name() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "hello", false, 100)).unwrap();
        store.set_display_name("aaa", "Alice").unwrap();

        let convos = store.list_conversations().unwrap();
        assert_eq!(convos[0].display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn delete_conversation() {
        let store = ConversationStore::in_memory().unwrap();
        store.insert_message(&test_message("aaa", "hello", false, 100)).unwrap();
        store.delete_conversation("aaa").unwrap();

        let convos = store.list_conversations().unwrap();
        assert!(convos.is_empty());
    }

    #[test]
    fn delivery_status_update() {
        let store = ConversationStore::in_memory().unwrap();
        let msg = test_message("aaa", "sending", true, 100);
        store.insert_message(&msg).unwrap();

        store.update_delivery_status(&msg.id, "delivered").unwrap();

        let msgs = store.get_messages("aaa", None, None).unwrap();
        assert_eq!(msgs[0].delivery_status, "delivered");
    }

    #[test]
    fn last_message_preview_truncated() {
        let store = ConversationStore::in_memory().unwrap();
        let long_msg = "a".repeat(200);
        store.insert_message(&test_message("aaa", &long_msg, false, 100)).unwrap();

        let convos = store.list_conversations().unwrap();
        let preview = convos[0].last_message_preview.as_ref().unwrap();
        assert!(preview.len() <= 103); // 97 chars + "..."
        assert!(preview.ends_with("..."));
    }
}
