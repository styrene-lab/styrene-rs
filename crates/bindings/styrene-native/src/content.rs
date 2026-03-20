//! PyO3 bindings for styrene-content types.
//!
//! Exposed classes:
//! - `ContentId`                    — Blake3 content hash
//! - `ChunkProfile`                 — LoRa / Balanced / WiFi chunk size profiles
//! - `StyreneManifest`              — signed content descriptor
//! - `ResourceAvailableAnnounce`    — seeder availability announce

use pyo3::prelude::*;
use pyo3::exceptions::PyValueError;
use pyo3::types::PyBytes;
use styrene_content::{
    announce::ResourceAvailableAnnounce,
    chunk_bitset::ChunkBitset,
    chunk_profile::ChunkProfile,
    content_id::ContentId,
    manifest::StyreneManifest,
};

// ── ContentId ────────────────────────────────────────────────────────────────

/// Blake3 content hash (32 bytes). Uniquely identifies a piece of content.
#[pyclass(name = "ContentId")]
#[derive(Clone)]
pub struct PyContentId {
    inner: ContentId,
}

#[pymethods]
impl PyContentId {
    /// Compute from raw bytes (hashes them with Blake3).
    #[staticmethod]
    pub fn from_bytes(data: &[u8]) -> Self {
        Self { inner: ContentId::from_bytes(data) }
    }

    /// Reconstruct from a 32-byte hash array (no re-hashing).
    #[staticmethod]
    pub fn from_raw(data: &[u8]) -> PyResult<Self> {
        let arr: [u8; 32] = data
            .try_into()
            .map_err(|_| PyValueError::new_err("expected 32 bytes"))?;
        Ok(Self { inner: ContentId::from_raw(arr) })
    }

    /// Lowercase hex string of the 32-byte hash.
    pub fn hex(&self) -> String {
        format!("{:x}", self.inner)
    }

    /// Raw 32-byte digest as bytes.
    pub fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.as_bytes())
    }

    fn __repr__(&self) -> String {
        format!("ContentId(\"{}\")", self.hex())
    }

    fn __eq__(&self, other: &PyContentId) -> bool {
        self.inner == other.inner
    }
}

// ── ChunkProfile ─────────────────────────────────────────────────────────────

/// Chunk size profile chosen by the publisher.
///
/// - ``LoRa``: 4 KB — RP2040 and strict LoRa paths
/// - ``Balanced``: 32 KB — mixed topologies and most ESP32
/// - ``WiFi``: 256 KB — hub nodes and ESP32 with PSRAM
#[pyclass(name = "ChunkProfile", eq, eq_int)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PyChunkProfile {
    LoRa = 0,
    Balanced = 1,
    WiFi = 2,
}

impl From<ChunkProfile> for PyChunkProfile {
    fn from(p: ChunkProfile) -> Self {
        match p {
            ChunkProfile::LoRa => Self::LoRa,
            ChunkProfile::Balanced => Self::Balanced,
            ChunkProfile::WiFi => Self::WiFi,
        }
    }
}

impl From<PyChunkProfile> for ChunkProfile {
    fn from(p: PyChunkProfile) -> Self {
        match p {
            PyChunkProfile::LoRa => Self::LoRa,
            PyChunkProfile::Balanced => Self::Balanced,
            PyChunkProfile::WiFi => Self::WiFi,
        }
    }
}

#[pymethods]
impl PyChunkProfile {
    /// Chunk size in bytes for this profile.
    pub fn chunk_size(&self) -> u32 {
        ChunkProfile::from(*self).chunk_size()
    }

    /// Maximum single-file size (256 chunks × chunk_size).
    pub fn max_file_size(&self) -> u64 {
        ChunkProfile::from(*self).max_file_size()
    }
}

// ── StyreneManifest ──────────────────────────────────────────────────────────

/// A signed content descriptor for a piece of content distributed over the mesh.
#[pyclass(name = "StyreneManifest")]
#[derive(Clone)]
pub struct PyStyreneManifest {
    inner: StyreneManifest,
}

