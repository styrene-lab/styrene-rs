use heapless::{String as HString, Vec as HVec};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{chunk_profile::ChunkProfile, content_id::ContentId, error::ManifestError};

/// Newtype wrapping a 64-byte Ed25519 signature with manual serde.
/// (serde only auto-derives for arrays up to [T;32] in some versions.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sig64(pub [u8; 64]);

impl core::fmt::Debug for Sig64 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Sig64([…64 bytes…])")
    }
}

impl Serialize for Sig64 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sig64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Sig64;
            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "64 bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Sig64, E> {
                let arr: [u8; 64] = v.try_into()
                    .map_err(|_| E::invalid_length(v.len(), &self))?;
                Ok(Sig64(arr))
            }
        }
        d.deserialize_bytes(V)
    }
}



/// Maximum chunks tracked — matches `ChunkBitset` capacity.
pub const MAX_CHUNKS: usize = 256;
/// Maximum name length (bytes, UTF-8).
pub const MAX_NAME_LEN: usize = 64;
/// Maximum content_type length.
pub const MAX_TYPE_LEN: usize = 48;

/// A signed descriptor for a piece of content distributed over the mesh.
///
/// # Signing
///
/// The `signature` field is an Ed25519 signature over
/// `canonical_bytes(&self)` — the CBOR encoding of all fields except
/// `signature`. Consumers must call `verify_signature` before trusting
/// any metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyreneManifest {
    /// Blake3 hash of the full assembled content.
    pub content_id: ContentId,
    /// Total content size in bytes.
    pub size: u64,
    /// Network profile — determines chunk size.
    pub chunk_profile: ChunkProfile,
    /// Number of chunks (≤ 256).
    pub chunk_count: u32,
    /// Blake3 hash of each chunk (index = chunk index).
    pub chunk_hashes: HVec<[u8; 32], MAX_CHUNKS>,
    /// Human-readable name (≤ 64 bytes UTF-8).
    pub name: HString<MAX_NAME_LEN>,
    /// Content type tag, e.g. `"firmware/styrened-rs"`, `"data/emergency"`.
    pub content_type: HString<MAX_TYPE_LEN>,
    /// Unix timestamp (seconds) when the manifest was created.
    pub created_at: u64,
    /// RNS identity_hash of the creator (16 bytes = 32 hex chars).
    pub creator_identity: [u8; 16],
    /// Ed25519 signature (64 bytes) over `canonical_bytes(&self)`.
    pub signature: Sig64,
}

impl StyreneManifest {
    /// Build and sign a manifest from raw content. Requires `alloc` feature.
    #[cfg(feature = "alloc")]
    ///
    /// `sign_fn` receives the canonical bytes and returns a 64-byte Ed25519
    /// signature. Pass `|data| identity.sign(data).to_bytes()`.
    pub fn build(
        content: &[u8],
        name: &str,
        content_type: &str,
        profile: ChunkProfile,
        created_at: u64,
        creator_identity: [u8; 16],
        sign_fn: impl Fn(&[u8]) -> [u8; 64],
    ) -> Result<Self, ManifestError> {
        let chunk_size = profile.chunk_size() as usize;
        let chunk_count = profile.chunk_count_for(content.len() as u64);

        if chunk_count > MAX_CHUNKS as u32 {
            return Err(ManifestError::TooManyChunks);
        }

        let content_id = ContentId::from_bytes(content);

        let mut chunk_hashes: HVec<[u8; 32], MAX_CHUNKS> = HVec::new();
        for i in 0..chunk_count as usize {
            let start = i * chunk_size;
            let end = (start + chunk_size).min(content.len());
            let hash = *blake3::hash(&content[start..end]).as_bytes();
            chunk_hashes.push(hash).map_err(|_| ManifestError::TooManyChunks)?;
        }

        let mut m = Self {
            content_id,
            size: content.len() as u64,
            chunk_profile: profile,
            chunk_count,
            chunk_hashes,
            name: HString::try_from(name).unwrap_or_default(),
            content_type: HString::try_from(content_type).unwrap_or_default(),
            created_at,
            creator_identity,
            signature: Sig64([0u8; 64]),
        };

        let to_sign = m.canonical_bytes()?;
        m.signature = Sig64(sign_fn(&to_sign));
        Ok(m)
    }

    /// Serialize to CBOR bytes (includes signature). Requires `alloc` feature.
    #[cfg(feature = "alloc")]
    pub fn encode(&self) -> Result<HVec<u8, 8192>, ManifestError> {
        encode_cbor(self)
    }

    /// Deserialize from CBOR bytes. Requires `alloc` feature.
    #[cfg(feature = "alloc")]
    pub fn decode(bytes: &[u8]) -> Result<Self, ManifestError> {
        ciborium::from_reader(bytes).map_err(|_| ManifestError::DecodeFailed)
    }

    /// Verify the Ed25519 signature. Requires `alloc` feature.
    #[cfg(feature = "alloc")]
    pub fn verify_signature(
        &self,
        verify_fn: impl Fn(&[u8], &[u8; 64]) -> bool,
    ) -> Result<(), ManifestError> {
        let canonical = self.canonical_bytes()?;
        if verify_fn(&canonical, &self.signature.0) {
            Ok(())
        } else {
            Err(ManifestError::InvalidSignature)
        }
    }

