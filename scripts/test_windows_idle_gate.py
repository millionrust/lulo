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

    def test_one_minute_tick_fits_clocks_budget_once_the_rasteriser_is_left_out(self) -> None:
        # A World Clock minute boundary inside the window: one frame, about
        # nine ticks of it in WARP, the runner's software GPU.
        results = {"rmac-clock": {"idle_ticks": 10.4, "idle_renderer_ticks": 8.9}}
        self.assertEqual(
            idle_gate.idle_failures(
                results, 1.0, (), idle_gate.PER_APP_BUDGET_TICKS
            ),
            [],
        )

    def test_the_old_minute_tick_no_longer_fits(self) -> None:
        # Runs 37633070594 / 37657567719 before op/win-settings: 19 and 28
        # ticks for one tick, most of it re-rasterising the map on the CPU.
        results = {"rmac-clock": {"idle_ticks": 28.05, "idle_renderer_ticks": 9.0}}
        failures = idle_gate.idle_failures(
            results, 1.0, (), idle_gate.PER_APP_BUDGET_TICKS
        )
        self.assertEqual(len(failures), 1)

    def test_rasteriser_time_does_not_hide_an_apps_own_cpu(self) -> None:
        results = {"rmac-notes": {"idle_ticks": 12.0, "idle_renderer_ticks": 9.0}}
        failures = idle_gate.idle_failures(results, 1.0, ())
        self.assertEqual(failures, ["rmac-notes: 3.00 ticks idle over the budget of 1"])

    def test_a_world_clock_redraw_is_gated_on_clocks_own_cost(self) -> None:
        cheap = {"rmac-clock": {"world_tick": {"own_ticks_per_redraw": 1.1, "ticks_per_redraw": 9.9}}}
        self.assertEqual(idle_gate.world_tick_failures(cheap, 2.0), [])
        dear = {"rmac-clock": {"world_tick": {"own_ticks_per_redraw": 6.4, "ticks_per_redraw": 15.8}}}
        self.assertEqual(len(idle_gate.world_tick_failures(dear, 2.0)), 1)
        self.assertEqual(idle_gate.world_tick_failures(dear, None), [])
        self.assertEqual(idle_gate.world_tick_failures({}, 2.0), [])

    def test_the_rasteriser_is_the_thread_pool_less_the_apps_own_tasks(self) -> None:
        threads = [
            ("unnamed (4) [ntdll.dll]", 8.0),
            ("unnamed (5) [ntdll.dll]", 3.0),
            ("main (1) [rmac-clock.exe]", 0.5),
        ]
        tick = launch_smoke.TICK_100NS
        trace = "\n".join(
            [
                f"gpui_windows cpu: pool crates\\clock\\src\\view.rs:9:1 {tick} at 10.0 ms",
                f"gpui_windows cpu: timer crates\\clock\\src\\view.rs:3:2 {tick} at 12.0 ms",
                "gpui_windows wake: pool crates\\clock\\src\\view.rs:9:1 at 10.0 ms",
            ]
        )
        self.assertAlmostEqual(launch_smoke.pool_task_ticks(trace), 2.0)
        self.assertAlmostEqual(launch_smoke.software_renderer_ticks(threads, trace), 9.0)

    def test_clocks_per_app_budget_still_catches_a_real_regression(self) -> None:
        results = {
            "rmac-clock": {
                "idle_ticks": 400.0,
                "idle_wake_sources": [{"count": 1200, "source": "vsync tick"}],
            }
        }
        failures = idle_gate.idle_failures(
            results, 1.0, (), idle_gate.PER_APP_BUDGET_TICKS
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("rmac-clock", failures[0])

    def test_the_lulo_layer_is_gated_like_the_apps(self) -> None:
        results = {
            "lulo-session": {"idle_ticks": 0.0},
            "lulo-shell": {"idle_ticks": 3.0},
        }
        failures = idle_gate.idle_failures(results, 1.0, ("rmac-terminal",))
        self.assertEqual(len(failures), 1)
        self.assertIn("lulo-shell", failures[0])

    def test_the_lulo_bars_minute_clock_fits_its_budget(self) -> None:
        # Run 37713970007: the bar's once-a-minute clock update.
        results = {"lulo-shell": {"idle_ticks": 2.0, "idle_renderer_ticks": 0.0}}
        self.assertEqual(
            idle_gate.idle_failures(results, 1.0, (), idle_gate.PER_APP_BUDGET_TICKS),
            [],
        )


class MissingReadingTests(unittest.TestCase):
    def test_an_expected_app_that_never_ran_fails(self) -> None:
        results = {"rmac-calculator": {"idle_ticks": 0.0}}
        self.assertEqual(
            idle_gate.missing_failures(results, ["rmac-calculator", "lulo-shell"]),
            ["lulo-shell: not measured (it did not run)"],
        )

    def test_a_missing_results_file_fails(self) -> None:
        import sys
        import tempfile
        from unittest import mock

        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "windows-results.json"
            with mock.patch.object(sys, "argv", ["idle_gate.py", str(missing)]):
                self.assertEqual(idle_gate.main(), 1)


class ShellMemoryGateTests(unittest.TestCase):
    def test_the_shell_within_its_budget_passes(self) -> None:
        results = {"lulo-shell": {"idle_working_set_mb": 41.5, "idle_private_mb": 30.0}}
        self.assertEqual(idle_gate.memory_failures(results, 60.0), [])

    def test_the_shell_over_its_budget_fails(self) -> None:
        results = {"lulo-shell": {"idle_working_set_mb": 114.0, "idle_private_mb": 80.0}}
        failures = idle_gate.memory_failures(results, 60.0)
        self.assertEqual(len(failures), 1)
        self.assertIn("114.0 MB", failures[0])

    def test_a_missing_reading_fails_only_when_the_shell_ran(self) -> None:
        self.assertEqual(idle_gate.memory_failures({}, 60.0), [])
        self.assertEqual(
            idle_gate.memory_failures({"lulo-shell": {"idle_ticks": 0.0}}, 60.0),
            ["lulo-shell: no idle memory reading"],
        )

    def test_memory_given_back_after_use_passes(self) -> None:
        results = {"lulo-shell": {"idle_private_mb": 38.0, "after_use_private_mb": 40.5}}
        self.assertEqual(idle_gate.after_use_failures(results, 50.0, 5.0), [])

    def test_memory_kept_by_closed_panels_fails(self) -> None:
        # The owner's PC: 49 MB at start, 90 MB after one use of Spotlight
        # and the menus (WIN-OS-43).
        results = {"lulo-shell": {"idle_private_mb": 49.0, "after_use_private_mb": 90.0}}
        failures = idle_gate.after_use_failures(results, 50.0, 5.0)
        self.assertEqual(len(failures), 2)
        self.assertIn("over the budget of 50 MB", failures[0])
        self.assertIn("41.0 MB above", failures[1])

    def test_growth_is_measured_from_the_first_use(self) -> None:
        # CI run 37750709076: 61.1 MB idle, 73.6 after the first use (the
        # catalogue loads once), 70.6 after the second: nothing kept per use.
        results = {
            "lulo-shell": {
                "idle_private_mb": 61.1,
                "after_spotlight_private_mb": 73.6,
                "after_use_private_mb": 70.6,
            }
        }
        self.assertEqual(idle_gate.after_use_failures(results, 80.0, 4.0), [])
        results["lulo-shell"]["after_use_private_mb"] = 79.0
        failures = idle_gate.after_use_failures(results, 80.0, 4.0)
        self.assertEqual(len(failures), 1)
        self.assertIn("after the first use", failures[0])

    def test_the_after_use_gate_needs_a_reading_once_asked_for(self) -> None:
        self.assertEqual(idle_gate.after_use_failures({}, 50.0, 5.0), [])
        self.assertEqual(idle_gate.after_use_failures({"lulo-shell": {}}, None, None), [])
        self.assertEqual(
            idle_gate.after_use_failures({"lulo-shell": {"idle_private_mb": 30.0}}, 50.0, None),
            ["lulo-shell: no after-use memory reading"],
        )


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
