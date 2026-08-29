#!/usr/bin/env python3
"""Tests for versioned fixture provenance validation."""

from __future__ import annotations

import hashlib
import json
import pathlib
import tempfile
import unittest

from validate_fixture_provenance import validate


class FixtureProvenanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        (self.root / "fixtures").mkdir()
        (self.root / "generators").mkdir()
        self.artifact = self.root / "fixtures" / "packet.bin"
        self.artifact.write_bytes(b"packet")
        (self.root / "generators" / "generate.py").write_text("# fixture generator\n")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def index(self) -> dict:
        return {
            "schema_version": 2,
            "authorities": {
                "rns-1.5.1": {
                    "repository": "https://github.com/markqvist/Reticulum.git",
                    "revision": "149e4151095adf098b8f53eab0c03b37169e8559",
                    "release": "1.5.1",
                }
            },
            "vectors": [
                {
                    "id": "packet",
                    "authority_id": "rns-1.5.1",
                    "kind": "packet",
                    "artifact": "fixtures/packet.bin",
                    "sha256": hashlib.sha256(b"packet").hexdigest(),
                    "generator": "generators/generate.py",
                    "source_symbols": ["RNS.Packet.Packet.unpack"],
                    "expected": {"type": "packet", "accepted": True},
                }
            ],
        }

    def write_index(self, value: dict) -> pathlib.Path:
        path = self.root / "index-v2.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def assert_error_contains(self, value: dict, text: str) -> None:
        errors = validate(self.write_index(value), self.root)
        self.assertTrue(any(text in error for error in errors), errors)

    def test_valid_v2_index(self) -> None:
        self.assertEqual(validate(self.write_index(self.index()), self.root), [])

    def test_rejects_mutable_authority_revision(self) -> None:
        value = self.index()
        value["authorities"]["rns-1.5.1"]["revision"] = "main"
        self.assert_error_contains(value, "revision must be a full commit SHA")

    def test_rejects_duplicate_vector_id(self) -> None:
        value = self.index()
        value["vectors"].append(value["vectors"][0].copy())
        self.assert_error_contains(value, "duplicate vector id")

    def test_rejects_unknown_authority(self) -> None:
        value = self.index()
        value["vectors"][0]["authority_id"] = "missing"
        self.assert_error_contains(value, "unknown authority")

    def test_rejects_repository_escape(self) -> None:
        value = self.index()
        value["vectors"][0]["artifact"] = "../packet.bin"
        self.assert_error_contains(value, "artifact escapes repository root")

    def test_rejects_digest_mismatch(self) -> None:
        value = self.index()
        value["vectors"][0]["sha256"] = "0" * 64
        self.assert_error_contains(value, "digest mismatch")

    def test_rejects_empty_source_symbols(self) -> None:
        value = self.index()
        value["vectors"][0]["source_symbols"] = []
        self.assert_error_contains(value, "source_symbols must be a non-empty array")

    def test_rejects_untyped_expected_outcome(self) -> None:
        value = self.index()
        value["vectors"][0]["expected"] = {"accepted": False}
        self.assert_error_contains(value, "expected.type must be a non-empty string")


if __name__ == "__main__":
    unittest.main()
