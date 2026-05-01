//! IPC server e2e — Unix socket client→daemon round-trip.
//!
//! Spawns a real IpcServer on a temp socket backed by a real daemon node,
//! connects as a raw client, sends framed msgpack requests, and verifies
//! typed responses. This is the path nex CLI and TUI actually use.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::UnixStream;

use styrene_e2e::helpers::{with_timeout, SETTLE};
use tokio::io::AsyncWriteExt;
use styrene_e2e::node::TestNodeBuilder;
use styrened::daemon_facade::DaemonFacade;
use styrene_ipc::traits::Daemon;
use styrene_ipc_server::wire::{
    self, encode_frame, MessageType, REQUEST_ID_SIZE,
};

fn random_request_id() -> [u8; REQUEST_ID_SIZE] {
    let mut id = [0u8; 16];
    use rand_core::RngCore;
    rand_core::OsRng.fill_bytes(&mut id);
    id
}

fn empty_payload() -> HashMap<String, rmpv::Value> {
    HashMap::new()
}

/// Start an IPC server on a temp socket, returning the server and socket path.
async fn start_ipc_server(
    node: &styrene_e2e::node::TestNode,
) -> (styrene_ipc_server::IpcServer, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("test.sock");

    let facade = Arc::new(DaemonFacade::new(
        node.app_context.clone(),
        node.identity_hash.clone(),
    )) as Arc<dyn Daemon>;

    let config = styrene_ipc_server::IpcServerConfig {
        socket_path: socket_path.clone(),
        event_capacity: 64,
    };

    let mut server = styrene_ipc_server::IpcServer::new(facade, config);
    server.start().await.expect("start ipc server");

    // Keep tempdir alive by leaking it (socket needs the directory)
    std::mem::forget(dir);

    (server, socket_path)
}

/// Send a request and read the response over a Unix stream.
async fn request(
    stream: &mut UnixStream,
    msg_type: MessageType,
    payload: &HashMap<String, rmpv::Value>,
) -> wire::Frame {
    let request_id = random_request_id();
    let (mut read, mut write) = stream.split();
    wire::write_frame_async(&mut write, msg_type, &request_id, payload)
        .await
        .expect("write frame");
    let frame = wire::read_frame_async(&mut read).await.expect("read frame");
    assert_eq!(
        frame.request_id, request_id,
        "response should echo request_id"
    );
    frame
}

// ── Tests ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn ping_pong() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-ping")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        let frame = request(&mut stream, MessageType::Ping, &empty_payload()).await;

        assert_eq!(frame.msg_type, MessageType::Pong, "ping should get pong");
    })
    .await;
}

#[tokio::test]
async fn query_status_returns_daemon_info() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-status")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        let frame = request(&mut stream, MessageType::QueryStatus, &empty_payload()).await;

        assert_eq!(frame.msg_type, MessageType::Result);

        // Should contain daemon version
        let version = frame
            .payload
            .get("daemon_version")
            .and_then(|v| v.as_str());
        assert!(
            version.is_some(),
            "status should include daemon_version, payload: {:?}",
            frame.payload
        );

        // Should report transport initialized
        let rns_init = frame
            .payload
            .get("rns_initialized")
            .and_then(|v| v.as_bool());
        assert_eq!(rns_init, Some(true), "transport should be initialized");
    })
    .await;
}

#[tokio::test]
async fn query_identity_returns_hashes() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-identity")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        let frame = request(&mut stream, MessageType::QueryIdentity, &empty_payload()).await;

        assert_eq!(frame.msg_type, MessageType::Result);

        let identity_hash = frame
            .payload
            .get("identity_hash")
            .and_then(|v| v.as_str())
            .expect("should have identity_hash");
        assert_eq!(identity_hash, node.identity_hash);

        let dest_hash = frame
            .payload
            .get("destination_hash")
            .and_then(|v| v.as_str())
            .expect("should have destination_hash");
        assert!(!dest_hash.is_empty());
    })
    .await;
}

#[tokio::test]
async fn announce_via_ipc() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-announce")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        let frame = request(&mut stream, MessageType::CmdAnnounce, &empty_payload()).await;

        assert_eq!(
            frame.msg_type,
            MessageType::Result,
            "announce should succeed, got: {:?}",
            frame.payload
        );
    })
    .await;
}

