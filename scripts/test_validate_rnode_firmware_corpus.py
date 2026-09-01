#!/usr/bin/env python3
"""Mutation tests for the RNode firmware provisioning corpus validator."""

from __future__ import annotations

import copy
import json
import pathlib
import unittest

from validate_rnode_firmware_corpus import DEFAULT_DIRECTORY, validate, validate_document


class RNodeFirmwareCorpusTests(unittest.TestCase):
    def load(self, name: str) -> dict:
        return json.loads((DEFAULT_DIRECTORY / name).read_text(encoding="utf-8"))

    def errors(self, name: str, document: dict) -> list[str]:
        return validate_document(pathlib.Path(name), document)

    def assert_error_contains(self, name: str, document: dict, text: str) -> None:
        errors = self.errors(name, document)
        self.assertTrue(any(text in error for error in errors), errors)

    def test_reference_corpuses(self) -> None:
        self.assertEqual(validate(), [])

    def test_rejects_duplicate_case_id(self) -> None:
        document = self.load("capabilities.json")
        document["cases"].append(copy.deepcopy(document["cases"][0]))
        self.assert_error_contains("capabilities.json", document, "duplicate case id")

    def test_rejects_stable_device_identity(self) -> None:
        document = self.load("capabilities.json")
        document["targets"][0]["usb_serial"] = "must-not-be-committed"
        self.assert_error_contains("capabilities.json", document, "forbidden stable identity")

    def test_rejects_mobile_esp_upgrade_allowance(self) -> None:
        document = self.load("capabilities.json")
        case = next(case for case in document["cases"] if case["id"] == "capability.ios.esp.upgrade")
        case["executor"] = "ios_nrf_ble_dfu"
        case["expected"] = {"decision": "allow", "reason": "accepted_exact_target"}
        self.assert_error_contains("capabilities.json", document, "accepted nRF52840 evidence")

    def test_rejects_mobile_fresh_install_allowance(self) -> None:
        document = self.load("capabilities.json")
        case = next(
            case for case in document["cases"] if case["id"] == "capability.ios.factory-rak.fresh-install"
        )
        case["expected"] = {"decision": "allow", "reason": "accepted_exact_target"}
        self.assert_error_contains("capabilities.json", document, "nRF BLE upgrade only")

    def test_rejects_mobile_upgrade_allowance_without_bootloader_revision(self) -> None:
        document = self.load("capabilities.json")
        target = next(
            target
            for target in document["targets"]
            if target["id"] == "synthetic-rak4631-accepted"
        )
        target["bootloader_revision"] = None
        self.assert_error_contains(
            "capabilities.json",
            document,
            "exact bootloader revision",
        )

    def test_rejects_mutable_upstream_revision(self) -> None:
        document = self.load("artifacts.json")
        document["authorities"][0]["revision"] = "master"
        self.assert_error_contains("artifacts.json", document, "full commit SHA")

    def test_rejects_unsafe_artifact_allowance(self) -> None:
        document = self.load("artifacts.json")
        case = next(case for case in document["cases"] if case["id"] == "artifact.path-traversal")
        case["expected"] = {"decision": "allow", "reason": "artifact_admitted"}
        self.assert_error_contains("artifacts.json", document, "unsafe artifact cannot be allowed")

    def test_rejects_success_without_post_verification(self) -> None:
        document = self.load("workflows.json")
        case = next(case for case in document["cases"] if case["id"] == "workflow.mobile-upgrade-verified")
        case["post_verified"] = False
        self.assert_error_contains("workflows.json", document, "success requires")

    def test_rejects_destructive_failure_without_recovery(self) -> None:
        document = self.load("workflows.json")
        case = next(
            case for case in document["cases"] if case["id"] == "workflow.ios-disconnect-during-write"
        )
        case["expected"]["recovery_required"] = False
        self.assert_error_contains("workflows.json", document, "destructive failure must require recovery")

    def test_rejects_mobile_non_dfu_executor(self) -> None:
        document = self.load("workflows.json")
        case = next(case for case in document["cases"] if case["id"] == "workflow.cancel-before-write")
        case["executor"] = "host_serial_nrf_dfu"
        self.assert_error_contains("workflows.json", document, "mobile workflow must use")


if __name__ == "__main__":
    unittest.main()
