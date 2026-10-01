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


def accessible_point(pid: int, label: str, *, leading: bool = False) -> tuple[float, float]:
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
                    x = box[0] + (min(45, box[2] / 2) if leading else box[2] / 2)
                    return x, box[1] + box[3] / 2
    raise RuntimeError(f"no accessible {label!r} for pid {pid}")


def file_point(run: run_window_move.Run, pid: int, name: str) -> tuple[float, float]:
    x, y = accessible_point(pid, name, leading=True)
    window = run.window("org.rmac.Files")
    if window is None:
        raise RuntimeError("Files window disappeared")
    left, top, _, _ = run.geometry(window)
    # GPUI's 12 px client frame is included in AT-SPI window coordinates,
    # while niri's layout position starts at the outer edge of that frame.
    return x + left - 12, y + top - 12


def sidebar_point(run: run_window_move.Run, pid: int, label: str) -> tuple[float, float]:
    """Find a named row in the sidebar rather than an item with the same name."""
    run_lulo.pump()
    desktop = run_lulo.atspi().Registry.getDesktop(0)
    for index in range(desktop.childCount):
        app = desktop.getChildAtIndex(index)
        if app is None or app.get_process_id() != pid:
            continue
        for node in run_lulo.descendants(app):
            if run_lulo.name(node) != label or run_lulo.role(node) != "list item":
                continue
            box = run_lulo.extents(node)
            if box and box[0] < 220 and box[2] > 0:
                window = run.window("org.rmac.Files")
                left, top, _, _ = run.geometry(window)
                return left + box[0] + 45 - 12, top + box[1] + box[3] / 2 - 12
    raise RuntimeError(f"no sidebar row {label!r} for pid {pid}")


def saved_favourites(run: run_window_move.Run) -> list[str]:
    path = Path(run.env["XDG_STATE_HOME"]) / "rmac/files/favourites.json"
    if not path.exists():
        return []
    document = json.loads(path.read_text())
    if isinstance(document, list):
        return document
    if document.get("version") != 1:
        raise RuntimeError(f"unexpected sidebar version: {document.get('version')}")
    return document["paths"]


def saved_favourite_order(run: run_window_move.Run) -> list[str]:
    path = Path(run.env["XDG_STATE_HOME"]) / "rmac/files/favourites.json"
    document = json.loads(path.read_text())
    return [item["value"] for item in document.get("order", [])
            if item["kind"] == "path"]


def drag_once(run: run_window_move.Run, pid: int, name: str,
              destination: tuple[float, float], *, steps: int = 20,
              delay: float = .12, hold: float = .8) -> None:
    start = file_point(run, pid, name)
    drag_points(run, start, destination, steps=steps, delay=delay, hold=hold)


def drag_points(run: run_window_move.Run, start: tuple[float, float],
                destination: tuple[float, float], *, steps: int = 20,
                delay: float = .12, hold: float = .8) -> None:
    pointer = run.pointer
    pointer.move(*run.parent_point(*start), run.parent_width, run.parent_height)
    time.sleep(.1)
    pointer.button(True)
    for step in range(1, steps + 1):
        part = step / steps
        point = (start[0] + (destination[0] - start[0]) * part,
                 start[1] + (destination[1] - start[1]) * part)
        pointer.move(*run.parent_point(*point), run.parent_width, run.parent_height)
        time.sleep(delay)
    time.sleep(hold)
    pointer.button(False)


