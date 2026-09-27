"""Focused fixtures for the I1 automated journey runner."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


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


class PreviousResultTests(unittest.TestCase):
    revision = "a" * 40

    def write_result(self, path: Path, *, revision: str | None = None, results=None):
        path.write_text(
            json.dumps(
                {
                    "format": 1,
                    "revision": revision or self.revision,
                    "results": results if results is not None else [
                        {"duration_ms": 23, "id": 1, "name": "desktop-session", "status": "pass"},
                        {"duration_ms": 0, "id": 2, "name": "text-document", "status": "fail"},
                    ],
                }
            ),
            encoding="utf-8",
        )

    def test_resume_reuses_only_passed_records(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "journeys.json"
            self.write_result(path)

            passes = suite._previous_passes(path, self.revision)

        self.assertEqual(set(passes), {1})
        self.assertEqual(passes[1]["name"], "desktop-session")

    def test_resume_rejects_results_from_a_different_revision(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "journeys.json"
            self.write_result(path, revision="b" * 40)

            with self.assertRaisesRegex(suite.JourneyError, "does not match this revision"):
                suite._previous_passes(path, self.revision)

    def test_resume_rejects_malformed_result_records(self):
        invalid_records = [
            {"duration_ms": -1, "id": 1, "name": "desktop-session", "status": "pass"},
            {"duration_ms": 1, "id": True, "name": "desktop-session", "status": "pass"},
            {"duration_ms": 1, "id": 1, "name": "desktop-session", "status": "unknown"},
        ]
        for record in invalid_records:
            with self.subTest(record=record), tempfile.TemporaryDirectory() as raw:
                path = Path(raw) / "journeys.json"
                self.write_result(path, results=[record])

                with self.assertRaises(suite.JourneyError):
                    suite._previous_passes(path, self.revision)


class ResultPublicationTests(unittest.TestCase):
    document = {"format": 1, "revision": "c" * 40, "results": []}

    def test_publication_replaces_the_destination_with_complete_json(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "journeys.json"
            path.write_text("old result", encoding="utf-8")

            suite._publish(path, self.document)

            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), self.document)
            self.assertEqual(list(Path(raw).iterdir()), [path])

    def test_replace_failure_preserves_previous_result_and_cleans_temporary_file(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "journeys.json"
            path.write_text("previous result", encoding="utf-8")

            with mock.patch.object(suite.os, "replace", side_effect=OSError("disk error")):
                with self.assertRaisesRegex(suite.JourneyError, "cannot be published"):
                    suite._publish(path, self.document)

            self.assertEqual(path.read_text(encoding="utf-8"), "previous result")
            self.assertEqual(list(Path(raw).iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
