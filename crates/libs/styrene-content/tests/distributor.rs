//! Integration test: publish → download round-trip using RamChunkStore + MockTransport.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use styrene_content::{
    announce::ResourceAvailableAnnounce,
    chunk_profile::ChunkProfile,
    content_id::ContentId,
    distributor::ContentDistributor,
    impls::RamChunkStore,
    manifest::StyreneManifest,
    store::ChunkStore,
    transport::{ContentEvent, ContentTransport},
};

/// Minimal mock transport: pushes events into queues between two nodes.
struct MockTransport {
    /// Events queued for this node's recv_event().
    inbox: Arc<Mutex<VecDeque<ContentEvent>>>,
    /// Events this node sends get delivered to the other node's inbox.
    peer_inbox: Arc<Mutex<VecDeque<ContentEvent>>>,
    /// Announcements broadcast (for assertions).
    announces: Arc<Mutex<Vec<ResourceAvailableAnnounce>>>,
}

impl ContentTransport for MockTransport {
    type Error = ();

    async fn broadcast_announce(
        &mut self,
        announce: &ResourceAvailableAnnounce,
    ) -> Result<(), ()> {
        self.announces.lock().unwrap().push(*announce);
        // Also deliver announce to peer's inbox.
        self.peer_inbox
            .lock()
            .unwrap()
            .push_back(ContentEvent::Announce(*announce));
        Ok(())
    }

    async fn send_chunk_request(
        &mut self,
        seeder: &[u8; 16],
        content_id: ContentId,
        index: u32,
    ) -> Result<(), ()> {
        self.peer_inbox.lock().unwrap().push_back(ContentEvent::ChunkRequest {
            from: *seeder,
            content_id,
            index,
        });
        Ok(())
    }

    async fn send_chunk_response(
        &mut self,
        _to: &[u8; 16],
        content_id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), ()> {
        self.peer_inbox.lock().unwrap().push_back(ContentEvent::ChunkResponse {
            content_id,
            index,
            data: data.to_vec(),
        });
        Ok(())
    }

    async fn recv_event(&mut self) -> Result<Option<ContentEvent>, ()> {
        Ok(self.inbox.lock().unwrap().pop_front())
    }
}

fn make_pair() -> (MockTransport, MockTransport) {
    let a_inbox = Arc::new(Mutex::new(VecDeque::new()));
    let b_inbox = Arc::new(Mutex::new(VecDeque::new()));
    let announces = Arc::new(Mutex::new(Vec::new()));

    let a = MockTransport {
        inbox: a_inbox.clone(),
        peer_inbox: b_inbox.clone(),
        announces: announces.clone(),
    };
    let b = MockTransport {
        inbox: b_inbox,
        peer_inbox: a_inbox,
        announces,
    };
    (a, b)
}

fn dummy_sign(_: &[u8]) -> [u8; 64] { [0xBBu8; 64] }

#[tokio::test]
async fn publish_then_self_download() {
    let content = b"styrened-rs firmware v0.2.0 binary stub data for testing xxxxxxxx";

    let manifest = StyreneManifest::build(
        content,
        "styrened-rs",
        "firmware/styrened-rs",
        ChunkProfile::LoRa,
        1_700_000_000,
        [0u8; 16],
        dummy_sign,
    )
    .unwrap();

    // Single node: publish and then download from itself (self-seeder).
    let (tx, _rx) = make_pair();
    let mut dist = ContentDistributor::new(RamChunkStore::new(), tx, [0xAAu8; 16]);

    dist.publish(&manifest, content).await.unwrap();

    // Verify all chunks are stored.
    for i in 0..manifest.chunk_count {
        assert!(dist.store().has_chunk(manifest.content_id, i).await);
    }

    // Download (reads from own store since all chunks are present).
    let assembled = dist.download(&manifest).await.unwrap();
    assert_eq!(assembled.as_slice(), content);
}

#[tokio::test]
async fn chunk_verification_rejects_tampered_data() {
    let content = b"some content to protect";

    let manifest = StyreneManifest::build(
        content,
        "test",
        "data/test",
        ChunkProfile::LoRa,
        0,
        [0u8; 16],
        dummy_sign,
    )
    .unwrap();

    // Tamper with the chunk after storing.
    let (tx, _rx) = make_pair();
    let mut dist = ContentDistributor::new(RamChunkStore::new(), tx, [0u8; 16]);

    // Manually write a corrupted chunk.
    dist.store()
        .write_chunk(manifest.content_id, 0, b"corrupted data!!!")
        .await
        .unwrap();

    // Verify the manifest rejects it.
    assert!(!manifest.verify_chunk(0, b"corrupted data!!!"));
    assert!(manifest.verify_chunk(0, content));
}

#[tokio::test]
async fn announce_broadcast_on_publish() {
    let content = b"broadcast test content";
    let manifest = StyreneManifest::build(
        content, "broadcast", "data/test", ChunkProfile::LoRa, 0, [0u8; 16], dummy_sign,
    ).unwrap();

    let (tx, _rx) = make_pair();
    let announces = tx.announces.clone();
    let mut dist = ContentDistributor::new(RamChunkStore::new(), tx, [0xCCu8; 16]);

    dist.publish(&manifest, content).await.unwrap();

    let sent = announces.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].content_id, manifest.content_id);
    assert_eq!(sent[0].seeder_hash, [0xCCu8; 16]);
    assert!(sent[0].is_complete_seeder(manifest.chunk_count));
}
