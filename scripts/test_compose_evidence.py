#!/usr/bin/env python3
"""Tests for the side-by-side evidence composer."""

from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

try:
    from PIL import Image

    PIL_AVAILABLE = True
except ImportError:  # pragma: no cover - exercised only without Pillow
    PIL_AVAILABLE = False

REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = REPO_ROOT / "scripts" / "compose-evidence.py"


@unittest.skipUnless(PIL_AVAILABLE, "Pillow is required to compose evidence")
class ComposeEvidenceTests(unittest.TestCase):
    def _write(self, path: Path, size: tuple[int, int], color: tuple[int, int, int]) -> None:
        Image.new("RGB", size, color).save(path)

    def test_composes_pair_with_default_and_explicit_labels(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rmac = root / "rmac.png"
            macos = root / "macos.png"
            output = root / "out.png"
            self._write(rmac, (40, 20), (200, 10, 10))
            self._write(macos, (30, 40), (10, 10, 200))
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    str(rmac),
                    str(macos),
                    str(output),
                    "--rmac-label",
                    "rmac @ abc1234",
                    "--macos-label",
                    "macOS 26",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            with Image.open(output) as composed:
                self.assertEqual(composed.size, (40 + 30 + 12 + 16, 40 + 28 + 8))

    def test_composes_with_derived_rmac_label(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rmac = root / "rmac.png"
            macos = root / "macos.png"
            output = root / "out.png"
            self._write(rmac, (10, 10), (0, 0, 0))
            self._write(macos, (10, 10), (255, 255, 255))
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(rmac), str(macos), str(output)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(output.is_file())

    def test_missing_input_fails_with_a_bounded_message(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            macos = root / "macos.png"
            output = root / "out.png"
            self._write(macos, (10, 10), (0, 0, 0))
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    str(root / "missing.png"),
                    str(macos),
                    str(output),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("compose-evidence:", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
