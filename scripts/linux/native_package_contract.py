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
DEBIAN_REVISION = 38
MAX_BINARY_BYTES = 1024 * 1024 * 1024
MAX_SOURCE_DATE_EPOCH = 253_402_300_799
ARCHITECTURES = {
    "amd64": 62,
    "arm64": 183,
}
MAINTAINER = "Jacob Samas <samasjacob@icloud.com>"
MAINTAINER_SCRIPT_NAMES = ("postinst", "prerm", "postrm")
MAX_MAINTAINER_SCRIPT_BYTES = 64 * 1024


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
    # Debian maintainer scripts read from packaging/<name>/debian/.
    maintainer_scripts: tuple[str, ...] = ()


@dataclass(frozen=True)
class BinaryRecord:
    name: str
    architecture: str
    size: int
    sha256: str


APPLICATION_BINARIES = (
    "rmac-app-drawer",
    "rmac-archive-utility",
    "rmac-calculator",
    "rmac-clock",
    "rmac-files",
    "rmac-notes",
    "rmac-player",
    "rmac-preview",
    "rmac-system-monitor",
    "rmac-system-settings",
    "rmac-terminal",
    "rmac-text-editor",
    "rmac-weather",
)

SESSION_BINARIES = (
    "rmac-session-supervisor",
    "rmac-sound",
    "rmac-media",
    "rmac-wallpaper",
    "rmac-top-bar",
    "rmac-dock",
    "rmac-osd",
    "rmac-app-switcher",
    "rmac-screenshot",
    "rmac-mission-control",
    "rmac-launcher",
    "rmac-app-drawer",
    "rmac-quick-settings",
    "rmac-notification-center-panel",
    "rmac-system-settings",
    "rmac-notification-center",
    "rmac-focus-service",
    "rmac-clipboard-service",
    "rmac-file-chooser",
    "rmac-shortcut-broker",
    "rmac-shortcut-dispatch",
    "rmac-locker",
    "rmac-lock-provider",
    "rmac-lock-coordinator",
    "rmac-idle-locker",
    "rmac-mac-keyboard",
    "rmac-setup-assistant",
)

# The seven shell surfaces are built from the separately locked Linux GPUI
# graph. Each host must consume the maintained runtime/model crate listed here;
# packaging and contract tests treat this mapping as part of the release ABI.
SHIPPING_SHELL_SOURCES = {
    "rmac-wallpaper": ("wallpaper", ("rmac-wallpaper-runtime",)),
    "rmac-top-bar": ("top-bar", ("rmac-shell-runtime",)),
    "rmac-dock": ("dock", ("rmac-dock-runtime", "rmac-dock-system")),
    "rmac-osd": ("osd", ("rmac-osd",)),
    "rmac-app-switcher": ("app-switcher", ("rmac-compositor", "rmac-apps")),
    "rmac-screenshot": ("screenshot", ("rmac-compositor", "rmac-sound")),
    "rmac-mission-control": ("mission-control", ("rmac-compositor", "rmac-shell-settings")),
}

