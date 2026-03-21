//! Integration tests for the IPC server.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use styrene_ipc::error::IpcError;
use styrene_ipc::traits::*;
use styrene_ipc::types::*;
use styrene_ipc_server::wire::{self, MessageType, REQUEST_ID_SIZE};
use styrene_ipc_server::{IpcServer, IpcServerConfig};
use tokio::net::UnixStream;

// ── Test daemon ─────────────────────────────────────────────────────────

struct TestDaemon;

#[async_trait]
impl DaemonStatus for TestDaemon {
    async fn query_status(&self) -> Result<DaemonStatusInfo, IpcError> {
        let mut info = DaemonStatusInfo::default();
        info.uptime = 42;
        info.daemon_version = "test-0.1.0".into();
        info.rns_initialized = true;
        Ok(info)
    }
    async fn query_config(&self) -> Result<ConfigSnapshot, IpcError> {
        Ok(ConfigSnapshot::default())
    }
    async fn query_devices(&self, _styrene_only: bool) -> Result<Vec<DeviceInfo>, IpcError> {
        let mut d = DeviceInfo::default();
        d.destination_hash = "abcd1234".into();
        d.name = "test-node".into();
        Ok(vec![d])
    }
    async fn query_path_info(&self, _dest: &str) -> Result<PathInfo, IpcError> {
        Ok(PathInfo::default())
    }
    async fn query_auto_reply(&self) -> Result<AutoReplyConfig, IpcError> {
        Ok(AutoReplyConfig::default())
    }
    async fn set_auto_reply(
        &self,
        _mode: &str,
        _msg: Option<&str>,
        _cd: Option<u64>,
    ) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn save_config(&self, _config: ConfigSnapshot) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn block_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn unblock_peer(&self, _hash: &str) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn blocked_peers(&self) -> Result<Vec<String>, IpcError> {
        Ok(vec![])
    }
}

#[async_trait]
impl DaemonIdentity for TestDaemon {
    async fn query_identity(&self) -> Result<IdentityInfo, IpcError> {
        let mut info = IdentityInfo::default();
        info.identity_hash = "deadbeef".into();
        info.display_name = "Test Node".into();
        Ok(info)
    }
    async fn set_identity(
        &self,
        _name: Option<&str>,
        _icon: Option<&str>,
        _short: Option<&str>,
    ) -> Result<bool, IpcError> {
        Ok(true)
    }
    async fn announce(&self) -> Result<bool, IpcError> {
        Ok(true)
    }
}

