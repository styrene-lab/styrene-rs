use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::IpcError;
use crate::traits::*;
use crate::types::*;

/// A daemon implementation that returns `NotImplemented` for every method.
///
/// This is the starting point for incremental development — wire it into
/// `styrened-rs`, then replace stubs one method at a time.
pub struct StubDaemon;

#[async_trait]
impl DaemonMessaging for StubDaemon {
    async fn send_chat(&self, _request: SendChatRequest) -> Result<MessageId, IpcError> {
        Err(IpcError::not_implemented("send_chat"))
    }

    async fn mark_read(&self, _peer_hash: &str) -> Result<u64, IpcError> {
        Err(IpcError::not_implemented("mark_read"))
    }

    async fn delete_conversation(&self, _peer_hash: &str) -> Result<u64, IpcError> {
        Err(IpcError::not_implemented("delete_conversation"))
    }

    async fn delete_message(&self, _message_id: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("delete_message"))
    }

    async fn retry_message(&self, _message_id: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("retry_message"))
    }

    async fn query_conversations(
        &self,
        _include_unread: bool,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        Err(IpcError::not_implemented("query_conversations"))
    }

    async fn query_messages(
        &self,
        _peer_hash: &str,
        _limit: u32,
        _before_ts: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Err(IpcError::not_implemented("query_messages"))
    }

    async fn search_messages(
        &self,
        _query: &str,
        _peer_hash: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Err(IpcError::not_implemented("search_messages"))
    }

    async fn query_attachment(&self, _message_id: &str) -> Result<Vec<u8>, IpcError> {
        Err(IpcError::not_implemented("query_attachment"))
    }

    async fn set_contact(
        &self,
        _peer_hash: &str,
        _alias: Option<&str>,
        _notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError> {
        Err(IpcError::not_implemented("set_contact"))
    }

    async fn remove_contact(&self, _peer_hash: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("remove_contact"))
    }

    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError> {
        Err(IpcError::not_implemented("query_contacts"))
    }

    async fn resolve_name(
        &self,
        _name: &str,
        _prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError> {
        Err(IpcError::not_implemented("resolve_name"))
    }
}

#[async_trait]
impl DaemonIdentity for StubDaemon {
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError> {
        Err(IpcError::not_implemented("query_identity"))
    }

    async fn set_identity(
        &self,
        _display_name: Option<&str>,
        _icon: Option<&str>,
        _short_name: Option<&str>,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("set_identity"))
    }

    async fn announce(&self) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("announce"))
    }
}

#[async_trait]
impl DaemonStatus for StubDaemon {
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError> {
        Err(IpcError::not_implemented("query_status"))
    }

    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError> {
        Err(IpcError::not_implemented("query_config"))
    }

    async fn query_devices(&self, _styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError> {
        Err(IpcError::not_implemented("query_devices"))
    }

    async fn query_path_info(&self, _dest_hash: &str) -> Result<PathInfo, IpcError> {
        Err(IpcError::not_implemented("query_path_info"))
    }

    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError> {
        Err(IpcError::not_implemented("query_auto_reply"))
    }

    async fn set_auto_reply(
        &self,
        _mode: &str,
        _message: Option<&str>,
        _cooldown_secs: Option<u64>,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("set_auto_reply"))
    }
}

#[async_trait]
impl DaemonFleet for StubDaemon {
    async fn device_status(
        &self,
        _dest: &str,
        _timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, IpcError> {
        Err(IpcError::not_implemented("device_status"))
    }

    async fn exec(
        &self,
        _dest: &str,
        _cmd: &str,
        _args: Vec<String>,
        _timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError> {
        Err(IpcError::not_implemented("exec"))
    }

    async fn reboot_device(
        &self,
        _dest: &str,
        _delay: Option<u64>,
        _timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError> {
        Err(IpcError::not_implemented("reboot_device"))
    }

    async fn self_update(
        &self,
        _dest: &str,
        _version: Option<&str>,
        _timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError> {
        Err(IpcError::not_implemented("self_update"))
    }

    async fn remote_inbox(
        &self,
        _dest: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        Err(IpcError::not_implemented("remote_inbox"))
    }

    async fn remote_messages(
        &self,
        _dest: &str,
        _peer_hash: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Err(IpcError::not_implemented("remote_messages"))
    }

    async fn terminal_open(&self, _request: TerminalOpenRequest) -> Result<SessionId, IpcError> {
        Err(IpcError::not_implemented("terminal_open"))
    }

    async fn terminal_input(&self, _session_id: &str, _data: &[u8]) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("terminal_input"))
    }

    async fn terminal_resize(
        &self,
        _session_id: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("terminal_resize"))
    }

    async fn terminal_close(&self, _session_id: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("terminal_close"))
    }
}

