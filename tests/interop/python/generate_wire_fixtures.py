#!/usr/bin/env python3
"""Generate binary wire protocol fixtures for Rust interop testing.

Standalone — reimplements the wire format encoding to avoid import issues.
The format must match styrened/src/styrened/models/styrene_wire.py exactly.

Wire Format v2:
    [PREFIX:11][VERSION:1][TYPE:1][REQUEST_ID:16][PAYLOAD:N]
    PREFIX = b"styrene.io:"
"""
import json
import os

import msgpack

FIXTURE_DIR = os.path.join(os.path.dirname(__file__), "..", "fixtures")
os.makedirs(FIXTURE_DIR, exist_ok=True)

# Python authority uses b"styrene.io:" (11 bytes) + version 2
# Rust currently uses b"styrene.io" (10 bytes) + version 1
# Fixtures are generated in PYTHON (authority) format.
# The Rust wire module must be updated to match.
PREFIX = b"styrene.io:"
V1 = 1
V2 = 2
NO_CORRELATION = b"\x00" * 16
FIXED_REQ_ID = bytes(range(16))  # 00 01 02 ... 0f

# Message types (must match StyreneMessageType enum)
PING = 0x01
PONG = 0x02
HEARTBEAT = 0x03
STATUS_REQUEST = 0x10
STATUS_RESPONSE = 0x11
CHAT = 0x20
CHAT_ACK = 0x21
FILE_CHUNK = 0x24
ANNOUNCE = 0x30
EXEC = 0x40
EXEC_RESULT = 0x60
TERMINAL_REQUEST = 0xC0
PQC_INITIATE = 0xD0
ERROR = 0xFF

TYPE_NAMES = {
    0x01: "PING", 0x02: "PONG", 0x03: "HEARTBEAT",
    0x10: "STATUS_REQUEST", 0x11: "STATUS_RESPONSE",
    0x20: "CHAT", 0x21: "CHAT_ACK", 0x24: "FILE_CHUNK",
    0x30: "ANNOUNCE",
    0x40: "EXEC", 0x60: "EXEC_RESULT",
    0xC0: "TERMINAL_REQUEST", 0xD0: "PQC_INITIATE",
    0xFF: "ERROR",
}


def encode_v2(msg_type: int, payload: bytes, req_id: bytes = FIXED_REQ_ID) -> bytes:
    return PREFIX + bytes([V2, msg_type]) + req_id + payload


def encode_v1(msg_type: int, payload: bytes) -> bytes:
    return PREFIX + bytes([V1, msg_type]) + payload


manifest = []


def fixture(name: str, wire_bytes: bytes, version: int, msg_type: int,
            req_id: bytes | None, payload: bytes, description: str):
    filename = f"wire_{name}.bin"
    with open(os.path.join(FIXTURE_DIR, filename), "wb") as f:
        f.write(wire_bytes)

    manifest.append({
        "name": name,
        "file": filename,
        "description": description,
        "version": version,
        "message_type": msg_type,
        "message_type_name": TYPE_NAMES.get(msg_type, f"0x{msg_type:02x}"),
        "request_id_hex": req_id.hex() if req_id else None,
        "payload_hex": payload.hex() if payload else "",
        "wire_hex": wire_bytes.hex(),
        "wire_length": len(wire_bytes),
    })
    print(f"  {name}: {len(wire_bytes)} bytes")


# ── V2 fixtures ──────────────────────────────────────────────────────────────

print("V2 fixtures:")

fixture("v2_ping_empty", encode_v2(PING, b""), V2, PING, FIXED_REQ_ID, b"",
        "PING with empty payload")

fixture("v2_pong_empty", encode_v2(PONG, b""), V2, PONG, FIXED_REQ_ID, b"",
        "PONG with empty payload")

fixture("v2_heartbeat", encode_v2(HEARTBEAT, b""), V2, HEARTBEAT, FIXED_REQ_ID, b"",
        "HEARTBEAT with empty payload")

status_payload = msgpack.packb({"uptime": 3600, "version": "0.10.70", "peers": 5})
fixture("v2_status_response", encode_v2(STATUS_RESPONSE, status_payload),
        V2, STATUS_RESPONSE, FIXED_REQ_ID, status_payload,
        "STATUS_RESPONSE with uptime/version/peers")

chat_payload = msgpack.packb({"text": "Hello from Python!", "timestamp": 1776000000})
fixture("v2_chat", encode_v2(CHAT, chat_payload),
        V2, CHAT, FIXED_REQ_ID, chat_payload,
        "CHAT with text and timestamp")

exec_payload = msgpack.packb({"command": "uptime", "args": []})
fixture("v2_exec", encode_v2(EXEC, exec_payload),
        V2, EXEC, FIXED_REQ_ID, exec_payload,
        "EXEC command")

exec_result_payload = msgpack.packb({"exit_code": 0, "stdout": "up 14 days", "stderr": ""})
fixture("v2_exec_result", encode_v2(EXEC_RESULT, exec_result_payload),
        V2, EXEC_RESULT, FIXED_REQ_ID, exec_result_payload,
        "EXEC_RESULT with output")

fixture("v2_announce_no_correlation", encode_v2(ANNOUNCE, b"", NO_CORRELATION),
        V2, ANNOUNCE, NO_CORRELATION, b"",
        "ANNOUNCE with NO_CORRELATION request_id")

terminal_payload = msgpack.packb({"rows": 24, "cols": 80, "term": "xterm-256color"})
fixture("v2_terminal_request", encode_v2(TERMINAL_REQUEST, terminal_payload),
        V2, TERMINAL_REQUEST, FIXED_REQ_ID, terminal_payload,
        "TERMINAL_REQUEST with dimensions")

# PQC types are feature-gated in Rust — skip for default interop fixtures
# fixture("v2_pqc_initiate", encode_v2(PQC_INITIATE, b""),
#         V2, PQC_INITIATE, FIXED_REQ_ID, b"",
#         "PQC_INITIATE placeholder")

error_payload = msgpack.packb({"code": "AUTH_FAILED", "message": "Invalid credentials"})
fixture("v2_error", encode_v2(ERROR, error_payload),
        V2, ERROR, FIXED_REQ_ID, error_payload,
        "ERROR with code and message")

large_payload = msgpack.packb({"data": "x" * 1000})
fixture("v2_chat_large", encode_v2(CHAT, large_payload),
        V2, CHAT, FIXED_REQ_ID, large_payload,
        "CHAT with 1KB payload")

fixture("v2_file_chunk_binary", encode_v2(FILE_CHUNK, bytes(range(256))),
        V2, FILE_CHUNK, FIXED_REQ_ID, bytes(range(256)),
        "FILE_CHUNK with raw binary payload")

# ── V1 fixtures ──────────────────────────────────────────────────────────────

print("V1 fixtures:")

fixture("v1_ping", encode_v1(PING, b""), V1, PING, None, b"",
        "V1 PING (no request_id)")

fixture("v1_chat", encode_v1(CHAT, chat_payload), V1, CHAT, None, chat_payload,
        "V1 CHAT with payload (no request_id)")

# ── Write manifest ───────────────────────────────────────────────────────────

manifest_path = os.path.join(FIXTURE_DIR, "wire_manifest.json")
with open(manifest_path, "w") as f:
    json.dump(manifest, f, indent=2)

print(f"\n{len(manifest)} fixtures → {manifest_path}")
