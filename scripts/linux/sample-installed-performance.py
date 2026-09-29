#!/usr/bin/env python3
"""Sample one installed Lulo app in a private, headless Wayland session.

This records launch-to-first-window, then idle CPU, context-switch rate, and
PSS for the app process tree. It does not inject input or inspect pixels, so it
is performance evidence only, not a functional or visual test.

Example (on the Lulo reference host):
  python3 scripts/linux/sample-installed-performance.py --app rmac-files

The nested Sway compositor and D-Bus session are private. Each app gets a
temporary HOME and XDG tree. No systemd units, power actions, or Cargo builds
are run. Do not run while a build is active: startup and idle CPU are host-load
sensitive. Use --idle-seconds 30 or more for comparisons.
Use --thread-breakdown to add approximate per-thread CPU attribution for
threads present at both ends of the idle interval.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


SCRIPT_DIR = Path(__file__).resolve().parent


def load_script(name: str, path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path.name}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


smoke = load_script("lulo_smoke_app_launches", SCRIPT_DIR / "smoke-app-launches.py")
budgets = load_script("lulo_measure_budgets", SCRIPT_DIR / "measure-budgets.py")
SAMPLE_APPS = budgets.APPS + (
    ("Player", "rmac-player", "org.rmac.Player"),
    ("Archive Utility", "rmac-archive-utility", "org.rmac.ArchiveUtility"),
)


def args_parser() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", required=True, choices=[row[1] for row in SAMPLE_APPS],
                        help="measure exactly one installed application")
    parser.add_argument("--idle-seconds", type=float, default=30.0)
    parser.add_argument("--settle-seconds", type=float, default=3.0)
    parser.add_argument("--startup-timeout", type=float, default=20.0)
    parser.add_argument("--output", type=Path, help="write JSON report here")
    parser.add_argument("--binary-dir", type=Path, default=Path("/usr/bin"),
                        help="directory containing installed app binaries")
    parser.add_argument("--thread-breakdown", action="store_true",
                        help="include the busiest app-process threads in the idle report")
    args = parser.parse_args()
    for field in ("idle_seconds", "settle_seconds", "startup_timeout"):
        value = getattr(args, field)
        if not math.isfinite(value) or value <= 0:
            parser.error(f"--{field.replace('_', '-')} must be a finite positive number")
    return args


def app_environment(base: dict[str, str], root: Path, ready: Path) -> dict[str, str]:
    env = base.copy()
    home = root / "home"
    for key, path in (
        ("HOME", home),
        ("XDG_CONFIG_HOME", home / ".config"),
        ("XDG_DATA_HOME", home / ".local/share"),
        ("XDG_STATE_HOME", home / ".local/state"),
        ("XDG_CACHE_HOME", home / ".cache"),
    ):
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    env["RMAC_BENCHMARK_READY_FILE"] = str(ready)
    return env


def wait_ready(
    process: subprocess.Popen[Any], sway: Any, ready: Path, started: float,
    timeout: float, expected_app_id: str,
) -> tuple[float, str]:
    deadline = started + timeout
    while time.monotonic() < deadline:
        if ready.is_file():
            return (time.monotonic() - started) * 1000, "first_gpui_frame_marker"
        if process.poll() is not None:
            raise RuntimeError(f"app exited before readiness (status {process.returncode})")
        if sway.has_window(process.pid, expected_app_id):
            return (time.monotonic() - started) * 1000, "window_mapped"
        time.sleep(0.025)
    raise RuntimeError("app did not create its first frame or map a window before timeout")


def binary_provenance(binary: Path) -> dict[str, Any]:
    resolved = binary.resolve(strict=True)
    digest = hashlib.sha256()
    with resolved.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)

    result: dict[str, Any] = {
        "resolved_path": str(resolved),
        "sha256": digest.hexdigest(),
        "dpkg_owner_verified": False,
        "dpkg_package": None,
        "dpkg_version": None,
    }
    query = subprocess.run(
        ["dpkg-query", "--search", str(resolved)], capture_output=True, text=True,
        timeout=5, check=False,
    ) if shutil.which("dpkg-query") else None
    if query is None or query.returncode != 0:
        result["provenance_note"] = "binary hash recorded; dpkg owner/version could not be verified"
        return result
    owners = [line.partition(":")[0].strip() for line in query.stdout.splitlines() if ":" in line]
    if not owners:
        result["provenance_note"] = "binary hash recorded; dpkg did not identify an owning package"
        return result

    owner = owners[0]
    version = subprocess.run(
        ["dpkg-query", "--show", "--showformat=${Version}", owner],
        capture_output=True, text=True, timeout=5, check=False,
    )
    result.update(dpkg_owner_verified=True, dpkg_package=owner)
    if version.returncode == 0 and version.stdout.strip():
        result["dpkg_version"] = version.stdout.strip()
    else:
        result["provenance_note"] = "dpkg owner verified; package version query failed"
    return result


def launch_command(binary: Path, spec: Any, fixture_dir: Path) -> list[str]:
    """Build the smoke runner's installed-app launch command."""
    resolved = binary.resolve(strict=True)
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise RuntimeError(f"installed executable unavailable: {binary.name}")
    return [str(resolved), *smoke.fixture_arguments(spec, fixture_dir)]


