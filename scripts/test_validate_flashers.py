#!/usr/bin/env python3
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from validate_flashers import validate

ROOT = Path(__file__).resolve().parents[1]


class FlasherContractTests(unittest.TestCase):
    def test_reference_contracts(self) -> None:
        for name in ("rpi4b-builder-v1.toml", "rg35xxsp-bringup-v1.toml"):
            validate(ROOT / "product/flashers" / name, ROOT)

    def test_planned_target_cannot_enable_delivery(self) -> None:
        source = (ROOT / "product/flashers/rg35xxsp-bringup-v1.toml").read_text()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.toml"
            path.write_text(source.replace("enabled = false", "enabled = true"))
            with self.assertRaisesRegex(ValueError, "planned target must disable delivery"):
                validate(path, ROOT)

    def test_hardware_validated_builder_still_requires_native_build(self) -> None:
        source = (ROOT / "product/flashers/rpi4b-builder-v1.toml").read_text()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.toml"
            path.write_text(source.replace('  "native-derivation-build",\n', ""))
            with self.assertRaisesRegex(ValueError, "native-derivation-build"):
                validate(path, ROOT)


if __name__ == "__main__":
    unittest.main()
