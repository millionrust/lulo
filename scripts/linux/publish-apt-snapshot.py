#!/usr/bin/env python3
"""Verify and atomically promote one prepared rmac APT snapshot."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from email.utils import format_datetime, parsedate_to_datetime
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from datetime import datetime, timezone


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/apt/publisher.json"
TRUST_PATH = REPO_ROOT / "packaging/apt/update-trust.json"
INRELEASE_PATH = "dists/resolute/InRelease"
SNAPSHOT_RE = re.compile(r"[0-9]{8}T[0-9]{6}Z")
REVISION_RE = re.compile(r"[0-9a-f]{40}")
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
HASH256_RE = re.compile(r"[0-9a-f]{64}")
HASH512_RE = re.compile(r"[0-9a-f]{128}")
MAX_TOOL_OUTPUT = 1024 * 1024
CHUNK = 1024 * 1024
ROLES = {"index", "pool-binary", "pool-source"}
GATES = (
    "binary_packages_verified",
    "licenses_verified",
    "reproducibility_verified",
    "source_offer_verified",
)
KEYRING_PACKAGE = "rmac-archive-keyring"
# Every built architecture's index names exactly these binaries. An
# architecture that is not built (arm64 until a runner exists) carries only
# the Architecture: all keyring, so its clients see no rmac candidate rather
# than a partial set.
BINARY_PACKAGES = (
    "niri",
    "rmac-apps",
    "rmac-archive-keyring",
    "rmac-session",
    "xwayland-satellite",
)
KEYRING_ONLY_PACKAGES = (KEYRING_PACKAGE,)
REQUIRED_BUILT_ARCHITECTURES = ("amd64",)
SOURCE_PACKAGES = ("niri", "rmac", "rmac-archive-keyring", "xwayland-satellite")
PHASE_PERCENTAGES = {0, 10, 25, 50, 100}


class PublisherError(RuntimeError):
    """A bounded, privacy-safe APT publication failure."""


@dataclass(frozen=True)
class FileRecord:
    path: str
    role: str
    size: int
    sha256: str
    sha512: str


@dataclass(frozen=True)
class Publication:
    snapshot: str
    product_revision: str
    date_seconds: int
    valid_until_seconds: int
    signers: tuple[str, ...]
    records: tuple[FileRecord, ...]
    release_bytes: bytes
    manifest_identity: tuple[int, str, str]
    inrelease_identity: tuple[int, str, str]


def _regular_bytes(path: Path, maximum: int, label: str) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PublisherError(f"{label} is unavailable") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise PublisherError(f"{label} must be a regular file")
    if metadata.st_size > maximum:
        raise PublisherError(f"{label} exceeds its size limit")
    try:
        value = path.read_bytes()
    except OSError as error:
        raise PublisherError(f"{label} cannot be read") from error
    if len(value) != metadata.st_size:
        raise PublisherError(f"{label} changed while reading")
    return value


def _load_json(path: Path, maximum: int, label: str) -> object:
    try:
        return json.loads(_regular_bytes(path, maximum, label))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PublisherError(f"{label} is invalid JSON") from error


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path, 256 * 1024, "publisher contract")
    expected = {
        "component": "main",
        "format": 1,
        "maximum_files": 4096,
        "maximum_keyring_bytes": 4 * 1024 * 1024,
        "maximum_metadata_bytes": 16 * 1024 * 1024,
        "minimum_free_gib": 15,
        "minimum_retained_snapshots": 3,
        "publication_manifest": "dists/resolute/rmac-publication.json",
        "required_indices": [
            "dists/resolute/main/binary-amd64/Packages",
            "dists/resolute/main/binary-amd64/Packages.gz",
            "dists/resolute/main/binary-arm64/Packages",
            "dists/resolute/main/binary-arm64/Packages.gz",
            "dists/resolute/main/source/Sources",
            "dists/resolute/main/source/Sources.gz",
        ],
        "state_directory": ".rmac-publisher",
        "suite": "resolute",
    }
    if document != expected:
        raise PublisherError("publisher contract differs from the reviewed boundary")
    trust = _load_json(TRUST_PATH, 256 * 1024, "update trust policy")
    if (
        not isinstance(trust, dict)
        or trust.get("format") != 1
        or trust.get("release", {}).get("codename") != document["suite"]
        or trust.get("release", {}).get("component") != document["component"]
        or trust.get("rollback", {}).get("minimum_retained_snapshots")
        != document["minimum_retained_snapshots"]
        or trust.get("signing", {}).get("keyring_format") != "binary-openpgp"
    ):
        raise PublisherError("publisher contract is inconsistent with update trust")
    return document


def _safe_relative(value: object) -> str:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        raise PublisherError("publication path is invalid")
    path = PurePosixPath(value)
    if path.is_absolute() or value != path.as_posix() or any(
        part in {"", ".", ".."} for part in path.parts
    ):
        raise PublisherError("publication path is not canonical")
    if not (
        value.startswith("pool/")
        or value.startswith("dists/resolute/main/")
    ):
        raise PublisherError("publication path escapes the approved archive areas")
    return value


def _safe_release_relative(value: object) -> str:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        raise PublisherError("signed Release path is invalid")
    path = PurePosixPath(value)
    if (
        path.is_absolute()
        or value != path.as_posix()
        or any(part in {"", ".", ".."} for part in path.parts)
        or not (value.startswith("main/") or value == "rmac-publication.json")
    ):
        raise PublisherError("signed Release path is outside the suite")
    return value


def _hash_file(path: Path) -> tuple[int, str, str]:
    sha256 = hashlib.sha256()
    sha512 = hashlib.sha512()
    size = 0
    try:
        with path.open("rb") as source:
            while chunk := source.read(CHUNK):
                size += len(chunk)
                sha256.update(chunk)
                sha512.update(chunk)
    except OSError as error:
        raise PublisherError("publication file cannot be read") from error
    return size, sha256.hexdigest(), sha512.hexdigest()


def _parse_manifest(
    stage: Path, contract: dict[str, object]
) -> tuple[dict[str, object], tuple[FileRecord, ...]]:
    manifest_path = stage / str(contract["publication_manifest"])
    document = _load_json(
        manifest_path,
        int(contract["maximum_metadata_bytes"]),
        "publication manifest",
    )
    if not isinstance(document, dict) or set(document) != {
        "date_seconds",
        "files",
        "format",
        "gates",
        "product_revision",
        "signer_fingerprints",
        "snapshot",
        "valid_until_seconds",
    }:
        raise PublisherError("publication manifest fields are not exact")
    if document["format"] != 1 or type(document["format"]) is not int:
        raise PublisherError("publication manifest format is invalid")
    if not isinstance(document["snapshot"], str) or not SNAPSHOT_RE.fullmatch(
        document["snapshot"]
    ):
        raise PublisherError("publication snapshot identity is invalid")
    if not isinstance(document["product_revision"], str) or not REVISION_RE.fullmatch(
        document["product_revision"]
    ):
        raise PublisherError("publication product revision is invalid")
    for field in ("date_seconds", "valid_until_seconds"):
        if type(document[field]) is not int or document[field] < 0:
            raise PublisherError("publication timestamp is invalid")
    try:
        snapshot_from_date = datetime.fromtimestamp(
            document["date_seconds"], timezone.utc
        ).strftime("%Y%m%dT%H%M%SZ")
    except (OSError, OverflowError, ValueError) as error:
        raise PublisherError("publication Date is outside the supported range") from error
    if document["snapshot"] != snapshot_from_date:
        raise PublisherError("publication snapshot must be the exact UTC Date")
    gates = document["gates"]
    if (
        not isinstance(gates, dict)
        or tuple(sorted(gates)) != GATES
        or any(value is not True for value in gates.values())
    ):
        raise PublisherError("required publication gates are not proven")
    signers = document["signer_fingerprints"]
    if (
        not isinstance(signers, list)
        or not signers
        or len(signers) > 2
        or signers != sorted(set(signers))
        or any(not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value) for value in signers)
    ):
        raise PublisherError("publication signer inventory is invalid")
    files = document["files"]
    if (
        not isinstance(files, list)
        or not files
        or len(files) > int(contract["maximum_files"])
    ):
        raise PublisherError("publication file inventory is invalid")
    records: list[FileRecord] = []
    for item in files:
        if not isinstance(item, dict) or set(item) != {
            "path",
            "role",
            "sha256",
            "sha512",
            "size",
        }:
            raise PublisherError("publication file record fields are not exact")
        path = _safe_relative(item["path"])
        if item["role"] not in ROLES:
            raise PublisherError("publication file role is invalid")
        if type(item["size"]) is not int or item["size"] < 0:
            raise PublisherError("publication file size is invalid")
        if (
            item["role"] == "index"
            and item["size"] > int(contract["maximum_metadata_bytes"])
        ):
            raise PublisherError("publication index exceeds its size limit")
        if not isinstance(item["sha256"], str) or not HASH256_RE.fullmatch(
            item["sha256"]
        ):
            raise PublisherError("publication SHA-256 is invalid")
        if not isinstance(item["sha512"], str) or not HASH512_RE.fullmatch(
            item["sha512"]
        ):
            raise PublisherError("publication SHA-512 is invalid")
        records.append(
            FileRecord(
                path=path,
                role=item["role"],
                size=item["size"],
                sha256=item["sha256"],
                sha512=item["sha512"],
            )
        )
    if [record.path for record in records] != sorted(
        {record.path for record in records}
    ):
        raise PublisherError("publication paths must be unique and sorted")
    required = set(contract["required_indices"])
    indexed = {record.path for record in records if record.role == "index"}
    if indexed != required:
        raise PublisherError("publication index inventory is not exact")
    pool = tuple(record for record in records if record.path.startswith("pool/"))
    if not pool or any(record.role == "index" for record in pool):
        raise PublisherError("publication pool inventory is incomplete")
    suffixes = tuple(Path(record.path).name for record in pool)
    if (
        not any(name.endswith(".deb") for name in suffixes)
        or not any(name.endswith(".dsc") for name in suffixes)
        or not any(".tar." in name for name in suffixes)
        or not any(name.endswith(".buildinfo") for name in suffixes)
        or not any(name.endswith(".changes") for name in suffixes)
    ):
        raise PublisherError("binary and source publication artifacts are incomplete")
    return document, tuple(records)


def _by_hash_paths(record: FileRecord) -> tuple[str, str]:
    parent = PurePosixPath(record.path).parent
    return (
        (parent / "by-hash" / "SHA256" / record.sha256).as_posix(),
        (parent / "by-hash" / "SHA512" / record.sha512).as_posix(),
    )


def _deb822_paragraphs(path: Path, maximum: int, label: str) -> list[dict[str, str]]:
    try:
        text = _regular_bytes(path, maximum, label).decode("utf-8")
    except UnicodeDecodeError as error:
        raise PublisherError(f"{label} is not UTF-8") from error
    if not text.endswith("\n") or "\r" in text or "\x00" in text:
        raise PublisherError(f"{label} encoding is invalid")
    paragraphs: list[dict[str, str]] = []
    current: dict[str, str] = {}
    current_field: str | None = None
    for line in text.splitlines():
        if not line:
            if current:
                paragraphs.append(current)
                current = {}
                current_field = None
            continue
        if line[0] in " \t":
            if current_field is None:
                raise PublisherError(f"{label} continuation is invalid")
            current[current_field] = (
                line[1:]
                if not current[current_field]
                else current[current_field] + "\n" + line[1:]
            )
            continue
        field, separator, value = line.partition(":")
        if (
            not separator
            or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", field)
            or field in current
            or len(current) >= 128
        ):
            raise PublisherError(f"{label} field inventory is invalid")
        current[field] = value.lstrip(" ")
        current_field = field
    if current:
        paragraphs.append(current)
    if not paragraphs or len(paragraphs) > 128:
        raise PublisherError(f"{label} paragraph inventory is invalid")
    return paragraphs


def _canonical_decimal(value: str, label: str) -> int:
    if not value.isascii() or not value.isdecimal() or str(int(value)) != value:
        raise PublisherError(f"{label} is not canonical decimal")
    return int(value)


def _verify_package_indices(
    stage: Path,
    records: tuple[FileRecord, ...],
    maximum: int,
) -> None:
    by_path = {record.path: record for record in records}
    referenced: set[str] = set()
    versions: dict[str, str] = {}
    phases: dict[str, int] = {}
    for architecture in ("amd64", "arm64"):
        relative = f"dists/resolute/main/binary-{architecture}/Packages"
        paragraphs = _deb822_paragraphs(
            stage / relative, maximum, f"{architecture} Packages index"
        )
        packages: dict[str, dict[str, str]] = {}
        for paragraph in paragraphs:
            required = {
                "Architecture",
                "Filename",
                "Package",
                "Phased-Update-Percentage",
                "SHA256",
                "SHA512",
                "Size",
                "Version",
            }
            if not required.issubset(paragraph):
                raise PublisherError("Packages index omits a required strong field")
            package = paragraph["Package"]
            if package not in BINARY_PACKAGES or package in packages:
                raise PublisherError("Packages index package inventory is not exact")
            if paragraph["Architecture"] != (
                "all" if package == KEYRING_PACKAGE else architecture
            ):
                raise PublisherError("Packages index architecture is invalid")
            version = paragraph["Version"]
            if not re.fullmatch(r"[0-9A-Za-z.+:~_-]{1,128}", version):
                raise PublisherError("Packages index version is invalid")
            prior_version = versions.setdefault(package, version)
            if prior_version != version:
                raise PublisherError("package version differs across architectures")
            path = _safe_relative(paragraph["Filename"])
            record = by_path.get(path)
            if record is None or record.role != "pool-binary":
                raise PublisherError("Packages index references an unreviewed binary")
            size = _canonical_decimal(paragraph["Size"], "package size")
            phase = _canonical_decimal(
                paragraph["Phased-Update-Percentage"], "package phase"
            )
            prior_phase = phases.setdefault(package, phase)
            if (
                phase not in PHASE_PERCENTAGES
                or prior_phase != phase
                or (package == KEYRING_PACKAGE and phase != 100)
                or size != record.size
                or paragraph["SHA256"] != record.sha256
                or paragraph["SHA512"] != record.sha512
            ):
                raise PublisherError("Packages index binary identity differs")
            referenced.add(path)
            packages[package] = paragraph
        allowed = [set(BINARY_PACKAGES)]
        if architecture not in REQUIRED_BUILT_ARCHITECTURES:
            allowed.append(set(KEYRING_ONLY_PACKAGES))
        if set(packages) not in allowed:
            raise PublisherError("Packages index package inventory is not exact")
    expected = {
        record.path for record in records if record.role == "pool-binary"
    }
    if referenced != expected:
        raise PublisherError("binary pool inventory is not exactly indexed")


def _checksum_inventory(
    value: str,
    *,
    algorithm: str,
    directory: str,
) -> dict[str, tuple[str, int]]:
    pattern = HASH256_RE if algorithm == "SHA256" else HASH512_RE
    inventory: dict[str, tuple[str, int]] = {}
    for line in value.splitlines():
        parts = line.split()
        if len(parts) != 3 or not pattern.fullmatch(parts[0]):
            raise PublisherError(f"Sources {algorithm} inventory is invalid")
        size = _canonical_decimal(parts[1], "source size")
        name = parts[2]
        if (
            not name
            or "/" in name
            or "\\" in name
            or name in {".", ".."}
        ):
            raise PublisherError("source artifact name is invalid")
        path = _safe_relative(f"{directory}/{name}")
        if path in inventory:
            raise PublisherError("source artifact is listed more than once")
        inventory[path] = (parts[0], size)
    if not inventory:
        raise PublisherError("source checksum inventory is empty")
    return inventory


def _verify_source_index(
    stage: Path,
    records: tuple[FileRecord, ...],
    maximum: int,
) -> None:
    paragraphs = _deb822_paragraphs(
        stage / "dists/resolute/main/source/Sources",
        maximum,
        "Sources index",
    )
    by_path = {record.path: record for record in records}
    packages: set[str] = set()
    referenced: set[str] = set()
    referenced_build_records: set[str] = set()
    for paragraph in paragraphs:
        required = {
            "Checksums-Sha256",
            "Checksums-Sha512",
            "Directory",
            "Package",
            "Version",
        }
        if not required.issubset(paragraph):
            raise PublisherError("Sources index omits a required strong field")
        package = paragraph["Package"]
        if package not in SOURCE_PACKAGES or package in packages:
            raise PublisherError("Sources index package inventory is not exact")
        if not re.fullmatch(r"[0-9A-Za-z.+:~_-]{1,128}", paragraph["Version"]):
            raise PublisherError("Sources index version is invalid")
        directory = _safe_relative(paragraph["Directory"])
        if not directory.startswith("pool/"):
            raise PublisherError("Sources index directory is outside the pool")
        sha256 = _checksum_inventory(
            paragraph["Checksums-Sha256"],
            algorithm="SHA256",
            directory=directory,
        )
        sha512 = _checksum_inventory(
            paragraph["Checksums-Sha512"],
            algorithm="SHA512",
            directory=directory,
        )
        if set(sha256) != set(sha512):
            raise PublisherError("Sources strong hash inventories differ")
        names = [Path(path).name for path in sha256]
        if (
            not any(name.endswith(".dsc") for name in names)
            or not any(".tar." in name for name in names)
        ):
            raise PublisherError("Sources index omits control or source tar material")
        for path in sha256:
            record = by_path.get(path)
            if (
                record is None
                or record.role != "pool-source"
                or sha256[path] != (record.sha256, record.size)
                or sha512[path] != (record.sha512, record.size)
            ):
                raise PublisherError("Sources index artifact identity differs")
            referenced.add(path)
        build_records = {
            record.path
            for record in records
            if record.role == "pool-source"
            and PurePosixPath(record.path).parent.as_posix() == directory
            and Path(record.path).name.startswith(package + "_")
            and (
                record.path.endswith(".buildinfo")
                or record.path.endswith(".changes")
            )
        }
        if (
            not any(path.endswith(".buildinfo") for path in build_records)
            or not any(path.endswith(".changes") for path in build_records)
        ):
            raise PublisherError("source package lacks matching build records")
        referenced_build_records.update(build_records)
        packages.add(package)
    if packages != set(SOURCE_PACKAGES):
        raise PublisherError("Sources index package inventory is not exact")
    indexable = {
        record.path
        for record in records
        if record.role == "pool-source"
        and (
            record.path.endswith(".dsc")
            or ".tar." in Path(record.path).name
        )
    }
    if referenced != indexable:
        raise PublisherError("source pool inventory is not exactly indexed")
    expected_build_records = {
        record.path
        for record in records
        if record.role == "pool-source"
        and (
            record.path.endswith(".buildinfo")
            or record.path.endswith(".changes")
        )
    }
    if referenced_build_records != expected_build_records:
        raise PublisherError("source build-record inventory is not exact")


def _verify_stage_inventory(
    stage: Path,
    contract: dict[str, object],
    records: tuple[FileRecord, ...],
) -> None:
    expected = {
        INRELEASE_PATH,
        str(contract["publication_manifest"]),
        *(record.path for record in records),
        *(
            path
            for record in records
            if record.role == "index"
            for path in _by_hash_paths(record)
        ),
    }
    actual: set[str] = set()
    try:
        for root, directories, files in os.walk(stage, followlinks=False):
            root_path = Path(root)
            for name in directories:
                path = root_path / name
                metadata = path.lstat()
                if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                    raise PublisherError("staged archive contains a linked directory")
            for name in files:
                path = root_path / name
                metadata = path.lstat()
                if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
                    raise PublisherError("staged archive contains a non-regular file")
                actual.add(path.relative_to(stage).as_posix())
    except OSError as error:
        raise PublisherError("staged archive cannot be inspected") from error
    if actual != expected:
        raise PublisherError("staged archive file inventory is not exact")
    by_path = {record.path: record for record in records}
    for path, record in by_path.items():
        actual_hash = _hash_file(stage / path)
        if actual_hash != (record.size, record.sha256, record.sha512):
            raise PublisherError("staged publication file differs from its manifest")
        if record.role == "index":
            for by_hash in _by_hash_paths(record):
                if _hash_file(stage / by_hash) != actual_hash:
                    raise PublisherError("staged by-hash index differs from canonical index")
    maximum = int(contract["maximum_metadata_bytes"])
    for uncompressed in (
        "dists/resolute/main/binary-amd64/Packages",
        "dists/resolute/main/binary-arm64/Packages",
        "dists/resolute/main/source/Sources",
    ):
        compressed = uncompressed + ".gz"
        raw_gzip = _regular_bytes(stage / compressed, maximum, "compressed index")
        if (
            len(raw_gzip) < 10
            or raw_gzip[:3] != b"\x1f\x8b\x08"
            or raw_gzip[3] != 0
            or raw_gzip[4:8] != b"\x00\x00\x00\x00"
        ):
            raise PublisherError("compressed index is not deterministic gzip")
        try:
            with gzip.open(stage / compressed, "rb") as source:
                expanded = source.read(maximum + 1)
                trailing = source.read(1)
        except (OSError, EOFError) as error:
            raise PublisherError("compressed index is invalid") from error
        if (
            len(expanded) > maximum
            or trailing
            or expanded
            != _regular_bytes(stage / uncompressed, maximum, "uncompressed index")
        ):
            raise PublisherError("compressed and uncompressed indices differ")
    _verify_package_indices(stage, records, maximum)
    _verify_source_index(stage, records, maximum)


def _parse_release_fields(release: bytes) -> dict[str, str]:
    try:
        text = release.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PublisherError("signed Release metadata is not UTF-8") from error
    if not text.endswith("\n") or "\r" in text or "\x00" in text:
        raise PublisherError("signed Release metadata encoding is invalid")
    fields: dict[str, str] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith(" "):
            if current not in {"SHA256", "SHA512"}:
                raise PublisherError("signed Release continuation is invalid")
            fields[current] = (
                line if not fields[current] else fields[current] + "\n" + line
            )
            continue
        name, separator, value = line.partition(":")
        if not separator or not name or name in fields:
            raise PublisherError("signed Release fields are invalid")
        current = name
        fields[name] = value.lstrip()
    return fields


def _parse_hash_section(value: str, algorithm: str) -> dict[str, tuple[str, int]]:
    expected_pattern = HASH256_RE if algorithm == "SHA256" else HASH512_RE
    result: dict[str, tuple[str, int]] = {}
    for line in value.splitlines():
        parts = line.split()
        if len(parts) != 3 or not expected_pattern.fullmatch(parts[0]):
            raise PublisherError(f"signed {algorithm} inventory is invalid")
        try:
            size = int(parts[1])
        except ValueError as error:
            raise PublisherError(f"signed {algorithm} size is invalid") from error
        path = _safe_release_relative(parts[2])
        if size < 0 or path in result:
            raise PublisherError(f"signed {algorithm} inventory is invalid")
        result[path] = (parts[0], size)
    return result


def validate_release(
    release: bytes,
    *,
    manifest: dict[str, object],
    records: tuple[FileRecord, ...],
    contract: dict[str, object],
    now_seconds: int,
) -> None:
    fields = _parse_release_fields(release)
    expected_fields = {
        "Acquire-By-Hash",
        "Architectures",
        "Codename",
        "Components",
        "Date",
        "Label",
        "Origin",
        "SHA256",
        "SHA512",
        "Signed-By",
        "Suite",
        "Valid-Until",
        "X-Rmac-Snapshot",
    }
    if set(fields) != expected_fields:
        raise PublisherError("signed Release field inventory is not exact")
    exact = {
        "Acquire-By-Hash": "yes",
        "Architectures": "amd64 arm64",
        "Codename": "resolute",
        "Components": "main",
        "Label": "rmac",
        "Origin": "rmac",
        # apt-secure(8): a comma-separated list of fingerprints.
        "Signed-By": ",".join(manifest["signer_fingerprints"]),
        "Suite": "stable",
        "X-Rmac-Snapshot": manifest["snapshot"],
    }
    if any(fields[name] != value for name, value in exact.items()):
        raise PublisherError("signed Release identity differs from the publication")
    try:
        date = int(parsedate_to_datetime(fields["Date"]).timestamp())
        valid_until = int(parsedate_to_datetime(fields["Valid-Until"]).timestamp())
        canonical_date = format_datetime(
            datetime.fromtimestamp(date, timezone.utc), usegmt=True
        )
        canonical_valid_until = format_datetime(
            datetime.fromtimestamp(valid_until, timezone.utc), usegmt=True
        )
    except (OSError, TypeError, ValueError, OverflowError) as error:
        raise PublisherError("signed Release timestamps are invalid") from error
    if (
        fields["Date"] != canonical_date
        or fields["Valid-Until"] != canonical_valid_until
        or date != manifest["date_seconds"]
        or valid_until != manifest["valid_until_seconds"]
        or date > now_seconds + 300
        or valid_until <= now_seconds
        or not 21_600 <= valid_until - date <= 172_800
    ):
        raise PublisherError("signed Release validity window is invalid")
    suite_prefix = "dists/resolute/"
    manifest_path = str(contract["publication_manifest"]).removeprefix(suite_prefix)
    signed_records = {
        record.path.removeprefix(suite_prefix): record
        for record in records
        if record.role == "index"
    }
    manifest_size, manifest_sha256, manifest_sha512 = _hash_file(
        Path(manifest["_manifest_path"])
    )
    expected_paths = set(signed_records) | {manifest_path}
    sha256 = _parse_hash_section(fields["SHA256"], "SHA256")
    sha512 = _parse_hash_section(fields["SHA512"], "SHA512")
    if set(sha256) != expected_paths or set(sha512) != expected_paths:
        raise PublisherError("signed Release index inventory is not exact")
    for path, record in signed_records.items():
        if sha256[path] != (record.sha256, record.size) or sha512[path] != (
            record.sha512,
            record.size,
        ):
            raise PublisherError("signed Release index hash differs")
    if sha256[manifest_path] != (manifest_sha256, manifest_size) or sha512[
        manifest_path
    ] != (manifest_sha512, manifest_size):
        raise PublisherError("signed Release manifest hash differs")


def _verify_inrelease(
    inrelease: Path, keyring: Path, maximum_keyring: int
) -> tuple[bytes, tuple[str, ...]]:
    if not keyring.is_absolute():
        raise PublisherError("verification keyring must be absolute")
    _regular_bytes(keyring, maximum_keyring, "verification keyring")
    _regular_bytes(inrelease, 16 * 1024 * 1024, "InRelease")
    gpgv = shutil.which("gpgv")
    if gpgv is None:
        raise PublisherError("gpgv is required for APT publication")
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        release = root / "Release"
        command = [
            gpgv,
            "--homedir",
            str(root),
            "--status-fd",
            "1",
            "--keyring",
            str(keyring),
            "--output",
            str(release),
            str(inrelease),
        ]
        try:
            result = subprocess.run(
                command,
                check=False,
                capture_output=True,
                timeout=30,
                env={"LC_ALL": "C", "PATH": os.environ.get("PATH", "")},
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise PublisherError("gpgv could not verify InRelease") from error
        if (
            result.returncode != 0
            or len(result.stdout) > MAX_TOOL_OUTPUT
            or len(result.stderr) > MAX_TOOL_OUTPUT
        ):
            raise PublisherError("InRelease signature verification failed")
        signer_identities: list[tuple[str, ...]] = []
        try:
            for line in result.stdout.decode("ascii").splitlines():
                prefix = "[GNUPG:] VALIDSIG "
                if line.startswith(prefix):
                    fields = line.removeprefix(prefix).split()
                    identities = [fields[0]]
                    if len(fields) >= 10 and fields[9] != fields[0]:
                        identities.append(fields[9])
                    signer_identities.append(tuple(identities))
        except (UnicodeDecodeError, IndexError) as error:
            raise PublisherError("gpgv status output is invalid") from error
        if (
            len(signer_identities) != 1
            or not signer_identities[0]
            or any(
                not FINGERPRINT_RE.fullmatch(fingerprint)
                for fingerprint in signer_identities[0]
            )
        ):
            raise PublisherError("InRelease must have exactly one valid signature")
        return _regular_bytes(
            release, 16 * 1024 * 1024, "verified Release metadata"
        ), signer_identities[0]


def validate_staging(
    stage: Path,
    keyring: Path,
    *,
    now_seconds: int,
    contract: dict[str, object] | None = None,
) -> Publication:
    contract = load_contract() if contract is None else contract
    if not stage.is_absolute() or stage.is_symlink() or not stage.is_dir():
        raise PublisherError("staging directory must be an absolute ordinary directory")
    manifest, records = _parse_manifest(stage, contract)
    manifest["_manifest_path"] = stage / str(contract["publication_manifest"])
    _verify_stage_inventory(stage, contract, records)
    release, signer_identities = _verify_inrelease(
        stage / INRELEASE_PATH,
        keyring,
        int(contract["maximum_keyring_bytes"]),
    )
    if not set(signer_identities).intersection(manifest["signer_fingerprints"]):
        raise PublisherError("InRelease signer is not authorized by the publication")
    validate_release(
        release,
        manifest=manifest,
        records=records,
        contract=contract,
        now_seconds=now_seconds,
    )
    return Publication(
        snapshot=manifest["snapshot"],
        product_revision=manifest["product_revision"],
        date_seconds=manifest["date_seconds"],
        valid_until_seconds=manifest["valid_until_seconds"],
        signers=tuple(manifest["signer_fingerprints"]),
        records=records,
        release_bytes=release,
        manifest_identity=_hash_file(
            stage / str(contract["publication_manifest"])
        ),
        inrelease_identity=_hash_file(stage / INRELEASE_PATH),
    )


def _ensure_directory(path: Path) -> None:
    missing: list[Path] = []
    current = path
    while not current.exists():
        missing.append(current)
        current = current.parent
    if current.is_symlink() or not current.is_dir():
        raise PublisherError("publication destination parent is unsafe")
    for directory in reversed(missing):
        try:
            directory.mkdir(mode=0o755)
        except OSError as error:
            raise PublisherError("publication destination cannot be created") from error


def _reject_linked_destination(root: Path, relative: str) -> None:
    current = root
    parts = PurePosixPath(relative).parts
    for index, part in enumerate(parts):
        current /= part
        if not current.exists() and not current.is_symlink():
            continue
        try:
            metadata = current.lstat()
        except OSError as error:
            raise PublisherError("publication destination cannot be inspected") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise PublisherError("publication destination traverses a link")
        if index < len(parts) - 1 and not stat.S_ISDIR(metadata.st_mode):
            raise PublisherError("publication destination parent is not a directory")


def _copy_stream(source: Path, destination: Path, *, exclusive: bool) -> None:
    _ensure_directory(destination.parent)
    flags = os.O_WRONLY | os.O_CREAT
    flags |= os.O_EXCL if exclusive else os.O_TRUNC
    try:
        descriptor = os.open(destination, flags, 0o644)
        with source.open("rb") as reader, os.fdopen(descriptor, "wb") as writer:
            while chunk := reader.read(CHUNK):
                writer.write(chunk)
            writer.flush()
            os.fsync(writer.fileno())
    except OSError as error:
        try:
            destination.unlink(missing_ok=True)
        except OSError:
            pass
        raise PublisherError("publication file copy failed") from error


def _copy_immutable(source: Path, destination: Path, expected: FileRecord) -> None:
    if destination.exists() or destination.is_symlink():
        try:
            metadata = destination.lstat()
        except OSError as error:
            raise PublisherError("existing immutable object cannot be inspected") from error
        if (
            destination.is_symlink()
            or not stat.S_ISREG(metadata.st_mode)
            or _hash_file(destination)
            != (expected.size, expected.sha256, expected.sha512)
        ):
            raise PublisherError("existing immutable object has different bytes")
        return
    _copy_stream(source, destination, exclusive=True)
    if _hash_file(destination) != (expected.size, expected.sha256, expected.sha512):
        raise PublisherError("published immutable object failed readback")


def _atomic_replace(
    source: Path,
    destination: Path,
    expected: tuple[int, str, str],
) -> None:
    if _hash_file(source) != expected:
        raise PublisherError("publication source changed after verification")
    _ensure_directory(destination.parent)
    temporary = destination.parent / f".{destination.name}.rmac-{os.getpid()}"
    if temporary.exists() or temporary.is_symlink():
        raise PublisherError("publication temporary path already exists")
    _copy_stream(source, temporary, exclusive=True)
    try:
        os.replace(temporary, destination)
        if _hash_file(destination) != expected:
            raise PublisherError("atomic publication failed readback")
        descriptor = os.open(destination.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise PublisherError("atomic publication replace failed") from error


def _current_identity(
    repository: Path,
    keyring: Path,
    contract: dict[str, object],
) -> tuple[int, str] | None:
    current = repository / INRELEASE_PATH
    if not current.exists():
        return None
    release, _ = _verify_inrelease(
        current, keyring, int(contract["maximum_keyring_bytes"])
    )
    fields = _parse_release_fields(release)
    exact = {
        "Acquire-By-Hash": "yes",
        "Architectures": "amd64 arm64",
        "Codename": "resolute",
        "Components": "main",
        "Label": "rmac",
        "Origin": "rmac",
        "Suite": "stable",
    }
    required = set(exact) | {
        "Date",
        "SHA256",
        "SHA512",
        "Signed-By",
        "Valid-Until",
        "X-Rmac-Snapshot",
    }
    if set(fields) != required or any(
        fields[name] != value for name, value in exact.items()
    ):
        raise PublisherError("current signed repository identity is invalid")
    try:
        date = int(parsedate_to_datetime(fields["Date"]).timestamp())
    except (KeyError, TypeError, ValueError, OverflowError) as error:
        raise PublisherError("current repository Date is invalid") from error
    snapshot = fields.get("X-Rmac-Snapshot", "")
    if not SNAPSHOT_RE.fullmatch(snapshot):
        raise PublisherError("current repository snapshot identity is invalid")
    return date, snapshot


def _metadata_paths(
    publication: Publication, contract: dict[str, object]
) -> tuple[str, ...]:
    return (
        INRELEASE_PATH,
        str(contract["publication_manifest"]),
        *(
            record.path
            for record in publication.records
            if record.role == "index"
        ),
        *(
            path
            for record in publication.records
            if record.role == "index"
            for path in _by_hash_paths(record)
        ),
    )


def _metadata_identity(
    relative: str,
    publication: Publication,
    contract: dict[str, object],
) -> tuple[int, str, str]:
    if relative == INRELEASE_PATH:
        return publication.inrelease_identity
    if relative == str(contract["publication_manifest"]):
        return publication.manifest_identity
    for record in publication.records:
        if record.role != "index":
            continue
        if relative == record.path or relative in _by_hash_paths(record):
            return record.size, record.sha256, record.sha512
    raise PublisherError("snapshot metadata path has no reviewed identity")


def _verify_retained_snapshot(
    stage: Path,
    destination: Path,
    publication: Publication,
    contract: dict[str, object],
) -> None:
    expected = set(_metadata_paths(publication, contract))
    actual: set[str] = set()
    try:
        for root, directories, files in os.walk(destination, followlinks=False):
            root_path = Path(root)
            if any((root_path / name).is_symlink() for name in directories):
                raise PublisherError("retained snapshot contains a linked directory")
            for name in files:
                path = root_path / name
                if path.is_symlink() or not path.is_file():
                    raise PublisherError("retained snapshot contains a non-regular file")
                actual.add(path.relative_to(destination).as_posix())
    except OSError as error:
        raise PublisherError("retained snapshot cannot be inspected") from error
    if actual != expected:
        raise PublisherError("retained snapshot inventory differs")
    for relative in expected:
        identity = _metadata_identity(relative, publication, contract)
        if (
            _hash_file(stage / relative) != identity
            or _hash_file(destination / relative) != identity
        ):
            raise PublisherError("retained snapshot bytes differ")


def _snapshot_copy(
    stage: Path,
    repository: Path,
    publication: Publication,
    contract: dict[str, object],
) -> Path:
    state = repository / str(contract["state_directory"])
    snapshots = state / "snapshots"
    _ensure_directory(snapshots)
    destination = snapshots / publication.snapshot
    if destination.exists():
        if destination.is_symlink() or not destination.is_dir():
            raise PublisherError("retained snapshot path is unsafe")
        _verify_retained_snapshot(
            stage, destination, publication, contract
        )
        return destination
    temporary = snapshots / f".{publication.snapshot}-{os.getpid()}"
    if temporary.exists() or temporary.is_symlink():
        raise PublisherError("snapshot temporary path already exists")
    temporary.mkdir(mode=0o755)
    try:
        for relative in _metadata_paths(publication, contract):
            _copy_stream(stage / relative, temporary / relative, exclusive=True)
            if _hash_file(temporary / relative) != _metadata_identity(
                relative, publication, contract
            ):
                raise PublisherError("retained snapshot failed verified readback")
        os.replace(temporary, destination)
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise
    return destination


def _write_state(
    repository: Path,
    publication: Publication,
    contract: dict[str, object],
) -> None:
    state = repository / str(contract["state_directory"])
    snapshots = state / "snapshots"
    retained = sorted(
        path.name
        for path in snapshots.iterdir()
        if path.is_dir() and not path.is_symlink() and SNAPSHOT_RE.fullmatch(path.name)
    )
    document = {
        "date_seconds": publication.date_seconds,
        "format": 1,
        "product_revision": publication.product_revision,
        "retained_snapshots": retained,
        "snapshot": publication.snapshot,
    }
    with tempfile.NamedTemporaryFile(
        mode="wb", dir=state, prefix=".state-", delete=False
    ) as output:
        temporary = Path(output.name)
        output.write((json.dumps(document, sort_keys=True, indent=2) + "\n").encode())
        output.flush()
        os.fsync(output.fileno())
    try:
        os.chmod(temporary, 0o644)
        os.replace(temporary, state / "state.json")
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise PublisherError("publisher state could not be committed") from error


def promote(
    stage: Path,
    repository: Path,
    keyring: Path,
    publication: Publication,
    *,
    retain: int,
    contract: dict[str, object] | None = None,
) -> None:
    contract = load_contract() if contract is None else contract
    if not repository.is_absolute() or repository.is_symlink() or not repository.is_dir():
        raise PublisherError("repository must be an absolute ordinary directory")
    try:
        stage_resolved = stage.resolve(strict=True)
        repository_resolved = repository.resolve(strict=True)
        keyring_resolved = keyring.resolve(strict=True)
    except OSError as error:
        raise PublisherError("publication roots cannot be resolved") from error
    if (
        stage_resolved == repository_resolved
        or stage_resolved.is_relative_to(repository_resolved)
        or repository_resolved.is_relative_to(stage_resolved)
        or keyring_resolved.is_relative_to(stage_resolved)
        or keyring_resolved.is_relative_to(repository_resolved)
    ):
        raise PublisherError("staging, repository, and keyring roots must be separate")
    minimum_retain = int(contract["minimum_retained_snapshots"])
    if retain < minimum_retain or retain > 32:
        raise PublisherError("retained snapshot count is outside the reviewed bound")
    current = _current_identity(repository, keyring, contract)
    intended = (publication.date_seconds, publication.snapshot)
    if current is not None:
        if current == intended:
            if _hash_file(repository / INRELEASE_PATH) != publication.inrelease_identity:
                raise PublisherError("visible snapshot identity has different signed bytes")
        elif publication.date_seconds <= current[0] or publication.snapshot <= current[1]:
            raise PublisherError("publication Date and snapshot must both increase")
    destination_paths = {
        INRELEASE_PATH,
        str(contract["publication_manifest"]),
        str(contract["state_directory"]),
        *(record.path for record in publication.records),
        *(
            path
            for record in publication.records
            if record.role == "index"
            for path in _by_hash_paths(record)
        ),
    }
    for relative in destination_paths:
        _reject_linked_destination(repository, relative)
    needed = sum(
        record.size
        for record in publication.records
        if record.path.startswith("pool/") and not (repository / record.path).exists()
    )
    metadata_bytes = sum(
        _metadata_identity(relative, publication, contract)[0]
        for relative in _metadata_paths(publication, contract)
    )
    # One retained snapshot plus the worst-case temporary/public metadata set.
    needed += metadata_bytes * 2
    free = shutil.disk_usage(repository).free
    minimum_free = int(contract["minimum_free_gib"]) * 1024**3
    if free - needed < minimum_free:
        raise PublisherError("publication would cross the 15 GiB storage floor")

    _snapshot_copy(stage, repository, publication, contract)
    for record in publication.records:
        if record.path.startswith("pool/"):
            _copy_immutable(stage / record.path, repository / record.path, record)
    for record in publication.records:
        if record.role == "index":
            for relative in _by_hash_paths(record):
                _copy_immutable(
                    stage / relative,
                    repository / relative,
                    FileRecord(
                        path=relative,
                        role="index",
                        size=record.size,
                        sha256=record.sha256,
                        sha512=record.sha512,
                    ),
                )
    records = {record.path: record for record in publication.records}
    for relative in contract["required_indices"]:
        record = records[relative]
        _atomic_replace(
            stage / relative,
            repository / relative,
            (record.size, record.sha256, record.sha512),
        )
    manifest_path = str(contract["publication_manifest"])
    _atomic_replace(
        stage / manifest_path,
        repository / manifest_path,
        publication.manifest_identity,
    )
    _atomic_replace(
        stage / INRELEASE_PATH,
        repository / INRELEASE_PATH,
        publication.inrelease_identity,
    )
    snapshots = repository / str(contract["state_directory"]) / "snapshots"
    retained = sorted(
        path
        for path in snapshots.iterdir()
        if path.is_dir() and not path.is_symlink() and SNAPSHOT_RE.fullmatch(path.name)
    )
    for obsolete in retained[:-retain]:
        shutil.rmtree(obsolete)
    _write_state(repository, publication, contract)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-contract", action="store_true")
    parser.add_argument("--staging-dir", type=Path)
    parser.add_argument("--repository-dir", type=Path)
    parser.add_argument("--keyring", type=Path)
    parser.add_argument("--retain", type=int, default=3)
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        supplied = (
            arguments.staging_dir is not None,
            arguments.repository_dir is not None,
            arguments.keyring is not None,
        )
        if arguments.check_contract:
            if any(supplied) or arguments.retain != 3:
                raise PublisherError("--check-contract cannot publish")
            print("rmac atomic APT publisher contract verified")
            return 0
        if not all(supplied):
            raise PublisherError(
                "absolute --staging-dir, --repository-dir, and --keyring are required"
            )
        now_seconds = int(datetime.now(timezone.utc).timestamp())
        publication = validate_staging(
            arguments.staging_dir,
            arguments.keyring,
            now_seconds=now_seconds,
            contract=contract,
        )
        promote(
            arguments.staging_dir,
            arguments.repository_dir,
            arguments.keyring,
            publication,
            retain=arguments.retain,
            contract=contract,
        )
    except PublisherError as error:
        parser.exit(4, f"publish-apt-snapshot: {error}\n")
    print(
        "rmac APT snapshot published "
        f"({publication.snapshot}, {publication.product_revision[:12]})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
