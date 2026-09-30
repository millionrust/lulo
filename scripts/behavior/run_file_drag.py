#!/usr/bin/env python3
"""Drag Files items to the Dock Bin and Desktop in a private nested niri.

    python3 scripts/behavior/run_file_drag.py --bin-dir ~/rmac-wt/target/iterate
"""

from __future__ import annotations

import argparse
import fcntl
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import run_lulo
import run_window_move


def accessible_point(pid: int, label: str) -> tuple[float, float]:
    """Return an element's screen point within its Wayland surface."""
    run_lulo.pump()
    desktop = run_lulo.atspi().Registry.getDesktop(0)
    for index in range(desktop.childCount):
        app = desktop.getChildAtIndex(index)
        if app is None or app.get_process_id() != pid:
            continue
        for node in run_lulo.descendants(app):
            name = run_lulo.name(node)
            if name == label or (label == "Trash" and name.startswith("Trash")):
                box = run_lulo.extents(node)
                if box and box[2] > 0 and box[3] > 0:
                    return box[0] + box[2] / 2, box[1] + box[3] / 2
    raise RuntimeError(f"no accessible {label!r} for pid {pid}")


def file_point(run: run_window_move.Run, pid: int, name: str) -> tuple[float, float]:
    x, y = accessible_point(pid, name)
    window = run.window("org.rmac.Files")
    if window is None:
        raise RuntimeError("Files window disappeared")
    rect = window["rect"]
    inner = window.get("window_rect") or {"x": 0, "y": 0}
    return x + rect["x"] + inner.get("x", 0), y + rect["y"] + inner.get("y", 0)


def drag_once(run: run_window_move.Run, pid: int, name: str,
              destination: tuple[float, float]) -> None:
    start = file_point(run, pid, name)
    run.drag(start, destination)


def check_move(run: run_window_move.Run, source: Path, destination: Path, label: str) -> None:
    moved = run.wait_for(lambda: not source.exists() and destination.exists(), 8)
    run.check(label, bool(moved), f"source={source.exists()}, destination={destination.exists()}")


def inner(args: argparse.Namespace) -> int:
    # The window-move harness already starts private Sway, nested niri, Dock,
    # wallpaper and mission-control with the shipped shell configuration.
    args.frame_only = True
    args.extra_zoom = False
    args.geometry_only = False
    run = run_window_move.Run(args, args.inner)
    try:
        run.start()
        scenario = json.loads((run_window_move.REPO / "tests/behavior/files/drag-to-dock-and-desktop.json")
                              .read_text())
        fixtures = scenario["setup"]["files"]
        source_dir = Path(run.env["HOME"]) / "Documents" / "Drag Source"
        source_dir.mkdir(parents=True)
        bin_dir = Path(args.bin_dir)
        files = run.spawn([str(bin_dir / "rmac-files"), "--path", str(source_dir)], "files")
        window = run.wait_for(lambda: run.window("org.rmac.Files"), 30)
        run.check("Files window mapped", window is not None)
        if not window:
            return run.finish()

        dock = next((child for child in run.children
                     if child.args and Path(child.args[0]).name == "dock"), None)
        if dock is None:
            raise RuntimeError("nested Dock was not started")
        bin_point = run.wait_for(lambda: accessible_point(dock.pid, "Trash"), 20)
        run.check("Dock Bin accessible", bin_point is not None)
        if bin_point is None:
            return run.finish()

        trash_source = source_dir / scenario["drags"][0]["source"]
        trash_source.write_text(fixtures[trash_source.name])
        run.wait_for(lambda: accessible_point(files.pid, trash_source.name), 10)
        drag_once(run, files.pid, trash_source.name, bin_point)
        trash_destination = Path(run.env["XDG_DATA_HOME"]) / "Trash" / "files" / trash_source.name
        check_move(run, trash_source, trash_destination, "Files → Dock Bin moves file to Trash")

        desktop_source = source_dir / scenario["drags"][1]["source"]
        desktop_source.write_text(fixtures[desktop_source.name])
        run.wait_for(lambda: accessible_point(files.pid, desktop_source.name), 10)
        files_rect = run.window("org.rmac.Files")["rect"]
        candidates = [(run.width - 80, 120), (80, 120), (run.width - 80, run.height - 180)]
        desktop_point = next(((x, y) for x, y in candidates
                              if not (files_rect["x"] <= x < files_rect["x"] + files_rect["width"]
                                      and files_rect["y"] <= y < files_rect["y"] + files_rect["height"])), None)
        if desktop_point is None:
            raise RuntimeError("no exposed Desktop point outside Files")
        drag_once(run, files.pid, desktop_source.name, desktop_point)
        desktop_destination = Path(run.env["HOME"]) / "Desktop" / desktop_source.name
        check_move(run, desktop_source, desktop_destination, "Files → Desktop moves file")
        return run.finish()
    except Exception as error:  # noqa: BLE001
        run.check("drag runner completed", False, str(error))
        return run.finish()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin-dir", type=Path)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    with open("/tmp/lulo-journey.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(tempfile.mkdtemp(prefix="lulo-file-drag-"))
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        try:
            return subprocess.call(
                ["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                 "--inner", str(work), "--niri", args.niri, "--bin-dir", str(args.bin_dir)],
                env=env,
            )
        finally:
            if run_lulo.reap(Path(env["XDG_RUNTIME_DIR"])):
                time.sleep(1)
                run_lulo.reap(Path(env["XDG_RUNTIME_DIR"]))
            if args.keep:
                print(f"kept {work}", file=sys.stderr)
            else:
                run_lulo.remove_tree(work)


if __name__ == "__main__":
    sys.exit(main())
