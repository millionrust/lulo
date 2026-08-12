#!/usr/bin/env python3
"""Verify staged or installed rmac session integration without private data."""

from __future__ import annotations

import argparse
import configparser
import hashlib
import json
import os
from pathlib import Path
import stat
import sys


MANIFEST = Path("usr/share/rmac/session-package-manifest.json")
MAX_MANIFEST_BYTES = 64 * 1024
REQUIRED_RMAC_EXECUTABLES = (
    "rmac-session-supervisor",
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
    "rmac-locker",
    "rmac-lock-provider",
    "rmac-lock-coordinator",
    "rmac-idle-locker",
)
EXPECTED_SYSTEMD_UNITS = (
    "rmac-app-drawer.service",
    "rmac-component-failure@.service",
    "rmac-dock.service",
    "rmac-focus.service",
    "rmac-idle-lock.service",
    "rmac-launcher.service",
    "rmac-lock-coordinator.service",
    "rmac-lock.service",
    "rmac-lock-fallback.service",
    "rmac-notification-center-panel.service",
    "rmac-notification-center.service",
    "rmac-osd.service",
    "rmac-quick-settings.service",
    "rmac-safe-mode.target",
    "rmac-session-supervisor.service",
    "rmac-session.target",
    "rmac-shortcut-broker.service",
    "rmac-top-bar.service",
    "rmac-wallpaper.service",
)
EXPECTED_PATHS = {
    Path("etc/pam.d/rmac-lock"),
    Path("usr/share/wayland-sessions/rmac.desktop"),
    Path("usr/libexec/rmac/rmac-wayland-session"),
    Path("usr/libexec/rmac/rmac-session-start"),
    Path("usr/share/rmac/niri/config.kdl"),
    Path("usr/share/rmac/niri/shell.kdl"),
    Path("usr/share/rmac/niri/shortcuts-fallback.kdl"),
    Path("usr/share/rmac/session/swaylock.conf"),
    Path("usr/share/rmac/session/lock-policy.json"),
    Path("usr/share/rmac/greeter/rmac-aurora.svg"),
    Path("usr/share/rmac/greeter/rmac-greeter-logo.svg"),
    Path("usr/share/glib-2.0/schemas/90_rmac-greeter.gschema.override"),
    Path("usr/share/xdg-desktop-portal/portals/rmac.portal"),
    Path("usr/share/xdg-desktop-portal/rmac-portals.conf"),
    Path("usr/share/doc/rmac-session/copyright"),
    Path(
        "usr/share/dbus-1/services/"
        "org.freedesktop.impl.portal.desktop.rmac.service"
    ),
    Path("usr/share/dbus-1/services/org.rmac.NotificationCenter1.service"),
    Path("usr/share/dbus-1/services/org.rmac.Focus1.service"),
} | {
    Path("usr/lib/systemd/user") / unit for unit in EXPECTED_SYSTEMD_UNITS
}


class VerificationError(RuntimeError):
    """A privacy-safe package verification failure."""


