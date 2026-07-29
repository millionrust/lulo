#!/usr/bin/env python3
"""Measure first-frame startup, idle CPU, and idle RSS for every rmac app."""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import platform
import signal
import statistics
import subprocess
import sys
import tempfile
import time


APPS = (
    ("System Monitor", "rmac-activity-monitor", "rmac-system-monitor"),
    ("App Drawer", "rmac-app-drawer", "rmac-app-drawer"),
    ("Files", "rmac-finder", "rmac-files"),
    ("Notes", "rmac-notes", "rmac-notes"),
    ("System Settings", "rmac-system-settings", "rmac-system-settings"),
    ("Terminal", "rmac-terminal", "rmac-terminal"),
    ("Text Editor", "rmac-text-editor", "rmac-text-editor"),
)
READY_FILE_ENV = "RMAC_BENCHMARK_READY_FILE"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--settle-seconds", type=float, default=3.0)
    parser.add_argument("--idle-seconds", type=float, default=10.0)
    parser.add_argument("--startup-timeout", type=float, default=20.0)
    parser.add_argument(
        "--package",
        action="append",
        choices=[package for _, package, _ in APPS],
        dest="packages",
        help="measure only this package; may be repeated",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/baselines/latest.json"),
    )
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if args.warmups < 0:
        parser.error("--warmups must be zero or greater")
    for field in ("repetitions", "settle_seconds", "idle_seconds", "startup_timeout"):
        if getattr(args, field) <= 0:
            parser.error(f"--{field.replace('_', '-')} must be greater than zero")
    return args


def build_apps(repo: Path, apps: tuple[tuple[str, str, str], ...]) -> None:
    command = ["cargo", "build", "--release", "--locked"]
    for _, package, _ in apps:
        command.extend(("--package", package))
    subprocess.run(command, cwd=repo, check=True)


def start_app(binary: Path, ready_file: Path) -> subprocess.Popen[bytes]:
    ready_file.unlink(missing_ok=True)
    environment = os.environ.copy()
    environment[READY_FILE_ENV] = str(ready_file)
    return subprocess.Popen(
        [binary],
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def wait_for_first_frame(
    process: subprocess.Popen[bytes], ready_file: Path, timeout: float
) -> float:
    started = time.monotonic()
    deadline = started + timeout
    while time.monotonic() < deadline:
        if ready_file.exists():
            return time.monotonic() - started
        exit_code = process.poll()
        if exit_code is not None:
            raise RuntimeError(f"process exited before its first frame: status {exit_code}")
        time.sleep(0.005)
    raise TimeoutError(f"first frame did not complete within {timeout:.1f} seconds")


def stop_app(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=3)
    except ProcessLookupError:
        return
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=3)


def cpu_seconds(value: str) -> float:
    value = value.strip()
    days = 0
    if "-" in value:
        day_text, value = value.split("-", 1)
        days = int(day_text)
    parts = value.split(":")
    if len(parts) == 2:
        hours = 0
        minutes, seconds = parts
    elif len(parts) == 3:
        hours, minutes, seconds = parts
    else:
        raise ValueError(f"unsupported ps CPU time: {value!r}")
    return days * 86_400 + int(hours) * 3_600 + int(minutes) * 60 + float(seconds)


def process_group_stats(process_group: int) -> tuple[float, int]:
    result = subprocess.run(
        ["ps", "-axo", "pgid=,time=,rss="],
        check=True,
        capture_output=True,
        text=True,
    )
    total_cpu = 0.0
    total_rss_kib = 0
    for line in result.stdout.splitlines():
        fields = line.split()
        if len(fields) != 3 or int(fields[0]) != process_group:
            continue
        total_cpu += cpu_seconds(fields[1])
        total_rss_kib += int(fields[2])
    return total_cpu, total_rss_kib


def percentile_nearest_rank(values: list[float], percentile: float) -> float:
    ordered = sorted(values)
    rank = max(1, math.ceil(percentile * len(ordered)))
    return ordered[rank - 1]


