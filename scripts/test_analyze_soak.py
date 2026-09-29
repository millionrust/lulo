"""Pure-logic unit tests for scripts/linux/analyze-soak.py.

Run with `python3 -m pytest scripts/test_analyze_soak.py` on macOS: these
cover the regression math, per-process budget evaluation, shell-combining
and JSONL grouping only, with synthetic sample data (no live soak run
required). See docs/beta-checklist.md's "Memory (8-hour soak)" gate.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

SCRIPT = Path(__file__).parent / "linux" / "analyze-soak.py"
SPEC = importlib.util.spec_from_file_location("analyze_soak", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
analyze_soak = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = analyze_soak
SPEC.loader.exec_module(analyze_soak)


def make_record(app: str, elapsed_seconds: float, rss_mib: float, pss_mib: float, **extra) -> dict:
    record = {
        "app": app,
        "elapsed_seconds": elapsed_seconds,
        "alive": True,
        "rss_kib": rss_mib * 1024,
        "pss_kib": pss_mib * 1024,
    }
    record.update(extra)
    return record


class LinearRegressionTests(unittest.TestCase):
    def test_perfect_line_has_r_squared_one(self):
        xs = [0.0, 1.0, 2.0, 3.0]
        ys = [10.0, 12.0, 14.0, 16.0]
        slope, intercept, r_squared = analyze_soak.linear_regression(xs, ys)
        self.assertAlmostEqual(slope, 2.0)
        self.assertAlmostEqual(intercept, 10.0)
        self.assertAlmostEqual(r_squared, 1.0)

    def test_flat_series_has_zero_slope(self):
        xs = [0.0, 1.0, 2.0]
        ys = [50.0, 50.0, 50.0]
        slope, _intercept, r_squared = analyze_soak.linear_regression(xs, ys)
        self.assertEqual(slope, 0.0)
        self.assertEqual(r_squared, 0.0)

    def test_two_points_have_no_r_squared(self):
        slope, intercept, r_squared = analyze_soak.linear_regression([0.0, 1.0], [10.0, 12.0])
        self.assertAlmostEqual(slope, 2.0)
        self.assertEqual(r_squared, 0.0)


class GroupByAppTests(unittest.TestCase):
    def test_groups_and_sorts_by_elapsed(self):
        records = [
            {"app": "clock", "elapsed_seconds": 300},
            {"app": "clock", "elapsed_seconds": 0},
            {"app": "notes", "elapsed_seconds": 0},
        ]
        grouped = analyze_soak.group_by_app(records)
        self.assertEqual([r["elapsed_seconds"] for r in grouped["clock"]], [0, 300])
        self.assertEqual(len(grouped["notes"]), 1)


class EvaluateProcessTests(unittest.TestCase):
    def test_stable_process_is_not_flagged(self):
        records = [make_record("clock", t * 300, 50.0, 45.0) for t in range(10)]
        result = analyze_soak.evaluate_process("clock", records, rss_budget_mib=128, growth_budget_mib_8h=16)
        self.assertFalse(result["leak_suspected"])
        self.assertEqual(result["rss"]["start_mib"], 50.0)
        self.assertEqual(result["rss"]["end_mib"], 50.0)

    def test_sustained_growth_is_flagged(self):
        # +2 MiB every 5 minutes for 8 hours: ~24 MiB/hour, far past 5%/h of
        # a 50 MiB baseline and past the 16 MiB/8h growth budget.
        records = [make_record("leaky", t * 300, 50.0 + t * 2.0, 45.0 + t * 2.0) for t in range(96)]
        result = analyze_soak.evaluate_process("leaky", records, rss_budget_mib=1000, growth_budget_mib_8h=16)
        self.assertTrue(result["leak_suspected"])
        self.assertTrue(result["rss"]["sustained_growth"])
        self.assertTrue(result["rss"]["over_growth_budget"])

    def test_over_peak_budget_is_flagged_even_without_growth(self):
        records = [make_record("bloated", t * 300, 200.0, 190.0) for t in range(10)]
        result = analyze_soak.evaluate_process("bloated", records, rss_budget_mib=128, growth_budget_mib_8h=16)
        self.assertTrue(result["leak_suspected"])
        self.assertTrue(result["rss"]["over_peak_budget"])
        self.assertFalse(result["rss"]["sustained_growth"])

    def test_dead_process_is_reported(self):
        records = [
            make_record("crashy", 0, 50.0, 45.0),
            make_record("crashy", 300, 51.0, 46.0),
            {"app": "crashy", "elapsed_seconds": 600, "alive": False, "exit_code": 1},
        ]
        result = analyze_soak.evaluate_process("crashy", records, rss_budget_mib=128, growth_budget_mib_8h=16)
        self.assertFalse(result["alive_at_end"])
        self.assertEqual(result["died_at_elapsed_seconds"], 600)
        self.assertEqual(result["died_exit_code"], 1)

    def test_single_noisy_sample_does_not_falsely_flag_a_leak(self):
        # One brief spike then back down: high nominal slope over very few
        # points, but a weak fit (R^2 below the sustained-growth threshold).
        records = [
            make_record("clock", 0, 50.0, 45.0),
            make_record("clock", 300, 90.0, 80.0),
            make_record("clock", 600, 50.0, 45.0),
        ]
        result = analyze_soak.evaluate_process("clock", records, rss_budget_mib=128, growth_budget_mib_8h=16)
        self.assertFalse(result["rss"]["sustained_growth"])


class CombineShellTests(unittest.TestCase):
    def test_sums_pieces_at_shared_timestamps(self):
        grouped = {
            "wallpaper": [make_record("wallpaper", 0, 20.0, 18.0), make_record("wallpaper", 300, 20.0, 18.0)],
            "top-bar": [make_record("top-bar", 0, 15.0, 13.0), make_record("top-bar", 300, 15.0, 13.0)],
            "dock": [make_record("dock", 0, 25.0, 22.0), make_record("dock", 300, 25.0, 22.0)],
            "clock": [make_record("clock", 0, 50.0, 45.0)],  # not a shell piece
        }
        combined = analyze_soak.combine_shell(grouped, combined_rss_budget_mib=256, combined_growth_budget_mib_8h=24)
        self.assertIsNotNone(combined)
        self.assertEqual(combined["rss"]["start_mib"], 60.0)
        self.assertEqual(combined["pieces"], ["dock", "top-bar", "wallpaper"])

    def test_partial_sample_point_is_excluded(self):
        grouped = {
            "wallpaper": [make_record("wallpaper", 0, 20.0, 18.0), make_record("wallpaper", 300, 20.0, 18.0)],
            "top-bar": [make_record("top-bar", 0, 15.0, 13.0)],  # missing the 300s sample
            "dock": [make_record("dock", 0, 25.0, 22.0), make_record("dock", 300, 25.0, 22.0)],
        }
        combined = analyze_soak.combine_shell(grouped, combined_rss_budget_mib=256, combined_growth_budget_mib_8h=24)
        self.assertIsNotNone(combined)
        self.assertEqual(combined["complete_sample_points"], 1)


class BuildReportTests(unittest.TestCase):
    def test_end_to_end_json_round_trips_and_flags_leaks(self):
        with TemporaryDirectory() as directory:
            samples_path = Path(directory) / "samples.jsonl"
            lines = []
            for t in range(20):
                lines.append(json.dumps(make_record("clock", t * 300, 50.0, 45.0)))
                lines.append(json.dumps(make_record("leaky-app", t * 300, 50.0 + t * 3.0, 45.0 + t * 3.0)))
            samples_path.write_text("\n".join(lines) + "\n")

            records = analyze_soak.load_samples(samples_path)
            grouped = analyze_soak.group_by_app(records)
            budgets = {
                "memory": {
                    "application_rss_mib": 128,
                    "application_rss_growth_mib_8h": 16,
                    "combined_shell_rss_mib": 256,
                    "combined_shell_rss_growth_mib_8h": 24,
                }
            }
            report = analyze_soak.build_report(samples_path, grouped, budgets)
            self.assertIn("leaky-app", report["leaks_detected"])
            self.assertNotIn("clock", report["leaks_detected"])
            self.assertFalse(report["within_budget"])
            # The report itself must be JSON-serialisable (as the CLI writes it).
            json.dumps(report)


if __name__ == "__main__":
    unittest.main()
