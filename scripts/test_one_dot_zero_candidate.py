"""Focused fixtures for the I10 rmac 1.0 candidate."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-one-dot-zero-candidate.py"
SPEC = importlib.util.spec_from_file_location("verify_one_dot_zero_candidate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class OneDotZeroCandidateTests(unittest.TestCase):
    def passing_document(self):
        contract = verify.load_contract()
        version = "1.0.0-rc.1"
        previous = "1" * 40
        revision = "2" * 40
        document = verify.evidence_template(
            contract, version, previous, revision
        )
        for artifact in document["artifacts"]:
            artifact.update(
                {"sha256": "a" * 64, "size_bytes": 1, "status": "pass"}
            )
        for build in document["builds"]:
            build["build_evidence_sha256"] = "b" * 64
            build["performance_status"] = "pass"
            build["status"] = "pass"
            for journey in build["journeys"]:
                journey.update({"attempts": 20, "passed": 19, "status": "pass"})
            for station in build["stations"]:
                station["status"] = "pass"
        for check in document["checks"]:
            check["status"] = "pass"
        document["limitations"]["status"] = "reviewed"
        for recovery in document["recovery"]:
            recovery["status"] = "pass"
        return contract, version, previous, revision, document

    def test_contract_requires_two_builds_five_stations_and_top_five(self):
        contract = verify.load_contract()
        stations, journeys = verify._source_inventory()
        self.assertEqual(contract["build_count"], 2)
        self.assertEqual(len(stations), 5)
        self.assertEqual(journeys[:5], verify.TOP_FIVE)
        self.assertEqual(len(contract["required_checks"]), 16)

    def test_accepts_two_exact_95_percent_full_matrix_builds(self):
        contract, version, previous, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            verify.verify_evidence(
                contract,
                path,
                version=version,
                previous_revision=previous,
                revision=revision,
            )

    def test_rejects_top_five_crash_and_sub_95_journey(self):
        contract, version, previous, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.json"
            document["builds"][1]["top_five_crashes"]["desktop-session"] = 1
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.CandidateError, "crash"):
                verify.verify_evidence(
                    contract,
                    path,
                    version=version,
                    previous_revision=previous,
                    revision=revision,
                )
            document["builds"][1]["top_five_crashes"]["desktop-session"] = 0
            document["builds"][1]["journeys"][0]["passed"] = 18
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.CandidateError, "95 percent"):
                verify.verify_evidence(
                    contract,
                    path,
                    version=version,
                    previous_revision=previous,
                    revision=revision,
                )


if __name__ == "__main__":
    unittest.main()