def launch_spec(app_id: str, package: str) -> Any:
    """Return a smoke launch definition or the app's ordinary window mode."""
    return next(
        (spec for spec in smoke.APP_SPECS if spec.binary == package),
        smoke.AppSpec(app_id, package),
    )


def thread_tick_samples(pids: set[int], proc_root: Path = Path("/proc")) -> dict[tuple[int, int, int], tuple[str, int]]:
    """Read each thread's identity and CPU ticks without stopping the app."""
    result: dict[tuple[int, int, int], tuple[str, int]] = {}
    for pid in pids:
        task_dir = proc_root / str(pid) / "task"
        try:
            threads = list(task_dir.iterdir())
        except OSError:
            continue
        for thread in threads:
            if not thread.name.isdigit():
                continue
            try:
                raw = (thread / "stat").read_text()
                fields = budgets.parse_proc_stat(raw)
                suffix = raw[raw.rindex(")") + 1:].split()
                if len(suffix) < 20:
                    continue
                start_ticks = int(suffix[19])
                name = (thread / "comm").read_text().strip() or "unnamed"
            except (OSError, ValueError, IndexError):
                continue
            result[(pid, int(thread.name), start_ticks)] = (
                name[:32], fields["utime"] + fields["stime"]
            )
    return result


def busiest_threads(
    before: dict[tuple[int, int, int], tuple[str, int]],
    after: dict[tuple[int, int, int], tuple[str, int]],
    elapsed: float,
    hertz: int,
) -> list[dict[str, Any]]:
    """Return top measured threads; start ticks prevent PID/TID reuse errors."""
    rows = []
    for identity, (name, ticks) in after.items():
        previous = before.get(identity)
        if previous is None or ticks < previous[1]:
            continue
        percent = budgets.cpu_percent_from_ticks(ticks - previous[1], hertz, elapsed)
        rows.append({
            "pid": identity[0], "tid": identity[1], "thread_name": name,
            "cpu_percent_one_core": round(percent, 3),
        })
    rows.sort(key=lambda row: row["cpu_percent_one_core"], reverse=True)
    return rows[:8]


