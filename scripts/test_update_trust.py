"""Focused fixtures for the H7 update trust policy."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux/verify-update-trust.py"
SPEC = importlib.util.spec_from_file_location("verify_update_trust", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class UpdateTrustTests(unittest.TestCase):
    def test_committed_policy_and_templates_verify(self):
        digest = verify.verify_policy()
        self.assertEqual(len(digest), 64)

    def fixtures(self, root: Path) -> tuple[Path, Path, Path]:
        policy = root / "policy.json"
        sources = root / "rmac.sources.in"
        preferences = root / "rmac.pref"
        policy.write_bytes(verify.POLICY_PATH.read_bytes())
        sources.write_bytes(verify.SOURCES_PATH.read_bytes())
        preferences.write_bytes(verify.PREFERENCES_PATH.read_bytes())
        return policy, sources, preferences

    def test_rejects_insecure_source_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            policy, sources, preferences = self.fixtures(Path(temporary))
            sources.write_text(
                verify.EXPECTED_SOURCES.decode().replace(
                    "Check-Valid-Until: yes",
                    "Check-Valid-Until: no\nTrusted: yes",
                ),
                encoding="utf-8",
            )
            with self.assertRaises(verify.VerificationError):
                verify.verify_policy(policy, sources, preferences)

    def test_rejects_expiry_rotation_rollout_and_rollback_weakening(self):
        mutations = (
            ("release", "maximum_validity_seconds", 604800),
            ("signing", "minimum_overlap_days", 0),
            ("rollout", "security_percentage", 10),
            ("rollback", "automatic_downgrade", True),
            ("source", "deb_src_required", False),
        )
        for section, field, value in mutations:
            with self.subTest(section=section, field=field):
                with tempfile.TemporaryDirectory() as temporary:
                    policy, sources, preferences = self.fixtures(Path(temporary))
                    document = json.loads(policy.read_text(encoding="utf-8"))
                    document[section][field] = value
                    policy.write_text(
                        json.dumps(document, sort_keys=True) + "\n",
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(
                        verify.VerificationError, "reviewed contract"
                    ):
                        verify.verify_policy(policy, sources, preferences)

    def test_rejects_global_keyring_and_repository_overreach(self):
        with tempfile.TemporaryDirectory() as temporary:
            policy, sources, preferences = self.fixtures(Path(temporary))
            sources.write_bytes(
                verify.EXPECTED_SOURCES.replace(
                    b"/usr/share/keyrings/rmac-archive-keyring.gpg",
                    b"/etc/apt/trusted.gpg",
                )
            )
            with self.assertRaises(verify.VerificationError):
                verify.verify_policy(policy, sources, preferences)

            sources.write_bytes(verify.EXPECTED_SOURCES)
            preferences.write_text(
                "Package: *\n"
                "Pin: release o=rmac\n"
                "Pin-Priority: 1001\n",
                encoding="utf-8",
            )
            with self.assertRaises(verify.VerificationError):
                verify.verify_policy(policy, sources, preferences)


if __name__ == "__main__":
    unittest.main()
