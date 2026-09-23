#!/usr/bin/env python3
"""Assemble immutable rmac application metadata into an empty DESTDIR."""

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
PACKAGE_NAME = "rmac-apps"
PACKAGE_FORMAT = 1
MAX_SOURCE_BYTES = 1024 * 1024
APPLICATION_IDS = (
    "org.rmac.AppDrawer",
    "org.rmac.Calculator",
    "org.rmac.Files",
    "org.rmac.Notes",
    "org.rmac.SystemMonitor",
    "org.rmac.SystemSettings",
    "org.rmac.Terminal",
    "org.rmac.TextEditor",
)
LOCALIZATION_FILES = {
    "LINGUAS",
    "POTFILES.in",
    "README.md",
    "hi.po",
    "rmac-apps.pot",
}


class PackageError(RuntimeError):
    """A path-safe package assembly failure."""


def _read_regular(path: Path) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PackageError(f"required package source is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise PackageError(f"required package source is not a regular file: {path.name}")
    if metadata.st_size > MAX_SOURCE_BYTES:
        raise PackageError(f"required package source is too large: {path.name}")
    try:
        contents = path.read_bytes()
    except OSError as error:
        raise PackageError(f"required package source cannot be read: {path.name}") from error
    if len(contents) != metadata.st_size:
        raise PackageError(f"required package source changed while reading: {path.name}")
    return contents


def _require_inventory(directory: Path, expected: set[str]) -> None:
    try:
        actual = {path.name for path in directory.iterdir()}
    except OSError as error:
        raise PackageError(f"package source inventory is unavailable: {directory.name}") from error
    if actual != expected:
        raise PackageError(f"package source inventory is not exact: {directory.name}")


def package_files() -> dict[str, tuple[bytes, int]]:
    """Return the complete immutable metadata payload except its manifest."""
    package = REPO_ROOT / "packaging" / "rmac-apps"
    applications = package / "applications"
    icons = package / "icons"
    metainfo = package / "metainfo"
    localization = package / "po"

    _require_inventory(
        applications, {f"{identity}.desktop" for identity in APPLICATION_IDS}
    )
    _require_inventory(icons, {f"{identity}.svg" for identity in APPLICATION_IDS})
    _require_inventory(
        metainfo, {f"{identity}.metainfo.xml" for identity in APPLICATION_IDS}
    )
    _require_inventory(localization, LOCALIZATION_FILES)

    files: dict[str, tuple[bytes, int]] = {}
    for identity in APPLICATION_IDS:
        files[f"usr/share/applications/{identity}.desktop"] = (
            _read_regular(applications / f"{identity}.desktop"),
            0o644,
        )
        files[f"usr/share/icons/hicolor/scalable/apps/{identity}.svg"] = (
            _read_regular(icons / f"{identity}.svg"),
            0o644,
        )
        files[f"usr/share/metainfo/{identity}.metainfo.xml"] = (
            _read_regular(metainfo / f"{identity}.metainfo.xml"),
            0o644,
        )
    for filename in sorted(LOCALIZATION_FILES):
        files[f"usr/share/doc/rmac-apps/localization/{filename}"] = (
            _read_regular(localization / filename),
            0o644,
        )
    files["usr/share/doc/rmac-apps/LICENSES.md"] = (
        _read_regular(package / "LICENSES.md"),
        0o644,
    )
    files["usr/share/doc/rmac-apps/copyright"] = (
        _read_regular(REPO_ROOT / "LICENSE"),
        0o644,
    )
    return files


def _manifest(files: dict[str, tuple[bytes, int]]) -> bytes:
    entries = [
        {
            "path": f"/{relative}",
            "mode": f"{mode:04o}",
            "sha256": hashlib.sha256(contents).hexdigest(),
        }
        for relative, (contents, mode) in sorted(files.items())
    ]
    document = {
        "files": entries,
        "format": PACKAGE_FORMAT,
        "package": PACKAGE_NAME,
        "preserves_user_data": True,
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
    manifest_path = "usr/share/rmac/application-package-manifest.json"
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
        description="assemble rmac application metadata into an empty DESTDIR"
    )
    parser.add_argument("--destdir", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        stage(arguments.destdir)
    except PackageError as error:
        parser.exit(3, f"stage-application-package: {error}\n")
    print(f"staged {PACKAGE_NAME} metadata in {arguments.destdir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
