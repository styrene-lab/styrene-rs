//! Compile-only test verifying that Zone 0 types work with default features (no_std, no alloc).
//!
//! This test file imports only types that must be available without `alloc` or `std`.
//! If this file fails to compile with `cargo test -p styrene-content` (default features),
//! the no_std contract is broken.

use styrene_content::{
    announce::ResourceAvailableAnnounce,
    chunk_bitset::ChunkBitset,
    chunk_profile::ChunkProfile,
    content_id::ContentId,
};

#[test]
fn content_id_no_alloc() {
    let id = ContentId::from_bytes(b"no_std test content");
    let id2 = ContentId::from_bytes(b"no_std test content");
    assert_eq!(id, id2);
    // from_raw is const — usable in embedded contexts
    let raw = *id.as_bytes();
    let id3 = ContentId::from_raw(raw);
    assert_eq!(id, id3);
}

#[test]
fn chunk_profile_no_alloc() {
    assert_eq!(ChunkProfile::LoRa.chunk_size(), 4096);
    assert_eq!(ChunkProfile::Balanced.chunk_size(), 32768);
    assert_eq!(ChunkProfile::WiFi.chunk_size(), 262144);
    // Max file sizes are deterministic
    assert_eq!(ChunkProfile::LoRa.max_file_size(), 4096 * 256);
    assert_eq!(ChunkProfile::WiFi.max_file_size(), 262144 * 256);
}

#[test]
fn chunk_bitset_no_alloc() {
    let mut bs = ChunkBitset::new();
    for i in 0u32..8 {
        bs.set(i);
    }
    assert_eq!(bs.count(), 8);
    assert!(bs.get(0));
    assert!(bs.get(7));
    assert!(!bs.get(8));
    assert!(bs.is_complete(8)); // first 8 bits set → complete for total=8
    assert!(!bs.is_complete(9)); // bit 8 not set
}

#[test]
fn resource_available_announce_no_alloc() {
    let mut held = ChunkBitset::new();
    held.set(0);
    held.set(1);
    let ann = ResourceAvailableAnnounce::new(
        ContentId::from_bytes(b"test"),
        [0xFFu8; 16],
        held,
        [0x01u8; 16],
    );
    assert!(ann.is_complete_seeder(2));
    assert!(!ann.is_complete_seeder(3));
}
