"""Contract checks for the offline native interaction trace analyzer."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("analyze-interaction-trace.py")
SPEC = importlib.util.spec_from_file_location("analyze_interaction_trace", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
analyzer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = analyzer
SPEC.loader.exec_module(analyzer)


def trace(refresh_hz: int = 60) -> dict[str, object]:
    return {
        "format": 1,
        "refresh_hz": refresh_hz,
        "start_ns": 0,
        "end_ns": 60_000_000_000,
        "interactions": [
            {"id": str(index), "input_ns": index * 1_000_000_000,
             "visible_ns": index * 1_000_000_000 + 20_000_000}
            for index in range(20)
        ],
        "frames": [
            {"at_ns": index * 1_000_000_000 // refresh_hz,
             "duration_ns": 8_000_000, "missed": False}
            for index in range(refresh_hz * 60)
        ],
    }


class InteractionTraceTests(unittest.TestCase):
    def test_reports_p95_nearest_rank_and_refresh_specific_budget(self):
        document = trace(120)
        document["interactions"][-1]["visible_ns"] += 30_000_000
        result = analyzer.analyze(document)
        self.assertEqual(result["metrics"]["input_to_visible_p95_ms"], 20)
        self.assertEqual(result["metrics"]["frame_p95_ms"], 8)
        self.assertTrue(result["within_limits"])
        self.assertTrue(result["native_trace_review_required"])

    def test_rejects_short_or_malformed_evidence(self):
        document = trace()
        document["end_ns"] = 59_999_999_999
        with self.assertRaisesRegex(analyzer.TraceError, "60 seconds"):
            analyzer.analyze(document)
        document = trace()
        document["interactions"][0]["visible_ns"] = -1
        with self.assertRaisesRegex(analyzer.TraceError, "non-negative"):
            analyzer.analyze(document)

    def test_reports_budget_failure_without_claiming_native_verification(self):
        document = trace(60)
        for sample in document["interactions"][-2:]:
            sample["visible_ns"] += 100_000_000
        result = analyzer.analyze(document)
        self.assertEqual(result["measurement_status"], "unverified_trace_summary")
        self.assertFalse(result["within_limits"])

    def test_rejects_sparse_or_clustered_frame_exports(self):
        document = trace(60)
        document["frames"] = document["frames"][::3]
        with self.assertRaisesRegex(analyzer.TraceError, "output-frame samples"):
            analyzer.analyze(document)
        document = trace(60)
        for sample in document["frames"]:
            sample["at_ns"] //= 2
        with self.assertRaisesRegex(analyzer.TraceError, "do not span"):
            analyzer.analyze(document)

    def test_unreported_output_slots_count_as_misses(self):
        document = trace(60)
        document["frames"].pop(100)
        result = analyzer.analyze(document)
        self.assertEqual(result["sample_counts"]["unreported_frames"], 1)
        self.assertGreater(result["metrics"]["missed_frames_percent"], 0)

    def test_trace_reader_rejects_symlink_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "trace.json"
            source.write_text(json.dumps(trace()), encoding="utf-8")
            link = Path(directory) / "link.json"
            link.symlink_to(source)
            with self.assertRaisesRegex(analyzer.TraceError, "unreadable"):
                analyzer.read_trace(link)


if __name__ == "__main__":
    unittest.main()
