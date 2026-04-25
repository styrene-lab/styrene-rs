#!/usr/bin/env python3
"""
Generate IFAC (Interface Access Code) interop test vectors for styrene-rs.

Standalone reimplementation of the IFAC algorithm using the same crypto
primitives (Ed25519 + HKDF-SHA256). Does NOT import the RNS library —
vectors are verified by both this implementation and the Rust implementation
producing byte-identical output. Run from repo root:

    cd tests/interop/python && python3 generate_ifac_fixtures.py

Outputs JSON to ../fixtures/ifac_vectors.json.

IFAC algorithm summary:
  1. Sign the inner packet (IFAC flag cleared) with the shared Ed25519 key
  2. Take last `ifac_size` bytes of the 64-byte signature as the IFAC token
  3. Derive XOR mask: HKDF-SHA256(ikm=token, salt=ifac_key, length=wire_len)
  4. Assemble wire: (flags|0x80) + hops + token + payload[2:]
  5. XOR-mask bytes 0, 1, and (2+ifac_size)..end — token bytes are NOT masked
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from pathlib import Path

# We need raw crypto primitives, not the full RNS Interface stack
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.kdf.hkdf import HKDF as _HKDF
from cryptography.hazmat.primitives import hashes, serialization

FIXTURES_DIR = Path(__file__).resolve().parent.parent / "fixtures"

DEFAULT_IFAC_SIZE = 8


def sha256(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def make_seed(label: str) -> bytes:
    """Derive a deterministic 32-byte seed from a label."""
    return sha256(label.encode("utf-8"))


def make_ed25519_key(label: str) -> Ed25519PrivateKey:
    """Create a deterministic Ed25519 key from a label."""
    seed = make_seed(label)
    return Ed25519PrivateKey.from_private_bytes(seed)


def hkdf_expand(ikm: bytes, salt: bytes, length: int) -> bytes:
    """HKDF-SHA256 expand (extract+expand)."""
    hkdf = _HKDF(
        algorithm=hashes.SHA256(),
        length=length,
        salt=salt,
        info=b"",
    )
    return hkdf.derive(ikm)


def ifac_wrap(raw: bytes, ifac_key: bytes, sign_key: Ed25519PrivateKey,
              ifac_size: int = DEFAULT_IFAC_SIZE) -> bytes:
    """
    Wrap a raw packet with IFAC authentication.

    `raw` is a serialized RNS packet WITHOUT the IFAC flag set.
    Returns the masked, IFAC-wrapped bytes.
    """
    # Sign the inner packet; take last ifac_size bytes as the token
    signature = sign_key.sign(raw)
    token = signature[-ifac_size:]

    # Build unmasked wire: (flags | 0x80) + hops + token + rest_of_packet
    wire = bytearray()
    wire.append(raw[0] | 0x80)  # Set IFAC flag
    wire.append(raw[1])          # hops
    wire.extend(token)           # IFAC token (unmasked)
    wire.extend(raw[2:])         # rest of inner packet

    # Derive XOR mask
    mask = hkdf_expand(bytes(token), ifac_key, len(wire))

    # Apply mask selectively
    wire[0] ^= mask[0]
    wire[0] |= 0x80  # Keep IFAC flag set after masking
    wire[1] ^= mask[1]  # Mask hops
    # Bytes 2..2+ifac_size: token — NOT masked
    # Remaining bytes: masked
    for i in range(2 + ifac_size, len(wire)):
        wire[i] ^= mask[i]

    return bytes(wire)


def ifac_unwrap(wire: bytes, ifac_key: bytes, sign_key: Ed25519PrivateKey,
                ifac_size: int = DEFAULT_IFAC_SIZE) -> bytes | None:
    """
    Unwrap an IFAC-authenticated packet.

    Returns the inner packet bytes if IFAC verification succeeds, None otherwise.
    """
    if len(wire) < 2 + ifac_size:
        return None

    wire = bytearray(wire)

    # Extract token (unmasked, bytes 2..2+ifac_size)
    token = bytes(wire[2:2 + ifac_size])

    # Derive mask
    mask = hkdf_expand(token, ifac_key, len(wire))

    # Unmask
    wire[0] ^= mask[0]
    wire[0] |= 0x80  # Preserve IFAC flag for now
    wire[1] ^= mask[1]
    for i in range(2 + ifac_size, len(wire)):
        wire[i] ^= mask[i]

    # Reconstruct inner packet: clear IFAC flag, strip token
    inner = bytearray()
    inner.append(wire[0] & 0x7F)  # Clear IFAC flag
    inner.append(wire[1])          # hops
    inner.extend(wire[2 + ifac_size:])  # payload after token

    inner_bytes = bytes(inner)

    # Verify: re-sign inner and compare token
    verify_sig = sign_key.sign(inner_bytes)
    verify_token = verify_sig[-ifac_size:]

    if verify_token == token:
        return inner_bytes
    return None


def generate_vectors() -> list[dict]:
    vectors = []

    # --- Vector 1: Basic wrap/unwrap ---
    key1 = make_ed25519_key("ifac-test-key-1")
    ifac_key1 = sha256(b"interface-secret-1")
    inner1 = bytes([0x00, 0x00]) + b"hello IFAC"  # flags=0, hops=0, payload
    wrapped1 = ifac_wrap(inner1, ifac_key1, key1)
    unwrapped1 = ifac_unwrap(wrapped1, ifac_key1, key1)
    assert unwrapped1 == inner1, "roundtrip failed for vector 1"

    # Get raw private key bytes for the Rust side
    key1_bytes = key1.private_bytes(
        serialization.Encoding.Raw,
        serialization.PrivateFormat.Raw,
        serialization.NoEncryption(),
    )

    vectors.append({
        "description": "basic wrap/unwrap with default ifac_size=8",
        "sign_key_hex": key1_bytes.hex(),
        "ifac_key_hex": ifac_key1.hex(),
        "ifac_size": DEFAULT_IFAC_SIZE,
        "inner_hex": inner1.hex(),
        "wrapped_hex": wrapped1.hex(),
    })

    # --- Vector 2: Non-zero hops ---
    inner2 = bytes([0x40, 0x03]) + b"hopped packet"  # flags=0x40, hops=3
    wrapped2 = ifac_wrap(inner2, ifac_key1, key1)
    unwrapped2 = ifac_unwrap(wrapped2, ifac_key1, key1)
    assert unwrapped2 == inner2, "roundtrip failed for vector 2"

    vectors.append({
        "description": "packet with non-zero hops and flags",
        "sign_key_hex": key1_bytes.hex(),
        "ifac_key_hex": ifac_key1.hex(),
        "ifac_size": DEFAULT_IFAC_SIZE,
        "inner_hex": inner2.hex(),
        "wrapped_hex": wrapped2.hex(),
    })

    # --- Vector 3: Small ifac_size (4 bytes) ---
    inner3 = bytes([0x00, 0x00]) + b"small token"
    wrapped3 = ifac_wrap(inner3, ifac_key1, key1, ifac_size=4)
    unwrapped3 = ifac_unwrap(wrapped3, ifac_key1, key1, ifac_size=4)
    assert unwrapped3 == inner3, "roundtrip failed for vector 3"

    vectors.append({
        "description": "small ifac_size=4",
        "sign_key_hex": key1_bytes.hex(),
        "ifac_key_hex": ifac_key1.hex(),
        "ifac_size": 4,
        "inner_hex": inner3.hex(),
        "wrapped_hex": wrapped3.hex(),
    })

    # --- Vector 4: Large ifac_size (16 bytes) ---
    inner4 = bytes([0x00, 0x00]) + b"large token test data payload"
    wrapped4 = ifac_wrap(inner4, ifac_key1, key1, ifac_size=16)
    unwrapped4 = ifac_unwrap(wrapped4, ifac_key1, key1, ifac_size=16)
    assert unwrapped4 == inner4, "roundtrip failed for vector 4"

    vectors.append({
        "description": "large ifac_size=16",
        "sign_key_hex": key1_bytes.hex(),
        "ifac_key_hex": ifac_key1.hex(),
        "ifac_size": 16,
        "inner_hex": inner4.hex(),
        "wrapped_hex": wrapped4.hex(),
    })

    # --- Vector 5: Different key ---
    key2 = make_ed25519_key("ifac-test-key-2")
    ifac_key2 = sha256(b"interface-secret-2")
    key2_bytes = key2.private_bytes(
        serialization.Encoding.Raw,
        serialization.PrivateFormat.Raw,
        serialization.NoEncryption(),
    )
    inner5 = bytes([0x20, 0x01]) + b"different key"
    wrapped5 = ifac_wrap(inner5, ifac_key2, key2)
    unwrapped5 = ifac_unwrap(wrapped5, ifac_key2, key2)
    assert unwrapped5 == inner5, "roundtrip failed for vector 5"

    # Verify wrong key rejects
    wrong_unwrap = ifac_unwrap(wrapped5, ifac_key1, key1)
    assert wrong_unwrap is None, "wrong key should reject"

    vectors.append({
        "description": "different key pair",
        "sign_key_hex": key2_bytes.hex(),
        "ifac_key_hex": ifac_key2.hex(),
        "ifac_size": DEFAULT_IFAC_SIZE,
        "inner_hex": inner5.hex(),
        "wrapped_hex": wrapped5.hex(),
    })

    # --- Vector 6: Binary payload (all byte values) ---
    inner6 = bytes([0x00, 0x00]) + bytes(range(256))
    wrapped6 = ifac_wrap(inner6, ifac_key1, key1)
    unwrapped6 = ifac_unwrap(wrapped6, ifac_key1, key1)
    assert unwrapped6 == inner6, "roundtrip failed for vector 6"

    vectors.append({
        "description": "binary payload with all 256 byte values",
        "sign_key_hex": key1_bytes.hex(),
        "ifac_key_hex": ifac_key1.hex(),
        "ifac_size": DEFAULT_IFAC_SIZE,
        "inner_hex": inner6.hex(),
        "wrapped_hex": wrapped6.hex(),
    })

    return vectors


def main():
    FIXTURES_DIR.mkdir(parents=True, exist_ok=True)
    vectors = generate_vectors()

    output_path = FIXTURES_DIR / "ifac_vectors.json"
    with open(output_path, "w") as f:
        json.dump(vectors, f, indent=2)

    print(f"wrote {len(vectors)} IFAC vectors to {output_path}")


if __name__ == "__main__":
    main()
