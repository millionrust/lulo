"""Focused fixtures for the I9 daily-driver Beta candidate."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-beta-candidate.py"
SPEC = importlib.util.spec_from_file_location("verify_beta_candidate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class BetaCandidateTests(unittest.TestCase):
    def passing_document(self):
        contract = verify.load_contract()
        version = "0.1.0-beta.1"
        revision = "f" * 40
        document = verify.evidence_template(contract, version, revision)
        for check in document["checks"]:
            check["status"] = "pass"
        document["cohort"] = {
            "duration_days": 14,
            "participant_days": 280,
            "participants": 20,
            "station_participants": dict(verify.STATION_MINIMUMS),
            "status": "pass",
        }
        for defect in document["defects"]:
            defect["status"] = "pass"
        for journey in document["journeys"]:
            journey.update({"attempts": 20, "passed": 18, "status": "pass"})
        for station in document["stations"]:
            station["status"] = "pass"
        return contract, version, revision, document

    def test_contract_has_three_stations_and_four_zero_defect_classes(self):
        contract = verify.load_contract()
        stations, journeys = verify._source_inventory()
        self.assertEqual(len(stations), 3)
        self.assertEqual(len(journeys), 10)
        self.assertEqual(len(contract["defect_classes"]), 4)
        self.assertEqual(len(contract["required_checks"]), 15)

    def test_accepts_exact_minimum_privacy_safe_cohort(self):
        contract, version, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            verify.verify_evidence(
                contract, path, version=version, revision=revision
            )

    def test_rejects_open_critical_accessibility_and_short_cohort(self):
        contract, version, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            document["defects"][0]["open_count"] = 1
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.BetaError, "defect"):
                verify.verify_evidence(
                    contract, path, version=version, revision=revision
                )
            document["defects"][0]["open_count"] = 0
            document["cohort"]["duration_days"] = 13
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.BetaError, "cohort"):
                verify.verify_evidence(
                    contract, path, version=version, revision=revision
                )


if __name__ == "__main__":
    unittest.main()
