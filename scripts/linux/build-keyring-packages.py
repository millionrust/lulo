#!/usr/bin/env python3
"""Build the standard rmac archive-keyring binary and source package set."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile

from keyring_package_contract import (
    KeyringPackageError,
    _run,
    build_architecture,
    canonicalize_keyring,
    create_manifest,
    fingerprints,
    load_contract,
    orig_tar_bytes,
    package_version,
    source_date_epoch,
    verify_directory,
    write_source_tree,
)


TOOLS = ("dpkg", "dpkg-buildpackage", "dpkg-deb", "dpkg-source", "dh")


def _tool(name: str) -> str:
    value = shutil.which(name)
    if value is None:
        raise KeyringPackageError(f"required Debian package tool is unavailable: {name}")
    path = Path(value)
    try:
        metadata = path.stat()
    except OSError as error:
        raise KeyringPackageError(f"required package tool is unavailable: {name}") from error
    if not stat.S_ISREG(metadata.st_mode) or not os.access(path, os.X_OK):
        raise KeyringPackageError(f"required package tool is not executable: {name}")
    return value


def _version(tool: str, working: Path, environment: dict[str, str]) -> str:
    result = _run(
        [tool, "--version"],
        environment=environment,
        cwd=working,
        label=f"{Path(tool).name} version query",
        timeout=30,
    )
    try:
        line = result.stdout.decode("utf-8").splitlines()[0]
    except (UnicodeDecodeError, IndexError) as error:
        raise KeyringPackageError("Debian tool version output is invalid") from error
    if not line or len(line) > 256:
        raise KeyringPackageError("Debian tool version output is invalid")
    return line


def _validate_output(output: Path) -> None:
    if not output.is_absolute() or output == Path("/"):
        raise KeyringPackageError("output must be an absolute non-root directory")
    if output.exists():
        if output.is_symlink() or not output.is_dir():
            raise KeyringPackageError("output must be an ordinary directory")
        try:
            if next(output.iterdir(), None) is not None:
                raise KeyringPackageError("output directory must be empty")
        except OSError as error:
            raise KeyringPackageError("output directory cannot be inspected") from error
    elif output.parent.is_symlink() or not output.parent.is_dir():
        raise KeyringPackageError(
            "output parent must be an existing ordinary directory"
        )


def build(
    *,
    keyring_path: Path,
    output: Path,
    requested_fingerprints: list[str],
    architecture: str,
    epoch: int,
) -> dict[str, object]:
    contract = load_contract()
    if not keyring_path.is_absolute():
        raise KeyringPackageError("public keyring input must be absolute")
    _validate_output(output)
    architecture = build_architecture(architecture, contract)
    epoch = source_date_epoch(epoch)
    primary = fingerprints(
        requested_fingerprints, int(contract["maximum_primary_keys"])
    )
    keyring = canonicalize_keyring(
        keyring_path,
        primary,
        epoch=epoch,
        maximum_bytes=int(contract["maximum_input_bytes"]),
    )
    tools = {name: _tool(name) for name in TOOLS}
    version = package_version(contract)
    upstream = version.rsplit("-", 1)[0]
    with tempfile.TemporaryDirectory(
        dir=output.parent,
        prefix=".rmac-keyring-build-",
    ) as temporary:
        working = Path(temporary)
        home = working / "home"
        home.mkdir(mode=0o700)
        source = working / f"rmac-archive-keyring-{upstream}"
        write_source_tree(
            source,
            keyring,
            version=version,
            epoch=epoch,
        )
        orig = working / f"rmac-archive-keyring_{upstream}.orig.tar.xz"
        orig.write_bytes(
            orig_tar_bytes(
                keyring,
                version=version,
                epoch=epoch,
            )
        )
        orig.chmod(0o644)
        os.utime(orig, (epoch, epoch))
        environment = {
            "DEB_BUILD_OPTIONS": "parallel=1",
            "DPKG_COLORS": "never",
            "HOME": str(home),
            "LC_ALL": "C.UTF-8",
            "PATH": os.environ.get("PATH", ""),
            "SOURCE_DATE_EPOCH": str(epoch),
            "TZ": "UTC",
        }
        host_architecture = _run(
            [tools["dpkg"], "--print-architecture"],
            environment=environment,
            cwd=working,
            label="native dpkg architecture query",
            timeout=30,
        ).stdout.strip()
        if host_architecture != architecture.encode("ascii"):
            raise KeyringPackageError(
                "keyring build architecture must match the native dpkg database"
            )
        tool_versions = {
            name: _version(tools[name], working, environment)
            for name in ("dpkg-buildpackage", "dpkg-deb", "dpkg-source")
        }
        _run(
            [
                tools["dpkg-buildpackage"],
                "--build=source,all",
                "-us",
                "-uc",
                "-sa",
            ],
            environment=environment,
            cwd=source,
            label="standard keyring package build",
            timeout=300,
        )
        names = {
            f"rmac-archive-keyring_{version}_all.deb",
            f"rmac-archive-keyring_{upstream}.orig.tar.xz",
            f"rmac-archive-keyring_{version}.debian.tar.xz",
            f"rmac-archive-keyring_{version}.dsc",
            f"rmac-archive-keyring_{version}_all.buildinfo",
            f"rmac-archive-keyring_{version}_all.changes",
        }
        if any(not (working / name).is_file() for name in names):
            raise KeyringPackageError(
                "standard package build omitted a required artifact"
            )
        staged = working / "verified-output"
        staged.mkdir(mode=0o755)
        for name in sorted(names):
            shutil.copyfile(working / name, staged / name)
            (staged / name).chmod(0o644)
        manifest, sums = create_manifest(
            staged,
            keyring=keyring,
            version=version,
            architecture=architecture,
            epoch=epoch,
            tool_versions=tool_versions,
        )
        (staged / "keyring-packages.json").write_text(
            json.dumps(manifest, sort_keys=True, indent=2) + "\n",
            encoding="utf-8",
        )
        (staged / "SHA256SUMS").write_bytes(sums)
        verify_directory(
            staged,
            expected_architecture=architecture,
            run_dpkg_source=True,
        )
        try:
            output.mkdir(mode=0o755, exist_ok=True)
            for path in sorted(staged.iterdir()):
                os.replace(path, output / path.name)
        except OSError as error:
            raise KeyringPackageError(
                "verified keyring artifacts could not be published"
            ) from error
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--keyring", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fingerprint", action="append", required=True)
    parser.add_argument(
        "--build-architecture",
        choices=("amd64", "arm64"),
        required=True,
    )
    parser.add_argument("--source-date-epoch", required=True)
    arguments = parser.parse_args()
    try:
        manifest = build(
            keyring_path=arguments.keyring,
            output=arguments.output,
            requested_fingerprints=arguments.fingerprint,
            architecture=arguments.build_architecture,
            epoch=source_date_epoch(arguments.source_date_epoch),
        )
    except KeyringPackageError as error:
        parser.exit(4, f"build-keyring-packages: {error}\n")
    print(
        "rmac archive keyring packages built "
        f"({manifest['version']}, {manifest['build_architecture']})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
