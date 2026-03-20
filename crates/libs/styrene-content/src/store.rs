use core::fmt;

use crate::{chunk_bitset::ChunkBitset, content_id::ContentId};

/// Async chunk storage backend.
///
/// Implementations are feature-gated in `impls/`. The trait uses AFIT
/// (async-fn-in-trait, stable since Rust 1.75) — no boxing, no alloc,
/// compatible with embassy, FreeRTOS, and tokio.
///
/// # Buffer contract
///
/// `read_chunk` uses a caller-provided buffer. The buffer must be at least
/// `chunk_profile.chunk_size()` bytes. Returns the number of bytes written.
pub trait ChunkStore {
    type Error: fmt::Debug;

    /// Read chunk `index` into `buf`. Returns bytes written.
    /// `buf` must be at least `chunk_profile.chunk_size()` bytes.
    async fn read_chunk(
        &self,
        id: ContentId,
        index: u32,
        buf: &mut [u8],
    ) -> Result<usize, Self::Error>;

    /// Write (and overwrite) chunk `index`.
    async fn write_chunk(
        &mut self,
        id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error>;

    /// Returns true if chunk `index` is stored for `id`.
    async fn has_chunk(&self, id: ContentId, index: u32) -> bool;

    /// Returns a bitset of all chunks currently held for `id`.
    async fn chunks_held(&self, id: ContentId) -> ChunkBitset;

    /// Remove all chunks for `id` from the store.
    async fn evict(&mut self, id: ContentId) -> Result<(), Self::Error>;
}
