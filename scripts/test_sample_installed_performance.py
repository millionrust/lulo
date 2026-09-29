"""Focused tests for private installed-app launch preparation."""

from __future__ import annotations

import importlib.util
from contextlib import redirect_stderr
from io import StringIO
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "sample-installed-performance.py"
SPEC = importlib.util.spec_from_file_location("sample_installed_performance", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
sampler = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = sampler
SPEC.loader.exec_module(sampler)


class InstalledLaunchPreparationTests(unittest.TestCase):
    def test_thread_samples_parse_names_and_ignore_disappeared_threads(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            task = root / "42" / "task"
            running = task / "43"
            running.mkdir(parents=True)
            fields = ["S"] + ["0"] * 21
            fields[11] = "20"  # utime
            fields[12] = "5"   # stime
            fields[19] = "100"  # thread start time
            (running / "stat").write_text(f"43 (render (worker)) {' '.join(fields)}\n")
            (running / "comm").write_text("render worker\n")
            (task / "44").mkdir()  # vanished before its stat could be read

            self.assertEqual(
                sampler.thread_tick_samples({42}, root),
                {(42, 43, 100): ("render worker", 25)},
            )

    def test_thread_breakdown_orders_cpu_and_rejects_reused_ids(self):
        before = {
            (42, 43, 100): ("ui", 20),
            (42, 44, 100): ("worker", 20),
            (42, 45, 100): ("old", 20),
        }
        after = {
            (42, 43, 100): ("ui", 30),
            (42, 44, 100): ("worker", 40),
            (42, 45, 101): ("reused", 90),
        }

        self.assertEqual(
            sampler.busiest_threads(before, after, 2.0, 100),
            [
                {"pid": 42, "tid": 44, "thread_name": "worker", "cpu_percent_one_core": 10.0},
                {"pid": 42, "tid": 43, "thread_name": "ui", "cpu_percent_one_core": 5.0},
            ],
        )

    def test_duration_options_reject_non_finite_values(self):
        for option in ("--idle-seconds", "--settle-seconds", "--startup-timeout"):
            for value in ("nan", "inf"):
                with self.subTest(option=option, value=value), redirect_stderr(StringIO()), patch(
                    "sys.argv", [str(SCRIPT), "--app", "rmac-files", option, value]
                ), self.assertRaises(SystemExit) as error:
                    sampler.args_parser()
                self.assertEqual(error.exception.code, 2)

    def test_private_sampler_covers_every_packaged_startup_app(self):
        sampled = {row[1]: row[2] for row in sampler.SAMPLE_APPS}
        self.assertEqual(
            set(sampled), {spec.binary for spec in sampler.smoke.APP_SPECS}
        )
        for spec in sampler.smoke.APP_SPECS:
            if spec.window_app_id is not None:
                self.assertEqual(sampled[spec.binary], spec.window_app_id)

    def test_app_drawer_uses_supervised_show_mode_and_resolved_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "bin" / "rmac-app-drawer"
            binary.parent.mkdir()
            binary.write_text("placeholder")
            binary.chmod(0o755)
            spec = next(row for row in sampler.smoke.APP_SPECS if row.binary == binary.name)

            command = sampler.launch_command(binary, spec, root / "fixtures")

            self.assertEqual(command, [str(binary.resolve()), "--service", "--show"])

    def test_preview_receives_generated_document_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixtures = root / "fixtures"
            sampler.smoke.create_fixtures(fixtures)
            binary = root / "rmac-preview"
            binary.write_text("placeholder")
            binary.chmod(0o755)
            spec = next(row for row in sampler.smoke.APP_SPECS if row.binary == binary.name)

            command = sampler.launch_command(binary, spec, fixtures)

            self.assertEqual(command[0], str(binary.resolve()))
            self.assertEqual(command[1:], [str(fixtures / "smoke-document.pdf")])
            self.assertTrue((fixtures / "smoke-document.pdf").read_bytes().startswith(b"%PDF-1.4"))

    def test_text_editor_uses_plain_window_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "rmac-text-editor"
            binary.write_text("placeholder")
            binary.chmod(0o755)

            spec = sampler.launch_spec("org.rmac.TextEditor", binary.name)
            command = sampler.launch_command(binary, spec, root / "fixtures")

            self.assertEqual(spec.mode, "window")
            self.assertEqual(command, [str(binary.resolve())])

    def test_readiness_rejects_a_different_app_window_from_same_process(self):
        class Process:
            pid = 41

            @staticmethod
            def poll():
                return None

        class Sway:
            def has_window(self, pid, expected_app_id):
                self.last_query = (pid, expected_app_id)
                return expected_app_id == "org.rmac.Files"

        with tempfile.TemporaryDirectory() as temporary:
            sway = Sway()
            ready = Path(temporary) / "ready"
            with self.assertRaisesRegex(RuntimeError, "did not create"):
                sampler.wait_ready(
                    Process(), sway, ready, time.monotonic(), 0.03,
                    "org.rmac.TextEditor",
                )
            self.assertEqual(sway.last_query, (41, "org.rmac.TextEditor"))


if __name__ == "__main__":
    unittest.main()
