#!/usr/bin/env python3
"""Offline validator for generated protocol fixture indexes and digests."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys


FIXTURES = Path(__file__).resolve().parent.parent / "fixtures"
EXPECTED_REVISIONS = {
    "rns": "b48b96e61676504e0a4e527b33b9a0b4495c6872",
    "lxmf": "795fdaa2b0777c13033787d933d1afc94a2377cb",
}


def main() -> int:
    fixture_dir = FIXTURES / "lxmf-propagation-v1"
    index_path = fixture_dir / "index.json"
    errors: list[str] = []
    if not index_path.exists() and (not fixture_dir.exists() or not any(fixture_dir.iterdir())):
        print("no canonical LXMF propagation fixture set has been generated")
        return 0
    try:
        index = json.loads(index_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"{index_path}: {error}", file=sys.stderr)
        return 1

    if index.get("schema_version") != 1:
        errors.append("unsupported LXMF propagation fixture schema")
    for upstream, revision in EXPECTED_REVISIONS.items():
        if index.get("upstreams", {}).get(upstream, {}).get("revision") != revision:
            errors.append(f"unexpected {upstream} revision")
    paths = index.get("request_paths", {})
    if paths.get("offer", {}).get("path") != "/offer" or paths.get("get", {}).get("path") != "/get":
        errors.append("unexpected propagation request paths")

    seen: set[str] = set()
    for artifact in index.get("artifacts", []):
        name = artifact.get("path", "")
        if not name or Path(name).name != name or name in seen:
            errors.append(f"invalid or duplicate artifact path: {name!r}")
            continue
        seen.add(name)
        path = fixture_dir / name
        try:
            data = path.read_bytes()
        except OSError as error:
            errors.append(f"{path}: {error}")
            continue
        digest = hashlib.sha256(data).hexdigest()
        if digest != artifact.get("sha256"):
            errors.append(f"digest mismatch for {path}")
        if len(data) != artifact.get("size"):
            errors.append(f"size mismatch for {path}")

    if not seen:
        errors.append("fixture index has no artifacts")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"validated {len(seen)} offline LXMF propagation fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
