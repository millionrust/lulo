#!/usr/bin/env python3
"""Measure first opens and shortcut-surface lifecycles in private nested niri."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time

from PIL import ImageChops

import run_lulo
import run_window_move

SURFACES = (
    ("launcher", "rmac-launcher", (), "cmd-space"),
    ("app-drawer", "rmac-app-drawer", ("--service",), None),
    ("quick-settings", "rmac-quick-settings", (), None),
    ("notification-center", "rmac-notification-center-panel", (), "cmd-ctrl-n"),
)
RESIDENT = {"launcher", "quick-settings"}


def wait_for_socket(path: Path, process: subprocess.Popen, timeout: float = 8) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and process.poll() is None:
        if path.is_socket():
            return True
        time.sleep(0.005)
    return False


def wait_for_frame(path: Path, process: subprocess.Popen, timeout: float = 5) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and process.poll() is None:
        if path.is_file():
            return True
        time.sleep(0.002)
    return False


def changed(before, after) -> bool:
    return ImageChops.difference(before, after).getbbox() is not None


def dispatch(run, bins: Path, action: str, through_niri: bool) -> bool:
    command = [str(bins / "rmac-shortcut-dispatch"), action]
    if through_niri:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(4)
            connection.connect(run.env["NIRI_SOCKET"])
            connection.sendall(json.dumps({"Action": {"Spawn": {"command": command}}}).encode() + b"\n")
            reply = json.loads(connection.makefile("rb").readline(1024 * 1024))
        return "Ok" in reply
    sent = subprocess.run(command, env=run.env, capture_output=True, text=True, timeout=4)
    return sent.returncode == 0


def inner(args: argparse.Namespace) -> int:
    args.frame_only = False
    args.fallback_shortcuts = True
    run = run_window_move.Run(args, args.inner)
    run.start()
    bins = Path(args.bin_dir)
    results = {}
    try:
        for action, binary, flags, chord in SURFACES:
            resident = action in RESIDENT
            before = None if resident else run.capture(f"{action}-before")
            frame_marker = run.work / f"{action}-first-frame.ready"
            started = time.monotonic()
            process = run.spawn([str(bins / binary), *flags], action,
                                {"RMAC_SURFACE_IDLE_SECONDS": "3",
                                 "RMAC_BENCHMARK_READY_FILE": str(frame_marker),
                                 "VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/intel_hasvk_icd.json"})
            endpoint = run.runtime / "rmac" / f"shortcut-{action}.sock"
            bound = wait_for_socket(endpoint, process)
            run.check(f"{action} cold endpoint bound", bound)
            if not bound:
                continue
            if resident:
                time.sleep(args.resident_settle)
                before = run.capture(f"{action}-before")
                started = time.monotonic()
            first_via_niri = bool(chord) and not (resident and args.resident_direct)
            sent = dispatch(run, bins, action, first_via_niri)
            run.check(f"{action} first dispatch accepted", sent)
            if not sent:
                continue
            frame_ready = wait_for_frame(frame_marker, process)
            frame_ms = round((time.monotonic() - started) * 1000, 1)
            run.check(f"{action} first frame completed", frame_ready, f"{frame_ms} ms")
            appeared = False
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                after = run.capture(f"{action}-open")
                if changed(before, after):
                    appeared = True
                    break
                time.sleep(0.02)
            elapsed_ms = round((time.monotonic() - started) * 1000, 1)
            timing = "resident first-open" if resident else "cold spawn-to-paint"
            run.check(f"{action} first request painted", appeared, f"{timing} screenshot upper bound {elapsed_ms} ms")
            results[action] = {"painted": appeared, "first_frame_ms": frame_ms if frame_ready else None,
                               "screenshot_upper_bound_ms": elapsed_ms if appeared else None,
                               "resident_at_login": resident, "via_niri_spawn": first_via_niri,
                               "resident_settle_seconds": args.resident_settle if resident else 0}
            if not appeared:
                print((run.logs / f"{action}.log").read_text()[-1200:], flush=True)
            dispatch(run, bins, action, bool(chord))
            if resident:
                time.sleep(1.2)
                run.check(f"{action} remains resident after dismissal", process.poll() is None)
            else:
                time.sleep(1.2)
                run.check(f"{action} stays alive during idle grace", process.poll() is None)
                reopened = dispatch(run, bins, action, bool(chord))
                run.check(f"{action} warm dispatch accepted", reopened)
                if reopened:
                    run.check(f"{action} warm reopen reuses process", process.poll() is None)
                    dispatch(run, bins, action, bool(chord))
                gone = run.wait_for(lambda: process.poll() is not None, timeout=6)
                run.check(f"{action} idle service exits", bool(gone))
        Path(args.output).write_text(json.dumps(results, indent=2) + "\n")
    finally:
        result = run.finish()
        if result:
            shutil.copytree(run.logs, Path(args.output).with_suffix(".logs"),
                            dirs_exist_ok=True)
    return result


def outer(args: argparse.Namespace) -> int:
    with open("/tmp/lulo-journey.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(tempfile.mkdtemp(prefix="lulo-cold-surfaces-"))
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        try:
            return subprocess.call([
                "dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                "--inner", str(work), "--bin-dir", args.bin_dir, "--niri", args.niri,
                "--output", args.output,
                "--resident-settle", str(args.resident_settle),
                *(["--resident-direct"] if args.resident_direct else []),
            ], env=env)
        finally:
            run_lulo.reap(Path(env["XDG_RUNTIME_DIR"]))
            shutil.rmtree(work, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--output", required=True)
    parser.add_argument("--resident-settle", type=float, default=0,
                        help="wait after resident endpoint readiness before first dispatch")
    parser.add_argument("--resident-direct", action="store_true",
                        help="diagnose resident dispatch without niri Spawn IPC")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    return inner(args) if args.inner else outer(args)


if __name__ == "__main__":
    sys.exit(main())
