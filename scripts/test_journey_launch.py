"""Pure-logic unit tests for scripts/linux/run-journey-launch.py.

These run with plain `python3 -m pytest scripts/test_journey_launch.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
JSON-parsing, environment-discovery, and report-building logic only. The
live AT-SPI/niri orchestration in the script itself can only be exercised on
the reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-launch.py"
SPEC = importlib.util.spec_from_file_location("run_journey_launch", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
journey = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journey
SPEC.loader.exec_module(journey)


class ModuleImportTests(unittest.TestCase):
    def test_imports_without_pyatspi(self):
        # macOS has no pyatspi; the module must still load (the try/except
        # ImportError guard at the top of the script is what this proves).
        self.assertTrue(hasattr(journey, "pyatspi"))


class DiscoverEnvironmentTests(unittest.TestCase):
    def test_fills_in_missing_variables_from_the_runtime_directory(self, tmp_path=None):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            runtime_dir = Path(raw)
            (runtime_dir / "niri.wayland-1.1234.sock").touch()
            (runtime_dir / "wayland-1").touch()
            (runtime_dir / "wayland-1.lock").touch()
            (runtime_dir / "bus").touch()

            additions = journey.discover_environment({}, runtime_dir)

        self.assertEqual(additions["XDG_RUNTIME_DIR"], str(runtime_dir))
        self.assertTrue(additions["NIRI_SOCKET"].endswith("niri.wayland-1.1234.sock"))
        self.assertEqual(additions["WAYLAND_DISPLAY"], "wayland-1")
        self.assertTrue(additions["DBUS_SESSION_BUS_ADDRESS"].startswith("unix:path="))

    def test_never_excludes_the_wayland_lock_file(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            runtime_dir = Path(raw)
            (runtime_dir / "niri.sock").touch()
            (runtime_dir / "wayland-1.lock").touch()
            (runtime_dir / "bus").touch()

            with self.assertRaises(journey.JourneyError):
                journey.discover_environment({}, runtime_dir)

    def test_leaves_already_set_variables_alone(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            runtime_dir = Path(raw)
            environ = {
                "XDG_RUNTIME_DIR": "/already/set",
                "NIRI_SOCKET": "/already/set.sock",
                "WAYLAND_DISPLAY": "wayland-9",
                "DBUS_SESSION_BUS_ADDRESS": "unix:path=/already/set/bus",
            }

            additions = journey.discover_environment(environ, runtime_dir)

        self.assertEqual(additions, {})

    def test_missing_socket_raises(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            with self.assertRaisesRegex(journey.JourneyError, "niri IPC socket"):
                journey.discover_environment({}, Path(raw))


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = journey.parse_windows(
            '[{"id": 1, "app_id": "org.rmac.TextEditor"}]'
        )
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.TextEditor"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_parse_focused_window_null_is_none(self):
        self.assertIsNone(journey.parse_focused_window("null"))

    def test_parse_focused_window_object(self):
        window = journey.parse_focused_window('{"id": 4, "app_id": "org.rmac.Notes"}')
        self.assertEqual(window, {"id": 4, "app_id": "org.rmac.Notes"})

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Notes"},
            {"id": 2, "app_id": "org.rmac.TextEditor"},
        ]
        found = journey.find_window_by_app_id(windows, "org.rmac.TextEditor")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))


class LaunchPerformanceTests(unittest.TestCase):
    def test_within_budget_when_interactive_is_fast(self):
        result = journey.evaluate_launch_performance(320.0, 250.0, 500.0)
        self.assertTrue(result["within_budget"])
        self.assertEqual(result["budget_ms"], 500.0)
        self.assertEqual(result["mapped_ms"], 320.0)
        self.assertEqual(result["interactive_ms"], 250.0)

    def test_over_budget_is_reported_not_raised(self):
        result = journey.evaluate_launch_performance(920.0, 900.0, 500.0)
        self.assertFalse(result["within_budget"])
        self.assertEqual(result["interactive_ms"], 900.0)

    def test_verdict_uses_interactive_even_when_mapped_alone_would_pass(self):
        # A window that appeared quickly but is still showing a loading
        # placeholder has not "launched" from a user's perspective -- the
        # budget verdict must follow interactive_ms, not the earlier mapped
        # signal.
        result = journey.evaluate_launch_performance(100.0, 900.0, 500.0)
        self.assertFalse(result["within_budget"])

    def test_falls_back_to_mapped_when_interactive_is_unavailable(self):
        # An accessible Dock/Spotlight launch cannot be instrumented with the
        # ready-file env var, so interactive_ms is None; the verdict must
        # still be reported against the one signal this script could get.
        result = journey.evaluate_launch_performance(250.0, None, 500.0)
        self.assertIsNone(result["interactive_ms"])
        self.assertTrue(result["within_budget"])

        slow = journey.evaluate_launch_performance(900.0, None, 500.0)
        self.assertFalse(slow["within_budget"])


class ReadyFileSpawnCommandTests(unittest.TestCase):
    def test_wraps_the_executable_with_env_and_the_marker_path(self):
        command = journey.ready_file_spawn_command(
            "/usr/bin/rmac-notes", Path("/tmp/example.ready")
        )
        self.assertEqual(
            command,
            ["env", "RMAC_BENCHMARK_READY_FILE=/tmp/example.ready", "/usr/bin/rmac-notes"],
        )

    def test_uses_the_same_env_var_rmac_ui_reads(self):
        self.assertEqual(journey.BENCHMARK_READY_FILE_ENV, "RMAC_BENCHMARK_READY_FILE")


class InteractiveReadinessStepTests(unittest.TestCase):
    def test_missing_ready_file_is_not_a_failure(self):
        # No ready_file means the app was launched through a real
        # Dock/Spotlight action this script cannot instrument -- that is a
        # good outcome, not something to fail the journey over.
        step = journey.interactive_readiness_step("content_ready", None, None)
        self.assertTrue(step["passed"])

    def test_timed_out_ready_file_is_a_failure(self):
        step = journey.interactive_readiness_step(
            "content_ready", Path("/tmp/example.ready"), None
        )
        self.assertFalse(step["passed"])

    def test_observed_marker_passes_and_reports_the_elapsed_time(self):
        step = journey.interactive_readiness_step(
            "content_ready", Path("/tmp/example.ready"), 123.4
        )
        self.assertTrue(step["passed"])
        self.assertIn("123", step["detail"])


class ReportShapeTests(unittest.TestCase):
    def test_overall_pass_requires_every_step(self):
        steps = [
            journey.make_step("a", True, "ok"),
            journey.make_step("b", False, "not ok"),
        ]
        report = journey.build_report(steps, [], {}, 0)
        self.assertFalse(report["overall_pass"])

    def test_overall_pass_true_when_all_steps_pass(self):
        steps = [journey.make_step("a", True, "ok")]
        report = journey.build_report(steps, [], {}, 0)
        self.assertTrue(report["overall_pass"])

    def test_report_matches_the_documented_schema(self):
        report = journey.build_report([], [], {}, 12345)
        self.assertEqual(
            set(report),
            {
                "format",
                "journey",
                "journey_title",
                "started_at_unix_ms",
                "steps",
                "performance",
                "gaps",
                "overall_pass",
            },
        )
        self.assertEqual(report["journey"], 1)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("close", True, "ok", app_id="org.rmac.Notes")]
        report = journey.build_report(steps, [], {}, 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class AppConstantTests(unittest.TestCase):
    def test_first_and_second_apps_are_distinct_simple_apps(self):
        self.assertNotEqual(journey.FIRST_APP["app_id"], journey.SECOND_APP["app_id"])
        for app in (journey.FIRST_APP, journey.SECOND_APP):
            self.assertTrue(app["app_id"].startswith("org.rmac."))
            self.assertTrue(app["exec"].startswith("/usr/bin/"))


if __name__ == "__main__":
    unittest.main()


class ResolveAppExecTests(unittest.TestCase):
    def test_user_desktop_entry_wins_over_the_packaged_path(self):
        with tempfile.TemporaryDirectory() as home:
            apps = Path(home) / ".local/share/applications"
            apps.mkdir(parents=True)
            (apps / "org.rmac.TextEditor.desktop").write_text(
                "[Desktop Entry]\nName=Text Editor\nExec=/home/u/.local/libexec/rmac/rmac-text-editor %F\n"
                "[Desktop Action new]\nExec=/other --new-document\n",
                encoding="utf-8",
            )
            app = {"app_id": "org.rmac.TextEditor", "exec": "/usr/bin/rmac-text-editor"}
            self.assertEqual(
                journey.resolve_app_exec(app, {"XDG_DATA_DIRS": "/nonexistent"}, home),
                "/home/u/.local/libexec/rmac/rmac-text-editor",
            )

    def test_falls_back_to_the_packaged_path_without_an_entry(self):
        with tempfile.TemporaryDirectory() as home:
            app = {"app_id": "org.rmac.Notes", "exec": "/usr/bin/rmac-notes"}
            self.assertEqual(
                journey.resolve_app_exec(app, {"XDG_DATA_DIRS": "/nonexistent"}, home),
                "/usr/bin/rmac-notes",
            )
