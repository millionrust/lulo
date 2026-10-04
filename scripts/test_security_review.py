"""Focused fixtures for the I6 security and privacy review contract."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from unittest.mock import patch


SCRIPT = Path(__file__).parent / "verify-security-review.py"
SPEC = importlib.util.spec_from_file_location("verify_security_review", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class SecurityReviewTests(unittest.TestCase):
    def test_contract_only_run_does_not_claim_candidate_evidence(self):
        output = StringIO()
        with patch.object(sys, "argv", [str(SCRIPT)]), redirect_stdout(output):
            self.assertEqual(verify.main(), 0)
        self.assertIn("contract inventory verified", output.getvalue())
        self.assertIn("candidate evidence not supplied", output.getvalue())

    def test_committed_review_covers_every_named_goal_domain(self):
        contract = verify.load_contract()
        self.assertEqual(len(contract["domains"]), 10)
        self.assertEqual(len(verify.expected_results(contract)), 83)
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

    def test_source_inventory_rejects_symlink_substitution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repo"
            root.mkdir()
            target = root / "real-source.md"
            target.write_text("different source", encoding="utf-8")
            (root / "source.md").symlink_to(target)
            with patch.object(verify, "REPO_ROOT", root):
                with self.assertRaises(verify.SecurityError):
                    verify._read_repo_source("source.md")

    def test_regular_file_reader_does_not_follow_symlink_swap(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.md"
            target = root / "secret.md"
            source.write_text("reviewed source", encoding="utf-8")
            target.write_text("replacement", encoding="utf-8")
            open_file = verify.os.open

            def swap_then_open(path, flags, *args, **kwargs):
                if Path(path) == source:
                    source.unlink()
                    source.symlink_to(target)
                return open_file(path, flags, *args, **kwargs)

            with patch.object(verify.os, "open", side_effect=swap_then_open):
                with self.assertRaises(verify.SecurityError):
                    verify._read_regular(source)

    def test_source_reader_does_not_follow_parent_directory_swap(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repo"
            parent = root / "docs"
            parent.mkdir(parents=True)
            (parent / "source.md").write_text("reviewed", encoding="utf-8")
            replacement = Path(temporary) / "replacement"
            replacement.mkdir()
            (replacement / "source.md").write_text("unreviewed", encoding="utf-8")
            open_file = verify.os.open

            def swap_then_open(path, flags, *args, **kwargs):
                if path == "docs":
                    parent.rename(root / "old-docs")
                    parent.symlink_to(replacement)
                return open_file(path, flags, *args, **kwargs)

            with patch.object(verify, "REPO_ROOT", root):
                with patch.object(verify.os, "open", side_effect=swap_then_open):
                    with self.assertRaises(verify.SecurityError):
                        verify._read_repo_source("docs/source.md")

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
            with patch.object(verify, "_verify_checkout"):
                verify.verify_evidence(
                    contract, path, tier="alpha", revision=revision
                )
            self.assertEqual(set(document["source_sha256"]), set(verify.REVIEW_SOURCES))
            first_source = verify.REVIEW_SOURCES[0]
            document["source_sha256"][first_source] = "0" * 64
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.SecurityError, "source_sha256"):
                    verify.verify_evidence(
                        contract, path, tier="alpha", revision=revision
                    )
            document["source_sha256"][first_source] = verify.evidence_template(
                contract, "alpha", revision
            )["source_sha256"][first_source]
            document["open_findings"].append(
                {"id": "SEC-1", "severity": "critical"}
            )
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                with self.assertRaisesRegex(verify.SecurityError, "open_findings"):
                    verify.verify_evidence(
                        contract, path, tier="alpha", revision=revision
                    )

    def _passing_beta_evidence(self, contract, revision):
        document = verify.evidence_template(contract, "beta", revision)
        document["results"] = verify.expected_results(contract)
        for station in document["stations"]:
            if station["status"] == "pending":
                station["status"] = "pass"
        return document

    def _verify_document(self, contract, document, revision, tier="beta"):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with patch.object(verify, "_verify_checkout"):
                verify.verify_evidence(contract, path, tier=tier, revision=revision)

    def test_beta_needs_the_disposable_install_and_records_the_desktop_waivers(self):
        contract = verify.load_contract()
        revision = "e" * 40
        document = self._passing_beta_evidence(contract, revision)
        self.assertEqual(
            [station["id"] for station in document["stations"]],
            [
                "amd64-intel-laptop",
                "amd64-amd-desktop",
                "amd64-nvidia-desktop",
                "disposable-install",
            ],
        )
        self.assertEqual(
            document["stations"][2],
            {
                "id": "amd64-nvidia-desktop",
                "status": "waived",
                "waiver": "owner-2026-10-04-beta1-without-amd-nvidia-desktops",
            },
        )
        self._verify_document(contract, document, revision)
        self.assertEqual(document["stations"][1]["status"], "waived")
        # A real run is still accepted in place of either waiver.
        document["stations"][1] = {"id": "amd64-amd-desktop", "status": "pass"}
        document["stations"][2] = {"id": "amd64-nvidia-desktop", "status": "pass"}
        self._verify_document(contract, document, revision)

    def test_a_waiver_never_passes_checks_or_other_stations(self):
        contract = verify.load_contract()
        revision = "e" * 40
        pending_check = self._passing_beta_evidence(contract, revision)
        pending_check["results"][0]["status"] = "pending"
        with self.assertRaisesRegex(verify.SecurityError, "every exact check"):
            self._verify_document(contract, pending_check, revision)
        for index, station in ((0, "amd64-intel-laptop"), (3, "disposable-install")):
            document = self._passing_beta_evidence(contract, revision)
            document["stations"][index] = {
                "id": station,
                "status": "waived",
                "waiver": "owner-2026-10-04-beta1-without-amd-nvidia-desktops",
            }
            with self.assertRaisesRegex(verify.SecurityError, "required station"):
                self._verify_document(contract, document, revision)
        wrong = self._passing_beta_evidence(contract, revision)
        wrong["stations"][2]["waiver"] = "someone-else"
        with self.assertRaisesRegex(verify.SecurityError, "required station"):
            self._verify_document(contract, wrong, revision)
        missing = self._passing_beta_evidence(contract, revision)
        del missing["stations"][3]
        with self.assertRaisesRegex(verify.SecurityError, "required station"):
            self._verify_document(contract, missing, revision)

    def test_nvidia_is_not_waived_outside_beta(self):
        contract = verify.load_contract()
        revision = "e" * 40
        document = verify.evidence_template(contract, "one-dot-zero", revision)
        self.assertNotIn("waived", {station["status"] for station in document["stations"]})
        document["results"] = verify.expected_results(contract)
        for station in document["stations"]:
            station["status"] = "pass"
        document["stations"][3] = {
            "id": "amd64-nvidia-desktop",
            "status": "waived",
            "waiver": "owner-2026-10-04-beta1-without-amd-nvidia-desktops",
        }
        with self.assertRaisesRegex(verify.SecurityError, "required station"):
            self._verify_document(contract, document, revision, tier="one-dot-zero")

    def test_evidence_revision_requires_matching_clean_checkout(self):
        revision = "d" * 40
        def completed(code: int, output: bytes) -> subprocess.CompletedProcess[bytes]:
            return subprocess.CompletedProcess(["git"], code, output, b"")

        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, revision.encode() + b"\n"), completed(0, b""),
        ]):
            verify._verify_checkout(revision)
        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, ("a" * 40).encode() + b"\n"), completed(0, b""),
        ]):
            with self.assertRaisesRegex(verify.SecurityError, "revision differs"):
                verify._verify_checkout(revision)
        with patch.object(verify.subprocess, "run", side_effect=[
            completed(0, revision.encode() + b"\n"), completed(0, b" M src/main.rs\n"),
        ]):
            with self.assertRaisesRegex(verify.SecurityError, "clean checkout"):
                verify._verify_checkout(revision)


class FixedFindingGuardTests(unittest.TestCase):
    """Source guards for 0.9.0-beta.1 findings whose fix is configuration."""

    root = Path(__file__).resolve().parents[1]

    def test_service_connections_bound_outgoing_calls(self):
        # SR-25: a peer that never replies must not hold a call open.
        for relative in (
            "crates/rmac-notifications-linux/src/service.rs",
            "crates/rmac-clipboard-linux/src/service.rs",
            "crates/rmac-focus-linux/src/service.rs",
            "crates/rmac-file-chooser/src/dbus.rs",
            "crates/rmac-wallpaper-portal/src/dbus.rs",
            "crates/rmac-network/src/secret_agent.rs",
            "crates/rmac-bluetooth/src/pairing_agent.rs",
        ):
            with self.subTest(source=relative):
                text = (self.root / relative).read_text(encoding="utf-8")
                builders = text.count("Builder::session()") + text.count("Builder::system()")
                self.assertGreater(builders, 0)
                self.assertEqual(text.count(".method_timeout(CALL_TIMEOUT)"), builders)
                self.assertIn("Duration::from_secs(5)", text)


if __name__ == "__main__":
    unittest.main()
