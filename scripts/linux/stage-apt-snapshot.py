#!/usr/bin/env python3
"""Assemble a prepared rmac APT staging tree from already-built packages.

This sits between the package builders and `publish-apt-snapshot.py`. It
never builds packages, never signs anything, and never touches a published
repository. Given the release's package sets:

- the ``build-native-packages.py`` output for amd64 (and arm64 when built),
- the ``build-niri-packages.sh`` output for the same architectures
  (Lulo OS's niri and xwayland-satellite builds and their source packages),
- the ``build-keyring-packages.py`` output, and
- the ``build-rmac-source-package.sh source`` output (the ``rmac`` source
  package: ``.dsc``, both orig tarballs, ``.debian.tar.xz``, ``.buildinfo``,
  ``.changes``),

it lays out ``pool/``, the ``Packages``/``Sources`` indices (uncompressed and
deterministic gzip, with SHA-256/SHA-512 by-hash copies), and the publication
manifest exactly as `docs/update-trust.md` and `publish-apt-snapshot.py`
require, then writes the *unsigned* clearsign-ready ``Release`` body. A
separate, isolated signing step turns that into ``dists/resolute/InRelease``.

``Packages`` paragraphs carry each binary's own control fields (Depends,
Description, Installed-Size, ...) read from the ``.deb`` itself, so APT and
PackageKit resolve dependencies from the index exactly as dpkg will.

When the currently published repository is supplied (``--previous-repository``
with its ``rmac-snapshot.json`` sidecar), every source package version that is
already published is carried forward byte for byte from it rather than taken
from the new build: a rebuilt ``niri 26.04+lulo1`` with different bytes must
never replace the published one (the pool is immutable). A package version
that would go *backwards* is refused; an emergency revert is a higher version.
``--rollout-only`` additionally refuses any pool object that is not already
published, so a rollout step can only change phasing and signatures.

All four publication gates (binary packages, license, reproducibility, and
source-offer verification) must be passed explicitly; none default to true.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
from datetime import datetime, timezone
from email.utils import format_datetime
from typing import Dict, List, Optional, Tuple

sys.path.insert(0, str(Path(__file__).resolve().parent))

import apt_archive  # noqa: E402
from apt_archive import ArchiveError  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/apt/publisher.json"
SNAPSHOT_RE = re.compile(r"[0-9]{8}T[0-9]{6}Z")
REVISION_RE = re.compile(r"[0-9a-f]{40}")
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
TAG_RE = re.compile(r"v[0-9][0-9A-Za-z.+-]{0,63}")
NATIVE_PACKAGES = ("rmac-apps", "rmac-session")
THIRD_PARTY_PACKAGES = ("niri", "xwayland-satellite")
PRODUCT_PACKAGES = tuple(sorted(NATIVE_PACKAGES + THIRD_PARTY_PACKAGES))
KEYRING_PACKAGE = "rmac-archive-keyring"
SOURCE_PACKAGES = ("niri", "rmac", "rmac-archive-keyring", "xwayland-satellite")
ARCHITECTURES = ("amd64", "arm64")
PHASE_PERCENTAGES = {0, 10, 25, 50, 100}
REPOSITORY_FIELDS = {
    "Filename",
    "Size",
    "MD5sum",
    "SHA1",
    "SHA256",
    "SHA512",
    "Phased-Update-Percentage",
}
SOURCE_FIELDS = (
    "Binary",
    "Version",
    "Maintainer",
    "Uploaders",
    "Build-Depends",
    "Build-Depends-Arch",
    "Build-Depends-Indep",
    "Build-Conflicts",
    "Architecture",
    "Standards-Version",
    "Format",
    "Homepage",
    "Vcs-Browser",
    "Vcs-Git",
    "Testsuite",
    "Package-List",
)
SIDECAR_NAME = "rmac-snapshot.json"


class StagingError(RuntimeError):
    """A bounded, privacy-safe APT staging failure."""


def _load_contract() -> Dict[str, object]:
    try:
        return json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError("publisher contract is unavailable") from error


def _hash_file(path: Path) -> Tuple[int, str, str]:
    sha256 = hashlib.sha256()
    sha512 = hashlib.sha512()
    size = 0
    try:
        with path.open("rb") as source:
            while True:
                chunk = source.read(1024 * 1024)
                if not chunk:
                    break
                size += len(chunk)
                sha256.update(chunk)
                sha512.update(chunk)
    except OSError as error:
        raise StagingError(f"cannot read required input: {path.name}") from error
    return size, sha256.hexdigest(), sha512.hexdigest()


def _require_dir(path: Path, label: str) -> Path:
    if not path.is_absolute() or path.is_symlink() or not path.is_dir():
        raise StagingError(f"{label} must be an absolute ordinary directory")
    return path


def _regular_file(path: Path, label: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise StagingError(f"{label} is missing or not a regular file: {path.name}")
    return path


def _only(directory: Path, pattern: str, label: str) -> Path:
    matches = sorted(path for path in directory.glob(pattern) if path.is_file())
    if len(matches) != 1:
        raise StagingError(f"expected exactly one {label} in {directory.name} (found {len(matches)})")
    return _regular_file(matches[0], label)


def _deterministic_gzip(data: bytes) -> bytes:
    return gzip.compress(data, compresslevel=9, mtime=0)


# --- inputs -----------------------------------------------------------------------


class Binary:
    def __init__(self, path: Path, control: Dict[str, str], source: str):
        self.path = path
        self.control = control
        self.source = source

    @property
    def package(self) -> str:
        return self.control["Package"]

    @property
    def version(self) -> str:
        return self.control["Version"]

    @property
    def architecture(self) -> str:
        return self.control["Architecture"]

    @property
    def pool_path(self) -> str:
        return f"{apt_archive.pool_directory(self.source)}/{apt_archive.deb_filename(self.control)}"


class SourcePackage:
    def __init__(self, name: str, dsc: Path, fields: Dict[str, str], files: List[Path], records: List[Path]):
        self.name = name
        self.dsc = dsc
        self.fields = fields
        self.files = files
        self.records = records

    @property
    def version(self) -> str:
        return self.fields["Version"]

    @property
    def directory(self) -> str:
        return apt_archive.pool_directory(self.name)


def _control(path: Path) -> Dict[str, str]:
    try:
        return apt_archive.deb_control(path)
    except ArchiveError as error:
        raise StagingError(f"{path.name}: {error}") from error


def _load_native(directory: Path, architecture: str) -> List[Binary]:
    manifest_path = directory / "native-packages.json"
    try:
        document = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError(f"native package manifest is unavailable in {directory.name}") from error
    if document.get("architecture") != architecture:
        raise StagingError(f"native package manifest architecture mismatch in {directory.name}")
    records = {record.get("package"): record for record in document.get("packages", [])}
    if set(records) != set(NATIVE_PACKAGES):
        raise StagingError(f"native package manifest inventory is not exact in {directory.name}")
    binaries = []
    for package in NATIVE_PACKAGES:
        filename = records[package].get("filename")
        if not isinstance(filename, str) or "/" in filename:
            raise StagingError("native package manifest filename is invalid")
        path = _regular_file(directory / filename, "native package")
        control = _control(path)
        if (
            control["Package"] != package
            or control["Version"] != document.get("version")
            or control["Architecture"] != architecture
        ):
            raise StagingError(f"{filename} control differs from native-packages.json")
        binaries.append(Binary(path, control, "rmac"))
    return binaries


def _load_third_party(directory: Path, architecture: str) -> List[Binary]:
    binaries = []
    for package in THIRD_PARTY_PACKAGES:
        path = _only(directory, f"{package}_*_{architecture}.deb", f"{package} {architecture} package")
        control = _control(path)
        if control["Package"] != package or control["Architecture"] != architecture:
            raise StagingError(f"{path.name} control identity differs from its name")
        binaries.append(Binary(path, control, apt_archive.binary_source_name(control)))
    return binaries


def _load_keyring(directory: Path) -> Binary:
    try:
        document = json.loads((directory / "keyring-packages.json").read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise StagingError("keyring package manifest is unavailable") from error
    if document.get("package_architecture") != "all":
        raise StagingError("keyring package must be Architecture: all")
    version = document.get("version")
    if not isinstance(version, str) or not apt_archive.VERSION_RE.fullmatch(version):
        raise StagingError("keyring package version is invalid")
    path = _regular_file(directory / f"{KEYRING_PACKAGE}_{version}_all.deb", "keyring package")
    control = _control(path)
    if (
        control["Package"] != KEYRING_PACKAGE
        or control["Version"] != version
        or control["Architecture"] != "all"
    ):
        raise StagingError("keyring package control differs from keyring-packages.json")
    return Binary(path, control, KEYRING_PACKAGE)


def _load_source(name: str, directory: Path, record_directories: List[Path]) -> SourcePackage:
    dsc = _only(directory, f"{name}_*.dsc", f"{name} source control file")
    try:
        paragraphs = apt_archive.read_deb822_file(dsc, f"{name} .dsc")
    except ArchiveError as error:
        raise StagingError(str(error)) from error
    if len(paragraphs) != 1:
        raise StagingError(f"{dsc.name} must hold one paragraph")
    fields = paragraphs[0]
    if fields.get("Source") != name or not apt_archive.VERSION_RE.fullmatch(fields.get("Version", "")):
        raise StagingError(f"{dsc.name} does not describe source package {name}")
    version = apt_archive.version_without_epoch(fields["Version"])
    if dsc.name != f"{name}_{version}.dsc":
        raise StagingError(f"{dsc.name} is not named after its version")
    try:
        rows = apt_archive.checksum_rows(fields.get("Checksums-Sha256", ""), "Sha256")
    except ArchiveError as error:
        raise StagingError(f"{dsc.name}: {error}") from error
    if not rows or not any(".tar." in filename for _, _, filename in rows):
        raise StagingError(f"{dsc.name} names no source tarball")
    files = []
    for digest, size, filename in rows:
        path = _regular_file(directory / filename, f"{name} source file")
        actual_size, actual_sha256, _ = _hash_file(path)
        if (actual_size, actual_sha256) != (size, digest):
            raise StagingError(f"{filename} differs from {dsc.name}")
        files.append(path)
    records: List[Path] = []
    for record_directory in record_directories:
        for suffix in (".buildinfo", ".changes"):
            records.extend(
                sorted(
                    path
                    for path in record_directory.glob(f"{name}_{version}_*{suffix}")
                    if path.is_file() and not path.is_symlink()
                )
            )
    names = [path.name for path in records]
    if not any(value.endswith(".buildinfo") for value in names) or not any(
        value.endswith(".changes") for value in names
    ):
        raise StagingError(f"{name} {version} lacks its .buildinfo or .changes record")
    if len(set(names)) != len(names):
        raise StagingError(f"{name} build records are duplicated across architectures")
    return SourcePackage(name, dsc, fields, files, records)


# --- previous publication -------------------------------------------------------------


class Previous:
    """The currently published snapshot, as reconstructed by apt-publication.py."""

    def __init__(self, repository: Path, sidecar: Dict[str, object]):
        self.repository = repository
        contract = _load_contract()
        try:
            manifest = json.loads(
                (repository / str(contract["publication_manifest"])).read_text(encoding="utf-8")
            )
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise StagingError("previous publication manifest is unavailable") from error
        self.records = {
            item["path"]: (item["size"], item["sha256"], item["sha512"])
            for item in manifest.get("files", [])
            if str(item.get("path", "")).startswith("pool/")
        }
        self.snapshot = manifest.get("snapshot")
        origins = sidecar.get("origins")
        if (
            sidecar.get("format") != 1
            or sidecar.get("snapshot") != self.snapshot
            or not isinstance(origins, dict)
            or set(origins) != set(self.records)
            or any(not isinstance(tag, str) or not TAG_RE.fullmatch(tag) for tag in origins.values())
        ):
            raise StagingError("previous snapshot sidecar does not match its signed manifest")
        self.origins: Dict[str, str] = dict(origins)
        self.sidecar = sidecar
        self.package_versions: Dict[Tuple[str, str], str] = {}
        for architecture in ARCHITECTURES:
            path = repository / f"dists/resolute/main/binary-{architecture}/Packages"
            try:
                paragraphs = apt_archive.read_deb822_file(path, "previous Packages index")
            except ArchiveError as error:
                raise StagingError(str(error)) from error
            for paragraph in paragraphs:
                self.package_versions[(paragraph["Package"], architecture)] = paragraph["Version"]
        try:
            sources = apt_archive.read_deb822_file(
                repository / "dists/resolute/main/source/Sources", "previous Sources index"
            )
        except ArchiveError as error:
            raise StagingError(str(error)) from error
        self.source_versions = {paragraph["Package"]: paragraph["Version"] for paragraph in sources}

    def file(self, pool_path: str) -> Path:
        path = self.repository / pool_path
        if _hash_file(_regular_file(path, "previous pool object")) != self.records[pool_path]:
            raise StagingError(f"previous pool object differs from its signed manifest: {pool_path}")
        return path


# --- staging -------------------------------------------------------------------------


def _packages_paragraph(binary: Binary, pool_path: str, identity: Tuple[int, str, str], phase: int) -> str:
    size, sha256, sha512 = identity
    fields = [(name, value) for name, value in binary.control.items() if name not in REPOSITORY_FIELDS]
    fields.extend(
        [
            ("Filename", pool_path),
            ("Size", str(size)),
            ("SHA256", sha256),
            ("SHA512", sha512),
            ("Phased-Update-Percentage", str(phase)),
        ]
    )
    return apt_archive.format_paragraph(fields)


def _sources_paragraph(source: SourcePackage, rows: List[Tuple[str, int, str, str]]) -> str:
    fields: List[Tuple[str, str]] = [("Package", source.name)]
    for name in SOURCE_FIELDS:
        if name in source.fields:
            fields.append((name, source.fields[name]))
    fields.append(("Directory", source.directory))
    fields.append(
        ("Checksums-Sha256", "\n" + "\n".join(f"{sha256} {size} {name}" for name, size, sha256, _ in rows))
    )
    fields.append(
        ("Checksums-Sha512", "\n" + "\n".join(f"{sha512} {size} {name}" for name, size, _, sha512 in rows))
    )
    return apt_archive.format_paragraph(fields)


def stage(
    *,
    native_dirs: Dict[str, Path],
    third_party_dirs: Dict[str, Path],
    keyring_dir: Path,
    rmac_source_dir: Path,
    output: Path,
    phase: int,
    valid_hours: int,
    signer_fingerprints: List[str],
    product_revision: str,
    release_tag: str,
    sidecar_output: Path,
    previous_repository: Optional[Path] = None,
    previous_sidecar: Optional[Dict[str, object]] = None,
    rollout_only: bool = False,
    now_seconds: Optional[int] = None,
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
    if not TAG_RE.fullmatch(release_tag):
        raise StagingError("release tag is invalid")
    if not all((gate_binary_packages, gate_licenses, gate_reproducibility, gate_source_offer)):
        raise StagingError("all four publication gates must be proven before staging")
    if (previous_repository is None) != (previous_sidecar is None):
        raise StagingError("a previous repository needs its snapshot sidecar, and vice versa")
    if rollout_only and previous_repository is None:
        raise StagingError("a rollout step needs the published repository")
    if "amd64" not in native_dirs or set(native_dirs) != set(third_party_dirs):
        raise StagingError("amd64 is required, and every built architecture needs niri and xwayland-satellite")
    if not set(native_dirs) <= set(ARCHITECTURES):
        raise StagingError("unsupported architecture")
    if output.exists() and any(output.iterdir()):
        raise StagingError("output directory must be empty")
    sidecar_output = sidecar_output.resolve()
    if sidecar_output.exists() or sidecar_output.parent.resolve() == output.resolve() or str(
        sidecar_output
    ).startswith(str(output.resolve()) + "/"):
        raise StagingError("the sidecar must be a new file outside the staging directory")
    output.mkdir(parents=True, exist_ok=True)

    built = tuple(architecture for architecture in ARCHITECTURES if architecture in native_dirs)
    for architecture in built:
        _require_dir(native_dirs[architecture], f"--native-{architecture}")
        _require_dir(third_party_dirs[architecture], f"--third-party-{architecture}")
    keyring_dir = _require_dir(keyring_dir, "--keyring-dir")
    rmac_source_dir = _require_dir(rmac_source_dir, "--rmac-source-dir")

    # --- new inputs ---
    binaries: Dict[str, List[Binary]] = {}
    for architecture in built:
        binaries[architecture] = _load_native(native_dirs[architecture], architecture) + _load_third_party(
            third_party_dirs[architecture], architecture
        )
    keyring = _load_keyring(keyring_dir)
    sources = {
        "rmac": _load_source("rmac", rmac_source_dir, [rmac_source_dir]),
        KEYRING_PACKAGE: _load_source(KEYRING_PACKAGE, keyring_dir, [keyring_dir]),
    }
    for name in THIRD_PARTY_PACKAGES:
        sources[name] = _load_source(
            name, third_party_dirs["amd64"], [third_party_dirs[architecture] for architecture in built]
        )

    versions: Dict[str, str] = {}
    for architecture in built:
        for binary in binaries[architecture]:
            prior = versions.setdefault(binary.package, binary.version)
            if prior != binary.version:
                raise StagingError(f"{binary.package} version differs across architectures")
            if sources[binary.source].version != binary.version:
                raise StagingError(f"{binary.package} {binary.version} was not built from the staged {binary.source} source")
    versions[KEYRING_PACKAGE] = keyring.version
    if sources[KEYRING_PACKAGE].version != keyring.version:
        raise StagingError("the keyring package was not built from the staged keyring source")

    previous = Previous(previous_repository, previous_sidecar) if previous_repository else None

    # --- no version ever moves backwards; no architecture disappears ---
    if previous is not None:
        for (package, architecture), old in previous.package_versions.items():
            if package == KEYRING_PACKAGE:
                new = keyring.version
            elif architecture in built:
                new = versions.get(package)
            else:
                raise StagingError(f"{architecture} was published before and is missing from this release")
            if new is None:
                raise StagingError(f"{package} was published before and is missing from this release")
            if apt_archive.compare_versions(new, old) < 0:
                raise StagingError(
                    f"{package} {new} is older than the published {old}; publish a higher version instead"
                )

    # --- plan the pool: carried-forward objects come from the published pool ---
    carried = {
        name
        for name, source in sources.items()
        if previous is not None and previous.source_versions.get(name) == source.version
    }
    plan: Dict[str, Tuple[Path, str, str]] = {}  # pool path -> (file, role, origin)

    def place(pool_path: str, new_file: Path, role: str, carry: bool) -> None:
        if pool_path in plan:
            raise StagingError(f"duplicate pool destination: {pool_path}")
        if previous is not None and pool_path in previous.records:
            published = previous.file(pool_path)
            if carry or _hash_file(new_file) == previous.records[pool_path]:
                plan[pool_path] = (published, role, previous.origins[pool_path])
                return
            raise StagingError(
                f"{pool_path} is already published with different bytes; "
                "bump the version instead of republishing it"
            )
        if rollout_only:
            raise StagingError(f"a rollout step cannot publish a new pool object: {pool_path}")
        plan[pool_path] = (new_file, role, release_tag)

    staged_binaries: Dict[str, List[Binary]] = {}
    for architecture in built:
        staged_binaries[architecture] = []
        for binary in binaries[architecture]:
            place(binary.pool_path, binary.path, "pool-binary", binary.source in carried)
    place(keyring.pool_path, keyring.path, "pool-binary", KEYRING_PACKAGE in carried)

    staged_sources: Dict[str, SourcePackage] = {}
    for name in SOURCE_PACKAGES:
        source = sources[name]
        directory = source.directory
        place(f"{directory}/{source.dsc.name}", source.dsc, "pool-source", name in carried)
        for path in source.files + source.records:
            place(f"{directory}/{path.name}", path, "pool-source", name in carried)
        staged_sources[name] = source

    if rollout_only and set(plan) != set(previous.records):
        raise StagingError("a rollout step must publish exactly the already-published pool")

    # --- write the pool, then re-read identities from the bytes actually staged ---
    records: List[Dict[str, object]] = []
    identities: Dict[str, Tuple[int, str, str]] = {}
    for pool_path, (source_file, role, _) in sorted(plan.items()):
        destination = output / pool_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_file, destination)
        identity = _hash_file(destination)
        identities[pool_path] = identity
        records.append(
            {"path": pool_path, "role": role, "size": identity[0], "sha256": identity[1], "sha512": identity[2]}
        )

    # Controls and .dsc fields must describe the staged (possibly carried) bytes.
    def staged_binary(binary: Binary) -> Binary:
        path = output / binary.pool_path
        return Binary(path, _control(path), binary.source)

    for architecture in built:
        staged_binaries[architecture] = [staged_binary(binary) for binary in binaries[architecture]]
    staged_keyring = staged_binary(keyring)
    for name, source in list(staged_sources.items()):
        staged_sources[name] = _load_source(
            name,
            output / source.directory,
            [output / source.directory],
        )

    # --- Packages indices ---
    dists_main = "dists/resolute/main"
    index_records: Dict[str, bytes] = {}
    for architecture in ARCHITECTURES:
        entries = list(staged_binaries.get(architecture, [])) + [staged_keyring]
        rows = [
            _packages_paragraph(
                binary,
                binary.pool_path,
                identities[binary.pool_path],
                100 if binary.package == KEYRING_PACKAGE else phase,
            )
            for binary in sorted(entries, key=lambda item: item.package)
        ]
        index_records[f"{dists_main}/binary-{architecture}/Packages"] = ("\n\n".join(rows) + "\n").encode()

    # --- Sources index ---
    source_rows = []
    for name in SOURCE_PACKAGES:
        source = staged_sources[name]
        rows = []
        for path in [source.dsc] + source.files:
            size, sha256, sha512 = identities[f"{source.directory}/{path.name}"]
            rows.append((path.name, size, sha256, sha512))
        source_rows.append(_sources_paragraph(source, rows))
    index_records[f"{dists_main}/source/Sources"] = ("\n\n".join(source_rows) + "\n").encode()

    for relative, raw in index_records.items():
        path = output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        compressed_path = path.with_name(path.name + ".gz")
        compressed_path.write_bytes(_deterministic_gzip(raw))
        for candidate, name in ((path, relative), (compressed_path, relative + ".gz")):
            size, sha256, sha512 = _hash_file(candidate)
            records.append({"path": name, "role": "index", "size": size, "sha256": sha256, "sha512": sha512})
            for algorithm, digest in (("SHA256", sha256), ("SHA512", sha512)):
                by_hash = candidate.parent / "by-hash" / algorithm / digest
                by_hash.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(candidate, by_hash)

    # --- publication manifest ---
    contract = _load_contract()
    date_seconds = int(datetime.now(timezone.utc).timestamp()) if now_seconds is None else now_seconds
    snapshot = datetime.fromtimestamp(date_seconds, timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    if not SNAPSHOT_RE.fullmatch(snapshot):
        raise StagingError("derived snapshot identity is invalid")
    valid_until_seconds = date_seconds + valid_hours * 3600
    signers = sorted(set(signer_fingerprints))
    manifest = {
        "date_seconds": date_seconds,
        "files": sorted(records, key=lambda item: item["path"]),
        "format": 1,
        "gates": {
            "binary_packages_verified": True,
            "licenses_verified": True,
            "reproducibility_verified": True,
            "source_offer_verified": True,
        },
        "product_revision": product_revision,
        "signer_fingerprints": signers,
        "snapshot": snapshot,
        "valid_until_seconds": valid_until_seconds,
    }
    manifest_path = output / str(contract["publication_manifest"])
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    # --- unsigned Release body (a separate signing step clearsigns this) ---
    signed_paths: Dict[str, Tuple[int, str, str]] = {"rmac-publication.json": _hash_file(manifest_path)}
    for relative in index_records:
        suite_relative = relative[len("dists/resolute/"):]
        for candidate in (suite_relative, suite_relative + ".gz"):
            signed_paths[candidate] = _hash_file(output / "dists/resolute" / candidate)
    release_lines = [
        "Origin: rmac",
        "Label: rmac",
        "Suite: stable",
        "Codename: resolute",
        "Date: " + format_datetime(datetime.fromtimestamp(date_seconds, timezone.utc), usegmt=True),
        "Valid-Until: "
        + format_datetime(datetime.fromtimestamp(valid_until_seconds, timezone.utc), usegmt=True),
        "Architectures: amd64 arm64",
        "Components: main",
        "Acquire-By-Hash: yes",
        # apt-secure(8): a comma-separated fingerprint list for the next Release.
        "Signed-By: " + ",".join(signers),
        f"X-Rmac-Snapshot: {snapshot}",
        "SHA256:",
        *(f" {sha256} {size} {path}" for path, (size, sha256, _) in sorted(signed_paths.items())),
        "SHA512:",
        *(f" {sha512} {size} {path}" for path, (size, _, sha512) in sorted(signed_paths.items())),
    ]
    (output / "dists/resolute/Release").write_bytes(("\n".join(release_lines) + "\n").encode())

    # --- unsigned sidecar: where each pool object can be re-fetched, and phase timing ---
    staged_versions = {
        f"{binary.package}:{architecture}": binary.version
        for architecture in built
        for binary in staged_binaries[architecture]
    }
    phase_since = date_seconds
    if previous is not None:
        previous_since = previous.sidecar.get("phase_since_seconds")
        if (
            previous.sidecar.get("phase") == phase
            and previous.sidecar.get("versions") == staged_versions
            and isinstance(previous_since, int)
            and 0 <= previous_since <= date_seconds
        ):
            phase_since = previous_since
    sidecar = {
        "format": 1,
        "origins": {path: origin for path, (_, _, origin) in sorted(plan.items())},
        "phase": phase,
        "phase_since_seconds": phase_since,
        "product_revision": product_revision,
        "release_tag": release_tag,
        "snapshot": snapshot,
        "versions": staged_versions,
    }
    sidecar_output.write_text(json.dumps(sidecar, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--inputs", type=Path, help="an extracted apt-inputs-<tag>.tar (sets the directories below)")
    parser.add_argument("--native-amd64", type=Path)
    parser.add_argument("--native-arm64", type=Path)
    parser.add_argument("--third-party-amd64", type=Path)
    parser.add_argument("--third-party-arm64", type=Path)
    parser.add_argument("--keyring-dir", type=Path)
    parser.add_argument("--rmac-source-dir", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sidecar-output", required=True, type=Path)
    parser.add_argument("--phase", required=True, type=int, choices=sorted(PHASE_PERCENTAGES))
    parser.add_argument("--valid-hours", type=int, default=24)
    parser.add_argument("--signer-fingerprint", action="append", required=True)
    parser.add_argument("--product-revision", required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--previous-repository", type=Path)
    parser.add_argument("--previous-sidecar", type=Path)
    parser.add_argument("--rollout-only", action="store_true")
    parser.add_argument("--binary-packages-verified", action="store_true")
    parser.add_argument("--licenses-verified", action="store_true")
    parser.add_argument("--reproducibility-verified", action="store_true")
    parser.add_argument("--source-offer-verified", action="store_true")
    arguments = parser.parse_args()
    try:
        if arguments.inputs is not None:
            inputs = _require_dir(arguments.inputs, "--inputs")
            for architecture in ARCHITECTURES:
                for kind in ("native", "third_party"):
                    attribute = f"{kind}_{architecture}"
                    candidate = inputs / f"{kind.replace('_', '-')}-{architecture}"
                    if getattr(arguments, attribute) is None and candidate.is_dir():
                        setattr(arguments, attribute, candidate)
            if arguments.keyring_dir is None:
                arguments.keyring_dir = inputs / "keyring"
            if arguments.rmac_source_dir is None:
                arguments.rmac_source_dir = inputs / "rmac-source"
        native_dirs = {
            architecture: getattr(arguments, f"native_{architecture}")
            for architecture in ARCHITECTURES
            if getattr(arguments, f"native_{architecture}") is not None
        }
        third_party_dirs = {
            architecture: getattr(arguments, f"third_party_{architecture}")
            for architecture in ARCHITECTURES
            if getattr(arguments, f"third_party_{architecture}") is not None
        }
        if arguments.keyring_dir is None or arguments.rmac_source_dir is None:
            raise StagingError("--keyring-dir and --rmac-source-dir (or --inputs) are required")
        previous_sidecar = None
        if arguments.previous_sidecar is not None:
            try:
                previous_sidecar = json.loads(arguments.previous_sidecar.read_text(encoding="utf-8"))
            except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
                raise StagingError("previous snapshot sidecar is unreadable") from error
        output = stage(
            native_dirs=native_dirs,
            third_party_dirs=third_party_dirs,
            keyring_dir=arguments.keyring_dir,
            rmac_source_dir=arguments.rmac_source_dir,
            output=arguments.output,
            sidecar_output=arguments.sidecar_output,
            phase=arguments.phase,
            valid_hours=arguments.valid_hours,
            signer_fingerprints=arguments.signer_fingerprint,
            product_revision=arguments.product_revision,
            release_tag=arguments.release_tag,
            previous_repository=arguments.previous_repository,
            previous_sidecar=previous_sidecar,
            rollout_only=arguments.rollout_only,
            gate_binary_packages=arguments.binary_packages_verified,
            gate_licenses=arguments.licenses_verified,
            gate_reproducibility=arguments.reproducibility_verified,
            gate_source_offer=arguments.source_offer_verified,
        )
    except (StagingError, ArchiveError) as error:
        parser.exit(3, f"stage-apt-snapshot: {error}\n")
    print(f"staged an unsigned rmac APT snapshot in {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