def measure_app(
    binary: Path,
    warmups: int,
    repetitions: int,
    settle_seconds: float,
    idle_seconds: float,
    startup_timeout: float,
    temp_dir: Path,
) -> dict[str, object]:
    warmup_samples: list[float] = []
    for index in range(warmups):
        ready_file = temp_dir / f"warmup-{index}.ready"
        process = start_app(binary, ready_file)
        try:
            warmup_samples.append(
                wait_for_first_frame(process, ready_file, startup_timeout)
            )
        finally:
            stop_app(process)

    startup_samples: list[float] = []
    for index in range(repetitions):
        process = start_app(binary, temp_dir / f"startup-{index}.ready")
        try:
            startup_samples.append(
                wait_for_first_frame(
                    process, temp_dir / f"startup-{index}.ready", startup_timeout
                )
            )
        finally:
            stop_app(process)

    ready_file = temp_dir / "idle.ready"
    process = start_app(binary, ready_file)
    try:
        wait_for_first_frame(process, ready_file, startup_timeout)
        time.sleep(settle_seconds)
        if process.poll() is not None:
            raise RuntimeError("process exited during the idle settling period")
        cpu_before, _ = process_group_stats(process.pid)
        idle_started = time.monotonic()
        time.sleep(idle_seconds)
        if process.poll() is not None:
            raise RuntimeError("process exited during the idle measurement")
        elapsed = time.monotonic() - idle_started
        cpu_after, rss_kib = process_group_stats(process.pid)
        if rss_kib == 0:
            raise RuntimeError("ps returned no processes in the application process group")
        idle_cpu_percent = max(0.0, cpu_after - cpu_before) / elapsed * 100
    finally:
        stop_app(process)

    return {
        "warmup_startup_ms": [round(value * 1_000, 1) for value in warmup_samples],
        "startup_ms": [round(value * 1_000, 1) for value in startup_samples],
        "startup_median_ms": round(statistics.median(startup_samples) * 1_000, 1),
        "startup_p95_ms": round(
            percentile_nearest_rank(startup_samples, 0.95) * 1_000, 1
        ),
        "idle_cpu_percent": round(idle_cpu_percent, 2),
        "idle_rss_mib": round(rss_kib / 1_024, 1),
    }


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parent.parent
    output = args.output if args.output.is_absolute() else repo / args.output
    apps = tuple(
        app for app in APPS if args.packages is None or app[1] in args.packages
    )
    if not args.skip_build:
        build_apps(repo, apps)

    results: dict[str, object] = {
        "schema_version": 1,
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=repo, text=True
        ).strip(),
        "git_dirty": bool(
            subprocess.check_output(
                ["git", "status", "--porcelain", "--untracked-files=no"],
                cwd=repo,
                text=True,
            ).strip()
        ),
        "system": {
            "os": platform.platform(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "settings": {
            "profile": "release",
            "warmups": args.warmups,
            "repetitions": args.repetitions,
            "settle_seconds": args.settle_seconds,
            "idle_seconds": args.idle_seconds,
            "startup_timeout": args.startup_timeout,
            "packages": [package for _, package, _ in apps],
        },
        "applications": {},
    }

    with tempfile.TemporaryDirectory(prefix="rmac-baseline-") as directory:
        temp_root = Path(directory)
        for display_name, package, executable in apps:
            print(f"Measuring {display_name}...", flush=True)
            app_temp = temp_root / package
            app_temp.mkdir()
            binary = repo / "target" / "release" / executable
            if not binary.is_file():
                raise FileNotFoundError(f"release binary not found: {binary}")
            results["applications"][package] = {
                "display_name": display_name,
                **measure_app(
                    binary,
                    args.warmups,
                    args.repetitions,
                    args.settle_seconds,
                    args.idle_seconds,
                    args.startup_timeout,
                    app_temp,
                ),
            }

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(results, indent=2) + "\n")
    print(f"Wrote {output}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
