"""Unit tests for the isolated first-party startup smoke support."""

from __future__ import annotations

import configparser
import importlib.util
import sys
import tempfile
import unittest
import wave
import zipfile
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).parent / "linux" / "smoke-app-launches.py"
SPEC = importlib.util.spec_from_file_location("smoke_app_launches", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
smoke = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = smoke
SPEC.loader.exec_module(smoke)


class StartupSmokeTests(unittest.TestCase):
    def test_readiness_fails_if_process_exits_during_stability_window(self):
        process = mock.Mock()
        process.poll.side_effect = [None, 17]
        with mock.patch.object(smoke.time, "monotonic", return_value=0), mock.patch.object(
            smoke.time, "sleep"
        ):
            self.assertFalse(smoke.readiness_is_stable(process, lambda: True, duration=1))

    def test_readiness_fails_if_window_disappears_during_stability_window(self):
        process = mock.Mock()
        process.poll.return_value = None
        check = mock.Mock(side_effect=[True, False])
        with mock.patch.object(smoke.time, "monotonic", return_value=0), mock.patch.object(
            smoke.time, "sleep"
        ):
            self.assertFalse(smoke.readiness_is_stable(process, check, duration=1))

    def test_specs_match_packaged_desktop_entry_points(self):
        entries = Path(__file__).resolve().parents[1] / "packaging/rmac-apps/applications"
        desktop_files = sorted(entries.glob("org.rmac.*.desktop"))
        self.assertEqual(len(desktop_files), len(smoke.APP_SPECS))
        by_binary = {spec.binary: spec for spec in smoke.APP_SPECS}
        self.assertEqual(len(by_binary), len(smoke.APP_SPECS))
        for path in desktop_files:
            with self.subTest(desktop=path.name):
                entry = configparser.ConfigParser(interpolation=None)
                entry.read(path)
                launch_binary = Path(entry["Desktop Entry"]["Exec"].split()[0]).name
                spec = by_binary[launch_binary]
                if spec.mode != "layer":
                    self.assertEqual(spec.window_app_id, entry["Desktop Entry"]["StartupWMClass"])
                else:
                    self.assertIsNone(spec.window_app_id)

    def test_inventory_covers_startup_apps_and_installed_gui_entry_points(self):
        self.assertEqual(
            {spec.app_id for spec in smoke.APP_SPECS},
            {
                "archive-utility",
                "app-drawer",
                "clock",
                "calendar",
                "mail",
                "notes",
                "player",
                "preview",
                "system-monitor",
                "terminal",
                "weather",
                "calculator",
                "system-settings",
                "text-editor",
                "files",
            },
        )

    def test_a_final_binary_can_be_rerun_without_restarting_other_apps(self):
        selected = smoke.selected_apps(["preview"])
        self.assertEqual([spec.app_id for spec in selected], ["preview"])
        self.assertEqual(smoke.selected_apps(None), smoke.APP_SPECS)

    def test_fixture_arguments_only_pass_disposable_documents_to_path_apps(self):
        fixture_dir = Path("/temporary/smoke-fixtures")
        by_id = {spec.app_id: spec for spec in smoke.APP_SPECS}
        self.assertEqual(
            smoke.fixture_arguments(by_id["archive-utility"], fixture_dir),
            ["/temporary/smoke-fixtures/smoke-archive.zip"],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["player"], fixture_dir),
            ["/temporary/smoke-fixtures/smoke-audio.wav"],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["preview"], fixture_dir),
            ["/temporary/smoke-fixtures/smoke-document.pdf"],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["app-drawer"], fixture_dir),
            ["--service", "--show"],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["calculator"], fixture_dir),
            [],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["system-settings"], fixture_dir),
            [],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["text-editor"], fixture_dir),
            [],
        )
        self.assertEqual(
            smoke.fixture_arguments(by_id["files"], fixture_dir),
            ["--path", "/temporary/smoke-fixtures"],
        )

    def test_window_readiness_pins_every_app_to_its_wayland_app_id(self):
        self.assertEqual(
            {spec.app_id: spec.window_app_id for spec in smoke.APP_SPECS if spec.window_app_id is not None},
            {
                "archive-utility": "org.rmac.ArchiveUtility",
                "clock": "org.rmac.Clock",
                "calendar": "org.rmac.Calendar",
                "mail": "org.rmac.Mail",
                "notes": "org.rmac.Notes",
                "player": "org.rmac.Player",
                "preview": "org.rmac.Preview",
                "system-monitor": "org.rmac.SystemMonitor",
                "terminal": "org.rmac.Terminal",
                "weather": "org.rmac.Weather",
                "calculator": "org.rmac.Calculator",
                "system-settings": "org.rmac.SystemSettings",
                "text-editor": "org.rmac.TextEditor",
                "files": "org.rmac.Files",
            },
        )

    def test_window_readiness_is_scoped_to_process_and_expected_app_id(self):
        sway = object.__new__(smoke.NestedSway)
        sway.tree = lambda: {
            "type": "root",
            "nodes": [
                {
                    "type": "con",
                    "pid": 41,
                    "app_id": "org.rmac.Files",
                    "nodes": [],
                    "floating_nodes": [],
                }
            ],
            "floating_nodes": [],
        }
        self.assertTrue(sway.has_window(41, "org.rmac.Files"))
        self.assertFalse(sway.has_window(41, "org.rmac.TextEditor"))
        self.assertFalse(sway.has_window(42, "org.rmac.Files"))

    def test_generated_fixtures_are_valid_and_private_run_scoped(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture_dir = root / "fixtures"
            smoke.create_fixtures(fixture_dir)

            with zipfile.ZipFile(fixture_dir / "smoke-archive.zip") as archive:
                self.assertEqual(archive.namelist(), ["smoke-extracted.txt"])
            pdf = (fixture_dir / "smoke-document.pdf").read_bytes()
            self.assertTrue(pdf.startswith(b"%PDF-1.4\n"))
            stream = b"BT /F1 12 Tf 20 50 Td (Lulo smoke fixture) Tj ET\n"
            self.assertIn(f"/Length {len(stream)}".encode(), pdf)
            self.assertIn(stream, pdf)
            with wave.open(str(fixture_dir / "smoke-audio.wav"), "rb") as audio:
                self.assertEqual((audio.getnchannels(), audio.getframerate(), audio.getnframes()), (1, 8000, 800))

            env = smoke.isolated_environment(root / "session", {})
            self.assertEqual(env["HOME"], str(root / "session" / "home"))
            self.assertEqual(env["XDG_RUNTIME_DIR"], str(root / "session" / "runtime"))
            self.assertEqual(env["GSETTINGS_BACKEND"], "memory")

    def test_live_runtime_is_refused(self):
        with self.assertRaisesRegex(RuntimeError, "live Wayland"):
            smoke.refuse_live_environment(
                {"XDG_RUNTIME_DIR": "/run/user/1000", "WAYLAND_DISPLAY": "wayland-1"}
            )

    def test_diagnostic_excerpt_is_bounded_and_redacts_home_and_temp_paths(self):
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp)
            stderr = (
                f"failed in {work}/runtime\n"
                "at /home/jacob/Documents/private-note.txt\n"
                "socket /tmp/other-private-session.sock\n"
            )
            excerpt = smoke.bounded_diagnostic(stderr, work, limit=500)
        self.assertIsNotNone(excerpt)
        self.assertNotIn("private-note", excerpt)
        self.assertNotIn("other-private-session", excerpt)
        self.assertNotIn(temp, excerpt)

    def test_empty_diagnostic_is_omitted(self):
        self.assertIsNone(smoke.bounded_diagnostic("  \n", Path("/tmp/private")))


if __name__ == "__main__":
    unittest.main()
