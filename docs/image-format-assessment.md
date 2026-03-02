# Image Format Assessment for Styrene Wire Protocol

**Date**: 2026-03-02
**Status**: Accepted — JXL as native format, JPEG as fallback
**Applies to**: `styrene-mesh` crate (wire protocol), `styrened` Python daemon

## Context

Styrene sends images over LXMF on Reticulum mesh networks — low bandwidth, high latency, resource-constrained edge devices. The wire protocol (`StyreneEnvelope`) needs a native image format for media attachments.

**Constraints**:
- Target **15–50 KB** per image (radio time is expensive)
- Decode on **Pi Zero 2W** (ARM Cortex-A53, 512 MB RAM)
- Encode on desktop/Pi 4B (ARM Cortex-A72 or x86_64)
- MCUs (ESP32, nRF52840, RP2040) skip image payloads entirely
- Both Python (`styrened`) and Rust (`styrene-rs`) must encode and decode
- Wire protocol is the shared contract — no FFI between implementations

## Decision

**JXL (JPEG XL)** as the native Styrene image format, with JPEG accepted as a universal fallback.

## Format Comparison

| | **JXL** | **WebP** | **JPEG+mozjpeg** | **AVIF** |
|---|---|---|---|---|
| Quality at ≤50 KB | **Best** | Good | Acceptable | Best (tied) |
| Lossless JPEG repack | **✅ unique** | ❌ | N/A | ❌ |
| Progressive decode | ✅ multi-resolution | ✅ basic | ✅ baseline | ✅ |
| ARM Cortex-A53 decode | ~10 ms | ~12 ms | **~5 ms** | ~30 ms (slow) |
| License | BSD-3 (clean) | BSD-3 | Public domain | BSD + AOM patent grant |

### Rust Crate Ecosystem (as of 2026-03)

| Format | Decode | Encode | Notes |
|--------|--------|--------|-------|
| **JXL** | `jxl-oxide` 0.12 — **pure Rust**, active, modular | `zune-jpegxl` 0.5 (pure Rust, early) / `jxl-encoder` 0.1 | `jpegxl-rs` 0.13 available as libjxl C binding alternative |
| WebP | `zenwebp` 0.3 (pure Rust, new) | `zenwebp` 0.3 | `webp` 0.3 is mature but C bindings to libwebp |
| JPEG | `jpeg-decoder` (mature, in `image` crate) | `mozjpeg-rs` 0.8 (pure Rust mozjpeg port) | Most mature ecosystem |
| AVIF | `avif-decode` 1.0 | `cavif` 1.6 (pure Rust) | Fragmented across many single-purpose crates |

**`jxl-oxide`** is the deciding factor. A pure-Rust, actively maintained JXL decoder at v0.12 with modular sub-crates (`jxl-bitstream`, `jxl-frame`, `jxl-render`, `jxl-color`) means `styrene-rs` can decode JXL without C dependencies — critical for cross-compilation to ARM targets.

### Python Library Support

| Format | Library | Status |
|--------|---------|--------|
| **JXL** | `imagecodecs` (jxl_decode/jxl_encode) or `pillow-jxl-plugin` | Requires libjxl system lib |
| WebP | Pillow (built-in) | Zero extra deps ✅ |
| JPEG | Pillow (built-in) | Zero extra deps ✅ |
| AVIF | `pillow-avif-plugin` | Requires libavif |

### Python ↔ Rust Binding Landscape (PyO3/Maturin)

While `styrened` and `styrene-rs` communicate over the wire (no FFI), a Python binding to Rust image codecs is a viable optimization path. The current landscape:

| Tool | Purpose | Maturity |
|------|---------|----------|
| **PyO3** | Rust ↔ Python FFI framework | Production-grade, v0.23+. The standard. Used by Polars, Pydantic V2, ruff, cryptography, tiktoken. Supports `#[pyfunction]`, `#[pyclass]`, async, GIL management, buffer protocol. |
| **Maturin** | Build backend for PyO3 crates → Python wheels | Production-grade, v1.8+. `pip install maturin && maturin develop`. Generates manylinux/musllinux/macOS wheels. Integrates with pyproject.toml. |
| **pyo3-asyncio** | Bridge Rust futures ↔ Python asyncio | Stable. Maps `tokio::spawn` to Python awaitable. |
| **rust-numpy** | Zero-copy NumPy ↔ ndarray | Stable. For image data as numpy arrays without copying. |
| **uniffi** | Mozilla's multi-language binding generator | Generates Python/Kotlin/Swift from Rust. IDL-based. Less flexible than PyO3 but generates bindings for multiple targets from one definition. |
| **CFFI + cbindgen** | C ABI approach | Lower-level. `cbindgen` generates C headers from Rust; Python calls via `cffi`. No GIL awareness. Legacy approach. |