#[async_trait]
impl DaemonMessaging for TestDaemon {
    async fn send_chat(&self, _req: SendChatRequest) -> Result<MessageId, IpcError> {
        Err(IpcError::not_implemented("send_chat"))
    }
    async fn mark_read(&self, _peer: &str) -> Result<u64, IpcError> {
        Ok(0)
    }
    async fn delete_conversation(&self, _peer: &str) -> Result<u64, IpcError> {
        Ok(0)
    }
    async fn delete_message(&self, _id: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn retry_message(&self, _id: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn query_conversations(&self, _unread: bool) -> Result<Vec<ConversationInfo>, IpcError> {
        Ok(vec![])
    }
    async fn query_messages(
        &self,
        _peer: &str,
        _limit: u32,
        _before: Option<i64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Ok(vec![])
    }
    async fn search_messages(
        &self,
        _q: &str,
        _peer: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Ok(vec![])
    }
    async fn query_attachment(&self, _id: &str) -> Result<Vec<u8>, IpcError> {
        Ok(vec![])
    }
    async fn set_contact(
        &self,
        _peer: &str,
        _alias: Option<&str>,
        _notes: Option<&str>,
    ) -> Result<ContactInfo, IpcError> {
        Ok(ContactInfo::default())
    }
    async fn remove_contact(&self, _peer: &str) -> Result<bool, IpcError> {
        Ok(false)
    }
    async fn query_contacts(&self) -> Result<Vec<ContactInfo>, IpcError> {
        Ok(vec![])
    }
    async fn resolve_name(
        &self,
        _name: &str,
        _prefix: Option<&str>,
    ) -> Result<Option<PeerHash>, IpcError> {
        Ok(None)
    }
}

#[async_trait]
impl DaemonFleet for TestDaemon {
    async fn device_status(
        &self,
        _dest: &str,
        _timeout: Option<u64>,
    ) -> Result<RemoteStatusInfo, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn exec(
        &self,
        _dest: &str,
        _cmd: &str,
        _args: Vec<String>,
        _timeout: Option<u64>,
    ) -> Result<ExecResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn reboot_device(
        &self,
        _dest: &str,
        _delay: Option<u64>,
        _timeout: Option<u64>,
    ) -> Result<RebootResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn self_update(
        &self,
        _dest: &str,
        _version: Option<&str>,
        _timeout: Option<u64>,
    ) -> Result<SelfUpdateResult, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn remote_inbox(
        &self,
        _dest: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<ConversationInfo>, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn remote_messages(
        &self,
        _dest: &str,
        _peer_hash: &str,
        _limit: u32,
        _timeout: Option<u64>,
    ) -> Result<Vec<MessageInfo>, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_open(&self, _req: TerminalOpenRequest) -> Result<SessionId, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_input(&self, _session: &str, _data: &[u8]) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_resize(
        &self,
        _session: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
    async fn terminal_close(&self, _session: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("fleet"))
    }
}

#[async_trait]
impl DaemonEvents for TestDaemon {
    async fn subscribe_messages(
        &self,
        _peers: &[String],
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }
    async fn subscribe_devices(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<DaemonEvent>, IpcError> {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        drop(tx);
        Ok(rx)
    }
}

#[async_trait]
impl DaemonTunnel for TestDaemon {
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, IpcError> {
        Ok(vec![])
    }
    async fn tunnel_status(&self, _peer: &str) -> Result<TunnelInfo, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn tunnel_rekey(&self, _peer: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn tunnel_teardown(&self, _peer: &str) -> Result<bool, IpcError> {
        Err(IpcError::not_implemented("tunnel"))
    }
    async fn list_tunnel_sas(&self, _peer: &str) -> Result<Vec<TunnelSaInfo>, IpcError> {
        Ok(vec![])
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

async fn setup_server() -> (IpcServer, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock_path = dir.path().join("test.sock");
    let sock = sock_path.clone();
    std::mem::forget(dir);

    let config = IpcServerConfig {
        socket_path: sock.clone(),
        event_capacity: 64,
    };
    let daemon: Arc<dyn Daemon> = Arc::new(TestDaemon);
    let mut server = IpcServer::new(daemon, config);
    server.start().await.expect("start");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    (server, sock)
}

fn gen_request_id() -> [u8; REQUEST_ID_SIZE] {
    let mut id = [0u8; REQUEST_ID_SIZE];
    id[0] = 42;
    id[15] = 99;
    id
}

async fn send_and_recv(
    stream: &mut UnixStream,
    msg_type: MessageType,
    payload: &HashMap<String, rmpv::Value>,
) -> wire::Frame {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let req_id = gen_request_id();
    let bytes = wire::encode_frame(msg_type, &req_id, payload).expect("encode");
    stream.write_all(&bytes).await.expect("write");
    stream.flush().await.expect("flush");

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.expect("read len");
    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read frame");

    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    wire::decode_frame(&full).expect("decode")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn ping_pong() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Pong);
    server.stop().await;
}

#[tokio::test]
async fn query_status() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryStatus, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    assert_eq!(resp.payload.get("uptime").and_then(|v| v.as_u64()), Some(42));
    assert_eq!(
        resp.payload.get("daemon_version").and_then(|v| v.as_str()),
        Some("test-0.1.0")
    );
    server.stop().await;
}

#[tokio::test]
async fn query_identity() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryIdentity, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    assert_eq!(
        resp.payload.get("identity_hash").and_then(|v| v.as_str()),
        Some("deadbeef")
    );
    server.stop().await;
}

#[tokio::test]
async fn query_devices() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::QueryDevices, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);
    let devices = resp.payload.get("devices").and_then(|v| v.as_array());
    assert!(devices.is_some());
    assert_eq!(devices.expect("arr").len(), 1);
    server.stop().await;
}

#[tokio::test]
async fn unknown_message_returns_error() {
    let (mut server, sock) = setup_server().await;
    let mut stream = UnixStream::connect(&sock).await.expect("connect");
    let resp = send_and_recv(&mut stream, MessageType::CmdRemoteMessages, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Error);
    assert!(resp.payload.get("error").and_then(|v| v.as_str()).is_some());
    server.stop().await;
}

#[tokio::test]
async fn multiple_concurrent_clients() {
    let (mut server, sock) = setup_server().await;
    let s1 = sock.clone();
    let s2 = sock.clone();

    let h1 = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&s1).await.expect("c1");
        let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
        assert_eq!(resp.msg_type, MessageType::Pong);
    });
    let h2 = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&s2).await.expect("c2");
        let resp = send_and_recv(&mut stream, MessageType::Ping, &HashMap::new()).await;
        assert_eq!(resp.msg_type, MessageType::Pong);
    });

    h1.await.expect("c1");
    h2.await.expect("c2");
    server.stop().await;
}

#[tokio::test]
async fn stop_removes_socket() {
    let (mut server, sock) = setup_server().await;
    assert!(sock.exists());
    server.stop().await;
    assert!(!sock.exists());
}

#[tokio::test]
async fn subscribe_and_event_push() {
    let (mut server, sock) = setup_server().await;
    let event_tx = server.event_sender();

    let mut stream = UnixStream::connect(&sock).await.expect("connect");

    // Subscribe to devices
    let resp = send_and_recv(&mut stream, MessageType::SubDevices, &HashMap::new()).await;
    assert_eq!(resp.msg_type, MessageType::Result);

    // Push an event
    let mut device = DeviceInfo::default();
    device.destination_hash = "event-device".into();
    device.name = "pushed-node".into();
    let _ = event_tx.send(DaemonEvent::Device { device });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Read the pushed event frame
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        stream.read_exact(&mut len_buf),
    )
    .await
    .expect("timeout")
    .expect("read len");

    let total = u32::from_be_bytes(len_buf) as usize;
    let mut frame_buf = vec![0u8; total];
    stream.read_exact(&mut frame_buf).await.expect("read");

    let mut full = Vec::with_capacity(4 + total);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&frame_buf);
    let event_frame = wire::decode_frame(&full).expect("decode");

    assert_eq!(event_frame.msg_type, MessageType::EventDevice);
    assert_eq!(
        event_frame.payload.get("destination_hash").and_then(|v| v.as_str()),
        Some("event-device")
    );

    server.stop().await;
}
