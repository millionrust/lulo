#!/usr/bin/env python3
"""Run the private UI and startup checks against an installed Lulo build.

Usage on the Lulo host:
    python3 scripts/behavior/run_installed_suite.py \
      --bin-dir /usr/bin --shell-bin-dir /usr/libexec/rmac \
      --output /tmp/lulo-installed-report

Coverage is 27 recorded Mac behavior scenarios, startup-only readiness for 14
first-party apps, a private Terminal typed-command roundtrip, Notes crash
recovery, and nested power-dialog and shutdown flows. Screenshots are
captured for review, but no automatic pixel score is calculated. The suite
does not exercise real hardware, live login/logout, or real power state.

Each child suite owns a private HOME, XDG runtime, D-Bus session and nested
headless compositor. The commands run sequentially and never use the active
desktop session or real system power commands. Each phase has a bounded
runtime; a timeout terminates that phase's private process group and records
a failure before the next phase starts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
PHASE_NAMES = {
    "behavior-27",
    "startup-smoke",
    "terminal-roundtrip",
    "notes-recovery",
    "power-dialogs",
    "shutdown-completion",
}
EXPECTED_BEHAVIOR_COUNT = 27
EXPECTED_STARTUP_APPS = {
    "archive-utility": "rmac-archive-utility",
    "app-drawer": "rmac-app-drawer",
    "clock": "rmac-clock",
    "calendar": "rmac-calendar",
    "notes": "rmac-notes",
    "player": "rmac-player",
    "preview": "rmac-preview",
    "system-monitor": "rmac-system-monitor",
    "terminal": "rmac-terminal",
    "weather": "rmac-weather",
    "calculator": "rmac-calculator",
    "system-settings": "rmac-system-settings",
    "text-editor": "rmac-text-editor",
    "files": "rmac-files",
}
PHASE_TIMEOUT_SECONDS = {
    "behavior-27": 900,
    "startup-smoke": 600,
    "terminal-roundtrip": 120,
    "notes-recovery": 180,
    "power-dialogs": 300,
    "shutdown-completion": 300,
}
SOURCE_INPUTS = (
    "scripts/behavior",
    "scripts/linux/run-journey-terminal.py",
    "scripts/linux/smoke-app-launches.py",
    "packaging/rmac-session/shell.kdl",
    "tests/behavior",
)


def count_not_run(results: list[dict[str, object]]) -> int:
    """Count suite phases absent from the cumulative report."""
    represented = {row.get("name") for row in results if row.get("name") in PHASE_NAMES}
    return len(PHASE_NAMES - represented)


def report_accounting(results: list[dict[str, object]]) -> tuple[int, int, int]:
    """Return passed, failed, and not-run counts, rejecting ambiguous rows."""
    rows_by_name: dict[str, list[dict[str, object]]] = {}
    malformed = 0
    for row in results:
        name = row.get("name")
        if name not in PHASE_NAMES:
            malformed += 1
            continue
        rows_by_name.setdefault(name, []).append(row)

    passed = failed = 0
    for rows in rows_by_name.values():
        if len(rows) != 1:
            failed += 1
        elif type(rows[0].get("exit_code")) is int and rows[0]["exit_code"] == 0:
            passed += 1
        else:
            failed += 1
    return passed, failed + malformed, count_not_run(results)


def behavior_report_complete(
    report: object, scenario_root: Path = ROOT / "tests/behavior",
    expected_count: int = EXPECTED_BEHAVIOR_COUNT,
) -> bool:
    """Require one passing observation for every recorded Mac scenario."""
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        return False
    expected = {
        path.relative_to(scenario_root).with_suffix("").as_posix()
        for path in scenario_root.rglob("*.json")
        if not path.name.endswith(".mac.json")
    }
    rows = report["results"]
    if len(expected) != expected_count or len(rows) != expected_count:
        return False
    if not all(isinstance(row, dict) for row in rows):
        return False
    actual = [row.get("scenario") for row in rows]
    if not all(isinstance(identifier, str) for identifier in actual):
        return False
    return len(set(actual)) == expected_count and set(actual) == expected and all(
        row.get("status") == "pass"
        and row.get("mismatches") == []
        and isinstance(row.get("lulo"), dict)
        and row["lulo"].get("scenario") == row["scenario"]
        for row in rows
    )


def startup_report_complete(
    report: object, inventory: list[dict[str, str]], startup_bin_dir: Path,
    expected_apps: dict[str, str] = EXPECTED_STARTUP_APPS,
) -> bool:
    """Require all packaged startup apps to pass on inventoried binaries."""
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        return False
    rows = report["results"]
    if len(rows) != len(expected_apps) or not all(isinstance(row, dict) for row in rows):
        return False
    if report.get("summary") != {
        "passed": len(expected_apps), "failed": 0, "skipped": 0,
        "total": len(expected_apps),
    }:
        return False
    hashes: dict[str, str] = {}
    for item in inventory:
        path = Path(item["path"])
        if path.parent == startup_bin_dir.resolve():
            hashes[path.name] = item["sha256"]
    actual_apps = [row.get("app") for row in rows]
    if not all(isinstance(app, str) for app in actual_apps):
        return False
    return len(set(actual_apps)) == len(expected_apps) and set(actual_apps) == set(expected_apps) and all(
        row.get("binary") == expected_apps[row["app"]]
        and row.get("outcome") == "passed"
        and row.get("binary_sha256") == hashes.get(row["binary"])
        and row.get("readiness") == (
            "fixture_extracted" if row["app"] == "archive-utility" else
            "accessible_layer_surface" if row["app"] == "app-drawer" else
            "mapped_and_accessible"
        )
        for row in rows
    )


def source_inputs_sha256(root: Path = ROOT, inputs: tuple[str, ...] = SOURCE_INPUTS) -> str:
    """Fingerprint actual runner and scenario bytes, including uncommitted files."""
    files = []
    for name in inputs:
        path = root / name
        if path.is_dir():
            files.extend(child for child in path.rglob("*") if child.suffix in {".py", ".json"})
        elif path.is_file():
            files.append(path)
        else:
            raise RuntimeError(f"installed-suite source input unavailable: {name}")
    digest = hashlib.sha256()
    for path in sorted(files):
        data = path.read_bytes()
        digest.update(path.relative_to(root).as_posix().encode("utf-8") + b"\0")
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    return digest.hexdigest()


def binary_inventory(directories: list[Path]) -> list[dict[str, str]]:
    inventory = []
    seen: set[Path] = set()
    for directory in sorted({item.resolve() for item in directories}):
        for path in sorted(directory.glob("rmac-*")):
            path = path.resolve()
            if path in seen or not path.is_file() or not os.access(path, os.X_OK):
                continue
            seen.add(path)
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            owner = subprocess.run(
                ["dpkg-query", "-S", str(path)], text=True, capture_output=True, check=False
            )
            package_owner = owner.stdout.strip() if owner.returncode == 0 else "unowned"
            inventory.append({
                "path": str(path),
                "sha256": digest,
                "package_owner": package_owner,
            })
    return inventory


def installed_inventory_valid(
    inventory: list[dict[str, str]], app_dirs: list[Path], shell_dirs: list[Path]
) -> bool:
    """Reject build-tree binaries presented as installed package evidence."""
    app_dir = Path("/usr/bin").resolve()
    shell_dir = Path("/usr/libexec/rmac").resolve()
    if set(app_dirs) != {app_dir} or set(shell_dirs) != {shell_dir}:
        return False
    if not inventory:
        return False
    for item in inventory:
        path = Path(item["path"])
        package = (
            "rmac-apps" if path.parent == app_dir else
            "rmac-session" if path.parent == shell_dir else None
        )
        if package is None or item["package_owner"] != f"{package}: {path}":
            return False
    return True


def installed_package_versions() -> dict[str, str]:
    """Record the actual Debian versions under test, separate from runner HEAD."""
    versions = {}
    for package in ("rmac-apps", "rmac-session"):
        result = subprocess.run(
            ["dpkg-query", "-W", "-f=${Version}", package],
            text=True, capture_output=True, check=False,
        )
        version = result.stdout.strip()
        if result.returncode != 0 or not version or "\n" in version:
            raise RuntimeError(f"installed package version unavailable: {package}")
        versions[package] = version
    return versions


def installed_packages_verified() -> bool:
    """Require installed package files to match dpkg's recorded checksums."""
    result = subprocess.run(
        ["dpkg", "--verify", "rmac-apps", "rmac-session"],
        text=True, capture_output=True, check=False,
    )
    return result.returncode == 0 and not result.stdout.strip()


