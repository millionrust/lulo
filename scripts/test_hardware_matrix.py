"""Focused fixtures for the H8 hardware matrix."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux/verify-hardware-matrix.py"
SPEC = importlib.util.spec_from_file_location("verify_hardware_matrix", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class HardwareMatrixTests(unittest.TestCase):
    def test_committed_matrix_has_complete_release_coverage(self):
        matrix = verify.load_matrix()
        self.assertEqual(len(matrix["stations"]), 5)
        self.assertIn("amd64-nvidia-desktop", matrix["release_tiers"]["beta"])
        self.assertIn("arm64-reference", matrix["release_tiers"]["one-dot-zero"])

    def test_rejects_missing_architecture_vendor_scale_and_device_coverage(self):
        mutations = (
            ("architecture", "arm64", "amd64"),
            ("gpu_vendor", "nvidia", "amd"),
        )
        for field, old, new in mutations:
            with self.subTest(field=field):
                with tempfile.TemporaryDirectory() as temporary:
                    path = Path(temporary) / "matrix.json"
                    document = json.loads(verify.MATRIX_PATH.read_text())
                    for station in document["stations"]:
                        if station[field] == old:
                            station[field] = new
                    path.write_text(json.dumps(document), encoding="utf-8")
                    with self.assertRaises(verify.MatrixError):
                        verify.load_matrix(path)

    def test_alpha_evidence_requires_every_exact_pass(self):
        matrix = verify.load_matrix()
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            stations = {station["id"]: station for station in matrix["stations"]}
            for station_id in matrix["release_tiers"]["alpha"]:
                results = [
                    {"check": check, "status": "pass"}
                    for check in verify.required_checks(stations[station_id])
                ]
                (directory / f"{station_id}.json").write_text(
                    json.dumps(
                        {
                            "format": 1,
                            "results": results,
                            "revision": revision,
                            "station": station_id,
                        }
                    ),
                    encoding="utf-8",
                )
            verify.verify_evidence(
                matrix, directory, tier="alpha", revision=revision
            )
            first = directory / "amd64-intel-laptop.json"
            document = json.loads(first.read_text())
            document["results"][0]["status"] = "blocked"
            first.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.MatrixError, "every required check"):
                verify.verify_evidence(
                    matrix, directory, tier="alpha", revision=revision
                )


if __name__ == "__main__":
    unittest.main()
