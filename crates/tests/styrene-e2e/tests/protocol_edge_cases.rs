//! Protocol edge cases — wire version rejection, IPC event ordering,
//! cross-client IPC scenarios, and fleet exec on self.

use std::collections::HashMap;
use std::time::Duration;

use tokio::net::UnixStream;

use styrene_e2e::helpers::{
    await_identity_resolved, await_inbound_count, two_connected_nodes, with_timeout, SETTLE,
};
use styrene_e2e::node::TestNodeBuilder;
use styrene_mesh::{StyreneMessage, StyreneMessageType, WIRE_VERSION};
use styrene_rbac::{Role, RosterEntry};

// ── Wire Version Rejection ─────────────────────────────────────────────

#[test]
fn wire_message_rejects_future_version() {
    // Craft a valid-looking message with a future wire version
    let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
    let mut encoded = msg.encode();

    // Byte 11 is the version byte (after 11-byte namespace)
    assert_eq!(encoded[11], WIRE_VERSION, "sanity: current version");

    // Set to a future version
    encoded[11] = WIRE_VERSION + 1;

    let result = StyreneMessage::decode(&encoded);
    assert!(
        result.is_err(),
        "future wire version should be rejected, got: {:?}",
        result.ok().map(|m| m.version)
    );
}

#[test]
fn wire_message_rejects_zero_version() {
    let msg = StyreneMessage::new(StyreneMessageType::Ping, &[]);
    let mut encoded = msg.encode();
    encoded[11] = 0x00;

    let result = StyreneMessage::decode(&encoded);
    assert!(result.is_err(), "version 0 should be rejected");
}

// ── IPC Event Ordering ─────────────────────────────────────────────────

#[tokio::test]
async fn ipc_events_arrive_in_send_order() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-order", "bob-order").await;

        // Subscribe to message events on bob
        let mut event_rx = bob.app_context.events().subscribe_daemon_events();

        // Send 3 messages sequentially
        for i in 0..3 {
            let content = format!("order-{}", i);
            alice.send_chat(&bob.delivery_hash, &content).await.expect(&format!("send {}", i));
            await_inbound_count(&bob.app_context, i + 1, Duration::from_secs(15)).await;
        }

        // Collect message events
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut message_contents = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            if let styrene_ipc::types::DaemonEvent::Message {
                kind: styrene_ipc::types::MessageEventKind::New,
                message,
            } = event
            {
                if !message.content.is_empty() {
                    message_contents.push(message.content.clone());
                }
            }
        }

        // Filter to our test messages
        let ordered: Vec<_> =
            message_contents.iter().filter(|c| c.starts_with("order-")).cloned().collect();

        assert_eq!(ordered.len(), 3, "should have 3 ordered events");
        assert_eq!(ordered[0], "order-0");
        assert_eq!(ordered[1], "order-1");
        assert_eq!(ordered[2], "order-2");
    })
    .await;
}

// ── Cross-Client IPC ───────────────────────────────────────────────────