def run_step(
    name: str, argv: list[str], log_dir: Path, *, timeout_seconds: float | None = None
) -> dict[str, object]:
    """Bound one isolated phase and reap its private process group on exit."""
    timeout = PHASE_TIMEOUT_SECONDS[name] if timeout_seconds is None else timeout_seconds
    started = time.monotonic()
    timed_out = False
    with (log_dir / f"{name}.stdout.txt").open("wb") as stdout_log, \
         (log_dir / f"{name}.stderr.txt").open("wb") as stderr_log:
        process = subprocess.Popen(
            argv, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=stdout_log,
            stderr=stderr_log, start_new_session=True,
        )
        try:
            exit_code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
                process.wait(timeout=5)
            # A child may outlive the runner after it exits; all suite children
            # share this private process group, so close it before the next phase.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            exit_code = 124
        else:
            # A phase can return successfully while a helper it started is
            # still running. Reap the private session before the next phase
            # so those helpers cannot overlap it or outlive the suite.
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
            time.sleep(0.1)
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
    elapsed = round(time.monotonic() - started, 2)
    return {
        "name": name,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "timeout_seconds": timeout,
        "elapsed_seconds": elapsed,
        "stdout_log": f"logs/{name}.stdout.txt",
        "stderr_log": f"logs/{name}.stderr.txt",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", action="append", required=True, help="installed app binary directory")
    parser.add_argument("--shell-bin-dir", action="append", required=True, help="installed shell binary directory")
    parser.add_argument("--output", type=Path, required=True, help="report directory (created if needed)")
    parser.add_argument("--only", choices=("behavior-27", "startup-smoke", "terminal-roundtrip", "notes-recovery", "power-dialogs", "shutdown-completion"),
                        help="run one phase and merge it into an existing report when binary hashes match")
    parser.add_argument("--skip-behavior", action="store_true", help="reuse behavior.json already in --output")
    parser.add_argument("--skip-startup", action="store_true")
    parser.add_argument("--skip-shell", action="store_true", help="skip power dialogs and shutdown private suites")
    args = parser.parse_args()

    if args.only == "startup-smoke" and args.skip_startup:
        parser.error("--only startup-smoke cannot be combined with --skip-startup")
    if args.only == "behavior-27" and args.skip_behavior:
        parser.error("--only behavior-27 cannot be combined with --skip-behavior")
    if args.only in {"power-dialogs", "shutdown-completion"} and args.skip_shell:
        parser.error(f"--only {args.only} cannot be combined with --skip-shell")

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
    if not installed_inventory_valid(inventory, app_dirs, shell_dirs):
        parser.error("installed suite requires package-owned binaries in /usr/bin and /usr/libexec/rmac")
    try:
        package_versions = installed_package_versions()
    except RuntimeError as error:
        parser.error(str(error))
    if not installed_packages_verified():
        parser.error("installed rmac-apps or rmac-session files differ from package checksums")
    previous_manifest_path = out / "binaries.json"
    previous_report_path = out / "report.json"
    previous_inventory = json.loads(previous_manifest_path.read_text(encoding="utf-8")) \
        if previous_manifest_path.is_file() else None
    previous_report = json.loads(previous_report_path.read_text(encoding="utf-8")) \
        if previous_report_path.is_file() else None
    current_revision = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                                      text=True, capture_output=True, check=False).stdout.strip()
    current_source_inputs = source_inputs_sha256()
    if (args.only and previous_report) or args.skip_behavior:
        if previous_inventory != inventory:
            parser.error("refusing to merge phases: installed binary hashes/package owners differ from the existing report")
        if previous_report and previous_report.get("installed_package_versions") != package_versions:
            parser.error("refusing to merge phases: installed package versions differ from the existing report")
        if not previous_report or previous_report.get("source_revision") != current_revision:
            parser.error("refusing to merge phases: scenario source revision differs from the existing report")
        if previous_report.get("source_inputs_sha256") != current_source_inputs:
            parser.error("refusing to merge phases: modified suite source differs from the existing report")
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

    if args.only is None or args.only == "notes-recovery":
        notes = [sys.executable, str(ROOT / "scripts/behavior/run_private_notes_recovery.py"),
                 "--bin-dir", str(app_dirs[0])]
        commands.append(("notes-recovery", notes))

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
            "exit_code": 0 if behavior_report_complete(behavior_report) else 1,
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
    complete_behavior = behavior_report_complete(behavior_data)
    for item in results:
        if item.get("name") == "behavior-27":
            if item.get("exit_code") == 0 and not complete_behavior:
                item["exit_code"] = 1
                item["artifact_validation"] = "missing, duplicate, or non-passing recorded scenario"
                print("behavior-27: FAIL (recorded scenario artifact is incomplete)", flush=True)
            item.update({
                "scenario_count": len(behavior_statuses),
                "scenario_statuses": behavior_counts,
            })
    startup_data = json.loads((out / "startup.json").read_text(encoding="utf-8")) \
        if (out / "startup.json").is_file() else {"results": []}
    complete_startup = startup_report_complete(startup_data, inventory, app_dirs[0])
    for item in results:
        if item.get("name") == "startup-smoke" and item.get("exit_code") == 0 and not complete_startup:
            item["exit_code"] = 1
            item["artifact_validation"] = "missing, duplicate, or mismatched packaged startup app"
            print("startup-smoke: FAIL (packaged app artifact is incomplete)", flush=True)
    session_capture_dir = out / "session-dialogs"
    report = {
        "format": 1,
        "created_unix": int(time.time()),
        "source_revision": current_revision or "unknown",
        "installed_package_versions": package_versions,
        "source_inputs_sha256": current_source_inputs,
        "binary_count": len(inventory),
        "binary_manifest": "binaries.json",
        "binary_inventory_scope": "all executable rmac-* files in /usr/bin and /usr/libexec/rmac; every entry must be owned by its expected Lulo package",
        "results": results,
        "coverage": {
            "recorded_mac_behavior_scenarios": len(behavior_statuses),
            "behavior_statuses": behavior_counts,
            "startup_smoke_app_count": len(startup_data.get("results", [])),
            "nested_confirmation_and_shutdown_suites_completed": sum(
                row.get("name") in {"power-dialogs", "shutdown-completion"}
                and row.get("exit_code") == 0 for row in results
            ),
            "private_terminal_typed_roundtrip_completed": sum(
                row.get("name") == "terminal-roundtrip" and row.get("exit_code") == 0
                for row in results
            ),
            "private_notes_crash_recovery_completed": sum(
                row.get("name") == "notes-recovery" and row.get("exit_code") == 0
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
        "summary": dict(zip(("passed", "failed", "not_run"), report_accounting(results))),
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
