#!/usr/bin/env python3
"""Generate bounded empty-carrier evidence from canonical RNS 1.5.2."""

from __future__ import annotations

import argparse
import ast
import json
import pathlib
import subprocess

REVISION = "ea98db4f53dcf0defc0e71a16e60d28b1229c4e6"
SYMBOLS = {
    "RNS.Interfaces.I2PInterface.I2PInterfacePeer.process_incoming": "RNS/Interfaces/I2PInterface.py",
    "RNS.Interfaces.KISSInterface.KISSInterface.process_incoming": "RNS/Interfaces/KISSInterface.py",
    "RNS.Interfaces.PipeInterface.PipeInterface.process_incoming": "RNS/Interfaces/PipeInterface.py",
    "RNS.Interfaces.RNodeInterface.RNodeInterface.process_incoming": "RNS/Interfaces/RNodeInterface.py",
    "RNS.Interfaces.RNodeMultiInterface.RNodeSubInterface.process_incoming": "RNS/Interfaces/RNodeMultiInterface.py",
    "RNS.Interfaces.SerialInterface.SerialInterface.process_incoming": "RNS/Interfaces/SerialInterface.py",
    "RNS.Interfaces.TCPInterface.TCPClientInterface.process_incoming": "RNS/Interfaces/TCPInterface.py",
    "RNS.Interfaces.UDPInterface.UDPInterface.process_incoming": "RNS/Interfaces/UDPInterface.py",
}


def has_empty_input_guard(path: pathlib.Path, class_name: str) -> bool:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    class_node = next(
        node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    function = next(
        node
        for node in class_node.body
        if isinstance(node, ast.FunctionDef) and node.name == "process_incoming"
    )
    first = function.body[0]
    return (
        isinstance(first, ast.If)
        and isinstance(first.test, ast.UnaryOp)
        and isinstance(first.test.op, ast.Not)
        and isinstance(first.test.operand, ast.Name)
        and first.test.operand.id == "data"
        and len(first.body) == 1
        and isinstance(first.body[0], ast.Return)
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reticulum-checkout", required=True, type=pathlib.Path)
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path("tests/interop/fixtures/rns/rns-1.5.2"),
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

    cases = []
    for symbol, relative_path in SYMBOLS.items():
        class_name = symbol.rsplit(".", 2)[-2]
        if not has_empty_input_guard(args.reticulum_checkout / relative_path, class_name):
            raise SystemExit(f"missing empty-input guard in {symbol}")
        cases.append(
            {
                "source_symbol": symbol,
                "empty_input": "ignored",
                "inbound_calls": 0,
                "rx_bytes_delta": 0,
            }
        )

    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "empty-carrier-input.json").write_text(
        json.dumps(cases, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
