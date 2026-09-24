"""Pure-logic unit tests for scripts/linux/run-journey-files.py.

These run with plain `python3 -m pytest scripts/test_journey_files.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
JSON-parsing, environment-discovery, trash-matching, and report-building
logic only. The live AT-SPI/niri orchestration in the script itself can only
be exercised on the reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-files.py"
SPEC = importlib.util.spec_from_file_location("run_journey_files", SCRIPT)
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
        windows = journey.parse_windows('[{"id": 1, "app_id": "org.rmac.Files"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.Files"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_find_window_by_title_matches_app_id_and_title_substring(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Files", "title": "copy-me — Files"},
            {"id": 2, "app_id": "org.rmac.Files", "title": "copy-dest — Files"},
            {"id": 3, "app_id": "org.rmac.TextEditor", "title": "copy-me — Text Editor"},
        ]
        found = journey.find_window_by_title(windows, "org.rmac.Files", "copy-me")
        self.assertEqual(found["id"], 1)
        self.assertIsNone(
            journey.find_window_by_title(windows, "org.rmac.Files", "no-such-folder")
        )

    def test_find_window_by_title_never_matches_the_wrong_app_id(self):
        windows = [{"id": 3, "app_id": "org.rmac.TextEditor", "title": "copy-me — Text Editor"}]
        self.assertIsNone(
            journey.find_window_by_title(windows, "org.rmac.Files", "copy-me")
        )

    def test_find_window_by_title_tolerates_a_missing_title(self):
        windows = [{"id": 1, "app_id": "org.rmac.Files"}]
        self.assertIsNone(journey.find_window_by_title(windows, "org.rmac.Files", "copy-me"))


class TrashMatchingTests(unittest.TestCase):
    def test_matches_a_trashinfo_path_line_containing_the_marker(self):
        text = (
            "[Trash Info]\n"
            "Path=/home/jacob/Documents/lulo-journey-2-abc123/trash-me/journey-note.txt\n"
            "DeletionDate=2026-09-24T08:00:00\n"
        )
        self.assertTrue(
            journey.relative_trashinfo_path_matches(text, "lulo-journey-2-abc123")
        )

    def test_does_not_match_an_unrelated_trashinfo(self):
        text = "[Trash Info]\nPath=/home/jacob/Downloads/unrelated.pdf\n"
        self.assertFalse(
            journey.relative_trashinfo_path_matches(text, "lulo-journey-2-abc123")
        )

    def test_ignores_a_marker_appearing_only_outside_the_path_line(self):
        text = (
            "[Trash Info]\n"
            "Path=/home/jacob/Downloads/unrelated.pdf\n"
            "# lulo-journey-2-abc123 mentioned only in a comment\n"
        )
        self.assertFalse(
            journey.relative_trashinfo_path_matches(text, "lulo-journey-2-abc123")
        )


class ReportShapeTests(unittest.TestCase):
    def test_overall_pass_requires_every_step(self):
        steps = [
            journey.make_step("a", True, "ok"),
            journey.make_step("b", False, "not ok"),
        ]
        report = journey.build_report(steps, [], 0, "lulo-journey-2-abc")
        self.assertFalse(report["overall_pass"])

    def test_overall_pass_true_when_all_steps_pass(self):
        steps = [journey.make_step("a", True, "ok")]
        report = journey.build_report(steps, [], 0, "lulo-journey-2-abc")
        self.assertTrue(report["overall_pass"])

    def test_report_matches_the_documented_schema(self):
        report = journey.build_report([], [], 12345, "lulo-journey-2-abc")
        self.assertEqual(
            set(report),
            {
                "format",
                "journey",
                "journey_title",
                "started_at_unix_ms",
                "test_root",
                "steps",
                "gaps",
                "overall_pass",
            },
        )
        self.assertEqual(report["journey"], 2)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("trash", True, "ok", folder="trash-me")]
        report = journey.build_report(steps, [], 0, "lulo-journey-2-abc123")
        text = json.dumps(report)
        for forbidden in ("/home/", "/Users/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class MakeStepTests(unittest.TestCase):
    def test_extra_fields_are_preserved(self):
        step = journey.make_step("close_window", True, "closed", folder="trash-me", method="fallback")
        self.assertEqual(step["folder"], "trash-me")
        self.assertEqual(step["method"], "fallback")


if __name__ == "__main__":
    unittest.main()
