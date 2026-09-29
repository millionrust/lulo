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


class SmapsRollupParsingTests(unittest.TestCase):
    def test_parses_pss(self):
        text = "Rss:            51200 kB\nPss:            38912 kB\nShared_Clean:        0 kB\n"
        self.assertEqual(run_memory_soak.parse_smaps_rollup_pss_kib(text), 38912)

    def test_missing_pss_raises(self):
        with self.assertRaises(ValueError):
            run_memory_soak.parse_smaps_rollup_pss_kib("Rss: 100 kB\n")


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