#[async_trait]
impl DaemonEvents for StubDaemon {
    async fn subscribe_messages(
        &self,
        _peer_hashes: &[String],
    ) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        Err(IpcError::not_implemented("subscribe_messages"))
    }

    async fn subscribe_devices(&self) -> Result<broadcast::Receiver<DaemonEvent>, IpcError> {
        Err(IpcError::not_implemented("subscribe_devices"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that StubDaemon returns NotImplemented for every trait method.
    /// This validates the wiring — all traits are implemented and the error
    /// type works correctly.
    #[tokio::test]
    async fn stub_returns_not_implemented() {
        let stub = StubDaemon;

        // DaemonMessaging
        let err = stub
            .send_chat(SendChatRequest::default())
            .await
            .expect_err("should be NotImplemented");
        assert_eq!(
            err,
            IpcError::NotImplemented {
                method: "send_chat".into()
            }
        );
        assert!(!err.is_retryable());

        assert!(stub.mark_read("abc").await.is_err());
        assert!(stub.delete_conversation("abc").await.is_err());
        assert!(stub.delete_message("abc").await.is_err());
        assert!(stub.retry_message("abc").await.is_err());
        assert!(stub.query_conversations(true).await.is_err());
        assert!(stub.query_messages("abc", 10, None).await.is_err());
        assert!(stub.search_messages("q", None, 10).await.is_err());
        assert!(stub.query_attachment("abc").await.is_err());
        assert!(stub.set_contact("abc", None, None).await.is_err());
        assert!(stub.remove_contact("abc").await.is_err());
        assert!(stub.query_contacts().await.is_err());
        assert!(stub.resolve_name("name", None).await.is_err());

        // DaemonIdentity
        assert!(stub.query_identity().await.is_err());
        assert!(stub.set_identity(None, None, None).await.is_err());
        assert!(stub.announce().await.is_err());

        // DaemonStatus
        assert!(stub.query_status().await.is_err());
        assert!(stub.query_config().await.is_err());
        assert!(stub.query_devices(false).await.is_err());
        assert!(stub.query_path_info("abc").await.is_err());
        assert!(stub.query_auto_reply().await.is_err());
        assert!(stub.set_auto_reply("off", None, None).await.is_err());

        // DaemonFleet
        assert!(stub.device_status("abc", None).await.is_err());
        assert!(stub.exec("abc", "ls", vec![], None).await.is_err());
        assert!(stub.reboot_device("abc", None, None).await.is_err());
        assert!(stub.self_update("abc", None, None).await.is_err());
        assert!(stub.remote_inbox("abc", 10, None).await.is_err());
        assert!(stub.remote_messages("abc", "def", 10, None).await.is_err());
        assert!(
            stub.terminal_open(TerminalOpenRequest::default())
                .await
                .is_err()
        );
        assert!(stub.terminal_input("sid", b"data").await.is_err());
        assert!(stub.terminal_resize("sid", 24, 80).await.is_err());
        assert!(stub.terminal_close("sid").await.is_err());

        // DaemonEvents
        assert!(stub.subscribe_messages(&[]).await.is_err());
        assert!(stub.subscribe_devices().await.is_err());
    }

    /// Verify StubDaemon satisfies the composite Daemon trait and can be
    /// used behind Arc<dyn Daemon>.
    #[tokio::test]
    async fn stub_is_object_safe() {
        let daemon: std::sync::Arc<dyn Daemon> = std::sync::Arc::new(StubDaemon);
        let err = daemon.query_status().await.expect_err("should be stub");
        assert!(matches!(err, IpcError::NotImplemented { .. }));
    }

    #[test]
    fn error_retryable_variants() {
        assert!(!IpcError::not_implemented("x").is_retryable());
        assert!(
            IpcError::Unavailable {
                reason: "down".into()
            }
            .is_retryable()
        );
        assert!(
            IpcError::Timeout {
                operation: "op".into()
            }
            .is_retryable()
        );
        assert!(
            IpcError::Transport {
                message: "err".into()
            }
            .is_retryable()
        );
        assert!(
            !IpcError::InvalidRequest {
                message: "bad".into()
            }
            .is_retryable()
        );
        assert!(
            !IpcError::NotFound {
                resource: "x".into()
            }
            .is_retryable()
        );
        assert!(
            !IpcError::Conflict {
                message: "x".into()
            }
            .is_retryable()
        );
        assert!(
            !IpcError::Internal {
                message: "x".into()
            }
            .is_retryable()
        );
    }
}