#[tokio::test]
async fn multiple_requests_on_same_connection() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-multi")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Send 5 requests on the same connection
        for i in 0..5 {
            let frame = request(&mut stream, MessageType::QueryStatus, &empty_payload()).await;
            assert_eq!(
                frame.msg_type,
                MessageType::Result,
                "request {} should succeed",
                i
            );
        }
    })
    .await;
}

#[tokio::test]
async fn concurrent_clients() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-concurrent")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        // Spawn 3 concurrent clients
        let mut handles = Vec::new();
        for i in 0..3 {
            let path = socket_path.clone();
            handles.push(tokio::spawn(async move {
                let mut stream = UnixStream::connect(&path).await.expect("connect");
                let frame =
                    request(&mut stream, MessageType::QueryIdentity, &empty_payload()).await;
                assert_eq!(frame.msg_type, MessageType::Result, "client {} failed", i);
                frame
                    .payload
                    .get("identity_hash")
                    .and_then(|v| v.as_str())
                    .expect("identity_hash")
                    .to_string()
            }));
        }

        // All should return the same identity hash
        let mut hashes = Vec::new();
        for h in handles {
            hashes.push(h.await.expect("join"));
        }
        assert_eq!(hashes[0], hashes[1]);
        assert_eq!(hashes[1], hashes[2]);
        assert_eq!(hashes[0], node.identity_hash);
    })
    .await;
}

#[tokio::test]
async fn query_contacts_via_ipc() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-contacts")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Set a contact via DaemonFacade directly (to have something to query)
        let facade = DaemonFacade::new(node.app_context.clone(), node.identity_hash.clone());
        use styrene_ipc::traits::DaemonMessaging;
        facade
            .set_contact("aabbccddaabbccddaabbccddaabbccdd", Some("TestPeer"), None)
            .await
            .expect("set contact");

        // Query contacts via IPC
        let frame = request(&mut stream, MessageType::QueryContacts, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);

        let contacts = frame
            .payload
            .get("contacts")
            .and_then(|v| v.as_array())
            .expect("should have contacts array");
        assert_eq!(contacts.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn auto_reply_crud_via_ipc() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-autoreply")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Query default auto-reply config
        let frame = request(&mut stream, MessageType::QueryAutoReply, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);
        let mode = frame.payload.get("mode").and_then(|v| v.as_str());
        assert_eq!(mode, Some("disabled"));

        // Set auto-reply via IPC
        let mut set_payload = HashMap::new();
        set_payload.insert("mode".to_string(), rmpv::Value::from("all"));
        set_payload.insert("message".to_string(), rmpv::Value::from("Away"));
        set_payload.insert("cooldown_secs".to_string(), rmpv::Value::from(60));

        let frame = request(&mut stream, MessageType::CmdSetAutoReply, &set_payload).await;
        assert_eq!(frame.msg_type, MessageType::Result);

        // Query again to verify
        let frame = request(&mut stream, MessageType::QueryAutoReply, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);
        let mode = frame.payload.get("mode").and_then(|v| v.as_str());
        assert_eq!(mode, Some("all"));
        let message = frame.payload.get("message").and_then(|v| v.as_str());
        assert_eq!(message, Some("Away"));
    })
    .await;
}

