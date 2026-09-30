"""Focused fixtures for the I9 daily-driver Beta candidate."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from unittest.mock import patch


SCRIPT = Path(__file__).parent / "verify-beta-candidate.py"
SPEC = importlib.util.spec_from_file_location("verify_beta_candidate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class BetaCandidateTests(unittest.TestCase):
    def test_contract_only_run_does_not_claim_candidate_evidence(self):
        output = StringIO()
        with patch.object(sys, "argv", [str(SCRIPT)]), redirect_stdout(output):
            self.assertEqual(verify.main(), 0)
        self.assertIn("contract inventory verified", output.getvalue())
        self.assertIn("candidate evidence not supplied", output.getvalue())

    def passing_document(self):
        contract = verify.load_contract()
        version = "0.1.0-beta.1"
        revision = "f" * 40
        document = verify.evidence_template(contract, version, revision)
        for check in document["checks"]:
            check["status"] = "pass"
        document["cohort"] = {
            "duration_days": 14,
            "minimum_days_per_participant": 14,
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

    def test_source_inventory_rejects_an_uncovered_malformed_journey(self):
        original_load_json = verify._load_json
        journey_path = verify.REPO_ROOT / verify.SOURCES["journeys"]
        source = original_load_json(journey_path)
        source["journeys"].append({"id": 11})

        def load_with_extra_entry(path):
            return source if path == journey_path else original_load_json(path)

        with patch.object(verify, "_load_json", side_effect=load_with_extra_entry):
            with self.assertRaisesRegex(verify.BetaError, "journey source inventory"):
                verify._source_inventory()

    def test_checkout_binding_rejects_stale_revision_and_dirty_tree(self):
        with patch.object(verify, "_checkout_revision", return_value="a" * 40):
            with self.assertRaisesRegex(verify.BetaError, "differs from the checkout"):
                verify._verify_checkout("b" * 40)
        with (
            patch.object(verify, "_checkout_revision", return_value="a" * 40),
            patch.object(
                verify,
                "_require_clean_checkout",
                side_effect=verify.BetaError("Beta evidence requires a clean checkout"),
            ),
        ):
            with self.assertRaisesRegex(verify.BetaError, "clean checkout"):
                verify._verify_checkout("a" * 40)

    def test_accepts_exact_minimum_privacy_safe_cohort(self):
        contract, version, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                verify.verify_evidence(
                    contract, path, version=version, revision=revision
                )

    def test_successful_evidence_run_does_not_claim_evidence_was_missing(self):
        _, version, revision, document = self.passing_document()
        output = StringIO()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            arguments = [
                str(SCRIPT),
                "--evidence",
                str(path),
                "--version",
                version,
                "--revision",
                revision,
            ]
            with (
                patch.object(sys, "argv", arguments),
                patch.object(verify, "_verify_checkout"),
                redirect_stdout(output),
            ):
                self.assertEqual(verify.main(), 0)
        self.assertIn("candidate evidence verified", output.getvalue())
        self.assertNotIn("candidate evidence not supplied", output.getvalue())

    def test_rejects_open_critical_accessibility_and_short_cohort(self):
        contract, version, revision, document = self.passing_document()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            document["defects"][0]["open_count"] = 1
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.BetaError, "defect"):
                    verify.verify_evidence(
                        contract, path, version=version, revision=revision
                    )
            document["defects"][0]["open_count"] = 0
            document["cohort"]["duration_days"] = 13
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.BetaError, "cohort"):
                    verify.verify_evidence(
                        contract, path, version=version, revision=revision
                    )

    def test_rejects_short_participant_hidden_by_cohort_totals(self):
        contract, version, revision, document = self.passing_document()
        document["cohort"]["participant_days"] = 300
        document["cohort"]["minimum_days_per_participant"] = 13
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "beta.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.BetaError, "cohort"):
                    verify.verify_evidence(
                        contract, path, version=version, revision=revision
                    )


if __name__ == "__main__":
    unittest.main()
