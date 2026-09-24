#!/usr/bin/env python3
"""Assemble a prepared rmac APT staging tree from already-built packages.

This is the piece that sits between the native/keyring package builders and
`publish-apt-snapshot.py`. It never builds packages, never signs anything, and
never touches a published repository. Given:

- a directory produced by ``build-native-packages.py`` for amd64,
- the matching directory for arm64,
- a directory produced by ``build-keyring-packages.py``, and
- a directory holding the already-built Debian source artifacts for the
  ``rmac`` source package (``.dsc``, source tarball(s), ``.buildinfo``,
  ``.changes``),

it lays out ``pool/``, the ``Packages``/``Sources`` indices (uncompressed and
deterministic-gzip, with SHA-256/SHA-512 by-hash copies), and the publication
manifest exactly as `docs/update-trust.md` and `publish-apt-snapshot.py`
require, then writes the *unsigned* clearsign-ready `Release` body next to
them. A separate, isolated signing step turns that `Release` file into
`dists/resolute/InRelease` (and removes the plaintext copy) before
`publish-apt-snapshot.py` is run against the staging directory.

All four publication gates (binary packages, license, reproducibility, and
source-offer verification) must be passed explicitly on the command line; none
default to true.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
from datetime import datetime, timezone
from email.utils import format_datetime


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/apt/publisher.json"
SNAPSHOT_RE = re.compile(r"[0-9]{8}T[0-9]{6}Z")
REVISION_RE = re.compile(r"[0-9a-f]{40}")
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
NATIVE_PACKAGES = ("rmac-apps", "rmac-session")
ARCHITECTURES = ("amd64", "arm64")
PHASE_PERCENTAGES = {0, 10, 25, 50, 100}


class StagingError(RuntimeError):
    """A bounded, privacy-safe APT staging failure."""


def _load_contract() -> dict[str, object]:
    try:
        document = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError("publisher contract is unavailable") from error
    return document


def _hash_file(path: Path) -> tuple[int, str, str]:
    sha256 = hashlib.sha256()
    sha512 = hashlib.sha512()
    size = 0
    try:
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                size += len(chunk)
                sha256.update(chunk)
                sha512.update(chunk)
    except OSError as error:
        raise StagingError(f"cannot read required input: {path}") from error
    return size, sha256.hexdigest(), sha512.hexdigest()


def _require_dir(path: Path, label: str) -> Path:
    if not path.is_absolute() or path.is_symlink() or not path.is_dir():
        raise StagingError(f"{label} must be an absolute ordinary directory")
    return path


def _pool_directory(package: str) -> str:
    letter = "r"
    return f"pool/main/{letter}/{package}"


def _copy_into_pool(source: Path, output: Path, pool_directory: str) -> tuple[str, int, str, str]:
    if source.is_symlink() or not source.is_file():
        raise StagingError(f"pool input is not a regular file: {source}")
    relative = f"{pool_directory}/{source.name}"
    destination = output / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise StagingError(f"duplicate pool destination: {relative}")
    shutil.copyfile(source, destination)
    size, sha256, sha512 = _hash_file(destination)
    return relative, size, sha256, sha512


def _deterministic_gzip(data: bytes) -> bytes:
    return gzip.compress(data, compresslevel=9, mtime=0)


def _load_native_manifest(directory: Path, architecture: str) -> dict[str, object]:
    path = directory / "native-packages.json"
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError(f"native package manifest is unavailable: {path}") from error
    if document.get("architecture") != architecture:
        raise StagingError(f"native package manifest architecture mismatch: {path}")
    packages = {record["package"]: record for record in document.get("packages", [])}
    if set(packages) != set(NATIVE_PACKAGES):
        raise StagingError(f"native package manifest inventory is not exact: {path}")
    return document


def _load_keyring_manifest(directory: Path) -> dict[str, object]:
    path = directory / "keyring-packages.json"
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError(f"keyring package manifest is unavailable: {path}") from error


def _find_source_artifacts(directory: Path, package: str) -> list[Path]:
    matches = sorted(
        path
        for path in directory.iterdir()
        if path.is_file()
        and not path.is_symlink()
        and path.name.startswith(f"{package}_")
    )
    names = [path.name for path in matches]
    if (
        not any(name.endswith(".dsc") for name in names)
        or not any(".tar." in name for name in names)
        or not any(name.endswith(".buildinfo") for name in names)
        or not any(name.endswith(".changes") for name in names)
    ):
        raise StagingError(
            f"{package} source directory is missing a required artifact type"
        )
    return matches


def _packages_paragraph(
    *,
    package: str,
    version: str,
    architecture: str,
    filename: str,
    size: int,
    sha256: str,
    sha512: str,
    phase: int,
) -> str:
    return "\n".join(
        (
            f"Package: {package}",
            f"Version: {version}",
            f"Architecture: {architecture}",
            f"Filename: {filename}",
            f"Size: {size}",
            f"SHA256: {sha256}",
            f"SHA512: {sha512}",
            f"Phased-Update-Percentage: {phase}",
        )
    )


def _sources_paragraph(
    *,
    package: str,
    version: str,
    directory: str,
    artifacts: list[tuple[str, int, str, str]],
) -> str:
    sha256_rows = [f" {sha256} {size} {name}" for name, size, sha256, sha512 in artifacts]
    sha512_rows = [f" {sha512} {size} {name}" for name, size, sha256, sha512 in artifacts]
    return "\n".join(
        (
            f"Package: {package}",
            f"Version: {version}",
            f"Directory: {directory}",
            "Checksums-Sha256:",
            *sha256_rows,
            "Checksums-Sha512:",
            *sha512_rows,
        )
    )


def stage(
    *,
    native_amd64: Path,
    native_arm64: Path,
    keyring_dir: Path,
    rmac_source_dir: Path,
    output: Path,
    phase: int,
    valid_hours: int,
    signer_fingerprints: list[str],
    product_revision: str,
    now_seconds: int | None = None,
    gate_binary_packages: bool,
    gate_licenses: bool,
    gate_reproducibility: bool,
    gate_source_offer: bool,
) -> Path:
    if phase not in PHASE_PERCENTAGES:
        raise StagingError("phase must be one of 0, 10, 25, 50, 100")
    if not 6 <= valid_hours <= 48:
        raise StagingError("valid-hours must be between 6 and 48")
    if not 1 <= len(signer_fingerprints) <= 2:
        raise StagingError("one or two signer fingerprints are required")
    if any(not FINGERPRINT_RE.fullmatch(value) for value in signer_fingerprints):
        raise StagingError("signer fingerprint is not a canonical OpenPGP fingerprint")
    if not REVISION_RE.fullmatch(product_revision):
        raise StagingError("product revision must be an exact lowercase 40-hex commit")
    if not all(
        (gate_binary_packages, gate_licenses, gate_reproducibility, gate_source_offer)
    ):
        raise StagingError("all four publication gates must be proven before staging")
    if output.exists() and any(output.iterdir()):
        raise StagingError("output directory must be empty")
    output.mkdir(parents=True, exist_ok=True)

    native_dirs = {
        "amd64": _require_dir(native_amd64, "--native-amd64"),
        "arm64": _require_dir(native_arm64, "--native-arm64"),
    }
    keyring_dir = _require_dir(keyring_dir, "--keyring-dir")
    rmac_source_dir = _require_dir(rmac_source_dir, "--rmac-source-dir")

    manifests = {
        architecture: _load_native_manifest(native_dirs[architecture], architecture)
        for architecture in ARCHITECTURES
    }
    versions = {document["version"] for document in manifests.values()}
    if len(versions) != 1:
        raise StagingError("native package version differs across architectures")
    native_version = versions.pop()

    keyring_manifest = _load_keyring_manifest(keyring_dir)
    if keyring_manifest.get("package_architecture") != "all":
        raise StagingError("keyring package must be Architecture: all")
    keyring_version = keyring_manifest["version"]
    keyring_binary_name = f"rmac-archive-keyring_{keyring_version}_all.deb"

    records: list[dict[str, object]] = []

    def record_pool(relative: str, size: int, sha256: str, sha512: str, role: str) -> None:
        records.append(
            {"path": relative, "role": role, "size": size, "sha256": sha256, "sha512": sha512}
        )

    # --- binary pool: rmac-apps / rmac-session per architecture ---
    binary_locations: dict[str, dict[str, tuple[str, int, str, str]]] = {
        "amd64": {},
        "arm64": {},
    }
    for architecture in ARCHITECTURES:
        for package in NATIVE_PACKAGES:
            filename = f"{package}_{native_version}_{architecture}.deb"
            source = native_dirs[architecture] / filename
            if not source.is_file():
                raise StagingError(f"expected native package artifact is missing: {filename}")
            relative, size, sha256, sha512 = _copy_into_pool(
                source, output, _pool_directory("rmac")
            )
            binary_locations[architecture][package] = (relative, size, sha256, sha512)
            record_pool(relative, size, sha256, sha512, "pool-binary")

    # --- binary pool: rmac-archive-keyring (Architecture: all, one copy) ---
    keyring_binary_source = keyring_dir / keyring_binary_name
    if not keyring_binary_source.is_file():
        raise StagingError("expected keyring binary artifact is missing")
    keyring_relative, keyring_size, keyring_sha256, keyring_sha512 = _copy_into_pool(
        keyring_binary_source, output, _pool_directory("rmac-archive-keyring")
    )
    record_pool(keyring_relative, keyring_size, keyring_sha256, keyring_sha512, "pool-binary")

    # --- source pool: rmac ---
    rmac_source_artifacts = _find_source_artifacts(rmac_source_dir, "rmac")
    rmac_source_directory = _pool_directory("rmac")
    rmac_source_rows: list[tuple[str, int, str, str]] = []
    for source in rmac_source_artifacts:
        relative, size, sha256, sha512 = _copy_into_pool(source, output, rmac_source_directory)
        role = "pool-source"
        record_pool(relative, size, sha256, sha512, role)
        if source.name.endswith(".dsc") or ".tar." in source.name:
            rmac_source_rows.append((source.name, size, sha256, sha512))

    # --- source pool: rmac-archive-keyring ---
    keyring_source_artifacts = _find_source_artifacts(keyring_dir, "rmac-archive-keyring")
    keyring_source_directory = _pool_directory("rmac-archive-keyring")
    keyring_source_rows: list[tuple[str, int, str, str]] = []
    for source in keyring_source_artifacts:
        if source.name == keyring_binary_name:
            continue
        relative, size, sha256, sha512 = _copy_into_pool(
            source, output, keyring_source_directory
        )
        record_pool(relative, size, sha256, sha512, "pool-source")
        if source.name.endswith(".dsc") or ".tar." in source.name:
            keyring_source_rows.append((source.name, size, sha256, sha512))

    # --- Packages indices ---
    dists_main = Path("dists/resolute/main")
    index_records: dict[str, bytes] = {}
    for architecture in ARCHITECTURES:
        rows = [
            _packages_paragraph(
                package="rmac-apps",
                version=native_version,
                architecture=architecture,
                filename=binary_locations[architecture]["rmac-apps"][0],
                size=binary_locations[architecture]["rmac-apps"][1],
                sha256=binary_locations[architecture]["rmac-apps"][2],
                sha512=binary_locations[architecture]["rmac-apps"][3],
                phase=phase,
            ),
            _packages_paragraph(
                package="rmac-archive-keyring",
                version=keyring_version,
                architecture="all",
                filename=keyring_relative,
                size=keyring_size,
                sha256=keyring_sha256,
                sha512=keyring_sha512,
                phase=100,
            ),
            _packages_paragraph(
                package="rmac-session",
                version=native_version,
                architecture=architecture,
                filename=binary_locations[architecture]["rmac-session"][0],
                size=binary_locations[architecture]["rmac-session"][1],
                sha256=binary_locations[architecture]["rmac-session"][2],
                sha512=binary_locations[architecture]["rmac-session"][3],
                phase=phase,
            ),
        ]
        index_records[f"{dists_main}/binary-{architecture}/Packages"] = (
            "\n\n".join(rows) + "\n"
        ).encode()

    sources_rows = [
        _sources_paragraph(
            package="rmac",
            version=native_version,
            directory=rmac_source_directory,
            artifacts=rmac_source_rows,
        ),
        _sources_paragraph(
            package="rmac-archive-keyring",
            version=keyring_version,
            directory=keyring_source_directory,
            artifacts=keyring_source_rows,
        ),
    ]
    index_records[f"{dists_main}/source/Sources"] = ("\n\n".join(sources_rows) + "\n").encode()

    for relative, raw in index_records.items():
        path = output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        compressed = _deterministic_gzip(raw)
        (path.with_name(path.name + ".gz")).write_bytes(compressed)
        size, sha256, sha512 = _hash_file(path)
        record_pool(relative, size, sha256, sha512, "index")
        compressed_path = path.with_name(path.name + ".gz")
        gsize, gsha256, gsha512 = _hash_file(compressed_path)
        record_pool(str(relative) + ".gz", gsize, gsha256, gsha512, "index")
        for algorithm, digest in (("SHA256", sha256), ("SHA512", sha512)):
            by_hash = path.parent / "by-hash" / algorithm / digest
            by_hash.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, by_hash)
        for algorithm, digest in (("SHA256", gsha256), ("SHA512", gsha512)):
            by_hash = compressed_path.parent / "by-hash" / algorithm / digest
            by_hash.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(compressed_path, by_hash)

    # --- publication manifest ---
    contract = _load_contract()
    date_seconds = (
        int(datetime.now(timezone.utc).timestamp())
        if now_seconds is None
        else now_seconds
    )
    snapshot = datetime.fromtimestamp(date_seconds, timezone.utc).strftime(
        "%Y%m%dT%H%M%SZ"
    )
    if not SNAPSHOT_RE.fullmatch(snapshot):
        raise StagingError("derived snapshot identity is invalid")
    valid_until_seconds = date_seconds + valid_hours * 3600
    records_sorted = sorted(records, key=lambda item: item["path"])
    manifest = {
        "date_seconds": date_seconds,
        "files": records_sorted,
        "format": 1,
        "gates": {
            "binary_packages_verified": True,
            "licenses_verified": True,
            "reproducibility_verified": True,
            "source_offer_verified": True,
        },
        "product_revision": product_revision,
        "signer_fingerprints": sorted(set(signer_fingerprints)),
        "snapshot": snapshot,
        "valid_until_seconds": valid_until_seconds,
    }
    manifest_relative = str(contract["publication_manifest"])
    manifest_path = output / manifest_relative
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    manifest_size, manifest_sha256, manifest_sha512 = _hash_file(manifest_path)

    # --- unsigned Release body (a separate signing step clearsigns this) ---
    signed_paths: dict[str, tuple[int, str, str]] = {
        "rmac-publication.json": (manifest_size, manifest_sha256, manifest_sha512)
    }
    for relative, raw in index_records.items():
        suite_relative = relative.removeprefix("dists/resolute/")
        for candidate in (suite_relative, suite_relative + ".gz"):
            path = output / "dists/resolute" / candidate
            signed_paths[candidate] = _hash_file(path)

    date_text = format_datetime(datetime.fromtimestamp(date_seconds, timezone.utc), usegmt=True)
    valid_until_text = format_datetime(
        datetime.fromtimestamp(valid_until_seconds, timezone.utc), usegmt=True
    )
    release_lines = [
        "Origin: rmac",
        "Label: rmac",
        "Suite: stable",
        "Codename: resolute",
        f"Date: {date_text}",
        f"Valid-Until: {valid_until_text}",
        "Architectures: amd64 arm64",
        "Components: main",
        "Acquire-By-Hash: yes",
        "Signed-By: " + " ".join(sorted(set(signer_fingerprints))),
        f"X-Rmac-Snapshot: {snapshot}",
        "SHA256:",
        *(
            f" {sha256} {size} {path}"
            for path, (size, sha256, sha512) in sorted(signed_paths.items())
        ),
        "SHA512:",
        *(
            f" {sha512} {size} {path}"
            for path, (size, sha256, sha512) in sorted(signed_paths.items())
        ),
    ]
    release_path = output / "dists/resolute/Release"
    release_path.write_bytes(("\n".join(release_lines) + "\n").encode())

    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-amd64", required=True, type=Path)
    parser.add_argument("--native-arm64", required=True, type=Path)
    parser.add_argument("--keyring-dir", required=True, type=Path)
    parser.add_argument("--rmac-source-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--phase", required=True, type=int, choices=sorted(PHASE_PERCENTAGES))
    parser.add_argument("--valid-hours", type=int, default=24)
    parser.add_argument("--signer-fingerprint", action="append", required=True)
    parser.add_argument("--product-revision", required=True)
    parser.add_argument("--binary-packages-verified", action="store_true")
    parser.add_argument("--licenses-verified", action="store_true")
    parser.add_argument("--reproducibility-verified", action="store_true")
    parser.add_argument("--source-offer-verified", action="store_true")
    arguments = parser.parse_args()
    try:
        output = stage(
            native_amd64=arguments.native_amd64,
            native_arm64=arguments.native_arm64,
            keyring_dir=arguments.keyring_dir,
            rmac_source_dir=arguments.rmac_source_dir,
            output=arguments.output,
            phase=arguments.phase,
            valid_hours=arguments.valid_hours,
            signer_fingerprints=arguments.signer_fingerprint,
            product_revision=arguments.product_revision,
            gate_binary_packages=arguments.binary_packages_verified,
            gate_licenses=arguments.licenses_verified,
            gate_reproducibility=arguments.reproducibility_verified,
            gate_source_offer=arguments.source_offer_verified,
        )
    except StagingError as error:
        parser.exit(3, f"stage-apt-snapshot: {error}\n")
    print(f"staged an unsigned rmac APT snapshot in {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
