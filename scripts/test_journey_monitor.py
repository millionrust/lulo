"""Pure-logic unit tests for scripts/linux/run-journey-monitor.py.

These run with plain `python3 -m pytest scripts/test_journey_monitor.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
disposable-process naming, JSON-parsing, and report-building logic only. The
live AT-SPI/niri orchestration in the script itself can only be exercised on
the reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-monitor.py"
SPEC = importlib.util.spec_from_file_location("run_journey_monitor", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
journey = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journey
SPEC.loader.exec_module(journey)


class ModuleImportTests(unittest.TestCase):
    def test_imports_without_pyatspi(self):
        # macOS has no pyatspi; the module must still load.
        self.assertTrue(hasattr(journey, "pyatspi"))


class DisposableProcessNamingTests(unittest.TestCase):
    def test_marker_embeds_the_token(self):
        self.assertEqual(journey.disposable_marker("ab12cd34"), "lulo-journey-6-ab12cd34")

    def test_marker_rejects_whitespace_and_empty_tokens(self):
        for bad_token in ("", "has space", "tab\ttoken", "new\nline"):
            with self.assertRaises(journey.JourneyError):
                journey.disposable_marker(bad_token)

    def test_disposable_command_never_passes_the_marker_as_a_sleep_duration(self):
        # GNU `sleep 600 marker` fails outright ("invalid time interval"), so
        # the marker must never appear as a bare argv entry after the
        # duration; it must be embedded via `exec -a`, inside the shell -c
        # string.
        command = journey.build_disposable_command("tok123")
        self.assertNotIn("lulo-journey-6-tok123", command)
        self.assertIn(journey.DISPOSABLE_DURATION_S, " ".join(command))
        self.assertIn("lulo-journey-6-tok123", " ".join(command))

    def test_disposable_command_is_deterministic(self):
        self.assertEqual(
            journey.build_disposable_command("tok"), journey.build_disposable_command("tok")
        )


class ExpectedUiStringTests(unittest.TestCase):
    def test_row_label_matches_accessibility_rs_format(self):
        # crates/activity-monitor/src/accessibility.rs's AccessibleProcessRow
        # builds its label as `"{name} (PID {pid})"`.
        self.assertEqual(journey.expected_row_label("sleep", 4242), "sleep (PID 4242)")

    def test_dialog_title_distinguishes_quit_and_force_quit(self):
        self.assertNotEqual(
            journey.expected_dialog_title(force=False), journey.expected_dialog_title(force=True)
        )
        self.assertEqual(journey.expected_dialog_title(force=True), "Force Quit Process")

    def test_confirm_button_label_distinguishes_quit_and_force_quit(self):
        self.assertEqual(journey.expected_confirm_button_label(force=False), "Quit")
        self.assertEqual(journey.expected_confirm_button_label(force=True), "Force Quit")


class RowNameMatchesPidTests(unittest.TestCase):
    def test_matches_the_full_live_accessible_name(self):
        # crates/activity-monitor/src/process_table.rs's render_tr appends
        # ", {cpu}% CPU, {mem}" after accessibility.rs's row label.
        self.assertTrue(journey.row_name_matches_pid("sleep (PID 4242), 0.1% CPU, 1.2 MB", 4242))

    def test_rejects_a_different_pid(self):
        self.assertFalse(journey.row_name_matches_pid("sleep (PID 4242), 0.1% CPU, 1.2 MB", 9999))

    def test_rejects_a_pid_that_is_only_a_substring(self):
        # 424 must not match a row actually naming PID 4242.
        self.assertFalse(journey.row_name_matches_pid("sleep (PID 4242), 0.1% CPU, 1.2 MB", 424))


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = journey.parse_windows('[{"id": 1, "app_id": "org.rmac.SystemMonitor"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.SystemMonitor"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Notes"},
            {"id": 2, "app_id": "org.rmac.SystemMonitor"},
        ]
        found = journey.find_window_by_app_id(windows, "org.rmac.SystemMonitor")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))


class ReportShapeTests(unittest.TestCase):
    def test_overall_pass_requires_every_step(self):
        steps = [
            journey.make_step("a", True, "ok"),
            journey.make_step("b", False, "not ok"),
        ]
        report = journey.build_report(steps, [], 0)
        self.assertFalse(report["overall_pass"])

    def test_overall_pass_true_when_all_steps_pass(self):
        steps = [journey.make_step("a", True, "ok")]
        report = journey.build_report(steps, [], 0)
        self.assertTrue(report["overall_pass"])

    def test_report_matches_the_documented_schema(self):
        report = journey.build_report([], [], 12345)
        self.assertEqual(
            set(report),
            {
                "format",
                "journey",
                "journey_title",
                "started_at_unix_ms",
                "steps",
                "gaps",
                "overall_pass",
            },
        )
        self.assertEqual(report["journey"], 6)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("confirm_terminates_process", True, "ok")]
        report = journey.build_report(steps, [], 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class SystemMonitorConstantTests(unittest.TestCase):
    def test_system_monitor_identity_is_well_formed(self):
        self.assertEqual(journey.SYSTEM_MONITOR["app_id"], "org.rmac.SystemMonitor")
        # Launched by desktop id (gtk-launch), never a hard-coded binary path
        # -- /usr/bin/rmac-* on the reference laptop is a stale package.
        self.assertEqual(journey.SYSTEM_MONITOR["desktop_id"], "org.rmac.SystemMonitor")


if __name__ == "__main__":
    unittest.main()


class PidAliveTests(unittest.TestCase):
    """A killed child stays in /proc as a zombie until reaped; that is not alive."""

    def _stat(self, text):
        import tempfile
        from unittest import mock

        directory = Path(tempfile.mkdtemp())
        (directory / "stat").write_text(text)
        real_path = journey.Path

        def fake_path(value):
            if str(value).startswith("/proc/4242"):
                return real_path(str(value).replace("/proc/4242", str(directory)))
            return real_path(value)

        return mock.patch.object(journey, "Path", side_effect=fake_path)

    def test_a_running_process_is_alive(self):
        with self._stat("4242 (sleep) S 1 4242 4242 0 -1\n"):
            self.assertTrue(journey.pid_alive(4242))

    def test_a_zombie_is_not_alive(self):
        with self._stat("4242 (sleep) Z 1 4242 4242 0 -1\n"):
            self.assertFalse(journey.pid_alive(4242))

    def test_a_command_name_with_parentheses_is_parsed(self):
        with self._stat("4242 (odd) name) R 1 4242 4242 0 -1\n"):
            self.assertTrue(journey.pid_alive(4242))

    def test_a_missing_process_is_not_alive(self):
        self.assertFalse(journey.pid_alive(2**22 + 12345))
