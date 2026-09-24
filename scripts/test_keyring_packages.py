"""Focused fixtures for the H7 standard archive-keyring package boundary."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


LINUX = Path(__file__).parent / "linux"
sys.path.insert(0, str(LINUX))
import keyring_package_contract as contract


def generated_key(root: Path) -> tuple[Path, Path, str, int]:
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
            "rmac archive fixture",
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
    fingerprint = next(
        line.split(":")[9]
        for line in listing.splitlines()
        if line.startswith("fpr:")
    )
    public = root / "archive-public.gpg"
    public.write_bytes(
        subprocess.run(
            [
                "gpg",
                "--batch",
                "--export-options",
                "export-minimal",
                "--export",
                fingerprint,
            ],
            env=environment,
            check=True,
            capture_output=True,
            timeout=30,
        ).stdout
    )
    secret = root / "archive-secret.gpg"
    secret.write_bytes(
        subprocess.run(
            ["gpg", "--batch", "--export-secret-keys", fingerprint],
            env=environment,
            check=True,
            capture_output=True,
            timeout=30,
        ).stdout
    )
    return public, secret, fingerprint, int(time.time())


class KeyringPackageTests(unittest.TestCase):
    def test_contract_and_source_tree_are_exact(self):
        policy = contract.load_contract()
        self.assertEqual(
            policy["installed_keyring"],
            "/usr/share/keyrings/rmac-archive-keyring.gpg",
        )
        self.assertEqual(contract.package_version(policy), "0.9.0~beta.1-1")
        identity = contract.KeyringIdentity(("A" * 40,), b"public")
        files = contract.source_files(
            identity,
            version="0.1.0-1",
            epoch=1_700_000_000,
        )
        self.assertEqual(files["debian/source/format"][0], b"3.0 (quilt)\n")
        self.assertIn(b"Rules-Requires-Root: no", files["debian/control"][0])
        self.assertIn(b"dh_builddeb -- -Zxz -z9", files["debian/rules"][0])
        self.assertEqual(files["debian/rules"][1], 0o755)
        self.assertEqual(
            files["debian/install"][0],
            b"rmac-archive-keyring.gpg usr/share/keyrings\n",
        )

    def test_original_source_tar_is_byte_reproducible(self):
        identity = contract.KeyringIdentity(("A" * 40,), b"public key material")
        first = contract.orig_tar_bytes(
            identity,
            version="0.1.0-1",
            epoch=1_700_000_000,
        )
        second = contract.orig_tar_bytes(
            identity,
            version="0.1.0-1",
            epoch=1_700_000_000,
        )
        self.assertEqual(first, second)
        parsed = contract._parse_tar_xz(first, "fixture source")
        self.assertEqual(
            parsed[
                "rmac-archive-keyring-0.1.0/rmac-archive-keyring.gpg"
            ],
            (b"public key material", 0o644),
        )
        self.assertFalse(any(path.startswith("debian/") for path in parsed))

    @unittest.skipUnless(shutil.which("gpg"), "GnuPG is unavailable")
    def test_public_key_canonicalization_rejects_secret_and_wrong_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            public, secret, fingerprint, epoch = generated_key(root)
            identity = contract.canonicalize_keyring(
                public,
                (fingerprint,),
                epoch=epoch,
                maximum_bytes=4 * 1024 * 1024,
            )
            self.assertEqual(identity.fingerprints, (fingerprint,))
            self.assertTrue(identity.bytes)
            with self.assertRaisesRegex(
                contract.KeyringPackageError, "secret key"
            ):
                contract.canonicalize_keyring(
                    secret,
                    (fingerprint,),
                    epoch=epoch,
                    maximum_bytes=4 * 1024 * 1024,
                )
            with self.assertRaisesRegex(
                contract.KeyringPackageError, "differs"
            ):
                contract.canonicalize_keyring(
                    public,
                    ("A" * 40,),
                    epoch=epoch,
                    maximum_bytes=4 * 1024 * 1024,
                )

    def test_checksum_inventory_rejects_wrong_column_shape(self):
        with self.assertRaisesRegex(
            contract.KeyringPackageError, "checksum inventory"
        ):
            contract._checksum_names(
                "deadbeef 10 missing-section.deb",
                5,
                "upload Files",
            )


if __name__ == "__main__":
    unittest.main()
