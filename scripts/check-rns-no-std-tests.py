#!/usr/bin/env python3
"""Offline runner boundary tests; no Rust target or emulator prerequisite."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("rns_gate", Path(__file__).with_name("check-rns-no-std.py"))
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class RunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="rns-gate-test-", dir="/tmp/opencode")
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_child(self, program: str, timeout: float = 10) -> tuple[dict, str]:
        log = self.root / "child.log"
        result = gate.run_command([sys.executable, "-c", program], self.root,
                                  {"PATH": os.defpath}, log, timeout)
        return result, log.read_text()

    def test_marker_and_zero_exit_required(self) -> None:
        self.assertTrue(gate.result_ok(0, False, gate.COMPLETE + "\n", gate.COMPLETE))
        self.assertFalse(gate.result_ok(0, False, "", gate.COMPLETE))
        self.assertFalse(gate.result_ok(1, False, gate.COMPLETE, gate.COMPLETE))
        self.assertFalse(gate.result_ok(-9, False, gate.COMPLETE, gate.COMPLETE))
        self.assertFalse(gate.result_ok(2, False, "RNS-NOSTD:TRAP", gate.COMPLETE))
        self.assertFalse(gate.result_ok(0, True, gate.COMPLETE, gate.COMPLETE))
        self.assertFalse(gate.result_ok(0, False, gate.COMPLETE * 2, gate.COMPLETE))
        self.assertFalse(gate.result_ok(0, False, gate.BOOT_ONLY, gate.COMPLETE))
        self.assertFalse(gate.result_ok(0, False, gate.COMPLETE + "\n" + gate.FAILURE, gate.COMPLETE))

    def test_marker_only_process_rejected(self) -> None:
        result, output = self.run_child(f"print({gate.COMPLETE!r}); raise SystemExit(1)")
        self.assertFalse(gate.result_ok(result["status"], result["timed_out"], output, gate.COMPLETE))

    def test_exit_only_process_rejected(self) -> None:
        result, output = self.run_child("raise SystemExit(0)")
        self.assertEqual(result["status"], 0)
        self.assertFalse(gate.result_ok(result["status"], result["timed_out"], output, gate.COMPLETE))

    def test_timeout_even_after_marker_is_rejected_and_reaped(self) -> None:
        result, output = self.run_child(
            f"import signal; print({gate.COMPLETE!r}, flush=True); signal.pause()", timeout=0.2
        )
        self.assertTrue(result["timed_out"])
        self.assertEqual(result["status"], -9)
        self.assertFalse(gate.result_ok(result["status"], True, output, gate.COMPLETE))

    def test_only_intended_std_diagnostic_satisfies_control(self) -> None:
        message = {
            "reason": "compiler-message", "target": {"name": "rns_no_std_negative"},
            "message": {"level": "error", "code": {"code": "E0463"},
                        "message": "can't find crate for `std`",
                        "spans": [{"file_name": "src/lib.rs"}]},
        }
        valid = json.dumps(message)
        self.assertTrue(gate.expected_std_failure(101, valid))
        self.assertFalse(gate.expected_std_failure(0, valid))
        self.assertFalse(gate.expected_std_failure(-9, valid))
        self.assertFalse(gate.expected_std_failure(101, "missing lockfile"))
        self.assertFalse(gate.expected_std_failure(101, valid.replace("`std`", "`core`")))
        self.assertFalse(gate.expected_std_failure(101, valid.replace("rns_no_std_negative", "dependency")))
        self.assertFalse(gate.expected_std_failure(101, valid + "\n" + valid.replace("E0463", "E0001")))

    def test_fault_requires_exact_armed_handshake(self) -> None:
        for scenario in ("panic", "oom"):
            with self.subTest(scenario=scenario):
                armed = f"RNS-NOSTD:ARMED-{scenario.upper()}"
                intended = armed + "\n" + gate.FAILURE + "\n"
                self.assertTrue(gate.fault_ok(1, False, intended, scenario))
                self.assertFalse(gate.fault_ok(1, False, gate.FAILURE, scenario))
                self.assertFalse(gate.fault_ok(1, False, gate.FAILURE + "\n" + armed, scenario))
                self.assertFalse(gate.fault_ok(1, False, intended + gate.FAILURE, scenario))
                self.assertFalse(gate.fault_ok(1, True, intended, scenario))
                self.assertFalse(gate.fault_ok(0, False, intended, scenario))
                self.assertFalse(gate.fault_ok(1, False, intended.replace(armed, "RNS-NOSTD:TRAP"), scenario))

    def test_successful_allocation_cannot_satisfy_oom_control(self) -> None:
        returned = "RNS-NOSTD:ARMED-OOM\n" + gate.OOM_RETURNED + "\n"
        self.assertFalse(gate.fault_ok(3, False, returned, "oom"))
        self.assertFalse(gate.fault_ok(1, False, returned, "oom"))
        self.assertFalse(gate.fault_ok(1, False, "RNS-NOSTD:ARMED-PANIC\n" + gate.FAILURE, "oom"))

    def test_source_overlap_and_symlink_outputs_rejected(self) -> None:
        source = self.root / "source"
        source.mkdir()
        for destination in (source, source / "target", self.root):
            with self.assertRaises(gate.GateError):
                gate.external_root(destination, source)
        (self.root / "link").symlink_to(source, target_is_directory=True)
        with self.assertRaises(gate.GateError):
            gate.external_root(self.root / "link/output", source)
        self.assertEqual(gate.external_root(self.root / "output", source), self.root / "output")

    def test_fingerprint_detects_dirty_untracked_and_mutated_source(self) -> None:
        source = self.root / "source"
        source.mkdir()
        env = os.environ | {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}
        def git(*args: str) -> None:
            subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                            "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", *args],
                           cwd=source, env=env, capture_output=True, check=True)
        git("init")
        (source / "tracked").write_text("original")
        git("add", "tracked")
        git("commit", "-m", "fixture")
        before = gate.snapshot(source)
        gate.unchanged(source, before)
        self.assertEqual(gate.reviewed_product(source, before["head"]), before)
        with self.assertRaisesRegex(gate.GateError, "full --product-revision"):
            gate.reviewed_product(source, before["head"][:7])
        with self.assertRaisesRegex(gate.GateError, "exact reviewed revision"):
            gate.reviewed_product(source, "0" * 40)
        (source / "tracked").write_text("changed")
        with self.assertRaisesRegex(gate.GateError, "must be clean"):
            gate.reviewed_product(source, before["head"])
        with self.assertRaisesRegex(gate.GateError, "source changed"):
            gate.unchanged(source, before)
        dirty = gate.snapshot(source)
        (source / "tracked").write_text("changed again")
        self.assertNotEqual(gate.snapshot(source)["content_sha256"], dirty["content_sha256"])
        (source / "untracked").write_text("fixture only")
        self.assertIn("untracked", gate.snapshot(source)["files"])

    def test_materialized_fixture_has_independent_policy(self) -> None:
        destination = self.root / "fixture"
        gate.materialize(gate.SOURCE, destination)
        workspace = tomllib.loads((destination / "Cargo.toml").read_text())["workspace"]
        self.assertEqual(workspace["resolver"], "3")
        self.assertEqual(workspace["members"], ["smoke", "boot"])
        self.assertEqual(workspace["package"]["edition"], "2024")
        self.assertEqual(workspace["package"]["rust-version"], "1.97")
        self.assertFalse(workspace["package"]["publish"])
        dependency = workspace["dependencies"]["rns_core"]
        self.assertEqual(dependency["version"], "^0.1.0")
        self.assertFalse(dependency["default-features"])
        self.assertEqual(Path(dependency["path"]), gate.SOURCE / "crates/libs/styrene-rns")
        smoke = tomllib.loads((destination / "smoke/Cargo.toml").read_text())
        boot = tomllib.loads((destination / "boot/Cargo.toml").read_text())
        self.assertEqual(smoke["lints"]["rust"]["unsafe_code"], "forbid")
        self.assertEqual(boot["lints"]["rust"]["unsafe_code"], "deny")
        self.assertEqual(smoke["features"]["default"], [])
        separate = self.root / "separate"
        product = self.root / "reviewed-product"
        gate.materialize(gate.SOURCE, separate, product)
        selected = tomllib.loads((separate / "Cargo.toml").read_text())
        self.assertEqual(selected["workspace"]["dependencies"]["rns_core"]["path"],
                         str(product / "crates/libs/styrene-rns"))

    def test_cache_copy_omits_configuration_and_credentials(self) -> None:
        seed = self.root / "seed"
        (seed / "registry").mkdir(parents=True)
        (seed / "registry/example").write_text("public cached data")
        (seed / "credentials.toml").write_text("synthetic must not copy")
        (seed / "config.toml").write_text("synthetic must not copy")
        destination = self.root / "cargo-home"
        destination.mkdir()
        gate.seed_cache(seed, destination)
        self.assertEqual(list(destination.iterdir()), [destination / "registry"])

    def test_missing_tool_refused_without_path_fallback(self) -> None:
        for path in ("rustc", str(self.root / "missing")):
            with self.assertRaisesRegex(gate.GateError, "existing absolute executable"):
                gate.tool(path, "rustc", self.root)

    def test_materialized_mutations_change_fingerprint(self) -> None:
        fixture = self.root / "fixture"
        fixture.mkdir()
        (fixture / "lib.rs").write_text("original")
        before = gate.fixture_snapshot(fixture)
        (fixture / "lib.rs").write_text("changed")
        self.assertNotEqual(before, gate.fixture_snapshot(fixture))
        (fixture / "outside").symlink_to(self.root / "elsewhere")
        with self.assertRaisesRegex(gate.GateError, "symlink"):
            gate.fixture_snapshot(fixture)


if __name__ == "__main__":
    unittest.main()
