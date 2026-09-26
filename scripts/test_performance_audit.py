"""Focused fixtures for the I4 performance release audit."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-performance-audit.py"
SPEC = importlib.util.spec_from_file_location("verify_performance_audit", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class PerformanceAuditTests(unittest.TestCase):
    def test_committed_budgets_cover_release_dimensions(self):
        budgets = verify.load_budgets()
        specs = verify.result_specs(budgets)
        self.assertEqual(len(specs), 94)
        metrics = {spec["metric"] for spec in specs}
        self.assertIn("startup-p95", metrics)
        self.assertIn("idle-wakeups", metrics)
        self.assertIn("frame-p95-120hz", metrics)
        self.assertIn("rss-growth-absolute-8h", metrics)

    def test_rejects_relaxed_frame_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "budgets.json"
            document = json.loads(verify.BUDGET_PATH.read_text(encoding="utf-8"))
            document["rendering"][1]["frame_p95_ms"] = 12
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.PerformanceError, "differ"):
                verify.load_budgets(path)

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
            verify.verify_evidence_directory(
                budgets, directory, tier="alpha", revision=revision
            )
            first = directory / "amd64-intel-laptop.json"
            document = json.loads(first.read_text(encoding="utf-8"))
            document["results"][0]["value"] = document["results"][0]["limit"] + 0.1
            first.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.PerformanceError, "budget"):
                verify.verify_evidence_directory(
                    budgets, directory, tier="alpha", revision=revision
                )


if __name__ == "__main__":
    unittest.main()
