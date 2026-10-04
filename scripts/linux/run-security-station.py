#!/usr/bin/env python3
"""Run the security review's native checks on a disposable GitHub runner.

This is the "disposable install" station of docs/security-review-0.9.0-beta.1.md.
It installs, upgrades, rolls back and purges packages, adds users, polkit rules
and an APT source, and schedules PackageKit offline updates, so it refuses to
run anywhere but a GitHub-hosted runner that the workflow has marked
disposable (`/run/rmac-disposable-vm`, the same marker as
run-package-lifecycle.py). Never run it on the reference laptop or any machine
with data on it.

    run-security-station.py run --candidate DIR --candidate-source JSON \
        --work DIR --evidence DIR
    run-security-station.py gate --evidence DIR

`run` writes one JSON file per check plus `station-evidence.json`; `gate`
fails unless every check passed. All fixtures are synthetic: the users, the
password, the package names, the repository and its key exist only on the
throwaway runner, and the planted marker strings let the journal check prove
that none of them reached the log.
"""

from __future__ import annotations

import argparse
import datetime
import grp
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import traceback


REPO_ROOT = Path(__file__).resolve().parents[2]
STATION_ID = "github-disposable-ubuntu-26.04"
FORMAT = 1
MARKER = Path("/run/rmac-disposable-vm")
MARKER_CONTENTS = b"rmac-package-lifecycle-v1\n"
RMAC_PACKAGES = ("rmac-apps", "rmac-session")
THIRD_PARTY = ("niri", "xwayland-satellite")
# Planted synthetic values. None of them may ever appear in the journal.
SYNTHETIC_PASSWORD = "Synthetic-SR-Passw0rd-4c1f9e"
SYNTHETIC_PATH_TOKEN = "sr-private-path-8d2b7a"
SYNTHETIC_PACKAGE_TOKEN = "sr29-fixture-victim"
TEST_USER = "sr-station"
UPDATE_USER = "sr-update"
STATION_LIB = Path("/usr/local/lib/rmac-security-station")
SR29_REPO = Path("/srv") / f"sr29-repo-{SYNTHETIC_PATH_TOKEN}"
SR29_KEYRING = Path("/usr/share/keyrings/sr29-station.gpg")
SR29_SOURCE = Path("/etc/apt/sources.list.d/sr29-station.sources")
SR29_POLKIT_RULE = Path("/etc/polkit-1/rules.d/49-sr29-station.rules")
SR29_DENY_FLAG = Path("/run/sr29-deny-offline")
SR29_APT_SAVED = Path("/etc/apt/sr29-saved")
MAX_TEXT = 4000

# Which review checks each station check feeds (scripts/security-review.json).
REVIEW_MAP = {
    "candidate-provenance": [("packages", "artifact-contains-no-build-host-data")],
    "install-effects": [
        ("packages", "native-and-sandbox-boundaries-explicit"),
        ("packages", "rollback-and-uninstall-tested"),
    ],
    "package-permissions": [
        ("packages", "architecture-and-file-inventory-exact"),
        ("packages", "unpackaged-executables-rejected"),
    ],
    "package-lifecycle": [("packages", "rollback-and-uninstall-tested")],
    "systemd-hardening": [("dbus-polkit", "system-bus-callers-treated-untrusted")],
    "keyboard-relay": [
        ("dbus-polkit", "no-credential-collection"),
        ("desktop-entry-execution", "no-shell-interpolation"),
    ],
    "polkit-policy": [
        ("dbus-polkit", "interactive-authorization-only-from-user-action"),
        ("dbus-polkit", "denial-and-cancel-distinct"),
    ],
    "untrusted-open": [("file-operations", "untrusted-content-never-executed")],
    "terminal-wrapper": [
        ("desktop-entry-execution", "terminal-wrapper-argument-boundary")
    ],
    "lock-units": [("lock-boundary", "tty-recovery-proven")],
    "sr29-packagekit": [
        ("updates", "backend-failure-recovers-authoritatively"),
        ("updates", "cancellation-and-restart-readback"),
        ("updates", "signature-failure-fails-closed"),
        ("updates", "trusted-only-install-enforced"),
    ],
    "journal-redaction": [
        ("logs-diagnostics", "no-secrets-credentials-or-tokens"),
        ("logs-diagnostics", "no-private-paths-or-content"),
        ("logs-diagnostics", "control-characters-normalized"),
        ("logs-diagnostics", "failure-text-and-output-bounded"),
        ("logs-diagnostics", "bus-peers-and-session-identities-redacted"),
    ],
}


class StationError(RuntimeError):
    """A check failed; carries bounded observations."""


def bounded(text: str | bytes, limit: int = MAX_TEXT) -> str:
    if isinstance(text, bytes):
        text = text.decode("utf-8", "replace")
    return text if len(text) <= limit else text[:limit] + "…[truncated]"


