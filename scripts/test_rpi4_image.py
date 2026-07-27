#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

MODULE_PATH = Path(__file__).with_name("rpi4_image.py")
spec = importlib.util.spec_from_file_location("rpi4_image", MODULE_PATH)
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DebugfsQuotingTests(unittest.TestCase):
    def test_preserves_literal_backslash_in_systemd_filename(self) -> None:
        value = r"/nix/store/example/system-systemd\x2dcryptsetup.slice"
        self.assertEqual(module.quote(value), f'"{value}"')

    def test_escapes_only_double_quote_delimiter(self) -> None:
        self.assertEqual(module.quote('/a/"quoted"'), '"/a/\\"quoted\\""')

    def test_run_batch_keeps_literal_backslash(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / "root.img"
            command = r'mkdir "/system-systemd\x2dcryptsetup.slice"'
            captured: dict[str, str] = {}
            original_write_text = Path.write_text

            def capture(path: Path, content: str, *args: object, **kwargs: object) -> int:
                captured["content"] = content
                return original_write_text(path, content, *args, **kwargs)

            with patch.object(Path, "write_text", capture), patch.object(
                module.subprocess, "run"
            ):
                module.run_batch(image, [command])
            self.assertEqual(captured["content"], command + "\n")


if __name__ == "__main__":
    unittest.main()
