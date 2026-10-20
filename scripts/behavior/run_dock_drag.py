#!/usr/bin/env python3
"""Keep apps in the Dock by dragging them, in a private nested niri.

    python3 scripts/behavior/run_dock_drag.py --bin-dir ~/rmac-wt/target/iterate

The scenario (tests/behavior/shell/dock-drag-pinning.md) follows macOS 26:
a running app dragged left across the separator is kept where it is
dropped; a kept app reorders by dragging; a kept app dragged off the Dock
and held is removed but stays among the running apps; an app dragged out of
Apps is kept where it is dropped. Each check reads the shell settings the
Dock saved, so it proves persistence as well as the drop.

Input goes only to this run's headless Sway parent (wlinput's guard).
"""

from __future__ import annotations

import argparse
import fcntl
import os
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import run_lulo
import run_window_move

PROBE_ID = "org.example.DockDragProbe"
PROBE_NAME = "Dock Drag Probe"
SECOND_ID = "org.example.DockDragSecond"
SECOND_NAME = "Dock Drag Second"


def probe_window() -> int:
    """A plain GTK window whose app id names the probe's desktop entry."""
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import Gtk

    app = Gtk.Application(application_id=PROBE_ID)

    def activate(app) -> None:
        window = Gtk.ApplicationWindow(application=app, title=PROBE_NAME)
        window.set_default_size(360, 240)
        window.present()

    app.connect("activate", activate)
    return app.run([])


def canonical(app_id: str) -> str:
    return app_id.removesuffix(".desktop").lower()


def pinned(run: run_window_move.Run) -> list[str] | None:
    """The kept apps in the shell settings the Dock saved, or None before
    the Dock has saved any."""
    path = Path(run.env["XDG_CONFIG_HOME"]) / "rmac" / "shell.json"
    if not path.exists():
        return None
    try:
        document = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    settings = document.get("settings", document)
    return [canonical(app) for app in settings.get("pinned_apps", [])]


def accessible_box(pid: int, label: str) -> tuple[float, float, float, float] | None:
    """The extents of `pid`'s accessible node named `label` (or `label, …`,
    as Dock tiles add their state)."""
    run_lulo.pump()
    desktop = run_lulo.atspi().Registry.getDesktop(0)
    for index in range(desktop.childCount):
        app = desktop.getChildAtIndex(index)
        if app is None or app.get_process_id() != pid:
            continue
        for node in run_lulo.descendants(app):
            name = run_lulo.name(node)
            if name == label or name.startswith(label + ","):
                box = run_lulo.extents(node)
                if box and box[2] > 0 and box[3] > 0:
                    return box
    return None


def centre(box: tuple[float, float, float, float]) -> tuple[float, float]:
    return box[0] + box[2] / 2, box[1] + box[3] / 2


def dock_centre(dock_pid: int, label: str) -> tuple[float, float] | None:
    box = accessible_box(dock_pid, label)
    return centre(box) if box else None


def drag(run: run_window_move.Run, start: tuple[float, float], end: tuple[float, float],
         hold: float = 0.4) -> None:
    """Hover, then a press, a smooth 60 Hz-ish move and a held release. The
    hover lets the surface under the pointer see it enter first, as a hand
    always does."""
    run.pointer.move(*run.parent_point(*start), run.parent_width, run.parent_height)
    time.sleep(0.5)
    run.pointer.drag(run.parent_point(*start), run.parent_point(*end),
                     run.parent_width, run.parent_height, steps=30, step_delay=0.016,
                     grab_delay=0.05, hold=hold)
    time.sleep(0.4)


def write_entries(run: run_window_move.Run) -> None:
    applications = Path(run.env["XDG_DATA_HOME"]) / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    here = Path(__file__).resolve()
    (applications / f"{PROBE_ID}.desktop").write_text(
        "[Desktop Entry]\nType=Application\n"
        f"Name={PROBE_NAME}\nExec={sys.executable} {here} --probe-window\n"
        "Icon=application-x-executable\n"
    )
    (applications / f"{SECOND_ID}.desktop").write_text(
        "[Desktop Entry]\nType=Application\n"
        f"Name={SECOND_NAME}\nExec=true\nIcon=application-x-executable\n"
    )


