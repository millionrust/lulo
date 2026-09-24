"""Focused fixtures for the H7 atomic APT publisher."""

from __future__ import annotations

from datetime import datetime, timezone
from email.utils import format_datetime
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux/publish-apt-snapshot.py"
SPEC = importlib.util.spec_from_file_location("publish_apt_snapshot", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
publisher = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = publisher
SPEC.loader.exec_module(publisher)

# Promotion checks the repository's free space against the 15 GiB floor. The
# fixtures are tiny, so report ample space instead of the host's real disk
# (CI runners often have less than the floor free).
PLENTY_OF_SPACE = lambda _path: 1 << 50  # noqa: E731


def hashes(value: bytes) -> tuple[str, str]:
    return hashlib.sha256(value).hexdigest(), hashlib.sha512(value).hexdigest()


def fixture(stage: Path, signer: str = "A" * 40):
    contract = publisher.load_contract()
    pool_contents = {
        "pool/main/r/rmac/rmac-apps_1_amd64.deb": b"amd64 package\n",
        "pool/main/r/rmac/rmac-apps_1_arm64.deb": b"arm64 package\n",
        "pool/main/r/rmac/rmac-session_1_amd64.deb": b"amd64 session\n",
        "pool/main/r/rmac/rmac-session_1_arm64.deb": b"arm64 session\n",
        "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1_all.deb": b"keyring package\n",
        "pool/main/r/rmac/rmac_1.dsc": b"source control\n",
        "pool/main/r/rmac/rmac_1.tar.xz": b"source archive\n",
        "pool/main/r/rmac/rmac_1_amd64.buildinfo": b"build record\n",
        "pool/main/r/rmac/rmac_1_amd64.changes": b"upload record\n",
        "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1.dsc": b"keyring source control\n",
        "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1.tar.xz": b"keyring source archive\n",
        "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1_all.buildinfo": b"keyring build record\n",
        "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1_all.changes": b"keyring upload record\n",
    }
    for name in ("niri", "xwayland-satellite"):
        directory = f"pool/main/{name[0]}/{name}"
        for architecture in ("amd64", "arm64"):
            pool_contents[f"{directory}/{name}_1_{architecture}.deb"] = f"{name} {architecture}\n".encode()
        pool_contents[f"{directory}/{name}_1.dsc"] = f"{name} source control\n".encode()
        pool_contents[f"{directory}/{name}_1.orig.tar.gz"] = f"{name} source archive\n".encode()
        pool_contents[f"{directory}/{name}_1_amd64.buildinfo"] = f"{name} build record\n".encode()
        pool_contents[f"{directory}/{name}_1_amd64.changes"] = f"{name} upload record\n".encode()
    pool_identity = {
        path: (*hashes(value), len(value))
        for path, value in pool_contents.items()
    }

    package_indices = {}
    for architecture in ("amd64", "arm64"):
        package_rows = []
        paths = {
            "niri": f"pool/main/n/niri/niri_1_{architecture}.deb",
            "rmac-apps": f"pool/main/r/rmac/rmac-apps_1_{architecture}.deb",
            "rmac-archive-keyring": "pool/main/r/rmac-archive-keyring/rmac-archive-keyring_1_all.deb",
            "rmac-session": f"pool/main/r/rmac/rmac-session_1_{architecture}.deb",
            "xwayland-satellite": f"pool/main/x/xwayland-satellite/xwayland-satellite_1_{architecture}.deb",
        }
        for package, path in paths.items():
            sha256, sha512, size = pool_identity[path]
            package_rows.append(
                "\n".join(
                    (
                        f"Package: {package}",
                        "Version: 1.0.0-1",
                        "Architecture: "
                        + ("all" if package == "rmac-archive-keyring" else architecture),
                        f"Filename: {path}",
                        f"Size: {size}",
                        f"SHA256: {sha256}",
                        f"SHA512: {sha512}",
                        "Phased-Update-Percentage: 100",
                    )
                )
            )
        raw = ("\n\n".join(package_rows) + "\n").encode()
        base = f"dists/resolute/main/binary-{architecture}/Packages"
        package_indices[base] = raw
        package_indices[base + ".gz"] = gzip.compress(raw, mtime=0)

    source_rows = []
    for package, directory, names in (
        ("niri", "pool/main/n/niri", ("niri_1.dsc", "niri_1.orig.tar.gz")),
        (
            "rmac",
            "pool/main/r/rmac",
            ("rmac_1.dsc", "rmac_1.tar.xz"),
        ),
        (
            "rmac-archive-keyring",
            "pool/main/r/rmac-archive-keyring",
            (
                "rmac-archive-keyring_1.dsc",
                "rmac-archive-keyring_1.tar.xz",
            ),
        ),
        (
            "xwayland-satellite",
            "pool/main/x/xwayland-satellite",
            ("xwayland-satellite_1.dsc", "xwayland-satellite_1.orig.tar.gz"),
        ),
    ):
        sha256_rows = []
        sha512_rows = []
        for name in names:
            path = f"{directory}/{name}"
            sha256, sha512, size = pool_identity[path]
            sha256_rows.append(f" {sha256} {size} {name}")
            sha512_rows.append(f" {sha512} {size} {name}")
        source_rows.append(
            "\n".join(
                (
                    f"Package: {package}",
                    "Version: 1.0.0-1",
                    f"Directory: {directory}",
                    "Checksums-Sha256:",
                    *sha256_rows,
                    "Checksums-Sha512:",
                    *sha512_rows,
                )
            )
        )
    sources = ("\n\n".join(source_rows) + "\n").encode()
    source_indices = {
        "dists/resolute/main/source/Sources": sources,
        "dists/resolute/main/source/Sources.gz": gzip.compress(
            sources, mtime=0
        ),
    }
    contents = {**package_indices, **source_indices, **pool_contents}
    records = []
    for relative, value in sorted(contents.items()):
        sha256, sha512 = hashes(value)
        role = (
            "index"
            if relative in contract["required_indices"]
            else "pool-binary"
            if relative.endswith(".deb")
            else "pool-source"
        )
        records.append(
            {
                "path": relative,
                "role": role,
                "sha256": sha256,
                "sha512": sha512,
                "size": len(value),
            }
        )
        path = stage / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value)
        if role == "index":
            for algorithm, digest in (("SHA256", sha256), ("SHA512", sha512)):
                by_hash = path.parent / "by-hash" / algorithm / digest
                by_hash.parent.mkdir(parents=True, exist_ok=True)
                by_hash.write_bytes(value)

    date = 1_700_000_000
    manifest = {
        "date_seconds": date,
        "files": records,
        "format": 1,
        "gates": {
            "binary_packages_verified": True,
            "licenses_verified": True,
            "reproducibility_verified": True,
            "source_offer_verified": True,
        },
        "product_revision": "a" * 40,
        "signer_fingerprints": [signer],
        "snapshot": "20231114T221320Z",
        "valid_until_seconds": date + 21_600,
    }
    manifest_path = stage / contract["publication_manifest"]
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )

    signed = {}
    for record in records:
        if record["role"] == "index":
            signed[record["path"].removeprefix("dists/resolute/")] = record
    manifest_bytes = manifest_path.read_bytes()
    manifest_sha256, manifest_sha512 = hashes(manifest_bytes)
    signed["rmac-publication.json"] = {
        "sha256": manifest_sha256,
        "sha512": manifest_sha512,
        "size": len(manifest_bytes),
    }
    release_lines = [
        "Origin: rmac",
        "Label: rmac",
        "Suite: stable",
        "Codename: resolute",
        "Date: "
        + format_datetime(datetime.fromtimestamp(date, timezone.utc), usegmt=True),
        "Valid-Until: "
        + format_datetime(
            datetime.fromtimestamp(date + 21_600, timezone.utc), usegmt=True
        ),
        "Architectures: amd64 arm64",
        "Components: main",
        "Acquire-By-Hash: yes",
        "Signed-By: " + signer,
        "X-Rmac-Snapshot: 20231114T221320Z",
        "SHA256:",
        *(
            f" {record['sha256']} {record['size']} {path}"
            for path, record in sorted(signed.items())
        ),
        "SHA512:",
        *(
            f" {record['sha512']} {record['size']} {path}"
            for path, record in sorted(signed.items())
        ),
    ]
    release = ("\n".join(release_lines) + "\n").encode()
    (stage / publisher.INRELEASE_PATH).write_bytes(release)
    parsed_manifest, parsed_records = publisher._parse_manifest(stage, contract)
    parsed_manifest["_manifest_path"] = manifest_path
    publication = publisher.Publication(
        snapshot=manifest["snapshot"],
        product_revision=manifest["product_revision"],
        date_seconds=date,
        valid_until_seconds=date + 21_600,
        signers=(signer,),
        records=parsed_records,
        release_bytes=release,
        manifest_identity=publisher._hash_file(manifest_path),
        inrelease_identity=publisher._hash_file(
            stage / publisher.INRELEASE_PATH
        ),
    )
    return contract, parsed_manifest, parsed_records, publication, release


