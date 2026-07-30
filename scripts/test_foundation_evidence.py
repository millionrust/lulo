"""Focused fixtures for the A1–A4 Linux foundation evidence bundle."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-foundation-evidence.py"
SPEC = importlib.util.spec_from_file_location("verify_foundation_evidence", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


def platform_report(arch: str = "x86_64") -> str:
    lines = [
        "rmac-platform-lab-report=1",
        "os=linux",
        f"arch={arch}",
        "recording_complete=true",
        "exercisable_probes_passed=true",
        "expected_blockers_confirmed=true",
        "recorded=12/12",
        *(f"probe.{probe}=pass" for probe in verify.PROBES),
        *(f"blocker.{blocker}=blocker-confirmed" for blocker in verify.BLOCKERS),
    ]
    return "\n".join(lines) + "\n"


def a4_report() -> str:
    lines = [
        "rmac-upstream-a4-report=1",
        f"revision={verify.UPSTREAM_REVISION}",
        *(f"result.{result}=pass" for result in verify.A4_RESULTS),
    ]
    return "\n".join(lines) + "\n"


class FoundationEvidenceTests(unittest.TestCase):
    def test_contract_covers_a1_a3_and_both_four_hour_soaks(self):
        contract = verify.load_contract()
        self.assertEqual(len(verify.expected_results()), 16)
        self.assertEqual(len(verify.A4_RESULTS), 23)
        self.assertEqual(
            contract["durations_seconds"],
            {"niri-stable-soak": 14_400, "upstream-soak": 14_400},
        )

    def test_platform_report_requires_all_pass_and_confirmed_api_blockers(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "platform.txt"
            path.write_text(platform_report(), encoding="ascii")
            self.assertEqual(verify.parse_platform_report(path), "x86_64")
            path.write_text(
                platform_report().replace("probe.input=pass", "probe.input=fail"),
                encoding="ascii",
            )
            with self.assertRaisesRegex(verify.FoundationError, "stable gate"):
                verify.parse_platform_report(path)

    def test_bundle_requires_exact_reports_hashes_durations_and_evidence(self):
        contract = verify.load_contract()
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary)
            (bundle / "gnome-platform-lab.txt").write_text(
                platform_report(), encoding="ascii"
            )
            (bundle / "niri-platform-lab.txt").write_text(
                platform_report(), encoding="ascii"
            )
            (bundle / "a4-upstream-report.txt").write_text(
                a4_report(), encoding="ascii"
            )
            document = verify.summary_template(bundle, revision)
            document["durations_seconds"] = {
                "niri-stable-soak": 14_400,
                "upstream-soak": 14_400,
            }
            document["results"] = verify.expected_results()
            document["supporting_evidence_sha256"] = {
                name: "b" * 64 for name in verify.SUPPORTING_EVIDENCE
            }
            (bundle / "foundation-summary.json").write_text(
                json.dumps(document), encoding="utf-8"
            )
            verify.verify_bundle(contract, bundle, revision=revision)
            document["durations_seconds"]["upstream-soak"] -= 1
            (bundle / "foundation-summary.json").write_text(
                json.dumps(document), encoding="utf-8"
            )
            with self.assertRaisesRegex(verify.FoundationError, "duration"):
                verify.verify_bundle(contract, bundle, revision=revision)


if __name__ == "__main__":
    unittest.main()
