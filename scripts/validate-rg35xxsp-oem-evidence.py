#!/usr/bin/env python3
"""Validate a local, non-redistributable RG35XXSP OEM evidence bundle."""
from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path

REQUIRED = {
    "first-64MiB.bin": 64 * 1024 * 1024,
    "disk4s2.img": 32 * 1024 * 1024,
    "disk4s3.img": 16 * 1024 * 1024,
    "disk4s4.img": 64 * 1024 * 1024,
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate(root: Path, require_full: bool) -> list[str]:
    errors: list[str] = []
    checksums = root / "checksums.sha256"
    metadata = root / "media-metadata.txt"
    if not checksums.is_file():
        errors.append("missing checksums.sha256")
    if not metadata.is_file():
        errors.append("missing media-metadata.txt")

    expected: dict[str, str] = {}
    if checksums.is_file():
        for line in checksums.read_text().splitlines():
            match = re.fullmatch(r"([0-9a-f]{64})  \./(.+)", line)
            if match:
                expected[match.group(2)] = match.group(1)

    for name, minimum in REQUIRED.items():
        path = root / name
        if not path.is_file():
            errors.append(f"missing {name}")
            continue
        if path.stat().st_size < minimum:
            errors.append(f"{name} is shorter than {minimum} bytes")
        if expected.get(name) != digest(path):
            errors.append(f"checksum mismatch or absent checksum for {name}")

    if metadata.is_file():
        text = metadata.read_text(errors="replace")
        for marker in ("GUID_partition_scheme", "62.5 GB", "disk4s1", "disk4s7"):
            if marker not in text:
                errors.append(f"metadata missing marker: {marker}")

    boot = root / "disk4s4.img"
    if boot.is_file():
        prefix = boot.read_bytes()[:8]
        if prefix != b"ANDROID!":
            errors.append("disk4s4.img lacks Android boot image magic")

    full_images = list(root.glob("full-device*.img"))
    if require_full and not full_images:
        errors.append("complete OEM preservation required: missing full-device*.img")
    for image in full_images:
        sidecar = image.with_suffix(image.suffix + ".sha256")
        if not sidecar.is_file() or digest(image) not in sidecar.read_text():
            errors.append(f"missing or invalid full-image checksum: {sidecar.name}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--require-full", action="store_true")
    args = parser.parse_args()
    errors = validate(args.bundle, args.require_full)
    if errors:
        print("\n".join(errors))
        return 1
    print(f"rg35xxsp_oem_evidence=pass bundle={args.bundle} full={args.require_full}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
