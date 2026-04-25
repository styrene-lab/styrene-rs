#!/usr/bin/env python3
"""IPC contract test — Python client connects to Rust daemon via Unix socket.

Exercises the IPC wire protocol to verify Python ↔ Rust compatibility.
The Rust daemon must be running with --socket <path> before this script runs.

Usage:
    python3 tests/interop/python/ipc_contract.py /path/to/daemon.sock

Exit code 0 = all tests passed, non-zero = failures.
"""
import json
import os
import socket
import struct
import sys
import time

import msgpack

# IPC wire format: [LENGTH:4][TYPE:1][REQUEST_ID:16][PAYLOAD:N]
# LENGTH is the total frame size EXCLUDING the 4-byte length prefix.
REQUEST_ID_SIZE = 16

# Message types (must match Rust MessageType enum)
PING = 0x01
PONG = 0x80
QUERY_STATUS = 0x12
QUERY_IDENTITY = 0x11
QUERY_DEVICES = 0x10
QUERY_AUTO_REPLY = 0x19
CMD_ANNOUNCE = 0x22
RESULT = 0x81
ERROR = 0x82


def make_request_id():
    return os.urandom(REQUEST_ID_SIZE)


def encode_frame(msg_type: int, request_id: bytes, payload: dict) -> bytes:
    payload_bytes = msgpack.packb(payload) if payload else b""
    frame = bytes([msg_type]) + request_id + payload_bytes
    return struct.pack(">I", len(frame)) + frame


def decode_frame(sock) -> tuple:
    """Read one frame from the socket. Returns (msg_type, request_id, payload_dict)."""
    len_buf = recv_exact(sock, 4)
    total = struct.unpack(">I", len_buf)[0]
    frame = recv_exact(sock, total)

    msg_type = frame[0]
    request_id = frame[1:1 + REQUEST_ID_SIZE]
    payload_bytes = frame[1 + REQUEST_ID_SIZE:]

    if payload_bytes:
        payload = msgpack.unpackb(payload_bytes, raw=False)
    else:
        payload = {}

    return msg_type, request_id, payload


def recv_exact(sock, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf += chunk
    return buf


def send_and_recv(sock, msg_type: int, payload: dict = None) -> tuple:
    req_id = make_request_id()
    frame = encode_frame(msg_type, req_id, payload or {})
    sock.sendall(frame)
    return decode_frame(sock)


class ContractTest:
    def __init__(self, sock_path: str):
        self.sock_path = sock_path
        self.sock = None
        self.passed = 0
        self.failed = 0
        self.errors = []

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(5.0)
        self.sock.connect(self.sock_path)

    def close(self):
        if self.sock:
            self.sock.close()

    def test(self, name: str, fn):
        try:
            fn()
            self.passed += 1
            print(f"  ✓ {name}")
        except Exception as e:
            self.failed += 1
            self.errors.append((name, str(e)))
            print(f"  ✗ {name}: {e}")

    def run(self):
        print(f"IPC contract tests → {self.sock_path}\n")
        self.connect()

        self.test("ping_pong", self.test_ping_pong)
        self.test("query_status", self.test_query_status)
        self.test("query_identity", self.test_query_identity)
        self.test("query_devices", self.test_query_devices)
        self.test("query_auto_reply", self.test_query_auto_reply)
        self.test("announce", self.test_announce)
        self.test("request_id_correlation", self.test_request_id_correlation)

        self.close()

        print(f"\n{self.passed} passed, {self.failed} failed")
        if self.errors:
            print("\nFailures:")
            for name, err in self.errors:
                print(f"  {name}: {err}")

        return self.failed == 0

    # ── Individual tests ──────────────────────────────────────────────────

    def test_ping_pong(self):
        msg_type, req_id, payload = send_and_recv(self.sock, PING)
        assert msg_type == PONG, f"expected PONG (0x{PONG:02x}), got 0x{msg_type:02x}"

    def test_query_status(self):
        msg_type, req_id, payload = send_and_recv(self.sock, QUERY_STATUS)
        assert msg_type == RESULT, f"expected RESULT, got 0x{msg_type:02x}"
        assert "uptime" in payload, f"missing 'uptime' in response: {payload.keys()}"
        assert "daemon_version" in payload, f"missing 'daemon_version'"

    def test_query_identity(self):
        msg_type, req_id, payload = send_and_recv(self.sock, QUERY_IDENTITY)
        assert msg_type == RESULT, f"expected RESULT, got 0x{msg_type:02x}"
        assert "identity_hash" in payload, f"missing 'identity_hash'"
        assert len(payload["identity_hash"]) > 0, "identity_hash is empty"

    def test_query_devices(self):
        msg_type, req_id, payload = send_and_recv(self.sock, QUERY_DEVICES)
        assert msg_type == RESULT, f"expected RESULT, got 0x{msg_type:02x}"
        assert "devices" in payload, f"missing 'devices' in response"
        assert isinstance(payload["devices"], list), "devices is not a list"

    def test_query_auto_reply(self):
        msg_type, req_id, payload = send_and_recv(self.sock, QUERY_AUTO_REPLY)
        assert msg_type == RESULT, f"expected RESULT, got 0x{msg_type:02x}"
        assert "mode" in payload, f"missing 'mode' in response"

    def test_announce(self):
        msg_type, req_id, payload = send_and_recv(self.sock, CMD_ANNOUNCE)
        assert msg_type == RESULT, f"expected RESULT, got 0x{msg_type:02x}"

    def test_request_id_correlation(self):
        """Verify request_id in response matches request."""
        req_id = make_request_id()
        frame = encode_frame(PING, req_id, {})
        self.sock.sendall(frame)
        resp_type, resp_id, _ = decode_frame(self.sock)
        assert resp_type == PONG
        assert resp_id == req_id, f"request_id mismatch: sent {req_id.hex()}, got {resp_id.hex()}"


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <socket_path>")
        sys.exit(1)

    tester = ContractTest(sys.argv[1])
    success = tester.run()
    sys.exit(0 if success else 1)
