#!/usr/bin/env python3
"""Large-document memory check for rmac-text-editor on the reference laptop.

Generates a plain-text document of a given size in a private temporary
directory, opens it by launching `rmac-text-editor <path>` directly (the same
command the desktop entry runs), and samples the process's VmRSS/VmHWM from
/proc while the document loads and settles. Reports, as JSON:

  * peak_rss_mib   -- VmHWM, the kernel's own high-water mark
  * shown_ms       -- launch until niri reports a window titled with the file
                      name (the title is set by the frame that renders the
                      loaded document)
  * settled_rss_mib and idle_cpu_percent over the idle window after load

and exits non-zero when the peak exceeds --budget-mib. Recovery drafts and
Recents go to a throwaway XDG_STATE_HOME, the document and that directory are
deleted afterwards, and the window is closed through niri (SIGTERM, then
SIGKILL, as fallbacks).

Shapes:
  lines        -- ~100-byte log lines (a typical large log)
  single-line  -- journey 5's large fixture: one header line then a single
                  repeating 1 KiB pattern with no newlines

UI runs share the laptop's screen with other agents, so the check takes
/tmp/lulo-journey.lock (flock, --lock-timeout) itself unless --no-lock is given.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

JOURNEY_LOCK = "/tmp/lulo-journey.lock"
SAMPLE_INTERVAL_S = 0.05
CLOCK_TICKS = os.sysconf("SC_CLK_TCK")


def make_document(path: Path, size: int, shape: str) -> None:
    with path.open("wb") as file:
        if shape == "single-line":
            header = b"rmac large-file check\n"
            pattern = b"0123456789abcdef" * 64
            file.write(header)
            written = len(header)
            while written < size:
                chunk = pattern[: size - written]
                file.write(chunk)
                written += len(chunk)
        else:
            written = 0
            index = 0
            while written < size:
                line = (
                    f"{index:09d} 2026-09-24T10:00:00Z info rmac.check "
                    f"request completed status=200 bytes={index * 7 % 99991:05d}\n"
                ).encode()
                line = line[: size - written]
                file.write(line)
                written += len(line)
                index += 1


def proc_status(pid: int) -> dict[str, int] | None:
    try:
        text = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return None
    values: dict[str, int] = {}
    for line in text.splitlines():
        key, _, rest = line.partition(":")
        if key in ("VmRSS", "VmHWM"):
            values[key] = int(rest.split()[0])  # kB
    return values


def proc_cpu_seconds(pid: int) -> float | None:
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    except OSError:
        return None
    # utime and stime are fields 14 and 15; index 11/12 after the comm split.
    return (int(fields[11]) + int(fields[12])) / CLOCK_TICKS


def niri_windows() -> list[dict[str, Any]]:
    try:
        result = subprocess.run(
            ["niri", "msg", "--json", "windows"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        windows = json.loads(result.stdout or "[]")
    except (OSError, subprocess.TimeoutExpired, json.JSONDecodeError):
        return []
    return windows if isinstance(windows, list) else []


def window_for(pid: int) -> dict[str, Any] | None:
    for window in niri_windows():
        if window.get("pid") == pid:
            return window
    return None


def close(process: subprocess.Popen[bytes]) -> None:
    window = window_for(process.pid)
    if window is not None:
        subprocess.run(
            ["niri", "msg", "action", "close-window", "--id", str(window["id"])],
            capture_output=True,
            timeout=5,
            check=False,
        )
    for sig, wait in ((None, 5.0), (signal.SIGTERM, 5.0), (signal.SIGKILL, 5.0)):
        if sig is not None and process.poll() is None:
            process.send_signal(sig)
        try:
            process.wait(timeout=wait)
            return
        except subprocess.TimeoutExpired:
            continue


def measure(binary: str, size: int, shape: str, timeout_s: float, idle_s: float) -> dict[str, Any]:
    workdir = Path(tempfile.mkdtemp(prefix="rmac-large-file-"))
    try:
        document = workdir / f"large-{shape}-{size // (1024 * 1024)}mib.txt"
        make_document(document, size, shape)
        state_home = workdir / "state"
        state_home.mkdir()
        env = dict(os.environ)
        env["XDG_STATE_HOME"] = str(state_home)
        started = time.monotonic()
        process = subprocess.Popen(
            [binary, str(document)],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        peak_kib = 0
        shown_ms: int | None = None
        next_window_poll = 0.0
        try:
            while time.monotonic() - started < timeout_s:
                status = proc_status(process.pid)
                if status is None or process.poll() is not None:
                    break
                peak_kib = max(peak_kib, status.get("VmHWM", 0), status.get("VmRSS", 0))
                now = time.monotonic()
                if shown_ms is None and now >= next_window_poll:
                    next_window_poll = now + 0.2
                    window = window_for(process.pid)
                    if window is not None and document.name in (window.get("title") or ""):
                        shown_ms = int((now - started) * 1000)
                        break
                time.sleep(SAMPLE_INTERVAL_S)
            # Let deferred work (wrapping, autosave timers) finish, then
            # measure idle CPU over a quiet window.
            settle_until = time.monotonic() + 3.0
            while time.monotonic() < settle_until and process.poll() is None:
                status = proc_status(process.pid) or {}
                peak_kib = max(peak_kib, status.get("VmHWM", 0))
                time.sleep(SAMPLE_INTERVAL_S)
            cpu_before = proc_cpu_seconds(process.pid)
            idle_started = time.monotonic()
            while time.monotonic() - idle_started < idle_s and process.poll() is None:
                status = proc_status(process.pid) or {}
                peak_kib = max(peak_kib, status.get("VmHWM", 0))
                time.sleep(0.25)
            cpu_after = proc_cpu_seconds(process.pid)
            status = proc_status(process.pid) or {}
            idle_elapsed = time.monotonic() - idle_started
            exited = process.poll()
        finally:
            close(process)
        idle_cpu = (
            round((cpu_after - cpu_before) / idle_elapsed * 100.0, 2)
            if cpu_before is not None and cpu_after is not None and idle_elapsed > 0
            else None
        )
        return {
            "shape": shape,
            "document_mib": round(size / (1024 * 1024), 2),
            "shown_ms": shown_ms,
            "peak_rss_mib": round(max(peak_kib, status.get("VmHWM", 0)) / 1024, 1),
            "settled_rss_mib": round(status.get("VmRSS", 0) / 1024, 1),
            "idle_cpu_percent": idle_cpu,
            "exited_early": exited,
        }
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", default=str(Path.home() / "rmac-dev-bin/rmac-text-editor"))
    parser.add_argument("--size-mib", type=float, default=24.0)
    parser.add_argument("--shape", choices=("lines", "single-line"), action="append")
    # An empty window idles near 105 MiB on the reference laptop; the budget
    # allows about 200 MiB for a 24 MiB document on top of that. Before the
    # long-line view the single-line fixture peaked at 1858 MiB.
    parser.add_argument("--budget-mib", type=float, default=320.0,
                        help="maximum peak RSS (VmHWM) allowed for each shape")
    parser.add_argument("--timeout", type=float, default=120.0,
                        help="seconds to wait for the document window")
    parser.add_argument("--idle", type=float, default=5.0,
                        help="seconds of idle CPU sampling after load")
    parser.add_argument("--no-lock", action="store_true")
    parser.add_argument("--lock-timeout", type=int, default=1800,
                        help="seconds to wait for the journey screen lock")
    args = parser.parse_args()

    for variable in ("WAYLAND_DISPLAY", "NIRI_SOCKET"):
        if not os.environ.get(variable):
            print(f"{variable} must be set to the live session", file=sys.stderr)
            return 2

    lock = None
    if not args.no_lock:
        lock = open(JOURNEY_LOCK, "a")
        # Block in flock(2) like the shell `flock -w` other runs use, rather
        # than polling, so this run is not starved by a busy screen.
        # (Python retries an interrupted flock, so the handler must raise.)
        def lock_timed_out(*_: Any) -> None:
            raise TimeoutError

        signal.signal(signal.SIGALRM, lock_timed_out)
        signal.alarm(args.lock_timeout)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX)
        except TimeoutError:
            print("timed out waiting for the journey screen lock", file=sys.stderr)
            return 2
        finally:
            signal.alarm(0)

    size = int(args.size_mib * 1024 * 1024)
    results = []
    failed = False
    for shape in args.shape or ["lines", "single-line"]:
        result = measure(args.binary, size, shape, args.timeout, args.idle)
        result["budget_mib"] = args.budget_mib
        result["passed"] = (
            result["shown_ms"] is not None
            and result["exited_early"] is None
            and result["peak_rss_mib"] <= args.budget_mib
        )
        failed |= not result["passed"]
        results.append(result)
        print(json.dumps(result), flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
