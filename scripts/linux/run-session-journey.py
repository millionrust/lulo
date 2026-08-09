#!/usr/bin/env python3
"""Collect real GDM, safe-mode, TTY, and GNOME recovery evidence."""

from __future__ import annotations

import argparse
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
import time


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = REPO_ROOT / "packaging/native/session-journey.json"
MAX_INPUT_BYTES = 256 * 1024
MAX_TOOL_OUTPUT_BYTES = 1024 * 1024
STATE_NAME = ".session-journey-state.json"
REPORT_NAME = "session-journey.json"
NORMAL_UNITS = (
    "rmac-top-bar.service",
    "rmac-dock.service",
    "rmac-launcher.service",
    "rmac-app-drawer.service",
    "rmac-quick-settings.service",
    "rmac-notification-center.service",
    "rmac-notification-center-panel.service",
    "rmac-focus.service",
    "rmac-wallpaper.service",
    "rmac-osd.service",
    "rmac-shortcut-broker.service",
)
SAFE_UNITS = (
    "rmac-session-supervisor.service",
    "rmac-lock-coordinator.service",
    "rmac-idle-lock.service",
)
VALID_SETTINGS = b'{\n  "version": 4,\n  "settings": {}\n}\n'
REJECTED_SETTINGS = (
    b'{"version":4,"settings":{"wallpaper":{"default":'
    b'{"source":"/home/rmac-journey/private-secret.jpg"}}\n'
)


class JourneyError(RuntimeError):
    """A bounded, privacy-safe session-journey failure."""


