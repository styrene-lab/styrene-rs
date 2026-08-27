#!/usr/bin/env python3
"""Validate Styrene product capability manifests using only the standard library."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tomllib

STATUS_SECTIONS = ("required", "experimental", "planned", "excluded")
VALID_STATUSES = {"implemented", "experimental", "planned"}
VALID_PARITY_LEVELS = {"unsupported", "experimental", "verified", "degraded"}
VALID_GATE_KINDS = {"fixture", "rust", "live", "manual"}
VALID_GATE_PROTOCOLS = {"native", "internal", "styrene-specific"}
REQUIRED_PARITY_CLAIMS = {
    "rns.primitives",
    "rns.operations",
    "lxmf.codec",
    "lxmf.direct",
    "lxmf.resources",
    "lxmf.propagation",
    "micron.rendering",
    "nomadnet.transport",
}
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")


def load(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def validate_parity_registry(registry: dict, registry_path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    if registry.get("parity_schema_version") != 1:
        return [f"{registry_path}: unsupported parity_schema_version"]

    upstreams = {}
    for entry in registry.get("parity_upstreams", []):
        upstream_id = entry.get("id")
        if not upstream_id or upstream_id in upstreams:
            errors.append(f"{registry_path}: missing or duplicate parity upstream id {upstream_id!r}")
            continue
        revision = entry.get("revision", "")
        if not COMMIT_RE.fullmatch(revision):
            errors.append(f"{registry_path}: {upstream_id}: revision must be a full commit SHA")
        if not entry.get("repository") or not entry.get("version"):
            errors.append(f"{registry_path}: {upstream_id}: repository and version are required")
        upstreams[upstream_id] = entry

    gates = {}
    for entry in registry.get("parity_gates", []):
        gate_id = entry.get("id")
        if not gate_id or gate_id in gates:
            errors.append(f"{registry_path}: missing or duplicate parity gate id {gate_id!r}")
            continue
        if entry.get("kind") not in VALID_GATE_KINDS:
            errors.append(f"{registry_path}: {gate_id}: invalid gate kind {entry.get('kind')!r}")
        if entry.get("protocol") not in VALID_GATE_PROTOCOLS:
            errors.append(f"{registry_path}: {gate_id}: invalid protocol {entry.get('protocol')!r}")
        for key in ("automated", "enabled", "ignored"):
            if not isinstance(entry.get(key), bool):
                errors.append(f"{registry_path}: {gate_id}: {key} must be boolean")
        for upstream_id in entry.get("upstreams", []):
            if upstream_id not in upstreams:
                errors.append(f"{registry_path}: {gate_id}: unknown upstream {upstream_id}")
        for evidence in entry.get("evidence", []):
            if not pathlib.Path(evidence).exists():
                errors.append(f"{registry_path}: {gate_id}: evidence does not exist: {evidence}")
        gates[gate_id] = entry

    claims = {}
    for entry in registry.get("parity_claims", []):
        claim_id = entry.get("id")
        if not claim_id or claim_id in claims:
            errors.append(f"{registry_path}: missing or duplicate parity claim id {claim_id!r}")
            continue
        level = entry.get("level")
        if level not in VALID_PARITY_LEVELS:
            errors.append(f"{registry_path}: {claim_id}: invalid parity level {level!r}")
        if level in {"unsupported", "degraded"} and not entry.get("reason"):
            errors.append(f"{registry_path}: {claim_id}: {level} claim requires a reason")

        required_gates = entry.get("required_gates", [])
        evidence_gates = entry.get("evidence_gates", [])
        for gate_id in [*required_gates, *evidence_gates]:
            if gate_id not in gates:
                errors.append(f"{registry_path}: {claim_id}: unknown gate {gate_id}")
        for gate_id in required_gates:
            gate = gates.get(gate_id)
            if gate is not None and gate.get("protocol") == "styrene-specific":
                errors.append(
                    f"{registry_path}: {claim_id}: Styrene-specific gate {gate_id} cannot be required for native parity"
                )

        if level == "verified":
            if not required_gates:
                errors.append(f"{registry_path}: {claim_id}: verified claim requires at least one gate")
            for gate_id in required_gates:
                gate = gates.get(gate_id)
                if gate is None:
                    continue
                if not gate.get("automated"):
                    errors.append(f"{registry_path}: {claim_id}: required gate {gate_id} is manual")
                if not gate.get("enabled"):
                    errors.append(f"{registry_path}: {claim_id}: required gate {gate_id} is disabled")
                if gate.get("ignored"):
                    errors.append(f"{registry_path}: {claim_id}: required gate {gate_id} is ignored")
                if gate.get("protocol") != "native":
                    errors.append(
                        f"{registry_path}: {claim_id}: required gate {gate_id} is not native protocol evidence"
                    )
                if not gate.get("upstreams"):
                    errors.append(
                        f"{registry_path}: {claim_id}: required gate {gate_id} has no pinned upstream"
                    )
                if not gate.get("command"):
                    errors.append(f"{registry_path}: {claim_id}: required gate {gate_id} has no command")
        claims[claim_id] = entry

    missing_claims = sorted(REQUIRED_PARITY_CLAIMS - claims.keys())
    extra_claims = sorted(claims.keys() - REQUIRED_PARITY_CLAIMS)
    if missing_claims:
        errors.append(f"{registry_path}: missing parity claims: {', '.join(missing_claims)}")
    if extra_claims:
        errors.append(f"{registry_path}: unknown parity claims: {', '.join(extra_claims)}")
    return errors


def validate(registry_path: pathlib.Path, manifest_paths: list[pathlib.Path]) -> list[str]:
    errors: list[str] = []
    registry = load(registry_path)
    if registry.get("schema_version") != 1:
        errors.append(f"{registry_path}: unsupported schema_version")
    errors.extend(validate_parity_registry(registry, registry_path))

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
