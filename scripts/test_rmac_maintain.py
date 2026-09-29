"""Tests for current-commit package candidate selection."""

from __future__ import annotations

from pathlib import Path
import re
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux" / "rmac-maintain.sh"


class CandidateSelectionTests(unittest.TestCase):
    def test_package_version_gate_accepts_debian_beta_versions(self):
        source = SCRIPT.read_text(encoding="utf-8")
        match = re.search(
            r'^\[\[ "\$package_version" =~ (.+) \]\] \\\n',
            source,
            re.MULTILINE,
        )
        self.assertIsNotNone(match)
        expression = match.group(1)

        result = subprocess.run(
            [
                "bash",
                "-c",
                'for version in "$@"; do package_version="$version"; '
                f'[[ "$package_version" =~ {expression} ]] || exit 1; done',
                "test",
                "0.9.0~beta.1-38",
                "1.0.0-1",
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

        for version in ("0.9.0~beta.-38", "0.9.0~beta.1-0"):
            invalid = subprocess.run(
                [
                    "bash",
                    "-c",
                    f'package_version="$1"; [[ "$package_version" =~ {expression} ]]',
                    "test",
                    version,
                ],
                check=False,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotEqual(invalid.returncode, 0, version)

    def test_status_does_not_imply_candidate_was_verified(self):
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("Candidate directory (not yet verified):", source)

    def test_older_same_version_candidate_is_not_selected(self):
        source = SCRIPT.read_text(encoding="utf-8")
        match = re.search(r"(?ms)^resolve_candidate\(\) \{.*?^\}", source)
        self.assertIsNotNone(match)
        function = match.group(0)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            current = root / "target/native-amd64-current"
            older = root / "target/native-amd64-older"
            older.mkdir(parents=True)
            # This represents a prior build with the same unchanged Debian
            # version. Package contents cannot prove which source commit made it.
            for name in (
                "rmac-apps_0.9.0~beta.1-38_amd64.deb",
                "rmac-session_0.9.0~beta.1-38_amd64.deb",
                "native-packages.json",
                "SHA256SUMS",
            ):
                (older / name).touch()

            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    f"repo_root=\"$2\"; architecture=amd64; {function}\nresolve_candidate \"$1\"",
                    "test",
                    str(current),
                    str(root),
                ],
                check=True,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.stdout, "")

            current.mkdir(parents=True)
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    f"repo_root=\"$2\"; architecture=amd64; {function}\nresolve_candidate \"$1\"",
                    "test",
                    str(current),
                    str(root),
                ],
                check=True,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.stdout.strip(), str(current))


if __name__ == "__main__":
    unittest.main()
