#!/usr/bin/env python3
"""Verify an H2 rmac Debian package set without installing it."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile

from native_package_contract import (
    ARCHITECTURES,
    FORMAT_VERSION,
    PACKAGE_SPECS,
    ContractError,
    combined_dependencies,
    control_bytes,
    dependency_entries,
    inspect_elf,
    maintainer_scripts,
    native_version,
    package_filename,
    resolved_static_dependencies,
    source_date_epoch,
)


REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_NAME = "native-packages.json"
CHECKSUM_NAME = "SHA256SUMS"
MAX_MANIFEST_BYTES = 256 * 1024
MAX_CONTROL_BYTES = 128 * 1024
MAX_TOOL_OUTPUT_BYTES = 1024 * 1024


class VerificationError(RuntimeError):
    """A privacy-safe native package verification failure."""


def _load_script(name: str, filename: str):
    path = Path(__file__).with_name(filename)
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise VerificationError("package payload verifier cannot be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def _regular_bytes(path: Path, maximum: int | None = None) -> tuple[bytes, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VerificationError(f"required package-set file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VerificationError(f"required package-set path is not regular: {path.name}")
    if maximum is not None and metadata.st_size > maximum:
        raise VerificationError(f"required package-set file is too large: {path.name}")
    try:
        contents = path.read_bytes()
    except OSError as error:
        raise VerificationError(f"required package-set file cannot be read: {path.name}") from error
    if len(contents) != metadata.st_size:
        raise VerificationError(f"required package-set file changed while reading: {path.name}")
    return contents, stat.S_IMODE(metadata.st_mode)


def _sha256(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
                size += len(chunk)
    except OSError as error:
        raise VerificationError("native package archive cannot be read") from error
    return digest.hexdigest(), size


def _regular_mode(path: Path) -> tuple[int, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VerificationError(
            f"required package-set file is unavailable: {path.name}"
        ) from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VerificationError(
            f"required package-set path is not regular: {path.name}"
        )
    return stat.S_IMODE(metadata.st_mode), metadata.st_size


def _with_parent_directories(files: set[Path]) -> set[Path]:
    entries = set(files)
    for path in files:
        entries.update(parent for parent in path.parents if parent != Path("."))
    return entries


def _payload_entries(root: Path) -> set[Path]:
    entries: set[Path] = set()

    def failed(error: OSError) -> None:
        raise VerificationError("extracted package tree cannot be inspected") from error

    for current, directories, filenames in os.walk(
        root, followlinks=False, onerror=failed
    ):
        current_path = Path(current)
        relative_current = current_path.relative_to(root)
        if relative_current == Path(".") and "DEBIAN" in directories:
            directories.remove("DEBIAN")
        for directory in list(directories):
            path = current_path / directory
            entries.add(path.relative_to(root))
            if path.is_symlink():
                directories.remove(directory)
            elif stat.S_IMODE(path.stat().st_mode) != 0o755:
                raise VerificationError(
                    f"package directory has the wrong mode: {path.name}"
                )
        for filename in filenames:
            entries.add((current_path / filename).relative_to(root))
    return entries


def _control_entries(root: Path) -> set[Path]:
    control = root / "DEBIAN"
    if (
        control.is_symlink()
        or not control.is_dir()
        or stat.S_IMODE(control.stat().st_mode) != 0o755
    ):
        raise VerificationError("extracted package control directory is invalid")
    try:
        entries = {path.relative_to(root) for path in control.iterdir()}
    except OSError as error:
        raise VerificationError("extracted package control cannot be inspected") from error
    return {Path("DEBIAN")} | entries


def _run_extract(dpkg_deb: str, archive: Path, destination: Path) -> None:
    try:
        result = subprocess.run(
            [dpkg_deb, "--raw-extract", str(archive), str(destination)],
            check=False,
            capture_output=True,
            timeout=300,
            env={
                **os.environ,
                "DPKG_COLORS": "never",
                "DPKG_NLS": "0",
                "LC_ALL": "C",
            },
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError("native package extraction could not run") from error
    if (
        len(result.stdout) > MAX_TOOL_OUTPUT_BYTES
        or len(result.stderr) > MAX_TOOL_OUTPUT_BYTES
    ):
        raise VerificationError("native package extraction produced excessive output")
    if result.returncode != 0:
        raise VerificationError("native package extraction failed")


def _load_manifest(directory: Path) -> dict[str, object]:
    raw, mode = _regular_bytes(directory / MANIFEST_NAME, MAX_MANIFEST_BYTES)
    if mode != 0o644:
        raise VerificationError("native package manifest has the wrong mode")
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("native package manifest is invalid") from error
    if not isinstance(document, dict) or set(document) != {
        "architecture",
        "format",
        "packages",
        "source_date_epoch",
        "version",
    }:
        raise VerificationError("native package manifest fields are not exact")
    return document


def _require_string_list(value: object, label: str) -> tuple[str, ...]:
    if (
        not isinstance(value, list)
        or any(not isinstance(entry, str) for entry in value)
        or len(value) != len(set(value))
    ):
        raise VerificationError(f"{label} inventory is invalid")
    return tuple(value)


def _verify_binary_records(
    root: Path,
    *,
    specification,
    records: object,
    architecture: str,
) -> set[Path]:
    if not isinstance(records, list) or len(records) != len(specification.binaries):
        raise VerificationError("native package binary inventory is invalid")
    expected_paths: set[Path] = set()
    for expected_name, record in zip(specification.binaries, records, strict=True):
        if not isinstance(record, dict) or set(record) != {
            "name",
            "path",
            "sha256",
            "size",
        }:
            raise VerificationError("native package binary record is invalid")
        expected_path = f"/{specification.install_directory}/{expected_name}"
        if record.get("name") != expected_name or record.get("path") != expected_path:
            raise VerificationError("native package binary identity is invalid")
        digest = record.get("sha256")
        size = record.get("size")
        if (
            not isinstance(digest, str)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)
            or type(size) is not int
            or size <= 0
        ):
            raise VerificationError("native package binary fingerprint is invalid")
        relative = Path(expected_path.removeprefix("/"))
        path = root / relative
        try:
            metadata = path.lstat()
        except OSError as error:
            raise VerificationError(f"packaged binary is unavailable: {expected_name}") from error
        if stat.S_IMODE(metadata.st_mode) != 0o755:
            raise VerificationError(f"packaged binary has the wrong mode: {expected_name}")
        try:
            inspected = inspect_elf(path, architecture)
        except ContractError as error:
            raise VerificationError(str(error)) from error
        if inspected.sha256 != digest or inspected.size != size:
            raise VerificationError(f"packaged binary fingerprint differs: {expected_name}")
        expected_paths.add(relative)
    return expected_paths


def verify_directory(
    directory: Path,
    *,
    architecture: str,
    expected_version: str,
    dpkg_deb: str,
) -> None:
    if architecture not in ARCHITECTURES:
        raise VerificationError("unsupported Debian architecture")
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise VerificationError(
            "native package directory must be an absolute ordinary directory"
        )
    expected_filenames = {
        package_filename(spec, expected_version, architecture)
        for spec in PACKAGE_SPECS
    }
    expected_inventory = expected_filenames | {MANIFEST_NAME, CHECKSUM_NAME}
    try:
        actual_inventory = {path.name for path in directory.iterdir()}
    except OSError as error:
        raise VerificationError("native package directory cannot be inspected") from error
    if actual_inventory != expected_inventory:
        raise VerificationError("native package directory inventory is not exact")

    document = _load_manifest(directory)
    if (
        document.get("format") != FORMAT_VERSION
        or type(document.get("format")) is not int
        or document.get("architecture") != architecture
        or document.get("version") != expected_version
    ):
        raise VerificationError("native package manifest identity is invalid")
    try:
        source_date_epoch(document.get("source_date_epoch"))
    except ContractError as error:
        raise VerificationError(str(error)) from error
    packages = document.get("packages")
    if not isinstance(packages, list) or len(packages) != len(PACKAGE_SPECS):
        raise VerificationError("native package manifest inventory is invalid")

    checksums_raw, checksums_mode = _regular_bytes(
        directory / CHECKSUM_NAME, MAX_MANIFEST_BYTES
    )
    if checksums_mode != 0o644:
        raise VerificationError("native package checksum manifest has the wrong mode")
    checksum_lines = []
    app_verifier = _load_script(
        "rmac_native_application_payload_verifier",
        "verify-application-package.py",
    )
    session_verifier = _load_script(
        "rmac_native_session_payload_verifier",
        "verify-session-package.py",
    )
    payload_verifiers = {
        "rmac-apps": app_verifier,
        "rmac-session": session_verifier,
    }

    with tempfile.TemporaryDirectory(prefix="rmac-native-verify-") as temporary:
        temporary_root = Path(temporary)
        for specification, record in zip(PACKAGE_SPECS, packages, strict=True):
            if not isinstance(record, dict) or set(record) != {
                "binaries",
                "depends",
                "filename",
                "package",
                "recommends",
                "sha256",
                "shared_library_dependencies",
                "size",
                "static_dependencies",
            }:
                raise VerificationError("native package manifest record is invalid")
            filename = package_filename(
                specification, expected_version, architecture
            )
            digest = record.get("sha256")
            size = record.get("size")
            if (
                record.get("package") != specification.name
                or record.get("filename") != filename
                or not isinstance(digest, str)
                or not re.fullmatch(r"[0-9a-f]{64}", digest)
                or type(size) is not int
                or size <= 0
            ):
                raise VerificationError("native package archive record is invalid")
            archive = directory / filename
            mode, archive_size = _regular_mode(archive)
            if mode != 0o644 or archive_size != size:
                raise VerificationError("native package archive has the wrong mode")
            actual_digest, actual_size = _sha256(archive)
            if actual_digest != digest or actual_size != size:
                raise VerificationError("native package archive fingerprint differs")
            checksum_lines.append(f"{digest}  {filename}\n")

            static_dependencies = _require_string_list(
                record.get("static_dependencies"), "static dependency"
            )
            shared_dependencies = _require_string_list(
                record.get("shared_library_dependencies"),
                "shared-library dependency",
            )
            dependencies = _require_string_list(
                record.get("depends"), "combined dependency"
            )
            recommends = _require_string_list(
                record.get("recommends"), "recommendation"
            )
            try:
                if (
                    static_dependencies
                    != resolved_static_dependencies(specification, expected_version)
                    or not shared_dependencies
                    or shared_dependencies
                    != dependency_entries(", ".join(shared_dependencies))
                    or dependencies
                    != combined_dependencies(
                        specification, expected_version, shared_dependencies
                    )
                    or recommends != tuple(specification.recommends)
                ):
                    raise VerificationError(
                        "native package dependency contract differs"
                    )
            except ContractError as error:
                raise VerificationError(str(error)) from error

            extracted = temporary_root / specification.name
            _run_extract(dpkg_deb, archive, extracted)
            try:
                scripts = maintainer_scripts(REPO_ROOT, specification)
            except ContractError as error:
                raise VerificationError(str(error)) from error
            if _control_entries(extracted) != {
                Path("DEBIAN"),
                Path("DEBIAN/control"),
            } | {Path("DEBIAN") / name for name in scripts}:
                raise VerificationError("native package control inventory is not exact")
            for name, expected_script in scripts.items():
                script, script_mode = _regular_bytes(
                    extracted / "DEBIAN" / name, MAX_CONTROL_BYTES
                )
                if script_mode != 0o755 or script != expected_script:
                    raise VerificationError(
                        f"native package maintainer script differs: {name}"
                    )
            control, control_mode = _regular_bytes(
                extracted / "DEBIAN/control", MAX_CONTROL_BYTES
            )
            if control_mode != 0o644 or control != control_bytes(
                specification,
                version=expected_version,
                architecture=architecture,
                dependencies=dependencies,
            ):
                raise VerificationError("native package control metadata differs")

            try:
                payload_verifiers[specification.name].verify_tree(
                    extracted, exact_tree=False
                )
            except Exception as error:
                raise VerificationError(
                    f"{specification.name} immutable payload verification failed"
                ) from error
            binary_paths = _verify_binary_records(
                extracted,
                specification=specification,
                records=record.get("binaries"),
                architecture=architecture,
            )
            base_files = set(payload_verifiers[specification.name].EXPECTED_PATHS)
            base_files.add(payload_verifiers[specification.name].MANIFEST)
            expected_payload = _with_parent_directories(base_files | binary_paths)
            if _payload_entries(extracted) != expected_payload:
                raise VerificationError(
                    f"{specification.name} payload contains an unexpected path"
                )

    try:
        checksum_text = checksums_raw.decode("ascii")
    except UnicodeDecodeError as error:
        raise VerificationError("native package checksum manifest is not ASCII") from error
    if checksum_text != "".join(checksum_lines):
        raise VerificationError("native package checksum manifest differs")


def _require_dpkg_deb() -> str:
    candidate = shutil.which("dpkg-deb")
    if candidate is None or not os.access(candidate, os.X_OK):
        raise VerificationError("required Debian tool is unavailable: dpkg-deb")
    return candidate


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument(
        "--architecture",
        required=True,
        choices=tuple(ARCHITECTURES),
    )
    parser.add_argument(
        "--version",
        help="expected Debian version (defaults to the workspace version)",
    )
    arguments = parser.parse_args()
    try:
        expected_version = arguments.version or native_version(REPO_ROOT)
        verify_directory(
            arguments.directory,
            architecture=arguments.architecture,
            expected_version=expected_version,
            dpkg_deb=_require_dpkg_deb(),
        )
    except (ContractError, VerificationError) as error:
        parser.exit(4, f"verify-native-packages: {error}\n")
    print(
        f"verified {arguments.architecture} rmac native package set "
        f"{expected_version}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
