use heapless::Vec as HVec;

use crate::{
    announce::ResourceAvailableAnnounce,
    chunk_bitset::ChunkBitset,
    content_id::ContentId,
    error::DistributorError,
    manifest::StyreneManifest,
    store::ChunkStore,
    transport::{ContentEvent, ContentTransport},
};

/// Maximum concurrent seeders tracked per content item.
const MAX_SEEDERS: usize = 16;

/// A known seeder: identity hash + which chunks they hold.
#[derive(Clone, Copy)]
struct Seeder {
    identity: [u8; 16],
    chunks:   ChunkBitset,
}

/// Protocol state machine for content distribution.
///
/// Generic over `ChunkStore` and `ContentTransport` — concrete types
/// selected at daemon startup. Must be `'static` for embassy compatibility.
///
/// # Usage
///
/// ```rust,no_run
/// # use styrene_content::{ContentDistributor, StyreneManifest};
/// // (pseudocode — actual types depend on your environment)
/// // let mut dist = ContentDistributor::new(store, transport);
/// // dist.publish(&manifest, &firmware_bytes).await?;
/// ```
pub struct ContentDistributor<S, T>
where
    S: ChunkStore + 'static,
    T: ContentTransport + 'static,
{
    store:     S,
    transport: T,
    /// Known seeders for content items we are downloading.
    seeders:   HVec<(ContentId, Seeder), MAX_SEEDERS>,
    /// Our own RNS identity hash (for announcements).
    local_id:  [u8; 16],
}

