//! Daemon unified binary self-test.
//!
//! Tests the `styrened::daemon::start()` API — the same code path used by
//! `styrene daemon`. Boots a real daemon with ephemeral identity, connects
//! an IPC client, and exercises the full path.

use std::collections::HashMap;
use std::time::Duration;

use tokio::net::UnixStream;

use styrene_e2e::helpers::with_timeout;
use styrened::daemon::{DaemonConfig2, start};
use styrene_ipc_server::wire::{self, MessageType, REQUEST_ID_SIZE};

fn random_request_id() -> [u8; REQUEST_ID_SIZE] {
    let mut id = [0u8; 16];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut id);
    id
}

fn empty_payload() -> HashMap<String, rmpv::Value> {
    HashMap::new()
}

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
    assert_eq!(frame.request_id, request_id);
    frame
}

#[tokio::test]
async fn daemon_start_and_query_via_ipc() {
    with_timeout(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("daemon-test.sock");
        let db_path = dir.path().join("test.db");

        let handle = start(DaemonConfig2 {
            db: Some(db_path),
            config: None,
            identity: None,
            socket: Some(socket_path.clone()),
            ephemeral: true,
        })
        .await
        .expect("daemon start");

        // Give the IPC server time to bind
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Connect IPC client
        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Ping
        let frame = request(&mut stream, MessageType::Ping, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Pong);

        // Query status
        let frame = request(&mut stream, MessageType::QueryStatus, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);
        assert_eq!(
            frame.payload.get("rns_initialized").and_then(|v| v.as_bool()),
            Some(true),
            "transport should be initialized"
        );
        let version = frame
            .payload
            .get("daemon_version")
            .and_then(|v| v.as_str());
        assert!(version.is_some(), "should have daemon version");

        // Query identity
        let frame = request(&mut stream, MessageType::QueryIdentity, &empty_payload()).await;
        assert_eq!(frame.msg_type, MessageType::Result);
        let identity_hash = frame
            .payload
            .get("identity_hash")
            .and_then(|v| v.as_str())
            .expect("identity_hash");
        assert_eq!(identity_hash.len(), 32, "identity hash should be 32 hex chars");

        // Verify it matches the handle's app_context
        assert_eq!(
            identity_hash,
            hex::encode(
                handle
                    .app_context
                    .identity()
                    .transport_identity_hash()
                    .as_slice()
            )
        );

        // Clean shutdown
        drop(handle);
    })
    .await;
}

#[tokio::test]
async fn daemon_ipc_events_bridge_works() {
    with_timeout(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("events-test.sock");
        let db_path = dir.path().join("test.db");

        let handle = start(DaemonConfig2 {
            db: Some(db_path),
            config: None,
            identity: None,
            socket: Some(socket_path.clone()),
            ephemeral: true,
        })
        .await
        .expect("daemon start");

        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Subscribe to devices via IPC wire protocol
        let req_id = random_request_id();
        let (mut read, mut write) = stream.split();
        wire::write_frame_async(
            &mut write,
            MessageType::SubDevices,
            &req_id,
            &empty_payload(),
        )
        .await
        .expect("write subscribe");
        let frame = wire::read_frame_async(&mut read).await.expect("read response");
        assert_eq!(frame.msg_type, MessageType::Result);
        drop((read, write));

        // Emit a device event through the daemon's EventService
        handle.app_context.events().emit_device_update("test-peer-hash");

        // Read the pushed event from the IPC socket
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (mut read, _write) = stream.into_split();
        let event_result = tokio::time::timeout(
            Duration::from_secs(3),
            wire::read_frame_async(&mut read),
        )
        .await;

        match event_result {
            Ok(Ok(frame)) => {
                assert_eq!(frame.msg_type, MessageType::EventDevice);
                assert_eq!(frame.request_id, [0u8; 16], "events have zero request_id");
                assert_eq!(
                    frame.payload.get("destination_hash").and_then(|v| v.as_str()),
                    Some("test-peer-hash")
                );
            }
            Ok(Err(e)) => panic!("read error: {e}"),
            Err(_) => panic!(
                "timed out waiting for IPC event — \
                 the event bridge between EventService and IpcServer may not be wired"
            ),
        }

        drop(handle);
    })
    .await;
}

#[tokio::test]
async fn daemon_announce_via_ipc() {
    with_timeout(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("announce-test.sock");
        let db_path = dir.path().join("test.db");

        let _handle = start(DaemonConfig2 {
            db: Some(db_path),
            config: None,
            identity: None,
            socket: Some(socket_path.clone()),
            ephemeral: true,
        })
        .await
        .expect("daemon start");

        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut stream = UnixStream::connect(&socket_path).await.expect("connect");

        // Announce via IPC
        let frame = request(&mut stream, MessageType::CmdAnnounce, &empty_payload()).await;
        assert_eq!(
            frame.msg_type,
            MessageType::Result,
            "announce via daemon IPC should succeed"
        );
    })
    .await;
}
