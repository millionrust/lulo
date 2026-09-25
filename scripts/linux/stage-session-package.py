#!/usr/bin/env python3
"""Assemble the package-owned rmac session integration into an empty DESTDIR."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile


REPO_ROOT = Path(__file__).resolve().parents[2]
PACKAGE_NAME = "rmac-session"
PACKAGE_FORMAT = 1
DEVELOPMENT_LIBEXEC = "%h/.local/libexec/rmac"
SYSTEM_LIBEXEC = "/usr/libexec/rmac"
MAX_SOURCE_BYTES = 1024 * 1024
# Each packaged wallpaper image stays under this budget.
MAX_WALLPAPER_BYTES = 3 * 1024 * 1024


class PackageError(RuntimeError):
    """A path-safe package assembly failure."""


def _read_regular(path: Path, maximum: int = MAX_SOURCE_BYTES) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PackageError(f"required package source is unavailable: {path.name}") from error
    if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
        raise PackageError(f"required package source is not a regular file: {path.name}")
    if metadata.st_size > maximum:
        raise PackageError(f"required package source is too large: {path.name}")
    try:
        value = path.read_bytes()
    except OSError as error:
        raise PackageError(f"required package source cannot be read: {path.name}") from error
    if len(value) != metadata.st_size:
        raise PackageError(f"required package source changed while reading: {path.name}")
    return value


def _text(path: Path) -> str:
    raw = _read_regular(path)
    if b"\0" in raw:
        raise PackageError(f"package source contains NUL: {path.name}")
    try:
        value = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PackageError(f"package source is not UTF-8: {path.name}") from error
    if not value.endswith("\n"):
        raise PackageError(f"package source lacks a final newline: {path.name}")
    return value


def _session_unit(path: Path) -> bytes:
    value = _text(path).replace(DEVELOPMENT_LIBEXEC, SYSTEM_LIBEXEC)
    if DEVELOPMENT_LIBEXEC in value or "%h/.local/bin" in value:
        raise PackageError(f"development executable path survived in {path.name}")
    return value.encode()


def _dbus_service(path: Path, token: str, executable: str) -> bytes:
    value = _text(path)
    if value.count(token) != 1:
        raise PackageError(f"unexpected activation template in {path.name}")
    value = value.replace(token, executable)
    if "@RMAC_" in value:
        raise PackageError(f"unresolved activation token in {path.name}")
    return value.encode()


def theme_sources(theme: Path) -> list[Path]:
    """Every file of the rmac GTK theme, refusing links and stray types."""
    if theme.is_symlink() or not theme.is_dir():
        raise PackageError("the rmac GTK theme is missing")
    sources = []
    for current, directories, filenames in os.walk(theme, followlinks=False):
        directories.sort()
        for directory in directories:
            if (Path(current) / directory).is_symlink():
                raise PackageError("GTK theme source inventory is not regular")
        for filename in sorted(filenames):
            source = Path(current) / filename
            if source.is_symlink() or not source.is_file():
                raise PackageError("GTK theme source inventory is not regular")
            if source.suffix not in {".css", ".theme"}:
                raise PackageError(f"unexpected GTK theme source: {filename}")
            sources.append(source)
    if not sources:
        raise PackageError("the rmac GTK theme is empty")
    return sorted(sources)


WALLPAPER_IDS = (
    "lulo",
    "lulo-grove",
    "lulo-ember",
    "lulo-dusk",
    "lulo-mist",
    "lulo-nocturne",
)
WALLPAPER_SIZES = ("3840x2160", "2560x1600", "1920x1080", "thumbnail")


def wallpaper_inventory() -> list[str]:
    """Exact packaged wallpaper file names, shared with the package verifier."""
    return [
        f"{identifier}-{appearance}-{size}.jpg"
        for identifier in WALLPAPER_IDS
        for appearance in ("light", "dark")
        for size in WALLPAPER_SIZES
    ]


def package_files() -> dict[str, tuple[bytes, int]]:
    """Return the complete immutable package payload except its manifest."""
    package = REPO_ROOT / "packaging" / "rmac-session"
    session = REPO_ROOT / "crates" / "rmac-session"
    notifications = REPO_ROOT / "crates" / "rmac-notifications-linux" / "install"
    focus = REPO_ROOT / "crates" / "rmac-focus-linux" / "install"
    file_chooser = REPO_ROOT / "crates" / "rmac-file-chooser" / "install"

    files: dict[str, tuple[bytes, int]] = {
        "usr/share/wayland-sessions/rmac.desktop": (
            _read_regular(package / "rmac.desktop"),
            0o644,
        ),
        "usr/libexec/rmac/rmac-wayland-session": (
            _read_regular(package / "rmac-wayland-session"),
            0o755,
        ),
        "usr/libexec/rmac/rmac-session-start": (
            _read_regular(REPO_ROOT / "scripts/linux/start-rmac-session.sh"),
            0o755,
        ),
        "usr/libexec/rmac/rmac-update-check": (
            _read_regular(REPO_ROOT / "scripts/linux/rmac-update-check"),
            0o755,
        ),
        "usr/share/rmac/niri/shortcuts-fallback.kdl": (
            _read_regular(package / "shortcuts-fallback.kdl"),
            0o644,
        ),
        "usr/share/rmac/niri/config.kdl": (
            _read_regular(package / "config.kdl"),
            0o644,
        ),
        "usr/share/rmac/niri/shell.kdl": (
            _read_regular(package / "shell.kdl"),
            0o644,
        ),
        "usr/share/rmac/session/swaylock.conf": (
            _read_regular(session / "swaylock.conf"),
            0o644,
        ),
        "usr/share/rmac/session/lock-policy.json": (
            _read_regular(session / "lock-policy.json"),
            0o644,
        ),
        "etc/pam.d/rmac-lock": (
            _read_regular(
                REPO_ROOT / "crates/rmac-lock-provider-linux/pam/rmac-lock"
            ),
            0o644,
        ),
        "usr/share/rmac/greeter/rmac-aurora.svg": (
            _read_regular(package / "greeter" / "rmac-aurora.svg"),
            0o644,
        ),
        "usr/share/glib-2.0/schemas/90_rmac-greeter.gschema.override": (
            _read_regular(package / "greeter" / "90_rmac-greeter.gschema.override"),
            0o644,
        ),
        "usr/share/glib-2.0/schemas/91_rmac-desktop.gschema.override": (
            _read_regular(package / "gsettings" / "91_rmac-desktop.gschema.override"),
            0o644,
        ),
        "etc/fonts/conf.d/99-rmac.conf": (
            _read_regular(package / "fontconfig" / "99-rmac.conf"),
            0o644,
        ),
        "usr/share/xdg-desktop-portal/portals/rmac.portal": (
            _read_regular(notifications / "rmac.portal"),
            0o644,
        ),
        "usr/share/xdg-desktop-portal/portals/rmac-file-chooser.portal": (
            _read_regular(file_chooser / "rmac-file-chooser.portal"),
            0o644,
        ),
        "usr/share/xdg-desktop-portal/rmac-portals.conf": (
            _read_regular(notifications / "rmac-portals.conf"),
            0o644,
        ),
        "usr/lib/systemd/system/rmac-mac-keyboard-relay.socket": (
            _read_regular(package / "systemd" / "rmac-mac-keyboard-relay.socket"),
            0o644,
        ),
        "usr/lib/systemd/system/rmac-mac-keyboard-relay@.service": (
            _read_regular(package / "systemd" / "rmac-mac-keyboard-relay@.service"),
            0o644,
        ),
        "usr/share/polkit-1/actions/org.rmac.mac-keyboard.policy": (
            _read_regular(package / "polkit" / "org.rmac.mac-keyboard.policy"),
            0o644,
        ),
        "usr/share/doc/rmac-session/copyright": (
            _read_regular(REPO_ROOT / "LICENSE"),
            0o644,
        ),
        "usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.rmac.service": (
            _dbus_service(
                notifications
                / "org.freedesktop.impl.portal.desktop.rmac.service.in",
                "@RMAC_NOTIFICATION_EXEC@",
                f"{SYSTEM_LIBEXEC}/rmac-notification-center",
            ),
            0o644,
        ),
        "usr/share/dbus-1/services/org.rmac.NotificationCenter1.service": (
            _dbus_service(
                notifications / "org.rmac.NotificationCenter1.service.in",
                "@RMAC_NOTIFICATION_EXEC@",
                f"{SYSTEM_LIBEXEC}/rmac-notification-center",
            ),
            0o644,
        ),
        "usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.rmac.filechooser.service": (
            _dbus_service(
                file_chooser
                / "org.freedesktop.impl.portal.desktop.rmac.filechooser.service.in",
                "@RMAC_FILE_CHOOSER_EXEC@",
                f"{SYSTEM_LIBEXEC}/rmac-file-chooser",
            ),
            0o644,
        ),
        "usr/share/dbus-1/services/org.rmac.Focus1.service": (
            _dbus_service(
                focus / "org.rmac.Focus1.service.in",
                "@RMAC_FOCUS_EXEC@",
                f"{SYSTEM_LIBEXEC}/rmac-focus-service",
            ),
            0o644,
        ),
    }
    for source in sorted((session / "units").iterdir(), key=lambda path: path.name):
        if source.is_symlink() or not source.is_file():
            raise PackageError("session unit source inventory is not regular")
        destination = f"usr/lib/systemd/user/{source.name}"
        if destination in files:
            raise PackageError(f"duplicate package destination: {source.name}")
        files[destination] = (_session_unit(source), 0o644)
    # Original rmac pointer theme (FEEL_SPEC.md §D.2).
    cursors = REPO_ROOT / "assets" / "cursors" / "rmac"
    if not cursors.is_dir():
        raise PackageError("the rmac cursor theme is missing; run scripts/build-cursors.py")
    for source in sorted(cursors.iterdir(), key=lambda path: path.name):
        if source.is_symlink() or not source.is_file():
            raise PackageError("cursor theme source inventory is not regular")
        destination = f"usr/share/icons/rmac/{source.name}"
        if destination in files:
            raise PackageError(f"duplicate package destination: {source.name}")
        files[destination] = (_read_regular(source), 0o644)
    # Original rmac GTK 3 / GTK 4 / libadwaita theme (FEEL_SPEC.md §D.7).
    theme = package / "themes" / "rmac"
    for source in theme_sources(theme):
        relative = source.relative_to(theme).as_posix()
        destination = f"usr/share/themes/rmac/{relative}"
        if destination in files:
            raise PackageError(f"duplicate package destination: {relative}")
        files[destination] = (_read_regular(source), 0o644)
    # Reproducible original interface sounds (FEEL_SPEC.md §D.1).
    sounds = REPO_ROOT / "assets" / "sounds"
    expected_sounds = {
        "alert.wav",
        "drag-drop.wav",
        "empty-trash.wav",
        "error.wav",
        "lock.wav",
        "login.wav",
        "mount.wav",
        "notification.wav",
        "power-plug.wav",
        "screenshot.wav",
        "trash.wav",
        "unlock.wav",
        "unmount.wav",
        "volume-tick.wav",
    }
    actual_sounds = {path.name for path in sounds.glob("*.wav")}
    if actual_sounds != expected_sounds:
        raise PackageError("sound source inventory is not exact")
    for name in sorted(expected_sounds):
        source = sounds / name
        destination = f"usr/share/rmac/sounds/{name}"
        files[destination] = (_read_regular(source), 0o644)
    # Original Lulo wallpapers (scripts/build-wallpapers.py): every built-in
    # has light and dark artwork at each packaged size plus a Settings
    # thumbnail. rmac-wallpaper resolves them under /usr/share/rmac/wallpapers.
    wallpapers = package / "wallpapers"
    expected_wallpapers = set(wallpaper_inventory())
    actual_wallpapers = {path.name for path in wallpapers.iterdir()} if wallpapers.is_dir() else set()
    if actual_wallpapers != expected_wallpapers:
        raise PackageError(
            "wallpaper source inventory is not exact; run scripts/build-wallpapers.py"
        )
    for name in sorted(expected_wallpapers):
        destination = f"usr/share/rmac/wallpapers/{name}"
        files[destination] = (
            _read_regular(wallpapers / name, MAX_WALLPAPER_BYTES),
            0o644,
        )
    # Dock special-item artwork must be available independently of the source
    # tree used to compile the installed binary.
    dock_icons = REPO_ROOT / "crates" / "rmac-dock" / "assets" / "icons"
    for name in (
        "application.svg",
        "trash-empty.svg",
        "trash-full.svg",
        "folder.svg",
        "downloads.svg",
        "stack-item-document.svg",
    ):
        source = dock_icons / name
        destination = f"usr/share/rmac/dock/icons/{name}"
        files[destination] = (_read_regular(source), 0o644)
    return files


def _manifest(files: dict[str, tuple[bytes, int]]) -> bytes:
    entries = []
    for relative, (contents, mode) in sorted(files.items()):
        entries.append(
            {
                "path": f"/{relative}",
                "mode": f"{mode:04o}",
                "sha256": hashlib.sha256(contents).hexdigest(),
            }
        )
    document = {
        "format": PACKAGE_FORMAT,
        "package": PACKAGE_NAME,
        "preserves_user_data": True,
        "files": entries,
    }
    return (json.dumps(document, indent=2, sort_keys=True) + "\n").encode()


def _validate_destination(destination: Path) -> Path:
    if not destination.is_absolute():
        raise PackageError("DESTDIR must be absolute")
    if destination == Path("/"):
        raise PackageError("refusing to assemble directly into the live root")
    if destination.exists():
        if destination.is_symlink() or not destination.is_dir():
            raise PackageError("DESTDIR must be an ordinary directory")
        try:
            if next(destination.iterdir(), None) is not None:
                raise PackageError("DESTDIR must be empty")
        except OSError as error:
            raise PackageError("DESTDIR cannot be inspected") from error
    elif destination.parent.is_symlink() or not destination.parent.is_dir():
        raise PackageError("DESTDIR parent must be an existing ordinary directory")
    return destination


def stage(destination: Path) -> None:
    """Build in a sibling directory, then atomically publish the package tree."""
    destination = _validate_destination(destination)
    files = package_files()
    manifest_path = "usr/share/rmac/session-package-manifest.json"
    if manifest_path in files:
        raise PackageError("manifest destination collides with package contents")
    files[manifest_path] = (_manifest(files), 0o644)

    temporary = Path(
        tempfile.mkdtemp(prefix=f".{destination.name}.", dir=destination.parent)
    )
    published = False
    try:
        for relative, (contents, mode) in sorted(files.items()):
            target = temporary / relative
            target.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
            target.write_bytes(contents)
            target.chmod(mode)
        if destination.exists():
            destination.rmdir()
        os.replace(temporary, destination)
        published = True
    except OSError as error:
        raise PackageError("could not publish the staged package tree") from error
    finally:
        if not published:
            shutil.rmtree(temporary, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="assemble rmac session integration into an empty DESTDIR"
    )
    parser.add_argument("--destdir", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        stage(arguments.destdir)
    except PackageError as error:
        parser.exit(3, f"stage-session-package: {error}\n")
    print(f"staged {PACKAGE_NAME} integration in {arguments.destdir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
