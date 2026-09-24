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
from pathlib import Path


SCRIPT = Path(__file__).parent / "linux" / "measure-budgets.py"
SPEC = importlib.util.spec_from_file_location("measure_budgets", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
measure_budgets = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = measure_budgets
SPEC.loader.exec_module(measure_budgets)


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
            "shell_combined": measure_budgets.evaluate_budget(0.1, 1.0),
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