def sh(
    command: list[str],
    *,
    check: bool = True,
    timeout: int = 900,
    user: str | None = None,
    env: dict[str, str] | None = None,
    input_bytes: bytes | None = None,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess[bytes]:
    if user is not None:
        command = ["runuser", "-u", user, "--", *command]
    environment = {
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "LC_ALL": "C.UTF-8",
        "DEBIAN_FRONTEND": "noninteractive",
    }
    if env:
        environment.update(env)
    result = subprocess.run(
        command,
        check=False,
        capture_output=True,
        timeout=timeout,
        env=environment,
        input=input_bytes,
        cwd=cwd,
    )
    if check and result.returncode != 0:
        raise StationError(
            f"command failed ({result.returncode}): {' '.join(command)[:300]}\n"
            f"{bounded(result.stderr, 1500)}"
        )
    return result


def text(result: subprocess.CompletedProcess[bytes]) -> str:
    return result.stdout.decode("utf-8", "replace")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise StationError(message)


# --------------------------------------------------------------------------
# preflight


def preflight(evidence: Path) -> None:
    if os.geteuid() != 0:
        sys.exit("run-security-station: run as root on the disposable runner")
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get(
        "RUNNER_ENVIRONMENT"
    ) != "github-hosted":
        sys.exit("run-security-station: only a GitHub-hosted runner is disposable")
    try:
        marker = MARKER.read_bytes()
    except OSError:
        marker = b""
    if marker != MARKER_CONTENTS or MARKER.is_symlink():
        sys.exit("run-security-station: the disposable-VM marker is missing")
    os_release = Path("/etc/os-release").read_text(encoding="utf-8")
    if 'ID=ubuntu' not in os_release or 'VERSION_ID="26.04"' not in os_release:
        sys.exit("run-security-station: Ubuntu 26.04 is required")
    evidence.mkdir(parents=True, exist_ok=True)


# --------------------------------------------------------------------------
# candidate set


def manifest(directory: Path) -> dict:
    return json.loads((directory / "native-packages.json").read_text(encoding="utf-8"))


def package_files(directory: Path) -> dict[str, Path]:
    files = {}
    for path in sorted(directory.glob("*.deb")):
        name = text(sh(["dpkg-deb", "-f", str(path), "Package"])).strip()
        files[name] = path
    return files


def check_candidate_provenance(state: dict) -> dict:
    candidate: Path = state["candidate"]
    source = json.loads(Path(state["candidate_source"]).read_text(encoding="utf-8"))
    document = manifest(candidate)
    files = package_files(candidate)
    require(set(RMAC_PACKAGES) <= set(files), "candidate lacks rmac packages")
    require(set(THIRD_PARTY) <= set(files), "candidate lacks niri/xwayland-satellite")
    verify = sh(
        [
            sys.executable,
            str(REPO_ROOT / "scripts/linux/verify-native-packages.py"),
            "--directory",
            str(candidate),
            "--architecture",
            "amd64",
        ],
        check=False,
    )
    patterns = {
        "home-directory": re.compile(rb"/home/[A-Za-z0-9._-]+/"),
        "macos-home": re.compile(rb"/Users/[A-Za-z0-9._-]+/"),
        "github-workspace": re.compile(rb"/(?:runner|github)/(?:work|home|workspace)\b"),
        "root-home": re.compile(rb"/root/\.(?:cargo|rustup)"),
    }
    hits: dict[str, dict[str, int]] = {}
    third_party_hits: dict[str, dict[str, int]] = {}
    scanned = 0
    for package, path in files.items():
        with tempfile.TemporaryDirectory(prefix="sr-scan-") as scratch:
            sh(["dpkg-deb", "-x", str(path), scratch])
            for item in Path(scratch).rglob("*"):
                if item.is_symlink() or not item.is_file():
                    continue
                data = item.read_bytes()
                scanned += 1
                for label, pattern in patterns.items():
                    count = len(pattern.findall(data))
                    if count:
                        target = hits if package in RMAC_PACKAGES else third_party_hits
                        relative = f"{package}:/{item.relative_to(scratch)}"
                        target.setdefault(relative, {})[label] = count
    observations = {
        "candidate_run_id": source.get("run_id"),
        "candidate_commit": source.get("head_sha"),
        "version": document.get("version"),
        "packages": {
            name: {
                "file": path.name,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }
            for name, path in files.items()
        },
        "verify_native_packages_exit": verify.returncode,
        "verify_native_packages_output": bounded(verify.stdout + verify.stderr, 800),
        "files_scanned": scanned,
        "rmac_build_host_hits": hits,
        "third_party_build_host_hits_informational": third_party_hits,
    }
    state["version"] = document.get("version")
    require(verify.returncode == 0, "verify-native-packages.py failed on the candidate")
    require(not hits, "rmac packages contain build-host paths")
    return observations


# --------------------------------------------------------------------------
# filesystem snapshots


SNAPSHOT_ROOTS = ("/etc", "/usr", "/opt", "/srv", "/var/lib", "/home", "/root")
SNAPSHOT_SKIP = (
    "/var/lib/dpkg",
    "/var/lib/apt",
    "/var/lib/PackageKit",
    "/var/lib/systemd",
    "/var/lib/ucf",
    "/var/lib/polkit-1",
    "/var/lib/sudo",
    "/var/lib/private",
    "/var/lib/dhcpcd",
    "/var/lib/waagent",
    "/var/lib/docker",
    "/var/lib/containerd",
    "/var/lib/snapd",
    "/var/lib/fwupd",
    "/var/lib/update-notifier",
    "/var/lib/ubuntu-advantage",
    "/var/lib/man-db",
    "/var/lib/aspell",
    "/var/lib/dictionaries-common",
    "/var/lib/logrotate",
    "/home/runner",
    "/usr/local/lib/rmac-security-station",
    "/etc/ld.so.cache",
)
# Caches regenerated by other packages' triggers when any package installs
# matching files. They are rebuilt, never authored by rmac's scripts.
TRIGGER_CACHES = (
    re.compile(r"^/usr/share/icons/[^/]+/icon-theme\.cache$"),
    re.compile(r"^/usr/share/applications/mimeinfo\.cache$"),
    re.compile(r"^/usr/share/glib-2\.0/schemas/gschemas\.compiled$"),
    re.compile(r"^/usr/share/mime/"),
    re.compile(r"^/usr/share/info/dir"),
    re.compile(r"^/usr/lib/x86_64-linux-gnu/gdk-pixbuf-2\.0/2\.10\.0/loaders\.cache$"),
    re.compile(r"^/usr/lib/python3[^/]*/.*__pycache__/"),
    re.compile(r"^/usr/lib/python3/dist-packages/.*__pycache__/"),
    re.compile(r"^/var/lib/swcatalog/"),
)


def snapshot() -> dict[str, tuple]:
    entries: dict[str, tuple] = {}
    for root in SNAPSHOT_ROOTS:
        for directory, subdirs, files in os.walk(root, followlinks=False):
            if directory.startswith(SNAPSHOT_SKIP):
                subdirs[:] = []
                continue
            subdirs[:] = [
                name
                for name in subdirs
                if not os.path.join(directory, name).startswith(SNAPSHOT_SKIP)
            ]
            for name in files + subdirs:
                path = os.path.join(directory, name)
                if path.startswith(SNAPSHOT_SKIP):
                    continue
                try:
                    metadata = os.lstat(path)
                except OSError:
                    continue
                kind = stat.S_IFMT(metadata.st_mode)
                target = os.readlink(path) if stat.S_ISLNK(metadata.st_mode) else ""
                entries[path] = (
                    kind,
                    stat.S_IMODE(metadata.st_mode),
                    metadata.st_uid,
                    metadata.st_gid,
                    metadata.st_size if not stat.S_ISDIR(metadata.st_mode) else 0,
                    metadata.st_mtime_ns if not stat.S_ISDIR(metadata.st_mode) else 0,
                    target,
                )
    return entries


def dpkg_paths(packages: tuple[str, ...]) -> set[str]:
    paths: set[str] = set()
    for package in packages:
        result = sh(["dpkg-query", "-L", package], check=False)
        for line in text(result).splitlines():
            line = line.strip()
            if line.startswith("/"):
                paths.add(line)
    return paths


def is_trigger_cache(path: str) -> bool:
    return any(pattern.search(path) for pattern in TRIGGER_CACHES)


def diff(before: dict, after: dict) -> tuple[list[str], list[str], list[str]]:
    added = sorted(set(after) - set(before))
    removed = sorted(set(before) - set(after))
    changed = sorted(
        path
        for path in set(before) & set(after)
        if before[path] != after[path] and not stat.S_ISDIR(after[path][0])
    )
    return added, removed, changed


def apt_install(paths: list[Path]) -> None:
    sh(["apt-get", "install", "--yes", "--no-install-recommends", *map(str, paths)])


def apt_purge(packages: tuple[str, ...]) -> None:
    sh(["apt-get", "purge", "--yes", *packages])


def install_candidate(state: dict) -> None:
    files = package_files(state["candidate"])
    apt_install([files[name] for name in RMAC_PACKAGES])


def keyd_config_text() -> str:
    return text(
        sh(
            [
                "/usr/libexec/rmac/rmac-mac-keyboard",
                "print-config",
                "--swap",
                "off",
                "--caps",
                "caps-lock",
                "--option-characters",
                "off",
            ]
        )
    )


def check_install_effects(state: dict) -> dict:
    files = package_files(state["candidate"])
    apt_install([files[name] for name in THIRD_PARTY])
    keyd_dir = Path("/etc/keyd")
    keyd_dir.mkdir(exist_ok=True)
    for stale in keyd_dir.glob("*.conf"):
        stale.unlink()
    before = snapshot()
    install_candidate(state)
    after_install = snapshot()
    owned = dpkg_paths(RMAC_PACKAGES)
    added, removed, changed = diff(before, after_install)
    unexpected_added = [
        path
        for path in added
        if path not in owned and not is_trigger_cache(path)
    ]
    unexpected_changed = [
        path for path in changed if path not in owned and not is_trigger_cache(path)
    ]
    keyd_created = Path("/etc/keyd/rmac.conf").exists()

    # Purge must leave exactly the pre-install tree (trigger caches aside).
    apt_purge(RMAC_PACKAGES)
    after_purge = snapshot()
    left_added, purge_removed, purge_changed = diff(before, after_purge)
    leftovers = [path for path in left_added if not is_trigger_cache(path)]
    foreign_removed = [path for path in purge_removed if not is_trigger_cache(path)]
    foreign_changed = [path for path in purge_changed if not is_trigger_cache(path)]

    # Maintainer scripts: a foreign /etc/keyd/rmac.conf is never touched.
    foreign = b"# an administrator's own keyd file\n[ids]\n*\n\n[main]\ncapslock = esc\n"
    Path("/etc/keyd/rmac.conf").write_bytes(foreign)
    install_candidate(state)
    foreign_after_install = Path("/etc/keyd/rmac.conf").read_bytes()
    apt_purge(RMAC_PACKAGES)
    foreign_after_purge = (
        Path("/etc/keyd/rmac.conf").read_bytes()
        if Path("/etc/keyd/rmac.conf").exists()
        else None
    )
    Path("/etc/keyd/rmac.conf").unlink(missing_ok=True)

    # An rmac-owned, stale file is regenerated on configure and removed on purge.
    stale = b"# rmac-mac-keyboard 1 swap=off caps=caps-lock option-characters=off\n[ids]\n*\n"
    Path("/etc/keyd/rmac.conf").write_bytes(stale)
    install_candidate(state)
    regenerated = Path("/etc/keyd/rmac.conf").read_text(encoding="utf-8")
    expected = keyd_config_text()
    apt_purge(RMAC_PACKAGES)
    owned_after_purge = Path("/etc/keyd/rmac.conf").exists()
    Path("/etc/keyd/rmac.conf").unlink(missing_ok=True)

    observations = {
        "install_added_entries": len(added),
        "install_added_package_owned": len([p for p in added if p in owned]),
        "install_unexpected_added": unexpected_added[:50],
        "install_unexpected_changed": unexpected_changed[:50],
        "install_removed_foreign": removed[:50],
        "keyd_config_created_by_install": keyd_created,
        "purge_leftovers": leftovers[:50],
        "purge_removed_foreign": foreign_removed[:50],
        "purge_changed_foreign": foreign_changed[:50],
        "foreign_keyd_config_untouched_by_configure": foreign_after_install == foreign,
        "foreign_keyd_config_survives_purge": foreign_after_purge == foreign,
        "rmac_keyd_config_regenerated_on_configure": regenerated == expected,
        "rmac_keyd_config_removed_on_purge": not owned_after_purge,
        "allowed_trigger_caches": [pattern.pattern for pattern in TRIGGER_CACHES],
    }
    require(not unexpected_added, "install created files outside the package")
    require(not unexpected_changed, "install changed files outside the package")
    require(not removed, "install removed existing files")
    require(not keyd_created, "install created /etc/keyd/rmac.conf")
    require(not leftovers, "purge left files behind")
    require(not foreign_removed, "purge removed files it did not install")
    require(not foreign_changed, "purge changed files it did not install")
    require(foreign_after_install == foreign, "configure changed a foreign keyd file")
    require(foreign_after_purge == foreign, "purge removed a foreign keyd file")
    require(regenerated == expected, "configure did not regenerate rmac's keyd file")
    require(not owned_after_purge, "purge kept rmac's keyd file")
    return observations


def check_package_permissions(state: dict) -> dict:
    install_candidate(state)
    problems: list[str] = []
    counted = 0
    for path in sorted(dpkg_paths(RMAC_PACKAGES)):
        try:
            metadata = os.lstat(path)
        except OSError:
            problems.append(f"missing {path}")
            continue
        if path in {"/", "/."}:
            continue
        counted += 1
        mode = stat.S_IMODE(metadata.st_mode)
        if metadata.st_uid != 0 or metadata.st_gid != 0:
            # Shared directories (for example /etc) belong to base-files.
            problems.append(f"owner {metadata.st_uid}:{metadata.st_gid} {path}")
        if mode & (stat.S_ISUID | stat.S_ISGID | stat.S_ISVTX):
            problems.append(f"special bits {oct(mode)} {path}")
        if stat.S_ISLNK(metadata.st_mode):
            continue
        if mode & 0o022:
            problems.append(f"group/world writable {oct(mode)} {path}")
        if stat.S_ISREG(metadata.st_mode) and mode not in (0o644, 0o755):
            problems.append(f"mode {oct(mode)} {path}")
    capabilities = text(sh(["getcap", "-r", "/usr/bin", "/usr/libexec/rmac"], check=False))
    rmac_caps = [
        line
        for line in capabilities.splitlines()
        if "/usr/libexec/rmac/" in line or "/rmac-" in line
    ]
    local = sorted(
        str(path)
        for path in Path("/usr/local/bin").glob("*")
        if "rmac" in path.name
    )
    observations = {
        "paths_checked": counted,
        "problems": problems[:50],
        "file_capabilities": rmac_caps,
        "usr_local_rmac": local,
    }
    require(not problems, "package ownership or permissions are wrong")
    require(not rmac_caps, "an rmac executable carries file capabilities")
    require(not local, "rmac files appeared in /usr/local")
    return observations


# --------------------------------------------------------------------------
# lifecycle


def build_baseline(state: dict) -> Path:
    candidate: Path = state["candidate"]
    work: Path = state["work"]
    document = manifest(candidate)
    files = package_files(candidate)
    extract = work / "extract"
    inputs = work / "baseline-inputs"
    shutil.rmtree(extract, ignore_errors=True)
    shutil.rmtree(inputs, ignore_errors=True)
    extract.mkdir(parents=True)
    inputs.mkdir()
    for package in RMAC_PACKAGES:
        sh(["dpkg-deb", "-x", str(files[package]), str(extract)])
    for record in document["packages"]:
        for binary in record["binaries"]:
            source = extract / binary["path"].lstrip("/")
            shutil.copy2(source, inputs / Path(binary["path"]).name)
    source_tree = work / "baseline-src"
    shutil.rmtree(source_tree, ignore_errors=True)
    shutil.copytree(
        REPO_ROOT,
        source_tree,
        symlinks=True,
        ignore=shutil.ignore_patterns(
            ".git", "target", "station-work", "station-evidence", "design-lab"
        ),
    )
    contract = source_tree / "scripts/linux/native_package_contract.py"
    original = contract.read_text(encoding="utf-8")
    match = re.search(r"^DEBIAN_REVISION = (\d+)$", original, re.M)
    require(match is not None, "DEBIAN_REVISION not found")
    revision = int(match.group(1))
    require(revision > 1, "baseline needs a lower Debian revision")
    contract.write_text(
        original.replace(
            match.group(0), f"DEBIAN_REVISION = {revision - 1}"
        ),
        encoding="utf-8",
    )
    baseline = work / "baseline"
    shutil.rmtree(baseline, ignore_errors=True)
    sh(
        [
            sys.executable,
            str(source_tree / "scripts/linux/build-native-packages.py"),
            "--binary-dir",
            str(inputs),
            "--output",
            str(baseline),
            "--architecture",
            "amd64",
            "--source-date-epoch",
            "1767225600",
        ],
        timeout=1800,
    )
    return baseline


def check_package_lifecycle(state: dict) -> dict:
    apt_purge(RMAC_PACKAGES)
    baseline = build_baseline(state)
    lifecycle_evidence = state["work"] / "lifecycle-evidence"
    shutil.rmtree(lifecycle_evidence, ignore_errors=True)
    result = sh(
        [
            sys.executable,
            str(REPO_ROOT / "scripts/linux/run-package-lifecycle.py"),
            "--baseline",
            str(baseline),
            "--candidate",
            str(state["candidate"]),
            "--evidence",
            str(lifecycle_evidence),
        ],
        check=False,
        timeout=3600,
    )
    report_path = lifecycle_evidence / "package-lifecycle.json"
    report = (
        json.loads(report_path.read_text(encoding="utf-8"))
        if report_path.exists()
        else None
    )
    observations = {
        "baseline_version": manifest(baseline)["version"],
        "candidate_version": manifest(state["candidate"])["version"],
        "baseline_note": (
            "the baseline repackages the candidate's own binaries one Debian "
            "revision lower, so the upgrade and rollback exercise packaging, "
            "maintainer scripts and dpkg state rather than different binaries"
        ),
        "exit": result.returncode,
        "output": bounded(result.stdout + result.stderr, 1500),
        "report": report,
    }
    require(result.returncode == 0 and report is not None, "package lifecycle failed")
    return observations


# --------------------------------------------------------------------------
# users


def ensure_user(name: str, *, password: bool = False) -> pwd.struct_passwd:
    try:
        account = pwd.getpwnam(name)
    except KeyError:
        sh(["useradd", "--create-home", "--shell", "/bin/bash", name])
        account = pwd.getpwnam(name)
    if password:
        sh(["chpasswd"], input_bytes=f"{name}:{SYNTHETIC_PASSWORD}\n".encode())
    require(account.pw_uid != 0, "test user must not be root")
    return account


def runtime_dir(account: pwd.struct_passwd) -> Path:
    path = Path(f"/run/user/{account.pw_uid}")
    if not path.exists():
        path.mkdir(mode=0o700, parents=True)
        os.chown(path, account.pw_uid, account.pw_gid)
    return path


def user_env(account: pwd.struct_passwd, extra: dict[str, str] | None = None) -> dict:
    environment = {
        "HOME": account.pw_dir,
        "USER": account.pw_name,
        "LOGNAME": account.pw_name,
        "XDG_RUNTIME_DIR": str(runtime_dir(account)),
        "XDG_CONFIG_HOME": f"{account.pw_dir}/.config",
        "XDG_DATA_HOME": f"{account.pw_dir}/.local/share",
        "XDG_STATE_HOME": f"{account.pw_dir}/.local/state",
        "XDG_CACHE_HOME": f"{account.pw_dir}/.cache",
    }
    if extra:
        environment.update(extra)
    return environment


def write_owned(path: Path, data: bytes, account: pwd.struct_passwd, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    os.chmod(path, mode)
    os.chown(path, account.pw_uid, account.pw_gid)


# --------------------------------------------------------------------------
# systemd and polkit


def check_systemd_hardening(state: dict) -> dict:
    unit_dir = Path("/usr/lib/systemd/system")
    socket_unit = unit_dir / "rmac-mac-keyboard-relay.socket"
    service_unit = unit_dir / "rmac-mac-keyboard-relay@.service"
    require(socket_unit.exists() and service_unit.exists(), "relay units missing")
    verify = sh(
        ["systemd-analyze", "verify", str(socket_unit), str(service_unit)], check=False
    )
    sh(["systemctl", "daemon-reload"])
    instance = "rmac-mac-keyboard-relay@0-0.service"
    security = sh(
        ["systemd-analyze", "security", "--no-pager", "--json=short", instance],
        check=False,
    )
    exposure = None
    failing: list[str] = []
    try:
        table = json.loads(text(security))
        failing = sorted(
            entry["name"]
            for entry in table
            if entry.get("set") is False and entry.get("exposure")
        )
    except (json.JSONDecodeError, TypeError, KeyError):
        table = None
    summary = sh(["systemd-analyze", "security", "--no-pager", instance], check=False)
    match = re.search(r"Overall exposure level for \S+: ([0-9.]+)", text(summary))
    if match:
        exposure = float(match.group(1))
    properties = text(
        sh(
            [
                "systemctl",
                "show",
                instance,
                "-p",
                "DynamicUser,NoNewPrivileges,CapabilityBoundingSet,AmbientCapabilities,"
                "PrivateNetwork,RestrictAddressFamilies,ProtectSystem,ProtectHome,"
                "SupplementaryGroups,User,MemoryDenyWriteExecute",
            ]
        )
    )
    parsed = dict(line.split("=", 1) for line in properties.splitlines() if "=" in line)
    observations = {
        "systemd_analyze_verify_exit": verify.returncode,
        "systemd_analyze_verify_output": bounded(verify.stderr, 800),
        "relay_overall_exposure": exposure,
        "relay_exposure_summary_tail": bounded(text(summary)[-600:], 600),
        "relay_unset_hardening_items": failing[:40],
        "relay_properties": parsed,
    }
    require(verify.returncode == 0, "systemd-analyze verify reported problems")
    require(exposure is not None and exposure <= 3.0, "relay exposure above 3.0 (OK)")
    require(parsed.get("DynamicUser") == "yes", "relay is not DynamicUser")
    require(parsed.get("NoNewPrivileges") == "yes", "relay lacks NoNewPrivileges")
    require(parsed.get("CapabilityBoundingSet", "x") == "", "relay keeps capabilities")
    require(parsed.get("PrivateNetwork") == "yes", "relay has network access")
    require(parsed.get("RestrictAddressFamilies") == "AF_UNIX", "relay families wider")
    return observations


def relay_request(account: pwd.struct_passwd, payload: bytes) -> str:
    script = (
        "import socket,sys\n"
        "s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)\n"
        "s.settimeout(8)\n"
        "s.connect('/run/rmac-mac-keyboard.socket')\n"
        "s.sendall(sys.stdin.buffer.read())\n"
        "s.shutdown(socket.SHUT_WR)\n"
        "data=b''\n"
        "while True:\n"
        "    chunk=s.recv(64)\n"
        "    if not chunk: break\n"
        "    data+=chunk\n"
        "sys.stdout.write(data.decode('ascii','replace'))\n"
    )
    result = sh(
        ["python3", "-c", script],
        user=account.pw_name,
        input_bytes=payload,
        check=False,
        timeout=30,
    )
    return text(result).strip() or f"<no reply, exit {result.returncode}>"


def check_keyboard_relay(state: dict) -> dict:
    account = ensure_user(TEST_USER, password=True)
    cursor = journal_cursor()
    apply = sh(
        [
            "/usr/libexec/rmac/rmac-mac-keyboard",
            "apply",
            "--shortcuts",
            "on",
            "--swap",
            "off",
            "--caps",
            "caps-lock",
            "--option-characters",
            "off",
        ],
        check=False,
        timeout=120,
    )
    if not Path("/etc/keyd/rmac.conf").exists():
        # localed may be absent on a runner; the relay test needs only the file.
        Path("/etc/keyd/rmac.conf").write_text(keyd_config_text(), encoding="utf-8")
    sh(["systemctl", "start", "keyd.service"], check=False)
    sh(["systemctl", "enable", "--now", "rmac-mac-keyboard-relay.socket"])
    metadata = os.stat("/run/rmac-mac-keyboard.socket")
    keyd_active = text(sh(["systemctl", "is-active", "keyd.service"], check=False)).strip()
    pwn = Path("/tmp/sr-relay-pwned")
    pwn.unlink(missing_ok=True)
    injected = {
        "valid-native": b"native\n",
        "command-binding": b"command(touch /tmp/sr-relay-pwned)\n",
        "shell-metacharacters": b"native; touch /tmp/sr-relay-pwned\n",
        "no-newline": b"native",
        "two-lines": b"terminal\nnative\n",
        "oversized": b"n" * 70000 + b"\n",
        "nul": b"native\0\n",
    }
    replies = {label: relay_request(account, payload) for label, payload in injected.items()}
    time.sleep(1)
    keyd_group = grp.getgrnam("keyd") if "keyd" in {g.gr_name for g in grp.getgrall()} else None
    log = journal_since(cursor, ["-u", "rmac-mac-keyboard-relay@*", "-u", "keyd.service"])
    sh(
        [
            "/usr/libexec/rmac/rmac-mac-keyboard",
            "apply",
            "--shortcuts",
            "off",
            "--swap",
            "off",
            "--caps",
            "caps-lock",
            "--option-characters",
            "off",
        ],
        check=False,
        timeout=120,
    )
    observations = {
        "helper_apply_exit": apply.returncode,
        "helper_apply_stderr": bounded(apply.stderr, 400),
        "socket_mode": oct(stat.S_IMODE(metadata.st_mode)),
        "socket_owner": f"{metadata.st_uid}:{metadata.st_gid}",
        "keyd_active": keyd_active,
        "replies": replies,
        "command_binding_executed": pwn.exists(),
        "keyd_group_members": list(keyd_group.gr_mem) if keyd_group else None,
        "session_user_in_keyd": bool(keyd_group and TEST_USER in keyd_group.gr_mem),
        "relay_log_echoes_request": "touch /tmp/sr-relay-pwned" in log,
        "relay_log_tail": bounded(log[-800:], 800),
    }
    for label in injected:
        if label == "valid-native":
            continue
        require(replies[label] == "error", f"relay accepted {label}")
    if keyd_active == "active":
        require(replies["valid-native"] == "ok", "relay refused a valid profile")
    require(not pwn.exists(), "a relay request ran a command")
    require(not observations["session_user_in_keyd"], "session user is in keyd")
    require(not observations["relay_log_echoes_request"], "relay logged the request")
    return observations


def check_polkit_policy(state: dict) -> dict:
    account = ensure_user(TEST_USER, password=True)
    sh(["systemctl", "start", "polkit.service"], check=False)
    action = text(
        sh(["pkaction", "--action-id", "org.rmac.mac-keyboard.apply", "--verbose"])
    )
    implicit = {
        key: value.strip()
        for key, value in re.findall(r"implicit (any|inactive|active):\s*(\S+)", action)
    }
    rules = sorted(p for p in dpkg_paths(RMAC_PACKAGES) if "/rules.d/" in p)
    policies = sorted(p for p in dpkg_paths(RMAC_PACKAGES) if p.endswith(".policy"))
    before = Path("/etc/keyd/rmac.conf").exists()
    pkexec = sh(
        [
            "pkexec",
            "/usr/libexec/rmac/rmac-mac-keyboard",
            "apply",
            "--shortcuts",
            "on",
            "--swap",
            "off",
            "--caps",
            "caps-lock",
            "--option-characters",
            "off",
        ],
        user=account.pw_name,
        env=user_env(account),
        check=False,
        timeout=60,
    )
    after = Path("/etc/keyd/rmac.conf").exists()
    pkcheck = sh(
        [
            "sh",
            "-c",
            "exec pkcheck --action-id org.rmac.mac-keyboard.apply --process $$",
        ],
        user=account.pw_name,
        check=False,
        timeout=60,
    )
    observations = {
        "implicit": implicit,
        "rmac_polkit_rules": rules,
        "rmac_polkit_policies": policies,
        "pkexec_without_agent_exit": pkexec.returncode,
        "pkexec_stderr": bounded(pkexec.stderr, 300),
        "helper_ran_without_authorization": after and not before,
        "pkcheck_exit": pkcheck.returncode,
        "pkcheck_output": bounded(pkcheck.stdout + pkcheck.stderr, 300),
    }
    require(
        implicit == {"any": "auth_admin", "inactive": "auth_admin", "active": "auth_admin_keep"},
        "keyboard polkit defaults differ from the reviewed policy",
    )
    require(not rules, "rmac ships polkit rules")
    require(pkexec.returncode == 127, "pkexec denial is not exit 127")
    require(not observations["helper_ran_without_authorization"], "helper ran unauthorized")
    require(pkcheck.returncode != 0, "pkcheck authorized an unprivileged user")
    return observations


# --------------------------------------------------------------------------
# untrusted content


ELF_SOURCE = (
    "#include <fcntl.h>\n#include <stdlib.h>\n#include <string.h>\n"
    "int main(void){char p[512];snprintf(p,sizeof p,\"%s/MARKER-elf\",getenv(\"HOME\"));"
    "return creat(p,0644)<0;}\n"
)


def check_untrusted_open(state: dict) -> dict:
    account = ensure_user(TEST_USER, password=True)
    home = Path(account.pw_dir)
    downloads = home / "Downloads"
    shutil.rmtree(downloads, ignore_errors=True)
    for marker in home.glob("MARKER-*"):
        marker.unlink()
    fixtures = {
        "evil.desktop": (
            b"[Desktop Entry]\nType=Application\nName=Invoice\n"
            b"Exec=sh -c 'touch \"$HOME/MARKER-desktop\"'\nIcon=text-x-generic\n",
            0o755,
        ),
        "evil-noexec.desktop": (
            b"[Desktop Entry]\nType=Application\nName=Invoice\n"
            b"Exec=sh -c 'touch \"$HOME/MARKER-desktop-noexec\"'\n",
            0o644,
        ),
        "evil.sh": (b"#!/bin/sh\ntouch \"$HOME/MARKER-sh\"\n", 0o755),
        "evil-noext": (b"#!/bin/sh\ntouch \"$HOME/MARKER-noext\"\n", 0o755),
        "evil.py": (
            b"#!/usr/bin/python3\nimport os\nopen(os.environ['HOME']+'/MARKER-py','w')\n",
            0o755,
        ),
        "$(touch MARKER-name).txt": (b"plain text\n", 0o644),
        "evil.txt": (b"#!/bin/sh\ntouch \"$HOME/MARKER-txt\"\n", 0o755),
    }
    for name, (data, mode) in fixtures.items():
        write_owned(downloads / name, data, account, mode)
    elf_source = state["work"] / "evil-elf.c"
    elf_source.write_text(ELF_SOURCE.replace("#include <string.h>", "#include <stdio.h>"))
    compiled = sh(["cc", "-O0", "-o", str(downloads / "evil-elf"), str(elf_source)], check=False)
    if compiled.returncode == 0:
        os.chown(downloads / "evil-elf", account.pw_uid, account.pw_gid)
        os.chmod(downloads / "evil-elf", 0o755)
    environment = user_env(
        account,
        {"XDG_CURRENT_DESKTOP": "rmac:niri", "XDG_SESSION_TYPE": "wayland"},
    )
    results = {}
    for item in sorted(downloads.iterdir()):
        mime = text(
            sh(["xdg-mime", "query", "filetype", str(item)], user=account.pw_name,
               env=environment, check=False)
        ).strip()
        handler = text(
            sh(["xdg-mime", "query", "default", mime], user=account.pw_name,
               env=environment, check=False)
        ).strip() if mime else ""
        # Files' open path: the OpenURI portal first, xdg-open when that fails
        # (crates/rmac-portal/src/open.rs open_item). Both run here inside a
        # private session bus with no display, as a headless session would.
        portal_script = (
            "set -u\n"
            "/usr/libexec/xdg-desktop-portal >/dev/null 2>&1 & portal=$!\n"
            "sleep 2\n"
            "fd_uri=\"file://$1\"\n"
            "timeout 10 gdbus call --session --dest org.freedesktop.portal.Desktop "
            "--object-path /org/freedesktop/portal/desktop "
            "--method org.freedesktop.portal.OpenURI.OpenURI '' \"$fd_uri\" '{}' "
            ">/dev/null 2>&1; echo \"portal=$?\"\n"
            "timeout 10 xdg-open \"$1\" >/dev/null 2>&1; echo \"xdg-open=$?\"\n"
            "sleep 3\n"
            "kill $portal 2>/dev/null; wait 2>/dev/null; exit 0\n"
        )
        run = sh(
            ["dbus-run-session", "--", "sh", "-c", portal_script, "open", str(item)],
            user=account.pw_name,
            env=environment,
            check=False,
            timeout=60,
        )
        results[item.name] = {
            "mime": mime,
            "default_handler": handler,
            "exits": text(run).strip().splitlines(),
        }
    time.sleep(2)
    markers = sorted(path.name for path in home.glob("MARKER-*"))
    markers += sorted(path.name for path in downloads.glob("MARKER-*"))
    observations = {
        "fixtures": results,
        "elf_fixture_built": compiled.returncode == 0,
        "markers_created": markers,
    }
    require(not markers, "opening untrusted content executed it")
    return observations


# --------------------------------------------------------------------------
# terminal wrapper


ARGV_PROBE = """#!/usr/bin/python3
import json, os, sys
with open(os.path.join(os.environ["HOME"], "argv-probe.json"), "w") as out:
    json.dump(sys.argv[1:], out)
"""

TERMINAL_ARGS = [
    "two words",
    ";touch $HOME/MARKER-term-semicolon",
    "$(touch $HOME/MARKER-term-subst)",
    "`touch $HOME/MARKER-term-backtick`",
    "--flag=value",
    "-e",
    "'quoted'",
    "",
]


def check_terminal_wrapper(state: dict) -> dict:
    account = ensure_user(TEST_USER, password=True)
    home = Path(account.pw_dir)
    probe = STATION_LIB / "argv-probe"
    probe.parent.mkdir(parents=True, exist_ok=True)
    probe.write_text(ARGV_PROBE, encoding="utf-8")
    probe.chmod(0o755)
    (home / "argv-probe.json").unlink(missing_ok=True)
    for marker in home.glob("MARKER-term-*"):
        marker.unlink()
    alternative = os.path.realpath("/usr/bin/x-terminal-emulator")
    sway_config = STATION_LIB / "sway.conf"
    sway_config.write_text("output * bg #000000 solid_color\n", encoding="utf-8")
    environment = user_env(
        account,
        {
            "WLR_BACKENDS": "headless",
            "WLR_RENDERER": "pixman",
            "WLR_LIBINPUT_NO_DEVICES": "1",
            "GSK_RENDERER": "cairo",
            "LIBGL_ALWAYS_SOFTWARE": "1",
            "XDG_CURRENT_DESKTOP": "rmac:niri",
        },
    )
    # The argv rmac builds for Terminal=true (crates/rmac-apps/src/catalog.rs
    # activation_spawn_argv_with_terminal): x-terminal-emulator -e PROGRAM ARGS…
    argv = ["x-terminal-emulator", "-e", str(probe), *TERMINAL_ARGS]
    quoted = " ".join("'" + part.replace("'", "'\\''") + "'" for part in argv)
    script = (
        "set -u\n"
        f"sway -c {sway_config} >/dev/null 2>&1 & sway=$!\n"
        "for i in $(seq 1 50); do [ -S \"$XDG_RUNTIME_DIR/wayland-1\" ] && break; sleep 0.2; done\n"
        "export WAYLAND_DISPLAY=wayland-1\n"
        f"{quoted} >/dev/null 2>\"$HOME/terminal-stderr.txt\" & term=$!\n"
        "for i in $(seq 1 150); do [ -s \"$HOME/argv-probe.json\" ] && break; sleep 0.2; done\n"
        "kill $term $sway 2>/dev/null; wait 2>/dev/null; exit 0\n"
    )
    sh(
        ["dbus-run-session", "--", "sh", "-c", script],
        user=account.pw_name,
        env=environment,
        check=False,
        timeout=120,
    )
    received = None
    if (home / "argv-probe.json").exists():
        received = json.loads((home / "argv-probe.json").read_text(encoding="utf-8"))
    stderr_path = home / "terminal-stderr.txt"
    markers = sorted(path.name for path in home.glob("MARKER-term-*"))
    observations = {
        "x_terminal_emulator": alternative,
        "terminal_package": text(
            sh(["dpkg-query", "-S", alternative], check=False)
        ).strip(),
        "argv_sent": TERMINAL_ARGS,
        "argv_received": received,
        "markers_created": markers,
        "terminal_stderr": bounded(
            stderr_path.read_text(errors="replace") if stderr_path.exists() else "", 600
        ),
    }
    require(not markers, "the terminal re-parsed arguments through a shell")
    require(received == TERMINAL_ARGS, "the terminal did not preserve the argv boundary")
    return observations


# --------------------------------------------------------------------------
# lock units (no display needed)


def check_lock_units(state: dict) -> dict:
    user_units = Path("/usr/lib/systemd/user")
    lock = (user_units / "rmac-lock.service").read_text(encoding="utf-8")
    fallback_path = user_units / "rmac-lock-fallback.service"
    fallback = fallback_path.read_text(encoding="utf-8") if fallback_path.exists() else ""
    pam = Path("/etc/pam.d/rmac-lock")
    pam_text = pam.read_text(encoding="utf-8") if pam.exists() else ""
    verify = sh(
        ["systemd-analyze", "verify", "--user", str(user_units / "rmac-lock.service")],
        check=False,
    )
    observations = {
        "assert_pam_service": "AssertPathExists=/etc/pam.d/rmac-lock" in lock,
        "on_failure_fallback": "OnFailure=rmac-lock-fallback.service" in lock,
        "core_dumps_disabled": "LimitCORE=0" in lock,
        "fallback_unit_installed": bool(fallback),
        "fallback_runs_swaylock": "swaylock" in fallback,
        "swaylock_installed": Path("/usr/bin/swaylock").exists(),
        "pam_service_installed": bool(pam_text),
        "pam_includes": re.findall(r"^@include\s+(\S+)", pam_text, re.M),
        "systemd_analyze_verify_exit": verify.returncode,
        "systemd_analyze_verify_output": bounded(verify.stderr, 600),
        "scope_note": (
            "proves the units and PAM service the TTY runbook names are installed "
            "as documented; switching to a TTY and recovering a hung lock needs "
            "the reference laptop"
        ),
    }
    for key in (
        "assert_pam_service",
        "on_failure_fallback",
        "core_dumps_disabled",
        "fallback_unit_installed",
        "fallback_runs_swaylock",
        "swaylock_installed",
        "pam_service_installed",
    ):
        require(bool(observations[key]), f"lock unit check failed: {key}")
    require(
        observations["pam_includes"] == ["common-auth", "common-account"],
        "rmac-lock PAM service includes more than common-auth/common-account",
    )
    return observations


# --------------------------------------------------------------------------
# SR-29: the automatic update checker against the real PackageKit


NOTIFY_CAPTURE = r"""#!/usr/bin/python3
import json, sys
from gi.repository import Gio, GLib
XML = '''<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/>
<arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/>
<arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
<arg type="u" direction="out"/></method>
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/>
<arg type="s" direction="out"/><arg type="s" direction="out"/></method>
</interface></node>'''
out = sys.argv[1]
node = Gio.DBusNodeInfo.new_for_xml(XML)
counter = [0]
def call(conn, sender, path, iface, method, params, invocation):
    if method == "Notify":
        values = params.unpack()
        with open(out, "a") as stream:
            stream.write(json.dumps({"app": values[0], "summary": values[3], "body": values[4]}) + "\n")
        counter[0] += 1
        invocation.return_value(GLib.Variant("(u)", (counter[0],)))
    elif method == "GetCapabilities":
        invocation.return_value(GLib.Variant("(as)", (["body"],)))
    else:
        invocation.return_value(GLib.Variant("(ssss)", ("sr", "sr", "1", "1.2")))
def acquired(conn, name):
    conn.register_object("/org/freedesktop/Notifications", node.interfaces[0], call, None, None)
Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.Notifications",
                 Gio.BusNameOwnerFlags.NONE, acquired, None, None)
GLib.MainLoop().run()
"""

PK_STATE = r"""
import json, gi
gi.require_version("PackageKitGlib", "1.0")
from gi.repository import PackageKitGlib as Pk, GLib
state = {}
try:
    state["action"] = Pk.offline_action_to_string(Pk.offline_get_action())
except GLib.Error as error:
    state["action"] = "error"
try:
    state["prepared"] = sorted(Pk.offline_get_prepared_ids() or [])
except GLib.Error:
    state["prepared"] = []
print(json.dumps(state))
"""

PK_CANCEL = r"""
import gi
gi.require_version("PackageKitGlib", "1.0")
from gi.repository import PackageKitGlib as Pk, GLib
try:
    Pk.offline_cancel_with_flags(Pk.OfflineFlags.NONE, None)
except GLib.Error:
    pass
"""

POLKIT_RULE = """// SR-29 station fixture (disposable runner only). Grants the synthetic
// update user what Ubuntu's PackageKit policy grants an active local user,
// and can deny the offline-update action to prove cancellation failure.
polkit.addRule(function(action, subject) {
    if (subject.user != "%(user)s" ||
        action.id.indexOf("org.freedesktop.packagekit.") != 0) {
        return polkit.Result.NOT_HANDLED;
    }
    if (action.id.indexOf("offline") >= 0) {
        try {
            polkit.spawn(["/usr/bin/test", "-e", "%(flag)s"]);
            return polkit.Result.NO;
        } catch (error) {
        }
    }
    return polkit.Result.YES;
});
"""


def pk_state() -> dict:
    return json.loads(text(sh(["python3", "-c", PK_STATE])))


def build_fixture_deb(work: Path, name: str, version: str, extra: str = "") -> Path:
    root = work / f"deb-{name}-{version}"
    shutil.rmtree(root, ignore_errors=True)
    (root / "DEBIAN").mkdir(parents=True)
    (root / "usr/share/doc" / name).mkdir(parents=True)
    (root / "usr/share/doc" / name / "fixture").write_text("sr29 synthetic fixture\n")
    (root / "DEBIAN/control").write_text(
        f"Package: {name}\nVersion: {version}\nArchitecture: all\n"
        "Maintainer: SR29 Station <sr29@invalid>\nPriority: optional\n"
        f"Section: misc\n{extra}Description: SR-29 station fixture\n"
        " Synthetic package for the disposable security station.\n"
    )
    output = work / f"{name}_{version}_all.deb"
    sh(["dpkg-deb", "--root-owner-group", "-b", str(root), str(output)])
    return output


def publish_repo(work: Path, debs: list[Path], gnupg: Path, key: str) -> None:
    shutil.rmtree(SR29_REPO, ignore_errors=True)
    SR29_REPO.mkdir(parents=True)
    for deb in debs:
        shutil.copy2(deb, SR29_REPO / deb.name)
    packages = text(sh(["apt-ftparchive", "packages", "."], cwd=SR29_REPO))
    (SR29_REPO / "Packages").write_text(packages)
    release = text(
        sh(
            [
                "apt-ftparchive",
                "-o", "APT::FTPArchive::Release::Origin=SR29Station",
                "-o", "APT::FTPArchive::Release::Label=SR29Station",
                "-o", "APT::FTPArchive::Release::Suite=sr29",
                "release",
                ".",
            ],
            cwd=SR29_REPO,
        )
    )
    (SR29_REPO / "Release").write_text(release)
    sh(
        [
            "gpg", "--homedir", str(gnupg), "--batch", "--yes", "--local-user", key,
            "--clearsign", "--output", str(SR29_REPO / "InRelease"),
            str(SR29_REPO / "Release"),
        ]
    )
    for path in [SR29_REPO, *SR29_REPO.iterdir()]:
        os.chmod(path, 0o755 if path.is_dir() else 0o644)


def new_key(gnupg: Path, uid: str) -> str:
    sh(
        [
            "gpg", "--homedir", str(gnupg), "--batch", "--pinentry-mode", "loopback",
            "--passphrase", "", "--quick-gen-key", uid, "ed25519", "sign", "never",
        ]
    )
    listing = text(
        sh(["gpg", "--homedir", str(gnupg), "--with-colons", "--list-keys", uid])
    )
    return next(line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr"))


def run_update_unit(account: pwd.struct_passwd, label: str) -> dict:
    machine = f"{account.pw_name}@"
    capture = Path(account.pw_dir) / "notifications.jsonl"
    capture.unlink(missing_ok=True)
    sh(["systemctl", "--user", "-M", machine, "reset-failed", "rmac-update-check.service"],
       check=False)
    cursor = journal_cursor()
    started = time.time()
    result = sh(
        ["systemctl", "--user", "-M", machine, "start", "rmac-update-check.service"],
        check=False,
        timeout=900,
    )
    log = journal_since(
        cursor, [f"_UID={account.pw_uid}", "--user-unit", "rmac-update-check.service"]
    )
    notifications = []
    if capture.exists():
        notifications = [
            json.loads(line) for line in capture.read_text().splitlines() if line.strip()
        ]
    status_file = Path(account.pw_dir) / ".local/state/rmac/software-update-status"
    return {
        "case": label,
        "unit_start_exit": result.returncode,
        "seconds": round(time.time() - started, 1),
        "checker_lines": [
            line for line in log.splitlines() if "rmac-update-check:" in line
        ][:20],
        "notifications": notifications,
        "status_file": status_file.read_text() if status_file.exists() else None,
        "packagekit_after": pk_state(),
    }


def check_sr29_packagekit(state: dict) -> dict:
    work: Path = state["work"] / "sr29"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    account = ensure_user(UPDATE_USER)
    STATION_LIB.mkdir(parents=True, exist_ok=True)
    capture_script = STATION_LIB / "notify-capture.py"
    capture_script.write_text(NOTIFY_CAPTURE, encoding="utf-8")
    capture_script.chmod(0o755)
    SR29_POLKIT_RULE.write_text(
        POLKIT_RULE % {"user": UPDATE_USER, "flag": str(SR29_DENY_FLAG)}, encoding="utf-8"
    )
    SR29_DENY_FLAG.unlink(missing_ok=True)
    sh(["systemctl", "restart", "polkit.service"], check=False)

    # Only the fixture repository is visible to PackageKit during this check,
    # so no unrelated runner update is downloaded.
    SR29_APT_SAVED.mkdir(exist_ok=True)
    moved = []
    for path in [Path("/etc/apt/sources.list"), *Path("/etc/apt/sources.list.d").glob("*")]:
        if path.exists() and path != SR29_SOURCE:
            target = SR29_APT_SAVED / path.name
            shutil.move(str(path), str(target))
            moved.append((target, path))
    gnupg = work / "gnupg"
    gnupg.mkdir(mode=0o700)
    key = new_key(gnupg, "SR29 Station <sr29@invalid>")
    exported = sh(["gpg", "--homedir", str(gnupg), "--export", key]).stdout
    SR29_KEYRING.write_bytes(exported)
    SR29_KEYRING.chmod(0o644)
    SR29_SOURCE.write_text(
        f"Types: deb\nURIs: file:{SR29_REPO}\nSuites: ./\nSigned-By: {SR29_KEYRING}\n"
    )

    target = "rmac-archive-keyring"  # a Lulo OS package name: automatic set
    debs = {
        "v1": build_fixture_deb(work, target, "1.0"),
        "v2": build_fixture_deb(work, target, "2.0"),
        "v21": build_fixture_deb(work, target, "2.1"),
        "v22": build_fixture_deb(work, target, "2.2"),
        "v3": build_fixture_deb(work, target, "3.0", f"Conflicts: {SYNTHETIC_PACKAGE_TOKEN}\n"),
        "victim": build_fixture_deb(work, SYNTHETIC_PACKAGE_TOKEN, "1.0"),
    }
    cases: list[dict] = []
    try:
        sh(["dpkg", "-i", str(debs["v1"]), str(debs["victim"])])
        sh(["loginctl", "enable-linger", UPDATE_USER])
        for _ in range(50):
            if Path(f"/run/user/{account.pw_uid}/bus").exists():
                break
            time.sleep(0.2)
        sh(
            [
                "systemd-run", "--user", "-M", f"{UPDATE_USER}@", "--unit", "sr29-notify",
                "--collect", "/usr/bin/python3", str(capture_script),
                f"{account.pw_dir}/notifications.jsonl",
            ]
        )
        time.sleep(2)

        def schedule_safe(label: str, version_deb: str) -> dict:
            publish_repo(work, [debs[version_deb], debs["victim"]], gnupg, key)
            return run_update_unit(account, label)

        # 1. safe: the automatic set is simulated, downloaded and scheduled.
        safe = schedule_safe("safe", "v2")
        cases.append(safe)
        # 2. stale-prepared: the scheduled 2.0 is no longer offered (2.1 is).
        publish_repo(work, [debs["v21"], debs["victim"]], gnupg, key)
        stale = run_update_unit(account, "stale-prepared")
        cases.append(stale)
        # 3. cancellation failure: schedule 2.0 again, then deny the offline
        #    action and make the plan stale so the checker must cancel.
        rescheduled = schedule_safe("safe-again", "v2")
        cases.append(rescheduled)
        SR29_DENY_FLAG.touch()
        publish_repo(work, [debs["v22"], debs["victim"]], gnupg, key)
        cancel_failure = run_update_unit(account, "cancellation-failure")
        cases.append(cancel_failure)
        SR29_DENY_FLAG.unlink(missing_ok=True)
        sh(["python3", "-c", PK_CANCEL])
        # 4. destructive: 3.0 conflicts with an installed package.
        publish_repo(work, [debs["v3"], debs["victim"]], gnupg, key)
        destructive = run_update_unit(account, "destructive")
        cases.append(destructive)
        # 5. destructive while scheduled: schedule 2.0, then offer only the
        #    conflicting 3.0 next to it.
        scheduled_again = schedule_safe("safe-before-destructive", "v2")
        cases.append(scheduled_again)
        publish_repo(work, [debs["v2"], debs["v3"], debs["victim"]], gnupg, key)
        destructive_scheduled = run_update_unit(account, "destructive-while-scheduled")
        cases.append(destructive_scheduled)
        sh(["python3", "-c", PK_CANCEL])
        # 6. signature failure: the repository is re-signed by an unknown key.
        other = new_key(gnupg, "SR29 Untrusted <sr29-untrusted@invalid>")
        publish_repo(work, [debs["v2"], debs["victim"]], gnupg, other)
        untrusted = run_update_unit(account, "untrusted-signature")
        cases.append(untrusted)
    finally:
        sh(["python3", "-c", PK_CANCEL], check=False)
        SR29_DENY_FLAG.unlink(missing_ok=True)
        sh(["systemctl", "--user", "-M", f"{UPDATE_USER}@", "stop", "sr29-notify"],
           check=False)
        SR29_SOURCE.unlink(missing_ok=True)
        for saved, original in moved:
            shutil.move(str(saved), str(original))
        SR29_POLKIT_RULE.unlink(missing_ok=True)
        sh(["apt-get", "update"], check=False, timeout=900)

    def lines(case: dict) -> str:
        return "\n".join(case["checker_lines"])

    expected_safe_id = f"{target};2.0;all;"
    verdicts = {
        "safe_scheduled": safe["unit_start_exit"] == 0
        and safe["packagekit_after"]["action"] == "reboot"
        and any(p.startswith(expected_safe_id) for p in safe["packagekit_after"]["prepared"]),
        "stale_cancelled": stale["unit_start_exit"] != 0
        and "stale-prepared-plan" in lines(stale)
        and stale["packagekit_after"]["action"] == "unset",
        "cancellation_failure_warns": cancel_failure["unit_start_exit"] != 0
        and "cancel-" in lines(cancel_failure)
        and any(
            n.get("summary") == "Update needs review"
            for n in cancel_failure["notifications"]
        )
        and cancel_failure["packagekit_after"]["action"] == "reboot",
        "destructive_not_scheduled": destructive["packagekit_after"]["action"] == "unset",
        "destructive_scheduled_cancelled": destructive_scheduled["packagekit_after"]["action"]
        == "unset"
        and destructive_scheduled["unit_start_exit"] != 0,
        "untrusted_fails_closed": untrusted["unit_start_exit"] != 0
        and untrusted["packagekit_after"]["action"] == "unset",
    }
    observations = {
        "checker": "/usr/libexec/rmac/rmac-update-check via the installed "
        "rmac-update-check.service user unit",
        "packagekit_version": text(
            sh(["dpkg-query", "-W", "-f=${Version}", "packagekit"], check=False)
        ),
        "fixture_note": (
            "synthetic signed flat repository; the fixture uses the Lulo OS "
            "package name rmac-archive-keyring so it is in the automatic set; "
            "the polkit rule grants the synthetic user the PackageKit actions an "
            "active local user has, and denies the offline action on demand"
        ),
        "incomplete_plan_case": (
            "not reproducible with the real apt backend; covered by the fake "
            "PackageKit tests (scripts/test_update_check.py)"
        ),
        "verdicts": verdicts,
        "cases": cases,
    }
    failed = [name for name, ok in verdicts.items() if not ok]
    require(not failed, f"SR-29 native cases failed: {', '.join(failed)}")
    return observations


# --------------------------------------------------------------------------
# journal


def journal_cursor() -> str:
    output = text(sh(["journalctl", "--show-cursor", "-n", "0", "--no-pager"], check=False))
    match = re.search(r"-- cursor: (\S+)", output)
    return match.group(1) if match else ""


def journal_since(cursor: str, filters: list[str]) -> str:
    command = ["journalctl", "--no-pager", "-o", "cat"]
    if cursor:
        command += ["--after-cursor", cursor]
    return text(sh(command + filters, check=False))


def check_journal_redaction(state: dict) -> dict:
    raw = text(sh(["journalctl", "-b", "--no-pager", "-o", "json"], check=False, timeout=300))
    rmac_entries = []
    for line in raw.splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        identity = " ".join(
            str(entry.get(key, ""))
            for key in ("SYSLOG_IDENTIFIER", "_COMM", "_SYSTEMD_UNIT", "_SYSTEMD_USER_UNIT",
                        "_CMDLINE", "UNIT", "USER_UNIT")
        )
        if "rmac" in identity:
            message = entry.get("MESSAGE", "")
            if isinstance(message, list):  # binary message
                message = bytes(message).decode("utf-8", "replace")
            rmac_entries.append({"identity": identity[:160], "message": str(message)})
    planted = {
        "password": SYNTHETIC_PASSWORD,
        "private-path": SYNTHETIC_PATH_TOKEN,
        "package-name": SYNTHETIC_PACKAGE_TOKEN,
        "repository-path": str(SR29_REPO),
        "relay-request": "touch /tmp/sr-relay-pwned",
        "fixture-key-uid": "sr29@invalid",
    }
    leaks = {
        label: sum(value in entry["message"] for entry in rmac_entries)
        for label, value in planted.items()
    }
    control = [
        entry["message"][:120]
        for entry in rmac_entries
        if re.search(r"[\x00-\x08\x0b-\x1f\x7f]", entry["message"])
    ]
    unique_names = [
        entry["message"][:120]
        for entry in rmac_entries
        if re.search(r"(?<![\w.]):1\.[0-9]+\b", entry["message"])
    ]
    longest = max((len(entry["message"]) for entry in rmac_entries), default=0)
    observations = {
        "rmac_journal_entries": len(rmac_entries),
        "planted_value_occurrences": leaks,
        "control_character_lines": control[:10],
        "dbus_unique_name_lines": unique_names[:10],
        "longest_rmac_message": longest,
        "sample_messages": sorted({entry["message"][:160] for entry in rmac_entries})[:40],
        "scope_note": (
            "covers the components this headless station runs: package scripts, "
            "the keyboard helper and relay, polkit/pkexec, and the update checker; "
            "graphical-session components are covered by the source pass"
        ),
    }
    require(rmac_entries, "no rmac journal entries were produced to inspect")
    require(not any(leaks.values()), "a planted synthetic secret reached the journal")
    require(not control, "an rmac journal line contains control characters")
    require(not unique_names, "an rmac journal line names a D-Bus unique name")
    require(longest <= 2048, "an rmac journal line is unbounded")
    return observations


# --------------------------------------------------------------------------
# driver


SEQUENCE = (
    ("candidate-provenance", check_candidate_provenance),
    ("install-effects", check_install_effects),
    ("package-permissions", check_package_permissions),
    ("package-lifecycle", check_package_lifecycle),
    ("reinstall", None),
    ("systemd-hardening", check_systemd_hardening),
    ("keyboard-relay", check_keyboard_relay),
    ("polkit-policy", check_polkit_policy),
    ("lock-units", check_lock_units),
    ("untrusted-open", check_untrusted_open),
    ("terminal-wrapper", check_terminal_wrapper),
    ("sr29-packagekit", check_sr29_packagekit),
    ("journal-redaction", check_journal_redaction),
)


def runner_identity() -> dict:
    return {
        "station": STATION_ID,
        "image_os": os.environ.get("ImageOS"),
        "image_version": os.environ.get("ImageVersion"),
        "kernel": os.uname().release,
        "workflow_run": (
            f"{os.environ.get('GITHUB_SERVER_URL')}/{os.environ.get('GITHUB_REPOSITORY')}"
            f"/actions/runs/{os.environ.get('GITHUB_RUN_ID')}"
        ),
        "station_scripts_revision": os.environ.get("GITHUB_SHA"),
    }


def run(arguments: argparse.Namespace) -> int:
    evidence: Path = arguments.evidence
    preflight(evidence)
    state = {
        "candidate": arguments.candidate.resolve(),
        "candidate_source": arguments.candidate_source,
        "work": arguments.work.resolve(),
    }
    state["work"].mkdir(parents=True, exist_ok=True)
    results = []
    for name, function in SEQUENCE:
        if function is None:
            try:
                install_candidate(state)
            except Exception as error:  # the following checks report it
                print(f"reinstall failed: {error}", file=sys.stderr)
            continue
        started = time.time()
        print(f"== {name}", flush=True)
        try:
            observations = function(state)
            status = "pass"
            failure = None
        except StationError as error:
            observations = getattr(error, "observations", {})
            status = "fail"
            failure = bounded(str(error), 1500)
        except Exception as error:  # record and continue with the next check
            observations = {}
            status = "error"
            failure = bounded(
                "".join(traceback.format_exception_only(type(error), error))
                + traceback.format_exc()[-1500:],
                2500,
            )
        record = {
            "check": name,
            "review_checks": [
                {"domain": domain, "check": check} for domain, check in REVIEW_MAP[name]
            ],
            "status": status,
            "failure": failure,
            "seconds": round(time.time() - started, 1),
            "observations": observations,
        }
        print(f"   {status}" + (f": {failure.splitlines()[0]}" if failure else ""), flush=True)
        (evidence / f"{name}.json").write_text(
            json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        results.append(record)
    summary = {
        "format": FORMAT,
        "generated": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "runner": runner_identity(),
        "candidate": json.loads(Path(state["candidate_source"]).read_text(encoding="utf-8"))
        | {"version": state.get("version")},
        "results": [
            {
                "check": record["check"],
                "status": record["status"],
                "review_checks": record["review_checks"],
                "failure": record["failure"],
            }
            for record in results
        ],
    }
    (evidence / "station-evidence.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


def gate(arguments: argparse.Namespace) -> int:
    summary = json.loads(
        (arguments.evidence / "station-evidence.json").read_text(encoding="utf-8")
    )
    names = [name for name, function in SEQUENCE if function is not None]
    statuses = {record["check"]: record["status"] for record in summary["results"]}
    failed = [name for name in names if statuses.get(name) != "pass"]
    for name in names:
        print(f"{statuses.get(name, 'missing'):6} {name}")
    if failed:
        print(f"security station: {len(failed)} check(s) did not pass", file=sys.stderr)
        return 1
    print(f"security station: all {len(names)} checks passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    run_parser = commands.add_parser("run")
    run_parser.add_argument("--candidate", type=Path, required=True)
    run_parser.add_argument("--candidate-source", type=Path, required=True)
    run_parser.add_argument("--work", type=Path, required=True)
    run_parser.add_argument("--evidence", type=Path, required=True)
    gate_parser = commands.add_parser("gate")
    gate_parser.add_argument("--evidence", type=Path, required=True)
    arguments = parser.parse_args()
    if arguments.command == "run":
        return run(arguments)
    return gate(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
