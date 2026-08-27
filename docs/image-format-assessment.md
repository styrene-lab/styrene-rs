# Image Format Assessment for Styrene Wire Protocol

**Date**: 2026-03-02
**Status**: Accepted — JXL as native format, JPEG as fallback
**Applies to**: `styrene-mesh` wire protocol and Rust clients

## Context

Styrene sends images over LXMF on Reticulum mesh networks — low bandwidth, high latency, resource-constrained edge devices. The wire protocol (`StyreneEnvelope`) needs a native image format for media attachments.

**Constraints**:
- Target **15–50 KB** per image (radio time is expensive)
- Decode on **Pi Zero 2W** (ARM Cortex-A53, 512 MB RAM)
- Encode on desktop/Pi 4B (ARM Cortex-A72 or x86_64)
- MCUs (ESP32, nRF52840, RP2040) skip image payloads entirely
- Maintained Rust clients must encode and decode the format
- The Styrene wire protocol is the shared contract

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
