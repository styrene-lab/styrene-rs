#!/usr/bin/env python3
"""Validate selected RG35XXSP boot-chain provenance and promotion state."""
from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

BOOT_COMPONENTS = ("u-boot", "trusted-firmware-a", "linux", "device-tree")
IMMUTABLE_FIELDS = ("repository", "revision", "license")


def validate(path: Path, require_build_ready: bool = False) -> None:
    data = tomllib.loads(path.read_text())
    errors: list[str] = []
    components = {entry["name"]: entry for entry in data.get("components", [])}
    for name in BOOT_COMPONENTS:
        entry = components.get(name)
        if entry is None:
            errors.append(f"missing component: {name}")
            continue
        if entry.get("status") not in {"selected", "pinned", "validated"}:
            errors.append(f"{name}: source is not selected")
        for field in IMMUTABLE_FIELDS:
            if not entry.get(field):
                errors.append(f"{name}: missing {field}")
        revision = str(entry.get("revision", ""))
        if len(revision) != 40 or any(char not in "0123456789abcdef" for char in revision):
            errors.append(f"{name}: revision must be a full lowercase Git SHA")
        if not entry.get("required_evidence"):
            errors.append(f"{name}: required_evidence is empty")
    firmware = components.get("firmware", {})
    if firmware.get("status") == "selected" and not firmware.get("revision"):
        errors.append("firmware: selected source requires revision")
    if require_build_ready:
        unresolved = [
            entry["name"] for entry in components.values()
            if entry.get("status") not in {"selected", "pinned", "validated"}
        ]
        if unresolved:
            errors.append("build-ready provenance has unresolved components: " + ", ".join(unresolved))
    if errors:
        raise ValueError("\n".join(errors))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("path", nargs="?", type=Path, default=Path("nix/hardware/rg35xxsp/provenance.toml"))
    parser.add_argument("--require-build-ready", action="store_true")
    args = parser.parse_args()
    try:
        validate(args.path, args.require_build_ready)
    except (OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(error)
        return 1
    print(f"rg35xxsp_provenance=pass path={args.path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
