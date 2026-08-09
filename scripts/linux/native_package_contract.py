"""Shared, side-effect-free contract for rmac native Debian packages."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import re
import stat
import struct


FORMAT_VERSION = 1
DEBIAN_REVISION = 3
MAX_BINARY_BYTES = 1024 * 1024 * 1024
MAX_SOURCE_DATE_EPOCH = 253_402_300_799
ARCHITECTURES = {
    "amd64": 62,
    "arm64": 183,
}
MAINTAINER = "Jacob Samas <samasjacob@icloud.com>"


class ContractError(RuntimeError):
    """A deterministic, privacy-safe native package contract failure."""


@dataclass(frozen=True)
class PackageSpec:
    name: str
    section: str
    install_directory: str
    binaries: tuple[str, ...]
    static_dependencies: tuple[str, ...]
    recommends: tuple[str, ...]
    summary: str
    description: str


@dataclass(frozen=True)
class BinaryRecord:
    name: str
    architecture: str
    size: int
    sha256: str


APPLICATION_BINARIES = (
    "rmac-app-drawer",
    "rmac-files",
    "rmac-notes",
    "rmac-system-monitor",
    "rmac-system-settings",
    "rmac-terminal",
    "rmac-text-editor",
)

SESSION_BINARIES = (
    "rmac-session-supervisor",
    "rmac-wallpaper",
    "rmac-top-bar",
    "rmac-dock",
    "rmac-osd",
    "rmac-launcher",
    "rmac-app-drawer",
    "rmac-quick-settings",
    "rmac-notification-center-panel",
    "rmac-system-settings",
    "rmac-notification-center",
    "rmac-focus-service",
    "rmac-shortcut-broker",
    "rmac-shortcut-dispatch",
    "rmac-lock-provider",
    "rmac-locker",
    "rmac-lock-coordinator",
    "rmac-idle-locker",
)

PACKAGE_SPECS = (
    PackageSpec(
        name="rmac-apps",
        section="utils",
        install_directory="usr/bin",
        binaries=APPLICATION_BINARIES,
        static_dependencies=(
            "bluez",
            "dbus-user-session",
            "libglib2.0-bin",
            "network-manager",
            "packagekit",
            "pipewire-bin",
            "power-profiles-daemon",
            "upower",
            "wireplumber",
            "xdg-desktop-portal",
            "xdg-utils",
        ),
        recommends=(),
        summary="macOS-inspired applications for the rmac Linux desktop",
        description=(
            "Provides Files, Terminal, Notes, Text Editor, System Monitor, "
            "Applications, and Settings with their original rmac desktop "
            "metadata and assets."
        ),
    ),
    PackageSpec(
        name="rmac-session",
        section="x11",
        install_directory="usr/libexec/rmac",
        binaries=SESSION_BINARIES,
        static_dependencies=(
            "coreutils",
            "dbus-user-session",
            "gawk | mawk",
            "niri",
            "libpam0g",
            "pipewire-bin",
            "rmac-apps (= {version})",
            "swayidle",
            "swaylock",
            "systemd",
            "xdg-desktop-portal",
            "xdg-desktop-portal-gnome",
            "xdg-desktop-portal-gtk",
        ),
        recommends=("gdm3",),
        summary="niri-based rmac Wayland desktop session",
        description=(
            "Provides the supervised rmac shell services, GDM session entry, "
            "portal selection, secure-lock integration, and immutable session "
            "defaults while retaining stock GNOME as the recovery session."
        ),
    ),
)

ALL_BINARIES = tuple(
    sorted({binary for package in PACKAGE_SPECS for binary in package.binaries})
)


def workspace_version(repo_root: Path) -> str:
    """Read the single workspace package version without a TOML dependency."""
    path = repo_root / "Cargo.toml"
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise ContractError("workspace manifest is unavailable") from error
    if len(raw) > 1024 * 1024 or b"\0" in raw:
        raise ContractError("workspace manifest is invalid")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ContractError("workspace manifest is not UTF-8") from error
    match = re.search(
        r"(?ms)^\[workspace\.package\]\s*$.*?^version\s*=\s*\"([^\"]+)\"\s*$",
        text,
    )
    if match is None or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){2}", match.group(1)):
        raise ContractError("workspace package version is not canonical")
    return match.group(1)


def native_version(repo_root: Path) -> str:
    return f"{workspace_version(repo_root)}-{DEBIAN_REVISION}"


def source_date_epoch(value: object) -> int:
    """Parse an explicit reproducible-build timestamp."""
    if type(value) is int:
        parsed = value
    elif isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]*", value):
        parsed = int(value)
    else:
        raise ContractError("SOURCE_DATE_EPOCH must be canonical decimal seconds")
    if not 0 <= parsed <= MAX_SOURCE_DATE_EPOCH:
        raise ContractError("SOURCE_DATE_EPOCH is outside the supported range")
    return parsed


def _hash_regular(path: Path, expected_size: int) -> str:
    digest = hashlib.sha256()
    remaining = expected_size
    try:
        with path.open("rb") as source:
            while remaining:
                chunk = source.read(min(1024 * 1024, remaining))
                if not chunk:
                    raise ContractError(f"binary changed while reading: {path.name}")
                digest.update(chunk)
                remaining -= len(chunk)
            if source.read(1):
                raise ContractError(f"binary changed while reading: {path.name}")
    except OSError as error:
        raise ContractError(f"binary cannot be read: {path.name}") from error
    return digest.hexdigest()


def inspect_elf(path: Path, architecture: str) -> BinaryRecord:
    """Validate and fingerprint one native ELF64 executable."""
    if architecture not in ARCHITECTURES:
        raise ContractError("unsupported Debian architecture")
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ContractError(f"required binary is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise ContractError(f"required binary is not a regular file: {path.name}")
    if not metadata.st_mode & 0o111:
        raise ContractError(f"required binary is not executable: {path.name}")
    if metadata.st_size < 64 or metadata.st_size > MAX_BINARY_BYTES:
        raise ContractError(f"required binary has an invalid size: {path.name}")
    try:
        with path.open("rb") as source:
            header = source.read(64)
    except OSError as error:
        raise ContractError(f"required binary cannot be read: {path.name}") from error
    if (
        len(header) != 64
        or header[:4] != b"\x7fELF"
        or header[4] != 2
        or header[5] != 1
        or header[6] != 1
    ):
        raise ContractError(f"required binary is not little-endian ELF64: {path.name}")
    executable_type, machine = struct.unpack_from("<HH", header, 16)
    if executable_type not in (2, 3):
        raise ContractError(f"required ELF is not an executable: {path.name}")
    if machine != ARCHITECTURES[architecture]:
        raise ContractError(f"required binary architecture does not match: {path.name}")
    return BinaryRecord(
        name=path.name,
        architecture=architecture,
        size=metadata.st_size,
        sha256=_hash_regular(path, metadata.st_size),
    )


def validate_binary_directory(
    directory: Path, architecture: str
) -> dict[str, BinaryRecord]:
    """Require the exact, non-symlinked H2 binary input inventory."""
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise ContractError("binary directory must be an absolute ordinary directory")
    try:
        actual = {entry.name for entry in directory.iterdir()}
    except OSError as error:
        raise ContractError("binary directory cannot be inspected") from error
    if actual != set(ALL_BINARIES):
        raise ContractError("binary directory inventory is not exact")
    return {
        name: inspect_elf(directory / name, architecture) for name in ALL_BINARIES
    }


def dependency_entries(value: str) -> tuple[str, ...]:
    """Parse and canonicalize a comma-separated Debian relationship field."""
    if not isinstance(value, str) or "\n" in value or "\r" in value or "\0" in value:
        raise ContractError("dependency field is invalid")
    entries = []
    for raw in value.split(","):
        entry = " ".join(raw.strip().split())
        if not entry:
            continue
        if not re.fullmatch(
            r"[a-z0-9][a-z0-9+.-]*(?::[a-z0-9][a-z0-9-]*)?"
            r"(?:\s+\((?:<<|<=|=|>=|>>)\s+[^()\s]+\))?"
            r"(?:\s+\|\s+[a-z0-9][a-z0-9+.-]*"
            r"(?::[a-z0-9][a-z0-9-]*)?"
            r"(?:\s+\((?:<<|<=|=|>=|>>)\s+[^()\s]+\))?)*",
            entry,
        ):
            raise ContractError("dependency field contains an unsupported relation")
        entries.append(entry)
    return tuple(sorted(set(entries)))


def resolved_static_dependencies(spec: PackageSpec, version: str) -> tuple[str, ...]:
    return dependency_entries(
        ", ".join(
            dependency.replace("{version}", version)
            for dependency in spec.static_dependencies
        )
    )


def combined_dependencies(
    spec: PackageSpec, version: str, shared_libraries: tuple[str, ...]
) -> tuple[str, ...]:
    return tuple(
        sorted(set(resolved_static_dependencies(spec, version)) | set(shared_libraries))
    )


def control_bytes(
    spec: PackageSpec,
    *,
    version: str,
    architecture: str,
    dependencies: tuple[str, ...],
) -> bytes:
    """Render one canonical binary-package control stanza."""
    if architecture not in ARCHITECTURES:
        raise ContractError("unsupported Debian architecture")
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+){2}-[1-9][0-9]*", version):
        raise ContractError("native package version is invalid")
    canonical = dependency_entries(", ".join(dependencies))
    if canonical != dependencies:
        raise ContractError("package dependencies are not canonical")
    lines = [
        f"Package: {spec.name}",
        f"Version: {version}",
        f"Section: {spec.section}",
        "Priority: optional",
        f"Architecture: {architecture}",
        f"Maintainer: {MAINTAINER}",
        f"Depends: {', '.join(dependencies)}",
    ]
    if spec.recommends:
        recommends = dependency_entries(", ".join(spec.recommends))
        lines.append(f"Recommends: {', '.join(recommends)}")
    lines.extend(
        [
            f"Description: {spec.summary}",
            f" {spec.description}",
            "",
        ]
    )
    return "\n".join(lines).encode("utf-8")


def package_filename(spec: PackageSpec, version: str, architecture: str) -> str:
    if architecture not in ARCHITECTURES:
        raise ContractError("unsupported Debian architecture")
    return f"{spec.name}_{version}_{architecture}.deb"


def normalize_tree_timestamps(root: Path, epoch: int) -> None:
    """Reject links, normalize directories, and apply one package-tree mtime."""
    timestamp = source_date_epoch(epoch)
    entries: list[Path] = [root]
    for current, directories, filenames in os.walk(root, followlinks=False):
        current_path = Path(current)
        for name in directories + filenames:
            path = current_path / name
            if path.is_symlink():
                raise ContractError("native package tree contains a symbolic link")
            entries.append(path)
    for path in entries:
        if path.is_dir():
            try:
                path.chmod(0o755)
            except OSError as error:
                raise ContractError(
                    "native package directory mode normalization failed"
                ) from error
    for path in reversed(entries):
        try:
            os.utime(path, (timestamp, timestamp), follow_symlinks=False)
        except OSError as error:
            raise ContractError("native package timestamp normalization failed") from error