#[tokio::test]
async fn ipc_reflects_state_after_real_message_delivery() {
    with_timeout(async {
        // Two nodes connected with real TCP
        let alice = TestNodeBuilder::new("alice-ipc-state")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;
        let bob = TestNodeBuilder::new("bob-ipc-state")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;

        styrene_e2e::helpers::await_identity_resolved(
            &alice.app_context,
            &bob.delivery_addr,
            std::time::Duration::from_secs(10),
        )
        .await;

        // Start IPC server on bob
        let (_server, socket_path) = start_ipc_server(&bob).await;
        tokio::time::sleep(SETTLE).await;

        // Alice sends a real message to bob over TCP
        alice
            .send_chat(&bob.delivery_hash, "ipc-state-test")
            .await
            .expect("send");
        styrene_e2e::helpers::await_inbound_count(
            &bob.app_context,
            1,
            std::time::Duration::from_secs(15),
        )
        .await;

        // Now query conversations via IPC — should reflect the delivered message
        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");
        let frame = request(&mut stream, MessageType::QueryConversations, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);

        let conversations = frame
            .payload
            .get("conversations")
            .and_then(|v| v.as_array())
            .expect("should have conversations");
        assert!(
            !conversations.is_empty(),
            "IPC should reflect the delivered message in conversations"
        );

        // Query messages for the specific conversation
        let mut msg_payload = HashMap::new();
        let peer_hash = conversations[0]
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("peer_hash"))
                    .and_then(|(_, v)| v.as_str())
            })
            .expect("peer_hash in conversation");
        msg_payload.insert("peer_hash".to_string(), rmpv::Value::from(peer_hash));
        msg_payload.insert("limit".to_string(), rmpv::Value::from(10));

        let frame = request(&mut stream, MessageType::QueryMessages, &msg_payload).await;
        assert_eq!(frame.msg_type, MessageType::Result);

        let messages = frame
            .payload
            .get("messages")
            .and_then(|v| v.as_array())
            .expect("should have messages");
        assert!(!messages.is_empty(), "IPC should return the delivered message");

        // Verify message content
        let content = messages[0]
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("content"))
                    .and_then(|(_, v)| v.as_str())
            });
        assert_eq!(content, Some("ipc-state-test"));
    })
    .await;
}

#[tokio::test]
async fn error_response_for_unimplemented_method() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-error")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Terminal operations are not implemented — should get an error
        let frame = request(&mut stream, MessageType::CmdTerminalOpen, &empty_payload()).await;
        assert_eq!(
            frame.msg_type,
            MessageType::Error,
            "unimplemented method should return Error"
        );
    })
    .await;
}

#[tokio::test]
async fn malformed_client_does_not_crash_server() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-malformed")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        // Client 1: sends garbage bytes then disconnects
        {
            let mut bad_stream = UnixStream::connect(&socket_path).await.expect("connect");
            bad_stream.write_all(b"this is not a valid frame").await.expect("write garbage");
            drop(bad_stream);
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Client 2: sends a truncated frame (length prefix says 1000 bytes, only sends 5)
        {
            let mut bad_stream = UnixStream::connect(&socket_path).await.expect("connect");
            let fake_len: u32 = 1000;
            bad_stream
                .write_all(&fake_len.to_be_bytes())
                .await
                .expect("write len");
            bad_stream.write_all(&[0x01, 0x02, 0x03, 0x04, 0x05]).await.expect("write partial");
            drop(bad_stream);
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Client 3: a legitimate client should still work after the bad ones
        let mut good_stream = UnixStream::connect(&socket_path).await.expect("connect good");
        let frame = request(&mut good_stream, MessageType::Ping, &empty_payload()).await;
        assert_eq!(
            frame.msg_type,
            MessageType::Pong,
            "server should still be alive after malformed clients"
        );
    })
    .await;
}

#[tokio::test]
async fn client_disconnect_mid_session_does_not_affect_others() {
    with_timeout(async {
        let node = TestNodeBuilder::new("ipc-disconnect")
            .tcp_server("127.0.0.1:0")
            .build()
            .await;

        let (_server, socket_path) = start_ipc_server(&node).await;
        tokio::time::sleep(SETTLE).await;

        // Client A: connect, send one request, then abruptly disconnect
        {
            let mut stream_a = UnixStream::connect(&socket_path).await.expect("connect A");
            let frame = request(&mut stream_a, MessageType::Ping, &empty_payload()).await;
            assert_eq!(frame.msg_type, MessageType::Pong);
            // Drop without clean shutdown
        }

        // Client B: should work fine after A disconnected
        let mut stream_b = UnixStream::connect(&socket_path).await.expect("connect B");
        let frame = request(&mut stream_b, MessageType::QueryStatus, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);

        // Send multiple requests to verify B's connection is stable
        let frame = request(&mut stream_b, MessageType::QueryIdentity, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);
    })
    .await;
}
