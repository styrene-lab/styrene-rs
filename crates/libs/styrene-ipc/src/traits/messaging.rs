use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// Core chat operations — the heart of the TUI.
#[async_trait]
pub trait DaemonMessaging: Send + Sync {
    /// Idempotently create a durable empty conversation for a canonical destination.
    async fn start_conversation(
        &self,
        _peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        Err(IpcError::not_implemented("start_conversation"))
    }

    /// Send a chat message to a peer.
    async fn send_chat(&self, request: SendChatRequest) -> Result<MessageId, IpcError>;

    async fn send_chat_outcome(
        &self,
        request: SendChatRequest,
    ) -> Result<SendChatOutcome, IpcError> {
        let message_id = self.send_chat(request).await?;
        let message = MessageInfo { id: message_id.clone(), ..Default::default() };
        Ok(SendChatOutcome {
            message_id,
            message,
            disposition: SendChatDisposition::Accepted,
            ..Default::default()
        })
    }

    async fn set_draft(
        &self,
        _peer_hash: &str,
        _content: &str,
    ) -> Result<ConversationDraft, IpcError> {
        Err(IpcError::not_implemented("set_draft"))
    }

    async fn draft(&self, _peer_hash: &str) -> Result<Option<ConversationDraft>, IpcError> {
        Err(IpcError::not_implemented("draft"))
    }

    async fn clear_draft(&self, _peer_hash: &str) -> Result<MessagingDisposition, IpcError> {
        Err(IpcError::not_implemented("clear_draft"))
    }

    async fn clear_draft_if_revision(
        &self,
        _peer_hash: &str,
        _revision: u64,
    ) -> Result<MessagingDisposition, IpcError> {
        Err(IpcError::not_implemented("clear_draft_if_revision"))
    }

    /// Mark all messages from a peer as read. Returns count of messages marked.
    async fn mark_read(&self, peer_hash: &str) -> Result<u64, IpcError>;

