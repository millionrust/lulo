"""Focused fixtures for the I2 visual-reference verifier."""

from __future__ import annotations

import binascii
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib


SCRIPT = Path(__file__).parent / "verify-visual-suite.py"
SPEC = importlib.util.spec_from_file_location("verify_visual_suite", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


def chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", binascii.crc32(kind + data) & 0xFFFFFFFF)
    )


def png(width: int, height: int, metadata: bool = False) -> bytes:
    rows = b"".join(b"\0" + b"\0\0\0" * width for _ in range(height))
    value = verify.PNG_SIGNATURE + chunk(
        b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    )
    if metadata:
        value += chunk(b"tEXt", b"private\0value")
    return value + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


class VisualSuiteTests(unittest.TestCase):
    def test_manifest_covers_all_critical_variants(self):
        manifest = verify.load_manifest()
        self.assertEqual(len(manifest["screens"]), 27)
        self.assertEqual(len(verify.expected_images(manifest)), 432)

    def test_png_gate_rejects_metadata_and_wrong_dimensions(self):
        self.assertEqual(verify.png_dimensions(png(10, 20)), (10, 20))
        with self.assertRaisesRegex(verify.VisualError, "metadata"):
            verify.png_dimensions(png(10, 20, metadata=True))

    def test_reference_gate_binds_hash_review_and_revision(self):
        manifest = {
            "screens": [{"group": "app", "id": "demo", "viewport": [640, 640]}],
            "themes": ["light"],
            "scales": [100],
        }
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            image = png(640, 640)
            name = "demo--light--100.png"
            (directory / name).write_bytes(image)
            review = {
                "font_sha256": {
                    "Inter": "b" * 64,
                    "JetBrains Mono": "c" * 64,
                },
                "format": 1,
                "manifest_sha256": hashlib.sha256(
                    verify.MANIFEST_PATH.read_bytes()
                ).hexdigest(),
                "results": [
                    {
                        "file": name,
                        "sha256": hashlib.sha256(image).hexdigest(),
                        "status": "pass",
                    }
                ],
                "revision": revision,
            }
            (directory / "visual-review.json").write_text(
                json.dumps(review), encoding="utf-8"
            )
            verify.verify_references(manifest, directory, revision=revision)
            review["results"][0]["status"] = "fail"
            (directory / "visual-review.json").write_text(
                json.dumps(review), encoding="utf-8"
            )
            with self.assertRaisesRegex(verify.VisualError, "every exact reference"):
                verify.verify_references(manifest, directory, revision=revision)


if __name__ == "__main__":
    unittest.main()
