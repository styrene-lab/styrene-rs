//! Integration tests for `StyreneManifest` encode/decode and signature verification.

#![cfg(feature = "alloc")]

use styrene_content::{
    chunk_profile::ChunkProfile,
    manifest::StyreneManifest,
};

fn noop_sign(_data: &[u8]) -> [u8; 64] { [0xBBu8; 64] }
fn noop_verify(_data: &[u8], sig: &[u8; 64]) -> bool { sig.iter().all(|&b| b == 0xBB) }

fn firmware_blob() -> Vec<u8> {
    // Simulate a small firmware blob spread across multiple LoRa chunks.
    // LoRa chunk size = 4096 bytes; 3 chunks = 12288 bytes.
    vec![0xDEu8; 12288]
}

#[test]
fn multi_chunk_encode_decode_roundtrip() {
    let blob = firmware_blob();
    let m = StyreneManifest::build(
        &blob,
        "styrened-rs-v1.0.0",
        "firmware/styrened-rs",
        ChunkProfile::LoRa,
        1_700_000_000,
        [0x42u8; 16],
        noop_sign,
    )
    .expect("build failed");

    assert_eq!(m.chunk_count, 3, "expected 3 LoRa chunks for 12288 bytes");
    m.validate().expect("validate failed");

    let encoded = m.encode().expect("encode failed");
    let decoded = StyreneManifest::decode(&encoded).expect("decode failed");

    assert_eq!(decoded.content_id, m.content_id);
    assert_eq!(decoded.chunk_count, m.chunk_count);
    assert_eq!(decoded.size, m.size);
    assert_eq!(decoded.name.as_str(), m.name.as_str());
    assert_eq!(decoded.content_type.as_str(), m.content_type.as_str());
    assert_eq!(decoded.created_at, m.created_at);
    assert_eq!(decoded.creator_identity, m.creator_identity);
    assert_eq!(decoded.chunk_hashes.len(), 3);
}

#[test]
fn signature_verify_pass() {
    let m = StyreneManifest::build(
        b"hello mesh",
        "test",
        "data/test",
        ChunkProfile::LoRa,
        0,
        [0u8; 16],
        noop_sign,
    )
    .unwrap();
    assert!(m.verify_signature(noop_verify).is_ok());
}

#[test]
fn signature_verify_fail_on_corruption() {
    let mut m = StyreneManifest::build(
        b"hello mesh",
        "test",
        "data/test",
        ChunkProfile::LoRa,
        0,
        [0u8; 16],
        noop_sign,
    )
    .unwrap();
    // Corrupt signature
    m.signature.0[3] ^= 0xFF;
    assert!(m.verify_signature(noop_verify).is_err());
}

#[test]
fn wifi_profile_single_large_chunk() {
    // WiFi chunk = 262144 bytes; content of 100KB fits in one chunk
    let blob = vec![0x55u8; 100 * 1024];
    let m = StyreneManifest::build(
        &blob,
        "large-data",
        "data/emergency",
        ChunkProfile::WiFi,
        0,
        [0u8; 16],
        noop_sign,
    )
    .unwrap();
    assert_eq!(m.chunk_count, 1);
    assert!(m.verify_chunk(0, &blob));
}

#[test]
fn per_chunk_verification() {
    // Build with balanced profile: 32KB chunks; use 3 chunks of distinct data
    let chunk_size = ChunkProfile::Balanced.chunk_size() as usize;
    let mut blob = vec![0xAAu8; chunk_size * 3];
    // Make each chunk distinct
    blob[chunk_size..chunk_size * 2].fill(0xBB);
    blob[chunk_size * 2..].fill(0xCC);

    let m = StyreneManifest::build(
        &blob,
        "multi-chunk",
        "data/test",
        ChunkProfile::Balanced,
        0,
        [0u8; 16],
        noop_sign,
    )
    .unwrap();

    assert_eq!(m.chunk_count, 3);
    assert!(m.verify_chunk(0, &blob[..chunk_size]));
    assert!(m.verify_chunk(1, &blob[chunk_size..chunk_size * 2]));
    assert!(m.verify_chunk(2, &blob[chunk_size * 2..]));
    // Tampered chunk must fail
    assert!(!m.verify_chunk(0, &blob[chunk_size..chunk_size * 2]));
}
