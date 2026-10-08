"""Trace analysis in scripts/behavior/run_spotlight_intents.py (ADR 0024
"Phase 1.1"): the keystroke that closes Spotlight is not a stutter, action
keys are reported apart from typing, and the row's own frame and the
service's timings are read from the launcher's marks."""

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_spotlight_intents as intents  # noqa: E402


def write_trace(rows: list[tuple[str, int]], origin: int = 1_000_000) -> Path:
    handle = tempfile.NamedTemporaryFile("w", suffix=".csv", delete=False)
    handle.write("event,micros\n")
    handle.write(f"monotonic_origin,{origin}\n")
    for event, micros in rows:
        handle.write(f"{event},{micros}\n")
    handle.close()
    return Path(handle.name)


class TraceTests(unittest.TestCase):
    def test_a_closing_key_is_not_paired_with_the_next_window(self):
        path = write_trace([
            ("input", 100_000), ("present", 120_000),
            # Escape closes Spotlight; the next frame is a new window's.
            ("input", 200_000), ("open_window", 750_000), ("present", 780_000),
        ])
        self.assertEqual(intents.input_to_present_latencies(path), [20_000])

    def test_action_keys_are_reported_apart(self):
        # origin 1 s: the action key at monotonic 1.3 s is trace time 300 ms.
        path = write_trace([
            ("input", 100_000), ("present", 110_000),
            ("input", 300_000), ("present", 520_000),
        ])
        self.assertEqual(intents.input_to_present_latencies(path, [1.29]), [10_000])
        self.assertEqual(intents.action_latencies(path, [1.29]), [220_000])

    def test_keystroke_to_row_uses_the_last_keystroke(self):
        path = write_trace([
            ("input", 100_000), ("present", 110_000),
            ("input", 150_000), ("present", 160_000),
            ("assist_row_applied", 700_000), ("present", 720_000),
        ])
        # Typing finished at monotonic 1.16 s.
        self.assertEqual(intents.keystroke_to_row(path, [1.16]), [570.0])

    def test_row_frames_and_service_timings(self):
        path = write_trace([
            ("assist_reply", 500_000),
            ("assist_timing:total_ms=240:prefill_ms=150:decode_ms=80:passes=1", 500_001),
            ("assist_row_applied", 500_100),
            ("launcher_render", 504_100), ("draw_start", 506_100), ("present", 516_100),
        ])
        frames = intents.assist_frames(path)
        self.assertEqual(len(frames), 1)
        self.assertAlmostEqual(frames[0]["render_ms"], 2.0)
        self.assertAlmostEqual(frames[0]["total_ms"], 16.0)
        timings = intents.service_timings(path)
        self.assertEqual(timings, [{"total_ms": 240.0, "prefill_ms": 150.0,
                                    "decode_ms": 80.0, "passes": 1.0}])


if __name__ == "__main__":
    unittest.main()
