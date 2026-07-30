"""Focused fixtures for the I3 accessibility release audit."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-accessibility-audit.py"
SPEC = importlib.util.spec_from_file_location("verify_accessibility_audit", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class AccessibilityAuditTests(unittest.TestCase):
    def test_committed_audit_covers_every_surface_journey_and_dimension(self):
        audit = verify.load_audit()
        results = verify.expected_results(audit)
        surfaces = {item["subject"] for item in results if item["kind"] == "surface"}
        journeys = {item["subject"] for item in results if item["kind"] == "journey"}
        dimensions = {item["check"] for item in results}
        self.assertEqual(len(surfaces), 27)
        self.assertEqual(len(journeys), 10)
        self.assertEqual(dimensions, set(verify.REQUIRED_DIMENSIONS))

    def test_rejects_audit_with_missing_ime_dimension(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "audit.json"
            document = json.loads(verify.AUDIT_PATH.read_text(encoding="utf-8"))
            document["required_dimensions"].remove("ime")
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.AuditError, "dimensions"):
                verify.load_audit(path)

    def test_evidence_requires_every_exact_pass(self):
        audit = verify.load_audit()
        revision = "a" * 40
        document = verify.evidence_template(audit, revision)
        document["results"] = verify.expected_results(audit)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            verify.verify_evidence(audit, path, revision=revision)
            document["results"][0]["status"] = "blocked"
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.AuditError, "every exact check"):
                verify.verify_evidence(audit, path, revision=revision)


if __name__ == "__main__":
    unittest.main()
