//! Filesystem-backed chunk store using tokio::fs. Requires `tokio` feature.

use std::path::PathBuf;

use crate::{chunk_bitset::ChunkBitset, content_id::ContentId, store::ChunkStore};

/// Persistent chunk store backed by the local filesystem via `tokio::fs`.
///
/// Layout: `{base_dir}/{content_id_hex}/{index:06}` — one file per chunk.
/// The directory for a content item is created on first write and removed
/// entirely on `evict`.
pub struct TokioFsChunkStore {
    base_dir: PathBuf,
}

impl TokioFsChunkStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self { base_dir: base_dir.into() }
    }

    fn chunk_path(&self, id: ContentId, index: u32) -> PathBuf {
        let id_hex = alloc::format!("{id:x}");
        self.base_dir.join(&id_hex).join(alloc::format!("{index:06}"))
    }

    fn content_dir(&self, id: ContentId) -> PathBuf {
        let id_hex = alloc::format!("{id:x}");
        self.base_dir.join(&id_hex)
    }
}

impl ChunkStore for TokioFsChunkStore {
    type Error = std::io::Error;

    async fn read_chunk(
        &self,
        id: ContentId,
        index: u32,
        buf: &mut [u8],
    ) -> Result<usize, Self::Error> {
        use tokio::io::AsyncReadExt;
        let path = self.chunk_path(id, index);
        let mut f = tokio::fs::File::open(&path).await?;
        let n = f.read(buf).await?;
        Ok(n)
    }

    async fn write_chunk(
        &mut self,
        id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        use tokio::io::AsyncWriteExt;
        let path = self.chunk_path(id, index);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut f = tokio::fs::File::create(&path).await?;
        f.write_all(data).await?;
        Ok(())
    }

    async fn has_chunk(&self, id: ContentId, index: u32) -> bool {
        tokio::fs::metadata(self.chunk_path(id, index)).await.is_ok()
    }

    async fn chunks_held(&self, id: ContentId) -> ChunkBitset {
        let mut bs = ChunkBitset::new();
        let dir = self.content_dir(id);
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            return bs;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                if let Ok(idx) = name.parse::<u32>() {
                    if idx < 256 {
                        bs.set(idx);
                    }
                }
            }
        }
        bs
    }

    async fn evict(&mut self, id: ContentId) -> Result<(), Self::Error> {
        let dir = self.content_dir(id);
        if tokio::fs::metadata(&dir).await.is_ok() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        Ok(())
    }
}
