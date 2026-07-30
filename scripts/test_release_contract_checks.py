"""Focused fixtures for the aggregate release-contract runner."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "run-release-contract-checks.py"
SPEC = importlib.util.spec_from_file_location("run_release_contract_checks", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


class ReleaseContractRunnerTests(unittest.TestCase):
    def test_committed_suite_has_every_exact_lightweight_stage(self):
        suite = runner.load_suite()
        self.assertEqual(len(suite["stages"]), 14)
        self.assertEqual(suite["minimum_free_gib"], 15)
        self.assertEqual(
            suite["stages"][-1]["id"], "one-dot-zero-candidate"
        )

    def test_rejects_suite_that_drops_security_review(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "suite.json"
            document = json.loads(runner.SUITE_PATH.read_text(encoding="utf-8"))
            document["stages"] = [
                stage for stage in document["stages"]
                if stage["id"] != "security-review"
            ]
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(runner.ContractSuiteError, "differs"):
                runner.load_suite(path)

    def test_stage_runner_propagates_failure_without_shell(self):
        passing = {
            "command": ["$PYTHON", "-c", "raise SystemExit(0)"],
            "id": "passing",
        }
        runner.run_stage(passing, 5)
        failing = {
            "command": ["$PYTHON", "-c", "print('bounded failure'); raise SystemExit(7)"],
            "id": "failing",
        }
        with self.assertRaisesRegex(
            runner.ContractSuiteError, "(?s)status 7.*bounded failure"
        ):
            runner.run_stage(failing, 5)


if __name__ == "__main__":
    unittest.main()