PACKAGE_SPECS = (
    PackageSpec(
        name="rmac-apps",
        section="utils",
        install_directory="usr/bin",
        binaries=APPLICATION_BINARIES,
        static_dependencies=(
            "bluez",
            "curl",
            "dbus-user-session",
            "fonts-inter",
            "fonts-jetbrains-mono",
            "libglib2.0-bin",
            "libmpv2",
            "network-manager",
            "packagekit",
            "pipewire-bin",
            "poppler-utils",
            "power-profiles-daemon",
            "upower",
            "wireplumber",
            # Files: Copy, Cut and Paste of files on the Wayland clipboard.
            "wl-clipboard",
            "xdg-desktop-portal",
            "xdg-utils",
        ),
        recommends=(),
        summary="macOS-inspired applications for the rmac Linux desktop",
        description=(
            "Provides Files, Terminal, Notes, Text Editor, System Monitor, "
            "Calculator, Preview, Archive Utility, Clock, Apps, and Settings with their original rmac desktop "
            "metadata and assets."
        ),
    ),
    PackageSpec(
        name="rmac-session",
        section="x11",
        install_directory="usr/libexec/rmac",
        binaries=SESSION_BINARIES,
        # packagekit, gir1.2-packagekitglib-1.0 and python3-gi back the
        # rmac-update-check.timer / .service daily update check
        # (docs/software-update.md "Automatic Lulo OS updates"): a Python
        # program drives PackageKit through its GObject-introspected client
        # library, prepares Lulo OS updates as an offline update, and sends
        # its notification over D-Bus. python3-gi pulls in gir1.2-glib-2.0,
        # which provides the Gio and GLib typelibs.
        #
        # niri and xwayland-satellite are not in the Ubuntu archive; Lulo OS
        # ships its own builds of the tested releases under the upstream
        # package names with a "-0luloN" revision (scripts/linux/
        # build-niri-packages.sh, packaging/third-party/upstreams.json). The
        # plain versioned relation accepts those builds, the danklinux PPA's
        # "26.04ppaN", and any future official package alike.
        static_dependencies=(
            "coreutils",
            "dbus-user-session",
            "fonts-inter",
            "fonts-jetbrains-mono",
            "gawk | mawk",
            "gir1.2-packagekitglib-1.0",
            "grim",
            "niri (>= 26.04)",
            "packagekit",
            "pipewire-bin",
            "python3-gi",
            "rmac-apps (= {version})",
            "swayidle",
            "swaylock",
            "systemd",
            "wl-clipboard",
            "xdg-desktop-portal",
            "xdg-desktop-portal-gnome",
            "xdg-desktop-portal-gtk",
            "xwayland-satellite (>= 0.8.2)",
        ),
        # keyd and pkexec serve the opt-in "Use Mac shortcuts in all apps"
        # (docs/decisions/0017-mac-keyboard.md); the Qt platform theme gives Qt
        # apps the rmac look (docs/decisions/0019-third-party-toolkit-theming.md).
        recommends=("gdm3", "keyd", "pkexec", "qt6-gtk-platformtheme"),
        summary="niri-based rmac Wayland desktop session",
        description=(
            "Provides the supervised rmac shell services, GDM session entry, "
            "portal selection, secure-lock integration, and immutable session "
            "defaults while retaining stock GNOME as the recovery session."
        ),
        maintainer_scripts=("postinst", "postrm"),
    ),
)

ALL_BINARIES = tuple(
    sorted({binary for package in PACKAGE_SPECS for binary in package.binaries})
)


_SEMVER_PATTERN = r"[0-9]+(?:\.[0-9]+){2}(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?"


def workspace_version(repo_root: Path) -> str:
    """Read the single workspace package version without a TOML dependency.

    Accepts plain ``X.Y.Z`` and Cargo/semver pre-release versions such as
    ``X.Y.Z-beta.1`` (used for Alpha/Beta tags -- see docs/beta-checklist.md).
    """
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
    if match is None or not re.fullmatch(_SEMVER_PATTERN, match.group(1)):
        raise ContractError("workspace package version is not canonical")
    return match.group(1)


def debian_upstream_version(version: str) -> str:
    """Map a Cargo/semver version to a Debian-ordering-safe upstream version.

    Debian compares versions component-by-component and a bare hyphenated
    pre-release (``0.9.0-beta.1``) would sort *after* ``0.9.0``, the opposite
    of what a pre-release needs. Replacing the first ``-`` with ``~`` (Debian
    Policy's own convention) makes ``0.9.0~beta.1`` sort before ``0.9.0``
    while a final release (no hyphen at all) is left unchanged.
    """
    if not re.fullmatch(_SEMVER_PATTERN, version):
        raise ContractError("version is not canonical")
    return version.replace("-", "~", 1)


def native_version(repo_root: Path) -> str:
    return f"{debian_upstream_version(workspace_version(repo_root))}-{DEBIAN_REVISION}"


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


def maintainer_scripts(repo_root: Path, spec: PackageSpec) -> dict[str, bytes]:
    """Read and validate one package's Debian maintainer scripts."""
    scripts: dict[str, bytes] = {}
    for name in spec.maintainer_scripts:
        if name not in MAINTAINER_SCRIPT_NAMES or name in scripts:
            raise ContractError("maintainer script name is not supported")
        path = repo_root / "packaging" / spec.name / "debian" / name
        if path.is_symlink() or not path.is_file():
            raise ContractError(f"maintainer script is missing: {name}")
        try:
            raw = path.read_bytes()
        except OSError as error:
            raise ContractError(f"maintainer script is unreadable: {name}") from error
        if (
            len(raw) > MAX_MAINTAINER_SCRIPT_BYTES
            or b"\0" in raw
            or b"\r" in raw
            or not raw.startswith(b"#!/bin/sh\nset -e\n")
            or not raw.endswith(b"\n")
        ):
            raise ContractError(f"maintainer script is not a strict shell script: {name}")
        scripts[name] = raw
    return scripts


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
    if not re.fullmatch(
        r"[0-9]+(?:\.[0-9]+){2}(?:~[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?-[1-9][0-9]*",
        version,
    ):
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