**PyO3 + Maturin is the clear winner** for Python-Rust integration. The ecosystem is mature, widely adopted, and well-documented. If we ever want to expose `jxl-oxide` directly to `styrened` (bypassing the system libjxl dependency), we'd publish a `styrene-imagecodecs` PyO3 crate:

```toml
# hypothetical crates/bindings/styrene-imagecodecs-py/Cargo.toml
[dependencies]
jxl-oxide = "0.12"
pyo3 = { version = "0.23", features = ["extension-module"] }

[build-system]
requires = ["maturin>=1.0"]
build-backend = "maturin"
```

```rust
#[pyfunction]
fn decode_jxl(data: &[u8]) -> PyResult<Vec<u8>> {
    // jxl-oxide decode → raw RGBA pixels
}

#[pyfunction]
fn encode_jxl(pixels: &[u8], width: u32, height: u32, quality: f32) -> PyResult<Vec<u8>> {
    // zune-jpegxl encode
}

#[pyfunction]
fn repack_jpeg_to_jxl(jpeg_data: &[u8]) -> PyResult<Vec<u8>> {
    // Lossless JPEG → JXL repack
}
```

This is **not needed immediately** — `imagecodecs` or `pillow-jxl-plugin` work fine for `styrened`. But it's a clean path to eliminating the libjxl system dependency for Python users while sharing the same Rust codec used by `styrene-rs`.

## Formats Rejected

| Format | Reason |
|--------|--------|
| **HEIF/HEIC** | Apple patent encumbrance. No viable Rust crate. Slow ARM decode. |
| **AVIF** | AOM patent grant complexity. Fragmented Rust crates. 3× slower ARM decode than JXL. |
| **QOI** | Lossy compression worse than JPEG at ≤50 KB. Lossless-only design mismatched for bandwidth-constrained mesh. |
| **FLIF** | Dead project. No maintained Rust or Python libraries. |
| **BPG** | Dead format. H.265 patent issues. No Rust crate. |
| **WebP** | Good but not best-in-class at ≤50 KB. No JPEG repack. Pure-Rust story (`zenwebp` 0.3) less mature than JXL's `jxl-oxide` 0.12. Remains acceptable as a received format. |

## Wire Protocol Design

```
StyreneEnvelope.media: Vec<MediaAttachment>

MediaAttachment {
    content_type: String,      // "image/jxl", "image/jpeg"
    data: Vec<u8>,             // Encoded image bytes
    original_type: Option<String>,  // Set when JXL is a lossless JPEG repack
    thumbnail: Option<Vec<u8>>,     // Optional low-res JXL progressive prefix
    width: u16,
    height: u16,
}
```

### Sending Flow

1. Source is JPEG (from phone camera, other LXMF client):
   - Lossless repack to JXL (~20% smaller, bit-exact reversible)
   - Set `original_type = "image/jpeg"`
2. Source is other (screenshot, generated):
   - Encode to JXL lossy at quality targeting ≤50 KB budget
   - `original_type = None`

### Receiving Flow

1. `content_type == "image/jxl"`:
   - Decode with `jxl-oxide` (Rust) or `imagecodecs` (Python)
   - If forwarding to non-Styrene client and `original_type == "image/jpeg"`: extract original JPEG losslessly
2. `content_type == "image/jpeg"`:
   - Accept as-is (universal fallback)
3. Unknown content type:
   - Store raw bytes, surface as downloadable attachment

### Progressive Decode (Mesh UX)

JXL's progressive mode allows decoding partial data into a usable low-resolution preview. On slow mesh links:

1. First ~2 KB: decode to 1/8 resolution thumbnail
2. First ~10 KB: decode to 1/4 resolution preview
3. Full payload: decode to full resolution

The TUI `ImagePreview` widget can render progressive refinement as bytes arrive over LXMF, providing immediate visual feedback on slow links.

## Dependencies to Add

### `styrene-mesh` (Rust)

```toml
[dependencies]
jxl-oxide = "0.12"       # Pure Rust JXL decode
zune-jpegxl = "0.5"      # Pure Rust JXL encode (when stabilized)
# Or initially:
jpegxl-rs = "0.13"       # libjxl bindings (more complete encode support)
```

### `styrened` (Python)

```toml
# pyproject.toml optional dependency
[project.optional-dependencies]
imaging = ["imagecodecs>=2024.1"]
# Or: pillow-jxl-plugin
```

## Open Questions

1. **Encode on Rust side**: `zune-jpegxl` (pure Rust) vs `jpegxl-rs` (libjxl bindings) — evaluate encode quality and speed at ≤50 KB targets before committing.
2. **Progressive prefix as thumbnail**: Can we extract a fixed-size progressive prefix from a JXL bitstream to use as `thumbnail` field, or should thumbnails be a separate encode at lower resolution?
3. **Size budget negotiation**: Should `StyreneEnvelope` support a size hint so the sender can target the recipient's bandwidth constraints?
