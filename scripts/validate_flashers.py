#!/usr/bin/env python3
"""Validate declarative Styrene flasher contracts."""
from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

REQUIRED_PHASES = ("materialization", "artifact_validation", "delivery", "first_boot")
BUILDER_BOOT_CHECKS = {
    "ssh-public-key",
    "aarch64-linux",
    "nix-daemon-active",
    "nix-sandbox-enabled",
    "native-derivation-build",
}
BRINGUP_BOOT_CHECKS = {
    "three-cold-boots",
    "nixos-stage2",
    "display",
    "evdev-controls",
    "controlled-shutdown",
    "filesystem-clean",
}
VALID_STATUSES = {
    "planned",
    "materializable",
    "artifact-validated",
    "delivery-approved",
    "hardware-validated",
}


def validate(path: Path, root: Path) -> None:
    data = tomllib.loads(path.read_text())
    errors: list[str] = []

    if data.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    status = data.get("status")
    if status not in VALID_STATUSES:
        errors.append(f"status must be one of: {', '.join(sorted(VALID_STATUSES))}")
    if not data.get("id") or not data.get("hardware_profile"):
        errors.append("id and hardware_profile are required")
    for phase in REQUIRED_PHASES:
        if not isinstance(data.get(phase), dict):
            errors.append(f"missing [{phase}] table")

    materialization = data.get("materialization", {})
    if not materialization.get("flake_attribute"):
        errors.append("materialization.flake_attribute is required")
    if materialization.get("builder_system") != "aarch64-linux":
        errors.append("first flasher builder_system must be aarch64-linux")

    validation = data.get("artifact_validation", {})
    validation_command = validation.get("command")
    if not isinstance(validation_command, str) or not (root / validation_command).is_file():
        errors.append("artifact_validation.command must name an existing repository file")
    contract_validator = materialization.get("contract_validator")
    if not isinstance(contract_validator, str) or not (root / contract_validator).is_file():
        errors.append("materialization.contract_validator must name an existing repository file")

    delivery = data.get("delivery", {})
    delivery_enabled = delivery.get("enabled", True)
    if status == "planned" and delivery_enabled is not False:
        errors.append("planned target must disable delivery")
    delivery_command = delivery.get("command")
    if not isinstance(delivery_command, str) or not (root / delivery_command).is_file():
        errors.append("delivery.command must name an existing repository file")
    if delivery.get("build_is_non_destructive") is not True:
        errors.append("delivery.build_is_non_destructive must be true")
    if delivery.get("whole_disk_only") is not True or delivery.get("removable_only") is not True:
        errors.append("delivery must require a whole removable disk")
    if delivery.get("confirmation") != "ERASE":
        errors.append("delivery.confirmation must be ERASE")

    first_boot = data.get("first_boot", {})
    acceptance_command = first_boot.get("acceptance_command")
    if not isinstance(acceptance_command, str) or not (root / acceptance_command).is_file():
        errors.append("first_boot.acceptance_command must name an existing repository file")
    checks = set(first_boot.get("required_checks", []))
    required_checks = BUILDER_BOOT_CHECKS if data.get("purpose") == "native-aarch64-builder" else BRINGUP_BOOT_CHECKS
    missing_checks = sorted(required_checks - checks)
    if missing_checks:
        errors.append(f"first_boot.required_checks missing: {', '.join(missing_checks)}")
    if first_boot.get("password_authentication") is not False:
        errors.append("first_boot.password_authentication must be false")

    evidence = data.get("evidence", {}).get("reference_run", {})
    if status == "hardware-validated":
        for field in ("date", "kernel", "nix", "native_derivation", "native_output"):
            if not evidence.get(field):
                errors.append(f"hardware-validated target requires evidence.reference_run.{field}")

    if errors:
        raise ValueError("\n".join(f"{path}: {error}" for error in errors))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        for path in args.paths:
            validate(path, args.root)
    except (OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(error)
        return 1
    print(f"validated {len(args.paths)} flasher contract(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
