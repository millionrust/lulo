#!/usr/bin/env python3
"""Measure idle process memory in a disposable nested niri session.

The parent holds the journey lock. Each application is launched alone; the
sampler reads smaps_rollup and never interacts with the owner's desktop.
"""

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
import wave

from PIL import ImageChops

import run_lulo
import run_window_move

FIELDS = ("Rss", "Pss", "Pss_Anon", "Pss_File", "Private_Dirty", "SwapPss")
APPS = (
    "rmac-calculator", "rmac-clock", "rmac-files", "rmac-notes",
    "rmac-player", "rmac-preview", "rmac-system-monitor", "rmac-system-settings",
    "rmac-terminal", "rmac-text-editor", "rmac-weather",
)
SHELL = (
    ("top-bar", ()), ("wallpaper", ()), ("app-switcher", ("--service",)),
    ("rmac-launcher", ()), ("rmac-app-drawer", ("--service",)),
    ("rmac-quick-settings", ()), ("rmac-notification-center", ()),
    ("rmac-notification-center-panel", ()), ("rmac-osd", ("--service",)),
    ("screenshot", ("--service",)),
)


def sample(pid: int) -> dict:
    proc = Path("/proc") / str(pid)
    values = {}
    for line in (proc / "smaps_rollup").read_text().splitlines():
        key, sep, remainder = line.partition(":")
        if sep and key in FIELDS:
            values[key] = round(int(remainder.strip().split()[0]) / 1024, 2)
    values["threads"] = len(list((proc / "task").iterdir()))
    values["pid"] = pid
    values["executable"] = os.readlink(proc / "exe")
    return values


def inner(args: argparse.Namespace) -> int:
    run = run_window_move.Run(args, args.inner)
    if args.arenas:
        run.env["MALLOC_ARENA_MAX"] = str(args.arenas)
    icd = "/usr/share/vulkan/icd.d/" + ("intel_hasvk_icd.json" if args.gpu == "intel" else "lvp_icd.json")
    probe = subprocess.run(["vulkaninfo", "--summary"], env={**run.env, "VK_ICD_FILENAMES": icd},
                           capture_output=True, text=True, timeout=15)
    device = "Intel(R) HD Graphics" if args.gpu == "intel" else "llvmpipe"
    if probe.returncode or device not in probe.stdout:
        raise RuntimeError(f"{args.gpu} Vulkan device unavailable through {icd}")
    # run_window_move starts these two services itself.
    run.start()
    bins = Path(args.bin_dir)
    extra = {"VK_ICD_FILENAMES": icd}
    processes = {"dock": run.children[-2], "mission-control": run.children[-1]}
    if args.gpu == "intel":
        for proc in processes.values():
            proc.terminate()
            proc.wait(timeout=10)
        processes = {
            "dock": run.spawn([str(bins / "dock")], "dock-intel", extra),
            "mission-control": run.spawn([str(bins / "mission-control"), "--service"],
                                         "mission-control-intel", extra),
        }
    try:
        for name, flags in SHELL:
            binary = bins / ({"rmac-osd": "osd"}.get(name, name))
            if not binary.is_file():
                binary = Path("/usr/libexec/rmac") / (name if name.startswith("rmac-") else "rmac-" + name)
            if binary.is_file():
                processes[name] = run.spawn([str(binary), *flags], name, extra)
        time.sleep(args.settle)
        shell = {name: sample(proc.pid) for name, proc in processes.items() if proc.poll() is None}
        osd_first_use = None
        if args.exercise_osd and "rmac-osd" in shell:
            before_image = run.capture("osd-before")
            with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as sender:
                sender.sendto(
                    b'{"version":1,"kind":"display","title":"Brightness","level":42,"muted":false}',
                    str(run.runtime / "rmac/osd.sock"),
                )
            time.sleep(0.6)
            after_image = run.capture("osd-after")
            width, _ = before_image.size
            corner = (max(0, width - 400), 0, width, 300)
            changed = (ImageChops.difference(before_image.crop(corner), after_image.crop(corner))
                       .getbbox() is not None)
            osd_first_use = {"visible_change": changed, **sample(processes["rmac-osd"].pid)}
            if not changed:
                raise RuntimeError("first OSD presentation did not change the nested display")
        apps = {}
        preview_fixture = args.inner / "guide.pdf"
        preview_scenario = run_window_move.REPO / "tests/behavior/preview/find-pdf.json"
        preview_fixture.write_text(json.loads(preview_scenario.read_text())["setup"]["files"]["guide.pdf"])
        player_fixture = args.inner / "silence.wav"
        with wave.open(str(player_fixture), "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(44100)
            audio.writeframes(b"\0\0" * 4410)
        for name in APPS:
            binary = bins / name
            if not binary.is_file():
                binary = Path("/usr/bin") / name
            if not binary.is_file():
                apps[name] = {"missing": True}
                continue
            fixture = ([str(preview_fixture)] if name == "rmac-preview" else
                       [str(player_fixture)] if name == "rmac-player" else [])
            proc = run.spawn([str(binary), *fixture], name, extra)
            time.sleep(args.settle)
            if proc.poll() is None:
                apps[name] = sample(proc.pid)
                proc.terminate()
                proc.wait(timeout=10)
            else:
                apps[name] = {"exit_code": proc.returncode}
        result = {"gpu": args.gpu, "arenas": args.arenas, "shell": shell, "apps": apps,
                  "osd_first_use": osd_first_use,
                  "shell_sum": {key: round(sum(row.get(key, 0) for row in shell.values()), 2)
                                for key in FIELDS}}
        Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result, indent=2), flush=True)
        return 0
    finally:
        run.finish()


def outer(args: argparse.Namespace) -> int:
    with open("/tmp/lulo-journey.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(tempfile.mkdtemp(prefix="lulo-memory-"))
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        try:
            return subprocess.call([
                "dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                "--inner", str(work), "--bin-dir", args.bin_dir, "--niri", args.niri,
                "--gpu", args.gpu, "--arenas", str(args.arenas), "--settle", str(args.settle),
                "--output", args.output, *(["--exercise-osd"] if args.exercise_osd else []),
            ], env=env)
        finally:
            run_lulo.reap(Path(env["XDG_RUNTIME_DIR"]))
            shutil.rmtree(work, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--gpu", choices=("intel", "software"), default="software")
    parser.add_argument("--arenas", type=int, default=0, help="0 means allocator default")
    parser.add_argument("--settle", type=float, default=5)
    parser.add_argument("--exercise-osd", action="store_true")
    parser.add_argument("--output", required=True)
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.frame_only = False
    return inner(args) if args.inner else outer(args)


if __name__ == "__main__":
    sys.exit(main())
