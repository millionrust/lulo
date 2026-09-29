"""Pure-logic unit tests for scripts/linux/measure-budgets.py.

These run with plain `python3 -m pytest scripts/test_measure_budgets.py` on
macOS (no /proc, no niri, no live systemd --user session required): they
cover the /proc parsing, CPU/wakeup math, budget evaluation, and Markdown
rendering only. The live-system sampling and process orchestration in the
script itself can only be exercised on the reference Linux laptop; see
docs/journey-suite.md and docs/perf/.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
import wave
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import call, patch


SCRIPT = Path(__file__).parent / "linux" / "measure-budgets.py"
SPEC = importlib.util.spec_from_file_location("measure_budgets", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
measure_budgets = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = measure_budgets
SPEC.loader.exec_module(measure_budgets)


class ShellWindowTests(unittest.TestCase):
    @staticmethod
    def snapshot(ticks: int, switches: int) -> dict[str, int]:
        return {
            "cpu_ticks": ticks,
            "voluntary_ctxt_switches": switches,
            "nonvoluntary_ctxt_switches": 0,
            "pss_kib": 1024,
            "process_count": 1,
        }

    def test_shell_surfaces_share_one_idle_window(self):
        samples = {
            101: [self.snapshot(100, 10), self.snapshot(106, 16)],
            102: [self.snapshot(50, 5), self.snapshot(53, 8)],
        }
        pids = {
            "rmac-top-bar.service": 101,
            "rmac-dock.service": 102,
            "rmac-launcher.service": None,
        }

        def sample(pid, _hertz):
            return samples[pid].pop(0)

        with (
            patch.object(measure_budgets, "get_unit_main_pid", side_effect=pids.get),
            patch.object(measure_budgets, "sample_process_tree", side_effect=sample) as sampled,
            patch.object(
                measure_budgets, "sample_process_forest",
                side_effect=[self.snapshot(150, 15), self.snapshot(159, 24)],
            ) as forest,
            patch.object(measure_budgets.time, "sleep") as sleep,
            patch.object(measure_budgets.time, "monotonic", side_effect=[100.0, 100.0, 160.0, 160.0]),
        ):
            surfaces, combined = measure_budgets.measure_shell_surfaces(
                ["rmac-top-bar", "rmac-dock", "rmac-launcher"], 3, 60, 100, 0.5
            )

        sleep.assert_has_calls([call(3), call(60)])
        self.assertEqual(sleep.call_count, 2)
        self.assertEqual(sampled.call_count, 4)
        self.assertEqual(forest.call_count, 2)
        self.assertEqual(surfaces["rmac-top-bar"]["idle_cpu_percent"]["value"], 0.1)
        self.assertEqual(surfaces["rmac-dock"]["idle_cpu_percent"]["value"], 0.05)
        self.assertFalse(surfaces["rmac-launcher"]["running"])
        self.assertEqual(combined, measure_budgets.evaluate_budget(0.15, 0.5))

    def test_restarted_unit_is_not_credited_to_the_combined_sample(self):
        pids = [101, 202]
        samples = [self.snapshot(100, 10), self.snapshot(106, 16)]
        with (
            patch.object(measure_budgets, "get_unit_main_pid", side_effect=pids),
            patch.object(measure_budgets, "sample_process_tree", side_effect=samples),
            patch.object(
                measure_budgets, "sample_process_forest",
                side_effect=[self.snapshot(100, 10), self.snapshot(106, 16)],
            ),
            patch.object(measure_budgets.time, "sleep"),
            patch.object(measure_budgets.time, "monotonic", side_effect=[100.0, 100.0, 160.0, 160.0]),
        ):
            surfaces, combined = measure_budgets.measure_shell_surfaces(
                ["rmac-top-bar"], 3, 60, 100, 0.5
            )
        self.assertFalse(surfaces["rmac-top-bar"]["running"])
        self.assertIn("restarted", surfaces["rmac-top-bar"]["note"])
        self.assertIsNone(combined)

    def test_newly_started_popover_invalidates_combined_idle_sample(self):
        with (
            patch.object(
                measure_budgets, "get_unit_main_pid",
                side_effect=[101, None, 101, 202],
            ),
            patch.object(
                measure_budgets, "sample_process_tree",
                side_effect=[self.snapshot(100, 10), self.snapshot(106, 16)],
            ),
            patch.object(
                measure_budgets, "sample_process_forest",
                side_effect=[self.snapshot(100, 10), self.snapshot(106, 16)],
            ),
            patch.object(measure_budgets.time, "sleep"),
            patch.object(
                measure_budgets.time,
                "monotonic",
                side_effect=[100.0, 100.0, 160.0, 160.0],
            ),
        ):
            surfaces, combined = measure_budgets.measure_shell_surfaces(
                ["rmac-top-bar", "rmac-launcher"], 3, 60, 100, 0.5
            )
        self.assertIn("started during measurement", surfaces["rmac-launcher"]["note"])
        self.assertIsNone(combined)

    def test_combined_shell_sample_counts_a_shared_child_once(self):
        children = {101: {101, 201}, 102: {102, 201}}
        with (
            patch.object(measure_budgets, "build_descendant_set", side_effect=children.get),
            patch.object(measure_budgets, "sample_process_set", return_value=self.snapshot(30, 3)) as sampled,
        ):
            result = measure_budgets.sample_process_forest({101, 102}, 100)
        self.assertEqual(result["cpu_ticks"], 30)
        sampled.assert_called_once_with({101, 102, 201}, 100)


class AppEnvironmentTests(unittest.TestCase):
    def test_measured_app_uses_private_home_and_xdg_files_but_live_session_sockets(self):
        with TemporaryDirectory() as directory:
            app_temp = Path(directory)
            original = {
                "HOME": "/home/owner",
                "XDG_CONFIG_HOME": "/home/owner/.config",
                "XDG_RUNTIME_DIR": "/run/user/1000",
                "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
                "WAYLAND_DISPLAY": "wayland-1",
            }
            ready = app_temp / "startup.ready"
            result = measure_budgets.app_environment(original, app_temp, ready)
            self.assertEqual(original["HOME"], "/home/owner")
            self.assertEqual(result["HOME"], str(app_temp / "home-root" / "user"))
            self.assertTrue((app_temp / "home-root" / "user").is_dir())
            for key, name in (
                ("XDG_CONFIG_HOME", "config"),
                ("XDG_DATA_HOME", "data"),
                ("XDG_STATE_HOME", "state"),
                ("XDG_CACHE_HOME", "cache"),
            ):
                self.assertEqual(result[key], str(app_temp / name))
                self.assertTrue((app_temp / name).is_dir())
                self.assertFalse(Path(result[key]).is_relative_to(Path(result["HOME"])))
                self.assertFalse(Path(result[key]).is_relative_to(Path(result["HOME"]).parent))
            self.assertFalse(ready.is_relative_to(Path(result["HOME"])))
            self.assertFalse(ready.is_relative_to(Path(result["HOME"]).parent))
            self.assertEqual(result["XDG_RUNTIME_DIR"], original["XDG_RUNTIME_DIR"])
            self.assertEqual(result["DBUS_SESSION_BUS_ADDRESS"], original["DBUS_SESSION_BUS_ADDRESS"])
            self.assertEqual(result["WAYLAND_DISPLAY"], original["WAYLAND_DISPLAY"])
            self.assertEqual(result[measure_budgets.READY_FILE_ENV], str(ready))

    def test_player_launch_uses_short_silent_media_fixture(self):
        with TemporaryDirectory() as directory:
            app_temp = Path(directory)
            command = measure_budgets.launch_command(Path("/usr/bin/rmac-player"), app_temp)
            self.assertEqual(command[0], "/usr/bin/rmac-player")
            self.assertEqual(command[1], str(app_temp / "silent-player-fixture.wav"))
            with wave.open(command[1], "rb") as fixture:
                self.assertEqual(fixture.getnchannels(), 1)
                self.assertEqual(fixture.getsampwidth(), 2)
                self.assertEqual(fixture.getframerate(), 44100)
                self.assertEqual(fixture.getnframes(), 4410)
                self.assertEqual(fixture.readframes(4410), b"\0\0" * 4410)

    def test_other_app_launches_without_fixture(self):
        with TemporaryDirectory() as directory:
            self.assertEqual(
                measure_budgets.launch_command(Path("/usr/bin/rmac-weather"), Path(directory)),
                ["/usr/bin/rmac-weather"],
            )


class ParseProcStatTests(unittest.TestCase):
    def test_parses_simple_comm(self):
        fields = measure_budgets.parse_proc_stat(
            "1234 (rmac-dock) S 1 1234 1234 0 -1 4194560 100 0 0 0 55 20 0 0 20 0 4 0 "
            "12345 123456789 1234 18446744073709551615 1 1"
        )
        self.assertEqual(fields["pid"], 1234)
        self.assertEqual(fields["ppid"], 1)
        self.assertEqual(fields["pgrp"], 1234)
        self.assertEqual(fields["utime"], 55)
        self.assertEqual(fields["stime"], 20)

    def test_comm_with_spaces_and_parens_does_not_confuse_the_parser(self):
        fields = measure_budgets.parse_proc_stat(
            "42 (weird (name) proc) R 7 42 42 0 -1 0 0 0 0 0 9 3 0 0 20 0 1 0 0 0 0 0 0 0"
        )
        self.assertEqual(fields["pid"], 42)
        self.assertEqual(fields["ppid"], 7)
        self.assertEqual(fields["utime"], 9)
        self.assertEqual(fields["stime"], 3)

    def test_too_few_fields_raises(self):
        with self.assertRaises(ValueError):
            measure_budgets.parse_proc_stat("1 (short) R 1 1")


class ParseStatusCtxtSwitchesTests(unittest.TestCase):
    def test_parses_both_counters(self):
        text = (
            "Name:\trmac-top-bar\n"
            "State:\tS (sleeping)\n"
            "voluntary_ctxt_switches:\t120\n"
            "nonvoluntary_ctxt_switches:\t4\n"
        )
        self.assertEqual(measure_budgets.parse_status_ctxt_switches(text), (120, 4))

    def test_missing_fields_raise(self):
        with self.assertRaises(ValueError):
            measure_budgets.parse_status_ctxt_switches("Name:\trmac-dock\n")


class ParseSmapsRollupTests(unittest.TestCase):
    def test_parses_pss_kib(self):
        text = (
            "12340000-7fff00000000 ---p 00000000 00:00 0 [rollup]\n"
            "Rss:               81920 kB\n"
            "Pss:               45678 kB\n"
            "Pss_Anon:          40000 kB\n"
        )
        self.assertEqual(measure_budgets.parse_smaps_rollup_pss_kib(text), 45678)

    def test_missing_pss_line_raises(self):
        with self.assertRaises(ValueError):
            measure_budgets.parse_smaps_rollup_pss_kib("Rss: 100 kB\n")


class CpuPercentTests(unittest.TestCase):
    def test_zero_ticks_is_zero_percent(self):
        self.assertEqual(measure_budgets.cpu_percent_from_ticks(0, 100, 60.0), 0.0)

    def test_full_core_second_over_one_second(self):
        # 100 ticks at HZ=100 is exactly one CPU-second; over a one-second
        # window that is 100% of one core.
        self.assertAlmostEqual(measure_budgets.cpu_percent_from_ticks(100, 100, 1.0), 100.0)

    def test_matches_todo_budget_scale(self):
        # 0.3% of one core over 60 s at HZ=100 is 100*0.06*0.3/100 = 0.018 s
        # of CPU time, i.e. 1.8 ticks.
        percent = measure_budgets.cpu_percent_from_ticks(2, 100, 60.0)
        self.assertLess(percent, 0.35)

    def test_negative_delta_clamped_to_zero(self):
        # A counter can appear to go backwards only from a measurement race
        # (a process tree member being replaced); never report negative CPU.
        self.assertEqual(measure_budgets.cpu_percent_from_ticks(-5, 100, 10.0), 0.0)

    def test_rejects_non_positive_elapsed(self):
        with self.assertRaises(ValueError):
            measure_budgets.cpu_percent_from_ticks(10, 100, 0.0)


class WakeupsPerSecondTests(unittest.TestCase):
    def test_basic_rate(self):
        self.assertEqual(measure_budgets.wakeups_per_second(30, 60.0), 0.5)

    def test_negative_delta_clamped_to_zero(self):
        self.assertEqual(measure_budgets.wakeups_per_second(-2, 10.0), 0.0)


class PercentileTests(unittest.TestCase):
    def test_nearest_rank_matches_measure_baseline_convention(self):
        self.assertEqual(
            measure_budgets.percentile_nearest_rank([5, 1, 3, 2, 4], 0.95), 5
        )


class WarmLaunchBudgetTests(unittest.TestCase):
    def test_files_and_terminal_get_the_wide_budget(self):
        self.assertEqual(measure_budgets.warm_launch_budget_ms("rmac-files"), 900.0)
        self.assertEqual(measure_budgets.warm_launch_budget_ms("rmac-terminal"), 900.0)

    def test_other_apps_get_the_simple_budget(self):
        self.assertEqual(measure_budgets.warm_launch_budget_ms("rmac-notes"), 500.0)


class IdleCpuBudgetTests(unittest.TestCase):
    def test_monitor_and_shell_have_their_own_beta_limits(self):
        self.assertEqual(measure_budgets.idle_cpu_budget_for_app("rmac-system-monitor"), 2.5)
        self.assertEqual(measure_budgets.idle_cpu_budget_for_app("rmac-weather"), 0.3)
        self.assertEqual(measure_budgets.IDLE_CPU_SHELL_COMBINED_BUDGET_PERCENT, 0.5)


class EvaluateBudgetTests(unittest.TestCase):
    def test_within_budget(self):
        result = measure_budgets.evaluate_budget(0.2, 0.3)
        self.assertTrue(result["within_budget"])

    def test_over_budget(self):
        result = measure_budgets.evaluate_budget(1.5, 1.0)
        self.assertFalse(result["within_budget"])

    def test_exactly_at_budget_passes(self):
        result = measure_budgets.evaluate_budget(0.3, 0.3)
        self.assertTrue(result["within_budget"])


class SuspectedIdleRedrawTests(unittest.TestCase):
    def test_below_threshold_is_fine(self):
        self.assertFalse(measure_budgets.suspected_idle_redraw(0.1, 0.5))

    def test_above_threshold_is_flagged(self):
        self.assertTrue(measure_budgets.suspected_idle_redraw(2.0, 0.5))


class DiscoverEnvironmentTests(unittest.TestCase):
    def test_fills_in_missing_variables(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            runtime_dir = Path(raw)
            (runtime_dir / "niri.wayland-1.1234.sock").touch()
            (runtime_dir / "wayland-1").touch()
            (runtime_dir / "wayland-1.lock").touch()
            (runtime_dir / "bus").touch()

            additions = measure_budgets.discover_environment({}, runtime_dir)

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
            additions = measure_budgets.discover_environment(environ, Path(raw))

        self.assertEqual(additions, {})

    def test_missing_socket_raises(self):
        import tempfile

        with tempfile.TemporaryDirectory() as raw:
            with self.assertRaises(measure_budgets.MeasurementError):
                measure_budgets.discover_environment({}, Path(raw))


class NiriJsonParsingTests(unittest.TestCase):
    def test_parse_windows_accepts_an_array(self):
        windows = measure_budgets.parse_windows('[{"id": 1, "app_id": "org.rmac.Notes"}]')
        self.assertEqual(windows, [{"id": 1, "app_id": "org.rmac.Notes"}])

    def test_parse_windows_rejects_non_array(self):
        with self.assertRaises(measure_budgets.MeasurementError):
            measure_budgets.parse_windows('{"not": "a list"}')

    def test_find_window_by_app_id(self):
        windows = [
            {"id": 1, "app_id": "org.rmac.Notes"},
            {"id": 2, "app_id": "org.rmac.TextEditor"},
        ]
        found = measure_budgets.find_window_by_app_id(windows, "org.rmac.TextEditor")
        self.assertEqual(found["id"], 2)
        self.assertIsNone(measure_budgets.find_window_by_app_id(windows, "org.rmac.Missing"))


class MarkdownRenderingTests(unittest.TestCase):
    def _fixture_report(self) -> dict:
        return {
            "captured_at": "2026-09-24T00:00:00Z",
            "rustc_processes": 0,
            "settings": {
                "idle_seconds": 60,
                "repetitions": 5,
                "warmups": 1,
                "wakeup_threshold_per_second": 0.5,
            },
            "shell_surfaces": {
                "rmac-dock": {
                    "running": True,
                    "idle_cpu_percent": measure_budgets.evaluate_budget(0.1, 0.3),
                    "wakeups_per_second": 0.05,
                    "suspected_idle_redraw": False,
                    "pss_mib": 42.0,
                },
                "rmac-launcher": {"running": False},
            },
            "shell_combined": measure_budgets.evaluate_budget(0.1, 0.5),
            "apps": {
                "rmac-notes": {
                    "display_name": "Notes",
                    "warm_launch": {
                        "samples_ms": [100.0],
                        "median_ms": 100.0,
                        "p95_ms": measure_budgets.evaluate_budget(120.0, 500.0),
                        "interactive_marker": "ready_file",
                    },
                    "idle": {
                        "idle_cpu_percent": measure_budgets.evaluate_budget(0.1, 0.3),
                        "wakeups_per_second": 0.02,
                        "suspected_idle_redraw": False,
                        "pss_mib": 70.0,
                    },
                    "frame_timing": "not_measured",
                }
            },
        }

    def test_renders_without_error_and_includes_key_sections(self):
        text = measure_budgets.render_markdown_report(self._fixture_report())
        self.assertIn("# Reference laptop performance budgets", text)
        self.assertIn("## Shell surfaces", text)
        self.assertIn("rmac-dock", text)
        self.assertIn("rmac-launcher", text)
        self.assertIn("## Applications", text)
        self.assertIn("Notes", text)
        self.assertIn("## Over budget", text)
        self.assertIn("- none", text)

    def test_flags_over_budget_items(self):
        report = self._fixture_report()
        report["shell_surfaces"]["rmac-dock"]["idle_cpu_percent"] = measure_budgets.evaluate_budget(
            5.0, 0.3
        )
        text = measure_budgets.render_markdown_report(report)
        self.assertIn("rmac-dock: idle CPU over budget", text)
        self.assertNotIn("- none", text)

    def test_flags_app_idle_redraw(self):
        report = self._fixture_report()
        report["apps"]["rmac-notes"]["idle"]["suspected_idle_redraw"] = True
        text = measure_budgets.render_markdown_report(report)
        self.assertIn("| Wake-ups/s | Idle redraw? | PSS |", text)
        self.assertIn(
            "| Notes | 120.0 ms | <=500 ms ✓ | ready_file | 0.10% | <=0.3% ✓ | 0.020 | yes | 70.0 MiB |",
            text,
        )
        self.assertIn("Notes: suspected idle redraw", text)

    def test_not_running_surface_has_no_numbers(self):
        text = measure_budgets.render_markdown_report(self._fixture_report())
        self.assertIn("| rmac-launcher | no | n/a |", text)

    def test_no_personal_data_in_the_fixture_report(self):
        # Guard against ever slipping a hostname or home-directory path into
        # the report -- the fixture above has none, and the renderer must
        # not introduce any of its own.
        text = measure_budgets.render_markdown_report(self._fixture_report())
        self.assertNotIn("/home/", text)
        self.assertNotIn("/Users/", text)


if __name__ == "__main__":
    unittest.main()
