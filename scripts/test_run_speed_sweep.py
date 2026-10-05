"""Pure-logic unit tests for scripts/behavior/run_speed_sweep.py.

Runs with plain `python3 -m pytest scripts/test_run_speed_sweep.py` on
macOS: covers only the trace-derived timing math and markdown rendering.
The live nested-niri sweep itself can only be exercised on the reference
Linux laptop; see docs/perf/speed-sweep-2026-10-05.md and AGENT-BRIEF.md.
"""

from __future__ import annotations

import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_speed_sweep as sweep  # noqa: E402


class FirstPresentTests(unittest.TestCase):
    def test_returns_the_first_present_row(self) -> None:
        events = [("frame_callback", 1), ("draw_start", 2), ("present", 10), ("present", 20)]
        self.assertEqual(sweep.first_present_micros(events), 10)

    def test_none_without_a_present_row(self) -> None:
        self.assertIsNone(sweep.first_present_micros([("draw_start", 1)]))


class QuiescenceTests(unittest.TestCase):
    def test_returns_none_when_the_trace_never_gains_a_present_row(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.csv"
            path.write_text("event,micros\ndraw_start,1\n")
            self.assertIsNone(sweep.quiescence_wait(path, deadline=time.monotonic() + 0.2))

    def test_returns_a_time_once_a_present_row_stops_growing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.csv"
            path.write_text("event,micros\npresent,1\n")
            # SETTLE_S is 0.25s; this trace never grows again, so quiescence
            # should land a little after that, well inside the deadline.
            before = time.monotonic()
            result = sweep.quiescence_wait(path, deadline=before + 2.0)
            self.assertIsNotNone(result)
            self.assertGreaterEqual(result - before, sweep.SETTLE_S)
            self.assertLess(result - before, 1.0)


class MarkdownRenderingTests(unittest.TestCase):
    def test_renders_app_and_panel_tables(self) -> None:
        report = {
            "captured_at": "2026-10-05T00:00:00Z",
            "profile": "iterate",
            "targets": {"app_first_frame_ms": 300.0, "panel_open_ms": 100.0},
            "apps": {
                "files": {
                    "first_frame_ms": 250.0,
                    "first_frame_pass": True,
                    "icons_painted_ms": 310.0,
                    "icons_painted_pass": False,
                },
                "terminal": {
                    "first_frame_ms": 200.0,
                    "first_frame_pass": True,
                    "icons_painted_ms": None,
                    "icons_painted_note": "animates continuously (not scored)",
                },
            },
            "panels": {
                "spotlight": {"open_ms": 85.0, "open_pass": True},
            },
            "not_measured": ["settings-pane-switching"],
        }
        markdown = sweep.render_markdown(report)
        self.assertIn("| files | 250 ms | PASS | 310 ms | FAIL |", markdown)
        self.assertIn("animates continuously (not scored)", markdown)
        self.assertIn("| spotlight | 85 ms | PASS |", markdown)
        self.assertIn("settings-pane-switching", markdown)

    def test_renders_an_error_row_without_crashing(self) -> None:
        report = {
            "apps": {"weather": {"error": "window never appeared"}},
            "panels": {},
        }
        markdown = sweep.render_markdown(report)
        self.assertIn("error: window never appeared", markdown)


if __name__ == "__main__":
    unittest.main()