    /// Validate structural consistency (chunk_count matches chunk_hashes len).
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.chunk_count as usize != self.chunk_hashes.len() {
            return Err(ManifestError::ChunkCountMismatch);
        }
        if self.chunk_count > MAX_CHUNKS as u32 {
            return Err(ManifestError::TooManyChunks);
        }
        Ok(())
    }

    /// Verify a single chunk's Blake3 hash against the manifest.
    pub fn verify_chunk(&self, index: u32, data: &[u8]) -> bool {
        let Some(expected) = self.chunk_hashes.get(index as usize) else {
            return false;
        };
        let actual = *blake3::hash(data).as_bytes();
        actual == *expected
    }

    #[cfg(feature = "alloc")]
    fn canonical_bytes(&self) -> Result<HVec<u8, 8192>, ManifestError> {
        let canonical = StyreneManifestCanonical {
            content_id:       &self.content_id,
            size:             self.size,
            chunk_profile:    self.chunk_profile,
            chunk_count:      self.chunk_count,
            chunk_hashes:     &self.chunk_hashes,
            name:             &self.name,
            content_type:     &self.content_type,
            created_at:       self.created_at,
            creator_identity: &self.creator_identity,
        };
        encode_cbor(&canonical)
    }
}

/// Encode a value to CBOR. Requires `alloc` feature (Vec<u8> as output).
#[cfg(feature = "alloc")]
fn encode_cbor<T: Serialize>(value: &T) -> Result<HVec<u8, 8192>, ManifestError> {
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    ciborium::into_writer(value, &mut out)
        .map_err(|_| ManifestError::EncodeFailed)?;
    HVec::from_slice(&out).map_err(|_| ManifestError::EncodeFailed)
}

/// Helper struct for canonical serialization (excludes signature field).
#[derive(Serialize)]
struct StyreneManifestCanonical<'a> {
    content_id:       &'a ContentId,
    size:             u64,
    chunk_profile:    ChunkProfile,
    chunk_count:      u32,
    chunk_hashes:     &'a HVec<[u8; 32], MAX_CHUNKS>,
    name:             &'a HString<MAX_NAME_LEN>,
    content_type:     &'a HString<MAX_TYPE_LEN>,
    created_at:       u64,
    creator_identity: &'a [u8; 16],
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn dummy_sign(_data: &[u8]) -> [u8; 64] { [0xAAu8; 64] }
    fn dummy_verify(_data: &[u8], sig: &[u8; 64]) -> bool { sig.iter().all(|&b| b == 0xAA) }

    fn small_content() -> &'static [u8] {
        b"hello from the mesh this is some test content for the manifest"
    }

    #[test]
    fn build_and_validate() {
        let m = StyreneManifest::build(
            small_content(),
            "test content",
            "data/test",
            ChunkProfile::LoRa,
            1_700_000_000,
            [0u8; 16],
            dummy_sign,
        ).unwrap();
        m.validate().unwrap();
        assert_eq!(m.chunk_count, 1); // fits in one LoRa chunk
    }

    #[test]
    fn encode_decode_roundtrip() {
        let m = StyreneManifest::build(
            small_content(),
            "roundtrip",
            "data/test",
            ChunkProfile::LoRa,
            0,
            [1u8; 16],
            dummy_sign,
        ).unwrap();
        let encoded = m.encode().unwrap();
        let decoded = StyreneManifest::decode(&encoded).unwrap();
        assert_eq!(decoded.content_id, m.content_id);
        assert_eq!(decoded.chunk_count, m.chunk_count);
        assert_eq!(decoded.name.as_str(), m.name.as_str());
    }

    #[test]
    fn signature_verify() {
        let m = StyreneManifest::build(
            small_content(),
            "signed",
            "data/test",
            ChunkProfile::LoRa,
            0,
            [0u8; 16],
            dummy_sign,
        ).unwrap();
        assert!(m.verify_signature(dummy_verify).is_ok());
    }

    #[test]
    fn bad_signature_rejected() {
        let mut m = StyreneManifest::build(
            small_content(),
            "signed",
            "data/test",
            ChunkProfile::LoRa,
            0,
            [0u8; 16],
            dummy_sign,
        ).unwrap();
        m.signature.0[0] ^= 0xFF; // corrupt
        assert!(m.verify_signature(dummy_verify).is_err());
    }

    #[test]
    fn chunk_verification() {
        let content = b"aaaa bbbb cccc dddd"; // small, fits in one chunk
        let m = StyreneManifest::build(
            content,
            "chunk-test",
            "data/test",
            ChunkProfile::LoRa,
            0,
            [0u8; 16],
            dummy_sign,
        ).unwrap();
        assert!(m.verify_chunk(0, content));
        assert!(!m.verify_chunk(0, b"wrong content"));
        assert!(!m.verify_chunk(99, b"out of range"));
    }
}
