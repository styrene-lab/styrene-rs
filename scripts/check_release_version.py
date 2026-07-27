#!/usr/bin/env python3
"""Fail when a release tag and product Cargo versions disagree."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

PRODUCT_MANIFESTS = (
    Path("crates/apps/styrene/Cargo.toml"),
    Path("crates/apps/styrened/Cargo.toml"),
    Path("crates/apps/styrene-tui/Cargo.toml"),
)


def package_version(path: Path) -> str:
    text = path.read_text()
    package = text.split("[package]", 1)[1].split("\n[", 1)[0]
    match = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE)
    if match is None:
        raise ValueError(f"{path}: [package] has no explicit version")
    return match.group(1)


def normalized_tag(tag: str) -> str:
    value = tag.removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", value):
        raise ValueError(f"invalid release tag: {tag!r}")
    return value


def validate(tag: str, root: Path) -> list[str]:
    expected = normalized_tag(tag)
    errors = []
    for relative in PRODUCT_MANIFESTS:
        actual = package_version(root / relative)
        if actual != expected:
            errors.append(f"{relative}: package version {actual} != release tag {expected}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("tag", help="release tag, with optional v prefix")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    try:
        errors = validate(args.tag, args.root)
    except (OSError, ValueError) as error:
        print(f"release version validation failed: {error}", file=sys.stderr)
        return 2
    if errors:
        print("release version validation failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print(f"release tag {normalized_tag(args.tag)} matches all product packages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
