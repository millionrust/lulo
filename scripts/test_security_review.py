"""Focused fixtures for the I6 security and privacy review contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-security-review.py"
SPEC = importlib.util.spec_from_file_location("verify_security_review", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class SecurityReviewTests(unittest.TestCase):
    def test_committed_review_covers_every_named_goal_domain(self):
        contract = verify.load_contract()
        self.assertEqual(len(contract["domains"]), 10)
        self.assertEqual(len(verify.expected_results(contract)), 80)
        self.assertEqual(
            {domain["id"] for domain in contract["domains"]},
            {
                "desktop-entry-execution",
                "dbus-polkit",
                "portals",
                "file-operations",
                "lock-boundary",
                "notifications",
                "search-indexing",
                "packages",
                "updates",
                "logs-diagnostics",
            },
        )

    def test_rejects_removed_no_shell_execution_check(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "review.json"
            document = json.loads(verify.CONTRACT_PATH.read_text(encoding="utf-8"))
            document["domains"][0]["checks"].remove("no-shell-interpolation")
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.SecurityError, "differs"):
                verify.load_contract(path)

    def test_evidence_requires_zero_findings_all_checks_and_stations(self):
        contract = verify.load_contract()
        revision = "d" * 40
        document = verify.evidence_template(contract, "alpha", revision)
        document["results"] = verify.expected_results(contract)
        for station in document["stations"]:
            station["status"] = "pass"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            verify.verify_evidence(
                contract, path, tier="alpha", revision=revision
            )
            document["open_findings"].append(
                {"id": "SEC-1", "severity": "critical"}
            )
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.SecurityError, "open_findings"):
                verify.verify_evidence(
                    contract, path, tier="alpha", revision=revision
                )


if __name__ == "__main__":
    unittest.main()
