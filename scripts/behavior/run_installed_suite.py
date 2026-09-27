#!/usr/bin/env python3
"""Run the private UI and startup checks against an installed Lulo build.

Usage on the Lulo host:
    python3 scripts/behavior/run_installed_suite.py \
      --bin-dir /usr/bin --shell-bin-dir /usr/libexec/rmac \
      --output /tmp/lulo-installed-report

Coverage is 27 recorded Mac behavior scenarios, startup-only readiness for 9
first-party apps, a private Terminal typed-command roundtrip, and nested
power-dialog and shutdown flows. Screenshots are
captured for review, but no automatic pixel score is calculated. The suite
does not exercise real hardware, live login/logout, or real power state.

Each child suite owns a private HOME, XDG runtime, D-Bus session and nested
headless compositor. The commands run sequentially and never use the active
desktop session or real system power commands.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]


def binary_inventory(directories: list[Path]) -> list[dict[str, str]]:
    names = {
        "rmac-files", "rmac-text-editor", "rmac-system-settings", "rmac-calculator",
        "rmac-preview", "rmac-notes", "rmac-system-monitor", "rmac-terminal",
        "rmac-archive-utility", "rmac-app-drawer", "rmac-clock", "rmac-player",
        "rmac-weather", "rmac-wallpaper", "rmac-top-bar", "rmac-dock",
        "rmac-shortcut-dispatch", "rmac-file-chooser",
    }
    inventory = []
    for directory in directories:
        for name in sorted(names):
            path = directory / name
            if not path.is_file() or not os.access(path, os.X_OK):
                continue
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            owner = subprocess.run(
                ["dpkg-query", "-S", str(path)], text=True, capture_output=True, check=False
            )
            inventory.append({
                "path": str(path),
                "sha256": digest,
                "package_owner": owner.stdout.strip() if owner.returncode == 0 else "unowned",
            })
    return inventory


def run_step(name: str, argv: list[str], log_dir: Path) -> dict[str, object]:
    started = time.monotonic()
    result = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, check=False)
    elapsed = round(time.monotonic() - started, 2)
    (log_dir / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
    (log_dir / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
    return {
        "name": name,
        "exit_code": result.returncode,
        "elapsed_seconds": elapsed,
        "stdout_log": f"logs/{name}.stdout.txt",
        "stderr_log": f"logs/{name}.stderr.txt",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", action="append", required=True, help="installed app binary directory")
    parser.add_argument("--shell-bin-dir", action="append", required=True, help="installed shell binary directory")
    parser.add_argument("--output", type=Path, required=True, help="report directory (created if needed)")
    parser.add_argument("--only", choices=("behavior-27", "startup-smoke", "terminal-roundtrip", "power-dialogs", "shutdown-completion"),
                        help="run one phase and merge it into an existing report when binary hashes match")
    parser.add_argument("--skip-behavior", action="store_true", help="reuse behavior.json already in --output")
    parser.add_argument("--skip-startup", action="store_true")
    parser.add_argument("--skip-shell", action="store_true", help="skip power dialogs and shutdown private suites")
    args = parser.parse_args()

    if sys.platform != "linux":
        parser.error("the installed suite must run on the Linux host")
    if os.environ.get("WAYLAND_DISPLAY") == "wayland-1" or os.environ.get("XDG_RUNTIME_DIR", "").startswith("/run/user/"):
        parser.error("run from a terminal outside the live graphical session; nested suites enforce isolation")
    if shutil.which("niri") is None:
        parser.error("niri must be installed")

    app_dirs = [Path(item).resolve() for item in args.bin_dir]
    shell_dirs = [Path(item).resolve() for item in args.shell_bin_dir]
    for required in ("rmac-files", "rmac-text-editor", "rmac-system-settings", "rmac-calculator", "rmac-preview"):
        if not any((directory / required).is_file() for directory in app_dirs + shell_dirs):
            parser.error(f"installed binary not found: {required}")
    for required in ("rmac-top-bar", "rmac-dock", "rmac-shortcut-dispatch", "rmac-wallpaper"):
        if not any((directory / required).is_file() for directory in shell_dirs + app_dirs):
            parser.error(f"installed shell binary not found: {required}")

    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    logs = out / "logs"
    logs.mkdir(exist_ok=True)
    inventory = binary_inventory(app_dirs + shell_dirs)
    previous_manifest_path = out / "binaries.json"
    previous_report_path = out / "report.json"
    previous_inventory = json.loads(previous_manifest_path.read_text(encoding="utf-8")) \
        if previous_manifest_path.is_file() else None
    previous_report = json.loads(previous_report_path.read_text(encoding="utf-8")) \
        if previous_report_path.is_file() else None
    current_revision = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                                      text=True, capture_output=True, check=False).stdout.strip()
    if (args.only and previous_report) or args.skip_behavior:
        if previous_inventory != inventory:
            parser.error("refusing to merge phases: installed binary hashes/package owners differ from the existing report")
        if not previous_report or previous_report.get("source_revision") != current_revision:
            parser.error("refusing to merge phases: scenario source revision differs from the existing report")
    (out / "binaries.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")

    # These nested shell suites predate the installed /usr/libexec/rmac
    # layout. Symlinks let them test the exact installed files without copying
    # or rebuilding executables.
    aliases = out / "shell-bin-aliases"
    aliases.mkdir(exist_ok=True)
    for alias, binary in {
        "top-bar": "rmac-top-bar",
        "dock": "rmac-dock",
        "mission-control": "rmac-mission-control",
        "rmac-shortcut-dispatch": "rmac-shortcut-dispatch",
    }.items():
        source = next((directory / binary for directory in shell_dirs + app_dirs
                       if (directory / binary).is_file()), None)
        if source is None:
            parser.error(f"installed shell binary not found: {binary}")
        destination = aliases / alias
        if destination.is_symlink() or destination.exists():
            destination.unlink()
        destination.symlink_to(source)

    commands: list[tuple[str, list[str]]] = []
    if args.skip_behavior or (args.only and args.only != "behavior-27"):
        existing = out / "behavior.json"
        if not existing.is_file():
            parser.error("skipping behavior requires an existing behavior.json in --output")
        if not previous_report:
            parser.error("skipping behavior requires an existing report.json in --output")
        if previous_inventory != inventory:
            parser.error("refusing to reuse behavior.json: installed binary hashes/package owners differ from its manifest")
    else:
        behavior = [sys.executable, str(ROOT / "scripts/behavior/run_lulo.py")]
        for directory in app_dirs:
            behavior += ["--bin-dir", str(directory)]
        for directory in shell_dirs:
            behavior += ["--shell-bin-dir", str(directory)]
        behavior += ["--output", str(out / "behavior.json"), "--capture-dir", str(out / "captures")]
        commands.append(("behavior-27", behavior))

    if not args.skip_startup and (args.only is None or args.only == "startup-smoke"):
        startup = [sys.executable, str(ROOT / "scripts/linux/smoke-app-launches.py"),
                   "--bin-dir", str(app_dirs[0]), "--output", str(out / "startup.json")]
        commands.append(("startup-smoke", startup))

    if args.only is None or args.only == "terminal-roundtrip":
        terminal = [sys.executable, str(ROOT / "scripts/behavior/run_private_beta_journeys.py"),
                    "--niri", shutil.which("niri") or "niri", "--bin-dir", str(aliases),
                    "--app-bin-dir", str(app_dirs[0])]
        commands.append(("terminal-roundtrip", terminal))

    if not args.skip_shell and (args.only is None or args.only in {"power-dialogs", "shutdown-completion"}):
        if args.only is None or args.only == "power-dialogs":
            commands.append(("power-dialogs", [sys.executable, str(ROOT / "scripts/behavior/run_power_dialogs.py"),
                              "--niri", shutil.which("niri") or "niri", "--bin-dir", str(aliases),
                              "--capture-dir", str(out / "session-dialogs")]))
        if args.only is None or args.only == "shutdown-completion":
            commands.append(("shutdown-completion", [sys.executable, str(ROOT / "scripts/behavior/run_shutdown.py"),
                              "--niri", shutil.which("niri") or "niri", "--bin-dir", str(aliases)]))

    results = []
    if args.skip_behavior or (args.only and args.only != "behavior-27"):
        behavior_report = json.loads((out / "behavior.json").read_text(encoding="utf-8"))
        statuses = [row.get("status") for row in behavior_report.get("results", [])]
        results.append({
            "name": "behavior-27",
            "exit_code": 0 if len(statuses) == 27 and all(status == "pass" for status in statuses) else 1,
            "reused_existing_report": "behavior.json",
            "scenario_count": len(statuses),
            "scenario_statuses": {status: statuses.count(status) for status in sorted(set(statuses))},
        })
    new_results = []
    for name, command in commands:
        print(f"\n=== {name} ===", flush=True)
        result = run_step(name, command, logs)
        new_results.append(result)
        print(f"{name}: {'PASS' if result['exit_code'] == 0 else 'FAIL'} ({result['elapsed_seconds']}s)", flush=True)

    if args.only and previous_report:
        replaced = args.only
        results = [row for row in previous_report.get("results", []) if row.get("name") != replaced]
        results.extend(new_results)
    else:
        results.extend(new_results)

    behavior_data = json.loads((out / "behavior.json").read_text(encoding="utf-8")) \
        if (out / "behavior.json").is_file() else {"results": []}
    behavior_statuses = [row.get("status") for row in behavior_data.get("results", [])]
    behavior_counts = {status: behavior_statuses.count(status) for status in sorted(set(behavior_statuses))}
    for item in results:
        if item.get("name") == "behavior-27":
            item.update({
                "scenario_count": len(behavior_statuses),
                "scenario_statuses": behavior_counts,
            })
    startup_data = json.loads((out / "startup.json").read_text(encoding="utf-8")) \
        if (out / "startup.json").is_file() else {"results": []}
    session_capture_dir = out / "session-dialogs"
    report = {
        "format": 1,
        "created_unix": int(time.time()),
        "source_revision": current_revision or "unknown",
        "binary_count": len(inventory),
        "binary_manifest": "binaries.json",
        "results": results,
        "coverage": {
            "recorded_mac_behavior_scenarios": len(behavior_statuses),
            "behavior_statuses": behavior_counts,
            "startup_only_first_party_apps": len(startup_data.get("results", [])),
            "nested_confirmation_and_shutdown_suites_completed": sum(
                row.get("name") in {"power-dialogs", "shutdown-completion"}
                and row.get("exit_code") == 0 for row in results
            ),
            "private_terminal_typed_roundtrip_completed": sum(
                row.get("name") == "terminal-roundtrip" and row.get("exit_code") == 0
                for row in results
            ),
            "behavior_screenshot_count": sum(1 for path in (out / "captures").rglob("*.png"))
            if (out / "captures").exists() else 0,
            "session_dialog_screenshot_count": sum(1 for path in session_capture_dir.rglob("*.png"))
            if session_capture_dir.exists() else 0,
            "automatic_pixel_comparison": False,
            "real_hardware_or_live_session_tests": False,
            "real_power_commands": False,
        },
        "summary": {
            "passed": sum(item["exit_code"] == 0 for item in results),
            "failed": sum(item["exit_code"] != 0 for item in results),
            "not_run": max(0, (5 if args.only is None or previous_report else len(commands)) - len(results)),
        },
        "limits": [
            "Interaction scenarios compare named behavior and state against recorded Mac expectations; captures are for review, not an automatic pixel score.",
            "The existing full journey scripts that drive the owner's live session are intentionally excluded.",
        ],
    }
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"\nReport: {out / 'report.json'}", flush=True)
    return 1 if report["summary"]["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
