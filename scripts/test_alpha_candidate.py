"""Focused fixtures for the I8 contributor Alpha candidate."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-alpha-candidate.py"
SPEC = importlib.util.spec_from_file_location("verify_alpha_candidate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class AlphaCandidateTests(unittest.TestCase):
    def test_contract_has_artifacts_safety_checks_stations_and_issue_context(self):
        contract = verify.load_contract()
        self.assertEqual(len(contract["artifacts"]), 6)
        self.assertEqual(len(contract["required_checks"]), 18)
        self.assertIn("zero-open-data-loss", contract["required_checks"])
        self.assertEqual(
            verify._alpha_stations(),
            ("amd64-intel-laptop", "amd64-amd-desktop"),
        )
        verify.verify_issue_form()

    def test_rejects_issue_form_without_portal_context(self):
        text = verify.ISSUE_FORM_PATH.read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "issue.yml"
            path.write_text(
                text.replace("    id: portals\n", "    id: removed-portals\n"),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(verify.AlphaError, "inventory"):
                verify.verify_issue_form(path)

    def test_candidate_requires_verified_artifacts_and_every_pass(self):
        contract = verify.load_contract()
        version = "0.1.0-alpha.1"
        revision = "e" * 40
        document = verify.evidence_template(contract, version, revision)
        for artifact in document["artifacts"]:
            artifact.update(
                {"sha256": "a" * 64, "size_bytes": 1, "status": "pass"}
            )
        for check in document["checks"]:
            check["status"] = "pass"
        for limitation in document["known_limitations"]:
            limitation["status"] = "disclosed"
        for station in document["stations"]:
            station["status"] = "pass"
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "alpha.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            verify.verify_evidence(
                contract, path, version=version, revision=revision
            )
            document["artifacts"][0]["size_bytes"] = 0
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.AlphaError, "artifact"):
                verify.verify_evidence(
                    contract, path, version=version, revision=revision
                )


if __name__ == "__main__":
    unittest.main()
