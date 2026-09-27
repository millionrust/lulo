"""Focused tests for the screenshot-pair evidence audit."""

from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("audit-visual-comparisons.py")
SPEC = importlib.util.spec_from_file_location("audit_visual_comparisons", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
audit_module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = audit_module
SPEC.loader.exec_module(audit_module)


def png(width: int, height: int) -> bytes:
    return (
        audit_module.PNG_SIGNATURE
        + struct.pack(">I", 13)
        + b"IHDR"
        + struct.pack(">II", width, height)
    )


def write_pair(directory: Path) -> None:
    mac = png(100, 80)
    lulo = png(100, 80)
    (directory / "mac.png").write_bytes(mac)
    (directory / "lulo.png").write_bytes(lulo)
    fields = {
        "Mac image": "mac.png",
        "Mac SHA-256": hashlib.sha256(mac).hexdigest(),
        "Mac pixel dimensions": "100 × 80",
        "Mac OS/build": "macOS 27 build 26A428",
        "Mac display": "1920 × 1080, 1×",
        "Mac appearance": "dark",
        "Mac locale/region": "en-GB, India",
        "Mac state": "Dock idle",
        "Lulo image": "lulo.png",
        "Lulo SHA-256": hashlib.sha256(lulo).hexdigest(),
        "Lulo pixel dimensions": "100 × 80",
        "Lulo revision": "a" * 40,
        "Lulo display": "1920 × 1080, 1×",
        "Lulo appearance": "dark",
        "Lulo locale/region": "en-GB, India",
        "Lulo state": "Dock idle",
        "Review": "mismatch",
        "Findings": "Shelf inset differs.",
        "Reviewer/date": "reviewer, 2026-09-27",
    }
    (directory / "comparison.md").write_text(
        "# Dock — idle\n\n" + "\n".join(f"{key}: {value}" for key, value in fields.items()) + "\n",
        encoding="utf-8",
    )


class VisualComparisonAuditTests(unittest.TestCase):
    def test_valid_pair_checks_hashes_dimensions_and_review(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pair = root / "dock-idle-dark"
            pair.mkdir()
            write_pair(pair)
            pairs, errors = audit_module.audit(root)
            self.assertEqual(errors, [])
            self.assertEqual(pairs, [("dock-idle-dark", [])])

    def test_missing_pair_files_are_reported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "dock-idle-dark").mkdir()
            pairs, _ = audit_module.audit(root)
            self.assertCountEqual(
                pairs[0][1],
                ["missing or non-regular mac.png", "missing or non-regular lulo.png",
                 "missing or non-regular comparison.md"],
            )

    def test_stale_hash_dimensions_and_review_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pair = root / "dock-idle-dark"
            pair.mkdir()
            write_pair(pair)
            record = pair / "comparison.md"
            text = record.read_text(encoding="utf-8")
            text = text.replace("Mac pixel dimensions: 100 × 80", "Mac pixel dimensions: 101 × 80")
            text = text.replace("Review: mismatch", "Review: almost")
            record.write_text(text, encoding="utf-8")
            errors = audit_module.audit_pair(pair)
            self.assertIn("Review must be pass, mismatch, or not comparable", errors)

            text = record.read_text(encoding="utf-8").replace("Review: almost", "Review: mismatch")
            record.write_text(text, encoding="utf-8")
            errors = audit_module.audit_pair(pair)
            self.assertIn("Mac pixel dimensions do not match mac.png", errors)

            (pair / "mac.png").write_bytes(png(101, 80))
            errors = audit_module.audit_pair(pair)
            self.assertIn("Mac SHA-256 does not match mac.png", errors)


if __name__ == "__main__":
    unittest.main()
