//! In-memory chunk store backed by a HashMap. Requires `alloc` feature.

use alloc::{collections::BTreeMap, vec::Vec};

use crate::{chunk_bitset::ChunkBitset, content_id::ContentId, store::ChunkStore};

/// In-memory chunk store. Ephemeral — cleared on drop.
///
/// Use for:
/// - Unit testing
/// - Firmware downloads where chunks are applied then discarded
/// - Any context where persistence across restarts is not required
pub struct RamChunkStore {
    /// Key: (content_id_bytes, chunk_index) → chunk bytes
    chunks: BTreeMap<([u8; 32], u32), Vec<u8>>,
}

impl RamChunkStore {
    pub fn new() -> Self {
        Self { chunks: BTreeMap::new() }
    }
}

impl Default for RamChunkStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkStore for RamChunkStore {
    type Error = core::convert::Infallible;

    async fn read_chunk(
        &self,
        id: ContentId,
        index: u32,
        buf: &mut [u8],
    ) -> Result<usize, Self::Error> {
        if let Some(data) = self.chunks.get(&(*id.as_bytes(), index)) {
            let n = data.len().min(buf.len());
            buf[..n].copy_from_slice(&data[..n]);
            Ok(n)
        } else {
            Ok(0)
        }
    }

    async fn write_chunk(
        &mut self,
        id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        self.chunks.insert((*id.as_bytes(), index), data.to_vec());
        Ok(())
    }

    async fn has_chunk(&self, id: ContentId, index: u32) -> bool {
        self.chunks.contains_key(&(*id.as_bytes(), index))
    }

    async fn chunks_held(&self, id: ContentId) -> ChunkBitset {
        let mut bs = ChunkBitset::new();
        for &(ref cid, idx) in self.chunks.keys() {
            if cid == id.as_bytes() && idx < 256 {
                bs.set(idx);
            }
        }
        bs
    }

    async fn evict(&mut self, id: ContentId) -> Result<(), Self::Error> {
        self.chunks.retain(|(cid, _), _| cid != id.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_read_roundtrip() {
        let mut store = RamChunkStore::new();
        let id = ContentId::from_bytes(b"test");
        store.write_chunk(id, 0, b"hello chunk").await.unwrap();
        assert!(store.has_chunk(id, 0).await);
        let mut buf = [0u8; 64];
        let n = store.read_chunk(id, 0, &mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello chunk");
    }

    #[tokio::test]
    async fn bitset_tracks_chunks() {
        let mut store = RamChunkStore::new();
        let id = ContentId::from_bytes(b"bitset-test");
        store.write_chunk(id, 0, b"c0").await.unwrap();
        store.write_chunk(id, 2, b"c2").await.unwrap();
        let bs = store.chunks_held(id).await;
        assert!(bs.get(0));
        assert!(!bs.get(1));
        assert!(bs.get(2));
    }

    #[tokio::test]
    async fn evict_removes_all() {
        let mut store = RamChunkStore::new();
        let id = ContentId::from_bytes(b"evict");
        for i in 0..3 {
            store.write_chunk(id, i, b"data").await.unwrap();
        }
        store.evict(id).await.unwrap();
        for i in 0..3 {
            assert!(!store.has_chunk(id, i).await);
        }
    }
}
