//! Content distribution scenarios.
//!
//! Tests the ContentDistributor state machine with channel-based transport
//! to verify publish, announce propagation, chunk request/response, BLAKE3
//! chunk verification, and content reassembly.

use styrene_content::announce::ResourceAvailableAnnounce;
use styrene_content::chunk_bitset::ChunkBitset;
use styrene_content::chunk_profile::ChunkProfile;
use styrene_content::content_id::ContentId;
use styrene_content::impls::ram::RamChunkStore;
use styrene_content::manifest::{Sig64, StyreneManifest};
use styrene_content::store::ChunkStore;
use styrene_content::transport::{ContentEvent, ContentTransport};

use styrene_e2e::helpers::with_timeout;

/// In-process paired transport for testing. Two ends connected by channels.
struct ChannelTransport {
    tx: tokio::sync::mpsc::Sender<ContentEvent>,
    rx: tokio::sync::mpsc::Receiver<ContentEvent>,
}

#[derive(Debug)]
struct ChannelError;
impl core::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "channel error")
    }
}

impl ContentTransport for ChannelTransport {
    type Error = ChannelError;

    async fn broadcast_announce(
        &mut self,
        announce: &ResourceAvailableAnnounce,
    ) -> Result<(), Self::Error> {
        self.tx
            .send(ContentEvent::Announce(*announce))
            .await
            .map_err(|_| ChannelError)
    }

    async fn send_chunk_request(
        &mut self,
        from: &[u8; 16],
        content_id: ContentId,
        index: u32,
    ) -> Result<(), Self::Error> {
        self.tx
            .send(ContentEvent::ChunkRequest {
                from: *from,
                content_id,
                index,
            })
            .await
            .map_err(|_| ChannelError)
    }

    async fn send_chunk_response(
        &mut self,
        _to: &[u8; 16],
        content_id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        self.tx
            .send(ContentEvent::ChunkResponse {
                content_id,
                index,
                data: data.to_vec(),
            })
            .await
            .map_err(|_| ChannelError)
    }

    async fn recv_event(&mut self) -> Result<Option<ContentEvent>, Self::Error> {
        Ok(self.rx.recv().await)
    }
}

fn make_channel_pair() -> (ChannelTransport, ChannelTransport) {
    let (tx_a, rx_b) = tokio::sync::mpsc::channel(64);
    let (tx_b, rx_a) = tokio::sync::mpsc::channel(64);
    (
        ChannelTransport { tx: tx_a, rx: rx_a },
        ChannelTransport { tx: tx_b, rx: rx_b },
    )
}

fn test_content() -> Vec<u8> {
    (0..100 * 1024).map(|i| (i % 256) as u8).collect()
}

fn build_manifest(content: &[u8], profile: ChunkProfile, signer_hash: [u8; 16]) -> StyreneManifest {
    let content_id = ContentId::from_bytes(content);
    let chunk_size = profile.chunk_size() as usize;
    let chunk_count = ((content.len() + chunk_size - 1) / chunk_size) as u32;

    let mut chunk_hashes = heapless::Vec::new();
    for i in 0..chunk_count {
        let start = i as usize * chunk_size;
        let end = (start + chunk_size).min(content.len());
        let hash = blake3::hash(&content[start..end]);
        let _ = chunk_hashes.push(*hash.as_bytes());
    }

    StyreneManifest {
        content_id,
        size: content.len() as u64,
        chunk_profile: profile,
        chunk_count,
        chunk_hashes,
        name: heapless::String::try_from("test-content").unwrap_or_default(),
        content_type: heapless::String::try_from("test/data").unwrap_or_default(),
        created_at: 1000,
        creator_identity: signer_hash,
        signature: Sig64([0u8; 64]),
    }
}

async fn seed_store(content: &[u8], manifest: &StyreneManifest) -> RamChunkStore {
    let mut store = RamChunkStore::new();
    let chunk_size = manifest.chunk_profile.chunk_size() as usize;
    for i in 0..manifest.chunk_count {
        let start = i as usize * chunk_size;
        let end = (start + chunk_size).min(content.len());
        store
            .write_chunk(manifest.content_id, i, &content[start..end])
            .await
            .expect("write chunk");
    }
    store
}

