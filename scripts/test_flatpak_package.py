#!/usr/bin/env python3
"""Focused tests for the Flatpak packaging policy gate."""

from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest


SCRIPT = Path(__file__).parent / "linux/verify-flatpak-package.py"
SPEC = importlib.util.spec_from_file_location("verify_flatpak_package", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)
ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "packaging/flatpak"


class FlatpakPackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.decisions = json.loads(
            (PACKAGE / "decisions.json").read_text(encoding="utf-8")
        )
        cls.manifest = json.loads(
            (PACKAGE / "org.rmac.TextEditor.json").read_text(encoding="utf-8")
        )
        cls.sources = json.loads(
            (PACKAGE / "cargo-sources.json").read_text(encoding="utf-8")
        )
        cls.locked = verify.registry_packages(ROOT / "Cargo.lock")

    def test_repository_policy_and_offline_sources_are_exact(self):
        verify.verify_repository(ROOT)

    def test_broad_runtime_permission_is_rejected(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["finish-args"].append("--filesystem=home")
        with self.assertRaisesRegex(verify.VerificationError, "permissions"):
            verify.verify_manifest(manifest)

    def test_unreviewed_application_cannot_gain_a_manifest(self):
        decisions = copy.deepcopy(self.decisions)
        notes = next(
            entry
            for entry in decisions["applications"]
            if entry["id"] == "org.rmac.Notes"
        )
        notes["sandbox_eligible"] = True
        notes["manifest"] = "org.rmac.Notes.json"
        with self.assertRaisesRegex(verify.VerificationError, "only the reviewed"):
            verify.verify_decisions(decisions)

    def test_dependency_checksum_drift_is_rejected(self):
        sources = copy.deepcopy(self.sources)
        archive = next(source for source in sources if source["type"] == "archive")
        archive["sha256"] = "0" * 64
        with self.assertRaisesRegex(verify.VerificationError, "source mismatch"):
            verify.verify_cargo_sources(sources, self.locked)

    def test_offline_candidate_phase_cannot_weaken_download_refusal(self):
        driver = (
            ROOT / "scripts/linux/build-flatpak-candidate.sh"
        ).read_text(encoding="utf-8")
        weakened = driver.replace("--disable-download", "--disable-updates", 1)
        with self.assertRaisesRegex(verify.VerificationError, "offline build"):
            verify.verify_offline_driver_text(weakened)


if __name__ == "__main__":
    unittest.main()
