"""Pure-logic unit tests for scripts/linux/run-journey-monitor.py.

These run with plain `python3 -m pytest scripts/test_journey_monitor.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
JSON-parsing, environment-discovery, process-identity, and report-building
logic only. The live AT-SPI/niri orchestration in the script itself can only
be exercised on the reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import os
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
        self.assertTrue(hasattr(journey, "pyatspi"))


class DiscoverEnvironmentTests(unittest.TestCase):
    def test_fills_in_missing_variables_from_the_runtime_directory(self):
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

    def test_missing_wayland_display_raises(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            runtime_dir = Path(raw)
            (runtime_dir / "niri.sock").touch()
            (runtime_dir / "bus").touch()
            with self.assertRaisesRegex(journey.JourneyError, "Wayland display"):
                journey.discover_environment({}, runtime_dir)


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = journey.parse_windows('[{"id": 7, "app_id": "org.rmac.SystemMonitor"}]')
        self.assertEqual(windows, [{"id": 7, "app_id": "org.rmac.SystemMonitor"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("{}")

    def test_find_window_by_app_id(self):
        windows = [{"id": 1, "app_id": "org.rmac.SystemMonitor"}]
        found = journey.find_window_by_app_id(windows, "org.rmac.SystemMonitor")
        self.assertEqual(found["id"], 1)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))


class MarkerTests(unittest.TestCase):
    def test_unique_marker_has_expected_prefix_and_is_unique(self):
        first = journey.unique_marker()
        second = journey.unique_marker()
        self.assertTrue(first.startswith("lulo-journey-6-"))
        self.assertNotEqual(first, second)

    def test_is_our_marker_process_false_for_dead_pid(self):
        # A PID essentially guaranteed not to exist.
        self.assertFalse(journey.is_our_marker_process(2**30, "anything"))

    def test_is_our_marker_process_true_for_self_with_matching_cmdline_fragment(self):
        # This test process's own cmdline always contains "python" somewhere
        # on the reference laptop's interpreter path, and always contains
        # itself as a live PID -- a safe, real (non-Linux-only) stand-in for
        # "the marker text is actually present in /proc/<pid>/cmdline".
        pid = os.getpid()
        cmdline = journey.cmdline_of(pid)
        if not cmdline:
            self.skipTest("this platform has no /proc/<pid>/cmdline (expected on macOS)")
        fragment = cmdline.strip().split(" ")[0]
        self.assertTrue(journey.is_our_marker_process(pid, fragment))

    def test_cmdline_of_missing_pid_is_empty(self):
        self.assertEqual(journey.cmdline_of(2**30), "")


class RoleCensusTests(unittest.TestCase):
    def test_count_nodes_by_role(self):
        counts = journey.count_nodes_by_role(
            ["application", "frame", "button", "button", "entry"]
        )
        self.assertEqual(counts, {"application": 1, "frame": 1, "button": 2, "entry": 1})

    def test_has_only_chrome_true_when_no_selectable_roles_present(self):
        counts = journey.count_nodes_by_role(["application", "frame", "button", "entry"])
        self.assertTrue(journey.has_only_chrome(counts, journey.SELECTABLE_ROLES))

    def test_has_only_chrome_false_once_a_table_role_appears(self):
        counts = journey.count_nodes_by_role(["application", "frame", "table", "table row"])
        self.assertFalse(journey.has_only_chrome(counts, journey.SELECTABLE_ROLES))

    def test_selectable_roles_cover_common_list_shapes(self):
        for role in ("table", "table row", "table cell", "list item", "tree item"):
            self.assertIn(role, journey.SELECTABLE_ROLES)


class ReportShapeTests(unittest.TestCase):
    def test_overall_pass_requires_every_step(self):
        steps = [
            journey.make_step("a", True, "ok"),
            journey.make_step("b", False, "not ok"),
        ]
        report = journey.build_report(steps, [], 0)
        self.assertFalse(report["overall_pass"])

    def test_report_matches_the_documented_schema(self):
        report = journey.build_report([], [], 999)
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
        steps = [journey.make_step("quit", False, "no selectable row")]
        report = journey.build_report(steps, [], 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class ConstantTests(unittest.TestCase):
    def test_system_monitor_identity(self):
        self.assertEqual(journey.SYSTEM_MONITOR["app_id"], "org.rmac.SystemMonitor")
        self.assertTrue(journey.SYSTEM_MONITOR["exec"].startswith("/usr/bin/"))

    def test_process_menu_labels_use_the_real_ellipsis_character(self):
        # crates/rmac-app-menu/src/lib.rs's MONITOR_MENUS uses "…", not
        # three ASCII dots -- a mismatch here would silently never match any
        # live AT-SPI node.
        self.assertIn("…", journey.QUIT_PROCESS_ITEM)
        self.assertIn("…", journey.FORCE_QUIT_PROCESS_ITEM)
        self.assertNotIn("...", journey.QUIT_PROCESS_ITEM)


if __name__ == "__main__":
    unittest.main()
