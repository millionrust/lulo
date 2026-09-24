#!/usr/bin/env python3
"""Measure todo.md's "Performance budgets" on the live reference laptop.

One command that measures, on the real rmac session (niri + systemd --user),
every shell surface (a fixed list of `rmac-*.service` units) and every rmac
application (launched directly, waited for, and closed), then reports:

  * idle CPU % over a fixed window, from `/proc/<pid>/stat` utime+stime
    deltas summed over the process's whole descendant tree (children
    included, since a surface can fork helpers);
  * idle wake-ups/s, from `/proc/<pid>/status` voluntary_ctxt_switches +
    nonvoluntary_ctxt_switches deltas over the same window and tree -- a
    surface with a non-trivial rate while nothing on screen changes is
    flagged as a suspected idle redraw (this project's own idle-frame fix,
    ADR 0013, took niri from 12% to 0.1% CPU, so a wakeup that keeps firing
    at idle is exactly the regression this guards against);
  * PSS memory, from `/proc/<pid>/smaps_rollup`'s `Pss:` line (not RSS --
    PSS is what is comparable across the shell's many small processes);
  * warm launch -> interactive time for applications: launches with
    RMAC_BENCHMARK_READY_FILE set (see scripts/measure-baseline.py) and
    prefers that file's appearance as the "interactive" marker; if an app
    does not yet create it, falls back to its window being mapped (from
    `niri msg --json windows`) and says so in the report rather than
    silently mixing methods;
  * frame timing: left as "not_measured" everywhere -- there is no cheap
    per-frame trace available from this harness (see docs/performance-baseline.md
    "Limits and next evidence"); a real number needs a platform-native
    trace this script does not attempt.

This script is self-contained (stdlib only) and is meant to be copied to the
reference laptop and run there, the same way as scripts/linux/run-journey-launch.py
(see docs/journey-suite.md):

    scp scripts/linux/measure-budgets.py jacob@<reference-pc>:/tmp/
    ssh jacob@<reference-pc> '
      exec 9>/tmp/lulo-journey.lock
      flock -w 900 9 &&
      python3 /tmp/measure-budgets.py \
        --json-output /tmp/rmac-budgets.json \
        --markdown-output /tmp/rmac-budgets.md
    '

The report is privacy-safe: no hostnames, no home-directory paths, no window
titles, no user names -- only unit names, package names, and numbers.

App launches drive the real screen; always run this under the shared-laptop
flock above, and never in parallel with another measurement or journey run.
This script never invokes cargo and never starts, stops, enables, or
reconfigures a systemd unit -- it only reads `/proc` and asks `systemctl
--user show` (a read-only query) for each surface's current MainPID.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import signal
import statistics
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional


# --------------------------------------------------------------------------
# Fixed inventory: shell surfaces (systemd --user units) and applications.
# --------------------------------------------------------------------------

# todo.md "Performance budgets": "Idle CPU ... <=1% for all shell surfaces
# combined". This is the fixed set of always-on-or-on-demand shell units;
# rmac-launcher/-quick-settings/-notification-center-panel are on-demand
# popovers (systemd Type=notify, Restart=on-success -- they exit when their
# last window closes) and are reported as "not running" at idle rather than
# treated as a measurement failure.
SHELL_SURFACES: tuple[str, ...] = (
    "rmac-top-bar",
    "rmac-dock",
    "rmac-wallpaper",
    "rmac-launcher",
    "rmac-quick-settings",
    "rmac-notification-center",
    "rmac-notification-center-panel",
    "rmac-osd",
    "rmac-app-switcher",
    "rmac-screenshot",
    "rmac-mission-control",
    "rmac-session-supervisor",
    "rmac-shortcut-broker",
    "rmac-focus",
    "rmac-clipboard",
)

# (display name, package/executable name, niri app_id) -- matches the
# executable names in each crate's Cargo.toml `[[bin]]` and the app_id in
# each packaging/rmac-apps/applications/org.rmac.*.desktop file.
APPS: tuple[tuple[str, str, str], ...] = (
    ("System Monitor", "rmac-system-monitor", "org.rmac.SystemMonitor"),
    ("Apps", "rmac-app-drawer", "org.rmac.AppDrawer"),
    ("Files", "rmac-files", "org.rmac.Files"),
    ("Notes", "rmac-notes", "org.rmac.Notes"),
    ("System Settings", "rmac-system-settings", "org.rmac.SystemSettings"),
    ("Terminal", "rmac-terminal", "org.rmac.Terminal"),
    ("Text Editor", "rmac-text-editor", "org.rmac.TextEditor"),
)

# todo.md: "p95 <= 500 ms for simple apps; <= 900 ms for Files and Terminal".
WIDE_BUDGET_PACKAGES = frozenset({"rmac-files", "rmac-terminal"})
SIMPLE_APP_BUDGET_MS = 500.0
WIDE_APP_BUDGET_MS = 900.0

IDLE_CPU_PER_APP_BUDGET_PERCENT = 0.3
IDLE_CPU_SHELL_COMBINED_BUDGET_PERCENT = 1.0

READY_FILE_ENV = "RMAC_BENCHMARK_READY_FILE"


class MeasurementError(RuntimeError):
    """A bounded, privacy-safe measurement failure."""


# --------------------------------------------------------------------------
# Pure parsing/computation helpers -- unit-tested from
# scripts/test_measure_budgets.py without a live /proc or session.
# --------------------------------------------------------------------------


def parse_proc_stat(text: str) -> dict[str, int]:
    """Parse a `/proc/<pid>/stat` line. `comm` can contain spaces and
    parentheses, so the split anchors on the *last* ')' rather than
    whitespace (see proc(5))."""

    text = text.strip()
    open_paren = text.index("(")
    close_paren = text.rindex(")")
    pid = int(text[:open_paren].strip())
    rest = text[close_paren + 1 :].split()
    if len(rest) < 15:
        raise ValueError("not enough fields in /proc/<pid>/stat")
    return {
        "pid": pid,
        "ppid": int(rest[1]),
        "pgrp": int(rest[2]),
        "utime": int(rest[11]),
        "stime": int(rest[12]),
    }


def parse_status_ctxt_switches(text: str) -> tuple[int, int]:
    """Parse `/proc/<pid>/status`'s voluntary/nonvoluntary_ctxt_switches."""

    voluntary: Optional[int] = None
    nonvoluntary: Optional[int] = None
    for line in text.splitlines():
        if line.startswith("voluntary_ctxt_switches:"):
            voluntary = int(line.split(":", 1)[1].strip())
        elif line.startswith("nonvoluntary_ctxt_switches:"):
            nonvoluntary = int(line.split(":", 1)[1].strip())
    if voluntary is None or nonvoluntary is None:
        raise ValueError("missing ctxt_switches fields in /proc/<pid>/status")
    return voluntary, nonvoluntary


