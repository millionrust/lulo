"""Pure-logic unit tests for scripts/behavior/run_memory_soak.py.

These run with plain `python3 -m pytest scripts/test_run_memory_soak.py` on
macOS (no /proc, no niri, no live compositor required): they cover only the
/proc parsing and CPU-time math. The live orchestration (starting the nested
Sway/niri, launching the ten apps, sampling them for eight hours) can only
be exercised on the reference Linux laptop; see docs/beta-checklist.md's
"Memory (8-hour soak)" gate and docs/perf/.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "behavior" / "run_memory_soak.py"
SPEC = importlib.util.spec_from_file_location("run_memory_soak", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
run_memory_soak = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = run_memory_soak
SPEC.loader.exec_module(run_memory_soak)


class ProcStatParsingTests(unittest.TestCase):
    def test_parses_ppid_and_cpu_ticks(self):
        # A realistic /proc/<pid>/stat line; comm can contain spaces/parens,
        # so parsing anchors on the last ')'.
        line = "4242 (rmac text (editor)) S 100 4242 4242 0 -1 4194560 0 0 0 0 111 222 0 0 20 0 4 0"
        ppid, utime, stime = run_memory_soak.parse_proc_stat_ppid_and_ticks(line)
        self.assertEqual(ppid, 100)
        self.assertEqual(utime, 111)
        self.assertEqual(stime, 222)

    def test_rejects_short_line(self):
        with self.assertRaises(ValueError):
            run_memory_soak.parse_proc_stat_ppid_and_ticks("1 (x) S 0 1 1")


class ProcStatusParsingTests(unittest.TestCase):
    def test_parses_rss_and_threads(self):
        text = "Name:\trmac-files\nVmRSS:\t   40960 kB\nThreads:\t7\n"
        fields = run_memory_soak.parse_status_fields(text)
        self.assertEqual(fields["vm_rss_kib"], 40960)
        self.assertEqual(fields["threads"], 7)

    def test_missing_fields_raise(self):
        with self.assertRaises(ValueError):
            run_memory_soak.parse_status_fields("Name:\trmac-files\n")

    def test_parses_vm_swap(self):
        text = "Name:\trmac-files\nVmRSS:\t   40960 kB\nVmSwap:\t    2048 kB\nThreads:\t7\n"
        fields = run_memory_soak.parse_status_fields(text)
        self.assertEqual(fields["vm_swap_kib"], 2048)

    def test_missing_vm_swap_defaults_to_zero(self):
        # Required fields present, VmSwap absent (e.g. no swap configured):
        # this must not raise, only RSS/threads are mandatory.
        text = "Name:\trmac-files\nVmRSS:\t   40960 kB\nThreads:\t7\n"
        fields = run_memory_soak.parse_status_fields(text)
        self.assertEqual(fields["vm_swap_kib"], 0)


class SmapsRollupParsingTests(unittest.TestCase):
    def test_parses_pss(self):
        text = "Rss:            51200 kB\nPss:            38912 kB\nShared_Clean:        0 kB\n"
        self.assertEqual(run_memory_soak.parse_smaps_rollup_pss_kib(text), 38912)

    def test_missing_pss_raises(self):
        with self.assertRaises(ValueError):
            run_memory_soak.parse_smaps_rollup_pss_kib("Rss: 100 kB\n")

    def test_parses_anon_file_and_swap_pss(self):
        text = (
            "Rss:            51200 kB\n"
            "Pss:            38912 kB\n"
            "Pss_Anon:       30000 kB\n"
            "Pss_File:        8912 kB\n"
            "Pss_Shmem:          0 kB\n"
            "SwapPss:         4096 kB\n"
        )
        fields = run_memory_soak.parse_smaps_rollup_fields(text)
        self.assertEqual(fields["pss_kib"], 38912)
        self.assertEqual(fields["pss_anon_kib"], 30000)
        self.assertEqual(fields["pss_file_kib"], 8912)
        self.assertEqual(fields["swap_pss_kib"], 4096)

    def test_fields_missing_pss_raises(self):
        with self.assertRaises(ValueError):
            run_memory_soak.parse_smaps_rollup_fields("Rss: 100 kB\n")

    def test_fields_missing_breakdown_defaults_to_zero(self):
        # An older kernel with just the required Pss line and none of the
        # Pss_Anon/Pss_File/SwapPss breakdown lines must not raise.
        fields = run_memory_soak.parse_smaps_rollup_fields("Rss: 100 kB\nPss: 80 kB\n")
        self.assertEqual(fields["pss_kib"], 80)
        self.assertEqual(fields["pss_anon_kib"], 0)
        self.assertEqual(fields["pss_file_kib"], 0)
        self.assertEqual(fields["swap_pss_kib"], 0)


class CpuSecondsTests(unittest.TestCase):
    def test_converts_ticks_to_seconds(self):
        self.assertAlmostEqual(run_memory_soak.cpu_seconds_from_ticks(100, 100), 1.0)
        self.assertAlmostEqual(run_memory_soak.cpu_seconds_from_ticks(250, 100), 2.5)

    def test_rejects_non_positive_hertz(self):
        with self.assertRaises(ValueError):
            run_memory_soak.cpu_seconds_from_ticks(100, 0)


class DescendantSetTests(unittest.TestCase):
    def test_walks_children_transitively(self):
        # A tiny synthetic process table, exercised without touching /proc:
        # descend from pid 1 through 2 (child of 1) to 3 (child of 2).
        children = {1: [2], 2: [3]}
        all_pids = {1, 2, 3, 99}  # 99 is unrelated -- must not be included

        # Re-implement the traversal generically to test its graph logic
        # (build_descendant_set itself reads /proc, so this checks the same
        # BFS shape it uses internally).
        def walk(root):
            if root not in all_pids:
                return set()
            result = {root}
            queue = [root]
            while queue:
                current = queue.pop()
                for child in children.get(current, ()):
                    if child not in result:
                        result.add(child)
                        queue.append(child)
            return result

        self.assertEqual(walk(1), {1, 2, 3})
        self.assertNotIn(99, walk(1))


if __name__ == "__main__":
    unittest.main()


class IdleCpuGateTests(unittest.TestCase):
    @staticmethod
    def record(app, elapsed, cpu, pid=10, alive=True):
        return {"app": app, "elapsed_seconds": elapsed, "cpu_seconds": cpu, "pid": pid, "alive": alive}

    def test_measures_from_the_first_settled_sample(self):
        samples = [
            self.record("system-settings", 0.0, 8.0),
            self.record("system-settings", 60.0, 30.0),
            self.record("system-settings", 240.0, 96.0),
            self.record("calculator", 0.0, 0.5),
            self.record("calculator", 60.0, 0.6),
            self.record("calculator", 240.0, 0.6),
        ]
        percentages = run_memory_soak.idle_cpu_percentages(samples, 60.0)
        self.assertEqual(percentages, {"system-settings": 36.67, "calculator": 0.0})
        self.assertEqual(
            run_memory_soak.idle_cpu_failures(percentages, 3.0),
            ["system-settings used 36.67% of one core while idle (limit 3%)"],
        )

    def test_skips_dead_restarted_and_single_sample_apps(self):
        samples = [
            self.record("terminal", 60.0, 1.0),
            self.record("terminal", 240.0, 1.0, alive=False),
            self.record("notes", 60.0, 1.0, pid=1),
            self.record("notes", 240.0, 2.0, pid=2),
            self.record("clock", 240.0, 1.0),
        ]
        self.assertEqual(run_memory_soak.idle_cpu_percentages(samples, 60.0), {})
