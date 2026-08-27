#!/usr/bin/env python3
"""Tests for product capability and parity-claim validation."""

from __future__ import annotations

import copy
import pathlib
import unittest

from validate_product_capabilities import REQUIRED_PARITY_CLAIMS, validate_parity_registry


def registry() -> dict:
    claims = []
    for claim_id in sorted(REQUIRED_PARITY_CLAIMS):
        claims.append(
            {
                "id": claim_id,
                "level": "experimental",
                "required_gates": ["native"],
                "evidence_gates": ["native"],
                "reason": "not yet verified",
            }
        )
    return {
        "parity_schema_version": 1,
        "parity_upstreams": [
            {
                "id": "upstream",
                "repository": "https://example.invalid/upstream",
                "revision": "1" * 40,
                "version": "1.0.0",
            }
        ],
        "parity_gates": [
            {
                "id": "native",
                "kind": "live",
                "automated": True,
                "enabled": True,
                "ignored": False,
                "protocol": "native",
                "upstreams": ["upstream"],
                "command": "run-native-gate",
                "evidence": [],
            }
        ],
        "parity_claims": claims,
    }


class ParityRegistryTests(unittest.TestCase):
    path = pathlib.Path("registry.toml")

    def assert_error_contains(self, value: dict, text: str) -> None:
        errors = validate_parity_registry(value, self.path)
        self.assertTrue(any(text in error for error in errors), errors)

    def test_valid_registry(self) -> None:
        self.assertEqual(validate_parity_registry(registry(), self.path), [])

    def test_verified_claim_rejects_ignored_gate(self) -> None:
        value = registry()
        value["parity_claims"][0]["level"] = "verified"
        value["parity_gates"][0]["ignored"] = True
        self.assert_error_contains(value, "is ignored")

    def test_verified_claim_rejects_manual_gate(self) -> None:
        value = registry()
        value["parity_claims"][0]["level"] = "verified"
        value["parity_gates"][0]["automated"] = False
        value["parity_gates"][0]["kind"] = "manual"
        self.assert_error_contains(value, "is manual")

    def test_verified_claim_rejects_disabled_gate(self) -> None:
        value = registry()
        value["parity_claims"][0]["level"] = "verified"
        value["parity_gates"][0]["enabled"] = False
        self.assert_error_contains(value, "is disabled")

    def test_unpinned_upstream_is_rejected(self) -> None:
        value = registry()
        value["parity_upstreams"][0]["revision"] = "main"
        self.assert_error_contains(value, "full commit SHA")

    def test_unsupported_claim_requires_reason(self) -> None:
        value = registry()
        value["parity_claims"][0]["level"] = "unsupported"
        value["parity_claims"][0]["reason"] = ""
        self.assert_error_contains(value, "unsupported claim requires a reason")

    def test_styrene_specific_gate_cannot_prove_native_claim(self) -> None:
        value = registry()
        value["parity_gates"][0]["protocol"] = "styrene-specific"
        self.assert_error_contains(value, "cannot be required for native parity")

    def test_missing_required_claim_is_rejected(self) -> None:
        value = registry()
        value["parity_claims"].pop()
        self.assert_error_contains(value, "missing parity claims")

    def test_input_is_not_mutated(self) -> None:
        value = registry()
        original = copy.deepcopy(value)
        validate_parity_registry(value, self.path)
        self.assertEqual(value, original)


if __name__ == "__main__":
    unittest.main()