def parse_smaps_rollup_pss_kib(text: str) -> int:
    """Parse the `Pss:` line of `/proc/<pid>/smaps_rollup`, in KiB."""

    for line in text.splitlines():
        if line.startswith("Pss:"):
            fields = line.split()
            if len(fields) < 2:
                raise ValueError("malformed Pss line in smaps_rollup")
            return int(fields[1])
    raise ValueError("no Pss line found in smaps_rollup")


def cpu_percent_from_ticks(delta_ticks: int, hertz: int, elapsed_seconds: float) -> float:
    if elapsed_seconds <= 0:
        raise ValueError("elapsed_seconds must be greater than zero")
    if hertz <= 0:
        raise ValueError("hertz must be greater than zero")
    return max(0.0, delta_ticks) / hertz / elapsed_seconds * 100


def wakeups_per_second(delta_ctxt_switches: int, elapsed_seconds: float) -> float:
    if elapsed_seconds <= 0:
        raise ValueError("elapsed_seconds must be greater than zero")
    return max(0.0, delta_ctxt_switches) / elapsed_seconds


def percentile_nearest_rank(values: list[float], percentile: float) -> float:
    ordered = sorted(values)
    rank = max(1, math.ceil(percentile * len(ordered)))
    return ordered[rank - 1]


def warm_launch_budget_ms(package: str) -> float:
    return WIDE_APP_BUDGET_MS if package in WIDE_BUDGET_PACKAGES else SIMPLE_APP_BUDGET_MS


def evaluate_budget(value: float, budget: float) -> dict[str, Any]:
    return {
        "value": round(value, 3),
        "budget": budget,
        "within_budget": value <= budget,
    }


