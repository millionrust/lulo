"""Fixture tests for the pre-signing rmac APT snapshot stager."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sys
import subprocess
import tempfile
import unittest

import apt_test_fixtures as fixtures


stager = fixtures.load_script("rmac_stage_apt_snapshot", "stage-apt-snapshot.py")
publisher = fixtures.load_script("rmac_publish_apt_snapshot", "publish-apt-snapshot.py")
apt_archive = __import__("apt_archive")

SIGNER = fixtures.SIGNER


def stage(root: Path, inputs: Path = None, **overrides) -> Path:
    inputs = inputs or fixtures.build_inputs(root / "inputs")
    output = root / "output"
    stager.stage(
        **fixtures.stage_arguments(
            inputs, output=output, sidecar_output=root / "rmac-snapshot.json", **overrides
        )
    )
    return output


def verify_everything_but_the_signature(output: Path):
    contract = publisher.load_contract()
    manifest, records = publisher._parse_manifest(output, contract)
    for record in records:
        assert publisher._hash_file(output / record.path) == (record.size, record.sha256, record.sha512)
        if record.role == "index":
            for by_hash in publisher._by_hash_paths(record):
                assert publisher._hash_file(output / by_hash) == (record.size, record.sha256, record.sha512)
    maximum = int(contract["maximum_metadata_bytes"])
    publisher._verify_package_indices(output, records, maximum)
    publisher._verify_source_index(output, records, maximum)
    manifest["_manifest_path"] = output / contract["publication_manifest"]
    publisher.validate_release(
        (output / "dists/resolute/Release").read_bytes(),
        manifest=manifest,
        records=records,
        contract=contract,
        now_seconds=manifest["date_seconds"],
    )
    return manifest, records


def packages(output: Path, architecture: str):
    return {
        paragraph["Package"]: paragraph
        for paragraph in apt_archive.read_deb822_file(
            output / f"dists/resolute/main/binary-{architecture}/Packages", "Packages"
        )
    }


class StageAptSnapshotTests(unittest.TestCase):
    def test_staged_tree_passes_the_publisher_checks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage(root)
            manifest, _ = verify_everything_but_the_signature(output)
            self.assertEqual(manifest["snapshot"], "20231114T221320Z")
            self.assertEqual(manifest["signer_fingerprints"], [SIGNER])
            # The stager never writes InRelease; a separate step signs Release.
            self.assertFalse((output / publisher.INRELEASE_PATH).exists())
            amd64 = packages(output, "amd64")
            self.assertEqual(
                sorted(amd64),
                ["niri", "rmac-apps", "rmac-archive-keyring", "rmac-session", "xwayland-satellite"],
            )
            # arm64 is not built: its clients see only the keyring.
            self.assertEqual(sorted(packages(output, "arm64")), ["rmac-archive-keyring"])
            sources = apt_archive.read_deb822_file(output / "dists/resolute/main/source/Sources", "Sources")
            self.assertEqual(
                [paragraph["Package"] for paragraph in sources],
                ["niri", "rmac", "rmac-archive-keyring", "xwayland-satellite"],
            )

    def test_packages_carry_each_binarys_own_control_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage(root)
            session = packages(output, "amd64")["rmac-session"]
            self.assertEqual(
                session["Depends"],
                "niri (>= 26.04), rmac-apps (= 1.0.0-38), xwayland-satellite (>= 0.8.2)",
            )
            self.assertEqual(session["Installed-Size"], "42")
            self.assertEqual(
                session["Description"],
                "rmac-session summary\nA longer description line.\n.\nA second paragraph.",
            )
            self.assertEqual(session["Filename"], "pool/main/r/rmac/rmac-session_1.0.0-38_amd64.deb")
            self.assertEqual(session["Phased-Update-Percentage"], "10")
            niri = packages(output, "amd64")["niri"]
            self.assertEqual(niri["Filename"], "pool/main/n/niri/niri_26.04-0lulo1_amd64.deb")
            self.assertEqual(packages(output, "amd64")["rmac-archive-keyring"]["Phased-Update-Percentage"], "100")

    def test_pre_release_versions_keep_their_tilde_in_the_pool(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs", rmac="0.9.0~beta.1-38")
            output = stage(root, inputs)
            verify_everything_but_the_signature(output)
            self.assertTrue(
                (output / "pool/main/r/rmac/rmac-apps_0.9.0~beta.1-38_amd64.deb").is_file()
            )
            self.assertTrue((output / "pool/main/r/rmac/rmac_0.9.0~beta.1.orig.tar.xz").is_file())

    def test_both_architectures_publish_the_full_set(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs", architectures=("amd64", "arm64"))
            output = stage(root, inputs)
            verify_everything_but_the_signature(output)
            self.assertEqual(len(packages(output, "arm64")), 5)
            self.assertTrue((output / "pool/main/n/niri/niri_26.04-0lulo1_arm64.buildinfo").is_file())

    def test_two_signers_are_comma_separated_for_apt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage(root, signer_fingerprints=["B" * 40, SIGNER])
            release = (output / "dists/resolute/Release").read_text(encoding="utf-8")
            self.assertIn(f"Signed-By: {SIGNER},{'B' * 40}\n", release)
            verify_everything_but_the_signature(output)

    def test_sidecar_names_each_pool_objects_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage(root, release_tag="v1.0.0")
            sidecar = json.loads((root / "rmac-snapshot.json").read_text(encoding="utf-8"))
            manifest = json.loads((output / "dists/resolute/rmac-publication.json").read_text(encoding="utf-8"))
            pool = {item["path"] for item in manifest["files"] if item["path"].startswith("pool/")}
            self.assertEqual(set(sidecar["origins"]), pool)
            self.assertEqual(set(sidecar["origins"].values()), {"v1.0.0"})
            self.assertEqual(sidecar["phase"], 10)
            self.assertEqual(sidecar["phase_since_seconds"], manifest["date_seconds"])
            self.assertEqual(sidecar["snapshot"], manifest["snapshot"])

    def test_missing_gate_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(stager.StagingError, "gate"):
                stage(Path(temporary), gate_source_offer=False)

    def test_the_sidecar_must_live_outside_the_staging_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs")
            with self.assertRaisesRegex(stager.StagingError, "sidecar"):
                stager.stage(
                    **fixtures.stage_arguments(
                        inputs, output=root / "output", sidecar_output=root / "output" / "x.json"
                    )
                )

    def test_mismatched_architecture_versions_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs", architectures=("amd64", "arm64"))
            other = fixtures.build_inputs(root / "other", rmac="9.9.9-1", architectures=("arm64",))
            shutil.rmtree(inputs / "native-arm64")
            shutil.copytree(other / "native-arm64", inputs / "native-arm64")
            with self.assertRaisesRegex(stager.StagingError, "version"):
                stage(root, inputs)

    def test_a_binary_must_come_from_the_staged_source_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs")
            other = fixtures.build_inputs(root / "other", niri="26.08-0lulo1")
            for path in (inputs / "third-party-amd64").glob("niri_*_amd64.deb"):
                path.unlink()
            shutil.copy(other / "third-party-amd64/niri_26.08-0lulo1_amd64.deb", inputs / "third-party-amd64")
            with self.assertRaisesRegex(stager.StagingError, "not built from"):
                stage(root, inputs)

    def test_a_dsc_that_does_not_match_its_files_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs")
            (inputs / "rmac-source/rmac_1.0.0.orig.tar.xz").write_bytes(b"tampered")
            with self.assertRaisesRegex(stager.StagingError, "differs"):
                stage(root, inputs)

    def test_a_control_file_that_lies_about_its_package_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = fixtures.build_inputs(root / "inputs")
            native = inputs / "native-amd64"
            document = json.loads((native / "native-packages.json").read_text(encoding="utf-8"))
            document["version"] = "1.0.0-39"
            (native / "native-packages.json").write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(stager.StagingError, "control differs"):
                stage(root, inputs)

    # APT publication runs only on the Linux release runners.
    @unittest.skipUnless(
        sys.platform.startswith("linux") and shutil.which("gpg") and shutil.which("gpgv"),
        "APT publication signing is exercised on Linux with GnuPG",
    )
    def test_full_round_trip_through_publish_apt_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "gnupg"
            home.mkdir(mode=0o700)
            environment = {**os.environ, "GNUPGHOME": str(home), "LC_ALL": "C"}

            def gpg(*arguments):
                return subprocess.run(
                    ["gpg", "--batch", *arguments], env=environment, check=True, capture_output=True, timeout=60
                ).stdout

            gpg("--passphrase", "", "--quick-generate-key", "rmac apt fixture", "ed25519", "sign", "1d")
            signer = next(
                line.split(":")[9]
                for line in gpg("--with-colons", "--list-keys").decode("ascii").splitlines()
                if line.startswith("fpr:")
            )
            keyring = root / "keyring.gpg"
            keyring.write_bytes(gpg("--export", signer))
            output = stage(root, signer_fingerprints=[signer], now_seconds=1_700_000_000)
            release = output / "dists/resolute/Release"
            gpg(
                "--yes", "--local-user", signer, "--digest-algo", "SHA512", "--clearsign",
                "--output", str(output / publisher.INRELEASE_PATH), str(release),
            )
            release.unlink()
            publication = publisher.validate_staging(output, keyring, now_seconds=1_700_000_000)
            self.assertEqual(publication.signers, (signer,))
            repository = root / "repository"
            repository.mkdir()
            publisher.promote(
                output,
                repository,
                keyring,
                publication,
                retain=3,
                # A tiny fixture: don't hold it to the host disk's 15 GiB floor.
                free_bytes=lambda _path: 1 << 50,
            )
            self.assertTrue((repository / publisher.INRELEASE_PATH).is_file())


if __name__ == "__main__":
    unittest.main()
