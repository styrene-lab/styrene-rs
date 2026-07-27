#!/usr/bin/env python3
"""Validate Styrene product capability manifests using only the standard library."""

from __future__ import annotations

import argparse
import pathlib
import sys
import tomllib

STATUS_SECTIONS = ("required", "experimental", "planned", "excluded")
VALID_STATUSES = {"implemented", "experimental", "planned"}


def load(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def validate(registry_path: pathlib.Path, manifest_paths: list[pathlib.Path]) -> list[str]:
    errors: list[str] = []
    registry = load(registry_path)
    if registry.get("schema_version") != 1:
        errors.append(f"{registry_path}: unsupported schema_version")

    capabilities = {}
    for entry in registry.get("capabilities", []):
        capability_id = entry.get("id")
        if not capability_id or capability_id in capabilities:
            errors.append(f"{registry_path}: missing or duplicate capability id {capability_id!r}")
            continue
        if entry.get("status") not in VALID_STATUSES:
            errors.append(f"{registry_path}: {capability_id}: invalid status {entry.get('status')!r}")
        for evidence in entry.get("evidence", []):
            if not pathlib.Path(evidence).exists():
                errors.append(f"{registry_path}: {capability_id}: evidence does not exist: {evidence}")
        capabilities[capability_id] = entry

    tiers = {entry.get("id"): entry for entry in registry.get("tiers", [])}
    if None in tiers or len(tiers) != len(registry.get("tiers", [])):
        errors.append(f"{registry_path}: missing or duplicate tier id")

    for manifest_path in manifest_paths:
        manifest = load(manifest_path)
        if manifest.get("schema_version") != 1:
            errors.append(f"{manifest_path}: unsupported schema_version")
        tier_id = manifest.get("deployment_tier")
        tier = tiers.get(tier_id)
        if tier is None:
            errors.append(f"{manifest_path}: unknown deployment tier {tier_id!r}")
            continue

        seen: dict[str, str] = {}
        for section in STATUS_SECTIONS:
            for capability_id in manifest.get(section, []):
                if capability_id not in capabilities:
                    errors.append(f"{manifest_path}: {section}: unknown capability {capability_id}")
                    continue
                if capability_id in seen:
                    errors.append(
                        f"{manifest_path}: {capability_id} appears in both {seen[capability_id]} and {section}"
                    )
                seen[capability_id] = section
                actual = capabilities[capability_id]["status"]
                if section == "required" and actual != "implemented":
                    errors.append(
                        f"{manifest_path}: required capability {capability_id} is {actual}, not implemented"
                    )
                if section == "experimental" and actual != "experimental":
                    errors.append(
                        f"{manifest_path}: experimental capability {capability_id} is classified {actual}"
                    )
                if section == "planned" and actual != "planned":
                    errors.append(
                        f"{manifest_path}: planned capability {capability_id} is classified {actual}"
                    )

        requirements = manifest.get("requirements", {})
        for key in ("process_model", "allocator", "interactive_display"):
            if requirements.get(key) != tier.get(key):
                errors.append(
                    f"{manifest_path}: requirement {key}={requirements.get(key)!r} "
                    f"does not match tier {tier_id} value {tier.get(key)!r}"
                )
        for key in ("min_memory_bytes", "min_storage_bytes"):
            if key in tier and requirements.get(key) != tier.get(key):
                errors.append(
                    f"{manifest_path}: requirement {key}={requirements.get(key)!r} "
                    f"does not match tier {tier_id} value {tier.get(key)!r}"
                )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=pathlib.Path, default=pathlib.Path("product/capabilities-v1.toml"))
    parser.add_argument("manifests", nargs="*", type=pathlib.Path)
    args = parser.parse_args()
    manifests = args.manifests or sorted(pathlib.Path("product/manifests").glob("*.toml"))
    errors = validate(args.registry, manifests)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"validated {len(manifests)} manifest(s) against {args.registry}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
