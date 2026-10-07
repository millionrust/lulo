"""Tests for scripts/windows/idle_gate.py, launch_smoke.py's trace parsing and
shell_smoke.py's reading of the Lulo layer's trace."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

WINDOWS = Path(__file__).resolve().parent / "windows"


def load(name: str):
    spec = importlib.util.spec_from_file_location(name, WINDOWS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


idle_gate = load("idle_gate")
launch_smoke = load("launch_smoke")
shell_smoke = load("shell_smoke")


class IdleGateTests(unittest.TestCase):
    def test_apps_within_budget_pass(self) -> None:
        results = {
            "rmac-calculator": {"idle_ticks": 0.0},
            "rmac-notes": {"idle_ticks": 1.002},
        }
        self.assertEqual(idle_gate.idle_failures(results, 1.0, ()), [])

    def test_an_app_over_budget_fails_and_names_its_wake_ups(self) -> None:
        results = {
            "rmac-clock": {
                "idle_ticks": 7.5,
                "idle_wake_sources": [{"count": 1200, "source": "vsync tick"}],
            }
        }
        failures = idle_gate.idle_failures(results, 1.0, ())
        self.assertEqual(len(failures), 1)
        self.assertIn("rmac-clock", failures[0])
        self.assertIn("1200 x vsync tick", failures[0])

    def test_terminal_is_exempt_and_a_missing_reading_fails(self) -> None:
        results = {
            "rmac-terminal": {"idle_ticks": 40.0},
            "rmac-weather": {},
        }
        failures = idle_gate.idle_failures(results, 1.0, ("rmac-terminal",))
        self.assertEqual(failures, ["rmac-weather: no idle CPU reading"])

    def test_the_lulo_layer_is_gated_like_the_apps(self) -> None:
        results = {
            "lulo-session": {"idle_ticks": 0.0},
            "lulo-shell": {"idle_ticks": 3.0},
        }
        failures = idle_gate.idle_failures(results, 1.0, ("rmac-terminal",))
        self.assertEqual(len(failures), 1)
        self.assertIn("lulo-shell", failures[0])


class TraceParsingTests(unittest.TestCase):
    def test_wake_ups_are_grouped_by_source_most_frequent_first(self) -> None:
        trace = "\n".join(
            [
                "gpui_windows wake: vsync tick at 1.0 ms",
                "gpui_windows wake: message 0x000f at 1.2 ms",
                "gpui_windows wake: vsync tick at 17.6 ms",
                "menu strip: unrelated",
            ]
        )
        self.assertEqual(
            launch_smoke.summarize_wakes(trace),
            [(2, "vsync tick"), (1, "message 0x000f")],
        )

    def test_startup_phases_keep_the_first_of_each(self) -> None:
        log = "\n".join(
            [
                "gpui_windows startup: platform_new at 12.0 ms",
                "gpui_windows startup: window_shown at 80.5 ms",
                "gpui_windows startup: window_shown at 300.0 ms",
            ]
        )
        self.assertEqual(
            launch_smoke.startup_phases(log),
            ["platform_new at 12.0 ms", "window_shown at 80.5 ms"],
        )


class ShellTraceTests(unittest.TestCase):
    def test_the_log_finds_the_latest_dock_tile_and_waits_from_a_line(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "shell.log"
            path.write_text(
                "\n".join(
                    [
                        "lulo-shell: starting pid 42 at 9 ms",
                        "gpui_windows wake: vsync tick at 1.0 ms",
                        "lulo-shell: dock tile rmac-calculator.exe Calculator at 100,700",
                        "lulo-shell: dock tile rmac-calculator.exe Calculator at 110,700 running",
                    ]
                ),
                encoding="utf-8",
            )
            log = shell_smoke.Log(path)
            self.assertEqual(log.wait_for(r"^starting pid (\d+)", 0.1).group(1), "42")
            tile = log.last(r"^dock tile rmac-calculator\.exe Calculator at (\d+),(\d+)")
            self.assertEqual(tile.group(1), "110")
            self.assertIsNone(log.wait_for(r"^starting", 0.05, after=1))
            self.assertIsNotNone(log.wait_for(r"running$", 0.1, after=2))


if __name__ == "__main__":
    unittest.main()
