use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// Core chat operations — the heart of the TUI.
#[async_trait]
pub trait DaemonMessaging: Send + Sync {
    /// Send a chat message to a peer.
    async fn send_chat(&self, request: SendChatRequest) -> Result<MessageId, IpcError>;

    /// Mark all messages from a peer as read. Returns count of messages marked.
    async fn mark_read(&self, peer_hash: &str) -> Result<u64, IpcError>;

    /// Delete an entire conversation with a peer. Returns count of messages deleted.
    async fn delete_conversation(&self, peer_hash: &str) -> Result<u64, IpcError>;

    /// Delete a single message by ID.
    async fn delete_message(&self, message_id: &str) -> Result<bool, IpcError>;

    /// Retry sending a failed message.
    async fn retry_message(&self, message_id: &str) -> Result<bool, IpcError>;

    /// List conversations, optionally filtering to those with unread messages.
    async fn query_conversations(
        &self,
        include_unread: bool,
    ) -> Result<Vec<ConversationInfo>, IpcError>;

    /// Fetch messages for a conversation, with pagination.
    async fn query_messages(
        &self,
        peer_hash: &str,
        limit: u32,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError>;

    /// Full-text search across messages, optionally scoped to a peer.
    async fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError>;

    /// Retrieve raw attachment data for a message.
    async fn query_attachment(&self, message_id: &str) -> Result<Vec<u8>, IpcError>;

    /// Create or update a contact entry for a peer.
    async fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError>;

    /// Remove a contact entry.
    async fn remove_contact(&self, peer_hash: &str) -> Result<bool, IpcError>;

    /// List all contacts.
    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError>;

    /// Resolve a display name to a peer hash, with optional prefix filter.
    async fn resolve_name(
        &self,
        name: &str,
        prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError>;
}
