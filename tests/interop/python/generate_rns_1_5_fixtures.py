#!/usr/bin/env python3
"""Generate the bounded canonical RNS 1.5.1 seed fixture set."""

from __future__ import annotations

import argparse
import importlib
import json
import pathlib
import subprocess
import sys

REVISION = "149e4151095adf098b8f53eab0c03b37169e8559"
TYPE1 = bytes.fromhex("017f000102030405060708090a0b0c0d0e0f00a5")
TYPE2 = bytes.fromhex(
    "417f101112131415161718191a1b1c1d1e1f000102030405060708090a0b0c0d0e0f00a5"
)
TOKEN_KEY = bytes(range(64))
TOKEN_IV = bytes(range(0xA0, 0xB0))
TOKEN_PLAINTEXT = b"RNS 1.5.1 token"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reticulum-checkout", required=True, type=pathlib.Path)
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path("tests/interop/fixtures/rns/rns-1.5.1"),
    )
    args = parser.parse_args()
    revision = subprocess.run(
        ["git", "-C", str(args.reticulum_checkout), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != REVISION:
        raise SystemExit(f"expected Reticulum {REVISION}, got {revision}")

    sys.path.insert(0, str(args.reticulum_checkout))
    import RNS  # type: ignore[import-not-found]  # noqa: PLC0415
    token_module = importlib.import_module("RNS.Cryptography.Token")

    for raw in (TYPE1, TYPE2):
        packet = RNS.Packet(None, raw)
        if packet.unpack() is not True:
            raise SystemExit("canonical RNS rejected a seed packet")

    mutations = [
        ("type1-incomplete-destination", TYPE1[:17], "malformed_packet"),
        ("type1-zero-data", TYPE1[:19], "empty_data"),
        ("hop-128", TYPE1[:1] + bytes([128]) + TYPE1[2:], "excessive_hops"),
        ("hop-255", TYPE1[:1] + bytes([255]) + TYPE1[2:], "excessive_hops"),
        ("type2-incomplete-transport", TYPE2[:17], "malformed_packet"),
        ("type2-incomplete-destination", TYPE2[:34], "malformed_packet"),
        ("type2-zero-data", TYPE2[:35], "empty_data"),
    ]
    admission = []
    for case_id, raw, rejection in mutations:
        if RNS.Packet(None, raw).unpack() is not False:
            raise SystemExit(f"canonical RNS unexpectedly accepted {case_id}")
        admission.append(
            {"id": case_id, "raw_hex": raw.hex(), "accepted": False, "class": rejection}
        )

    original_urandom = token_module.os.urandom
    token_module.os.urandom = lambda length: TOKEN_IV if length == len(TOKEN_IV) else original_urandom(length)
    try:
        token = token_module.Token(TOKEN_KEY)
        encrypted = token.encrypt(TOKEN_PLAINTEXT)
        if token.decrypt(encrypted) != TOKEN_PLAINTEXT:
            raise SystemExit("canonical RNS token round trip failed")
    finally:
        token_module.os.urandom = original_urandom

    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "packet-type1-hop127.bin").write_bytes(TYPE1)
    (args.output / "packet-type2-hop127.bin").write_bytes(TYPE2)
    (args.output / "token-valid.bin").write_bytes(encrypted)
    (args.output / "packet-admission.json").write_text(
        json.dumps(admission, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
