#!/usr/bin/env python3
"""
Generate interop test vector fixtures for styrene-rs.

Uses the real Python RNS library to produce test vectors that the Rust
implementation verifies against.  Run from repo root:

    cd tests/interop/python && python3 generate_fixtures.py

Outputs JSON files into ../fixtures/.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import time
from pathlib import Path

import RNS
from RNS.Cryptography import Token as RNSToken

FIXTURES_DIR = Path(__file__).resolve().parent.parent / "fixtures"

ADDRESS_HASH_SIZE = 16  # RNS.Identity.TRUNCATED_HASHLENGTH // 8
NAME_HASH_LENGTH = 10
RAND_HASH_LENGTH = 10
SIGNATURE_LENGTH = 64
PUBLIC_KEY_LENGTH = 32


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def to_hex(data: bytes | bytearray) -> str:
    return data.hex()


def sha256(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def make_seed(label: str) -> bytes:
    """Derive a deterministic 64-byte seed from a human-readable label."""
    h1 = sha256(label.encode("utf-8"))
    h2 = sha256(h1)
    return h1 + h2


def make_identity(label: str) -> RNS.Identity:
    """Create an RNS Identity from a deterministic seed."""
    seed = make_seed(label)
    return RNS.Identity.from_bytes(seed)


# ---------------------------------------------------------------------------
# 1. Identity vectors
# ---------------------------------------------------------------------------

def generate_identity_vectors() -> list[dict]:
    vectors = []
    labels = ["alice", "bob", "carol"]

    for label in labels:
        seed = make_seed(label)
        ident = RNS.Identity.from_bytes(seed)

        test_data = f"test message for {label}".encode("utf-8")
        signature = ident.sign(test_data)

        # Self-verify
        assert ident.validate(signature, test_data), f"self-verify failed for {label}"

        vectors.append({
            "description": f"Identity derived from '{label}'",
            "private_key_hex": to_hex(seed),
            "public_key_hex": to_hex(ident.pub_bytes),
            "verifying_key_hex": to_hex(ident.sig_pub_bytes),
            "address_hash_hex": to_hex(ident.hash),
            "sign_data_hex": to_hex(test_data),
            "signature_hex": to_hex(signature),
        })

    return vectors


# ---------------------------------------------------------------------------
# 2. Packet vectors
# ---------------------------------------------------------------------------

def generate_packet_vectors() -> list[dict]:
    """
    Build raw packet bytes matching the RNS wire format.

    Wire format:
        [flags:1][hops:1][transport?:16][destination:16][context:1][data:N]

    Flags byte (MSB to LSB):
        bit 7: ifac_flag
        bit 6: header_type (0=Type1, 1=Type2)
        bit 5: context_flag
        bit 4: propagation_type
        bits 3-2: destination_type
        bits 1-0: packet_type
    """
    vectors = []

    def make_flags(ifac=0, htype=0, cflag=0, prop=0, dtype=0, ptype=0) -> int:
        return (
            (ifac & 1) << 7
            | (htype & 1) << 6
            | (cflag & 1) << 5
            | (prop & 1) << 4
            | (dtype & 3) << 2
            | (ptype & 3)
        )

    # Vector 1: Type1 Data packet, Single destination, broadcast
    dest1 = bytes(range(ADDRESS_HASH_SIZE))
    data1 = b"Hello from Python RNS"
    flags1 = make_flags(ifac=0, htype=0, cflag=0, prop=0, dtype=0, ptype=0)
    raw1 = bytes([flags1, 0]) + dest1 + bytes([0x00]) + data1
    vectors.append({
        "description": "Type1 Data packet, Single destination, broadcast",
        "raw_hex": to_hex(raw1),
        "header_flags": flags1,
        "hops": 0,
        "ifac_flag": "open",
        "header_type": "type1",
        "context_flag": "unset",
        "propagation_type": "broadcast",
        "destination_type": "single",
        "packet_type": "data",
        "destination_hex": to_hex(dest1),
        "transport_hex": None,
        "context": 0x00,
        "data_hex": to_hex(data1),
    })

    # Vector 2: Type1 Announce packet, Single destination, broadcast, 3 hops
    dest2 = sha256(b"announce-dest")[:ADDRESS_HASH_SIZE]
    data2 = os.urandom(148)
    flags2 = make_flags(ifac=0, htype=0, cflag=0, prop=0, dtype=0, ptype=1)
    raw2 = bytes([flags2, 3]) + dest2 + bytes([0x00]) + data2
    vectors.append({
        "description": "Type1 Announce packet, Single, 3 hops",
        "raw_hex": to_hex(raw2),
        "header_flags": flags2,
        "hops": 3,
        "ifac_flag": "open",
        "header_type": "type1",
        "context_flag": "unset",
        "propagation_type": "broadcast",
        "destination_type": "single",
        "packet_type": "announce",
        "destination_hex": to_hex(dest2),
        "transport_hex": None,
        "context": 0x00,
        "data_hex": to_hex(data2),
    })

    # Vector 3: Type2 Data packet with transport address
    dest3 = sha256(b"type2-dest")[:ADDRESS_HASH_SIZE]
    transport3 = sha256(b"transport-addr")[:ADDRESS_HASH_SIZE]
    data3 = b"Type2 payload"
    flags3 = make_flags(ifac=0, htype=1, cflag=0, prop=1, dtype=0, ptype=0)
    raw3 = bytes([flags3, 5]) + transport3 + dest3 + bytes([0x00]) + data3
    vectors.append({
        "description": "Type2 Data packet with transport, 5 hops",
        "raw_hex": to_hex(raw3),
        "header_flags": flags3,
        "hops": 5,
        "ifac_flag": "open",
        "header_type": "type2",
        "context_flag": "unset",
        "propagation_type": "transport",
        "destination_type": "single",
        "packet_type": "data",
        "destination_hex": to_hex(dest3),
        "transport_hex": to_hex(transport3),
        "context": 0x00,
        "data_hex": to_hex(data3),
    })

    # Vector 4: Type1 LinkRequest, Link destination
    dest4 = sha256(b"link-dest")[:ADDRESS_HASH_SIZE]
    data4 = b"link-request-data"
    flags4 = make_flags(ifac=0, htype=0, cflag=0, prop=0, dtype=3, ptype=2)
    raw4 = bytes([flags4, 0]) + dest4 + bytes([0x09]) + data4
    vectors.append({
        "description": "Type1 LinkRequest, Link destination",
        "raw_hex": to_hex(raw4),
        "header_flags": flags4,
        "hops": 0,
        "ifac_flag": "open",
        "header_type": "type1",
        "context_flag": "unset",
        "propagation_type": "broadcast",
        "destination_type": "link",
        "packet_type": "linkrequest",
        "destination_hex": to_hex(dest4),
        "transport_hex": None,
        "context": 0x09,
        "data_hex": to_hex(data4),
    })

    # Vector 5: Type1 Proof, Plain dest, authenticated IFAC, 1 hop
    dest5 = sha256(b"proof-dest")[:ADDRESS_HASH_SIZE]
    data5 = os.urandom(64)
    flags5 = make_flags(ifac=1, htype=0, cflag=0, prop=0, dtype=2, ptype=3)
    raw5 = bytes([flags5, 1]) + dest5 + bytes([0x00]) + data5
    vectors.append({
        "description": "Type1 Proof, Plain dest, authenticated, 1 hop",
        "raw_hex": to_hex(raw5),
        "header_flags": flags5,
        "hops": 1,
        "ifac_flag": "authenticated",
        "header_type": "type1",
        "context_flag": "unset",
        "propagation_type": "broadcast",
        "destination_type": "plain",
        "packet_type": "proof",
        "destination_hex": to_hex(dest5),
        "transport_hex": None,
        "context": 0x00,
        "data_hex": to_hex(data5),
    })

    # Vector 6: Type2 Announce with context_flag set (ratchet indicator)
    dest6 = sha256(b"ratchet-dest")[:ADDRESS_HASH_SIZE]
    transport6 = sha256(b"ratchet-transport")[:ADDRESS_HASH_SIZE]
    data6 = os.urandom(180)
    flags6 = make_flags(ifac=0, htype=1, cflag=1, prop=1, dtype=0, ptype=1)
    raw6 = bytes([flags6, 2]) + transport6 + dest6 + bytes([0x00]) + data6
    vectors.append({
        "description": "Type2 Announce, context_flag set, transport, 2 hops",
        "raw_hex": to_hex(raw6),
        "header_flags": flags6,
        "hops": 2,
        "ifac_flag": "open",
        "header_type": "type2",
        "context_flag": "set",
        "propagation_type": "transport",
        "destination_type": "single",
        "packet_type": "announce",
        "destination_hex": to_hex(dest6),
        "transport_hex": to_hex(transport6),
        "context": 0x00,
        "data_hex": to_hex(data6),
    })

    return vectors


# ---------------------------------------------------------------------------
# 3. Announce vectors
# ---------------------------------------------------------------------------

def generate_announce_vectors() -> list[dict]:
    """
    Generate full announce packets that DestinationAnnounce::validate() can
    verify.  Built from scratch to control every field deterministically.

    Signed data layout (what the signature covers):
        [dest_hash:16][pub_key:32][verifying_key:32][name_hash:10][rand_hash:10]
        [optional ratchet:32][optional app_data:N]

    Packet data layout (in packet.data):
        [pub_key:32][verifying_key:32][name_hash:10][rand_hash:10]
        [optional ratchet:32][signature:64][app_data:N]
    """
    vectors = []

    configs = [
        {
            "label": "announce-no-appdata",
            "app_name": "testapp",
            "aspects": "delivery",
            "app_data": None,
            "description": "Announce without app_data",
        },
        {
            "label": "announce-with-appdata",
            "app_name": "myservice",
            "aspects": "receiver",
            "app_data": b"hello world",
            "description": "Announce with app_data",
        },
        {
            "label": "announce-binary-appdata",
            "app_name": "bintest",
            "aspects": "data",
            "app_data": bytes(range(256)),
            "description": "Announce with binary app_data (0x00..0xff)",
        },
    ]

    for cfg in configs:
        seed = make_seed(cfg["label"])
        ident = RNS.Identity.from_bytes(seed)

        # Compute name_hash: SHA256("app_name.aspects")[:10]
        full_name = f"{cfg['app_name']}.{cfg['aspects']}"
        name_hash = sha256(full_name.encode("utf-8"))[:NAME_HASH_LENGTH]

        # Compute destination address hash:
        #   identity_hash = SHA256(pub_key + verifying_key)[:16]
        #   dest_hash     = SHA256(name_hash[:10] + identity_hash)[:16]
        identity_hash = sha256(ident.pub_bytes + ident.sig_pub_bytes)[:ADDRESS_HASH_SIZE]
        dest_hash = sha256(name_hash + identity_hash)[:ADDRESS_HASH_SIZE]

        # Build deterministic rand_hash: 5 random-ish bytes + 5-byte timestamp
        rand_random = sha256(f"rand-{cfg['label']}".encode())[:RAND_HASH_LENGTH // 2]
        ts = int(time.time())
        ts_bytes = ts.to_bytes(8, "big")[3:8]
        rand_hash = rand_random + ts_bytes

        # Build signed data
        signed_data = dest_hash + ident.pub_bytes + ident.sig_pub_bytes + name_hash + rand_hash
        if cfg["app_data"] is not None:
            signed_data += cfg["app_data"]

        signature = ident.sign(signed_data)
        assert len(signature) == SIGNATURE_LENGTH

        # Self-verify
        assert ident.validate(signature, signed_data), "announce self-verify failed"

        # Build packet data
        packet_data = (
            ident.pub_bytes
            + ident.sig_pub_bytes
            + name_hash
            + rand_hash
            + signature
        )
        if cfg["app_data"] is not None:
            packet_data += cfg["app_data"]

        # Build wire packet: Type1, broadcast, single dest, announce type
        flags = (0 << 7) | (0 << 6) | (0 << 5) | (0 << 4) | (0 << 2) | 1
        hops = 0
        raw_packet = bytes([flags, hops]) + dest_hash + bytes([0x00]) + packet_data

        vectors.append({
            "description": cfg["description"],
            "raw_packet_hex": to_hex(raw_packet),
            "app_name": cfg["app_name"],
            "aspects": cfg["aspects"],
            "has_ratchet": False,
            "app_data_hex": to_hex(cfg["app_data"]) if cfg["app_data"] is not None else None,
            "private_key_hex": to_hex(seed),
            "public_key_hex": to_hex(ident.pub_bytes),
            "verifying_key_hex": to_hex(ident.sig_pub_bytes),
            "destination_hash_hex": to_hex(dest_hash),
            "name_hash_hex": to_hex(name_hash),
            "rand_hash_hex": to_hex(rand_hash),
            "signature_hex": to_hex(signature),
        })

    return vectors


# ---------------------------------------------------------------------------
# 4. Fernet vectors
# ---------------------------------------------------------------------------

def generate_fernet_vectors() -> list[dict]:
    """
    Generate Fernet encryption vectors using RNS's modified Fernet.

    With a 64-byte key, RNS Fernet uses AES-256-CBC + HMAC-SHA256.
    Token layout: [IV:16][ciphertext][HMAC:32]
    (No version or timestamp — stripped per RNS spec.)

    The Rust default (without fernet-aes128 feature) also uses AES-256.
    """
    vectors = []

    plaintext_lengths = [0, 1, 15, 16, 17, 31, 32, 48, 256]

    for i, pt_len in enumerate(plaintext_lengths):
        # Deterministic 64-byte key: 32 sign + 32 enc → AES-256 mode
        key = sha256(f"fernet-key-{i}".encode()) + sha256(f"fernet-enc-{i}".encode())
        assert len(key) == 64

        if pt_len == 0:
            plaintext = b""
        else:
            plaintext = bytes([b % 256 for b in range(pt_len)])

        fernet = RNSToken(key)
        token = fernet.encrypt(plaintext)

        # Roundtrip verify
        decrypted = fernet.decrypt(token)
        assert decrypted == plaintext, f"roundtrip failed for length {pt_len}"

        vectors.append({
            "description": f"Fernet AES-256, plaintext length {pt_len}",
            "sign_key_hex": to_hex(key[:32]),
            "enc_key_hex": to_hex(key[32:]),
            "plaintext_hex": to_hex(plaintext),
            "token_hex": to_hex(token),
        })

    return vectors


# ---------------------------------------------------------------------------
# 5. HDLC vectors
# ---------------------------------------------------------------------------

def generate_hdlc_vectors() -> list[dict]:
    """
    Generate HDLC framing vectors.

    Encoding rules:
        Frame delimiter: 0x7E
        Escape byte:     0x7D
        Escape mask:     0x20
        0x7E in payload → 0x7D 0x5E
        0x7D in payload → 0x7D 0x5D
        Output: [0x7E][escaped_payload][0x7E]
    """
    HDLC_FLAG = 0x7E
    HDLC_ESC = 0x7D
    HDLC_MASK = 0x20

    def hdlc_encode(data: bytes) -> bytes:
        out = bytearray([HDLC_FLAG])
        for b in data:
            if b == HDLC_FLAG or b == HDLC_ESC:
                out.append(HDLC_ESC)
                out.append(b ^ HDLC_MASK)
            else:
                out.append(b)
        out.append(HDLC_FLAG)
        return bytes(out)

    vectors = []

    test_cases = [
        ("Empty payload", b""),
        ("Simple ASCII", b"Hello"),
        ("Contains 0x7E (frame flag)", bytes([0x01, 0x7E, 0x02])),
        ("Contains 0x7D (escape byte)", bytes([0x01, 0x7D, 0x02])),
        ("Contains both 0x7E and 0x7D", bytes([0x7E, 0x7D])),
        ("All special bytes repeated", bytes([0x7E, 0x7D, 0x7E, 0x7D])),
        ("Binary payload 0-255", bytes(range(256))),
        ("Single 0x7E byte", bytes([0x7E])),
        ("Single 0x7D byte", bytes([0x7D])),
        ("Payload ending with 0x7E", bytes([0x01, 0x02, 0x7E])),
    ]

    for description, payload in test_cases:
        encoded = hdlc_encode(payload)
        vectors.append({
            "description": description,
            "decoded_hex": to_hex(payload),
            "encoded_hex": to_hex(encoded),
        })

    return vectors


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    FIXTURES_DIR.mkdir(parents=True, exist_ok=True)

    generators = [
        ("identity_vectors.json", generate_identity_vectors),
        ("packet_vectors.json", generate_packet_vectors),
        ("announce_vectors.json", generate_announce_vectors),
        ("fernet_vectors.json", generate_fernet_vectors),
        ("hdlc_vectors.json", generate_hdlc_vectors),
    ]

    for filename, gen_fn in generators:
        path = FIXTURES_DIR / filename
        try:
            vectors = gen_fn()
            with open(path, "w") as f:
                json.dump(vectors, f, indent=2)
            print(f"  OK  {filename} ({len(vectors)} vectors)")
        except Exception as e:
            print(f"  FAIL  {filename}: {e}", file=sys.stderr)
            raise

    print(f"\nAll fixtures written to {FIXTURES_DIR}")


if __name__ == "__main__":
    main()