def _load_script(name: str, filename: str):
    path = Path(__file__).with_name(filename)
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise JourneyError("session package verifier cannot be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def _regular_bytes(path: Path, maximum: int = MAX_INPUT_BYTES) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise JourneyError(f"required journey input is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise JourneyError(f"required journey input is not regular: {path.name}")
    if metadata.st_size > maximum:
        raise JourneyError(f"required journey input is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise JourneyError(f"required journey input cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise JourneyError(f"required journey input changed while reading: {path.name}")
    return raw


def expected_contract() -> dict[str, object]:
    return {
        "crash_unit": "rmac-dock.service",
        "disposable_marker": {
            "contents": "rmac-session-journey-v1\n",
            "path": "/run/rmac-session-journey-v1",
        },
        "evidence_directory": "/home/rmac-journey/rmac-session-evidence",
        "format": 1,
        "minimum_free_gib": 15,
        "phases": [
            "normal-crash-loop",
            "safe-login",
            "tty-restore",
            "recovered-rmac",
            "gnome-recovery",
        ],
        "platform": {"id": "ubuntu", "version_id": "26.04"},
        "test_user": "rmac-journey",
    }


def load_contract(path: Path = CONTRACT_PATH) -> dict[str, object]:
    try:
        document = json.loads(_regular_bytes(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("session journey contract is invalid") from error
    if document != expected_contract():
        raise JourneyError("session journey contract differs from the reviewed policy")
    return document


def _load_os_release(path: Path = Path("/etc/os-release")) -> dict[str, str]:
    try:
        lines = _regular_bytes(path, 64 * 1024).decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise JourneyError("operating-system identity is invalid") from error
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
        raise JourneyError(f"required session-journey tool is unavailable: {name}")
    return candidate


def _require_executable(path: Path, label: str) -> str:
    try:
        metadata = path.stat()
    except OSError as error:
        raise JourneyError(f"required session-journey tool is unavailable: {label}") from error
    if not stat.S_ISREG(metadata.st_mode) or not os.access(path, os.X_OK):
        raise JourneyError(f"required session-journey tool is unavailable: {label}")
    return str(path)


def _run(
    command: list[str],
    *,
    tools: dict[str, str],
    accepted: tuple[int, ...] = (0,),
    timeout: int = 30,
) -> subprocess.CompletedProcess[bytes]:
    resolved = [tools.get(command[0], command[0]), *command[1:]]
    environment = {
        **os.environ,
        "LC_ALL": "C",
        "SYSTEMD_COLORS": "0",
        "SYSTEMD_PAGER": "cat",
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
        raise JourneyError(f"session-journey command could not run: {command[0]}") from error
    if (
        len(result.stdout) > MAX_TOOL_OUTPUT_BYTES
        or len(result.stderr) > MAX_TOOL_OUTPUT_BYTES
    ):
        raise JourneyError(
            f"session-journey command produced excessive output: {command[0]}"
        )
    if result.returncode not in accepted:
        raise JourneyError(f"session-journey command failed: {command[0]}")
    return result


def _atomic_private(path: Path, contents: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.parent.chmod(0o700)
    temporary = path.with_name(f".{path.name}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as destination:
            destination.write(contents)
            destination.flush()
            os.fsync(destination.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def _write_json(path: Path, document: dict[str, object], *, private: bool) -> None:
    raw = (json.dumps(document, indent=2, sort_keys=True) + "\n").encode("utf-8")
    _atomic_private(path, raw)
    path.chmod(0o600 if private else 0o644)


def _read_state(path: Path) -> dict[str, object]:
    try:
        document = json.loads(_regular_bytes(path))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("private session-journey state is invalid") from error
    if (
        not isinstance(document, dict)
        or document.get("format") != 1
        or not isinstance(document.get("completed"), list)
        or not isinstance(document.get("sessions"), dict)
    ):
        raise JourneyError("private session-journey state is invalid")
    return document


def _session_property(
    session_id: str, property_name: str, tools: dict[str, str]
) -> str:
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", session_id):
        raise JourneyError("logind session identity is invalid")
    result = _run(
        [
            "loginctl",
            "show-session",
            session_id,
            f"--property={property_name}",
            "--value",
            "--no-pager",
        ],
        tools=tools,
    )
    try:
        return result.stdout.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise JourneyError("logind session property is invalid") from error


def _current_session(
    contract: dict[str, object], tools: dict[str, str], expected_type: str
) -> str:
    session_id = os.environ.get("XDG_SESSION_ID", "")
    expected = {
        "Name": contract["test_user"],
        "Type": expected_type,
        "Class": "user",
        "Remote": "no",
    }
    for property_name, expected_value in expected.items():
        if _session_property(session_id, property_name, tools) != expected_value:
            raise JourneyError(f"logind {expected_type} session identity is invalid")
    service = _session_property(session_id, "Service", tools)
    if expected_type == "wayland":
        if not service.startswith("gdm"):
            raise JourneyError("graphical phase is not a real GDM login")
    elif service != "login":
        raise JourneyError("TTY phase is not a real local text login")
    return session_id


def _unit_state(unit: str, tools: dict[str, str]) -> str:
    result = _run(
        [
            "systemctl",
            "--user",
            "show",
            unit,
            "--property=ActiveState",
            "--value",
            "--no-pager",
        ],
        tools=tools,
    )
    try:
        state = result.stdout.decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise JourneyError("systemd unit state is invalid") from error
    if state not in {"active", "inactive", "failed", "activating", "deactivating"}:
        raise JourneyError("systemd unit state is invalid")
    return state


def _unit_restarts(unit: str, tools: dict[str, str]) -> int:
    result = _run(
        [
            "systemctl",
            "--user",
            "show",
            unit,
            "--property=NRestarts",
            "--value",
            "--no-pager",
        ],
        tools=tools,
    )
    try:
        value = int(result.stdout.decode("ascii").strip())
    except (UnicodeDecodeError, ValueError) as error:
        raise JourneyError("systemd restart counter is invalid") from error
    if not 0 <= value <= 1_000_000:
        raise JourneyError("systemd restart counter is invalid")
    return value


def _require_units(
    active: tuple[str, ...], inactive: tuple[str, ...], tools: dict[str, str]
) -> None:
    for unit in active:
        if _unit_state(unit, tools) != "active":
            raise JourneyError(f"required rmac unit is not active: {unit}")
    for unit in inactive:
        if _unit_state(unit, tools) == "active":
            raise JourneyError(f"forbidden rmac unit remains active: {unit}")


def _diagnostics(tools: dict[str, str]) -> dict[str, object]:
    result = _run(
        ["rmac-session-supervisor", "diagnostics"],
        tools=tools,
    )
    try:
        document = json.loads(result.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("privacy-safe diagnostics are invalid") from error
    if not isinstance(document, dict) or set(document) != {
        "components",
        "format",
        "observed_at_unix_ms",
        "safe_mode",
        "safe_mode_trigger_unit",
        "shell_settings_recovery",
    }:
        raise JourneyError("privacy-safe diagnostics fields are not exact")
    if document.get("format") != 1 or not isinstance(document.get("components"), list):
        raise JourneyError("privacy-safe diagnostics identity is invalid")
    component_fields = {
        "available",
        "healthy",
        "restart_budget_exhausted",
        "restarts",
        "unit",
    }
    if (
        len(document["components"]) != len(NORMAL_UNITS)
        or {
            component.get("unit")
            for component in document["components"]
            if isinstance(component, dict)
            and set(component) == component_fields
        }
        != set(NORMAL_UNITS)
    ):
        raise JourneyError("privacy-safe component diagnostics are not exact")
    raw = result.stdout.decode("utf-8", "replace")
    for forbidden in (
        "main_pid",
        "/home/",
        "private-secret",
        "environment",
        "journal",
        "token",
    ):
        if forbidden in raw:
            raise JourneyError("privacy-safe diagnostics disclosed forbidden content")
    return document


def _portal_roundtrip(tools: dict[str, str], *, expect_rmac_backend: bool) -> None:
    backend_unit = "rmac-notification-center.service"
    expected_state = "active" if expect_rmac_backend else "inactive"
    if _unit_state(backend_unit, tools) != expected_state:
        raise JourneyError("notification portal backend state is incorrect")
    notification_id = "org.rmac.SessionJourney"
    payload = (
        "{'title': <'rmac session journey'>, "
        "'body': <'synthetic portal routing evidence'>}"
    )
    added = _run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Notification.AddNotification",
            notification_id,
            payload,
        ],
        tools=tools,
    )
    removed = _run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Notification.RemoveNotification",
            notification_id,
        ],
        tools=tools,
    )
    if added.stdout.strip() != b"()" or removed.stdout.strip() != b"()":
        raise JourneyError("notification portal round trip returned an invalid result")
    if _unit_state(backend_unit, tools) != expected_state:
        raise JourneyError("notification portal selected the wrong desktop backend")


def _check_disk(contract: dict[str, object]) -> None:
    minimum = int(contract["minimum_free_gib"]) * 1024**3
    if shutil.disk_usage("/").free < minimum:
        raise JourneyError("session journey stopped below the 15 GiB floor")


def _preflight(contract: dict[str, object], tools: dict[str, str]) -> tuple[Path, Path]:
    if os.geteuid() == 0:
        raise JourneyError("session journey must run as the disposable desktop user")
    account = pwd.getpwuid(os.geteuid())
    if (
        account.pw_name != contract["test_user"]
        or Path(account.pw_dir) != Path("/home") / contract["test_user"]
    ):
        raise JourneyError("session journey requires the fixed disposable account")
    marker = contract["disposable_marker"]
    marker_path = Path(marker["path"])
    marker_metadata = marker_path.lstat()
    if (
        marker_path.is_symlink()
        or not stat.S_ISREG(marker_metadata.st_mode)
        or marker_metadata.st_uid != 0
        or stat.S_IMODE(marker_metadata.st_mode) != 0o644
        or _regular_bytes(marker_path, 128) != marker["contents"].encode("ascii")
    ):
        raise JourneyError("root-owned disposable-session marker is invalid")
    identity = _load_os_release()
    if (
        identity.get("ID") != contract["platform"]["id"]
        or identity.get("VERSION_ID") != contract["platform"]["version_id"]
    ):
        raise JourneyError("session journey requires the reviewed Ubuntu release")
    evidence = Path(contract["evidence_directory"])
    if evidence != Path(account.pw_dir) / "rmac-session-evidence":
        raise JourneyError("session journey evidence path is invalid")
    if evidence.exists():
        metadata = evidence.lstat()
        if (
            evidence.is_symlink()
            or not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700
        ):
            raise JourneyError("session journey evidence directory is unsafe")
    _check_disk(contract)
    settings = Path(account.pw_dir) / ".config/rmac/shell.json"
    return evidence, settings


def _require_graphical_environment(kind: str) -> None:
    if os.environ.get("XDG_SESSION_TYPE") != "wayland" or not os.environ.get(
        "WAYLAND_DISPLAY"
    ):
        raise JourneyError("graphical phase lacks a real Wayland environment")
    current = {
        value.lower()
        for value in os.environ.get("XDG_CURRENT_DESKTOP", "").split(":")
        if value
    }
    desktop = os.environ.get("XDG_SESSION_DESKTOP", "").lower()
    if kind == "rmac":
        if "rmac" not in current or desktop != "rmac":
            raise JourneyError("normal phase is not the rmac GDM session")
    elif kind == "safe":
        if "rmac" in current or desktop != "niri":
            raise JourneyError("safe phase did not preserve niri portal identity")
    elif kind == "gnome":
        if "rmac" in current or not ({"gnome", "ubuntu"} & current):
            raise JourneyError("recovery phase is not the stock GNOME session")


def _verify_installed_session() -> None:
    verifier = _load_script("rmac_journey_session_package", "verify-session-package.py")
    try:
        verifier.verify_installed_host(Path("/"))
    except Exception as error:
        raise JourneyError("installed rmac/GNOME session contract failed") from error


def _wait_for(predicate, detail: str, timeout: float = 20.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise JourneyError(detail)


def _inject_crash_loop(
    unit: str, marker: Path, tools: dict[str, str]
) -> None:
    initial = _unit_restarts(unit, tools)
    for expected in range(initial + 1, initial + 7):
        if marker.exists():
            break
        if _unit_state(unit, tools) != "active":
            _run(
                ["systemctl", "--user", "start", unit],
                tools=tools,
                accepted=(0, 1),
            )
        if marker.exists():
            break
        _wait_for(
            lambda: marker.exists() or _unit_state(unit, tools) == "active",
            "crash-injection unit did not become active",
            5.0,
        )
        if marker.exists():
            break
        _run(
            [
                "systemctl",
                "--user",
                "kill",
                "--kill-whom=main",
                "--signal=KILL",
                unit,
            ],
            tools=tools,
        )
        _wait_for(
            lambda: marker.exists() or _unit_restarts(unit, tools) >= expected,
            "component did not consume its restart budget",
            5.0,
        )
    _wait_for(
        lambda: marker.exists()
        and _unit_state("rmac-safe-mode.target", tools) == "active",
        "real component crash loop did not enter safe mode",
    )


def _new_state(session_id: str) -> dict[str, object]:
    return {
        "completed": [],
        "format": 1,
        "sessions": {"normal-crash-loop": session_id},
    }


def _require_order(
    state: dict[str, object], contract: dict[str, object], phase: str
) -> None:
    completed = state["completed"]
    expected_index = len(completed)
    phases = contract["phases"]
    if expected_index >= len(phases) or phases[expected_index] != phase:
        raise JourneyError("session journey phase is out of order")


def _complete(
    state: dict[str, object],
    contract: dict[str, object],
    phase: str,
    session_id: str,
    state_path: Path,
) -> None:
    _require_order(state, contract, phase)
    sessions = state["sessions"]
    if phase not in sessions:
        if session_id in sessions.values():
            raise JourneyError("phase did not cross a real login boundary")
        sessions[phase] = session_id
    state["completed"].append(phase)
    _write_json(state_path, state, private=True)


def _normal_phase(
    contract: dict[str, object],
    evidence: Path,
    settings: Path,
    tools: dict[str, str],
) -> None:
    if evidence.exists():
        raise JourneyError("normal phase requires a fresh evidence directory")
    session_id = _current_session(contract, tools, "wayland")
    _require_graphical_environment("rmac")
    _verify_installed_session()
    _require_units(
        NORMAL_UNITS + SAFE_UNITS + ("rmac-session.target",),
        ("rmac-safe-mode.target",),
        tools,
    )
    marker = settings.parents[2] / ".local/state/rmac/session/safe-mode.json"
    backup = settings.with_name("shell.json.last-good")
    rejected = settings.with_name("shell.json.rejected-before-restore")
    if any(path.exists() for path in (settings, backup, rejected, marker)):
        raise JourneyError("normal phase requires fresh synthetic recovery state")
    _atomic_private(settings, VALID_SETTINGS)
    _atomic_private(backup, VALID_SETTINGS)
    _atomic_private(settings, REJECTED_SETTINGS)
    diagnostics = _diagnostics(tools)
    if diagnostics.get("shell_settings_recovery") != "last-good-available":
        raise JourneyError("broken settings did not expose last-good recovery")
    _portal_roundtrip(tools, expect_rmac_backend=True)

    evidence.mkdir(mode=0o700)
    state_path = evidence / STATE_NAME
    state = _new_state(session_id)
    _inject_crash_loop(contract["crash_unit"], marker, tools)
    diagnostics = _diagnostics(tools)
    if (
        diagnostics.get("safe_mode") is not True
        or diagnostics.get("safe_mode_trigger_unit") != contract["crash_unit"]
    ):
        raise JourneyError("diagnostics did not prove the real crash-loop trigger")
    _complete(
        state,
        contract,
        "normal-crash-loop",
        session_id,
        state_path,
    )


def _safe_phase(
    contract: dict[str, object],
    evidence: Path,
    tools: dict[str, str],
) -> None:
    session_id = _current_session(contract, tools, "wayland")
    _require_graphical_environment("safe")
    state_path = evidence / STATE_NAME
    state = _read_state(state_path)
    _require_order(state, contract, "safe-login")
    _require_units(
        SAFE_UNITS + ("rmac-safe-mode.target",),
        NORMAL_UNITS + ("rmac-session.target",),
        tools,
    )
    diagnostics = _diagnostics(tools)
    if (
        diagnostics.get("safe_mode") is not True
        or diagnostics.get("shell_settings_recovery") != "last-good-available"
    ):
        raise JourneyError("safe login did not retain diagnostics and recovery")
    _portal_roundtrip(tools, expect_rmac_backend=False)
    _complete(state, contract, "safe-login", session_id, state_path)


def _tty_phase(
    contract: dict[str, object],
    evidence: Path,
    settings: Path,
    tools: dict[str, str],
) -> None:
    session_id = _current_session(contract, tools, "tty")
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        raise JourneyError("TTY phase unexpectedly inherited a graphical display")
    state_path = evidence / STATE_NAME
    state = _read_state(state_path)
    _require_order(state, contract, "tty-restore")
    safe_session = state["sessions"].get("safe-login")
    if (
        not isinstance(safe_session, str)
        or _session_property(safe_session, "Type", tools) != "wayland"
        or _session_property(safe_session, "State", tools) not in {"active", "online"}
    ):
        raise JourneyError("TTY restore did not retain the safe graphical session")
    diagnostics = _diagnostics(tools)
    if diagnostics.get("shell_settings_recovery") != "last-good-available":
        raise JourneyError("TTY diagnostics did not expose settings recovery")
    _run(["rmac-session-supervisor", "restore-last-good-settings"], tools=tools)
    rejected = settings.with_name("shell.json.rejected-before-restore")
    try:
        restored = json.loads(_regular_bytes(settings))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JourneyError("TTY recovery produced invalid shell settings") from error
    if (
        not isinstance(restored, dict)
        or restored.get("version") != 4
        or not isinstance(restored.get("settings"), dict)
        or _regular_bytes(rejected) != REJECTED_SETTINGS
    ):
        raise JourneyError("TTY recovery did not restore and preserve exact settings")
    _run(["rmac-session-supervisor", "clear-safe-mode"], tools=tools)
    marker = settings.parents[2] / ".local/state/rmac/session/safe-mode.json"
    if marker.exists():
        raise JourneyError("TTY recovery did not clear safe mode")
    diagnostics = _diagnostics(tools)
    if (
        diagnostics.get("safe_mode") is not False
        or diagnostics.get("shell_settings_recovery") != "current"
    ):
        raise JourneyError("TTY recovery did not produce healthy current settings")
    _complete(state, contract, "tty-restore", session_id, state_path)


def _recovered_phase(
    contract: dict[str, object],
    evidence: Path,
    tools: dict[str, str],
) -> None:
    session_id = _current_session(contract, tools, "wayland")
    _require_graphical_environment("rmac")
    _verify_installed_session()
    state_path = evidence / STATE_NAME
    state = _read_state(state_path)
    _require_order(state, contract, "recovered-rmac")
    _require_units(
        NORMAL_UNITS + SAFE_UNITS + ("rmac-session.target",),
        ("rmac-safe-mode.target",),
        tools,
    )
    diagnostics = _diagnostics(tools)
    if (
        diagnostics.get("safe_mode") is not False
        or diagnostics.get("shell_settings_recovery") != "current"
    ):
        raise JourneyError("recovered rmac login is not healthy")
    _portal_roundtrip(tools, expect_rmac_backend=True)
    _complete(state, contract, "recovered-rmac", session_id, state_path)


def _gnome_phase(
    contract: dict[str, object],
    evidence: Path,
    tools: dict[str, str],
) -> None:
    session_id = _current_session(contract, tools, "wayland")
    _require_graphical_environment("gnome")
    state_path = evidence / STATE_NAME
    state = _read_state(state_path)
    _require_order(state, contract, "gnome-recovery")
    _require_units(
        (),
        NORMAL_UNITS
        + SAFE_UNITS
        + ("rmac-session.target", "rmac-safe-mode.target"),
        tools,
    )
    _portal_roundtrip(tools, expect_rmac_backend=False)
    _complete(state, contract, "gnome-recovery", session_id, state_path)
    if state["completed"] != contract["phases"]:
        raise JourneyError("session journey did not complete every reviewed phase")
    report = {
        "checks": {
            "config_restored": True,
            "diagnostics_redacted": True,
            "gdm_login_logout": True,
            "gnome_recovery": True,
            "notification_portal_roundtrip": True,
            "real_crash_loop": True,
            "safe_mode_login": True,
            "tty_restore": True,
        },
        "format": 1,
        "phases": [{"id": phase, "passed": True} for phase in contract["phases"]],
        "platform": contract["platform"],
    }
    _write_json(evidence / REPORT_NAME, report, private=False)
    state_path.unlink()


def run_phase(contract: dict[str, object], phase: str) -> None:
    tools = {
        "gdbus": _require_tool("gdbus"),
        "loginctl": _require_tool("loginctl"),
        "rmac-session-supervisor": _require_executable(
            Path("/usr/libexec/rmac/rmac-session-supervisor"),
            "rmac-session-supervisor",
        ),
        "systemctl": _require_tool("systemctl"),
    }
    evidence, settings = _preflight(contract, tools)
    handlers = {
        "normal-crash-loop": lambda: _normal_phase(
            contract, evidence, settings, tools
        ),
        "safe-login": lambda: _safe_phase(contract, evidence, tools),
        "tty-restore": lambda: _tty_phase(
            contract, evidence, settings, tools
        ),
        "recovered-rmac": lambda: _recovered_phase(contract, evidence, tools),
        "gnome-recovery": lambda: _gnome_phase(contract, evidence, tools),
    }
    try:
        handler = handlers[phase]
    except KeyError as error:
        raise JourneyError("unrecognized session-journey phase") from error
    handler()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-contract", action="store_true")
    parser.add_argument("--phase", choices=tuple(expected_contract()["phases"]))
    arguments = parser.parse_args()
    try:
        contract = load_contract()
        if arguments.check_contract:
            if arguments.phase is not None:
                raise JourneyError("--check-contract does not accept a phase")
            print("rmac session journey contract verified")
            return 0
        if arguments.phase is None:
            raise JourneyError("--phase is required")
        run_phase(contract, arguments.phase)
    except (JourneyError, KeyError, OSError) as error:
        parser.exit(4, f"run-session-journey: {error}\n")
    if arguments.phase == "gnome-recovery":
        print("rmac session journey passed; redacted evidence is complete")
    else:
        phases = contract["phases"]
        next_phase = phases[phases.index(arguments.phase) + 1]
        print(f"session phase passed; complete the real login transition: {next_phase}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