#[tokio::test]
async fn publish_and_download_via_channel_transport() {
    with_timeout(async {
        let content = test_content();
        let profile = ChunkProfile::WiFi;
        let publisher_id = [0xAAu8; 16];
        let downloader_id = [0xBBu8; 16];
        let manifest = build_manifest(&content, profile, publisher_id);

        // Seed the publisher's store with all chunks
        let publisher_store = seed_store(&content, &manifest).await;

        // Channels: publisher_side ↔ downloader_side
        let (mut publisher_transport, downloader_transport) = make_channel_pair();

        // Build announce and send it to the downloader
        let mut held = ChunkBitset::new();
        for i in 0..manifest.chunk_count {
            held.set(i);
        }
        let manifest_bytes = manifest.encode().expect("encode manifest");
        let manifest_hash = {
            let h = blake3::hash(&manifest_bytes);
            let mut out = [0u8; 16];
            out.copy_from_slice(&h.as_bytes()[..16]);
            out
        };
        let announce = ResourceAvailableAnnounce::new(
            manifest.content_id,
            manifest_hash,
            held,
            publisher_id,
        );
        publisher_transport
            .broadcast_announce(&announce)
            .await
            .expect("announce");

        // Downloader: ContentDistributor that will download chunks.
        // download() drains pending events (including the announce) before
        // checking for seeders, so we can call it directly.
        let mut downloader = styrene_content::ContentDistributor::new(
            RamChunkStore::new(),
            downloader_transport,
            downloader_id,
        );

        let manifest_for_dl = manifest.clone();
        let download_handle = tokio::spawn(async move {
            downloader.download(&manifest_for_dl).await
        });

        // Publisher: manually serve chunk requests from the channel
        let serve_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                match publisher_transport.recv_event().await {
                    Ok(Some(ContentEvent::ChunkRequest { from, content_id, index })) => {
                        let n = publisher_store
                            .read_chunk(content_id, index, &mut buf)
                            .await
                            .expect("read chunk");
                        publisher_transport
                            .send_chunk_response(&from, content_id, index, &buf[..n])
                            .await
                            .expect("send response");
                    }
                    Ok(Some(_)) => {} // ignore other events
                    Ok(None) | Err(_) => break,
                }
            }
        });

        let downloaded = download_handle.await.expect("join").expect("download");

        assert_eq!(downloaded.len(), content.len(), "content length mismatch");
        assert_eq!(downloaded, content, "content bytes mismatch");
        assert_eq!(
            ContentId::from_bytes(&downloaded),
            manifest.content_id,
            "content ID mismatch"
        );

        serve_handle.abort();
    })
    .await;
}

#[tokio::test]
async fn chunk_verification_rejects_corruption() {
    let content = b"short test content for corruption check".to_vec();
    let profile = ChunkProfile::WiFi;
    let manifest = build_manifest(&content, profile, [0xAAu8; 16]);

    // Correct chunk passes
    assert!(manifest.verify_chunk(0, &content));

    // Corrupted chunk fails
    let mut corrupted = content.clone();
    corrupted[0] ^= 0xFF;
    assert!(!manifest.verify_chunk(0, &corrupted));
}

#[tokio::test]
async fn content_id_deterministic() {
    let a = ContentId::from_bytes(b"deterministic");
    let b = ContentId::from_bytes(b"deterministic");
    assert_eq!(a, b);

    let c = ContentId::from_bytes(b"different");
    assert_ne!(a, c);
}

#[tokio::test]
async fn chunk_store_roundtrip() {
    let mut store = RamChunkStore::new();
    let id = ContentId::from_bytes(b"store-test");

    store.write_chunk(id, 0, b"chunk zero").await.unwrap();
    store.write_chunk(id, 1, b"chunk one").await.unwrap();

    assert!(store.has_chunk(id, 0).await);
    assert!(store.has_chunk(id, 1).await);
    assert!(!store.has_chunk(id, 2).await);

    let mut buf = [0u8; 64];
    let n = store.read_chunk(id, 0, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"chunk zero");

    let bs = store.chunks_held(id).await;
    assert!(bs.get(0));
    assert!(bs.get(1));
    assert!(!bs.get(2));

    store.evict(id).await.unwrap();
    assert!(!store.has_chunk(id, 0).await);
}

#[tokio::test]
async fn manifest_validates_chunk_count() {
    let content = vec![0u8; 1024]; // 1KB
    let manifest = build_manifest(&content, ChunkProfile::WiFi, [0u8; 16]);

    // WiFi profile = 256KB chunks, so 1KB content = 1 chunk
    assert_eq!(manifest.chunk_count, 1);
    assert!(manifest.validate().is_ok());
}
