"""Focused checks for selecting and validating a CI candidate download."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


sys.path.insert(0, str(Path(__file__).parent / "linux"))
import fetch_candidate  # noqa: E402
from native_package_contract import native_version  # noqa: E402
from third_party_packages import load_pins  # noqa: E402

copy_spec = importlib.util.spec_from_file_location(
    "copy_pinned_third_party_debs",
    Path(__file__).parent / "linux" / "copy-pinned-third-party-debs.py",
)
assert copy_spec and copy_spec.loader
copy_pair_module = importlib.util.module_from_spec(copy_spec)
copy_spec.loader.exec_module(copy_pair_module)


class FetchCandidateTests(unittest.TestCase):
    def test_selects_latest_successful_run_for_the_exact_commit(self):
        sha = "a" * 40
        runs = [
            {"databaseId": 10, "headSha": sha, "conclusion": "success", "createdAt": "2026-10-01T10:00:00Z"},
            {"databaseId": 11, "headSha": "b" * 40, "conclusion": "success", "createdAt": "2026-10-01T13:00:00Z"},
            {"databaseId": 12, "headSha": sha, "conclusion": "failure", "createdAt": "2026-10-01T12:00:00Z"},
            {"databaseId": 13, "headSha": sha, "conclusion": "success", "createdAt": "2026-10-01T11:00:00Z"},
        ]
        self.assertEqual(fetch_candidate.selected_run(json.dumps(runs), sha), 13)
        with self.assertRaisesRegex(fetch_candidate.FetchError, "no successful"):
            fetch_candidate.selected_run(json.dumps(runs), "c" * 40)

    def test_checksum_validation_rejects_tampering_and_extra_debs(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            names = [f"package-{index}_1_amd64.deb" for index in range(4)]
            lines = []
            for index, name in enumerate(names):
                raw = f"deb-{index}".encode()
                (directory / name).write_bytes(raw)
                lines.append(f"{hashlib.sha256(raw).hexdigest()}  {name}\n")
            (directory / "SHA256SUMS").write_text("".join(lines), encoding="ascii")
            fetch_candidate.verify_checksums(directory)
            (directory / names[0]).write_bytes(b"altered")
            with self.assertRaisesRegex(fetch_candidate.FetchError, "checksum differs"):
                fetch_candidate.verify_checksums(directory)
            (directory / names[0]).write_bytes(b"deb-0")
            (directory / "extra_1_amd64.deb").write_bytes(b"extra")
            with self.assertRaisesRegex(fetch_candidate.FetchError, "inventory differs"):
                fetch_candidate.verify_checksums(directory)

    def test_profile_comes_from_manifest_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            manifest = directory / "native-packages.json"
            for profile, metadata in (("release", None), ("iterate", "iterate")):
                manifest.write_text(
                    json.dumps({"version": native_version(fetch_candidate.REPO_ROOT, build_metadata=metadata)}),
                    encoding="utf-8",
                )
                self.assertEqual(fetch_candidate.candidate_profile(directory), profile)
            manifest.write_text('{"version":"other"}', encoding="utf-8")
            with self.assertRaisesRegex(fetch_candidate.FetchError, "version does not match"):
                fetch_candidate.candidate_profile(directory)

    def test_copy_pair_rejects_wrong_deb_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            destination = Path(temporary) / "destination"
            source.mkdir()
            destination.mkdir()
            for pin in load_pins().values():
                (source / f"{pin.name}_{pin.debian_version}_amd64.deb").write_bytes(b"deb")
            copy_pair_module.copy_pair(source, destination, "amd64")
            self.assertEqual(len(list(destination.glob("*.deb"))), 2)
            (source / "niri_0-wrong_amd64.deb").write_bytes(b"deb")
            with self.assertRaisesRegex(ValueError, "inventory differs"):
                copy_pair_module.copy_pair(source, destination, "amd64")


if __name__ == "__main__":
    unittest.main()
