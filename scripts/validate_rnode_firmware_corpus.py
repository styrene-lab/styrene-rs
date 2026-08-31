#!/usr/bin/env python3
"""Validate the corpus-first RNode firmware provisioning contract."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_DIRECTORY = ROOT / "tests" / "fixtures" / "rnode-firmware-provisioning-v1"
EXPECTED_CORPUS = {
    "capabilities.json": "styrene-rnode-firmware-capabilities-v1",
    "artifacts.json": "styrene-rnode-firmware-artifacts-v1",
    "workflows.json": "styrene-rnode-firmware-workflows-v1",
}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
FORBIDDEN_KEYS = {"serial_number", "usb_serial", "peripheral_id", "device_path", "secret"}

OPERATIONS = {"inspect", "plan", "upgrade", "fresh_install", "provision", "recovery"}
DECISIONS = {"allow", "deny"}
CAPABILITY_REASONS = {
    "accepted_exact_target",
    "read_only_inspection",
    "exact_target_unknown",
    "executor_mismatch",
    "mobile_executor_unavailable",
    "operation_not_mobile_supported",
    "physical_evidence_missing",
}
ARTIFACT_REASONS = {
    "artifact_admitted",
    "manifest_signature_required",
    "manifest_signature_invalid",
    "manifest_invalid",
    "archive_digest_mismatch",
    "target_mismatch",
    "unsafe_archive",
    "unsafe_layout",
    "application_digest_required",
}
TERMINALS = {
    "planned",
    "rejected",
    "cancelled",
    "failed",
    "verification_failed",
    "succeeded",
    "stale_event_rejected",
    "provisioning_incomplete",
}


def _walk_forbidden(value: Any, prefix: str, errors: list[str]) -> None:
    if isinstance(value, dict):
        for key, nested in value.items():
            if key in FORBIDDEN_KEYS:
                errors.append(f"{prefix}: forbidden stable identity or secret field {key}")
            _walk_forbidden(nested, prefix, errors)
    elif isinstance(value, list):
        for nested in value:
            _walk_forbidden(nested, prefix, errors)


def _validate_cases(document: dict[str, Any], prefix: str, errors: list[str]) -> list[dict[str, Any]]:
    cases = document.get("cases")
    if not isinstance(cases, list) or not cases:
        errors.append(f"{prefix}: cases must be a non-empty array")
        return []
    seen: set[str] = set()
    valid: list[dict[str, Any]] = []
    for case in cases:
        if not isinstance(case, dict):
            errors.append(f"{prefix}: case must be an object")
            continue
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id:
            errors.append(f"{prefix}: case id must be non-empty")
            continue
        if case_id in seen:
            errors.append(f"{prefix}: duplicate case id {case_id}")
            continue
        seen.add(case_id)
        valid.append(case)
    return valid


def _validate_expected(
    case: dict[str, Any], reasons: set[str], prefix: str, errors: list[str]
) -> tuple[str | None, str | None]:
    expected = case.get("expected")
    if not isinstance(expected, dict):
        errors.append(f"{prefix}: expected must be an object")
        return None, None
    decision = expected.get("decision")
    reason = expected.get("reason")
    if decision not in DECISIONS:
        errors.append(f"{prefix}: invalid decision {decision!r}")
    if reason not in reasons:
        errors.append(f"{prefix}: invalid reason {reason!r}")
    return decision, reason


def _validate_capabilities(document: dict[str, Any], path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    prefix = str(path)
    if set(document.get("operations", [])) != OPERATIONS:
        errors.append(f"{prefix}: operations must declare the complete operation set")
    targets = document.get("targets")
    if not isinstance(targets, list) or not targets:
        errors.append(f"{prefix}: targets must be a non-empty array")
        return errors
    target_by_id: dict[str, dict[str, Any]] = {}
    for target in targets:
        if not isinstance(target, dict):
            errors.append(f"{prefix}: target must be an object")
            continue
        target_id = target.get("id")
        if not isinstance(target_id, str) or not target_id or target_id in target_by_id:
            errors.append(f"{prefix}: missing or duplicate target id {target_id!r}")
            continue
        for field in ("mcu_family", "bootloader", "configured", "physical_acceptance"):
            if field not in target:
                errors.append(f"{prefix}: {target_id}: missing {field}")
        target_by_id[target_id] = target

    for case in _validate_cases(document, prefix, errors):
        case_id = case["id"]
        case_prefix = f"{prefix}: {case_id}"
        host = case.get("host")
        operation = case.get("operation")
        target = target_by_id.get(case.get("target"))
        decision, _ = _validate_expected(case, CAPABILITY_REASONS, case_prefix, errors)
        if host not in {"desktop", "ios_mobile"}:
            errors.append(f"{case_prefix}: invalid host {host!r}")
        if operation not in OPERATIONS:
            errors.append(f"{case_prefix}: invalid operation {operation!r}")
        if target is None:
            errors.append(f"{case_prefix}: unknown target")
            continue
        if host == "ios_mobile" and decision == "allow" and operation != "inspect":
            if operation != "upgrade" or case.get("executor") != "ios_nrf_ble_dfu":
                errors.append(f"{case_prefix}: mobile write capability must be nRF BLE upgrade only")
            if target.get("mcu_family") != "nrf52840" or not target.get("physical_acceptance"):
                errors.append(f"{case_prefix}: mobile upgrade requires accepted nRF52840 evidence")
            if target.get("configured") != "yes":
                errors.append(f"{case_prefix}: mobile upgrade requires a configured RNode")
    return errors


def _validate_artifacts(document: dict[str, Any], path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    prefix = str(path)
    if not SHA256_RE.fullmatch(document.get("synthetic_digest", "")):
        errors.append(f"{prefix}: synthetic_digest must be 64 lowercase hex characters")
    authorities = document.get("authorities")
    if not isinstance(authorities, list) or not authorities:
        errors.append(f"{prefix}: authorities must be a non-empty array")
    else:
        authority_ids: set[str] = set()
        for authority in authorities:
            authority_id = authority.get("id") if isinstance(authority, dict) else None
            if not authority_id or authority_id in authority_ids:
                errors.append(f"{prefix}: missing or duplicate authority id {authority_id!r}")
                continue
            authority_ids.add(authority_id)
            if not COMMIT_RE.fullmatch(authority.get("revision", "")):
                errors.append(f"{prefix}: {authority_id}: revision must be a full commit SHA")

    for case in _validate_cases(document, prefix, errors):
        case_prefix = f"{prefix}: {case['id']}"
        decision, _ = _validate_expected(case, ARTIFACT_REASONS, case_prefix, errors)
        signature = case.get("manifest_signature")
        digest = case.get("archive_digest")
        target = case.get("target_match")
        archive_findings = case.get("archive_findings")
        layout_findings = case.get("layout_findings")
        if signature not in {"valid", "invalid", "absent"}:
            errors.append(f"{case_prefix}: invalid manifest_signature")
        if digest not in {"match", "mismatch"}:
            errors.append(f"{case_prefix}: invalid archive_digest")
        if target not in {"exact", "model_mismatch", "radio_mismatch"}:
            errors.append(f"{case_prefix}: invalid target_match")
        if not isinstance(archive_findings, list) or not isinstance(layout_findings, list):
            errors.append(f"{case_prefix}: findings must be arrays")
            continue
        safe = signature == "valid" and digest == "match" and target == "exact"
        safe = safe and not archive_findings and not layout_findings
        if decision == "allow" and not safe:
            errors.append(f"{case_prefix}: unsafe artifact cannot be allowed")
        if decision == "deny" and safe:
            errors.append(f"{case_prefix}: safe artifact denial lacks a modeled cause")
    return errors


def _validate_workflows(document: dict[str, Any], path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    prefix = str(path)
    for case in _validate_cases(document, prefix, errors):
        case_prefix = f"{prefix}: {case['id']}"
        operation = case.get("operation")
        actions = case.get("actions")
        destructive = case.get("destructive_started")
        verified = case.get("post_verified")
        expected = case.get("expected")
        if operation not in OPERATIONS:
            errors.append(f"{case_prefix}: invalid operation {operation!r}")
        if not isinstance(actions, list) or not actions or any(not isinstance(v, str) for v in actions):
            errors.append(f"{case_prefix}: actions must be a non-empty string array")
            actions = []
        if not isinstance(destructive, bool) or not isinstance(verified, bool):
            errors.append(f"{case_prefix}: destructive_started and post_verified must be booleans")
            continue
        if not isinstance(expected, dict):
            errors.append(f"{case_prefix}: expected must be an object")
            continue
        terminal = expected.get("terminal")
        recovery = expected.get("recovery_required")
        if terminal not in TERMINALS:
            errors.append(f"{case_prefix}: invalid terminal {terminal!r}")
        if not isinstance(recovery, bool):
            errors.append(f"{case_prefix}: recovery_required must be boolean")
            continue
        if destructive and "begin_write" not in actions:
            errors.append(f"{case_prefix}: destructive workflow must include begin_write")
        if destructive and terminal != "succeeded" and not recovery:
            errors.append(f"{case_prefix}: destructive failure must require recovery")
        if terminal == "succeeded" and (not destructive or not verified or recovery):
            errors.append(f"{case_prefix}: success requires destructive write and post-verification")
        if verified and "verify_model_version_hash" not in actions:
            errors.append(f"{case_prefix}: post-verification action is missing")
        if case.get("host") == "ios_mobile" and case.get("executor") != "ios_nrf_ble_dfu":
            errors.append(f"{case_prefix}: mobile workflow must use the BLE DFU executor")
    return errors


def validate_document(path: pathlib.Path, document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    expected_corpus = EXPECTED_CORPUS.get(path.name)
    if expected_corpus is None:
        return [f"{path}: unexpected corpus file"]
    if document.get("schema_version") != 1:
        errors.append(f"{path}: unsupported schema_version")
    if document.get("corpus") != expected_corpus:
        errors.append(f"{path}: unexpected corpus identifier")
    if document.get("evidence_scope") != "synthetic_contract":
        errors.append(f"{path}: evidence_scope must be synthetic_contract")
    _walk_forbidden(document, str(path), errors)
    if path.name == "capabilities.json":
        errors.extend(_validate_capabilities(document, path))
    elif path.name == "artifacts.json":
        errors.extend(_validate_artifacts(document, path))
    else:
        errors.extend(_validate_workflows(document, path))
    return errors


def validate(directory: pathlib.Path = DEFAULT_DIRECTORY) -> list[str]:
    errors: list[str] = []
    for name in EXPECTED_CORPUS:
        path = directory / name
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{path}: invalid corpus: {error}")
            continue
        if not isinstance(document, dict):
            errors.append(f"{path}: corpus root must be an object")
            continue
        errors.extend(validate_document(path, document))
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", nargs="?", type=pathlib.Path, default=DEFAULT_DIRECTORY)
    args = parser.parse_args()
    errors = validate(args.directory)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"validated RNode firmware corpus: {args.directory}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
