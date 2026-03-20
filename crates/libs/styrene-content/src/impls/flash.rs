//! Flash-backed chunk store using embedded-storage. Requires `embedded-storage` feature.
//!
//! # Status
//!
//! Stub implementation. The `ChunkStore` trait is implemented but all
//! operations return `FlashError::NotImplemented`. A real implementation
//! requires:
//! - A flash partition layout (fixed offset, chunk index header)
//! - Per-content-item directory structure in flash
//! - Wear-leveling awareness for constrained NOR flash devices
//!
//! This stub allows the trait bounds to be satisfied for RP2040/ESP32
//! integration work to proceed before the full implementation is ready.

use crate::{chunk_bitset::ChunkBitset, content_id::ContentId, store::ChunkStore};

#[derive(Debug)]
pub enum FlashError {
    /// Flash backing not yet implemented.
    NotImplemented,
    /// Underlying NOR flash returned an error.
    NorFlash,
}

/// Chunk store backed by a NOR flash device via the `embedded-storage` trait.
///
/// `F` implements `embedded_storage::nor_flash::NorFlash`.
pub struct FlashChunkStore<F> {
    flash: F,
}

impl<F> FlashChunkStore<F> {
    pub fn new(flash: F) -> Self {
        Self { flash }
    }

    pub fn flash(&mut self) -> &mut F {
        &mut self.flash
    }
}

impl<F> ChunkStore for FlashChunkStore<F>
where
    F: Send + 'static,
{
    type Error = FlashError;

    async fn read_chunk(
        &self,
        _id: ContentId,
        _index: u32,
        _buf: &mut [u8],
    ) -> Result<usize, Self::Error> {
        Err(FlashError::NotImplemented)
    }

    async fn write_chunk(
        &mut self,
        _id: ContentId,
        _index: u32,
        _data: &[u8],
    ) -> Result<(), Self::Error> {
        Err(FlashError::NotImplemented)
    }

    async fn has_chunk(&self, _id: ContentId, _index: u32) -> bool {
        false
    }

    async fn chunks_held(&self, _id: ContentId) -> ChunkBitset {
        ChunkBitset::new()
    }

    async fn evict(&mut self, _id: ContentId) -> Result<(), Self::Error> {
        Err(FlashError::NotImplemented)
    }
}
