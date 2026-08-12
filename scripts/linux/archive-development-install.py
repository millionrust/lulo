#!/usr/bin/env python3
"""Retire source-install artifacts before testing the native package.

The development installer writes executable and activation files below the
test user's home directory. Those paths have higher lookup priority than the
immutable files from a Debian package, so leaving them in place would make a
native-package test silently run stale development binaries. This helper moves
only the exact generated namespace into a reversible state-directory archive.
User settings below ``~/.config/rmac`` are deliberately untouched.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile


REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "linux"))
from native_package_contract import ALL_BINARIES  # noqa: E402


ARCHIVE_NAMES = ("development-install-v1", "development-install-v2")
LEGACY_APPLICATION_IDS = (
    "org.rmac.Files",
    "org.rmac.Terminal",
    "org.rmac.TextEditor",
    "org.rmac.SystemMonitor",
    "org.rmac.SystemSettings",
)
DBUS_SERVICES = (
    "org.freedesktop.impl.portal.desktop.rmac.service",
    "org.rmac.Focus1.service",
    "org.rmac.NotificationCenter1.service",
)
PORTAL_FILES = (
    Path("portals/rmac.portal"),
    Path("rmac-portals.conf"),
)
PRESERVED_AUXILIARY_UNITS = (
    "rmac-lock-fallback-evidence.service",
    "rmac-lock-provider-evidence.service",
)


class MigrationError(RuntimeError):
    """A privacy-safe development-install migration failure."""


@dataclass(frozen=True)
class Roots:
    home: Path
    config: Path
    data: Path
    state: Path

    @classmethod
    def from_environment(cls) -> "Roots":
        home_value = os.environ.get("HOME", "")
        if not home_value:
            raise MigrationError("HOME is unavailable")
        home = Path(home_value)
        return cls(
            home=home,
            config=Path(os.environ.get("XDG_CONFIG_HOME", home / ".config")),
            data=Path(os.environ.get("XDG_DATA_HOME", home / ".local/share")),
            state=Path(os.environ.get("XDG_STATE_HOME", home / ".local/state")),
        ).validated()

    def validated(self) -> "Roots":
        for name, path in (
            ("home", self.home),
            ("config", self.config),
            ("data", self.data),
            ("state", self.state),
        ):
            if not path.is_absolute() or path == Path("/") or path.is_symlink():
                raise MigrationError(f"{name} root is unsafe")
        if not self.home.is_dir():
            raise MigrationError("home root is unavailable")
        return self


@dataclass(frozen=True)
class Artifact:
    source: Path
    archive_relative: Path
    directory: bool = False


def _unit_names() -> tuple[str, ...]:
    unit_dir = REPO_ROOT / "crates/rmac-session/units"
    try:
        names = tuple(sorted(path.name for path in unit_dir.iterdir() if path.is_file()))
    except OSError as error:
        raise MigrationError("session unit inventory is unavailable") from error
    if not names or any(not name.startswith("rmac-") for name in names):
        raise MigrationError("session unit inventory is invalid")
    return names


def _regular_file(path: Path) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    except OSError as error:
        raise MigrationError(f"development artifact cannot be inspected: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise MigrationError(f"development artifact is not a regular file: {path.name}")
    return True


def _development_libexec(path: Path) -> bool:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return False
    except OSError as error:
        raise MigrationError("development executable directory cannot be inspected") from error
    if path.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
        raise MigrationError("development executable path is not an ordinary directory")
    try:
        entries = tuple(path.iterdir())
    except OSError as error:
        raise MigrationError("development executable directory cannot be read") from error
    allowed = set(ALL_BINARIES)
    for entry in entries:
        if entry.name not in allowed or not _regular_file(entry):
            raise MigrationError("development executable directory contains an unknown entry")
    return True


def discover(roots: Roots) -> tuple[Artifact, ...]:
    artifacts: list[Artifact] = []
    unit_dir = roots.config / "systemd/user"
    for name in _unit_names():
        source = unit_dir / name
        if _regular_file(source):
            artifacts.append(Artifact(source, Path("config/systemd/user") / name))

    if unit_dir.is_dir() and not unit_dir.is_symlink():
        known = set(_unit_names()) | set(PRESERVED_AUXILIARY_UNITS)
        try:
            unknown = sorted(
                path.name
                for path in unit_dir.iterdir()
                if path.name.startswith("rmac-") and path.name not in known
            )
        except OSError as error:
            raise MigrationError("development unit directory cannot be read") from error
        if unknown:
            raise MigrationError("development unit directory contains an unknown rmac unit")

    for name in DBUS_SERVICES:
        source = roots.data / "dbus-1/services" / name
        if _regular_file(source):
            artifacts.append(Artifact(source, Path("data/dbus-1/services") / name))
    for relative in PORTAL_FILES:
        source = roots.data / "xdg-desktop-portal" / relative
        if _regular_file(source):
            artifacts.append(
                Artifact(source, Path("data/xdg-desktop-portal") / relative)
            )

    for identity in LEGACY_APPLICATION_IDS:
        desktop = roots.data / "applications" / f"{identity}.desktop"
        if _regular_file(desktop):
            artifacts.append(
                Artifact(desktop, Path("data/applications") / desktop.name)
            )
        icon = roots.data / "icons/hicolor/scalable/apps" / f"{identity}.svg"
        if _regular_file(icon):
            artifacts.append(
                Artifact(
                    icon,
                    Path("data/icons/hicolor/scalable/apps") / icon.name,
                )
            )

    manifest = roots.data / "rmac/development/upstream-shell-candidate.txt"
    if _regular_file(manifest):
        artifacts.append(
            Artifact(
                manifest,
                Path("data/rmac/development/upstream-shell-candidate.txt"),
            )
        )

    launcher = roots.home / ".local/bin/rmac-session-start"
    if _regular_file(launcher):
        artifacts.append(Artifact(launcher, Path("home-local/bin/rmac-session-start")))

    libexec = roots.home / ".local/libexec/rmac"
    if _development_libexec(libexec):
        artifacts.append(Artifact(libexec, Path("home-local/libexec/rmac"), directory=True))
    return tuple(artifacts)


def archive(roots: Roots, artifacts: tuple[Artifact, ...]) -> Path | None:
    if not artifacts:
        return None
    parent = roots.state / "rmac/migrations"
    destination = next(
        (
            parent / name
            for name in ARCHIVE_NAMES
            if not (parent / name).exists() and not (parent / name).is_symlink()
        ),
        None,
    )
    if destination is None:
        raise MigrationError("development-install archives already exist")
    try:
        parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}.", dir=parent))
        temporary.chmod(0o700)
    except OSError as error:
        raise MigrationError("development-install archive cannot be created") from error

    moved: list[tuple[Path, Path]] = []
    published = False
    retain_temporary = False
    try:
        for artifact in artifacts:
            target = temporary / artifact.archive_relative
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            os.replace(artifact.source, target)
            moved.append((artifact.source, target))
        os.replace(temporary, destination)
        published = True
    except OSError as error:
        rollback_failed = False
        for source, target in reversed(moved):
            try:
                source.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                os.replace(target, source)
            except OSError:
                rollback_failed = True
        if rollback_failed:
            retain_temporary = True
            raise MigrationError(
                "development-install rollback needs manual recovery"
            ) from error
        raise MigrationError("development-install artifacts could not be archived") from error
    finally:
        if not published and not retain_temporary:
            shutil.rmtree(temporary, ignore_errors=True)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(
        description="archive stale rmac source-install artifacts before package testing"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--execute", action="store_true")
    arguments = parser.parse_args()
    try:
        roots = Roots.from_environment()
        artifacts = discover(roots)
        print(f"legacy_development_artifacts={len(artifacts)}")
        if arguments.execute:
            destination = archive(roots, artifacts)
            if destination is not None:
                print(f"legacy_development_archive={destination}")
    except MigrationError as error:
        print(f"development-install migration refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