def suspected_idle_redraw(wakeups_per_sec: float, threshold_per_sec: float) -> bool:
    """todo.md: "No redraw while nothing changes." A handful of scheduler
    wakeups is normal background noise (timers, D-Bus pings); a rate above
    the threshold while the surface is otherwise untouched is flagged."""

    return wakeups_per_sec > threshold_per_sec


def discover_environment(environ: dict[str, str], runtime_dir: Path) -> dict[str, str]:
    """Return environment additions needed to reach the live session over
    SSH, without mutating `environ`. Mirrors
    scripts/linux/run-journey-launch.py's helper of the same name."""

    additions: dict[str, str] = {}
    if "XDG_RUNTIME_DIR" not in environ:
        additions["XDG_RUNTIME_DIR"] = str(runtime_dir)
    if "NIRI_SOCKET" not in environ:
        sockets = sorted(runtime_dir.glob("niri*.sock"))
        if not sockets:
            raise MeasurementError("no niri IPC socket found in the runtime directory")
        additions["NIRI_SOCKET"] = str(sockets[0])
    if "WAYLAND_DISPLAY" not in environ:
        displays = sorted(
            entry.name
            for entry in runtime_dir.glob("wayland-*")
            if not entry.name.endswith(".lock")
        )
        if not displays:
            raise MeasurementError("no Wayland display socket found in the runtime directory")
        additions["WAYLAND_DISPLAY"] = displays[0]
    if "DBUS_SESSION_BUS_ADDRESS" not in environ:
        bus = runtime_dir / "bus"
        if not bus.exists():
            raise MeasurementError("no D-Bus session bus socket found in the runtime directory")
        additions["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={bus}"
    return additions


def parse_windows(stdout: str) -> list[dict[str, Any]]:
    try:
        windows = json.loads(stdout)
    except json.JSONDecodeError as error:
        raise MeasurementError("niri windows output was not valid JSON") from error
    if not isinstance(windows, list):
        raise MeasurementError("niri windows output was not a JSON array")
    return windows


def find_window_by_app_id(windows: list[dict[str, Any]], app_id: str) -> Optional[dict[str, Any]]:
    for window in windows:
        if window.get("app_id") == app_id:
            return window
    return None


# --------------------------------------------------------------------------
# Markdown rendering -- pure, given an already-built result dict.
# --------------------------------------------------------------------------


def _fmt_percent(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.2f}%"


def _fmt_ms(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.1f} ms"


def _fmt_mib(value: Optional[float]) -> str:
    return "n/a" if value is None else f"{value:.1f} MiB"


def render_markdown_report(report: dict[str, Any]) -> str:
    lines: list[str] = []
    captured_at = report.get("captured_at", "unknown")
    lines.append(f"# Reference laptop performance budgets -- {captured_at}")
    lines.append("")
    lines.append(
        "Measured against todo.md \"Performance budgets\": idle CPU <=0.3% per "
        "app and <=1% for all shell surfaces combined, no idle redraw, warm "
        "launch p95 <=500 ms (<=900 ms for Files/Terminal), memory recorded. "
        "No personal data is recorded: no hostnames, home-directory paths, "
        "window titles, or user names."
    )
    lines.append("")

    settings = report.get("settings", {})
    lines.append(
        f"Idle window: {settings.get('idle_seconds', 'n/a')} s; warm-launch "
        f"repetitions: {settings.get('repetitions', 'n/a')} "
        f"(+{settings.get('warmups', 'n/a')} warm-up); "
        f"idle-redraw threshold: {settings.get('wakeup_threshold_per_second', 'n/a')} wakeups/s."
    )
    if report.get("rustc_processes"):
        lines.append(
            f"\n**Note:** {report['rustc_processes']} `rustc` process(es) were "
            "running during this measurement; idle numbers may be skewed by a "
            "concurrent build."
        )
    lines.append("")

    lines.append("## Shell surfaces")
    lines.append("")
    lines.append(
        "| Surface | Running | Idle CPU % | Budget | Wake-ups/s | Idle redraw? | PSS |"
    )
    lines.append("|---|---|---:|---|---:|---|---:|")
    surfaces: dict[str, Any] = report.get("shell_surfaces", {})
    for name in sorted(surfaces):
        surface = surfaces[name]
        if not surface.get("running"):
            lines.append(f"| {name} | no | n/a | <=0.3% | n/a | n/a | n/a |")
            continue
        idle = surface["idle_cpu_percent"]
        lines.append(
            f"| {name} | yes | {_fmt_percent(idle['value'])} | "
            f"<={idle['budget']}% {'✓' if idle['within_budget'] else '✗'} | "
            f"{surface['wakeups_per_second']:.3f} | "
            f"{'yes' if surface['suspected_idle_redraw'] else 'no'} | "
            f"{_fmt_mib(surface['pss_mib'])} |"
        )
    combined = report.get("shell_combined")
    if combined is not None:
        lines.append(
            f"| **All shell surfaces combined** | -- | "
            f"{_fmt_percent(combined['value'])} | <={combined['budget']}% "
            f"{'✓' if combined['within_budget'] else '✗'} | -- | -- | -- |"
        )
    lines.append("")

    lines.append("## Applications")
    lines.append("")
    lines.append(
        "| App | Warm p95 | Budget | Interactive marker | Idle CPU % | Budget | "
        "Wake-ups/s | PSS | Frame timing |"
    )
    lines.append("|---|---:|---|---|---:|---|---:|---:|---|")
    apps: dict[str, Any] = report.get("apps", {})
    for package in sorted(apps):
        app = apps[package]
        warm = app.get("warm_launch")
        idle = app.get("idle")
        if warm is None:
            warm_cell = "n/a | n/a | not_measured"
        else:
            p95 = warm["p95_ms"]
            warm_cell = (
                f"{_fmt_ms(p95['value'])} | <={p95['budget']:.0f} ms "
                f"{'✓' if p95['within_budget'] else '✗'} | {warm['interactive_marker']}"
            )
        if idle is None:
            idle_cell = "n/a | n/a | n/a | n/a"
        else:
            cpu = idle["idle_cpu_percent"]
            idle_cell = (
                f"{_fmt_percent(cpu['value'])} | <={cpu['budget']}% "
                f"{'✓' if cpu['within_budget'] else '✗'} | "
                f"{idle['wakeups_per_second']:.3f} | {_fmt_mib(idle['pss_mib'])}"
            )
        lines.append(
            f"| {app['display_name']} | {warm_cell} | {idle_cell} | "
            f"{app.get('frame_timing', 'not_measured')} |"
        )
    lines.append("")

    over_budget: list[str] = []
    for name, surface in surfaces.items():
        if surface.get("running") and not surface["idle_cpu_percent"]["within_budget"]:
            over_budget.append(f"{name}: idle CPU over budget")
        if surface.get("running") and surface["suspected_idle_redraw"]:
            over_budget.append(f"{name}: suspected idle redraw")
    if combined is not None and not combined["within_budget"]:
        over_budget.append("all shell surfaces combined: idle CPU over budget")
    for package, app in apps.items():
        warm = app.get("warm_launch")
        if warm is not None and not warm["p95_ms"]["within_budget"]:
            over_budget.append(f"{app['display_name']}: warm launch p95 over budget")
        idle = app.get("idle")
        if idle is not None and not idle["idle_cpu_percent"]["within_budget"]:
            over_budget.append(f"{app['display_name']}: idle CPU over budget")

    lines.append("## Over budget")
    lines.append("")
    if over_budget:
        for item in over_budget:
            lines.append(f"- {item}")
    else:
        lines.append("- none")
    lines.append("")

    lines.append("## Method")
    lines.append("")
    lines.append(
        "- Idle CPU %: `/proc/<pid>/stat` utime+stime deltas summed over the "
        "surface's whole descendant process tree, divided by the elapsed "
        "wall time (one CPU-core-second == 100%)."
    )
    lines.append(
        "- Wake-ups/s: `/proc/<pid>/status` voluntary_ctxt_switches + "
        "nonvoluntary_ctxt_switches deltas over the same window and tree; a "
        "rate above the configured threshold while nothing changes is "
        "flagged as a suspected idle redraw."
    )
    lines.append("- Memory: PSS from `/proc/<pid>/smaps_rollup`'s `Pss:` line, not RSS.")
    lines.append(
        "- Warm launch -> interactive: elapsed time from spawn to "
        f"`{READY_FILE_ENV}` appearing when the app writes it, else a "
        "fallback to its window being mapped (reported as "
        "`window_mapped_fallback`, a weaker proxy for \"interactive\")."
    )
    lines.append(
        "- Frame timing: not measured -- no cheap per-frame trace is "
        "available from this harness; see docs/performance-baseline.md."
    )
    lines.append("")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Live-system helpers (exercised only on the reference laptop).
# --------------------------------------------------------------------------


def _run(args: list[str], timeout: float = 5.0) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            args,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
            stdin=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise MeasurementError(f"{' '.join(args)} did not complete") from error


def get_unit_main_pid(unit: str) -> Optional[int]:
    result = _run(["systemctl", "--user", "show", "-p", "MainPID", "--value", unit])
    if result.returncode != 0:
        return None
    value = result.stdout.strip()
    if not value.isdigit():
        return None
    pid = int(value)
    return pid if pid > 0 else None


def _read_text(path: Path) -> Optional[str]:
    try:
        return path.read_text()
    except (OSError, PermissionError):
        return None


def build_descendant_set(root_pid: int) -> set[int]:
    """All live PIDs descended from (and including) `root_pid`, from a
    single /proc scan. Missing/vanished processes are skipped silently --
    a process exiting mid-scan is not a measurement error."""

    children: dict[int, list[int]] = {}
    all_pids: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        stat_text = _read_text(entry / "stat")
        if stat_text is None:
            continue
        try:
            fields = parse_proc_stat(stat_text)
        except (ValueError, IndexError):
            continue
        all_pids.append(pid)
        children.setdefault(fields["ppid"], []).append(pid)

    if root_pid not in all_pids:
        return set()

    result = {root_pid}
    queue = [root_pid]
    while queue:
        current = queue.pop()
        for child in children.get(current, ()):
            if child not in result:
                result.add(child)
                queue.append(child)
    return result


def sample_process_tree(root_pid: int, hertz: int) -> Optional[dict[str, int]]:
    """One point-in-time sample summed over `root_pid`'s descendant tree:
    CPU ticks, both ctxt-switch counters, and PSS. Returns None if the root
    process is not alive."""

    pids = build_descendant_set(root_pid)
    if not pids:
        return None

    total_ticks = 0
    total_voluntary = 0
    total_nonvoluntary = 0
    total_pss_kib = 0
    for pid in pids:
        proc = Path(f"/proc/{pid}")
        stat_text = _read_text(proc / "stat")
        status_text = _read_text(proc / "status")
        rollup_text = _read_text(proc / "smaps_rollup")
        if stat_text is not None:
            try:
                fields = parse_proc_stat(stat_text)
                total_ticks += fields["utime"] + fields["stime"]
            except (ValueError, IndexError):
                pass
        if status_text is not None:
            try:
                voluntary, nonvoluntary = parse_status_ctxt_switches(status_text)
                total_voluntary += voluntary
                total_nonvoluntary += nonvoluntary
            except ValueError:
                pass
        if rollup_text is not None:
            try:
                total_pss_kib += parse_smaps_rollup_pss_kib(rollup_text)
            except ValueError:
                pass
    _ = hertz  # kept as a parameter for symmetry with the pure conversion helpers
    return {
        "cpu_ticks": total_ticks,
        "voluntary_ctxt_switches": total_voluntary,
        "nonvoluntary_ctxt_switches": total_nonvoluntary,
        "pss_kib": total_pss_kib,
        "process_count": len(pids),
    }


def measure_idle(
    root_pid: int, settle_seconds: float, idle_seconds: float, hertz: int
) -> Optional[dict[str, Any]]:
    time.sleep(settle_seconds)
    before = sample_process_tree(root_pid, hertz)
    if before is None:
        return None
    started = time.monotonic()
    time.sleep(idle_seconds)
    elapsed = time.monotonic() - started
    after = sample_process_tree(root_pid, hertz)
    if after is None:
        return None
    delta_ticks = after["cpu_ticks"] - before["cpu_ticks"]
    delta_ctxt = (
        after["voluntary_ctxt_switches"] + after["nonvoluntary_ctxt_switches"]
    ) - (before["voluntary_ctxt_switches"] + before["nonvoluntary_ctxt_switches"])
    return {
        "cpu_percent": cpu_percent_from_ticks(delta_ticks, hertz, elapsed),
        "wakeups_per_second": wakeups_per_second(delta_ctxt, elapsed),
        "pss_kib": after["pss_kib"],
        "elapsed_seconds": elapsed,
    }


def measure_shell_surface(
    unit_name: str,
    settle_seconds: float,
    idle_seconds: float,
    hertz: int,
    wakeup_threshold_per_second: float,
) -> dict[str, Any]:
    pid = get_unit_main_pid(f"{unit_name}.service")
    if pid is None:
        return {"running": False}

    sample = measure_idle(pid, settle_seconds, idle_seconds, hertz)
    if sample is None:
        return {"running": False, "note": "unit exited during measurement"}

    idle_budget = evaluate_budget(sample["cpu_percent"], IDLE_CPU_PER_APP_BUDGET_PERCENT)
    return {
        "running": True,
        "idle_cpu_percent": idle_budget,
        "wakeups_per_second": round(sample["wakeups_per_second"], 3),
        "suspected_idle_redraw": suspected_idle_redraw(
            sample["wakeups_per_second"], wakeup_threshold_per_second
        ),
        "pss_mib": round(sample["pss_kib"] / 1024, 1),
    }


def niri_windows() -> list[dict[str, Any]]:
    result = _run(["niri", "msg", "--json", "windows"])
    if result.returncode != 0:
        raise MeasurementError("niri msg windows failed")
    return parse_windows(result.stdout)


def discover_binary(executable: str, extra_dirs: list[Path]) -> Optional[Path]:
    # Applications (this function's callers) are packaged to /usr/bin (see
    # each org.rmac.*.desktop file's Exec=); shell surface units run from
    # ~/.local/libexec/rmac or /usr/libexec/rmac instead (see
    # crates/rmac-session/units/*.service) but are looked up by systemd unit,
    # never through this function.
    candidates = [
        *extra_dirs,
        Path.home() / ".local" / "libexec" / "rmac",
        Path("/usr/libexec/rmac"),
        Path("/usr/bin"),
    ]
    for directory in candidates:
        candidate = directory / executable
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
    return None


def start_app(binary: Path, ready_file: Path) -> subprocess.Popen[bytes]:
    ready_file.unlink(missing_ok=True)
    environment = os.environ.copy()
    environment[READY_FILE_ENV] = str(ready_file)
    return subprocess.Popen(
        [str(binary)],
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def pids_in_group(pgid: int) -> set[int]:
    """All live PIDs currently in process group `pgid`, from a single /proc
    scan. Unlike a ppid-based descendant walk, this survives a member being
    reparented to the init/systemd subreaper -- group membership does not
    change on reparenting, only ancestry does."""

    result: set[int] = set()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        stat_text = _read_text(entry / "stat")
        if stat_text is None:
            continue
        try:
            fields = parse_proc_stat(stat_text)
        except (ValueError, IndexError):
            continue
        if fields["pgrp"] == pgid:
            result.add(fields["pid"])
    return result


def stop_app(process: subprocess.Popen[bytes]) -> None:
    """Close an app this script launched. `start_new_session=True` gives the
    launched process its own session and process group, equal to its own
    pid, at spawn time.

    A plain `killpg` + `wait()` on the tracked pid is not enough by itself:
    that pid can be a short-lived launcher that forks a long-lived
    grandchild and then exits on its own (unrelated to the signal), so
    `process.wait()` returns success while the grandchild -- still a member
    of the same process group -- keeps running (observed live under a
    concurrent cargo build: a `rmac-text-editor` instance from this launch
    path outlived `killpg` + `wait` and had to be killed by hand). So this
    always re-scans by process-group membership afterwards, independent of
    whether `wait()` reported success, and force-kills any survivor
    individually by pid."""

    pgid = process.pid
    if process.poll() is None:
        try:
            os.killpg(pgid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass

    for _ in range(4):
        survivors = pids_in_group(pgid)
        if not survivors:
            return
        for pid in survivors:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.3)
    remaining = pids_in_group(pgid)
    if remaining:
        print(
            f"measure-budgets: warning: pid(s) {sorted(remaining)} in "
            f"process group {pgid} outlived SIGKILL and may still be running",
            file=sys.stderr,
        )


def wait_for_interactive(
    process: subprocess.Popen[bytes],
    ready_file: Path,
    app_id: str,
    timeout: float,
) -> tuple[float, str]:
    """Elapsed seconds to "interactive" plus which marker was used."""

    started = time.monotonic()
    deadline = started + timeout
    while time.monotonic() < deadline:
        if ready_file.exists():
            return time.monotonic() - started, "ready_file"
        exit_code = process.poll()
        if exit_code is not None:
            raise MeasurementError(f"process exited before becoming ready: status {exit_code}")
        time.sleep(0.005)
    # Fall back to window-mapped, from where the ready-file wait left off, so
    # a slow app that also lacks the marker is not double-penalized.
    remaining_deadline = time.monotonic() + timeout
    while time.monotonic() < remaining_deadline:
        try:
            windows = niri_windows()
        except MeasurementError:
            windows = []
        if find_window_by_app_id(windows, app_id) is not None:
            return time.monotonic() - started, "window_mapped_fallback"
        exit_code = process.poll()
        if exit_code is not None:
            raise MeasurementError(f"process exited before mapping a window: status {exit_code}")
        time.sleep(0.05)
    raise MeasurementError(
        f"neither {READY_FILE_ENV} nor a mapped window appeared within "
        f"{timeout * 2:.1f} seconds"
    )


def measure_app(
    display_name: str,
    package: str,
    app_id: str,
    binary: Path,
    warmups: int,
    repetitions: int,
    settle_seconds: float,
    idle_seconds: float,
    startup_timeout: float,
    hertz: int,
    wakeup_threshold_per_second: float,
    temp_dir: Path,
) -> dict[str, Any]:
    markers: list[str] = []
    for index in range(warmups):
        ready_file = temp_dir / f"warmup-{index}.ready"
        process = start_app(binary, ready_file)
        try:
            wait_for_interactive(process, ready_file, app_id, startup_timeout)
        finally:
            stop_app(process)

    samples_ms: list[float] = []
    for index in range(repetitions):
        ready_file = temp_dir / f"startup-{index}.ready"
        process = start_app(binary, ready_file)
        try:
            elapsed_seconds, marker = wait_for_interactive(
                process, ready_file, app_id, startup_timeout
            )
            samples_ms.append(elapsed_seconds * 1000)
            markers.append(marker)
        finally:
            stop_app(process)

    budget_ms = warm_launch_budget_ms(package)
    p95_ms = percentile_nearest_rank(samples_ms, 0.95)
    warm_launch = {
        "samples_ms": [round(value, 1) for value in samples_ms],
        "median_ms": round(statistics.median(samples_ms), 1),
        "p95_ms": evaluate_budget(p95_ms, budget_ms),
        # ready_file is trusted only when every sample used it; a single
        # fallback sample marks the whole metric as the weaker proxy so the
        # report never quietly mixes the two.
        "interactive_marker": "ready_file" if set(markers) == {"ready_file"} else "window_mapped_fallback",
    }

    ready_file = temp_dir / "idle.ready"
    process = start_app(binary, ready_file)
    idle_result: Optional[dict[str, Any]] = None
    try:
        wait_for_interactive(process, ready_file, app_id, startup_timeout)
        sample = measure_idle(process.pid, settle_seconds, idle_seconds, hertz)
        if sample is not None:
            idle_result = {
                "idle_cpu_percent": evaluate_budget(
                    sample["cpu_percent"], IDLE_CPU_PER_APP_BUDGET_PERCENT
                ),
                "wakeups_per_second": round(sample["wakeups_per_second"], 3),
                "suspected_idle_redraw": suspected_idle_redraw(
                    sample["wakeups_per_second"], wakeup_threshold_per_second
                ),
                "pss_mib": round(sample["pss_kib"] / 1024, 1),
            }
    finally:
        stop_app(process)

    return {
        "display_name": display_name,
        "warm_launch": warm_launch,
        "idle": idle_result,
        "frame_timing": "not_measured",
    }


# --------------------------------------------------------------------------
# Orchestration.
# --------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--idle-seconds", type=float, default=60.0)
    parser.add_argument("--settle-seconds", type=float, default=3.0)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--startup-timeout", type=float, default=10.0)
    parser.add_argument(
        "--wakeup-threshold-per-second",
        type=float,
        default=0.5,
        help="wakeup rate above this while idle is flagged as a suspected idle redraw",
    )
    parser.add_argument(
        "--surface",
        action="append",
        dest="surfaces",
        choices=SHELL_SURFACES,
        help="measure only this shell surface; may be repeated",
    )
    parser.add_argument(
        "--app",
        action="append",
        dest="apps",
        choices=[package for _, package, _ in APPS],
        help="measure only this application; may be repeated",
    )
    parser.add_argument("--skip-surfaces", action="store_true")
    parser.add_argument("--skip-apps", action="store_true")
    parser.add_argument(
        "--binary-dir",
        action="append",
        type=Path,
        default=[],
        help="additional directory to search for app binaries before the installed paths",
    )
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path, default=None)
    args = parser.parse_args()
    for field in ("idle_seconds", "settle_seconds", "startup_timeout"):
        if getattr(args, field) <= 0:
            parser.error(f"--{field.replace('_', '-')} must be greater than zero")
    if args.warmups < 0:
        parser.error("--warmups must be zero or greater")
    if args.repetitions < 1:
        parser.error("--repetitions must be at least one")
    return args


def count_rustc_processes() -> int:
    result = _run(["pgrep", "-c", "rustc"], timeout=3.0)
    text = result.stdout.strip()
    return int(text) if text.isdigit() else 0


def main() -> int:
    args = parse_args()
    hertz = os.sysconf("SC_CLK_TCK")

    additions = discover_environment(dict(os.environ), Path(f"/run/user/{os.getuid()}"))
    os.environ.update(additions)

    report: dict[str, Any] = {
        "schema_version": 1,
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "rustc_processes": count_rustc_processes(),
        "settings": {
            "idle_seconds": args.idle_seconds,
            "settle_seconds": args.settle_seconds,
            "warmups": args.warmups,
            "repetitions": args.repetitions,
            "startup_timeout": args.startup_timeout,
            "wakeup_threshold_per_second": args.wakeup_threshold_per_second,
        },
        "shell_surfaces": {},
        "apps": {},
    }

    if not args.skip_surfaces:
        surfaces = args.surfaces if args.surfaces else list(SHELL_SURFACES)
        for name in surfaces:
            print(f"Measuring shell surface {name}...", flush=True)
            report["shell_surfaces"][name] = measure_shell_surface(
                name,
                args.settle_seconds,
                args.idle_seconds,
                hertz,
                args.wakeup_threshold_per_second,
            )
        running_cpu = [
            surface["idle_cpu_percent"]["value"]
            for surface in report["shell_surfaces"].values()
            if surface.get("running")
        ]
        if running_cpu:
            report["shell_combined"] = evaluate_budget(
                sum(running_cpu), IDLE_CPU_SHELL_COMBINED_BUDGET_PERCENT
            )

    if not args.skip_apps:
        selected_packages = set(args.apps) if args.apps else None
        apps_to_measure = [
            app for app in APPS if selected_packages is None or app[1] in selected_packages
        ]
        import tempfile

        with tempfile.TemporaryDirectory(prefix="rmac-budgets-") as directory:
            temp_root = Path(directory)
            for display_name, package, app_id in apps_to_measure:
                binary = discover_binary(package, args.binary_dir)
                if binary is None:
                    print(f"Skipping {display_name}: binary not found", flush=True)
                    report["apps"][package] = {
                        "display_name": display_name,
                        "warm_launch": None,
                        "idle": None,
                        "frame_timing": "not_measured",
                        "note": "binary not found",
                    }
                    continue
                print(f"Measuring {display_name}...", flush=True)
                app_temp = temp_root / package
                app_temp.mkdir()
                try:
                    report["apps"][package] = measure_app(
                        display_name,
                        package,
                        app_id,
                        binary,
                        args.warmups,
                        args.repetitions,
                        args.settle_seconds,
                        args.idle_seconds,
                        args.startup_timeout,
                        hertz,
                        args.wakeup_threshold_per_second,
                        app_temp,
                    )
                except MeasurementError as error:
                    report["apps"][package] = {
                        "display_name": display_name,
                        "warm_launch": None,
                        "idle": None,
                        "frame_timing": "not_measured",
                        "note": str(error),
                    }

    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Wrote {args.json_output}")

    if args.markdown_output is not None:
        args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_output.write_text(render_markdown_report(report))
        print(f"Wrote {args.markdown_output}")

    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except MeasurementError as error:
        print(f"measure-budgets: {error}", file=sys.stderr)
        sys.exit(4)
    except KeyboardInterrupt:
        sys.exit(130)
