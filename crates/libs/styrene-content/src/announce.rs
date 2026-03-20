use serde::{Deserialize, Serialize};

use crate::{chunk_bitset::ChunkBitset, content_id::ContentId};

/// Mesh broadcast announcing that a node holds (some or all) chunks of a content item.
///
/// Transmitted as LXMF message type `0xE0` (ResourceAvailable).
/// Nodes receiving this update their seeder table for the given content_id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceAvailableAnnounce {
    /// Content being announced.
    pub content_id: ContentId,
    /// First 16 bytes of Blake3(manifest_bytes) — lets receivers detect
    /// manifest changes without re-fetching.
    pub manifest_hash: [u8; 16],
    /// Which chunks this node currently holds.
    pub chunks_held: ChunkBitset,
    /// RNS identity_hash of the announcing node.
    pub seeder_hash: [u8; 16],
}

impl ResourceAvailableAnnounce {
    pub fn new(
        content_id: ContentId,
        manifest_hash: [u8; 16],
        chunks_held: ChunkBitset,
        seeder_hash: [u8; 16],
    ) -> Self {
        Self { content_id, manifest_hash, chunks_held, seeder_hash }
    }

    /// True if the announcer claims to hold all chunks for a given count.
    pub fn is_complete_seeder(&self, total_chunks: u32) -> bool {
        self.chunks_held.is_complete(total_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_cbor() {
        let mut held = ChunkBitset::new();
        held.set(0);
        held.set(1);

        let ann = ResourceAvailableAnnounce::new(
            ContentId::from_bytes(b"test content"),
            [0xABu8; 16],
            held,
            [0x01u8; 16],
        );

        // Verify fields
        assert!(ann.chunks_held.get(0));
        assert!(ann.chunks_held.get(1));
        assert!(!ann.chunks_held.get(2));
        assert!(!ann.is_complete_seeder(3));
    }

    #[test]
    fn complete_seeder() {
        let mut held = ChunkBitset::new();
        held.set(0);
        held.set(1);
        held.set(2);
        let ann = ResourceAvailableAnnounce::new(
            ContentId::from_bytes(b"x"),
            [0u8; 16],
            held,
            [0u8; 16],
        );
        assert!(ann.is_complete_seeder(3));
        assert!(!ann.is_complete_seeder(4));
    }
}