def material_span(image, y: int) -> tuple[int, int] | None:
    """The shelf material's left and right edge on row `y` of a capture:
    the outermost pixels that differ from the plain backdrop at the row's
    ends."""
    width = image.width
    background = image.getpixel((5, y))

    def differs(x: int) -> bool:
        pixel = image.getpixel((x, y))
        # The glass over the plain nested backdrop is only a few levels off.
        return sum(abs(a - b) for a, b in zip(pixel, background)) >= 3

    left = next((x for x in range(width) if differs(x)), None)
    right = next((x for x in range(width - 1, -1, -1) if differs(x)), None)
    return (left, right) if left is not None and right is not None else None


def drawer_size() -> tuple[float, float]:
    source = (run_window_move.REPO / "crates/app-drawer/src/view.rs").read_text()
    width = float(re.search(r"DRAWER_WIDTH: f32 = ([\d.]+)", source).group(1))
    height = float(re.search(r"DRAWER_HEIGHT: f32 = ([\d.]+)", source).group(1))
    return width, height


def inner(args: argparse.Namespace) -> int:
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
        write_entries(run)
        run.start()
        docks = [next((child for child in run.children
                       if child.args and Path(child.args[0]).name == "dock"), None)]
        if docks[0] is None:
            raise RuntimeError("nested Dock was not started")

        def dock_pid() -> int:
            return docks[-1].pid

        def restart_dock() -> None:
            """Start the Dock again, so it reads the saved settings afresh
            (persistence across a restart)."""
            docks[-1].terminate()
            docks[-1].wait(timeout=10)
            docks.append(run.spawn([str(Path(args.bin_dir) / "dock")], f"dock-{len(docks)}",
                                   {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"}))
            run.wait_for(lambda: dock_centre(dock_pid(), "Files"), 30)
            time.sleep(1.0)
        files = run.wait_for(lambda: dock_centre(dock_pid(), "Files"), 30)
        apps = run.wait_for(lambda: dock_centre(dock_pid(), "Apps"), 10)
        run.check("Dock shows Files and Apps", bool(files and apps), f"{files} {apps}")
        if not (files and apps):
            return run.finish()
        between_first_two = ((files[0] + apps[0]) / 2, files[1])

        # 1. A running app that is not kept, dragged left across the
        # separator between the first two kept apps, is kept second. The
        # pointer rests on the Dock while the probe opens and the shelf grows
        # under it: the Dock keeps its windows and still takes the drag.
        run.pointer.move(*run.parent_point(*files), run.parent_width, run.parent_height)
        time.sleep(0.5)
        run.spawn([sys.executable, str(Path(__file__).resolve()), "--probe-window"], "probe")
        probe = run.wait_for(lambda: dock_centre(dock_pid(), PROBE_NAME), 30)
        run.check("running probe app shows in the Dock", probe is not None)
        if probe is None:
            return run.finish()
        run.check("probe starts among the running apps", probe[0] > apps[0], f"{probe}")
        # The shelf grew for the probe; read the kept tiles again.
        time.sleep(1.0)
        files = dock_centre(dock_pid(), "Files")
        apps = dock_centre(dock_pid(), "Apps")
        probe = dock_centre(dock_pid(), PROBE_NAME)
        between_first_two = ((files[0] + apps[0]) / 2, files[1])
        drag(run, probe, between_first_two)
        kept = run.wait_for(lambda: (p := pinned(run)) and canonical(PROBE_ID) in p and p, 8)
        run.check("running app dragged across the separator (shelf grown under a "
                  "resting pointer) is kept second",
                  bool(kept) and kept.index(canonical(PROBE_ID)) == 1, str(kept))
        if not kept:
            return run.finish()

        # 2. Files, Probe, Apps, Notes…: at once (no wait for the Dock to
        # confirm the drop), dragging the kept probe past Apps (between Apps
        # and Notes) moves it one place right.
        probe = run.wait_for(lambda: dock_centre(dock_pid(), PROBE_NAME), 10)
        apps = dock_centre(dock_pid(), "Apps")
        notes = dock_centre(dock_pid(), "Notes")
        if probe and apps and notes:
            drag(run, probe, ((apps[0] + notes[0]) / 2, probe[1]))
            moved = run.wait_for(
                lambda: (p := pinned(run)) and canonical(PROBE_ID) in p
                and p.index(canonical(PROBE_ID)) == 2 and p, 8)
            run.check("dragging a kept app right after a drop reorders it", bool(moved),
                      str(pinned(run)))
        else:
            run.check("kept tiles visible for reorder", False, f"{probe} {apps} {notes}")

        # 3. Dragged off the Dock and held, it is removed, and as it is
        # still running it stays in the Dock's running apps.
        probe = run.wait_for(lambda: dock_centre(dock_pid(), PROBE_NAME), 10)
        if probe:
            drag(run, probe, (probe[0], probe[1] - 260), hold=1.0)
        removed = run.wait_for(
            lambda: (p := pinned(run)) is not None and canonical(PROBE_ID) not in p, 8)
        run.check("kept app dragged off the Dock is removed", bool(removed), str(pinned(run)))
        time.sleep(1.0)
        still = run.wait_for(lambda: dock_centre(dock_pid(), PROBE_NAME), 10)
        apps = dock_centre(dock_pid(), "Apps")
        run.check("removed app stays among the running apps",
                  bool(still and apps and still[0] > apps[0]), f"{still}")

        # 4. An app dragged out of Apps between the first two kept apps is
        # kept second.
        drawer = run.spawn([str(Path(args.bin_dir) / "rmac-app-drawer"), "--service", "--show"],
                           "app-drawer")
        run.wait_for(lambda: accessible_box(drawer.pid, SECOND_NAME), 30)
        # Search, so the app is the only tile and a press anywhere else
        # drags nothing.
        time.sleep(0.8)
        run.pointer.type_text(SECOND_NAME)
        time.sleep(1.0)
        tile = run.wait_for(lambda: accessible_box(drawer.pid, SECOND_NAME), 10)
        run.check("Apps shows the second app", tile is not None)
        if tile is None:
            return run.finish()
        width, height = drawer_size()
        files = dock_centre(dock_pid(), "Files")
        apps = dock_centre(dock_pid(), "Apps")
        local = centre(tile)
        # Apps is a centred layer surface; niri centres it in the area the
        # Dock leaves, or in the whole output.
        files_box = accessible_box(dock_pid(), "Files")
        dock_top = files_box[1] - 12 if files_box else run.height
        left = (run.width - width) / 2
        offsets = [(left, (dock_top - height) / 2), (left, (run.height - height) / 2)]
        kept_from_apps = None
        # The shelf's middle row, away from the material's rounded corners.
        rim_y = int(files[1])
        resting = material_span(run.capture("material-resting"), rim_y)
        widened = None
        for left, top in offsets:
            start = (left + local[0], top + local[1])
            target = ((files[0] + apps[0]) / 2, files[1])
            pointer = run.pointer
            pointer.move(*run.parent_point(*start), run.parent_width, run.parent_height)
            time.sleep(0.5)
            pointer.button(True)
            for step in range(1, 31):
                part = step / 30
                pointer.move(*run.parent_point(start[0] + (target[0] - start[0]) * part,
                                               start[1] + (target[1] - start[1]) * part),
                             run.parent_width, run.parent_height)
                time.sleep(0.016)
            time.sleep(1.5)
            widened = material_span(run.capture("material-slot-open"), rim_y)
            pointer.button(False)
            time.sleep(0.4)
            kept_from_apps = run.wait_for(
                lambda: (p := pinned(run)) and canonical(SECOND_ID) in p and p, 4)
            if kept_from_apps:
                break
            # Apps closes when it loses the keyboard; open it again.
            if drawer.poll() is not None:
                drawer = run.spawn([str(Path(args.bin_dir) / "rmac-app-drawer"),
                                    "--service", "--show"], "app-drawer-2")
                time.sleep(1.5)
                run.pointer.type_text(SECOND_NAME)
                run.wait_for(lambda: accessible_box(drawer.pid, SECOND_NAME), 20)
        run.check("app dragged out of Apps is kept where it was dropped",
                  bool(kept_from_apps) and kept_from_apps.index(canonical(SECOND_ID)) == 1,
                  str(kept_from_apps))
        grown = (widened[1] - widened[0]) - (resting[1] - resting[0]) if resting and widened else None
        run.check("the shelf's glass widens with the open slot (about one tile)",
                  grown is not None and 50 <= grown <= 90, f"resting={resting} open={widened}")

        # 5. The kept apps survive a Dock restart.
        before = pinned(run)
        restart_dock()
        run.check("kept apps persist across a Dock restart",
                  bool(before) and dock_centre(dock_pid(), SECOND_NAME) is not None
                  and pinned(run) == before, str(pinned(run)))
        return run.finish()
    except Exception as error:  # noqa: BLE001
        run.check("dock drag runner completed", False, repr(error))
        return run.finish()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin-dir", type=Path)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--probe-window", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.probe_window:
        return probe_window()
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    with open("/tmp/lulo-journey.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        work = Path(tempfile.mkdtemp(prefix="lulo-dock-drag-"))
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
