"""Unit tests for the dependency-free baseline harness."""

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("measure-baseline.py")
SPEC = importlib.util.spec_from_file_location("measure_baseline", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
measure_baseline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(measure_baseline)


class CpuSecondsTests(unittest.TestCase):
    def test_parses_macos_minutes_and_fractional_seconds(self):
        self.assertEqual(measure_baseline.cpu_seconds("0:00.03"), 0.03)

    def test_parses_hours(self):
        self.assertEqual(measure_baseline.cpu_seconds("01:02:03"), 3_723)

    def test_parses_days(self):
        self.assertEqual(measure_baseline.cpu_seconds("2-01:02:03.5"), 176_523.5)


class PercentileTests(unittest.TestCase):
    def test_uses_nearest_rank(self):
        self.assertEqual(
            measure_baseline.percentile_nearest_rank([5, 1, 3, 2, 4], 0.95), 5
        )


if __name__ == "__main__":
    unittest.main()
