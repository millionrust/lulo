"""Pure-logic unit tests for scripts/linux/run-journey-textfile.py.

These run with plain `python3 -m pytest scripts/test_journey_textfile.py` on
macOS (no pyatspi, no niri, no live session required): they cover the
JSON-parsing, environment-discovery, hashing, and report-building logic
only. The live AT-SPI/niri/portal orchestration in the script itself can
only be exercised on the reference Linux laptop; see docs/journey-suite.md.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "run-journey-textfile.py"
SPEC = importlib.util.spec_from_file_location("run_journey_textfile", SCRIPT)
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

    def test_leaves_already_set_variables_alone(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            environ = {
                "XDG_RUNTIME_DIR": "/already/set",
                "NIRI_SOCKET": "/already/set.sock",
                "WAYLAND_DISPLAY": "wayland-9",
                "DBUS_SESSION_BUS_ADDRESS": "unix:path=/already/set/bus",
            }
            additions = journey.discover_environment(environ, Path(raw))
        self.assertEqual(additions, {})

    def test_missing_socket_raises(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            with self.assertRaisesRegex(journey.JourneyError, "niri IPC socket"):
                journey.discover_environment({}, Path(raw))


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = journey.parse_windows('[{"id": 1, "app_id": "org.rmac.TextEditor"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.TextEditor"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows('{"not": "a list"}')

    def test_parse_windows_rejects_invalid_json(self):
        with self.assertRaises(journey.JourneyError):
            journey.parse_windows("not json")

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.FileChooser"},
            {"id": 2, "app_id": "org.rmac.TextEditor"},
        ]
        found = journey.find_window_by_app_id(windows, "org.rmac.TextEditor")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(journey.find_window_by_app_id(windows, "org.rmac.Missing"))

    def test_windows_by_app_id_returns_all_matches(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.FileChooser"},
            {"id": 2, "app_id": "org.rmac.FileChooser"},
            {"id": 3, "app_id": "org.rmac.TextEditor"},
        ]
        matches = journey.windows_by_app_id(windows, "org.rmac.FileChooser")
        self.assertEqual([window["id"] for window in matches], [1, 2])


class HashingTests(unittest.TestCase):
    def test_sha256_hex_matches_hashlib(self):
        data = b"rmac journey 5 fixture"
        self.assertEqual(journey.sha256_hex(data), hashlib.sha256(data).hexdigest())

    def test_sha256_hex_distinguishes_content(self):
        self.assertNotEqual(journey.sha256_hex(b"a"), journey.sha256_hex(b"b"))


class RandomFolderNameTests(unittest.TestCase):
    def test_folder_name_has_expected_prefix_and_is_unique(self):
        first = journey.random_folder_name()
        second = journey.random_folder_name()
        self.assertTrue(first.startswith("lulo-journey-5-"))
        self.assertNotEqual(first, second)


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
        self.assertEqual(report["journey"], 5)

    def test_report_is_json_serializable_and_privacy_safe(self):
        steps = [journey.make_step("open", True, "ok")]
        report = journey.build_report(steps, [], 0)
        text = json.dumps(report)
        for forbidden in ("/home/", "screencapture", ".png", ".jpg"):
            self.assertNotIn(forbidden, text)


class ConstantTests(unittest.TestCase):
    def test_text_editor_identity(self):
        self.assertEqual(journey.TEXT_EDITOR["app_id"], "org.rmac.TextEditor")
        self.assertTrue(journey.TEXT_EDITOR["exec"].startswith("/usr/bin/"))

    def test_file_chooser_app_ids_are_distinct(self):
        self.assertNotEqual(journey.FILE_CHOOSER_OPEN_APP_ID, journey.FILE_CHOOSER_SAVE_APP_ID)

    def test_menu_labels_use_the_real_ellipsis_character(self):
        # crates/rmac-app-menu/src/lib.rs's TEXT_EDITOR_MENUS uses "…",
        # not three ASCII dots -- a mismatch here would silently never match
        # any live AT-SPI node.
        self.assertIn("…", journey.OPEN_ITEM)
        self.assertIn("…", journey.SAVE_AS_ITEM)
        self.assertNotIn("...", journey.OPEN_ITEM)

    def test_initial_and_external_content_are_distinct(self):
        self.assertNotEqual(journey.INITIAL_CONTENT, journey.EXTERNAL_CONTENT)


if __name__ == "__main__":
    unittest.main()