    async fn mark_read_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let count = self.mark_read(peer_hash).await?;
        Ok(MessagingOperationOutcome {
            disposition: if count == 0 {
                MessagingDisposition::Unchanged
            } else {
                MessagingDisposition::Applied
            },
            affected_count: count,
            target_id: peer_hash.into(),
            ..Default::default()
        })
    }

    /// Delete an entire conversation with a peer. Returns count of messages deleted.
    async fn delete_conversation(&self, peer_hash: &str) -> Result<u64, IpcError>;

    async fn delete_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let count = self.delete_conversation(peer_hash).await?;
        Ok(MessagingOperationOutcome {
            disposition: if count == 0 {
                MessagingDisposition::NotFound
            } else {
                MessagingDisposition::Applied
            },
            affected_count: count,
            target_id: peer_hash.into(),
            ..Default::default()
        })
    }

    /// Delete a single message by ID.
    async fn delete_message(&self, message_id: &str) -> Result<bool, IpcError>;

    async fn delete_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let applied = self.delete_message(message_id).await?;
        Ok(MessagingOperationOutcome {
            disposition: if applied {
                MessagingDisposition::Applied
            } else {
                MessagingDisposition::NotFound
            },
            affected_count: u64::from(applied),
            target_id: message_id.into(),
            ..Default::default()
        })
    }

    /// Retry sending a failed message.
    async fn retry_message(&self, message_id: &str) -> Result<bool, IpcError>;

    async fn retry_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let applied = self.retry_message(message_id).await?;
        Ok(MessagingOperationOutcome {
            disposition: if applied {
                MessagingDisposition::Applied
            } else {
                MessagingDisposition::Unchanged
            },
            affected_count: u64::from(applied),
            target_id: message_id.into(),
            ..Default::default()
        })
    }

    /// Cancel a queued or in-flight outbound message.
    async fn cancel_message(&self, _message_id: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("cancel_message"))
    }

    async fn cancel_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let applied = self.cancel_message(message_id).await?;
        Ok(MessagingOperationOutcome {
            disposition: if applied {
                MessagingDisposition::Applied
            } else {
                MessagingDisposition::Unchanged
            },
            affected_count: u64::from(applied),
            target_id: message_id.into(),
            ..Default::default()
        })
    }

    /// List conversations, filtering to conversations with inbound unread messages when requested.
    async fn query_conversations(
        &self,
        unread_only: bool,
    ) -> Result<Vec<ConversationInfo>, IpcError>;

    /// List one stable keyset page of conversations.
    async fn query_conversation_page(
        &self,
        unread_only: bool,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ConversationPage, IpcError> {
        if !(1..=MAX_MESSAGE_QUERY_LIMIT).contains(&limit) {
            return Err(IpcError::invalid_request("page limit must be between 1 and 256"));
        }
        let _ = cursor;
        let mut page = ConversationPage {
            conversations: self.query_conversations(unread_only).await?,
            ..Default::default()
        };
        page.conversations.truncate(limit as usize);
        Ok(page)
    }

    /// Fetch messages for a conversation, with pagination.
    async fn query_messages(
        &self,
        peer_hash: &str,
        limit: u32,
        before_ts: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError>;

    /// Fetch one complete authorized message projection by its stable ID.
    async fn query_message(&self, message_id: &str) -> Result<Option<MessageInfo>, IpcError>;

    /// Fetch one stable keyset page of a conversation's message history.
    async fn query_message_page(
        &self,
        peer_hash: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<MessagePage, IpcError> {
        if !(1..=MAX_MESSAGE_QUERY_LIMIT).contains(&limit) {
            return Err(IpcError::invalid_request("page limit must be between 1 and 256"));
        }
        let _ = cursor;
        Ok(MessagePage {
            messages: self.query_messages(peer_hash, limit, None).await?,
            ..Default::default()
        })
    }

    /// Full-text search across messages, optionally scoped to a peer.
    async fn search_messages(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError>;

    async fn search_messages_outcome(
        &self,
        query: &str,
        peer_hash: Option<&str>,
        limit: u32,
    ) -> Result<MessageSearchOutcome, IpcError> {
        let messages = self.search_messages(query, peer_hash, limit).await?;
        Ok(MessageSearchOutcome {
            returned_count: messages.len() as u32,
            matched_count: messages.len() as u64,
            messages,
            order: "timestamp_desc_id_desc".into(),
            query: query.into(),
            peer_hash: peer_hash.map(str::to_owned),
            limit,
            ..Default::default()
        })
    }

    /// Retrieve raw attachment data for a message.
    async fn query_attachment(&self, message_id: &str) -> Result<Vec<u8>, IpcError>;

    async fn list_attachments(&self, _message_id: &str) -> Result<Vec<AttachmentInfo>, IpcError> {
        Err(IpcError::not_implemented("list_attachments"))
    }

    async fn query_attachment_chunk(
        &self,
        message_id: &str,
        ordinal: u8,
        offset: u64,
        max_bytes: u32,
    ) -> Result<AttachmentChunk, IpcError> {
        if ordinal == 0 && offset == 0 && max_bytes as usize >= 256 * 1024 {
            let data = self.query_attachment(message_id).await?;
            return Ok(AttachmentChunk {
                next_offset: data.len() as u64,
                done: true,
                data,
                ..Default::default()
            });
        }
        Err(IpcError::not_implemented("query_attachment_chunk"))
    }

    async fn cancel_attachment_transfer(
        &self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        self.cancel_message_outcome(message_id).await
    }

    async fn query_attachment_transfer(
        &self,
        message_id: &str,
    ) -> Result<AttachmentTransferInfo, IpcError> {
        self.list_attachments(message_id)
            .await?
            .into_iter()
            .find_map(|attachment| attachment.transfer.map(|transfer| *transfer))
            .ok_or_else(|| IpcError::not_found("attachment transfer", message_id))
    }

    /// Create or update a contact entry for a peer.
    async fn set_contact(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError>;

    async fn set_contact_outcome(
        &self,
        peer_hash: &str,
        alias: Option<&str>,
        notes: Option<&str>,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let contact = self.set_contact(peer_hash, alias, notes).await?;
        Ok(MessagingOperationOutcome {
            disposition: MessagingDisposition::Applied,
            affected_count: 1,
            target_id: peer_hash.into(),
            contact: Some(contact),
            ..Default::default()
        })
    }

    /// Remove a contact entry.
    async fn remove_contact(&self, peer_hash: &str) -> Result<bool, IpcError>;

    async fn remove_contact_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        let applied = self.remove_contact(peer_hash).await?;
        Ok(MessagingOperationOutcome {
            disposition: if applied {
                MessagingDisposition::Applied
            } else {
                MessagingDisposition::NotFound
            },
            affected_count: u64::from(applied),
            target_id: peer_hash.into(),
            ..Default::default()
        })
    }

    /// List all contacts.
    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError>;

    /// Resolve a display name to a peer hash, with optional prefix filter.
    async fn resolve_name(
        &self,
        name: &str,
        prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError>;

    /// Pin a conversation so it sorts to the top.
    async fn pin_conversation(&self, peer_hash: &str) -> Result<bool, IpcError>;

    async fn pin_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        conversation_flag_outcome(self.pin_conversation(peer_hash).await?, peer_hash)
    }

    /// Unpin a conversation.
    async fn unpin_conversation(&self, peer_hash: &str) -> Result<bool, IpcError>;

    async fn unpin_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        conversation_flag_outcome(self.unpin_conversation(peer_hash).await?, peer_hash)
    }

    /// Mute notifications for a conversation.
    async fn mute_conversation(&self, peer_hash: &str) -> Result<bool, IpcError>;

    async fn mute_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        conversation_flag_outcome(self.mute_conversation(peer_hash).await?, peer_hash)
    }

    /// Unmute notifications for a conversation.
    async fn unmute_conversation(&self, peer_hash: &str) -> Result<bool, IpcError>;

    async fn unmute_conversation_outcome(
        &self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, IpcError> {
        conversation_flag_outcome(self.unmute_conversation(peer_hash).await?, peer_hash)
    }
}

fn conversation_flag_outcome(
    applied: bool,
    peer_hash: &str,
) -> Result<MessagingOperationOutcome, IpcError> {
    Ok(MessagingOperationOutcome {
        disposition: if applied {
            MessagingDisposition::Applied
        } else {
            MessagingDisposition::Unchanged
        },
        affected_count: u64::from(applied),
        target_id: peer_hash.into(),
        ..Default::default()
    })
}
