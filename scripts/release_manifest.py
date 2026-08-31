#!/usr/bin/env python3
"""Generate canonical release manifests for styrene-rs artifacts.

Adapted from omegon's release_manifest.py for multi-binary archives.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
)

# Binaries included per target
BINARIES = {
    "aarch64-apple-darwin": ["styrene", "styrened", "styrene-tui"],
    "x86_64-apple-darwin": ["styrene", "styrened", "styrene-tui"],
    "aarch64-unknown-linux-gnu": ["styrene", "styrened", "styrene-tui"],
    "x86_64-unknown-linux-gnu": ["styrene", "styrened", "styrene-tui"],
}


def infer_channel(tag: str) -> str:
    if "-nightly." in tag:
        return "nightly"
    if "-rc." in tag:
        return "rc"
    return "stable"


def parse_checksums(checksums_path: Path) -> dict[str, dict[str, str]]:
    assets: dict[str, dict[str, str]] = {}
    for raw_line in checksums_path.read_text().splitlines():
        line = raw_line.strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 2:
            raise ValueError(f"Malformed checksum line: {raw_line!r}")
        sha256, filename = parts[0], parts[-1]
        archive_name = Path(filename).name
        target = next(
            (t for t in TARGETS if archive_name.endswith(f"-{t}.tar.gz")),
            None,
        )
        if target is None:
            continue
        assets[target] = {
            "target": target,
            "filename": archive_name,
            "sha256": sha256,
            "binaries": BINARIES.get(target, []),
        }
    missing = [t for t in TARGETS if t not in assets]
    if missing:
        print(f"Note: checksums not yet available for: {', '.join(missing)}", file=sys.stderr)
    return assets


def build_manifest(
    *,
    tag: str,
    checksums_path: Path,
    repo: str,
    commit: str,
) -> dict[str, Any]:
    version = tag.removeprefix("v")  # support both "v0.2.0" and "0.2.0" tags
    channel = infer_channel(tag)
    assets = parse_checksums(checksums_path)
    release_base = f"https://github.com/{repo}/releases/download/{tag}"

    manifest_assets = []
    for target in TARGETS:
        if target not in assets:
            continue
        asset = assets[target]
        filename = asset["filename"]
        manifest_assets.append(
            {
                **asset,
                "url": f"{release_base}/{filename}",
                "signature_url": f"{release_base}/{filename}.sig",
                "certificate_url": f"{release_base}/{filename}.pem",
            }
        )

    return {
        "version": version,
        "tag": tag,
        "channel": channel,
        "commit": commit,
        "release_url": f"https://github.com/{repo}/releases/tag/{tag}",
        "checksums_url": f"{release_base}/checksums.sha256",
        "assets": manifest_assets,
    }


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=False) + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    generate = subparsers.add_parser("generate", help="Generate release-manifest.json")
    generate.add_argument("--tag", required=True)
    generate.add_argument("--checksums", type=Path, required=True)
    generate.add_argument("--output", type=Path, required=True)
    generate.add_argument("--repo", required=True)
    generate.add_argument("--commit", required=True)

    args = parser.parse_args(argv)

    try:
        manifest = build_manifest(
            tag=args.tag,
            checksums_path=args.checksums,
            repo=args.repo,
            commit=args.commit,
        )
        write_json(args.output, manifest)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
