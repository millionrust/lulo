"""Focused fixtures for the I1 automated journey runner."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).parent / "run-journey-suite.py"
SPEC = importlib.util.spec_from_file_location("run_journey_suite", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
suite = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = suite
SPEC.loader.exec_module(suite)


class JourneySuiteTests(unittest.TestCase):
    def test_manifest_maps_all_ten_goal_journeys(self):
        manifest = suite.load_manifest()
        self.assertEqual(
            [journey["id"] for journey in manifest["journeys"]],
            list(range(1, 11)),
        )
        self.assertTrue(
            all("keyboard" in journey["coverage"] for journey in manifest["journeys"])
        )

    def test_commands_are_argument_separated_and_package_scoped(self):
        journey = suite.load_manifest()["journeys"][1]
        commands = suite.commands_for(journey)
        self.assertEqual(commands[0][:3], ["cargo", "test", "--locked"])
        self.assertNotIn("--workspace", commands[0])
        self.assertNotIn("--all-features", commands[0])
        self.assertIn("rmac-text-editor", commands[0])

    def test_rejects_unknown_workspace_package(self):
        original = suite.workspace_packages
        try:
            suite.workspace_packages = lambda root=suite.REPO_ROOT: {"one-package"}
            with self.assertRaisesRegex(suite.JourneyError, "package inventory"):
                suite.load_manifest()
        finally:
            suite.workspace_packages = original


if __name__ == "__main__":
    unittest.main()
