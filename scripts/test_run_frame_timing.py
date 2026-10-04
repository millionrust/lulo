"""Pure-logic unit tests for scripts/behavior/run_frame_timing.py.

These run with plain `python3 -m pytest scripts/test_run_frame_timing.py` on
macOS: they cover the frame_trace.csv parsing and the latency/budget math
only. The live nested-niri session itself can only be exercised on the
reference Linux laptop; see docs/perf/ and AGENT-BRIEF.md.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_frame_timing as rft  # noqa: E402


class TraceParsingTests(unittest.TestCase):
    def test_missing_file_is_empty(self) -> None:
        self.assertEqual(rft.read_trace(Path("/nonexistent/trace.csv")), [])

    def test_reads_rows_after_the_header_and_skips_malformed_lines(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.csv"
            path.write_text("event,micros\nframe_callback,10\ninput,12\npresent,20\nnot,a,row\n")
            self.assertEqual(
                rft.read_trace(path),
                [("frame_callback", 10), ("input", 12), ("present", 20)],
            )


class LatencyAndBudgetTests(unittest.TestCase):
    def test_input_to_present_pairs_each_input_with_the_next_present(self) -> None:
        events = [("input", 0), ("draw_start", 1), ("present", 5), ("input", 6), ("present", 20)]
        self.assertEqual(rft.input_to_present_latencies_ms(events), [5.0 / 1000, 14.0 / 1000])

    def test_input_with_no_later_present_is_dropped(self) -> None:
        events = [("present", 0), ("input", 10)]
        self.assertEqual(rft.input_to_present_latencies_ms(events), [])

    def test_frame_durations_pair_draw_start_with_the_next_present(self) -> None:
        events = [("draw_start", 0), ("present", 2000), ("draw_start", 3000), ("present", 5000)]
        self.assertEqual(rft.frame_durations_ms(events), [2.0, 2.0])

    def test_draw_skip_discards_the_pending_start_without_pairing_it(self) -> None:
        events = [("draw_start", 0), ("draw_skip", 1000), ("draw_start", 2000), ("present", 4000)]
        self.assertEqual(rft.frame_durations_ms(events), [2.0])

    def test_percentile_nearest_rank_matches_measure_budgets_convention(self) -> None:
        self.assertEqual(rft.percentile_nearest_rank([1.0, 2.0, 3.0, 4.0], 0.95), 4.0)
        self.assertIsNone(rft.percentile_nearest_rank([], 0.95))

    def test_share_within_budget(self) -> None:
        self.assertAlmostEqual(rft.share_within_budget([1.0, 2.0, 3.0, 20.0], 16.7), 0.75)
        self.assertIsNone(rft.share_within_budget([], 16.7))

    def test_summarize_scenario_reports_counts_and_worst_frame(self) -> None:
        events = [
            ("input", 0), ("draw_start", 0), ("present", 5000),
            ("draw_start", 10000), ("present", 40000),
        ]
        summary = rft.summarize_scenario(events)
        self.assertEqual(summary["frame_count"], 2)
        self.assertEqual(summary["input_count"], 1)
        self.assertEqual(summary["input_to_present_p95_ms"], 5.0)
        self.assertEqual(summary["worst_frame_ms"], 30.0)
        self.assertAlmostEqual(summary["within_60hz_budget_share"], 0.5)

    def test_merge_events_combines_every_scenario(self) -> None:
        first = [("draw_start", 0), ("present", 5000)]
        second = [("draw_start", 0), ("present", 9000)]
        merged = rft.merge_events([first, second])
        self.assertEqual(merged["frame_count"], 2)
        self.assertEqual(merged["worst_frame_ms"], 9.0)


class MarkdownRenderingTests(unittest.TestCase):
    def test_render_markdown_includes_every_scenario_and_an_error_row(self) -> None:
        report = {
            "captured_at": "2026-10-05T00:00:00Z",
            "profile": "release",
            "budgets": {
                "input_to_present_p95_ms": 50.0,
                "within_60hz_budget_share": 0.99,
                "within_120hz_budget_share": 0.95,
            },
            "scenarios": {
                "text-editor-typing": {
                    "frame_count": 10, "input_count": 8, "input_to_present_p95_ms": 12.5,
                    "within_60hz_budget_share": 1.0, "within_120hz_budget_share": 0.9,
                    "worst_frame_ms": 9.0,
                },
                "mission-control": {"error": "boom"},
            },
            "overall": {
                "frame_count": 10, "input_count": 8, "input_to_present_p95_ms": 12.5,
                "within_60hz_budget_share": 1.0, "within_120hz_budget_share": 0.9,
                "worst_frame_ms": 9.0,
            },
        }
        rendered = rft.render_markdown(report)
        self.assertIn("text-editor-typing", rendered)
        self.assertIn("error: boom", rendered)
        self.assertIn("Overall", rendered)
        self.assertIn("release", rendered)


if __name__ == "__main__":
    unittest.main()