impl<S, T> ContentDistributor<S, T>
where
    S: ChunkStore + 'static,
    T: ContentTransport + 'static,
{
    pub fn new(store: S, transport: T, local_id: [u8; 16]) -> Self {
        Self {
            store,
            transport,
            seeders: HVec::new(),
            local_id,
        }
    }

    /// Publish content: split into chunks, store all, broadcast announce.
    ///
    /// After publishing, this node is a full seeder for the content.
    /// Requires `alloc` feature (manifest encoding, chunk splitting).
    #[cfg(feature = "alloc")]
    pub async fn publish(
        &mut self,
        manifest: &StyreneManifest,
        content: &[u8],
    ) -> Result<(), DistributorError> {
        manifest.validate().map_err(DistributorError::ManifestError)?;

        let chunk_size = manifest.chunk_profile.chunk_size() as usize;

        // Write all chunks to the store.
        for i in 0..manifest.chunk_count {
            let start = i as usize * chunk_size;
            let end = (start + chunk_size).min(content.len());
            self.store
                .write_chunk(manifest.content_id, i, &content[start..end])
                .await
                .map_err(|_| DistributorError::StoreError)?;
        }

        // Build a complete bitset and broadcast.
        let mut held = ChunkBitset::new();
        for i in 0..manifest.chunk_count {
            held.set(i);
        }

        let manifest_bytes = manifest.encode().map_err(DistributorError::ManifestError)?;
        let manifest_hash = {
            let h = blake3::hash(&manifest_bytes);
            let b = h.as_bytes();
            let mut out = [0u8; 16];
            out.copy_from_slice(&b[..16]);
            out
        };

        let announce = ResourceAvailableAnnounce::new(
            manifest.content_id,
            manifest_hash,
            held,
            self.local_id,
        );

        self.transport
            .broadcast_announce(&announce)
            .await
            .map_err(|_| DistributorError::TransportError)?;

        Ok(())
    }

    /// Download content: request chunks from known seeders, verify, assemble.
    ///
    /// Caller must have already received at least one `Announce` event for
    /// this content (via `on_event`) so seeders are tracked.
    ///
    /// Returns the assembled content bytes (requires `alloc` feature).
    #[cfg(feature = "alloc")]
    pub async fn download(
        &mut self,
        manifest: &StyreneManifest,
    ) -> Result<alloc::vec::Vec<u8>, DistributorError> {
        manifest.validate().map_err(DistributorError::ManifestError)?;

        if manifest.chunk_count > 256 {
            return Err(DistributorError::ContentTooLarge);
        }

        let chunk_size = manifest.chunk_profile.chunk_size() as usize;
        let mut buf = alloc::vec![0u8; chunk_size];

        // Request each missing chunk from the first seeder that has it.
        for i in 0..manifest.chunk_count {
            if self.store.has_chunk(manifest.content_id, i).await {
                continue; // already have it
            }

            let seeder = self
                .seeder_for_chunk(manifest.content_id, i)
                .ok_or(DistributorError::NoSeedersKnown)?;

            self.transport
                .send_chunk_request(&seeder, manifest.content_id, i)
                .await
                .map_err(|_| DistributorError::TransportError)?;

            // Wait for the response (naive: blocking until the right event arrives).
            loop {
                let event = self
                    .transport
                    .recv_event()
                    .await
                    .map_err(|_| DistributorError::TransportError)?
                    .ok_or(DistributorError::Incomplete)?;

                match event {
                    ContentEvent::ChunkResponse { content_id, index, data }
                        if content_id == manifest.content_id && index == i =>
                    {
                        if !manifest.verify_chunk(i, &data) {
                            return Err(DistributorError::VerificationFailed { chunk_index: i });
                        }
                        self.store
                            .write_chunk(manifest.content_id, i, &data)
                            .await
                            .map_err(|_| DistributorError::StoreError)?;
                        break;
                    }
                    // Route other events.
                    other => {
                        let _ = self.on_event(other).await;
                    }
                }
            }
        }

        // Assemble.
        let mut result = alloc::vec::Vec::with_capacity(manifest.size as usize);
        for i in 0..manifest.chunk_count {
            let n = self
                .store
                .read_chunk(manifest.content_id, i, &mut buf)
                .await
                .map_err(|_| DistributorError::StoreError)?;
            result.extend_from_slice(&buf[..n]);
        }

        // Verify assembled content matches content_id.
        let actual_id = ContentId::from_bytes(&result);
        if actual_id != manifest.content_id {
            return Err(DistributorError::VerificationFailed { chunk_index: u32::MAX });
        }

        Ok(result)
    }

    /// Handle an incoming content event (announce, request, or response).
    pub async fn on_event(&mut self, event: ContentEvent) -> Result<(), DistributorError> {
        match event {
            ContentEvent::Announce(ann) => {
                self.record_seeder(ann.content_id, ann.seeder_hash, ann.chunks_held);
            }

            ContentEvent::ChunkRequest { from, content_id, index } => {
                let chunk_size = 262144; // WiFi max — safe upper bound for stack buf
                // For actual use, caller should pass the manifest to get the right size.
                // This is a simplified serve path.
                #[cfg(feature = "alloc")]
                {
                    let mut buf = alloc::vec![0u8; chunk_size];
                    if let Ok(n) = self.store.read_chunk(content_id, index, &mut buf).await {
                        let _ = self.transport
                            .send_chunk_response(&from, content_id, index, &buf[..n])
                            .await;
                    }
                }
                let _ = (from, content_id, index); // suppress unused on no_alloc
            }

            ContentEvent::ChunkResponse { .. } => {
                // Responses are handled inline in download(). If we receive one
                // outside of a download loop, ignore it.
            }
        }
        Ok(())
    }

    /// Record or update a seeder in our local table.
    fn record_seeder(&mut self, id: ContentId, identity: [u8; 16], chunks: ChunkBitset) {
        // Update existing entry if present.
        for (cid, seeder) in self.seeders.iter_mut() {
            if *cid == id && seeder.identity == identity {
                seeder.chunks = chunks;
                return;
            }
        }
        // Add new entry (silently drop if table is full).
        let _ = self.seeders.push((id, Seeder { identity, chunks }));
    }

    /// Find any seeder that holds chunk `index` for `id`.
    fn seeder_for_chunk(&self, id: ContentId, index: u32) -> Option<[u8; 16]> {
        self.seeders
            .iter()
            .find(|(cid, s)| *cid == id && s.chunks.get(index))
            .map(|(_, s)| s.identity)
    }

    /// Access the underlying store (e.g. to evict content after applying firmware).
    pub fn store(&mut self) -> &mut S {
        &mut self.store
    }
}
