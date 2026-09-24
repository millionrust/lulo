#!/usr/bin/env python3
"""Exercise native rmac packages on an explicitly disposable Ubuntu VM."""

from __future__ import annotations

import argparse
import configparser
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import stat
import subprocess
import sys
import tempfile


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/native/lifecycle.json"
MAX_INPUT_BYTES = 256 * 1024
MAX_TOOL_OUTPUT_BYTES = 1024 * 1024
TEST_USER = "rmac-lifecycle"
TEST_HOME = Path("/home") / TEST_USER
SENTINELS = {
    "documents": Path("Documents/rmac-lifecycle.txt"),
    "xdg-cache": Path(".cache/rmac/lifecycle.txt"),
    "xdg-config": Path(".config/rmac/lifecycle.json"),
    "xdg-data": Path(".local/share/rmac/lifecycle.txt"),
    "xdg-state": Path(".local/state/rmac/lifecycle.txt"),
}
SENTINEL_CONTENT = b"synthetic rmac package lifecycle data\n"
PACKAGE_NAMES = ("rmac-apps", "rmac-session")


class LifecycleError(RuntimeError):
    """A bounded, privacy-safe lifecycle failure."""


def _load_script(name: str, filename: str):
    path = Path(__file__).with_name(filename)
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise LifecycleError("native package verifier cannot be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def _regular_bytes(path: Path, maximum: int = MAX_INPUT_BYTES) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise LifecycleError(f"required input is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise LifecycleError(f"required input is not regular: {path.name}")
    if metadata.st_size > maximum:
        raise LifecycleError(f"required input is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise LifecycleError(f"required input cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise LifecycleError(f"required input changed while reading: {path.name}")
    return raw


def _regular_metadata(path: Path, maximum: int) -> os.stat_result:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise LifecycleError(f"required input is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise LifecycleError(f"required input is not regular: {path.name}")
    if metadata.st_size > maximum:
        raise LifecycleError(f"required input is too large: {path.name}")
    return metadata


def _sha256(path: Path, expected_size: int) -> str:
    digest = hashlib.sha256()
    remaining = expected_size
    try:
        with path.open("rb") as source:
            while remaining:
                chunk = source.read(min(1024 * 1024, remaining))
                if not chunk:
                    raise LifecycleError(f"required input changed while reading: {path.name}")
                digest.update(chunk)
                remaining -= len(chunk)
            if source.read(1):
                raise LifecycleError(f"required input changed while reading: {path.name}")
    except OSError as error:
        raise LifecycleError(f"required input cannot be read: {path.name}") from error
    return digest.hexdigest()


def expected_contract() -> dict[str, object]:
    return {
        "disposable_marker": {
            "contents": "rmac-package-lifecycle-v1\n",
            "path": "/run/rmac-disposable-vm",
        },
        "format": 1,
        "minimum_free_gib": 15,
        "packages": list(PACKAGE_NAMES),
        "platform": {"id": "ubuntu", "version_id": "26.04"},
        "protected_user_data": list(SENTINELS),
        "steps": [
            "install-baseline",
            "upgrade-candidate",
            "interrupt-rollback-after-unpack",
            "recover-rollback",
            "remove",
            "purge",
            "reinstall-candidate",
            "final-purge",
        ],
    }


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    try:
        document = json.loads(_regular_bytes(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise LifecycleError("package lifecycle contract is invalid") from error
    if document != expected_contract():
        raise LifecycleError("package lifecycle contract differs from the reviewed policy")
    return document


def _load_os_release(path: Path = Path("/etc/os-release")) -> dict[str, str]:
    try:
        lines = _regular_bytes(path, 64 * 1024).decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise LifecycleError("operating-system identity is invalid") from error
    fields: dict[str, str] = {}
    for line in lines:
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        if value.startswith('"') and value.endswith('"'):
            value = value[1:-1]
        fields[key] = value
    return fields


def _require_tool(name: str) -> str:
    candidate = shutil.which(name)
    if candidate is None or not os.access(candidate, os.X_OK):
        raise LifecycleError(f"required lifecycle tool is unavailable: {name}")
    return candidate


def _run(
    command: list[str],
    *,
    tools: dict[str, str],
    timeout: int = 600,
    accepted: tuple[int, ...] = (0,),
) -> subprocess.CompletedProcess[bytes]:
    resolved = [tools.get(command[0], command[0]), *command[1:]]
    environment = {
        **os.environ,
        "DEBIAN_FRONTEND": "noninteractive",
        "DPKG_COLORS": "never",
        "DPKG_NLS": "0",
        "LC_ALL": "C",
    }
    try:
        result = subprocess.run(
            resolved,
            check=False,
            capture_output=True,
            env=environment,
            stdin=subprocess.DEVNULL,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise LifecycleError(f"lifecycle command could not run: {command[0]}") from error
    if (
        len(result.stdout) > MAX_TOOL_OUTPUT_BYTES
        or len(result.stderr) > MAX_TOOL_OUTPUT_BYTES
    ):
        raise LifecycleError(f"lifecycle command produced excessive output: {command[0]}")
    if result.returncode not in accepted:
        raise LifecycleError(f"lifecycle command failed: {command[0]}")
    return result


def _validate_directory(path: Path, label: str, *, empty: bool = False) -> None:
    if not path.is_absolute() or path == Path("/") or path.is_symlink():
        raise LifecycleError(f"{label} must be an absolute ordinary directory")
    if path.exists():
        if not path.is_dir():
            raise LifecycleError(f"{label} must be an absolute ordinary directory")
        if empty and next(path.iterdir(), None) is not None:
            raise LifecycleError(f"{label} must be empty")
    elif empty:
        if path.parent.is_symlink() or not path.parent.is_dir():
            raise LifecycleError(f"{label} parent must be an ordinary directory")
    else:
        raise LifecycleError(f"{label} is unavailable")


def _package_set(directory: Path) -> tuple[str, str, dict[str, object]]:
    _validate_directory(directory, "package directory")
    try:
        document = json.loads(_regular_bytes(directory / "native-packages.json"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise LifecycleError("native package manifest is invalid") from error
    if not isinstance(document, dict):
        raise LifecycleError("native package manifest is invalid")
    version = document.get("version")
    architecture = document.get("architecture")
    packages = document.get("packages")
    if (
        not isinstance(version, str)
        or not re.fullmatch(
            r"[0-9]+(?:\.[0-9]+){2}(?:~[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?-[1-9][0-9]*",
            version,
        )
        or architecture not in {"amd64", "arm64"}
        or not isinstance(packages, list)
        or [entry.get("package") for entry in packages if isinstance(entry, dict)]
        != list(PACKAGE_NAMES)
    ):
        raise LifecycleError("native package manifest identity is invalid")
    return version, architecture, document


def _archives(directory: Path, document: dict[str, object]) -> list[str]:
    archives = []
    for record in document["packages"]:
        filename = record.get("filename")
        if not isinstance(filename, str) or Path(filename).name != filename:
            raise LifecycleError("native package archive identity is invalid")
        path = directory / filename
        _regular_metadata(path, 1024 * 1024 * 1024)
        archives.append(str(path))
    return archives


def _check_disk(contract: dict[str, object]) -> None:
    minimum = int(contract["minimum_free_gib"]) * 1024**3
    if shutil.disk_usage("/").free < minimum:
        raise LifecycleError("package lifecycle stopped below the 15 GiB floor")


def _preflight(
    contract: dict[str, object],
    *,
    baseline: Path,
    candidate: Path,
    evidence: Path,
    tools: dict[str, str],
) -> tuple[tuple[str, str, dict[str, object]], tuple[str, str, dict[str, object]]]:
    if os.geteuid() != 0:
        raise LifecycleError("package lifecycle requires root inside the disposable VM")
    marker = contract["disposable_marker"]
    marker_path = Path(marker["path"])
    if _regular_bytes(marker_path, 128) != marker["contents"].encode("ascii"):
        raise LifecycleError("disposable-VM marker does not grant lifecycle permission")
    identity = _load_os_release()
    if (
        identity.get("ID") != contract["platform"]["id"]
        or identity.get("VERSION_ID") != contract["platform"]["version_id"]
    ):
        raise LifecycleError("package lifecycle requires the reviewed Ubuntu release")
    if pwd.getpwnam("root").pw_uid != 0:
        raise LifecycleError("root account identity is invalid")
    try:
        pwd.getpwnam(TEST_USER)
    except KeyError:
        pass
    else:
        raise LifecycleError("fixed lifecycle test user already exists")
    if TEST_HOME.exists():
        raise LifecycleError("fixed lifecycle test home already exists")
    _validate_directory(evidence, "evidence directory", empty=True)
    _check_disk(contract)

    baseline_set = _package_set(baseline)
    candidate_set = _package_set(candidate)
    if baseline_set[1] != candidate_set[1]:
        raise LifecycleError("baseline and candidate architectures differ")
    comparison = _run(
        ["dpkg", "--compare-versions", baseline_set[0], "lt", candidate_set[0]],
        tools=tools,
        accepted=(0, 1),
    )
    if comparison.returncode != 0:
        raise LifecycleError("candidate package version must be newer than baseline")
    native = _load_script("rmac_lifecycle_preflight_native", "verify-native-packages.py")
    try:
        native.verify_directory(
            baseline,
            architecture=baseline_set[1],
            expected_version=baseline_set[0],
            dpkg_deb=tools["dpkg-deb"],
        )
        native.verify_directory(
            candidate,
            architecture=candidate_set[1],
            expected_version=candidate_set[0],
            dpkg_deb=tools["dpkg-deb"],
        )
    except Exception as error:
        raise LifecycleError("native package set failed pre-install verification") from error
    return baseline_set, candidate_set


def _create_fixture_user(tools: dict[str, str]) -> dict[str, str]:
    _run(["useradd", "--create-home", "--shell", "/bin/bash", TEST_USER], tools=tools)
    account = pwd.getpwnam(TEST_USER)
    if account.pw_uid == 0 or Path(account.pw_dir) != TEST_HOME:
        raise LifecycleError("lifecycle test user was not created safely")
    fingerprints: dict[str, str] = {}
    for label, relative in SENTINELS.items():
        path = TEST_HOME / relative
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        parent = path.parent
        while parent != TEST_HOME:
            os.chown(parent, account.pw_uid, account.pw_gid)
            parent = parent.parent
        path.write_bytes(SENTINEL_CONTENT)
        os.chown(path, account.pw_uid, account.pw_gid)
        fingerprints[label] = hashlib.sha256(SENTINEL_CONTENT).hexdigest()
    return fingerprints


def _verify_fixture_user(expected: dict[str, str]) -> None:
    for label, relative in SENTINELS.items():
        path = TEST_HOME / relative
        raw = _regular_bytes(path, 4096)
        if hashlib.sha256(raw).hexdigest() != expected[label]:
            raise LifecycleError(f"package lifecycle changed protected user data: {label}")


def _installed_versions(tools: dict[str, str]) -> dict[str, str]:
    versions = {}
    for package in PACKAGE_NAMES:
        result = _run(
            ["dpkg-query", "-W", "-f=${db:Status-Status}\\t${Version}", package],
            tools=tools,
        )
        try:
            status, version = result.stdout.decode("utf-8").strip().split("\t", 1)
        except (UnicodeDecodeError, ValueError) as error:
            raise LifecycleError("installed package state is invalid") from error
        if status != "installed":
            raise LifecycleError("native package is not fully installed")
        versions[package] = version
    return versions


def _require_versions(expected: str, tools: dict[str, str]) -> None:
    if _installed_versions(tools) != {package: expected for package in PACKAGE_NAMES}:
        raise LifecycleError("installed native package versions differ")


def _require_unpacked(expected: str, tools: dict[str, str]) -> None:
    for package in PACKAGE_NAMES:
        result = _run(
            ["dpkg-query", "-W", "-f=${db:Status-Status}\\t${Version}", package],
            tools=tools,
        )
        try:
            status, version = result.stdout.decode("utf-8").strip().split("\t", 1)
        except (UnicodeDecodeError, ValueError) as error:
            raise LifecycleError("interrupted package state is invalid") from error
        if status != "unpacked" or version != expected:
            raise LifecycleError("interrupted rollback state was not proven")


def _require_removed(tools: dict[str, str]) -> None:
    for package in PACKAGE_NAMES:
        result = _run(
            ["dpkg-query", "-W", "-f=${db:Status-Status}", package],
            tools=tools,
            accepted=(0, 1),
        )
        if result.returncode == 0 and result.stdout.decode("utf-8", "replace").strip() in {
            "installed",
            "unpacked",
            "config-files",
        }:
            raise LifecycleError("native package remained after purge")


def _gnome_recovery_exists() -> bool:
    sessions = Path("/usr/share/wayland-sessions")
    try:
        candidates = sorted(sessions.glob("*.desktop"))
    except OSError as error:
        raise LifecycleError("Wayland recovery-session inventory is unavailable") from error
    for candidate in candidates:
        if candidate.name == "rmac.desktop":
            continue
        parser = configparser.ConfigParser(interpolation=None, strict=True)
        parser.optionxform = str
        try:
            parser.read_string(_regular_bytes(candidate, 64 * 1024).decode("utf-8"))
        except (UnicodeDecodeError, configparser.Error, LifecycleError):
            continue
        if set(parser.sections()) != {"Desktop Entry"}:
            continue
        names = {
            value.strip().lower()
            for value in parser["Desktop Entry"].get("DesktopNames", "").split(";")
            if value.strip()
        }
        if "gnome" in names:
            return True
    return False


def _require_payload_removed() -> None:
    apps = _load_script("rmac_lifecycle_removed_apps", "verify-application-package.py")
    session = _load_script(
        "rmac_lifecycle_removed_session", "verify-session-package.py"
    )
    claimed = (
        set(apps.EXPECTED_PATHS)
        | {apps.MANIFEST}
        | set(session.EXPECTED_PATHS)
        | {session.MANIFEST}
        | {
            Path(specification.install_directory) / binary
            for specification in _load_script(
                "rmac_lifecycle_contract", "native_package_contract.py"
            ).PACKAGE_SPECS
            for binary in specification.binaries
        }
    )
    if any((Path("/") / relative).exists() for relative in claimed):
        raise LifecycleError("package-owned immutable payload remained after removal")


def _verify_installed(
    directory: Path,
    package_set: tuple[str, str, dict[str, object]],
    *,
    tools: dict[str, str],
) -> None:
    version, architecture, document = package_set
    native = _load_script("rmac_lifecycle_native", "verify-native-packages.py")
    session = _load_script("rmac_lifecycle_session", "verify-session-package.py")
    try:
        native.verify_directory(
            directory,
            architecture=architecture,
            expected_version=version,
            dpkg_deb=tools["dpkg-deb"],
        )
    except Exception as error:
        raise LifecycleError("installed package source failed verification") from error
    _require_versions(version, tools)
    for package in document["packages"]:
        for binary in package["binaries"]:
            path = Path(binary["path"])
            metadata = _regular_metadata(path, 1024 * 1024 * 1024)
            if (
                metadata.st_size != binary["size"]
                or _sha256(path, metadata.st_size) != binary["sha256"]
            ):
                raise LifecycleError("installed native binary differs from its package")
    try:
        session.verify_installed_host(Path("/"))
    except Exception as error:
        raise LifecycleError("installed session or GNOME recovery check failed") from error


def _record(
    reports: list[dict[str, object]],
    step: str,
    expected_version: str | None,
    fingerprints: dict[str, str],
) -> None:
    _verify_fixture_user(fingerprints)
    if not _gnome_recovery_exists():
        raise LifecycleError("stock GNOME recovery session disappeared")
    reports.append(
        {
            "gnome_recovery": True,
            "id": step,
            "package_version": expected_version,
            "protected_user_data": True,
        }
    )


def run_lifecycle(
    *,
    baseline: Path,
    candidate: Path,
    evidence: Path,
    contract: dict[str, object],
) -> None:
    tools = {
        name: _require_tool(name)
        for name in (
            "apt-get",
            "dpkg",
            "dpkg-deb",
            "dpkg-query",
            "useradd",
            "userdel",
        )
    }
    baseline_set, candidate_set = _preflight(
        contract,
        baseline=baseline,
        candidate=candidate,
        evidence=evidence,
        tools=tools,
    )
    baseline_archives = _archives(baseline, baseline_set[2])
    candidate_archives = _archives(candidate, candidate_set[2])
    reports: list[dict[str, object]] = []
    fingerprints = _create_fixture_user(tools)

    def mutation(command: list[str]) -> None:
        _check_disk(contract)
        _run(command, tools=tools)
        _verify_fixture_user(fingerprints)

    mutation(["apt-get", "install", "--yes", "--no-install-recommends", *baseline_archives])
    _verify_installed(baseline, baseline_set, tools=tools)
    _record(reports, "install-baseline", baseline_set[0], fingerprints)

    mutation(["apt-get", "install", "--yes", "--no-install-recommends", *candidate_archives])
    _verify_installed(candidate, candidate_set, tools=tools)
    _record(reports, "upgrade-candidate", candidate_set[0], fingerprints)

    mutation(["dpkg", "--unpack", *baseline_archives])
    _require_unpacked(baseline_set[0], tools)
    _record(
        reports,
        "interrupt-rollback-after-unpack",
        baseline_set[0],
        fingerprints,
    )

    mutation(["dpkg", "--configure", *PACKAGE_NAMES])
    _verify_installed(baseline, baseline_set, tools=tools)
    _record(reports, "recover-rollback", baseline_set[0], fingerprints)

    mutation(["apt-get", "remove", "--yes", *reversed(PACKAGE_NAMES)])
    _require_removed(tools)
    _require_payload_removed()
    _record(reports, "remove", None, fingerprints)

    mutation(["apt-get", "purge", "--yes", *reversed(PACKAGE_NAMES)])
    _require_removed(tools)
    _require_payload_removed()
    _record(reports, "purge", None, fingerprints)

    mutation(["apt-get", "install", "--yes", "--no-install-recommends", *candidate_archives])
    _verify_installed(candidate, candidate_set, tools=tools)
    _record(reports, "reinstall-candidate", candidate_set[0], fingerprints)

    mutation(["apt-get", "purge", "--yes", *reversed(PACKAGE_NAMES)])
    _require_removed(tools)
    _require_payload_removed()
    _record(reports, "final-purge", None, fingerprints)

    if [report["id"] for report in reports] != contract["steps"]:
        raise LifecycleError("lifecycle execution differs from the reviewed step order")
    document = {
        "architecture": candidate_set[1],
        "baseline_version": baseline_set[0],
        "candidate_version": candidate_set[0],
        "format": 1,
        "steps": reports,
    }
    _run(["userdel", "--remove", TEST_USER], tools=tools)
    evidence.mkdir(mode=0o755, exist_ok=True)
    temporary = evidence / ".package-lifecycle.json.tmp"
    temporary.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    temporary.chmod(0o644)
    os.replace(temporary, evidence / "package-lifecycle.json")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-contract", action="store_true")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--evidence", type=Path)
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        if arguments.check_contract:
            if any((arguments.baseline, arguments.candidate, arguments.evidence)):
                raise LifecycleError("--check-contract does not accept execution inputs")
            print("rmac package lifecycle contract verified")
            return 0
        if not all((arguments.baseline, arguments.candidate, arguments.evidence)):
            raise LifecycleError("--baseline, --candidate, and --evidence are required")
        run_lifecycle(
            baseline=arguments.baseline,
            candidate=arguments.candidate,
            evidence=arguments.evidence,
            contract=contract,
        )
    except LifecycleError as error:
        parser.exit(4, f"run-package-lifecycle: {error}\n")
    print(f"rmac package lifecycle passed; evidence: {arguments.evidence}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
