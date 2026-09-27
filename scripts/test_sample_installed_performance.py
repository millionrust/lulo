"""Focused tests for private installed-app launch preparation."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "sample-installed-performance.py"
SPEC = importlib.util.spec_from_file_location("sample_installed_performance", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
sampler = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = sampler
SPEC.loader.exec_module(sampler)


class InstalledLaunchPreparationTests(unittest.TestCase):
    def test_app_drawer_uses_supervised_show_mode_and_resolved_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "bin" / "rmac-app-drawer"
            binary.parent.mkdir()
            binary.write_text("placeholder")
            binary.chmod(0o755)
            spec = next(row for row in sampler.smoke.APP_SPECS if row.binary == binary.name)

            command = sampler.launch_command(binary, spec, root / "fixtures")

            self.assertEqual(command, [str(binary.resolve()), "--service", "--show"])

    def test_preview_receives_generated_document_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixtures = root / "fixtures"
            sampler.smoke.create_fixtures(fixtures)
            binary = root / "rmac-preview"
            binary.write_text("placeholder")
            binary.chmod(0o755)
            spec = next(row for row in sampler.smoke.APP_SPECS if row.binary == binary.name)

            command = sampler.launch_command(binary, spec, fixtures)

            self.assertEqual(command[0], str(binary.resolve()))
            self.assertEqual(command[1:], [str(fixtures / "smoke-document.pdf")])
            self.assertTrue((fixtures / "smoke-document.pdf").read_bytes().startswith(b"%PDF-1.4"))


if __name__ == "__main__":
    unittest.main()