def run_inner(args: argparse.Namespace, work: Path) -> dict[str, Any]:
    env = smoke.isolated_environment(work)
    for name in ("DBUS_SESSION_BUS_ADDRESS", "DBUS_SESSION_BUS_PID"):
        if name in os.environ:
            env[name] = os.environ[name]
    smoke.refuse_live_environment(env)
    sway = smoke.NestedSway(work, env)
    try:
        app = next(row for row in SAMPLE_APPS if row[1] == args.app)
        display, package, _app_id = app
        binary = args.binary_dir / package
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError(f"installed executable unavailable: {package}")
        binary = binary.resolve(strict=True)
        provenance = binary_provenance(binary)

        # Reuse the startup smoke's packaged entry-point arguments and app IDs.
        spec = launch_spec(_app_id, package)
        fixture_dir = work / "fixtures"
        smoke.create_fixtures(fixture_dir)

        app_root = work / "app"
        ready = app_root / "ready"
        app_root.mkdir()
        app_env = app_environment(env, app_root, ready)
        startup_started = time.monotonic()
        process = subprocess.Popen(
            launch_command(binary, spec, fixture_dir), cwd=app_env["HOME"], env=app_env,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True,
        )
        try:
            startup_ms, startup_marker = wait_ready(
                process, sway, ready, startup_started, args.startup_timeout, _app_id
            )
            time.sleep(args.settle_seconds)
            hertz = os.sysconf("SC_CLK_TCK")
            before = budgets.sample_process_tree(process.pid, hertz)
            if before is None:
                raise RuntimeError("app process exited before idle sampling")
            before_threads = (
                thread_tick_samples(budgets.build_descendant_set(process.pid))
                if args.thread_breakdown else {}
            )
            sample_started = time.monotonic()
            time.sleep(args.idle_seconds)
            elapsed = time.monotonic() - sample_started
            after = budgets.sample_process_tree(process.pid, hertz)
            if after is None:
                raise RuntimeError("app process exited during idle sampling")
            after_threads = (
                thread_tick_samples(budgets.build_descendant_set(process.pid))
                if args.thread_breakdown else {}
            )
            cpu = budgets.cpu_percent_from_ticks(
                after["cpu_ticks"] - before["cpu_ticks"], hertz, elapsed
            )
            switches_before = before["voluntary_ctxt_switches"] + before["nonvoluntary_ctxt_switches"]
            switches_after = after["voluntary_ctxt_switches"] + after["nonvoluntary_ctxt_switches"]
            idle = {
                "seconds": round(elapsed, 3),
                "cpu_percent_one_core": round(cpu, 3),
                "context_switches_per_second": round((switches_after - switches_before) / elapsed, 3),
                "pss_mib": round(after["pss_kib"] / 1024, 1),
                "process_count": after["process_count"],
            }
            if args.thread_breakdown:
                idle["top_threads"] = busiest_threads(before_threads, after_threads, elapsed, hertz)
            return {
                "schema_version": 1,
                "scope": "single_installed_app_private_nested_session",
                "app": package,
                "display_name": display,
                "binary": package,
                "binary_provenance": provenance,
                "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "startup": {
                    "elapsed_ms": round(startup_ms, 1),
                    "measured_from": "immediately_before_spawn",
                    "marker": startup_marker,
                },
                "idle": idle,
                "limits": [
                    "No input was injected; function, navigation, and control behavior are untested.",
                    "No screenshot or pixel comparison was performed; visual quality and feel are untested.",
                    "Nested Sway and software rendering differ from the installed Lulo desktop.",
                    "Context switches are a wakeup proxy, not a count of rendered frames.",
                    "One launch and a short idle interval are exploratory samples, not a percentile or release gate.",
                    *(["Thread CPU percentages are approximate and omit threads not present at both endpoints."]
                      if args.thread_breakdown else []),
                ],
            }
        finally:
            smoke._terminate_group(process)
    finally:
        sway.close()


def main() -> int:
    args = args_parser()
    if sys.platform != "linux":
        raise SystemExit("this sampler requires Linux")
    for executable in ("sway", "swaymsg", "dbus-run-session"):
        if shutil.which(executable) is None:
            raise SystemExit(f"{executable} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-private-perf-"))
    env = smoke.isolated_environment(work)
    smoke.refuse_live_environment(env)
    command = ["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
               "--app", args.app, "--idle-seconds", str(args.idle_seconds),
               "--settle-seconds", str(args.settle_seconds), "--startup-timeout", str(args.startup_timeout),
               "--binary-dir", str(args.binary_dir.resolve()), "--_inner"]
    if args.thread_breakdown:
        command.append("--thread-breakdown")
    try:
        stdout_path = work / "inner.stdout"
        stderr_path = work / "inner.stderr"
        # D-Bus-activated services can outlive dbus-run-session and inherit
        # its output descriptors. Redirect to regular files so these children
        # cannot keep subprocess.run waiting for pipe EOF after the session
        # leader has exited.
        with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
            result = subprocess.run(
                command, env=env, stdout=stdout, stderr=stderr, text=True,
                timeout=args.startup_timeout + args.idle_seconds + 90,
            )
        inner_stdout = stdout_path.read_text()
        if result.returncode:
            excerpt = "\n".join(stderr_path.read_text().strip().splitlines()[-8:])
            raise RuntimeError(excerpt or "private sampling session failed")
        report = json.loads(inner_stdout.strip().splitlines()[-1])
        rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered)
        print(rendered, end="")
        return 0
    finally:
        smoke.reap_private_processes(work / "runtime")
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    if "--_inner" in sys.argv:
        parser = argparse.ArgumentParser()
        parser.add_argument("--app", required=True)
        parser.add_argument("--idle-seconds", type=float, required=True)
        parser.add_argument("--settle-seconds", type=float, required=True)
        parser.add_argument("--startup-timeout", type=float, required=True)
        parser.add_argument("--binary-dir", type=Path, required=True)
        parser.add_argument("--thread-breakdown", action="store_true")
        parser.add_argument("--_inner", action="store_true")
        inner_args = parser.parse_args()
        try:
            print(json.dumps(run_inner(inner_args, Path(os.environ["XDG_RUNTIME_DIR"]).parent)))
        except Exception as error:
            print(f"private performance sampler: {error}", file=sys.stderr)
            raise SystemExit(1)
    else:
        try:
            raise SystemExit(main())
        except (RuntimeError, subprocess.SubprocessError) as error:
            print(f"private performance sampler: {error}", file=sys.stderr)
            raise SystemExit(1)
