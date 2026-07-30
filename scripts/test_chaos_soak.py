"""Focused fixtures for the I5 chaos and soak release contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-chaos-soak.py"
SPEC = importlib.util.spec_from_file_location("verify_chaos_soak", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class ChaosSoakTests(unittest.TestCase):
    def test_committed_contract_covers_all_faults_soaks_and_journeys(self):
        contract = verify.load_contract()
        results = verify.expected_results(contract)
        self.assertEqual(len(contract["scenarios"]), 14)
        self.assertEqual(len(results), 136)
        self.assertEqual(
            {item["phase"] for item in results},
            {"fault-injection", "eight-hour", "seven-day"},
        )
        self.assertEqual(
            len({item["subject"] for item in results if item["kind"] == "journey"}),
            10,
        )

    def test_rejects_contract_that_can_consume_host_storage_floor(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "chaos.json"
            document = json.loads(verify.CONTRACT_PATH.read_text(encoding="utf-8"))
            document["storage_safety"]["host_minimum_free_gib"] = 10
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.ChaosError, "differs"):
                verify.load_contract(path)

    def test_alpha_evidence_requires_full_duration_and_every_pass(self):
        contract = verify.load_contract()
        revision = "c" * 40
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for station in ("amd64-intel-laptop", "amd64-amd-desktop"):
                document = verify.evidence_template(contract, station, revision)
                document["results"] = verify.expected_results(contract)
                for run in document["runs"]:
                    run["status"] = "pass"
                (directory / f"{station}.json").write_text(
                    json.dumps(document), encoding="utf-8"
                )
            verify.verify_evidence_directory(
                contract, directory, tier="alpha", revision=revision
            )
            first = directory / "amd64-intel-laptop.json"
            document = json.loads(first.read_text(encoding="utf-8"))
            document["runs"][0]["duration_seconds"] -= 1
            first.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.ChaosError, "duration"):
                verify.verify_evidence_directory(
                    contract, directory, tier="alpha", revision=revision
                )


if __name__ == "__main__":
    unittest.main()
