# styrene-mesh

Wire protocol envelope format for Styrene mesh communications. Shared contract between the Rust and Python (`styrened`) implementations -- both must produce byte-identical output.

## Wire Format v2

```
[namespace:11][version:1][type:1][request_id:16][payload:variable]
 "styrene.io:"   0x02     enum    random bytes    msgpack-encoded
```

Header is 29 bytes. Payload is msgpack (planned migration to CBOR -- see workspace CLAUDE.md).

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Crate root, re-exports, constants (`NAMESPACE`, `WIRE_VERSION`) |
| `src/wire.rs` | `StyreneMessage` encode/decode, `StyreneMessageType` enum, content distribution payloads |
| `src/pqc.rs` | Post-quantum cryptography tunnel types (behind `pqc` feature) |

## Key Types

- **`StyreneMessage`** -- envelope: version, message_type, request_id (16 bytes), payload (raw bytes). `encode()` / `decode()` for wire format.
- **`StyreneMessageType`** -- `#[repr(u8)]` enum. Ranges: Control (0x01-0x0F), Status (0x10-0x1F), Content (0x20-0x2F), Network (0x30-0x3F), RPC Cmd (0x40-0x5F), RPC Resp (0x60-0x7F), Terminal (0xC0-0xCF), PQC (0xD0-0xD7, feature-gated), Content Distribution (0xE0-0xE3), Error (0xFF).
- **`WireError`** -- TooShort, InvalidNamespace, UnsupportedVersion, UnknownMessageType, MsgpackDecode/Encode.
- **`ResourceAvailablePayload`**, **`ChunkRequestPayload`**, **`ChunkResponsePayload`** -- content distribution system payloads (0xE0-0xE2).

## Feature Flags

| Flag | Effect |
|------|--------|
| `std` (default) | Std library support |
| `pqc` | Enables PQC message types (0xD0-0xD7) and `pqc` module |
| `interop-tests` | Gates cross-language interop tests |

## Dependencies

- `styrene-rns` -- RNS protocol core (identity types)
- `rmp-serde` / `rmpv` -- msgpack encoding
- `rand_core` -- random request ID generation

## Test Commands

```bash
cargo test -p styrene-mesh
cargo test -p styrene-mesh --features pqc
cargo test -p styrene-mesh --features interop-tests
```

## Gotchas

- Wire format must match Python `styrened/src/styrened/models/styrene_wire.py` byte-for-byte. Any changes require synchronized Python updates.
- `HEADER_SIZE` constant comment says 28 but value is 29 (11 + 1 + 1 + 16). The 29 is correct.
- `StyreneMessage.version` doc comment says "currently always 0x01" but `WIRE_VERSION` is `0x02`. The code is correct, comment is stale.
- PQC message types are compile-time gated -- `from_byte()` won't recognize 0xD0-0xD7 without `features = ["pqc"]`.

## Status

Stable. Wire protocol is in production use. Content distribution payloads (0xE0-0xE2) are implemented. Planned migration from msgpack to CBOR (RFC 8949) is documented but not started.