class AptPublisherTests(unittest.TestCase):
    def test_exact_prepared_archive_and_release_verify(self):
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            contract, manifest, records, _, release = fixture(stage)
            publisher._verify_stage_inventory(stage, contract, records)
            publisher.validate_release(
                release,
                manifest=manifest,
                records=records,
                contract=contract,
                now_seconds=manifest["date_seconds"],
            )

    def test_altered_by_hash_object_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            contract, _, records, _, _ = fixture(stage)
            index = next(record for record in records if record.role == "index")
            by_hash = publisher._by_hash_paths(index)[0]
            (stage / by_hash).write_bytes(b"altered\n")
            with self.assertRaisesRegex(publisher.PublisherError, "by-hash"):
                publisher._verify_stage_inventory(stage, contract, records)

    # APT publication runs only on the Linux release runners.
    @unittest.skipUnless(
        sys.platform.startswith("linux") and shutil.which("gpg") and shutil.which("gpgv"),
        "APT publication signing is exercised on Linux with GnuPG",
    )
    def test_real_single_signature_round_trip(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "gnupg"
            stage = root / "stage"
            home.mkdir(mode=0o700)
            stage.mkdir()
            environment = {**os.environ, "GNUPGHOME": str(home), "LC_ALL": "C"}
            subprocess.run(
                [
                    "gpg",
                    "--batch",
                    "--passphrase",
                    "",
                    "--quick-generate-key",
                    "rmac publisher fixture",
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
            fixture(stage, signer)
            inrelease = stage / publisher.INRELEASE_PATH
            release = inrelease.with_name("Release")
            inrelease.rename(release)
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
                    str(inrelease),
                    str(release),
                ],
                env=environment,
                check=True,
                capture_output=True,
                timeout=30,
            )
            release.unlink()
            publication = publisher.validate_staging(
                stage, keyring, now_seconds=1_700_000_000
            )
            self.assertEqual(publication.signers, (signer,))
            repository = root / "repository"
            repository.mkdir()
            contract = publisher.load_contract()
            publisher.promote(
                stage,
                repository,
                keyring,
                publication,
                retain=3,
                contract=contract,
                free_bytes=PLENTY_OF_SPACE,
            )
            state = (
                repository / contract["state_directory"] / "state.json"
            )
            state.unlink()
            publisher.promote(
                stage,
                repository,
                keyring,
                publication,
                retain=3,
                contract=contract,
                free_bytes=PLENTY_OF_SPACE,
            )
            self.assertTrue(state.is_file())

    def test_promotion_keeps_pool_immutable_and_inrelease_last_boundary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            repository = root / "repository"
            stage.mkdir()
            repository.mkdir()
            contract, _, records, publication, release = fixture(stage)
            keyring = root / "keyring.gpg"
            keyring.write_bytes(b"not used when no current release exists")
            publisher.promote(
                stage,
                repository,
                keyring,
                publication,
                retain=3,
                contract=contract,
                free_bytes=PLENTY_OF_SPACE,
            )
            self.assertEqual(
                (repository / publisher.INRELEASE_PATH).read_bytes(), release
            )
            state = json.loads(
                (
                    repository
                    / contract["state_directory"]
                    / "state.json"
                ).read_text(encoding="utf-8")
            )
            self.assertEqual(state["snapshot"], publication.snapshot)
            pool = next(record for record in records if record.role == "pool-binary")
            self.assertEqual(publisher._hash_file(repository / pool.path)[1], pool.sha256)

    def test_pool_collision_fails_before_inrelease_becomes_visible(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            repository = root / "repository"
            stage.mkdir()
            repository.mkdir()
            contract, _, records, publication, _ = fixture(stage)
            keyring = root / "keyring.gpg"
            keyring.write_bytes(b"unused")
            pool = next(record for record in records if record.role == "pool-binary")
            collision = repository / pool.path
            collision.parent.mkdir(parents=True)
            collision.write_bytes(b"different bytes\n")
            with self.assertRaisesRegex(
                publisher.PublisherError, "different bytes"
            ):
                publisher.promote(
                    stage,
                    repository,
                    keyring,
                    publication,
                    retain=3,
                    contract=contract,
                    free_bytes=PLENTY_OF_SPACE,
                )
            self.assertFalse((repository / publisher.INRELEASE_PATH).exists())

    def test_promotion_refuses_to_cross_the_storage_floor(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            repository = root / "repository"
            stage.mkdir()
            repository.mkdir()
            contract, _, _, publication, _ = fixture(stage)
            keyring = root / "keyring.gpg"
            keyring.write_bytes(b"unused")
            floor = int(contract["minimum_free_gib"]) * 1024**3
            with self.assertRaisesRegex(publisher.PublisherError, "storage floor"):
                publisher.promote(
                    stage,
                    repository,
                    keyring,
                    publication,
                    retain=3,
                    contract=contract,
                    free_bytes=lambda _path: floor,
                )
            self.assertFalse((repository / publisher.INRELEASE_PATH).exists())


if __name__ == "__main__":
    unittest.main()
