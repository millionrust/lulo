"""Unit tests for the isolated first-party startup smoke support."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
import wave
import zipfile
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "smoke-app-launches.py"
SPEC = importlib.util.spec_from_file_location("smoke_app_launches", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
smoke = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = smoke
SPEC.loader.exec_module(smoke)


class StartupSmokeTests(unittest.TestCase):
    def test_inventory_covers_all_nine_apps_without_behavior_scenarios(self):
        self.assertEqual(
            {spec.app_id for spec in smoke.APP_SPECS},
            {
                "archive-utility",
                "app-drawer",
                "clock",
                "notes",
                "player",
                "preview",
                "system-monitor",
                "terminal",
                "weather",
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
