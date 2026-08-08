#!/usr/bin/env python3
"""Fail-closed preflight for reproducible Linux reference-PC gates."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
from typing import Mapping, Optional, Sequence


GIB = 1024**3
MAX_AUTHORITY_OUTPUT_BYTES = 1024 * 1024
HARDWARE_VULKAN_DEVICE = re.compile(
    r"PHYSICAL_DEVICE_TYPE_(?:INTEGRATED|DISCRETE)_GPU", re.IGNORECASE
)
SOFTWARE_VULKAN_MARKERS = (
    "llvmpipe",
    "lavapipe",
    "software rasterizer",
    "swiftshader",
    "physical_device_type_cpu",
)


@dataclass(frozen=True)
class HostSnapshot:
    kernel: str
    effective_uid: int
    session_type: str
    current_desktop: str
    wayland_display: str
    free_bytes: int
    commands: Mapping[str, bool]
    vulkan_succeeded: bool
    vulkan_summary: str
    worktree_clean: bool
    portal_service_active: bool
    portal_bus_owned: bool
    manager_environment_matches: bool
    niri_ipc_succeeded: bool
    niri_enabled_outputs: int


def desktop_tokens(value: str) -> set[str]:
    return {
        token.strip().lower()
        for token in re.split(r"[:;,]", value)
        if token.strip()
    }


def evaluate_host(
    snapshot: HostSnapshot,
    *,
    expected_desktop: str,
    minimum_free_bytes: int,
    required_commands: Sequence[str],
) -> list[str]:
    """Return privacy-safe failure reasons; an empty list is a passing host."""

    failures: list[str] = []
    if snapshot.kernel != "Linux":
        failures.append("Linux is required")
    if snapshot.effective_uid == 0:
        failures.append("run as the graphical test user, not root")
    if snapshot.session_type.lower() != "wayland":
        failures.append("XDG_SESSION_TYPE must be wayland")
    if not snapshot.wayland_display:
        failures.append("WAYLAND_DISPLAY is unavailable")

    desktops = desktop_tokens(snapshot.current_desktop)
    if expected_desktop == "gnome":
        if "gnome" not in desktops or "niri" in desktops:
            failures.append("the untouched GNOME Wayland session is required")
    elif expected_desktop == "niri" and "niri" not in desktops:
        failures.append("the niri Wayland session is required")

    if snapshot.free_bytes < minimum_free_bytes:
        failures.append(
            f"at least {minimum_free_bytes // GIB} GiB free is required before builds"
        )

    missing = sorted(
        command for command in required_commands if not snapshot.commands.get(command, False)
    )
    if missing:
        failures.append(f"required commands are missing: {', '.join(missing)}")

    vulkan = snapshot.vulkan_summary.lower()
    hardware_vulkan_present = HARDWARE_VULKAN_DEVICE.search(
        snapshot.vulkan_summary
    ) is not None
    if not snapshot.vulkan_succeeded:
        failures.append("vulkaninfo --summary did not complete")
    elif hardware_vulkan_present:
        pass
    elif any(marker in vulkan for marker in SOFTWARE_VULKAN_MARKERS):
        failures.append("a software Vulkan renderer is not reference-PC evidence")
    else:
        failures.append("an integrated or discrete Vulkan GPU was not proven")

    if not snapshot.worktree_clean:
        failures.append("tracked worktree changes make the evidence non-reproducible")
    if not snapshot.portal_service_active:
        failures.append("xdg-desktop-portal.service is not active")
    if not snapshot.portal_bus_owned:
        failures.append("the desktop portal frontend does not own its session-bus name")

    if expected_desktop == "niri":
        if not snapshot.manager_environment_matches:
            failures.append("the user-manager graphical routing environment is stale")
        if not snapshot.niri_ipc_succeeded:
            failures.append("niri IPC did not return a valid bounded output snapshot")
        elif snapshot.niri_enabled_outputs == 0:
            failures.append("niri reports no enabled output")
    return failures


def command_output(command: Sequence[str], repo_root: Path) -> tuple[bool, str]:
    try:
        result = subprocess.run(
            command,
            cwd=repo_root,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, ""
    output_bytes = len(result.stdout.encode("utf-8")) + len(
        result.stderr.encode("utf-8")
    )
    if output_bytes > MAX_AUTHORITY_OUTPUT_BYTES:
        return False, ""
    return result.returncode == 0, result.stdout


def command_succeeded(command: Sequence[str], repo_root: Path) -> bool:
    succeeded, _ = command_output(command, repo_root)
    return succeeded


def vulkan_summary(repo_root: Path) -> tuple[bool, str]:
    return command_output(["vulkaninfo", "--summary"], repo_root)


def manager_environment_matches(output: str, environment: Mapping[str, str]) -> bool:
    required = (
        "DBUS_SESSION_BUS_ADDRESS",
        "NIRI_SOCKET",
        "WAYLAND_DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_ID",
        "XDG_SESSION_TYPE",
    )
    observed: dict[str, str] = {}
    for line in output.splitlines():
        key, separator, value = line.partition("=")
        if separator and key in required:
            observed[key] = value
    return all(
        environment.get(key, "") and observed.get(key) == environment[key]
        for key in required
    )


def niri_enabled_output_count(output: str) -> Optional[int]:
    try:
        document = json.loads(output)
    except (json.JSONDecodeError, TypeError):
        return None
    if not isinstance(document, dict):
        return None
    enabled = 0
    for value in document.values():
        if not isinstance(value, dict):
            return None
        logical = value.get("logical")
        if logical is None:
            continue
        if (
            not isinstance(logical, dict)
            or not isinstance(logical.get("width"), int)
            or not isinstance(logical.get("height"), int)
            or logical["width"] <= 0
            or logical["height"] <= 0
        ):
            return None
        enabled += 1
    return enabled


def capture_host(
    repo_root: Path,
    required_commands: Sequence[str],
    expected_desktop: str,
) -> HostSnapshot:
    commands = {command: shutil.which(command) is not None for command in required_commands}
    vulkan_succeeded, summary = vulkan_summary(repo_root)
    clean = command_succeeded(
        ["git", "diff", "--quiet", "--ignore-submodules", "--"], repo_root
    ) and command_succeeded(
        ["git", "diff", "--cached", "--quiet", "--ignore-submodules", "--"],
        repo_root,
    )
    portal_service_active = command_succeeded(
        [
            "systemctl",
            "--user",
            "is-active",
            "--quiet",
            "xdg-desktop-portal.service",
        ],
        repo_root,
    )
    portal_bus_owned = command_succeeded(
        ["busctl", "--user", "status", "org.freedesktop.portal.Desktop"],
        repo_root,
    )
    manager_matches = False
    niri_ipc_succeeded = False
    enabled_outputs = 0
    if expected_desktop == "niri":
        manager_succeeded, manager_output = command_output(
            ["systemctl", "--user", "show-environment"], repo_root
        )
        manager_matches = manager_succeeded and manager_environment_matches(
            manager_output, os.environ
        )
        niri_succeeded, niri_output = command_output(
            ["niri", "msg", "--json", "outputs"], repo_root
        )
        parsed_outputs = niri_enabled_output_count(niri_output) if niri_succeeded else None
        niri_ipc_succeeded = parsed_outputs is not None
        enabled_outputs = parsed_outputs or 0
    return HostSnapshot(
        kernel=platform.system(),
        effective_uid=os.geteuid(),
        session_type=os.environ.get("XDG_SESSION_TYPE", ""),
        current_desktop=os.environ.get("XDG_CURRENT_DESKTOP", ""),
        wayland_display=os.environ.get("WAYLAND_DISPLAY", ""),
        free_bytes=shutil.disk_usage(repo_root).free,
        commands=commands,
        vulkan_succeeded=vulkan_succeeded,
        vulkan_summary=summary,
        worktree_clean=clean,
        portal_service_active=portal_service_active,
        portal_bus_owned=portal_bus_owned,
        manager_environment_matches=manager_matches,
        niri_ipc_succeeded=niri_ipc_succeeded,
        niri_enabled_outputs=enabled_outputs,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument(
        "--expected-desktop",
        choices=("gnome", "niri", "any"),
        default="gnome",
    )
    parser.add_argument("--minimum-free-gib", type=int, default=25)
    parser.add_argument(
        "--require-command",
        action="append",
        default=[],
        dest="required_commands",
    )
    args = parser.parse_args()
    if args.minimum_free_gib < 15:
        parser.error("--minimum-free-gib must preserve at least the 15 GiB floor")
    return args


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    if not repo_root.is_dir():
        print("reference preflight failed: repository root is unavailable", file=sys.stderr)
        return 2

    required_commands = tuple(dict.fromkeys(args.required_commands))
    snapshot = capture_host(repo_root, required_commands, args.expected_desktop)
    failures = evaluate_host(
        snapshot,
        expected_desktop=args.expected_desktop,
        minimum_free_bytes=args.minimum_free_gib * GIB,
        required_commands=required_commands,
    )
    if failures:
        for failure in failures:
            print(f"reference preflight failed: {failure}", file=sys.stderr)
        return 3

    print("reference_preflight=pass")
    print(f"desktop_gate={args.expected_desktop}")
    print("wayland_session=pass")
    print("hardware_vulkan=pass")
    print(f"free_gib={snapshot.free_bytes // GIB}")
    print("tracked_worktree=clean")
    print("required_commands=pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
