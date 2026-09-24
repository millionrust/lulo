"""Fixture tests for the pre-signing rmac APT snapshot stager."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


def load_script(name: str, filename: str):
    script = Path(__file__).parent / "linux" / filename
    spec = importlib.util.spec_from_file_location(name, script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


stager = load_script("stage_apt_snapshot", "stage-apt-snapshot.py")
publisher = load_script("publish_apt_snapshot_for_staging_test", "publish-apt-snapshot.py")

VERSION = "1.0.0-38"
KEYRING_VERSION = "1.0.0-1"
SIGNER = "A" * 40
PRODUCT_REVISION = "b" * 40


def _write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def build_native_dir(root: Path, architecture: str) -> Path:
    directory = root / f"native-{architecture}"
    packages = []
    for package in ("rmac-apps", "rmac-session"):
        filename = f"{package}_{VERSION}_{architecture}.deb"
        contents = f"{package} {architecture} package\n".encode()
        _write(directory / filename, contents)
        packages.append({"package": package, "filename": filename})
    (directory / "native-packages.json").write_text(
        json.dumps(
            {
                "architecture": architecture,
                "format": 1,
                "packages": packages,
                "source_date_epoch": 1_700_000_000,
                "version": VERSION,
            }
        ),
        encoding="utf-8",
    )
    return directory


def build_keyring_dir(root: Path) -> Path:
    directory = root / "keyring"
    names = {
        "binary": f"rmac-archive-keyring_{KEYRING_VERSION}_all.deb",
        "dsc": f"rmac-archive-keyring_{KEYRING_VERSION}.dsc",
        "orig": f"rmac-archive-keyring_1.0.0.orig.tar.xz",
        "debian": f"rmac-archive-keyring_{KEYRING_VERSION}.debian.tar.xz",
        "buildinfo": f"rmac-archive-keyring_{KEYRING_VERSION}_all.buildinfo",
        "changes": f"rmac-archive-keyring_{KEYRING_VERSION}_all.changes",
    }
    for role, name in names.items():
        _write(directory / name, f"keyring {role}\n".encode())
    (directory / "keyring-packages.json").write_text(
        json.dumps(
            {
                "format": 1,
                "package": "rmac-archive-keyring",
                "package_architecture": "all",
                "source_package": "rmac-archive-keyring",
                "version": KEYRING_VERSION,
            }
        ),
        encoding="utf-8",
    )
    return directory


def build_rmac_source_dir(root: Path) -> Path:
    directory = root / "rmac-source"
    names = (
        f"rmac_{VERSION}.dsc",
        f"rmac_{VERSION}.tar.xz",
        f"rmac_{VERSION}_amd64.buildinfo",
        f"rmac_{VERSION}_amd64.changes",
    )
    for name in names:
        _write(directory / name, f"rmac source {name}\n".encode())
    return directory


def stage_fixture(root: Path, **overrides) -> Path:
    output = root / "output"
    arguments = {
        "native_amd64": build_native_dir(root, "amd64"),
        "native_arm64": build_native_dir(root, "arm64"),
        "keyring_dir": build_keyring_dir(root),
        "rmac_source_dir": build_rmac_source_dir(root),
        "output": output,
        "phase": 10,
        "valid_hours": 24,
        "signer_fingerprints": [SIGNER],
        "product_revision": PRODUCT_REVISION,
        "now_seconds": 1_700_000_000,
        "gate_binary_packages": True,
        "gate_licenses": True,
        "gate_reproducibility": True,
        "gate_source_offer": True,
    }
    arguments.update(overrides)
    stager.stage(**arguments)
    return output


class StageAptSnapshotTests(unittest.TestCase):
    def test_staged_tree_passes_inventory_and_index_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage_fixture(root)
            contract = publisher.load_contract()
            manifest, records = publisher._parse_manifest(output, contract)
            self.assertEqual(manifest["snapshot"], "20231114T221320Z")
            self.assertEqual(manifest["signer_fingerprints"], [SIGNER])
            self.assertTrue(all(manifest["gates"].values()))
            # The stager never writes InRelease itself; only a plaintext
            # Release body a separate signing step turns into InRelease.
            self.assertFalse((output / publisher.INRELEASE_PATH).exists())
            self.assertTrue((output / "dists/resolute/Release").is_file())
            # _verify_stage_inventory also demands InRelease to exist as part
            # of the *full* staging contract; verify everything else it
            # checks by calling the lower-level index/pool verifiers plus a
            # manual by-hash/pool pass excluding the not-yet-signed InRelease.
            self._verify_everything_but_inrelease(output, contract, records)

    def _verify_everything_but_inrelease(self, output, contract, records):
        by_path = {record.path: record for record in records}
        for record in records:
            self.assertEqual(
                publisher._hash_file(output / record.path),
                (record.size, record.sha256, record.sha512),
            )
            if record.role == "index":
                for by_hash in publisher._by_hash_paths(record):
                    self.assertEqual(
                        publisher._hash_file(output / by_hash),
                        (record.size, record.sha256, record.sha512),
                    )
        publisher._verify_package_indices(output, records, int(contract["maximum_metadata_bytes"]))
        publisher._verify_source_index(output, records, int(contract["maximum_metadata_bytes"]))

    def test_signed_release_validates_after_clearsigning(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = stage_fixture(root)
            contract = publisher.load_contract()
            manifest, records = publisher._parse_manifest(output, contract)
            manifest["_manifest_path"] = output / contract["publication_manifest"]
            release_path = output / "dists/resolute/Release"
            release_bytes = release_path.read_bytes()
            publisher.validate_release(
                release_bytes,
                manifest=manifest,
                records=records,
                contract=contract,
                now_seconds=manifest["date_seconds"],
            )

    @unittest.skipUnless(
        shutil.which("gpg") and shutil.which("gpgv"),
        "GnuPG tools are unavailable",
    )
    def test_full_round_trip_through_publish_apt_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "gnupg"
            home.mkdir(mode=0o700)
            environment = {**os.environ, "GNUPGHOME": str(home), "LC_ALL": "C"}
            subprocess.run(
                [
                    "gpg",
                    "--batch",
                    "--passphrase",
                    "",
                    "--quick-generate-key",
                    "rmac apt snapshot fixture",
                    "ed25519",
                    "sign",
                    "1d",
                ],
                env=environment,
                check=True,
                capture_output=True,
                timeout=30,
            )
            listing = subprocess.run(
                ["gpg", "--batch", "--with-colons", "--list-keys"],
                env=environment,
                check=True,
                capture_output=True,
                timeout=30,
            ).stdout.decode("ascii")
            signer = next(
                line.split(":")[9]
                for line in listing.splitlines()
                if line.startswith("fpr:")
            )
            keyring = root / "keyring.gpg"
            keyring.write_bytes(
                subprocess.run(
                    ["gpg", "--batch", "--export", signer],
                    env=environment,
                    check=True,
                    capture_output=True,
                    timeout=30,
                ).stdout
            )
            output = stage_fixture(root, signer_fingerprints=[signer])
            release_path = output / "dists/resolute/Release"
            inrelease_path = output / publisher.INRELEASE_PATH
            subprocess.run(
                [
                    "gpg",
                    "--batch",
                    "--yes",
                    "--local-user",
                    signer,
                    "--digest-algo",
                    "SHA512",
                    "--clearsign",
                    "--output",
                    str(inrelease_path),
                    str(release_path),
                ],
                env=environment,
                check=True,
                capture_output=True,
                timeout=30,
            )
            release_path.unlink()
            publication = publisher.validate_staging(
                output, keyring, now_seconds=1_700_000_000
            )
            self.assertEqual(publication.signers, (signer,))
            repository = root / "repository"
            repository.mkdir()
            publisher.promote(
                output,
                repository,
                keyring,
                publication,
                retain=3,
            )
            self.assertTrue((repository / publisher.INRELEASE_PATH).is_file())

    def test_missing_gate_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(stager.StagingError, "gate"):
                stage_fixture(root, gate_source_offer=False)

    def test_mismatched_architecture_versions_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native_amd64 = build_native_dir(root, "amd64")
            native_arm64 = build_native_dir(root, "arm64")
            manifest_path = native_arm64 / "native-packages.json"
            document = json.loads(manifest_path.read_text(encoding="utf-8"))
            document["version"] = "9.9.9-1"
            manifest_path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(stager.StagingError, "version"):
                stager.stage(
                    native_amd64=native_amd64,
                    native_arm64=native_arm64,
                    keyring_dir=build_keyring_dir(root),
                    rmac_source_dir=build_rmac_source_dir(root),
                    output=root / "output",
                    phase=10,
                    valid_hours=24,
                    signer_fingerprints=[SIGNER],
                    product_revision=PRODUCT_REVISION,
                    now_seconds=1_700_000_000,
                    gate_binary_packages=True,
                    gate_licenses=True,
                    gate_reproducibility=True,
                    gate_source_offer=True,
                )


if __name__ == "__main__":
    unittest.main()
