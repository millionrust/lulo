#!/usr/bin/env python3
"""Build reproducible rmac Debian packages from an exact prebuilt ELF set."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

from native_package_contract import (
    ARCHITECTURES,
    FORMAT_VERSION,
    MAINTAINER,
    PACKAGE_SPECS,
    BinaryRecord,
    ContractError,
    PackageSpec,
    combined_dependencies,
    control_bytes,
    dependency_entries,
    inspect_elf,
    maintainer_scripts,
    native_version,
    normalize_tree_timestamps,
    package_filename,
    resolved_static_dependencies,
    source_date_epoch,
    validate_binary_directory,
)


REPO_ROOT = Path(__file__).resolve().parents[2]
MAX_TOOL_OUTPUT_BYTES = 1024 * 1024


class PackageBuildError(RuntimeError):
    """A privacy-safe H2 assembly failure."""


def _load_script(name: str, filename: str):
    path = Path(__file__).with_name(filename)
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise PackageBuildError("package staging contract cannot be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def _require_tool(name: str) -> str:
    candidate = shutil.which(name)
    if candidate is None:
        raise PackageBuildError(f"required Debian tool is unavailable: {name}")
    path = Path(candidate)
    try:
        metadata = path.stat()
    except OSError as error:
        raise PackageBuildError(f"required Debian tool is unavailable: {name}") from error
    if not stat.S_ISREG(metadata.st_mode) or not os.access(path, os.X_OK):
        raise PackageBuildError(f"required Debian tool is not executable: {name}")
    return candidate


def _run(
    command: list[str],
    *,
    cwd: Path,
    environment: dict[str, str],
    timeout: int,
    label: str,
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
        raise PackageBuildError(f"{label} could not run") from error
    if (
        len(result.stdout) > MAX_TOOL_OUTPUT_BYTES
        or len(result.stderr) > MAX_TOOL_OUTPUT_BYTES
    ):
        raise PackageBuildError(f"{label} produced excessive output")
    if result.returncode != 0:
        raise PackageBuildError(f"{label} failed")
    return result


def _host_architecture(
    dpkg: str, architecture: str, environment: dict[str, str], working: Path
) -> None:
    result = _run(
        [dpkg, "--print-architecture"],
        cwd=working,
        environment=environment,
        timeout=30,
        label="dpkg architecture query",
    )
    try:
        host = result.stdout.decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise PackageBuildError("dpkg returned an invalid host architecture") from error
    if host != architecture:
        raise PackageBuildError(
            "package architecture must match the native dpkg dependency database"
        )


def _analysis_control() -> bytes:
    lines = [
        "Source: rmac",
        "Section: x11",
        "Priority: optional",
        f"Maintainer: {MAINTAINER}",
        "Standards-Version: 4.7.2.0",
        "Rules-Requires-Root: no",
        "",
    ]
    for package in PACKAGE_SPECS:
        lines.extend(
            [
                f"Package: {package.name}",
                "Architecture: any",
                f"Description: {package.summary}",
                f" {package.description}",
                "",
            ]
        )
    return "\n".join(lines).encode("utf-8")


def derive_shared_library_dependencies(
    *,
    dpkg_shlibdeps: str,
    spec: PackageSpec,
    binaries: tuple[Path, ...],
    architecture: str,
    epoch: int,
    working: Path,
    base_environment: dict[str, str] | None = None,
) -> tuple[str, ...]:
    """Derive strict package relations from the native dpkg symbols database."""
    if not binaries:
        raise PackageBuildError("package has no binaries for dependency analysis")
    debian = working / "debian"
    try:
        debian.mkdir(parents=True)
        (debian / "control").write_bytes(_analysis_control())
    except OSError as error:
        raise PackageBuildError("dependency analysis workspace cannot be prepared") from error
    environment = dict(os.environ if base_environment is None else base_environment)
    environment.pop("LD_LIBRARY_PATH", None)
    environment.update(
        {
            "DEB_HOST_ARCH": architecture,
            "DPKG_COLORS": "never",
            "DPKG_NLS": "0",
            "LC_ALL": "C",
            "SOURCE_DATE_EPOCH": str(epoch),
        }
    )
    command = [
        dpkg_shlibdeps,
        "-O",
        f"--package={spec.name}",
        *(f"-e{path}" for path in binaries),
    ]
    result = _run(
        command,
        cwd=working,
        environment=environment,
        timeout=120,
        label=f"{spec.name} shared-library dependency analysis",
    )
    try:
        lines = result.stdout.decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise PackageBuildError("dpkg-shlibdeps output is not UTF-8") from error
    prefix = "shlibs:Depends="
    values = [line.removeprefix(prefix) for line in lines if line.startswith(prefix)]
    if len(lines) != 1 or len(values) != 1:
        raise PackageBuildError("dpkg-shlibdeps returned an invalid dependency record")
    try:
        dependencies = dependency_entries(values[0])
    except ContractError as error:
        raise PackageBuildError(str(error)) from error
    if not dependencies:
        raise PackageBuildError("dpkg-shlibdeps returned no runtime dependency")
    return dependencies


def _validate_output(destination: Path) -> None:
    if not destination.is_absolute() or destination == Path("/"):
        raise PackageBuildError("output directory must be an absolute non-root path")
    if destination.exists():
        if destination.is_symlink() or not destination.is_dir():
            raise PackageBuildError("output directory must be an ordinary directory")
        try:
            if next(destination.iterdir(), None) is not None:
                raise PackageBuildError("output directory must be empty")
        except OSError as error:
            raise PackageBuildError("output directory cannot be inspected") from error
    elif destination.parent.is_symlink() or not destination.parent.is_dir():
        raise PackageBuildError(
            "output parent must be an existing ordinary directory"
        )


def _copy_binary(
    source: Path,
    destination: Path,
    *,
    architecture: str,
    expected: BinaryRecord,
) -> None:
    try:
        destination.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
    except OSError as error:
        raise PackageBuildError(f"could not stage binary: {source.name}") from error
    try:
        copied = inspect_elf(destination, architecture)
    except ContractError as error:
        raise PackageBuildError(str(error)) from error
    if copied.size != expected.size or copied.sha256 != expected.sha256:
        raise PackageBuildError(f"binary changed during staging: {source.name}")


def _sha256(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
                size += len(chunk)
    except OSError as error:
        raise PackageBuildError("built package cannot be read") from error
    return digest.hexdigest(), size


def _manifest_bytes(
    *,
    architecture: str,
    version: str,
    epoch: int,
    package_records: list[dict[str, object]],
) -> bytes:
    document = {
        "architecture": architecture,
        "format": FORMAT_VERSION,
        "packages": package_records,
        "source_date_epoch": epoch,
        "version": version,
    }
    return (json.dumps(document, indent=2, sort_keys=True) + "\n").encode("utf-8")


def build(
    *,
    binary_directory: Path,
    output_directory: Path,
    architecture: str,
    epoch: int,
) -> None:
    if architecture not in ARCHITECTURES:
        raise PackageBuildError("unsupported Debian architecture")
    _validate_output(output_directory)
    try:
        records = validate_binary_directory(binary_directory, architecture)
        version = native_version(REPO_ROOT)
        epoch = source_date_epoch(epoch)
    except ContractError as error:
        raise PackageBuildError(str(error)) from error

    dpkg = _require_tool("dpkg")
    dpkg_deb = _require_tool("dpkg-deb")
    dpkg_shlibdeps = _require_tool("dpkg-shlibdeps")
    environment = dict(os.environ)
    environment.pop("LD_LIBRARY_PATH", None)
    environment.update(
        {
            "DPKG_COLORS": "never",
            "DPKG_DEB_THREADS_MAX": "1",
            "DPKG_NLS": "0",
            "LC_ALL": "C",
            "SOURCE_DATE_EPOCH": str(epoch),
        }
    )

    workspace = Path(
        tempfile.mkdtemp(
            prefix=f".{output_directory.name}.work.", dir=output_directory.parent
        )
    )
    published = False
    try:
        _host_architecture(dpkg, architecture, environment, workspace)
        publish = workspace / "publish"
        roots = workspace / "roots"
        analysis = workspace / "analysis"
        publish.mkdir()
        roots.mkdir()
        analysis.mkdir()

        stage_apps = _load_script(
            "rmac_stage_application_package", "stage-application-package.py"
        )
        stage_session = _load_script(
            "rmac_stage_session_package", "stage-session-package.py"
        )
        verify_apps = _load_script(
            "rmac_verify_application_package", "verify-application-package.py"
        )
        verify_session = _load_script(
            "rmac_verify_session_package", "verify-session-package.py"
        )
        stage_by_name = {
            "rmac-apps": stage_apps,
            "rmac-session": stage_session,
        }
        verify_by_name = {
            "rmac-apps": verify_apps,
            "rmac-session": verify_session,
        }
        package_records: list[dict[str, object]] = []

        for spec in PACKAGE_SPECS:
            root = roots / spec.name
            try:
                stage_by_name[spec.name].stage(root)
                verify_by_name[spec.name].verify_tree(root)
            except Exception as error:
                raise PackageBuildError(
                    f"{spec.name} immutable payload staging failed"
                ) from error

            staged_paths = []
            binary_records = []
            for name in spec.binaries:
                destination = root / spec.install_directory / name
                _copy_binary(
                    binary_directory / name,
                    destination,
                    architecture=architecture,
                    expected=records[name],
                )
                staged_paths.append(destination)
                binary_records.append(
                    {
                        "name": name,
                        "path": f"/{spec.install_directory}/{name}",
                        "sha256": records[name].sha256,
                        "size": records[name].size,
                    }
                )
            try:
                verify_by_name[spec.name].verify_tree(root, exact_tree=False)
            except Exception as error:
                raise PackageBuildError(
                    f"{spec.name} staged payload verification failed"
                ) from error

            shared_dependencies = derive_shared_library_dependencies(
                dpkg_shlibdeps=dpkg_shlibdeps,
                spec=spec,
                binaries=tuple(staged_paths),
                architecture=architecture,
                epoch=epoch,
                working=analysis / spec.name,
                base_environment=environment,
            )
            dependencies = combined_dependencies(
                spec, version, shared_dependencies
            )
            control = root / "DEBIAN" / "control"
            try:
                control.parent.mkdir(mode=0o755)
                control.write_bytes(
                    control_bytes(
                        spec,
                        version=version,
                        architecture=architecture,
                        dependencies=dependencies,
                    )
                )
                control.chmod(0o644)
                for name, raw in maintainer_scripts(REPO_ROOT, spec).items():
                    script = control.parent / name
                    script.write_bytes(raw)
                    script.chmod(0o755)
                normalize_tree_timestamps(root, epoch)
            except (OSError, ContractError) as error:
                raise PackageBuildError(
                    f"{spec.name} control metadata could not be prepared"
                ) from error

            filename = package_filename(spec, version, architecture)
            archive = publish / filename
            _run(
                [
                    dpkg_deb,
                    "--root-owner-group",
                    "--uniform-compression",
                    "--threads-max=1",
                    "-Zxz",
                    "-z9",
                    "--build",
                    str(root),
                    str(archive),
                ],
                cwd=workspace,
                environment=environment,
                # Single-threaded xz -9 of the apps package takes several
                # minutes on the 2-core reference laptop.
                timeout=1800,
                label=f"{spec.name} archive build",
            )
            try:
                archive.chmod(0o644)
                os.utime(archive, (epoch, epoch))
            except OSError as error:
                raise PackageBuildError(
                    f"{spec.name} archive metadata could not be normalized"
                ) from error
            digest, size = _sha256(archive)
            package_records.append(
                {
                    "binaries": binary_records,
                    "depends": list(dependencies),
                    "filename": filename,
                    "package": spec.name,
                    "recommends": list(spec.recommends),
                    "sha256": digest,
                    "shared_library_dependencies": list(shared_dependencies),
                    "size": size,
                    "static_dependencies": list(
                        resolved_static_dependencies(spec, version)
                    ),
                }
            )

        try:
            (publish / "native-packages.json").write_bytes(
                _manifest_bytes(
                    architecture=architecture,
                    version=version,
                    epoch=epoch,
                    package_records=package_records,
                )
            )
            checksums = "".join(
                f"{record['sha256']}  {record['filename']}\n"
                for record in package_records
            )
            (publish / "SHA256SUMS").write_text(checksums, encoding="ascii")
            for path in (publish / "native-packages.json", publish / "SHA256SUMS"):
                path.chmod(0o644)
                os.utime(path, (epoch, epoch))
        except OSError as error:
            raise PackageBuildError("package publication manifest could not be written") from error

        verifier = _load_script(
            "rmac_verify_native_packages", "verify-native-packages.py"
        )
        try:
            verifier.verify_directory(
                publish,
                architecture=architecture,
                expected_version=version,
                dpkg_deb=dpkg_deb,
            )
        except Exception as error:
            raise PackageBuildError("built native package verification failed") from error

        if output_directory.exists():
            output_directory.rmdir()
        os.replace(publish, output_directory)
        published = True
    except OSError as error:
        raise PackageBuildError("native package set could not be published") from error
    finally:
        shutil.rmtree(workspace, ignore_errors=True)
        if not published and output_directory.exists():
            try:
                if next(output_directory.iterdir(), None) is None:
                    output_directory.rmdir()
            except OSError:
                pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary-dir",
        required=True,
        type=Path,
        help="absolute directory containing exactly the required prebuilt ELF files",
    )
    parser.add_argument(
        "--output",
        required=True,
        type=Path,
        help="empty absolute destination for the two packages and hash manifests",
    )
    parser.add_argument(
        "--architecture",
        required=True,
        choices=tuple(ARCHITECTURES),
    )
    parser.add_argument(
        "--source-date-epoch",
        default=os.environ.get("SOURCE_DATE_EPOCH"),
        help="canonical decimal reproducible-build timestamp",
    )
    arguments = parser.parse_args()
    try:
        if arguments.source_date_epoch is None:
            raise PackageBuildError("SOURCE_DATE_EPOCH is required")
        build(
            binary_directory=arguments.binary_dir,
            output_directory=arguments.output,
            architecture=arguments.architecture,
            epoch=source_date_epoch(arguments.source_date_epoch),
        )
    except (ContractError, PackageBuildError) as error:
        parser.exit(3, f"build-native-packages: {error}\n")
    print(f"built verified {arguments.architecture} native packages in {arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
