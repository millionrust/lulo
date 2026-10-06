"""Pure-logic unit tests for scripts/behavior/speed_interactions.py (the
speed sweep's interaction scenarios): trace math and fixtures only. The
scenarios themselves need the reference laptop's nested niri session."""

from __future__ import annotations

import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_speed_sweep  # noqa: E402
import speed_interactions as si  # noqa: E402


class FrameCostTests(unittest.TestCase):
    def test_pairs_each_callback_with_its_present(self) -> None:
        events = [("frame_callback", 0), ("draw_start", 4000), ("present", 6000),
                  ("frame_callback", 16000), ("draw_start", 30000), ("present", 40000)]
        self.assertEqual(si.frame_costs_ms(events), [6.0, 24.0])

    def test_skipped_and_idle_callbacks_are_not_frames(self) -> None:
        events = [("frame_callback", 0), ("draw_skip", 1000), ("present", 2000),
                  ("frame_callback", 10000), ("frame_callback", 20000), ("present", 25000)]
        self.assertEqual(si.frame_costs_ms(events), [5.0])

    def test_summary_scores_the_share_within_budget(self) -> None:
        events = []
        for index in range(100):
            start = index * 20000
            cost = 30000 if index == 0 else 5000
            events += [("frame_callback", start), ("present", start + cost)]
        summary = si.frames_summary(events)
        self.assertEqual(summary["frames"], 100)
        self.assertAlmostEqual(summary["within_16_7ms_share"], 0.99)
        self.assertTrue(summary["pass"])
        self.assertEqual(summary["worst_frame_ms"], 30.0)
        self.assertEqual(summary["stalls"], 0)


class KeyLatencyTests(unittest.TestCase):
    def test_measures_from_each_press_not_its_release(self) -> None:
        # press at 0, release at 20 ms, present at 30 ms; press at 100 ms,
        # present at 105 ms, release at 120 ms.
        events = [("input", 0), ("input", 20000), ("present", 30000),
                  ("input", 100000), ("present", 105000), ("input", 120000)]
        self.assertEqual(si.key_press_latencies_ms(events), [30.0, 5.0])

    def test_presses_are_inputs_after_a_gap(self) -> None:
        events = [("input", 0), ("input", 2000), ("input", 25000), ("input", 90000), ("input", 112000)]
        self.assertEqual(si.key_presses(events), [0, 90000])

    def test_settled_stops_at_the_first_quiet_gap(self) -> None:
        # Results land 10 and 40 ms after the key; a caret blink 600 ms
        # later is not content arriving.
        events = [("input", 0), ("present", 10000), ("present", 40000), ("present", 640000)]
        self.assertEqual(si.settled_after(events, 0), 40000)

    def test_typing_summary_judges_the_p95_echo(self) -> None:
        events = []
        for index in range(20):
            base = index * 100000
            events += [("input", base), ("frame_callback", base + 1000),
                       ("present", base + 8000), ("input", base + 20000)]
        summary = si.typing_summary(events)
        self.assertEqual(summary["keys"], 20)
        self.assertEqual(summary["echo_p95_ms"], 8.0)
        self.assertTrue(summary["pass"])


class FixtureTests(unittest.TestCase):
    def test_png_is_valid(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "a.png"
            si.write_png(path, 4, 3, (10, 20, 30))
            data = path.read_bytes()
            self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n"))
            idat = data.index(b"IDAT")
            length = int.from_bytes(data[idat - 4:idat], "big")
            raw = zlib.decompress(data[idat + 4:idat + 4 + length])
            self.assertEqual(len(raw), 3 * (1 + 4 * 3))

    def test_big_folder_has_the_requested_items(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            folder = si.make_big_folder(Path(tmp), count=25)
            self.assertEqual(len(list(folder.iterdir())), 25)


class SweepWiringTests(unittest.TestCase):
    def test_every_scenario_has_a_method(self) -> None:
        for name, method in si.SCENARIOS.items():
            self.assertTrue(callable(getattr(run_speed_sweep.Run, method, None)), name)

    def test_markdown_lists_interactions(self) -> None:
        report = {"interactions": {
            "files-list-scroll": {"frames": 80, "within_16_7ms_share": 1.0, "frame_p95_ms": 6.0,
                                  "worst_frame_ms": 9.0, "pass": True},
            "quick-look": {"error": "Space did not open Quick Look"},
        }}
        text = run_speed_sweep.render_markdown(report)
        self.assertIn("| files-list-scroll | 80 | 100.0% |", text)
        self.assertIn("error: Space did not open Quick Look", text)


if __name__ == "__main__":
    unittest.main()
