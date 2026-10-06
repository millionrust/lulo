#!/usr/bin/env python3
"""Dock icon sharpness check, in a private nested niri.

    python3 scripts/behavior/run_dock_sharpness.py --dock PATH \\
        [--shell-json PATH] [--scale 1.0] [--output DIR] [--min-sharpness N] [--keep]

Starts headless Sway as the parent display, niri nested inside it with one
output at --scale, and only the Dock from --dock. --shell-json is copied (read
only) into the private HOME, so the owner's pinned apps and tile size can be
replayed without touching their session. The Dock resolves its icons the way
an installed build does (hicolor `scalable/apps`, `/usr/share/rmac/dock`).

It captures the output with grim, finds the Dock tiles in the bottom strip
and scores each tile's sharpness as the mean absolute 3x3 Laplacian of its
luminance (higher is sharper). A Dock that draws its icons from a bitmap far
smaller than the tile (the blurry Dock of build d7fa75c9) scores well under
half of a sharp one at the same size. --min-sharpness turns the median tile
score into a pass/fail check; without it the runner only reports.

Isolation is run_lulo.py's: a private dbus-run-session, temporary HOME and
XDG_RUNTIME_DIR, `wayland-0`/`wayland-1` held so no socket is ever named like
the live session's, and the journey lock held for the run. No input is sent.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
WIDTH, HEIGHT = 1920, 1080


def tile_boxes(image, background: tuple[int, int, int]) -> list[tuple[int, int, int, int]]:
    """Square-ish runs of non-background columns in the bottom strip."""

    width, height = image.size
    top = int(height * 0.82)
    pixels = image.load()

    def differs(x: int, y: int) -> bool:
        r, g, b = pixels[x, y][:3]
        return abs(r - background[0]) + abs(g - background[1]) + abs(b - background[2]) > 24

    rows = [y for y in range(top, height) if sum(differs(x, y) for x in range(0, width, 4)) > 8]
    if not rows:
        return []
    # The Dock plate fills the strip; tiles are the columns inside it whose
    # contrast against the plate's own colour is high.
    y0, y1 = min(rows), max(rows)
    mid = (y0 + y1) // 2
    plate = pixels[next(x for x in range(width) if differs(x, mid)) + 6, mid][:3]

    def icon(x: int) -> bool:
        hits = 0
        for y in range(y0 + 4, y1 - 4, 2):
            r, g, b = pixels[x, y][:3]
            if abs(r - plate[0]) + abs(g - plate[1]) + abs(b - plate[2]) > 40:
                hits += 1
        return hits > (y1 - y0) // 6

    boxes, start = [], None
    for x in range(width):
        if icon(x):
            start = x if start is None else start
        elif start is not None:
            if x - start >= 24:
                boxes.append((start, y0, x, y1))
            start = None
    return boxes


def sharpness(image, box: tuple[int, int, int, int]) -> float:
    from PIL import ImageFilter, ImageStat

    crop = image.crop(box).convert("L")
    laplacian = crop.filter(ImageFilter.Kernel((3, 3), [0, 1, 0, 1, -4, 1, 0, 1, 0], scale=1, offset=128))
    values = [abs(v - 128) for v in laplacian.getdata()]
    return sum(values) / max(1, len(values)) if values else ImageStat.Stat(crop).stddev[0]


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.out = work / "logs"
        self.out.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        process = subprocess.Popen(argv, env={**self.env, **(extra or {})},
                                   stdout=open(self.out / f"{name}.log", "w"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    @staticmethod
    def wait_for(predicate, timeout: float = 30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.2)
        return None

    def run(self) -> int:
        try:
            return self.measure()
        finally:
            for process in reversed(self.children):
                if process.poll() is None:
                    process.terminate()
            for process in reversed(self.children):
                try:
                    process.wait(5)
                except subprocess.TimeoutExpired:
                    process.kill()

    def measure(self) -> int:
        from PIL import Image

        locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             f"output HEADLESS-1 mode {WIDTH}x{HEIGHT} position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        sway = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not sway:
            raise SystemExit("sway did not start")

        config = self.out / "niri.kdl"
        config.write_text(
            f'output "winit" {{\n    scale {self.args.scale}\n}}\n'
            "prefer-no-csd\nhotkey-overlay {\n    skip-at-startup\n}\n"
            "layout {\n    background-color \"#5a6270\"\n}\n"
        )
        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": sway, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})

        self.spawn([self.args.dock], "dock", {"VK_ICD_FILENAMES": LAVAPIPE})
        time.sleep(self.args.settle)
        shot = self.out / "dock.png"
        subprocess.run(["grim", str(shot)], env={**self.env, "WAYLAND_DISPLAY": sway}, check=True)

        image = Image.open(shot).convert("RGB")
        background = image.getpixel((8, 8))
        boxes = tile_boxes(image, background)
        scores = [round(sharpness(image, box), 2) for box in boxes]
        median = statistics.median(scores) if scores else 0.0
        report = {"scale": self.args.scale, "dock": self.args.dock, "tiles": len(boxes),
                  "boxes": boxes, "sharpness": scores, "median": median}
        print(json.dumps(report), flush=True)
        if self.args.output:
            output = Path(self.args.output)
            output.mkdir(parents=True, exist_ok=True)
            shutil.copy(shot, output / "dock.png")
            if boxes:
                left = min(b[0] for b in boxes) - 16
                right = max(b[2] for b in boxes) + 16
                image.crop((max(0, left), boxes[0][1] - 16, min(image.width, right), image.height)) \
                    .save(output / "dock-crop.png")
            (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        if not boxes:
            print("FAIL no Dock tiles found", flush=True)
            return 1
        if self.args.min_sharpness is not None and median < self.args.min_sharpness:
            print(f"FAIL median Dock tile sharpness {median} < {self.args.min_sharpness}", flush=True)
            return 1
        print(f"PASS {len(boxes)} Dock tiles, median sharpness {median}", flush=True)
        return 0


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-dock-sharpness-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)
        if args.shell_json:
            target = Path(env["XDG_CONFIG_HOME"]) / "rmac" / "shell.json"
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(args.shell_json, target)
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        with open(work / "logs" / "session.log", "w") as log:
            return subprocess.call(command, env=env, close_fds=True, stderr=log)
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)
        journey_lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--dock", required=True, help="the rmac-dock binary to measure")
    parser.add_argument("--shell-json", help="a shell.json to copy into the private HOME")
    parser.add_argument("--scale", type=float, default=1.0)
    parser.add_argument("--settle", type=float, default=12.0)
    parser.add_argument("--output", help="directory for dock.png, dock-crop.png and report.json")
    parser.add_argument("--min-sharpness", type=float)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
