#!/usr/bin/env python3
"""Validate pinned fixture provenance and artifact digests."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
import tomllib

COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
VALID_PROVENANCE = {"pinned-upstream", "legacy-unrecorded", "independent-reimplementation"}
DEFAULT_MANIFESTS = (
    pathlib.Path("tests/interop/fixtures/provenance-v1.toml"),
    pathlib.Path("tests/interop/fixtures/rns/index-v2.json"),
)


def _repository_path(value: object, field: str, errors: list[str], prefix: str) -> pathlib.Path | None:
    if not isinstance(value, str) or not value:
        errors.append(f"{prefix}: {field} must be a repository-relative path")
        return None
    path = pathlib.Path(value)
    if path.is_absolute():
        errors.append(f"{prefix}: {field} must be repository-relative")
        return None
    if ".." in path.parts:
        errors.append(f"{prefix}: {field} escapes repository root")
        return None
    return path


def _validate_v1(path: pathlib.Path, manifest: dict, root: pathlib.Path) -> list[str]:
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
        if generator != "manual-copy" and not (root / pathlib.Path(generator or "")).is_file():
            errors.append(f"{path}: {fixture_id}: generator does not exist: {generator}")
        artifacts = fixture_set.get("artifacts", [])
        if not artifacts:
            errors.append(f"{path}: {fixture_id}: no artifacts")
        for artifact in artifacts:
            artifact_path = root / pathlib.Path(artifact.get("path", ""))
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


def _validate_v2(path: pathlib.Path, index: dict, root: pathlib.Path) -> list[str]:
    errors: list[str] = []
    if index.get("schema_version") != 2:
        errors.append(f"{path}: unsupported schema_version")

    authorities = index.get("authorities")
    if not isinstance(authorities, dict) or not authorities:
        errors.append(f"{path}: authorities must be a non-empty object")
        authorities = {}
    for authority_id, authority in authorities.items():
        prefix = f"{path}: {authority_id}"
        if not isinstance(authority_id, str) or not authority_id or not isinstance(authority, dict):
            errors.append(f"{prefix}: invalid authority record")
            continue
        if not isinstance(authority.get("repository"), str) or not authority["repository"]:
            errors.append(f"{prefix}: repository must be non-empty")
        if not COMMIT_RE.fullmatch(authority.get("revision", "")):
            errors.append(f"{prefix}: revision must be a full commit SHA")
        if not isinstance(authority.get("release"), str) or not authority["release"]:
            errors.append(f"{prefix}: release must be non-empty")

    vectors = index.get("vectors")
    if not isinstance(vectors, list) or not vectors:
        errors.append(f"{path}: vectors must be a non-empty array")
        return errors
    seen: set[str] = set()
    for vector in vectors:
        if not isinstance(vector, dict):
            errors.append(f"{path}: vector must be an object")
            continue
        vector_id = vector.get("id")
        prefix = f"{path}: {vector_id}"
        if not isinstance(vector_id, str) or not vector_id:
            errors.append(f"{path}: vector id must be non-empty")
        elif vector_id in seen:
            errors.append(f"{prefix}: duplicate vector id")
        else:
            seen.add(vector_id)
        if vector.get("authority_id") not in authorities:
            errors.append(f"{prefix}: unknown authority")
        if not isinstance(vector.get("kind"), str) or not vector["kind"]:
            errors.append(f"{prefix}: kind must be non-empty")

        artifact = _repository_path(vector.get("artifact"), "artifact", errors, prefix)
        digest = vector.get("sha256")
        if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
            errors.append(f"{prefix}: sha256 must be 64 lowercase hex characters")
        elif artifact is not None:
            artifact_path = root / artifact
            if not artifact_path.is_file():
                errors.append(f"{prefix}: artifact does not exist: {artifact}")
            elif hashlib.sha256(artifact_path.read_bytes()).hexdigest() != digest:
                errors.append(f"{prefix}: digest mismatch for {artifact}")

        generator = vector.get("generator")
        if generator != "manual-copy":
            generator_path = _repository_path(generator, "generator", errors, prefix)
            if generator_path is not None and not (root / generator_path).is_file():
                errors.append(f"{prefix}: generator does not exist: {generator}")
        symbols = vector.get("source_symbols")
        if not isinstance(symbols, list) or not symbols:
            errors.append(f"{prefix}: source_symbols must be a non-empty array")
        elif any(not isinstance(symbol, str) or not symbol for symbol in symbols):
            errors.append(f"{prefix}: source_symbols must contain non-empty strings")
        elif len(set(symbols)) != len(symbols):
            errors.append(f"{prefix}: source_symbols must be unique")
        expected = vector.get("expected")
        if not isinstance(expected, dict) or not expected:
            errors.append(f"{prefix}: expected must be a non-empty object")
        elif not isinstance(expected.get("type"), str) or not expected["type"]:
            errors.append(f"{prefix}: expected.type must be a non-empty string")
    return errors


def validate(path: pathlib.Path, root: pathlib.Path | None = None) -> list[str]:
    root = pathlib.Path.cwd() if root is None else root
    try:
        if path.suffix == ".toml":
            with path.open("rb") as handle:
                return _validate_v1(path, tomllib.load(handle), root)
        if path.suffix == ".json":
            return _validate_v2(path, json.loads(path.read_text(encoding="utf-8")), root)
        return [f"{path}: unsupported manifest format"]
    except (OSError, tomllib.TOMLDecodeError, json.JSONDecodeError) as error:
        return [f"{path}: invalid fixture provenance: {error}"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifests", nargs="*", type=pathlib.Path)
    args = parser.parse_args()
    manifests = args.manifests or DEFAULT_MANIFESTS
    errors = [error for manifest in manifests for error in validate(manifest)]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    for manifest in manifests:
        print(f"validated fixture provenance: {manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
