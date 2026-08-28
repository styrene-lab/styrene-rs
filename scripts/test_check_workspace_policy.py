#!/usr/bin/env python3
"""Tests for the workspace policy checker."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from typing import Any

from check_workspace_policy import validate


def package(
    name: str,
    *,
    root: Path,
    publish: list[str] | None,
    dependencies: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "name": name,
        "version": "0.1.0",
        "edition": "2024",
        "rust_version": "1.97",
        "publish": publish,
        "dependencies": dependencies or [],
        "manifest_path": str(root / "crates" / name / "Cargo.toml"),
    }


def dependency(name: str, *, requirement: str = "^0.1.0") -> dict[str, Any]:
    return {"name": name, "kind": None, "path": f"crates/{name}", "req": requirement}


class WorkspacePolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        (self.root / "Cargo.toml").write_text(
            """\
[workspace]
resolver = "3"
members = []

[workspace.package]
edition = "2024"
rust-version = "1.97"

[workspace.dependencies]
foundation = { version = "0.1.0", path = "crates/foundation" }
protocol = { version = "0.1.0", path = "crates/protocol" }
"""
        )
        (self.root / "release-plz.toml").write_text("[workspace]\nrelease = false\n")

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def metadata(self) -> dict[str, Any]:
        return {
            "metadata": {
                "styrene": {
                    "automated-release-crates": [],
                    "public-crates": ["foundation", "protocol"],
                    "layers": {
                        "foundation": ["foundation"],
                        "protocol": ["protocol"],
                        "application": ["application"],
                    },
                }
            },
            "packages": [
                package("foundation", root=self.root, publish=None),
                package(
                    "protocol",
                    root=self.root,
                    publish=None,
                    dependencies=[dependency("foundation")],
                ),
                package(
                    "application",
                    root=self.root,
                    publish=[],
                    dependencies=[dependency("protocol")],
                ),
            ],
        }

    def test_accepts_layered_workspace(self) -> None:
        self.assertEqual(validate(self.root, self.metadata()), [])

    def test_rejects_boundary_and_publication_violations(self) -> None:
        metadata = self.metadata()
        foundation = metadata["packages"][0]
        foundation["dependencies"] = [dependency("protocol")]
        metadata["packages"][2]["publish"] = None
        metadata["packages"][1]["dependencies"][0]["req"] = "^0.2.0"

        errors = validate(self.root, metadata)

        self.assertTrue(
            any("foundation (foundation) must not depend on protocol (protocol)" in error for error in errors)
        )
        self.assertIn("application: internal crate must set publish = false", errors)
        self.assertIn(
            "protocol: workspace path dependency foundation requirement '^0.2.0' != '^0.1.0'",
            errors,
        )

    def test_rejects_outdated_rust_contract(self) -> None:
        metadata = self.metadata()
        metadata["packages"][0]["edition"] = "2021"
        metadata["packages"][0]["rust_version"] = "1.75"

        errors = validate(self.root, metadata)

        self.assertIn("foundation: edition '2021' != workspace edition '2024'", errors)
        self.assertIn("foundation: rust-version '1.75' != workspace rust-version '1.97'", errors)

    def test_requires_resolver_three(self) -> None:
        manifest = self.root / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('resolver = "3"', 'resolver = "2"'))

        self.assertIn("workspace: resolver must be 3", validate(self.root, self.metadata()))

    def test_rejects_nonmember_path_dependency(self) -> None:
        metadata = self.metadata()
        metadata["packages"][1]["dependencies"][0]["path"] = "crates/excluded"

        self.assertIn(
            "protocol: local path dependency foundation is not a workspace member",
            validate(self.root, metadata),
        )

    def test_rejects_duplicate_assignment_within_one_layer(self) -> None:
        metadata = self.metadata()
        metadata["metadata"]["styrene"]["layers"]["foundation"].append("foundation")

        self.assertIn(
            "foundation: assigned more than once (foundation, foundation)",
            validate(self.root, metadata),
        )

    def test_rejects_registry_dependency_with_workspace_name(self) -> None:
        metadata = self.metadata()
        metadata["packages"][2]["dependencies"] = [
            {"name": "foundation", "kind": None, "path": None, "req": "^0.1.0"}
        ]

        self.assertIn(
            "application: workspace package foundation must use a local path dependency",
            validate(self.root, metadata),
        )

    def test_rejects_public_crate_without_crates_io_publication(self) -> None:
        metadata = self.metadata()
        metadata["packages"][0]["publish"] = ["private-registry"]

        self.assertIn("foundation: public crate must publish to crates.io", validate(self.root, metadata))

    def test_release_plz_matches_automated_release_catalog(self) -> None:
        metadata = self.metadata()
        metadata["metadata"]["styrene"]["automated-release-crates"] = ["protocol"]
        (self.root / "release-plz.toml").write_text(
            """\
[workspace]
release = false

[[package]]
name = "protocol"
publish = true
release = true
"""
        )

        self.assertEqual(validate(self.root, metadata), [])

        (self.root / "release-plz.toml").write_text(
            """\
[workspace]
release = false

[[package]]
name = "protocol"
publish = true
release = false
"""
        )
        self.assertIn(
            "protocol: automated release crate is not enabled in release-plz",
            validate(self.root, metadata),
        )

        (self.root / "release-plz.toml").write_text(
            """\
[workspace]
release = false

[[package]]
name = "missing"
publish = false
release = false

[[package]]
name = "missing"
publish = false
release = false
"""
        )
        errors = validate(self.root, metadata)
        self.assertIn("missing: duplicate release-plz package entry", errors)
        self.assertIn("missing: release-plz package is not present in the workspace", errors)
        self.assertIn("protocol: automated release crate is not enabled in release-plz", errors)


if __name__ == "__main__":
    unittest.main()
