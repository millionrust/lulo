#!/usr/bin/env python3
"""Drag a Files item onto Text Editor and onto Preview in a private nested niri.

    python3 scripts/behavior/run_drop_in.py --bin-dir ~/rmac-wt/target/iterate

Covers the drag-in feature (a file dropped on the window opens there):
Text Editor loads the dropped text file into its clean untitled window, and
Preview opens the dropped image in a new window. Files starts a real Wayland
drag, exactly as run_file_drag.py does for the Dock and Desktop.
"""

from __future__ import annotations

import argparse
import fcntl
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

import run_file_drag as drag
import run_lulo
import run_window_move


def tiny_png(path: Path) -> None:
    """A valid 8x8 grey PNG, written without any imaging library."""
    width = height = 8
    raw = b"".join(b"\x00" + b"\x80" * width for _ in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                     + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def exposed_point(run: run_window_move.Run, target_app: str) -> tuple[float, float]:
    """A point inside the target window that the Files window does not cover."""
    target = run.window(target_app)
    files = run.window("org.rmac.Files")
    if target is None or files is None:
        raise RuntimeError("window disappeared")
    left, top, width, height = run.geometry(target)
    f_left, f_top, f_width, f_height = run.geometry(files)
    for fx, fy in ((.5, .5), (.8, .6), (.2, .6), (.5, .85), (.5, .2)):
        x, y = left + width * fx, top + height * fy
        if not (f_left <= x < f_left + f_width and f_top <= y < f_top + f_height):
            return x, y
    raise RuntimeError(f"no exposed point of {target_app} outside Files")


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
        run.start()
        bin_dir = Path(args.bin_dir)
        source_dir = Path(run.env["HOME"]) / "Documents" / "Drop Source"
        source_dir.mkdir(parents=True)
        text_file = source_dir / "dropped-note.txt"
        text_file.write_text("Dropped from Files\n")
        image_file = source_dir / "dropped-image.png"
        tiny_png(image_file)

        files = run.spawn([str(bin_dir / "rmac-files"), "--path", str(source_dir)], "files")
        run.check("Files window mapped", bool(run.wait_for(lambda: run.window("org.rmac.Files"), 30)))
        run.spawn([str(bin_dir / "rmac-text-editor")], "text-editor")
        editor = run.wait_for(lambda: run.window("org.rmac.TextEditor"), 30)
        run.check("Text Editor window mapped", editor is not None)
        run.spawn([str(bin_dir / "rmac-preview")], "preview")
        preview_before = run.wait_for(lambda: run.window("org.rmac.Preview"), 30)
        run.check("Preview window mapped", preview_before is not None)
        if not (editor and preview_before):
            return run.finish()

        run.wait_for(lambda: drag.accessible_point(files.pid, text_file.name), 10)
        drag.drag_points(run, drag.file_point(run, files.pid, text_file.name),
                         exposed_point(run, "org.rmac.TextEditor"))
        loaded = run.wait_for(
            lambda: text_file.name in ((run.window("org.rmac.TextEditor") or {}).get("title") or ""), 10)
        run.check("Files → Text Editor drop opens the file", bool(loaded),
                  str((run.window("org.rmac.TextEditor") or {}).get("title")))

        windows_before = len([w for w in run.windows() if w.get("app_id") == "org.rmac.Preview"])
        run.wait_for(lambda: drag.accessible_point(files.pid, image_file.name), 10)
        drag.drag_points(run, drag.file_point(run, files.pid, image_file.name),
                         exposed_point(run, "org.rmac.Preview"))
        opened = run.wait_for(
            lambda: len([w for w in run.windows() if w.get("app_id") == "org.rmac.Preview"])
            > windows_before or any(image_file.name in (w.get("title") or "")
                                    for w in run.windows()
                                    if w.get("app_id") == "org.rmac.Preview"), 10)
        run.check("Files → Preview drop opens the image", bool(opened),
                  str([w.get("title") for w in run.windows()
                       if w.get("app_id") == "org.rmac.Preview"]))
        return run.finish()
    except Exception as error:  # noqa: BLE001
        run.check("drop-in runner completed", False, str(error))
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
        work = Path(tempfile.mkdtemp(prefix="lulo-drop-in-"))
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