#[tokio::test]
async fn ipc_send_triggers_event_for_subscriber() {
    with_timeout(async {
        let alice = TestNodeBuilder::new("alice-cross").tcp_server("127.0.0.1:0").build().await;
        let bob = TestNodeBuilder::new("bob-cross")
            .tcp_client(alice.listen_addr.expect("addr"))
            .build()
            .await;

        tokio::time::sleep(SETTLE).await;
        alice.announce().await;
        bob.announce().await;
        await_identity_resolved(&alice.app_context, &bob.delivery_addr, Duration::from_secs(10))
            .await;

        // Start IPC server on bob with event bridge
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("cross.sock");
        let facade = std::sync::Arc::new(styrened::daemon_facade::DaemonFacade::new(
            bob.app_context.clone(),
            bob.identity_hash.clone(),
        )) as std::sync::Arc<dyn styrene_ipc::traits::Daemon>;
        let config = styrene_ipc_server::IpcServerConfig {
            socket_path: socket_path.clone(),
            event_capacity: 64,
        };
        let mut server = styrene_ipc_server::IpcServer::new(facade, config);
        server.start().await.expect("start");

        // Bridge events
        let event_tx = server.event_sender();
        let mut daemon_rx = bob.app_context.events().subscribe_daemon_events();
        tokio::spawn(async move {
            loop {
                match daemon_rx.recv().await {
                    Ok(ev) => {
                        let _ = event_tx.send(ev);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });

        std::mem::forget(dir);
        tokio::time::sleep(SETTLE).await;

        // Client 1: subscribes to message events
        let mut subscriber = UnixStream::connect(&socket_path).await.expect("connect sub");
        {
            let req_id = [0x01u8; 16];
            let (mut read, mut write) = subscriber.split();
            styrene_ipc_server::wire::write_frame_async(
                &mut write,
                styrene_ipc_server::wire::MessageType::SubMessages,
                &req_id,
                &HashMap::new(),
            )
            .await
            .expect("subscribe");
            let _resp = styrene_ipc_server::wire::read_frame_async(&mut read)
                .await
                .expect("subscribe response");
        }

        // Client 2 (alice) sends a chat to bob — this should trigger
        // a message event that client 1 (subscriber) receives
        alice.send_chat(&bob.delivery_hash, "cross-client").await.expect("send");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        // Read event on subscriber connection
        let (mut read, _) = subscriber.into_split();
        let event = tokio::time::timeout(
            Duration::from_secs(5),
            styrene_ipc_server::wire::read_frame_async(&mut read),
        )
        .await
        .expect("timeout")
        .expect("read event");

        assert_eq!(
            event.msg_type,
            styrene_ipc_server::wire::MessageType::EventMessage,
            "subscriber should receive EventMessage"
        );
        assert_eq!(event.payload.get("content").and_then(|v| v.as_str()), Some("cross-client"));
    })
    .await;
}

// ── Fleet Exec on Self ─────────────────────────────────────────────────

#[tokio::test]
async fn fleet_exec_on_own_delivery_hash() {
    with_timeout(async {
        let node = TestNodeBuilder::new("self-exec").tcp_server("127.0.0.1:0").build().await;

        tokio::time::sleep(SETTLE).await;

        // Grant self Admin role (exec requires Admin)
        let entry = RosterEntry::new(&node.identity_hash, Role::Admin);
        node.app_context.policy().grant(entry, node.app_context.store()).expect("grant");

        // Exec on self — expected to fail because RNS links are between
        // two different nodes, not self-loops. The delivery pipeline will
        // time out trying to establish a link to its own destination.
        let result = node
            .app_context
            .fleet()
            .exec(&node.delivery_hash, "echo", &["self-test".into()], Some(3))
            .await;

        assert!(result.is_err(), "fleet exec on self should fail (can't establish self-link)");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timeout") || err.contains("failed"),
            "error should indicate delivery failure, got: {}",
            err
        );
    })
    .await;
}

// ── Multi-Chunk Content Distribution ───────────────────────────────────

#[tokio::test]
async fn content_distribution_multi_chunk() {
    use styrene_content::chunk_bitset::ChunkBitset;
    use styrene_content::chunk_profile::ChunkProfile;
    use styrene_content::content_id::ContentId;
    use styrene_content::impls::ram::RamChunkStore;
    use styrene_content::manifest::{Sig64, StyreneManifest};
    use styrene_content::store::ChunkStore;
    use styrene_content::transport::{ContentEvent, ContentTransport};

    // Use LoRa profile (4KB chunks) with 10KB content → 3 chunks
    let content: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
    let profile = ChunkProfile::LoRa;
    let chunk_size = profile.chunk_size() as usize;
    let chunk_count = ((content.len() + chunk_size - 1) / chunk_size) as u32;
    assert_eq!(chunk_count, 3, "10KB / 4KB = 3 chunks");

    let content_id = ContentId::from_bytes(&content);
    let mut chunk_hashes = heapless::Vec::new();
    for i in 0..chunk_count {
        let start = i as usize * chunk_size;
        let end = (start + chunk_size).min(content.len());
        let _ = chunk_hashes.push(*blake3::hash(&content[start..end]).as_bytes());
    }

    let manifest = StyreneManifest {
        content_id,
        size: content.len() as u64,
        chunk_profile: profile,
        chunk_count,
        chunk_hashes,
        name: heapless::String::try_from("multi-chunk").unwrap_or_default(),
        content_type: heapless::String::try_from("test/data").unwrap_or_default(),
        created_at: 1000,
        creator_identity: [0xAAu8; 16],
        signature: Sig64([0u8; 64]),
    };

    // Seed publisher store
    let publisher_store = {
        let mut store = RamChunkStore::new();
        for i in 0..chunk_count {
            let start = i as usize * chunk_size;
            let end = (start + chunk_size).min(content.len());
            store.write_chunk(content_id, i, &content[start..end]).await.unwrap();
        }
        store
    };

    // Channel transport pair
    let (tx_a, rx_b) = tokio::sync::mpsc::channel(64);
    let (tx_b, rx_a) = tokio::sync::mpsc::channel(64);

    struct ChanTransport {
        tx: tokio::sync::mpsc::Sender<ContentEvent>,
        rx: tokio::sync::mpsc::Receiver<ContentEvent>,
    }
    #[derive(Debug)]
    struct ChanErr;
    impl core::fmt::Display for ChanErr {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "chan")
        }
    }
    impl ContentTransport for ChanTransport {
        type Error = ChanErr;
        async fn broadcast_announce(
            &mut self,
            a: &styrene_content::announce::ResourceAvailableAnnounce,
        ) -> Result<(), ChanErr> {
            self.tx.send(ContentEvent::Announce(*a)).await.map_err(|_| ChanErr)
        }
        async fn send_chunk_request(
            &mut self,
            f: &[u8; 16],
            c: ContentId,
            i: u32,
        ) -> Result<(), ChanErr> {
            self.tx
                .send(ContentEvent::ChunkRequest { from: *f, content_id: c, index: i })
                .await
                .map_err(|_| ChanErr)
        }
        async fn send_chunk_response(
            &mut self,
            _: &[u8; 16],
            c: ContentId,
            i: u32,
            d: &[u8],
        ) -> Result<(), ChanErr> {
            self.tx
                .send(ContentEvent::ChunkResponse { content_id: c, index: i, data: d.to_vec() })
                .await
                .map_err(|_| ChanErr)
        }
        async fn recv_event(&mut self) -> Result<Option<ContentEvent>, ChanErr> {
            Ok(self.rx.recv().await)
        }
    }

    let mut pub_transport = ChanTransport { tx: tx_a, rx: rx_a };
    let dl_transport = ChanTransport { tx: tx_b, rx: rx_b };

    // Broadcast announce
    let mut held = ChunkBitset::new();
    for i in 0..chunk_count {
        held.set(i);
    }
    let manifest_bytes = manifest.encode().expect("encode");
    let manifest_hash = {
        let h = blake3::hash(&manifest_bytes);
        let mut o = [0u8; 16];
        o.copy_from_slice(&h.as_bytes()[..16]);
        o
    };
    let announce = styrene_content::announce::ResourceAvailableAnnounce::new(
        content_id,
        manifest_hash,
        held,
        [0xAAu8; 16],
    );
    pub_transport.broadcast_announce(&announce).await.expect("announce");

    // Downloader
    let mut downloader =
        styrene_content::ContentDistributor::new(RamChunkStore::new(), dl_transport, [0xBBu8; 16]);

    let manifest_clone = manifest.clone();
    let dl_handle = tokio::spawn(async move { downloader.download(&manifest_clone).await });

    // Publisher serves chunks
    let serve_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 16384];
        loop {
            match pub_transport.recv_event().await {
                Ok(Some(ContentEvent::ChunkRequest { from, content_id, index })) => {
                    let n = publisher_store.read_chunk(content_id, index, &mut buf).await.unwrap();
                    pub_transport
                        .send_chunk_response(&from, content_id, index, &buf[..n])
                        .await
                        .unwrap();
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
    });

    let downloaded = dl_handle.await.expect("join").expect("download");
    assert_eq!(downloaded.len(), content.len());
    assert_eq!(downloaded, content);
    assert_eq!(ContentId::from_bytes(&downloaded), content_id);

    serve_handle.abort();
}
