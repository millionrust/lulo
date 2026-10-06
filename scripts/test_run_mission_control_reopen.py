"""Pure-logic unit tests for scripts/behavior/run_mission_control_reopen.py.

The nested-niri probe itself runs only on the reference Linux laptop; these
cover the screen comparison and trace counting it decides with.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_mission_control_reopen as reopen  # noqa: E402


class DiffShareTest(unittest.TestCase):
    def test_identical_captures_do_not_differ(self) -> None:
        picture = bytes(range(256)) * 100
        self.assertEqual(reopen.diff_share(picture, picture), 0.0)

    def test_counts_differing_bytes_as_a_share(self) -> None:
        before = bytes(10_000)
        after = bytes(9_900) + b"\xff" * 100
        self.assertAlmostEqual(reopen.diff_share(before, after), 0.01)
        self.assertGreater(reopen.diff_share(before, after), reopen.SAME_SCREEN_MAX_DIFF)

    def test_a_size_change_or_empty_capture_never_matches(self) -> None:
        self.assertEqual(reopen.diff_share(b"abc", b"abcd"), 1.0)
        self.assertEqual(reopen.diff_share(b"", b""), 1.0)


class CountTest(unittest.TestCase):
    def test_counts_one_event_kind(self) -> None:
        events = [("present", 1), ("input", 2), ("present", 3)]
        self.assertEqual(reopen.count(events, "present"), 2)
        self.assertEqual(reopen.count(events, "input"), 1)
        self.assertEqual(reopen.count(events, "map"), 0)


if __name__ == "__main__":
    unittest.main()
