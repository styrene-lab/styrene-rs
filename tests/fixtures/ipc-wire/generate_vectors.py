#!/usr/bin/env python3
"""Generate IPC wire protocol test vectors for cross-language validation.

Produces binary (.bin) and JSON sidecar (.json) files for each test case.
Rust tests in styrene-ipc-server consume these to verify wire compatibility.

Run from the styrened venv:
    python tests/fixtures/ipc-wire/generate_vectors.py
"""
import json
import os
import struct
import sys

# Add styrened to path
sys.path.insert(0, os.path.expanduser("~/workspace/styrene-lab/styrened/src"))

import msgpack
from styrened.ipc.protocol import IPCMessageType, encode_frame

OUTPUT_DIR = os.path.dirname(os.path.abspath(__file__))

# Fixed request_id for deterministic test vectors
FIXED_REQUEST_ID = bytes(range(16))  # 0x00..0x0f


def write_vector(name: str, msg_type: IPCMessageType, payload: dict):
    """Write a test vector as .bin (raw frame) and .json (metadata)."""
    frame = encode_frame(msg_type, FIXED_REQUEST_ID, payload)

    bin_path = os.path.join(OUTPUT_DIR, f"{name}.bin")
    json_path = os.path.join(OUTPUT_DIR, f"{name}.json")

    with open(bin_path, "wb") as f:
        f.write(frame)

    # Decode the frame to verify and extract parts
    length = struct.unpack(">I", frame[:4])[0]
    type_byte = frame[4]
    req_id = frame[5:21].hex()
    payload_bytes = frame[21:]
    decoded_payload = msgpack.unpackb(payload_bytes, raw=False)

    meta = {
        "name": name,
        "msg_type": msg_type.name,
        "msg_type_byte": f"0x{type_byte:02x}",
        "request_id_hex": req_id,
        "frame_length": len(frame),
        "length_field": length,
        "payload": decoded_payload,
    }

    with open(json_path, "w") as f:
        json.dump(meta, f, indent=2, default=str)

    print(f"  {name}: {len(frame)} bytes (type=0x{type_byte:02x})")


def main():
    print("Generating IPC wire protocol test vectors...")

    # --- Keepalive ---
    write_vector("ping", IPCMessageType.PING, {})
    write_vector("pong", IPCMessageType.PONG, {})

    # --- Queries ---
    write_vector("query_status", IPCMessageType.QUERY_STATUS, {})
    write_vector("query_identity", IPCMessageType.QUERY_IDENTITY, {})
    write_vector("query_devices", IPCMessageType.QUERY_DEVICES, {"limit": 100})
    write_vector("query_auto_reply", IPCMessageType.QUERY_AUTO_REPLY, {})
    write_vector("query_conversations", IPCMessageType.QUERY_CONVERSATIONS, {"unread_only": False})
    write_vector("query_messages", IPCMessageType.QUERY_MESSAGES, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
        "limit": 50,
    })
    write_vector("query_search_messages", IPCMessageType.QUERY_SEARCH_MESSAGES, {
        "query": "hello world",
        "limit": 20,
    })
    write_vector("query_contacts", IPCMessageType.QUERY_CONTACTS, {})
    write_vector("query_resolve_name", IPCMessageType.QUERY_RESOLVE_NAME, {
        "name": "AlphaNode",
    })

    # --- Commands ---
    write_vector("cmd_announce", IPCMessageType.CMD_ANNOUNCE, {})
    write_vector("cmd_send_chat", IPCMessageType.CMD_SEND_CHAT, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
        "content": "Hello from Python!",
    })
    write_vector("cmd_mark_read", IPCMessageType.CMD_MARK_READ, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
    })
    write_vector("cmd_delete_conversation", IPCMessageType.CMD_DELETE_CONVERSATION, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
    })
    write_vector("cmd_delete_message", IPCMessageType.CMD_DELETE_MESSAGE, {
        "message_id": "msg_abc123",
    })
    write_vector("cmd_set_auto_reply", IPCMessageType.CMD_SET_AUTO_REPLY, {
        "mode": "enabled",
        "message": "I'm away",
        "cooldown_secs": 300,
    })
    write_vector("cmd_set_identity", IPCMessageType.CMD_SET_IDENTITY, {
        "display_name": "TestOperator",
        "icon": "🔑",
    })
    write_vector("cmd_retry_message", IPCMessageType.CMD_RETRY_MESSAGE, {
        "message_id": "msg_retry_123",
    })
    write_vector("cmd_set_contact", IPCMessageType.CMD_SET_CONTACT, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
        "alias": "Alice",
        "notes": "Met at conference",
    })
    write_vector("cmd_remove_contact", IPCMessageType.CMD_REMOVE_CONTACT, {
        "peer_hash": "abcdef0123456789abcdef0123456789",
    })

    # --- Subscriptions ---
    write_vector("sub_devices", IPCMessageType.SUB_DEVICES, {})
    write_vector("sub_messages", IPCMessageType.SUB_MESSAGES, {
        "peer_hashes": ["aabb", "ccdd"],
    })

    # --- Responses ---
    write_vector("result_ok", IPCMessageType.RESULT, {
        "status": "ok",
        "data": {"uptime": 3600, "version": "0.15.0"},
    })
    write_vector("error_response", IPCMessageType.ERROR, {
        "error": "not_found",
        "message": "Device not found",
    })

    # --- Events ---
    write_vector("event_device", IPCMessageType.EVENT_DEVICE, {
        "peer_hash": "abcdef0123456789",
        "name": "RemoteNode",
        "event": "announce",
    })
    write_vector("event_message", IPCMessageType.EVENT_MESSAGE, {
        "id": "msg_event_1",
        "source_hash": "1122334455667788",
        "content": "New message arrived",
        "kind": "new",
    })

    print(f"\nGenerated {len(os.listdir(OUTPUT_DIR)) // 2} test vectors in {OUTPUT_DIR}")


if __name__ == "__main__":
    main()
