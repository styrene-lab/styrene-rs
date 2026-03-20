use core::fmt;

use heapless::Vec as HVec;

use crate::{announce::ResourceAvailableAnnounce, content_id::ContentId};

/// Maximum chunk data size for no_alloc ContentEvent variants.
/// Sized to WiFi profile (256 KB) — the largest possible chunk.
const MAX_CHUNK_DATA: usize = 256 * 1024;

/// An event received from the mesh content transport layer.
#[derive(Debug)]
pub enum ContentEvent {
    /// A peer announced availability of content chunks.
    Announce(ResourceAvailableAnnounce),

    /// A peer requests a specific chunk from us.
    ChunkRequest {
        /// RNS identity_hash of the requester.
        from: [u8; 16],
        content_id: ContentId,
        index: u32,
    },

    /// A peer sent us a chunk we requested.
    ChunkResponse {
        content_id: ContentId,
        index: u32,
        /// Raw chunk bytes. Size ≤ chunk_profile.chunk_size().
        #[cfg(not(feature = "alloc"))]
        data: HVec<u8, MAX_CHUNK_DATA>,
        #[cfg(feature = "alloc")]
        data: alloc::vec::Vec<u8>,
    },
}

/// Async mesh transport for content distribution messages.
///
/// Uses AFIT — compatible with any async executor.
pub trait ContentTransport {
    type Error: fmt::Debug;

    /// Broadcast a `RESOURCE_AVAILABLE` announce to the mesh.
    async fn broadcast_announce(
        &mut self,
        announce: &ResourceAvailableAnnounce,
    ) -> Result<(), Self::Error>;

    /// Send a chunk request directly to a specific seeder.
    async fn send_chunk_request(
        &mut self,
        seeder: &[u8; 16],
        content_id: ContentId,
        index: u32,
    ) -> Result<(), Self::Error>;

    /// Send a chunk response to a requester.
    async fn send_chunk_response(
        &mut self,
        to: &[u8; 16],
        content_id: ContentId,
        index: u32,
        data: &[u8],
    ) -> Result<(), Self::Error>;

    /// Receive the next content-layer event (announce, request, or response).
    /// Returns `None` when the transport is closed.
    async fn recv_event(&mut self) -> Result<Option<ContentEvent>, Self::Error>;
}