def _regular_bytes(path: Path, maximum: int | None = None) -> tuple[bytes, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VerificationError(f"required file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VerificationError(f"required path is not a regular file: {path.name}")
    if maximum is not None and metadata.st_size > maximum:
        raise VerificationError(f"required file is too large: {path.name}")
    try:
        contents = path.read_bytes()
    except OSError as error:
        raise VerificationError(f"required file cannot be read: {path.name}") from error
    if len(contents) != metadata.st_size:
        raise VerificationError(f"required file changed while reading: {path.name}")
    return contents, stat.S_IMODE(metadata.st_mode)


def _load_manifest(root: Path) -> list[dict[str, str]]:
    raw, mode = _regular_bytes(root / MANIFEST, MAX_MANIFEST_BYTES)
    if mode != 0o644:
        raise VerificationError("package manifest has the wrong mode")
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("package manifest is invalid") from error
    if (
        not isinstance(document, dict)
        or type(document.get("format")) is not int
        or document.get("format") != 1
        or document.get("package") != "rmac-session"
        or document.get("preserves_user_data") is not True
        or not isinstance(document.get("files"), list)
    ):
        raise VerificationError("package manifest identity is invalid")
    entries = document["files"]
    paths = [entry.get("path") for entry in entries if isinstance(entry, dict)]
    if len(paths) != len(entries) or paths != sorted(paths) or len(paths) != len(set(paths)):
        raise VerificationError("package manifest paths are not canonical")
    return entries


def _safe_manifest_path(value: object) -> Path:
    if not isinstance(value, str) or not (
        value.startswith("/usr/") or value == "/etc/pam.d/rmac-lock"
    ):
        raise VerificationError("package manifest contains a non-system path")
    relative = Path(value.removeprefix("/"))
    if (
        ".." in relative.parts
        or not relative.parts
        or f"/{relative.as_posix()}" != value
    ):
        raise VerificationError("package manifest contains an unsafe path")
    if any(part.lower() in {"gnome", "ubuntu"} for part in relative.parts):
        raise VerificationError("package manifest claims a recovery-desktop path")
    return relative


def _tree_entries(root: Path) -> set[Path]:
    entries: set[Path] = set()

    def failed(error: OSError) -> None:
        raise VerificationError("staged package tree cannot be inspected") from error

    for current, directories, filenames in os.walk(
        root, followlinks=False, onerror=failed
    ):
        current_path = Path(current)
        for directory in list(directories):
            path = current_path / directory
            entries.add(path.relative_to(root))
            if path.is_symlink():
                directories.remove(directory)
        for filename in filenames:
            entries.add((current_path / filename).relative_to(root))
    return entries


def _with_parent_directories(files: set[Path]) -> set[Path]:
    entries = set(files)
    for path in files:
        entries.update(parent for parent in path.parents if parent != Path("."))
    return entries


def verify_tree(root: Path, *, exact_tree: bool = True) -> None:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise VerificationError("verification root must be an absolute ordinary directory")
    entries = _load_manifest(root)
    claimed: set[Path] = set()
    for entry in entries:
        relative = _safe_manifest_path(entry.get("path"))
        claimed.add(relative)
        expected_mode = entry.get("mode")
        expected_hash = entry.get("sha256")
        if (
            not isinstance(expected_mode, str)
            or len(expected_mode) != 4
            or any(character not in "01234567" for character in expected_mode)
            or not isinstance(expected_hash, str)
            or len(expected_hash) != 64
            or any(character not in "0123456789abcdef" for character in expected_hash)
        ):
            raise VerificationError("package manifest metadata is invalid")
        contents, mode = _regular_bytes(root / relative)
        if mode != int(expected_mode, 8):
            raise VerificationError(f"installed mode differs: {relative.name}")
        if hashlib.sha256(contents).hexdigest() != expected_hash:
            raise VerificationError(f"installed content differs: {relative.name}")

    if claimed != EXPECTED_PATHS:
        raise VerificationError("package manifest inventory is not exact")
    if exact_tree:
        actual = _tree_entries(root)
        expected = _with_parent_directories(claimed | {MANIFEST})
        if actual != expected:
            raise VerificationError("staged package tree contains an unexpected path")

    greeter_override, greeter_mode = _regular_bytes(
        root / "usr/share/glib-2.0/schemas/90_rmac-greeter.gschema.override",
        16 * 1024,
    )
    if greeter_mode != 0o644 or greeter_override != (
        b"[org.gnome.login-screen]\n"
        b"logo='/usr/share/rmac/greeter/rmac-greeter-logo.svg'\n"
        b"\n"
        b"[org.gnome.desktop.background]\n"
        b"picture-uri='file:///usr/share/rmac/greeter/rmac-aurora.svg'\n"
        b"picture-uri-dark='file:///usr/share/rmac/greeter/rmac-aurora.svg'\n"
        b"picture-options='zoom'\n"
        b"primary-color='#10162f'\n"
        b"secondary-color='#3949ab'\n"
    ):
        raise VerificationError("GDM appearance override exceeds the reviewed boundary")

    for relative in claimed:
        if relative.parent == Path("usr/lib/systemd/user"):
            raw, _mode = _regular_bytes(root / relative, 1024 * 1024)
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError as error:
                raise VerificationError("package unit is not UTF-8") from error
            if "%h/.local/libexec/rmac" in text or "@RMAC_" in text:
                raise VerificationError("package unit retains a development path")

    desktop = _desktop_entry(root / "usr/share/wayland-sessions/rmac.desktop")
    if (
        desktop.get("Type") != "Application"
        or desktop.get("Exec") != "/usr/libexec/rmac/rmac-wayland-session"
        or desktop.get("TryExec") != "/usr/bin/niri-session"
        or desktop.get("DesktopNames") != "rmac;niri"
    ):
        raise VerificationError("rmac GDM session entry is invalid")


def _desktop_entry(path: Path) -> dict[str, str]:
    raw, _mode = _regular_bytes(path, 64 * 1024)
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    try:
        parser.read_string(raw.decode("utf-8"))
    except (UnicodeDecodeError, configparser.Error) as error:
        raise VerificationError(f"desktop entry is invalid: {path.name}") from error
    if set(parser.sections()) != {"Desktop Entry"}:
        raise VerificationError(f"desktop entry has unexpected groups: {path.name}")
    return dict(parser["Desktop Entry"])


def verify_recovery_session(root: Path) -> None:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise VerificationError("installed root must be an absolute ordinary directory")
    sessions = root / "usr/share/wayland-sessions"
    recovery_found = False
    try:
        candidates = sorted(sessions.glob("*.desktop"))
    except OSError as error:
        raise VerificationError("Wayland session inventory is unavailable") from error
    for candidate in candidates:
        if candidate.name == "rmac.desktop":
            continue
        try:
            entry = _desktop_entry(candidate)
        except VerificationError:
            continue
        names = {
            name.strip().lower()
            for name in entry.get("DesktopNames", "").split(";")
            if name.strip()
        }
        if "gnome" in names:
            recovery_found = True
            break
    if not recovery_found:
        raise VerificationError("a stock GNOME Wayland recovery session was not proven")


def verify_installed_host(root: Path) -> None:
    verify_tree(root, exact_tree=False)
    for relative in (
        Path("usr/bin/niri-session"),
        Path("usr/bin/systemctl"),
        Path("usr/bin/install"),
        Path("usr/bin/awk"),
        Path("usr/bin/sleep"),
        Path("usr/sbin/gdm3"),
    ):
        path = root / relative
        if not path.exists() or not os.access(path, os.X_OK):
            raise VerificationError(f"required runtime executable is missing: {relative.name}")
    for executable in REQUIRED_RMAC_EXECUTABLES:
        path = root / "usr/libexec/rmac" / executable
        if not path.exists() or not os.access(path, os.X_OK):
            raise VerificationError(f"required rmac executable is missing: {executable}")
    verify_recovery_session(root)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument(
        "--installed-host",
        action="store_true",
        help="also require runtime executables and a separate GNOME session",
    )
    modes.add_argument(
        "--recovery-only",
        action="store_true",
        help="require only a separate stock GNOME Wayland recovery session",
    )
    arguments = parser.parse_args()
    try:
        if arguments.recovery_only:
            verify_recovery_session(arguments.root)
        elif arguments.installed_host:
            verify_installed_host(arguments.root)
        else:
            verify_tree(arguments.root)
    except VerificationError as error:
        print(f"verify-session-package: {error}", file=sys.stderr)
        return 3
    if arguments.recovery_only:
        print("stock GNOME Wayland recovery session verified")
    elif arguments.installed_host:
        print("installed rmac and GNOME recovery session boundary verified")
    else:
        print("rmac session package layout verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
