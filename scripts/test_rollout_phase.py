"""Unit tests for rollout.yml's next-Phased-Update-Percentage decision."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).parent / "linux" / "next-rollout-phase.py"
spec = importlib.util.spec_from_file_location("next_rollout_phase", SCRIPT)
assert spec is not None and spec.loader is not None
rollout = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = rollout
spec.loader.exec_module(rollout)

DAY = 24 * 3600


class NextPhaseTests(unittest.TestCase):
    def test_advances_after_24_hours(self):
        self.assertEqual(
            rollout.next_phase(
                current_phase=10,
                current_date_seconds=0,
                now_seconds=DAY,
                requested_phase=None,
            ),
            25,
        )

    def test_does_not_advance_before_24_hours(self):
        self.assertIsNone(
            rollout.next_phase(
                current_phase=10,
                current_date_seconds=0,
                now_seconds=DAY - 1,
                requested_phase=None,
            )
        )

    def test_full_progression(self):
        for current, expected in ((10, 25), (25, 50), (50, 100)):
            with self.subTest(current=current):
                self.assertEqual(
                    rollout.next_phase(
                        current_phase=current,
                        current_date_seconds=0,
                        now_seconds=DAY,
                        requested_phase=None,
                    ),
                    expected,
                )

    def test_never_advances_past_100(self):
        self.assertIsNone(
            rollout.next_phase(
                current_phase=100,
                current_date_seconds=0,
                now_seconds=100 * DAY,
                requested_phase=None,
            )
        )

    def test_scheduled_tick_never_resumes_a_halt(self):
        self.assertIsNone(
            rollout.next_phase(
                current_phase=0,
                current_date_seconds=0,
                now_seconds=100 * DAY,
                requested_phase=None,
            )
        )

    def test_manual_dispatch_can_halt_immediately_at_any_time(self):
        self.assertEqual(
            rollout.next_phase(
                current_phase=25,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=0,
            ),
            0,
        )

    def test_manual_dispatch_can_resume_from_a_halt(self):
        self.assertEqual(
            rollout.next_phase(
                current_phase=0,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=10,
            ),
            10,
        )

    def test_manual_dispatch_can_jump_forward_without_waiting(self):
        self.assertEqual(
            rollout.next_phase(
                current_phase=10,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=100,
            ),
            100,
        )

    def test_manual_dispatch_cannot_move_backward_except_to_zero(self):
        with self.assertRaises(rollout.RolloutError):
            rollout.next_phase(
                current_phase=50,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=25,
            )

    def test_rejects_an_unrecognized_current_phase(self):
        with self.assertRaises(rollout.RolloutError):
            rollout.next_phase(
                current_phase=42,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=None,
            )

    def test_rejects_an_unrecognized_requested_phase(self):
        with self.assertRaises(rollout.RolloutError):
            rollout.next_phase(
                current_phase=10,
                current_date_seconds=0,
                now_seconds=1,
                requested_phase=42,
            )

    def test_rejects_now_before_the_live_date(self):
        with self.assertRaises(rollout.RolloutError):
            rollout.next_phase(
                current_phase=10,
                current_date_seconds=100,
                now_seconds=0,
                requested_phase=None,
            )

    def test_cli_prints_no_step_when_nothing_to_do(self):
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--current-phase",
                "10",
                "--current-date-seconds",
                "0",
                "--now-seconds",
                "1",
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "no-step")

    def test_cli_prints_the_next_phase(self):
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--current-phase",
                "10",
                "--current-date-seconds",
                "0",
                "--now-seconds",
                str(DAY),
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "25")


if __name__ == "__main__":
    unittest.main()
