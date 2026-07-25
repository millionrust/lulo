#!/usr/bin/env python3
"""Fail-closed preflight for reproducible Linux reference-PC gates."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
from typing import Mapping, Sequence


GIB = 1024**3
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
    if not snapshot.vulkan_succeeded:
        failures.append("vulkaninfo --summary did not complete")
    elif any(marker in vulkan for marker in SOFTWARE_VULKAN_MARKERS):
        failures.append("a software Vulkan renderer is not reference-PC evidence")
    elif HARDWARE_VULKAN_DEVICE.search(snapshot.vulkan_summary) is None:
        failures.append("an integrated or discrete Vulkan GPU was not proven")

    if not snapshot.worktree_clean:
        failures.append("tracked worktree changes make the evidence non-reproducible")
    return failures


def command_succeeded(command: Sequence[str], repo_root: Path) -> bool:
    try:
        return (
            subprocess.run(
                command,
                cwd=repo_root,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=20,
                check=False,
            ).returncode
            == 0
        )
    except (OSError, subprocess.TimeoutExpired):
        return False


def vulkan_summary(repo_root: Path) -> tuple[bool, str]:
    try:
        result = subprocess.run(
            ["vulkaninfo", "--summary"],
            cwd=repo_root,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, ""
    return result.returncode == 0, f"{result.stdout}\n{result.stderr}"


def capture_host(repo_root: Path, required_commands: Sequence[str]) -> HostSnapshot:
    commands = {command: shutil.which(command) is not None for command in required_commands}
    vulkan_succeeded, summary = vulkan_summary(repo_root)
    clean = command_succeeded(
        ["git", "diff", "--quiet", "--ignore-submodules", "--"], repo_root
    ) and command_succeeded(
        ["git", "diff", "--cached", "--quiet", "--ignore-submodules", "--"],
        repo_root,
    )
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
    snapshot = capture_host(repo_root, required_commands)
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
