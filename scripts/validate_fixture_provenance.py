#!/usr/bin/env python3
"""Validate pinned fixture provenance and artifact digests."""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import re
import sys
import tomllib

COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
VALID_PROVENANCE = {"pinned-upstream", "legacy-unrecorded", "independent-reimplementation"}


def validate(path: pathlib.Path) -> list[str]:
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)
    errors: list[str] = []
    if manifest.get("schema_version") != 1:
        errors.append(f"{path}: unsupported schema_version")

    upstreams: set[str] = set()
    for entry in manifest.get("upstreams", []):
        upstream_id = entry.get("id")
        if not upstream_id or upstream_id in upstreams:
            errors.append(f"{path}: missing or duplicate upstream id {upstream_id!r}")
            continue
        if not COMMIT_RE.fullmatch(entry.get("revision", "")):
            errors.append(f"{path}: {upstream_id}: revision must be a full commit SHA")
        upstreams.add(upstream_id)

    fixture_sets: set[str] = set()
    for fixture_set in manifest.get("fixture_sets", []):
        fixture_id = fixture_set.get("id")
        if not fixture_id or fixture_id in fixture_sets:
            errors.append(f"{path}: missing or duplicate fixture set id {fixture_id!r}")
            continue
        fixture_sets.add(fixture_id)
        if fixture_set.get("reference_upstream") not in upstreams:
            errors.append(f"{path}: {fixture_id}: unknown reference upstream")
        if fixture_set.get("provenance") not in VALID_PROVENANCE:
            errors.append(f"{path}: {fixture_id}: invalid provenance")
        generator = fixture_set.get("generator")
        if generator != "manual-copy" and not pathlib.Path(generator or "").is_file():
            errors.append(f"{path}: {fixture_id}: generator does not exist: {generator}")
        artifacts = fixture_set.get("artifacts", [])
        if not artifacts:
            errors.append(f"{path}: {fixture_id}: no artifacts")
        for artifact in artifacts:
            artifact_path = pathlib.Path(artifact.get("path", ""))
            if not artifact_path.is_file():
                errors.append(f"{path}: {fixture_id}: artifact does not exist: {artifact_path}")
                continue
            actual = hashlib.sha256(artifact_path.read_bytes()).hexdigest()
            expected = artifact.get("sha256")
            if actual != expected:
                errors.append(
                    f"{path}: {fixture_id}: digest mismatch for {artifact_path}: {actual} != {expected}"
                )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "manifest",
        nargs="?",
        type=pathlib.Path,
        default=pathlib.Path("tests/interop/fixtures/provenance-v1.toml"),
    )
    args = parser.parse_args()
    errors = validate(args.manifest)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"validated fixture provenance: {args.manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
