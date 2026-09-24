"""Pure-logic unit tests for scripts/linux/run-journey-notes.py.

These run with plain `python3 -m pytest scripts/test_journey_notes.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
JSON-parsing, environment-discovery, structural-classification, and
report-building logic only. The live AT-SPI/niri orchestration in the
script itself can only be exercised on the reference Linux laptop; see
docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-notes.py"
SPEC = importlib.util.spec_from_file_location("run_journey_notes", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
journey = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = journey
SPEC.loader.exec_module(journey)


class ModuleImportTests(unittest.TestCase):
    def test_imports_without_pyatspi(self):
        self.assertTrue(hasattr(journey, "pyatspi"))

    def test_journey_identity(self):
        self.assertEqual(journey.JOURNEY_ID, 4)
        self.assertIn("note", journey.JOURNEY_TITLE.lower())
        self.assertIn("crash", journey.JOURNEY_TITLE.lower())


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
        windows = journey.parse_windows('[{"id": 1, "app_id": "org.rmac.Notes"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.Notes"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Terminal"},
            {"id": 2, "app_id": "org.rmac.Notes"},
        ]
        found = journey.find_window_by_app_id(windows, "org.rmac.Notes")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))


class BudgetTests(unittest.TestCase):
    def test_within_budget(self):
        result = journey.evaluate_budget(250.0, journey.SIMPLE_APP_BUDGET_MS)
        self.assertTrue(result["within_budget"])
        self.assertEqual(result["budget_ms"], 500.0)

    def test_over_budget_is_reported_not_raised(self):
        result = journey.evaluate_budget(900.0, journey.SIMPLE_APP_BUDGET_MS)
        self.assertFalse(result["within_budget"])


class ClassifyNoteSurfaceTests(unittest.TestCase):
    def test_flat_toolbar_and_entries_has_no_list_surface(self):
        # This is exactly the live-observed shape of a fresh rmac-notes
        # window: 16 button nodes (12 clickable + 2 inert + 2 named) and 4
        # entries, no other role at all.
        nodes = (
            [{"role": "button", "name": "", "has_click": False}]
            + [{"role": "button", "name": "", "has_click": True} for _ in range(12)]
            + [{"role": "button", "name": "", "has_click": False}]
            + [{"role": "button", "name": "Edit", "has_click": True}]
            + [{"role": "button", "name": "Preview", "has_click": True}]
            + [{"role": "entry", "name": "", "has_click": False} for _ in range(4)]
        )
        counts = journey.classify_note_surface(nodes)
        self.assertEqual(counts["entry"], 4)
        self.assertEqual(counts["button_named"], 2)
        self.assertEqual(counts["button_unnamed_clickable"], 12)
        self.assertEqual(counts["button_unnamed_inert"], 2)
        self.assertFalse(journey.has_list_shaped_surface(counts))

    def test_application_and_frame_root_nodes_are_not_mistaken_for_a_list(self):
        # Regression test: a live run once reported a false-positive list
        # surface because the snapshot always includes the app's own root
        # "application" and "frame" nodes, which used to fall through to
        # "other" (see run-journey-notes.py's classify_note_surface).
        nodes = [
            {"role": "application", "name": "rmac-notes", "has_click": False},
            {"role": "frame", "name": "Notes", "has_click": False},
            {"role": "button", "name": "", "has_click": True},
            {"role": "entry", "name": "", "has_click": False},
        ]
        counts = journey.classify_note_surface(nodes)
        self.assertEqual(counts["other"], 0)
        self.assertFalse(journey.has_list_shaped_surface(counts))

    def test_a_list_or_row_role_is_detected_as_a_list_surface(self):
        nodes = [
            {"role": "button", "name": "", "has_click": True},
            {"role": "list item", "name": "", "has_click": True},
        ]
        counts = journey.classify_note_surface(nodes)
        self.assertTrue(journey.has_list_shaped_surface(counts))

    def test_empty_surface_has_no_list(self):
        self.assertFalse(journey.has_list_shaped_surface(journey.classify_note_surface([])))


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
        self.assertEqual(report["journey"], 4)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("close", True, "ok", app_id="org.rmac.Notes")]
        report = journey.build_report(steps, [], {}, 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class AppConstantTests(unittest.TestCase):
    def test_app_identity(self):
        self.assertEqual(journey.APP["app_id"], "org.rmac.Notes")
        self.assertEqual(journey.APP["exec"], "/usr/bin/rmac-notes")
        self.assertEqual(journey.APP["atspi_app_name"], "rmac-notes")

    def test_notes_menus_are_well_formed(self):
        self.assertTrue(journey.NOTES_MENUS)
        for label, items in journey.NOTES_MENUS:
            self.assertTrue(label.endswith(" menu"))
            self.assertTrue(items)


if __name__ == "__main__":
    unittest.main()
