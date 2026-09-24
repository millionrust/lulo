"""Contract for the rmac archive-keyring binary and source packages."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from email.utils import format_datetime
import hashlib
import io
import json
import lzma
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import tempfile


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/apt/keyring-package.json"
TRUST_PATH = REPO_ROOT / "packaging/apt/update-trust.json"
PACKAGE = "rmac-archive-keyring"
MAINTAINER = "Jacob Samas <samasjacob@icloud.com>"
FINGERPRINT_RE = re.compile(r"(?:[0-9A-F]{40}|[0-9A-F]{64})")
VERSION_RE = re.compile(
    r"[0-9]+\.[0-9]+\.[0-9]+(?:~[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?-[1-9][0-9]*"
)
MAX_TOOL_OUTPUT = 1024 * 1024
MAX_ARTIFACT_BYTES = 64 * 1024 * 1024


class KeyringPackageError(RuntimeError):
    """A bounded, privacy-safe keyring package failure."""


@dataclass(frozen=True)
class KeyringIdentity:
    fingerprints: tuple[str, ...]
    bytes: bytes

    @property
    def sha256(self) -> str:
        return hashlib.sha256(self.bytes).hexdigest()

    @property
    def sha512(self) -> str:
        return hashlib.sha512(self.bytes).hexdigest()


def _regular_bytes(path: Path, maximum: int, label: str) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise KeyringPackageError(f"{label} is unavailable") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise KeyringPackageError(f"{label} must be a regular file")
    if metadata.st_size > maximum:
        raise KeyringPackageError(f"{label} exceeds its size limit")
    try:
        value = path.read_bytes()
    except OSError as error:
        raise KeyringPackageError(f"{label} cannot be read") from error
    if len(value) != metadata.st_size:
        raise KeyringPackageError(f"{label} changed while reading")
    return value


def _load_json(path: Path, label: str) -> object:
    try:
        return json.loads(_regular_bytes(path, 1024 * 1024, label))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise KeyringPackageError(f"{label} is invalid JSON") from error


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    document = _load_json(path, "keyring package contract")
    expected = {
        "binary_package": PACKAGE,
        "debian_revision": 1,
        "format": 1,
        "installed_keyring": "/usr/share/keyrings/rmac-archive-keyring.gpg",
        "license": "MIT",
        "maximum_input_bytes": 4 * 1024 * 1024,
        "maximum_primary_keys": 2,
        "source_package": PACKAGE,
        "supported_build_architectures": ["amd64", "arm64"],
    }
    if document != expected:
        raise KeyringPackageError(
            "keyring package contract differs from the reviewed boundary"
        )
    trust = _load_json(TRUST_PATH, "update trust policy")
    if (
        not isinstance(trust, dict)
        or trust.get("client", {}).get("keyring_package") != PACKAGE
        or trust.get("client", {}).get("keyring_path")
        != document["installed_keyring"]
        or trust.get("signing", {}).get("keyring_format") != "binary-openpgp"
    ):
        raise KeyringPackageError("keyring package conflicts with update trust")
    return document


def workspace_version() -> str:
    raw = _regular_bytes(REPO_ROOT / "Cargo.toml", 1024 * 1024, "workspace manifest")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise KeyringPackageError("workspace manifest is not UTF-8") from error
    match = re.search(
        r'(?ms)^\[workspace\.package\]\s*$.*?'
        r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)"\s*$',
        text,
    )
    if match is None:
        raise KeyringPackageError("workspace version is not canonical")
    return match.group(1)


def debian_upstream_version(version: str) -> str:
    """Same Debian tilde convention as native_package_contract.py.

    A Cargo/semver pre-release hyphen (``0.9.0-beta.1``) sorts *after*
    ``0.9.0`` under Debian's comparison rules; replacing the first ``-``
    with ``~`` makes it sort before the final release instead.
    """
    return version.replace("-", "~", 1)


def package_version(contract: dict[str, object] | None = None) -> str:
    contract = load_contract() if contract is None else contract
    return f"{debian_upstream_version(workspace_version())}-{contract['debian_revision']}"


def source_date_epoch(value: object) -> int:
    if type(value) is int:
        parsed = value
    elif isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]*", value):
        parsed = int(value)
    else:
        raise KeyringPackageError(
            "SOURCE_DATE_EPOCH must be canonical decimal seconds"
        )
    if not 0 <= parsed <= 8_589_934_591:
        raise KeyringPackageError("SOURCE_DATE_EPOCH is outside the supported range")
    return parsed


def build_architecture(value: object, contract: dict[str, object]) -> str:
    if not isinstance(value, str) or value not in contract[
        "supported_build_architectures"
    ]:
        raise KeyringPackageError("unsupported keyring build architecture")
    return value


def fingerprints(values: object, maximum: int) -> tuple[str, ...]:
    if (
        not isinstance(values, list)
        or not values
        or len(values) > maximum
        or values != sorted(set(values))
        or any(not isinstance(value, str) or not FINGERPRINT_RE.fullmatch(value) for value in values)
    ):
        raise KeyringPackageError(
            "fingerprints must be unique sorted uppercase primary fingerprints"
        )
    return tuple(values)


def _run(
    command: list[str],
    *,
    environment: dict[str, str],
    cwd: Path,
    label: str,
    timeout: int = 120,
) -> subprocess.CompletedProcess[bytes]:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=environment,
            check=False,
            capture_output=True,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise KeyringPackageError(f"{label} could not run") from error
    if (
        result.returncode != 0
        or len(result.stdout) > MAX_TOOL_OUTPUT
        or len(result.stderr) > MAX_TOOL_OUTPUT
    ):
        raise KeyringPackageError(f"{label} failed")
    return result


def canonicalize_keyring(
    path: Path,
    requested: tuple[str, ...],
    *,
    epoch: int,
    maximum_bytes: int,
) -> KeyringIdentity:
    _regular_bytes(path, maximum_bytes, "public keyring input")
    with tempfile.TemporaryDirectory() as temporary:
        home = Path(temporary)
        home.chmod(0o700)
        environment = {
            "GNUPGHOME": str(home),
            "LC_ALL": "C",
            "PATH": os.environ.get("PATH", ""),
        }
        shown = _run(
            [
                "gpg",
                "--batch",
                "--no-options",
                "--with-colons",
                "--import-options",
                "show-only",
                "--dry-run",
                "--import",
                str(path),
            ],
            environment=environment,
            cwd=home,
            label="public keyring inspection",
        )
        try:
            lines = shown.stdout.decode("utf-8").splitlines()
        except UnicodeDecodeError as error:
            raise KeyringPackageError("GnuPG key inventory is not UTF-8") from error
        if any(line.startswith(("sec:", "ssb:")) for line in lines):
            raise KeyringPackageError("secret key material is forbidden")
        primary: list[str] = []
        signing: dict[str, bool] = {}
        current: str | None = None
        pending_primary = False
        primary_signs = False
        for line in lines:
            fields = line.split(":")
            record = fields[0]
            if record == "pub":
                if (
                    len(fields) < 12
                    or fields[1] in {"e", "r"}
                    or not fields[5].isdecimal()
                    or int(fields[5]) > epoch + 300
                ):
                    raise KeyringPackageError("primary key is not valid at build time")
                expiration = fields[6]
                if expiration not in {"", "0"} and (
                    not expiration.isdecimal() or int(expiration) <= epoch
                ):
                    raise KeyringPackageError("primary key is expired")
                pending_primary = True
                primary_signs = "s" in fields[11].lower()
                current = None
            elif record == "sub":
                if current is None or len(fields) < 12:
                    raise KeyringPackageError("subkey appears before its primary")
                expiration = fields[6]
                valid = fields[1] not in {"e", "r"} and (
                    expiration in {"", "0"}
                    or expiration.isdecimal()
                    and int(expiration) > epoch
                )
                if valid and "s" in fields[11].lower():
                    signing[current] = True
            elif record == "fpr":
                if len(fields) < 10 or not FINGERPRINT_RE.fullmatch(fields[9]):
                    raise KeyringPackageError("GnuPG returned an invalid fingerprint")
                if pending_primary:
                    current = fields[9]
                    primary.append(current)
                    signing[current] = primary_signs
                    pending_primary = False
        if tuple(sorted(primary)) != requested or any(
            not signing.get(value, False) for value in requested
        ):
            raise KeyringPackageError(
                "keyring primary/signing inventory differs from the request"
            )
        _run(
            [
                "gpg",
                "--batch",
                "--no-options",
                "--import-options",
                "import-minimal,import-clean",
                "--import",
                str(path),
            ],
            environment=environment,
            cwd=home,
            label="public keyring import",
        )
        exported = _run(
            [
                "gpg",
                "--batch",
                "--no-options",
                "--export-options",
                "export-minimal",
                "--export",
                *requested,
            ],
            environment=environment,
            cwd=home,
            label="public keyring export",
        ).stdout
    if not exported or len(exported) > maximum_bytes:
        raise KeyringPackageError("canonical public keyring is empty or oversized")
    return KeyringIdentity(requested, exported)


def _copyright() -> bytes:
    license_text = _regular_bytes(
        REPO_ROOT / "LICENSE", 256 * 1024, "MIT license"
    ).decode("utf-8")
    formatted = "\n".join(
        " ." if not line else f" {line}" for line in license_text.rstrip().splitlines()
    )
    return (
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n"
        "Upstream-Name: rmac\n"
        "Source: https://github.com/snehacodex/rmac\n"
        "\n"
        "Files: *\n"
        "Copyright: 2026 rmac contributors\n"
        "License: MIT\n"
        f"{formatted}\n"
    ).encode()


def source_files(
    keyring: KeyringIdentity,
    *,
    version: str,
    epoch: int,
) -> dict[str, tuple[bytes, int]]:
    if not VERSION_RE.fullmatch(version):
        raise KeyringPackageError("keyring package version is invalid")
    date = format_datetime(datetime.fromtimestamp(epoch, timezone.utc), usegmt=True)
    return {
        "LICENSE": (
            _regular_bytes(REPO_ROOT / "LICENSE", 256 * 1024, "MIT license"),
            0o644,
        ),
        "README.md": (
            b"# rmac archive keyring\n\n"
            b"This source package contains only reviewed public OpenPGP archive keys. "
            b"Secret key material is never part of the package.\n",
            0o644,
        ),
        "rmac-archive-keyring.gpg": (keyring.bytes, 0o644),
        "debian/changelog": (
            (
                f"{PACKAGE} ({version}) resolute; urgency=medium\n"
                "\n"
                "  * Publish the package-managed rmac archive public keyring.\n"
                "\n"
                f" -- {MAINTAINER}  {date}\n"
            ).encode(),
            0o644,
        ),
        "debian/control": (
            (
                f"Source: {PACKAGE}\n"
                "Section: misc\n"
                "Priority: optional\n"
                f"Maintainer: {MAINTAINER}\n"
                "Build-Depends: debhelper-compat (= 13)\n"
                "Standards-Version: 4.7.2.0\n"
                "Rules-Requires-Root: no\n"
                "\n"
                f"Package: {PACKAGE}\n"
                "Architecture: all\n"
                "Multi-Arch: foreign\n"
                "Description: OpenPGP trust anchor for the rmac APT repository\n"
                " Installs only the public keyring used by the isolated rmac source.\n"
            ).encode(),
            0o644,
        ),
        "debian/copyright": (_copyright(), 0o644),
        "debian/docs": (b"README.md\n", 0o644),
        "debian/install": (
            b"rmac-archive-keyring.gpg usr/share/keyrings\n",
            0o644,
        ),
        "debian/rules": (
            b"#!/usr/bin/make -f\n\n"
            b"%:\n"
            b"\tdh $@\n\n"
            b"override_dh_builddeb:\n"
            b"\tdh_builddeb -- -Zxz -z9\n",
            0o755,
        ),
        "debian/source/format": (b"3.0 (quilt)\n", 0o644),
    }


def _tar_xz(
    entries: dict[str, tuple[bytes, int]],
    *,
    root: str,
    epoch: int,
) -> bytes:
    directories = {root}
    for relative in entries:
        path = PurePosixPath(root) / relative
        directories.update(parent.as_posix() for parent in path.parents if parent.as_posix() != ".")
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for directory in sorted(directories):
            info = tarfile.TarInfo(directory)
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            info.mtime = epoch
            archive.addfile(info)
        for relative, (value, mode) in sorted(entries.items()):
            info = tarfile.TarInfo((PurePosixPath(root) / relative).as_posix())
            info.type = tarfile.REGTYPE
            info.mode = mode
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            info.mtime = epoch
            info.size = len(value)
            archive.addfile(info, io.BytesIO(value))
    return lzma.compress(
        output.getvalue(),
        format=lzma.FORMAT_XZ,
        check=lzma.CHECK_CRC64,
        preset=9,
    )


def orig_tar_bytes(
    keyring: KeyringIdentity,
    *,
    version: str,
    epoch: int,
) -> bytes:
    upstream = version.rsplit("-", 1)[0]
    upstream_files = {
        name: record
        for name, record in source_files(
            keyring, version=version, epoch=epoch
        ).items()
        if not name.startswith("debian/")
    }
    return _tar_xz(
        upstream_files,
        root=f"{PACKAGE}-{upstream}",
        epoch=epoch,
    )


def write_source_tree(
    root: Path,
    keyring: KeyringIdentity,
    *,
    version: str,
    epoch: int,
) -> None:
    if root.exists() or root.is_symlink():
        raise KeyringPackageError("source tree destination must not exist")
    try:
        root.mkdir(mode=0o755)
        for relative, (value, mode) in source_files(
            keyring, version=version, epoch=epoch
        ).items():
            path = root / relative
            path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
            path.write_bytes(value)
            path.chmod(mode)
        for path in sorted(root.rglob("*"), reverse=True):
            os.utime(path, (epoch, epoch), follow_symlinks=False)
        os.utime(root, (epoch, epoch), follow_symlinks=False)
    except OSError as error:
        raise KeyringPackageError("keyring source tree cannot be staged") from error


def artifact_names(version: str) -> dict[str, str]:
    upstream = version.rsplit("-", 1)[0]
    base = f"{PACKAGE}_{version}"
    return {
        "binary": f"{base}_all.deb",
        "orig": f"{PACKAGE}_{upstream}.orig.tar.xz",
        "debian": f"{base}.debian.tar.xz",
        "dsc": f"{base}.dsc",
        "buildinfo": f"{base}_all.buildinfo",
        "changes": f"{base}_all.changes",
    }


def _sha(path: Path) -> tuple[int, str, str]:
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
        raise KeyringPackageError("package artifact cannot be read") from error
    return size, sha256.hexdigest(), sha512.hexdigest()


def create_manifest(
    directory: Path,
    *,
    keyring: KeyringIdentity,
    version: str,
    architecture: str,
    epoch: int,
    tool_versions: dict[str, str],
) -> tuple[dict[str, object], bytes]:
    names = artifact_names(version)
    roles = {
        names["binary"]: "binary-package",
        names["orig"]: "source-tar",
        names["debian"]: "debian-source-tar",
        names["dsc"]: "source-control",
        names["buildinfo"]: "build-record",
        names["changes"]: "upload-record",
    }
    records = {}
    sums = []
    for name, role in sorted(roles.items()):
        size, sha256, sha512 = _sha(directory / name)
        records[name] = {
            "role": role,
            "sha256": sha256,
            "sha512": sha512,
            "size": size,
        }
        sums.append(f"{sha256}  {name}\n")
    manifest = {
        "artifacts": records,
        "build_architecture": architecture,
        "format": 1,
        "keyring": {
            "fingerprints": list(keyring.fingerprints),
            "installed_path": "/usr/share/keyrings/rmac-archive-keyring.gpg",
            "sha256": keyring.sha256,
            "sha512": keyring.sha512,
            "size": len(keyring.bytes),
        },
        "package": PACKAGE,
        "package_architecture": "all",
        "source_date_epoch": epoch,
        "source_package": PACKAGE,
        "tool_versions": tool_versions,
        "version": version,
    }
    return manifest, "".join(sums).encode("ascii")


def _parse_ar(value: bytes) -> dict[str, bytes]:
    if not value.startswith(b"!<arch>\n"):
        raise KeyringPackageError("binary package is not an ar archive")
    offset = 8
    members: dict[str, bytes] = {}
    while offset < len(value):
        if offset + 60 > len(value):
            raise KeyringPackageError("binary package ar header is truncated")
        header = value[offset : offset + 60]
        offset += 60
        if header[58:60] != b"`\n":
            raise KeyringPackageError("binary package ar header is invalid")
        try:
            name = header[:16].decode("ascii").strip().removesuffix("/")
            size = int(header[48:58].decode("ascii").strip())
        except (UnicodeDecodeError, ValueError) as error:
            raise KeyringPackageError("binary package ar metadata is invalid") from error
        if not name or name in members or size < 0 or offset + size > len(value):
            raise KeyringPackageError("binary package ar inventory is invalid")
        members[name] = value[offset : offset + size]
        offset += size + size % 2
    if offset != len(value):
        raise KeyringPackageError("binary package ar padding is invalid")
    return members


def _parse_tar_xz(value: bytes, label: str) -> dict[str, tuple[bytes | None, int]]:
    try:
        raw = lzma.decompress(
            value, format=lzma.FORMAT_XZ, memlimit=128 * 1024 * 1024
        )
    except lzma.LZMAError as error:
        raise KeyringPackageError(f"{label} is not valid xz") from error
    if len(raw) > MAX_ARTIFACT_BYTES:
        raise KeyringPackageError(f"{label} expands beyond its limit")
    result: dict[str, tuple[bytes | None, int]] = {}
    try:
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
            for member in archive.getmembers():
                if member.name in result or member.issym() or member.islnk():
                    raise KeyringPackageError(f"{label} contains a link or duplicate")
                if member.isdir():
                    result[member.name] = (None, member.mode)
                elif member.isfile():
                    source = archive.extractfile(member)
                    if source is None:
                        raise KeyringPackageError(f"{label} file is unreadable")
                    result[member.name] = (source.read(), member.mode)
                else:
                    raise KeyringPackageError(f"{label} contains a special file")
    except (OSError, tarfile.TarError) as error:
        raise KeyringPackageError(f"{label} cannot be parsed") from error
    return result


def extract_binary_keyring(binary: bytes) -> bytes:
    members = _parse_ar(binary)
    if (
        tuple(members) != ("debian-binary", "control.tar.xz", "data.tar.xz")
        or members["debian-binary"] != b"2.0\n"
    ):
        raise KeyringPackageError("binary package member inventory is not exact")
    control = _parse_tar_xz(members["control.tar.xz"], "control archive")
    forbidden = {"preinst", "postinst", "prerm", "postrm", "triggers", "conffiles"}
    if any(PurePosixPath(path).name in forbidden for path in control):
        raise KeyringPackageError("keyring package contains a maintainer hook")
    data = _parse_tar_xz(members["data.tar.xz"], "data archive")
    candidates = [
        value
        for path, value in data.items()
        if path.lstrip("./") == "usr/share/keyrings/rmac-archive-keyring.gpg"
    ]
    if len(candidates) != 1 or candidates[0][0] is None or candidates[0][1] != 0o644:
        raise KeyringPackageError("installed keyring payload is invalid")
    return candidates[0][0]


def _deb822(path: Path, label: str) -> dict[str, str]:
    try:
        text = _regular_bytes(path, 4 * 1024 * 1024, label).decode("utf-8")
    except UnicodeDecodeError as error:
        raise KeyringPackageError(f"{label} is not UTF-8") from error
    if not text.endswith("\n") or "\x00" in text or text.startswith("-----BEGIN"):
        raise KeyringPackageError(f"{label} encoding or signature boundary is invalid")
    fields: dict[str, str] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith((" ", "\t")):
            if current is None:
                raise KeyringPackageError(f"{label} continuation is invalid")
            fields[current] = (
                line[1:]
                if not fields[current]
                else fields[current] + "\n" + line[1:]
            )
            continue
        name, separator, value = line.partition(":")
        if not separator or name in fields:
            raise KeyringPackageError(f"{label} fields are invalid")
        fields[name] = value.lstrip()
        current = name
    return fields


def _checksum_names(value: str, columns: int, label: str) -> set[str]:
    names = set()
    for line in value.splitlines():
        parts = line.split()
        if len(parts) != columns or parts[-1] in names:
            raise KeyringPackageError(f"{label} checksum inventory is invalid")
        names.add(parts[-1])
    if not names:
        raise KeyringPackageError(f"{label} checksum inventory is empty")
    return names


def verify_directory(
    directory: Path,
    *,
    expected_architecture: str | None = None,
    run_dpkg_source: bool = True,
) -> dict[str, object]:
    contract = load_contract()
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise KeyringPackageError(
            "keyring package directory must be an absolute ordinary directory"
        )
    manifest = _load_json(directory / "keyring-packages.json", "package manifest")
    if not isinstance(manifest, dict) or set(manifest) != {
        "artifacts",
        "build_architecture",
        "format",
        "keyring",
        "package",
        "package_architecture",
        "source_date_epoch",
        "source_package",
        "tool_versions",
        "version",
    }:
        raise KeyringPackageError("keyring package manifest fields are not exact")
    architecture = build_architecture(manifest["build_architecture"], contract)
    if expected_architecture is not None and architecture != expected_architecture:
        raise KeyringPackageError("keyring build architecture differs from expectation")
    if (
        manifest["format"] != 1
        or manifest["package"] != PACKAGE
        or manifest["source_package"] != PACKAGE
        or manifest["package_architecture"] != "all"
        or manifest["version"] != package_version(contract)
    ):
        raise KeyringPackageError("keyring package identity is invalid")
    epoch = source_date_epoch(manifest["source_date_epoch"])
    tool_versions = manifest["tool_versions"]
    if (
        not isinstance(tool_versions, dict)
        or set(tool_versions) != {
            "dpkg-buildpackage",
            "dpkg-deb",
            "dpkg-source",
        }
        or any(
            not isinstance(value, str)
            or not value
            or len(value) > 256
            or "\n" in value
            for value in tool_versions.values()
        )
    ):
        raise KeyringPackageError("Debian tool version inventory is invalid")
    keyring_record = manifest["keyring"]
    if not isinstance(keyring_record, dict) or set(keyring_record) != {
        "fingerprints",
        "installed_path",
        "sha256",
        "sha512",
        "size",
    }:
        raise KeyringPackageError("keyring identity fields are not exact")
    primary = fingerprints(
        keyring_record["fingerprints"], int(contract["maximum_primary_keys"])
    )
    names = artifact_names(manifest["version"])
    expected_files = {
        "SHA256SUMS",
        "keyring-packages.json",
        *names.values(),
    }
    try:
        entries = list(directory.iterdir())
    except OSError as error:
        raise KeyringPackageError("package directory cannot be inspected") from error
    if (
        {path.name for path in entries} != expected_files
        or any(path.is_symlink() or not path.is_file() for path in entries)
    ):
        raise KeyringPackageError("keyring package output inventory is not exact")
    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, dict) or set(artifacts) != set(names.values()):
        raise KeyringPackageError("artifact manifest inventory is not exact")
    for name, record in artifacts.items():
        if not isinstance(record, dict) or set(record) != {
            "role",
            "sha256",
            "sha512",
            "size",
        }:
            raise KeyringPackageError("artifact record fields are not exact")
        actual = _sha(directory / name)
        if actual != (record["size"], record["sha256"], record["sha512"]):
            raise KeyringPackageError("artifact differs from its manifest")
    sums = "".join(
        f"{record['sha256']}  {name}\n"
        for name, record in sorted(artifacts.items())
    ).encode()
    if _regular_bytes(directory / "SHA256SUMS", 1024 * 1024, "SHA256SUMS") != sums:
        raise KeyringPackageError("SHA256SUMS differs from the artifact manifest")
    binary = _regular_bytes(
        directory / names["binary"], MAX_ARTIFACT_BYTES, "binary package"
    )
    packaged_keyring = extract_binary_keyring(binary)
    with tempfile.TemporaryDirectory() as temporary:
        path = Path(temporary) / "rmac-archive-keyring.gpg"
        path.write_bytes(packaged_keyring)
        canonical = canonicalize_keyring(
            path,
            primary,
            epoch=epoch,
            maximum_bytes=int(contract["maximum_input_bytes"]),
        )
    if (
        packaged_keyring != canonical.bytes
        or keyring_record["installed_path"] != contract["installed_keyring"]
        or keyring_record["size"] != len(canonical.bytes)
        or keyring_record["sha256"] != canonical.sha256
        or keyring_record["sha512"] != canonical.sha512
    ):
        raise KeyringPackageError("packaged public keyring identity is invalid")
    orig = _parse_tar_xz(
        _regular_bytes(
            directory / names["orig"], MAX_ARTIFACT_BYTES, "original source tar"
        ),
        "original source tar",
    )
    source_keyrings = [
        value
        for path, value in orig.items()
        if path.endswith("/rmac-archive-keyring.gpg")
    ]
    if len(source_keyrings) != 1 or source_keyrings[0][0] != canonical.bytes:
        raise KeyringPackageError("source and binary public keyrings differ")

    dsc = _deb822(directory / names["dsc"], "source control")
    buildinfo = _deb822(directory / names["buildinfo"], "build record")
    changes = _deb822(directory / names["changes"], "upload record")
    if (
        dsc.get("Format") != "3.0 (quilt)"
        or dsc.get("Source") != PACKAGE
        or dsc.get("Binary") != PACKAGE
        or dsc.get("Version") != manifest["version"]
        or not {"Checksums-Sha1", "Checksums-Sha256", "Files"}.issubset(dsc)
    ):
        raise KeyringPackageError("source control identity is invalid")
    if (
        buildinfo.get("Format") != "1.0"
        or buildinfo.get("Source") != PACKAGE
        or buildinfo.get("Binary") != PACKAGE
        or buildinfo.get("Version") != manifest["version"]
        or buildinfo.get("Build-Architecture") != architecture
        or not {
            "Checksums-Md5",
            "Checksums-Sha1",
            "Checksums-Sha256",
            "Installed-Build-Depends",
        }.issubset(buildinfo)
    ):
        raise KeyringPackageError("build record identity is invalid")
    if (
        changes.get("Format") != "1.8"
        or changes.get("Source") != PACKAGE
        or changes.get("Binary") != PACKAGE
        or changes.get("Version") != manifest["version"]
        or changes.get("Distribution") != "resolute"
        or not {
            "Changes",
            "Checksums-Sha1",
            "Checksums-Sha256",
            "Files",
        }.issubset(changes)
    ):
        raise KeyringPackageError("upload record identity is invalid")
    dsc_names = _checksum_names(dsc["Checksums-Sha256"], 3, "source control")
    if dsc_names != {names["orig"], names["debian"]}:
        raise KeyringPackageError("source control artifact inventory is not exact")
    upload_names = _checksum_names(changes["Checksums-Sha256"], 3, "upload")
    expected_upload = set(names.values()) - {names["changes"]}
    if upload_names != expected_upload or _checksum_names(
        changes["Files"], 5, "upload Files"
    ) != expected_upload:
        raise KeyringPackageError("upload artifact inventory is not exact")
    if run_dpkg_source:
        with tempfile.TemporaryDirectory() as temporary:
            environment = {
                "HOME": temporary,
                "LC_ALL": "C.UTF-8",
                "PATH": os.environ.get("PATH", ""),
            }
            _run(
                [
                    "dpkg-source",
                    "--no-copy",
                    "--require-strong-checksums",
                    "-x",
                    str(directory / names["dsc"]),
                    str(Path(temporary) / "source"),
                ],
                environment=environment,
                cwd=directory,
                label="standard source extraction",
            )
    return manifest
