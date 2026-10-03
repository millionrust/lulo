"""Focused fixtures for the I4 performance release audit."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parent / "verify-performance-audit.py"
SPEC = importlib.util.spec_from_file_location("verify_performance_audit", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)
SMOKE_SPEC = importlib.util.spec_from_file_location(
    "performance_smoke_inventory", Path(__file__).parent / "linux" / "smoke-app-launches.py"
)
assert SMOKE_SPEC is not None and SMOKE_SPEC.loader is not None
smoke = importlib.util.module_from_spec(SMOKE_SPEC)
sys.modules[SMOKE_SPEC.name] = smoke
SMOKE_SPEC.loader.exec_module(smoke)
MEASURE_SPEC = importlib.util.spec_from_file_location(
    "performance_linux_measurement_inventory", Path(__file__).parent / "linux" / "measure-budgets.py"
)
assert MEASURE_SPEC is not None and MEASURE_SPEC.loader is not None
measure = importlib.util.module_from_spec(MEASURE_SPEC)
sys.modules[MEASURE_SPEC.name] = measure
MEASURE_SPEC.loader.exec_module(measure)


class PerformanceAuditTests(unittest.TestCase):
    def test_committed_budgets_cover_release_dimensions(self):
        budgets = verify.load_budgets()
        specs = verify.result_specs(budgets)
        self.assertEqual(len(specs), 119)
        metrics = {spec["metric"] for spec in specs}
        self.assertIn("startup-p95", metrics)
        self.assertIn("idle-wakeups", metrics)
        self.assertIn("frame-p95-120hz", metrics)
        self.assertIn("rss-growth-absolute-8h", metrics)

    def test_budgets_cover_every_packaged_window_app(self):
        package_binary = {
            "rmac-activity-monitor": "rmac-system-monitor",
            "rmac-finder": "rmac-files",
        }
        budget_binaries = {
            package_binary.get(app["id"], app["id"])
            for app in verify.load_budgets()["applications"]
        }
        window_binaries = {
            spec.binary for spec in smoke.APP_SPECS if spec.mode != "archive"
        }
        self.assertEqual(budget_binaries, window_binaries)
        self.assertEqual({app[1] for app in measure.APPS}, window_binaries)

    def test_rejects_relaxed_frame_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "budgets.json"
            document = json.loads(verify.BUDGET_PATH.read_text(encoding="utf-8"))
            document["rendering"][1]["frame_p95_ms"] = 12
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.PerformanceError, "differ"):
                verify.load_budgets(path)

    def test_rejects_unlisted_malformed_journey_record(self):
        original = verify._load_json
        document = original(verify.JOURNEY_PATH)
        document["journeys"].append({"unexpected": "missing name"})
        with patch.object(
            verify, "_load_json",
            side_effect=lambda path: document if path == verify.JOURNEY_PATH else original(path),
        ):
            with self.assertRaisesRegex(verify.PerformanceError, "journey source inventory"):
                verify._journey_names()

    def test_rejects_unlisted_malformed_hardware_station(self):
        original = verify._load_json
        document = original(verify.HARDWARE_PATH)
        document["stations"].append({"unexpected": "missing id"})
        with patch.object(
            verify, "_load_json",
            side_effect=lambda path: document if path == verify.HARDWARE_PATH else original(path),
        ):
            with self.assertRaisesRegex(verify.PerformanceError, "hardware source inventory"):
                verify._hardware()

    def test_alpha_directory_requires_every_station_and_budget(self):
        budgets = verify.load_budgets()
        revision = "b" * 40
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for station in ("amd64-intel-laptop", "amd64-amd-desktop"):
                document = verify.evidence_template(budgets, station, revision)
                for result in document["results"]:
                    result["status"] = "pass"
                    result["value"] = result["limit"]
                (directory / f"{station}.json").write_text(
                    json.dumps(document), encoding="utf-8"
                )
            with patch.object(verify, "_verify_checkout"):
                verify.verify_evidence_directory(
                    budgets, directory, tier="alpha", revision=revision
                )
            first = directory / "amd64-intel-laptop.json"
            document = json.loads(first.read_text(encoding="utf-8"))
            document["results"][0]["value"] = document["results"][0]["limit"] + 0.1
            first.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.PerformanceError, "budget"):
                    verify.verify_evidence_directory(
                        budgets, directory, tier="alpha", revision=revision
                    )

    def test_candidate_measurements_require_matching_clean_checkout(self):
        revision = "b" * 40
        def completed(code: int, output: bytes) -> subprocess.CompletedProcess[bytes]:
            return subprocess.CompletedProcess(["git"], code, output, b"")

        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, revision.encode() + b"\n"), completed(0, b""),
        ]):
            verify._verify_checkout(revision)
        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, ("a" * 40).encode() + b"\n"), completed(0, b""),
        ]):
            with self.assertRaisesRegex(verify.PerformanceError, "revision differs"):
                verify._verify_checkout(revision)
        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, revision.encode() + b"\n"), completed(0, b" M src/main.rs\n"),
        ]):
            with self.assertRaisesRegex(verify.PerformanceError, "clean checkout"):
                verify._verify_checkout(revision)

    def test_old_eight_app_station_cannot_pass_new_inventory(self):
        budgets = verify.load_budgets()
        station = "amd64-intel-laptop"
        document = verify.evidence_template(budgets, station, "b" * 40)
        new_apps = {"rmac-calculator", "rmac-player", "rmac-preview", "rmac-weather"}
        document["results"] = [
            result for result in document["results"]
            if result["subject"] not in new_apps
        ]
        for result in document["results"]:
            result["status"] = "pass"
            result["value"] = result["limit"]
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / f"{station}.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.PerformanceError, "inventory"):
                verify._verify_station(budgets, path, station, "b" * 40)


if __name__ == "__main__":
    unittest.main()