#[pymethods]
impl PyStyreneManifest {
    /// Build and sign a manifest from raw content.
    ///
    /// ``sign_fn`` is a Python callable ``(data: bytes) -> bytes`` (must return 64 bytes).
    #[staticmethod]
    #[pyo3(signature = (content, name, content_type, profile, created_at, creator_identity, sign_fn))]
    pub fn build(
        content: &[u8],
        name: &str,
        content_type: &str,
        profile: PyChunkProfile,
        created_at: u64,
        creator_identity: &[u8],
        sign_fn: PyObject,
    ) -> PyResult<Self> {
        let identity: [u8; 16] = creator_identity
            .try_into()
            .map_err(|_| PyValueError::new_err("creator_identity must be 16 bytes"))?;

        Python::with_gil(|py| {
            let sign_closure = |data: &[u8]| -> [u8; 64] {
                let result: PyResult<[u8; 64]> = (|| {
                    let sig_obj = sign_fn.call1(py, (PyBytes::new(py, data),))?;
                    let sig_slice: &[u8] = sig_obj.extract(py)?;
                    sig_slice
                        .try_into()
                        .map_err(|_| PyValueError::new_err("sign_fn must return 64 bytes").into())
                })();
                result.unwrap_or([0u8; 64])
            };

            let inner = StyreneManifest::build(
                content,
                name,
                content_type,
                ChunkProfile::from(profile),
                created_at,
                identity,
                sign_closure,
            )
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))?;

            Ok(Self { inner })
        })
    }

    /// CBOR-encode the manifest (includes signature).
    pub fn encode<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.inner.encode()
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Decode a CBOR-encoded manifest.
    #[staticmethod]
    pub fn decode(data: &[u8]) -> PyResult<Self> {
        let inner = StyreneManifest::decode(data)
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))?;
        Ok(Self { inner })
    }

    /// Verify the Ed25519 signature.
    ///
    /// ``verify_fn`` is a Python callable ``(data: bytes, sig: bytes) -> bool``.
    pub fn verify_signature(&self, verify_fn: PyObject) -> PyResult<bool> {
        Python::with_gil(|py| {
            let verify_closure = |data: &[u8], sig: &[u8; 64]| -> bool {
                verify_fn
                    .call1(py, (PyBytes::new(py, data), PyBytes::new(py, sig)))
                    .and_then(|r| r.extract::<bool>(py))
                    .unwrap_or(false)
            };
            Ok(self.inner.verify_signature(verify_closure).is_ok())
        })
    }

    /// Validate structural consistency.
    pub fn validate(&self) -> PyResult<()> {
        self.inner.validate()
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))
    }

    /// Verify a single chunk's Blake3 hash against the manifest.
    pub fn verify_chunk(&self, index: u32, data: &[u8]) -> bool {
        self.inner.verify_chunk(index, data)
    }

    // ── Properties ──────────────────────────────────────────────────────────

    #[getter]
    pub fn content_id(&self) -> PyContentId {
        PyContentId { inner: self.inner.content_id }
    }

    #[getter]
    pub fn size(&self) -> u64 { self.inner.size }

    #[getter]
    pub fn chunk_count(&self) -> u32 { self.inner.chunk_count }

    #[getter]
    pub fn name(&self) -> &str { self.inner.name.as_str() }

    #[getter]
    pub fn content_type(&self) -> &str { self.inner.content_type.as_str() }

    #[getter]
    pub fn created_at(&self) -> u64 { self.inner.created_at }

    #[getter]
    pub fn creator_identity<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.creator_identity)
    }

    #[getter]
    pub fn chunk_profile(&self) -> PyChunkProfile {
        PyChunkProfile::from(self.inner.chunk_profile)
    }

    fn __repr__(&self) -> String {
        format!(
            "StyreneManifest(name={:?}, size={}, chunks={}, profile={:?})",
            self.inner.name.as_str(),
            self.inner.size,
            self.inner.chunk_count,
            self.chunk_profile(),
        )
    }
}

// ── ResourceAvailableAnnounce ─────────────────────────────────────────────────

/// Announce that a node holds chunks for a content item.
#[pyclass(name = "ResourceAvailableAnnounce")]
#[derive(Clone)]
pub struct PyResourceAvailableAnnounce {
    inner: ResourceAvailableAnnounce,
}

#[pymethods]
impl PyResourceAvailableAnnounce {
    #[new]
    pub fn new(
        content_id_bytes: &[u8],
        manifest_hash: &[u8],
        chunks_held_bytes: &[u8],
        seeder_hash: &[u8],
    ) -> PyResult<Self> {
        let content_id_arr: [u8; 32] = content_id_bytes
            .try_into()
            .map_err(|_| PyValueError::new_err("content_id must be 32 bytes"))?;
        let manifest_hash_arr: [u8; 16] = manifest_hash
            .try_into()
            .map_err(|_| PyValueError::new_err("manifest_hash must be 16 bytes"))?;
        let seeder_hash_arr: [u8; 16] = seeder_hash
            .try_into()
            .map_err(|_| PyValueError::new_err("seeder_hash must be 16 bytes"))?;

        // Build ChunkBitset from 32 raw bytes
        let mut bitset = ChunkBitset::new();
        if chunks_held_bytes.len() == 32 {
            for byte_idx in 0..32usize {
                for bit in 0..8usize {
                    if chunks_held_bytes[byte_idx] & (1 << bit) != 0 {
                        bitset.set((byte_idx * 8 + bit) as u32);
                    }
                }
            }
        }

        Ok(Self {
            inner: ResourceAvailableAnnounce::new(
                ContentId::from_raw(content_id_arr),
                manifest_hash_arr,
                bitset,
                seeder_hash_arr,
            ),
        })
    }

    #[getter]
    pub fn content_id<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.content_id.as_bytes())
    }

    #[getter]
    pub fn manifest_hash<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.manifest_hash)
    }

    #[getter]
    pub fn seeder_hash<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.seeder_hash)
    }

    #[getter]
    pub fn chunks_held_count(&self) -> u32 {
        self.inner.chunks_held.count()
    }

    pub fn is_complete_seeder(&self, total_chunks: u32) -> bool {
        self.inner.is_complete_seeder(total_chunks)
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceAvailableAnnounce(chunks_held={}/256)",
            self.inner.chunks_held.count()
        )
    }
}
