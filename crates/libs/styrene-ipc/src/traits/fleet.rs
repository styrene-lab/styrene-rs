use async_trait::async_trait;

use crate::error::IpcError;
use crate::types::*;

/// Remote device operations over mesh and terminal sessions.
#[async_trait]
pub trait DaemonFleet: Send + Sync {
    /// Query status of a remote device.
    async fn device_status(
        &self,
        dest: &str,
        timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, IpcError>;

    /// Execute a command on a remote device.
    async fn exec(
        &self,
        dest: &str,
        cmd: &str,
        args: Vec<String>,
        timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError>;

    /// Request a remote device to reboot.
    async fn reboot_device(
        &self,
        dest: &str,
        delay: Option<u64>,
        timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError>;

    /// Request a remote device to self-update.
    async fn self_update(
        &self,
        dest: &str,
        version: Option<&str>,
        timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError>;

    /// Fetch conversation list from a remote device's inbox.
    async fn remote_inbox(
        &self,
        dest: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError>;

    /// Fetch messages from a specific conversation on a remote device.
    async fn remote_messages(
        &self,
        dest: &str,
        peer_hash: &str,
        limit: u32,
        timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError>;

    /// Open a terminal session to a remote device.
    async fn terminal_open(&self, request: TerminalOpenRequest) -> Result<SessionId, IpcError>;

    /// Send input data to a terminal session.
    async fn terminal_input(&self, session_id: &str, data: &[u8]) -> Result<bool, IpcError>;

    /// Resize a terminal session.
    async fn terminal_resize(
        &self,
        session_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<bool, IpcError>;

    /// Close a terminal session.
    async fn terminal_close(&self, session_id: &str) -> Result<bool, IpcError>;
}