def files_content_point(run: run_window_move.Run) -> tuple[float, float]:
    """Use empty space in the Files content pane, above its status bar."""
    window = run.window("org.rmac.Files")
    if window is None:
        raise RuntimeError("Files window disappeared")
    left, top, width, height = run.geometry(window)
    return left + width * .75, top + height * .65


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
        accessibility = subprocess.run(
            ["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
             "org.a11y.Status", "IsEnabled", "b", "true"],
            env=run.env, capture_output=True, text=True, timeout=10,
        )
        run.check("private AT-SPI bus enabled", accessibility.returncode == 0,
                  accessibility.stderr[-200:])
        if accessibility.returncode:
            return run.finish()
        run.start()
        scenario = json.loads((run_window_move.REPO / "docs/behavior-pending/files/drag-to-dock-and-desktop.json")
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
        run.check("Dock Bin accessible", bin_point is not None, str(bin_point))
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
        files_left, files_top, files_width, files_height = run.geometry(run.window("org.rmac.Files"))
        candidates = [(run.width - 80, 120), (80, 120), (run.width - 80, run.height - 180)]
        desktop_point = next(((x, y) for x, y in candidates
                              if not (files_left <= x < files_left + files_width
                                      and files_top <= y < files_top + files_height)), None)
        if desktop_point is None:
            raise RuntimeError("no exposed Desktop point outside Files")
        drag_once(run, files.pid, desktop_source.name, desktop_point)
        desktop_destination = Path(run.env["HOME"]) / "Desktop" / desktop_source.name
        check_move(run, desktop_source, desktop_destination, "Files → Desktop moves file")

        folder = source_dir / "Drop Folder"
        folder.mkdir()
        within_source = source_dir / scenario["drags"][2]["source"]
        within_source.write_text(fixtures[within_source.name])
        run.wait_for(lambda: accessible_point(files.pid, within_source.name), 10)
        folder_point = run.wait_for(lambda: file_point(run, files.pid, folder.name), 10)
        run.check("Files folder drop target accessible", folder_point is not None)
        if folder_point is None:
            return run.finish()
        drag_once(run, files.pid, within_source.name, folder_point,
                  steps=8, delay=.04, hold=.1)
        check_move(run, within_source, folder / within_source.name,
                   "Files in-window drag moves file into folder")

        sidebar_scenario = json.loads(
            (run_window_move.REPO / "tests/behavior/files/sidebar-favourites.json").read_text()
        )
        pinned_one, pinned_two = [source_dir / name for name in sidebar_scenario["setup"]["folders"]]
        pinned_one.mkdir()
        pinned_two.mkdir()
        heading = run.wait_for(lambda: file_point(run, files.pid, "Favourites"), 10)
        run.check("Favourites drop target accessible", heading is not None)
        if heading is None:
            return run.finish()
        for target in (pinned_one, pinned_two):
            run.wait_for(lambda: file_point(run, files.pid, target.name), 10)
            drag_once(run, files.pid, target.name, heading, steps=8, delay=.04, hold=.15)
            added = run.wait_for(lambda: str(target) in saved_favourites(run), 8)
            run.check(f"Files → sidebar adds {target.name}", bool(added), str(saved_favourites(run)))
        row = run.wait_for(lambda: sidebar_point(run, files.pid, pinned_two.name), 10)
        run.check("Sidebar favourite row accessible", row is not None)
        if row is None:
            return run.finish()
        drag_points(run, row, heading, steps=8, delay=.04, hold=.15)
        reordered = run.wait_for(
            lambda: saved_favourite_order(run).index(str(pinned_two))
            < saved_favourite_order(run).index(str(pinned_one)), 8
        )
        run.check("Sidebar drag reorders favourites", bool(reordered),
                  str(saved_favourite_order(run)))

        row = sidebar_point(run, files.pid, pinned_one.name)
        pointer = run.pointer
        pointer.move(*run.parent_point(*row), run.parent_width, run.parent_height)
        pointer.button(True, "right")
        pointer.button(False, "right")
        remove_point = run.wait_for(lambda: file_point(run, files.pid, "Remove from Sidebar"), 8)
        run.check("Sidebar context menu offers Remove from Sidebar", remove_point is not None)
        if remove_point is None:
            return run.finish()
        pointer.move(*run.parent_point(*remove_point), run.parent_width, run.parent_height)
        pointer.button(True)
        pointer.button(False)
        removed = run.wait_for(lambda: str(pinned_one) not in saved_favourites(run), 8)
        run.check("Remove from Sidebar preserves the folder", bool(removed) and pinned_one.is_dir(),
                  str(saved_favourites(run)))
        second_row = sidebar_point(run, files.pid, pinned_two.name)
        drag_points(run, second_row, files_content_point(run), steps=8, delay=.04, hold=.15)
        dragged_out = run.wait_for(lambda: str(pinned_two) not in saved_favourites(run), 8)
        run.check("Dragging out of sidebar preserves the folder",
                  bool(dragged_out) and pinned_two.is_dir(), str(saved_favourites(run)))

        wallpaper = next((child for child in run.children
                          if child.args and Path(child.args[0]).name == "wallpaper"), None)
        if wallpaper is None:
            raise RuntimeError("nested wallpaper was not started")
        desktop_to_files = desktop_destination.parent / scenario["drags"][3]["source"]
        before_icon = run.capture("desktop-before-source")
        desktop_to_files.write_text(fixtures[desktop_to_files.name])
        time.sleep(.8)  # Desktop watcher debounce and one paint in private niri.
        after_icon = run.capture("desktop-source-visible")
        top_right = (run.width - 150, 35, run.width, 170)
        painted = run.changed_pixels(before_icon, after_icon, top_right)
        run.check("Desktop drag icon painted", painted > 100, f"changed pixels={painted}")
        # The isolated Desktop has default view options. Its first sorted icon
        # is at Grid(ICON_RIGHT=34, ICON_TOP=41, icon_size=64), and this file
        # sorts before the two other fixtures.
        desktop_icon = (run.width - 34 - 32, 41 + 32)
        drag_points(run, desktop_icon, files_content_point(run))
        check_move(run, desktop_to_files, source_dir / desktop_to_files.name,
                   "Desktop → Files moves file into open folder")
        # The new folder sorts into the first Desktop grid slot after the
        # source file moves into Files, so desktop_icon still points at it.
        desktop_favourite = desktop_destination.parent / "A Sidebar Desktop Folder"
        desktop_favourite.mkdir()
        time.sleep(.8)
        drag_points(run, desktop_icon, heading, steps=20, delay=.08, hold=.2)
        added_desktop = run.wait_for(
            lambda: str(desktop_favourite) in saved_favourites(run), 8
        )
        run.check("Desktop → Files sidebar adds folder without moving it",
                  bool(added_desktop) and desktop_favourite.is_dir(), str(saved_favourites(run)))
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
