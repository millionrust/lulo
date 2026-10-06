#!/usr/bin/env python3
"""Tall top-bar menus on small and fractional-scale outputs, in a nested niri.

    python3 scripts/behavior/run_menu_scroll.py --bin-dir DIR [--app-dir DIR]
        [--niri PATH] [--shots DIR] [--keep]

--bin-dir must hold this branch's `top-bar`; `wallpaper` and `rmac-files`
are taken from --bin-dir or else --app-dir (default /usr/libexec/rmac and
/usr/bin). Each case starts its own private session (a private
dbus-run-session, temporary HOME and XDG dirs, a headless Sway hosting
niri), opens Files' File menu with a real click and checks, through the
private AT-SPI tree and screenshots:

  - fits (1920x1080 at scale 1.25, the reference laptop: logical 1536x864):
    every row of the File menu is inside the panel, the panel stays on
    screen, and the material backdrop ends with the panel (no light slab
    under it);
  - scrolls (1280x720 at scale 1.25, logical 1024x576): the panel stops
    5 points above the screen's bottom with rows hidden below it; Up from
    no selection highlights the last row and scrolls it into view; the
    mouse wheel scrolls back; resting on the top arrow scrolls to the top.

Both cases run in light appearance; --dark-too also captures the fitting
case in dark appearance for comparison. Screenshots go to --shots (kept
local, never committed). No input reaches anything but this run's own Sway.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageChops

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import run_menu_dismiss as base  # noqa: E402
import wlinput  # noqa: E402

CASES = {
    "fits": (1920, 1080, 1.25),
    "scrolls": (1280, 720, 1.25),
}
SCREEN_MARGIN = 5
ARROW = 19


class ScrollRun(base.Run):
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        super().__init__(args, work)
        self.width, self.height, self.scale = CASES[args.case]
        self.logical_h = round(self.height / self.scale)
        self.margin = SCREEN_MARGIN * self.scale
        self.arrow = ARROW * self.scale
        self.shots = Path(args.shots) if args.shots else None

    def bin(self, name: str) -> str:
        for directory in (self.args.bin_dir, *self.args.app_dir):
            for candidate in (name, f"rmac-{name}"):
                path = Path(directory) / candidate
                if path.exists():
                    return str(path)
        raise SystemExit(f"{name} not found")

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        theme = Path(self.env["XDG_CONFIG_HOME"]) / "rmac" / "theme.json"
        theme.parent.mkdir(parents=True, exist_ok=True)
        theme.write_text(json.dumps({"version": 1, "preferences": {
            "color_scheme": self.args.appearance}}), encoding="utf-8")
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             f"output HEADLESS-1 mode {self.width}x{self.height} position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise SystemExit("sway did not start")
        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        config = self.out / "niri.kdl"
        config.write_text(shell + f'\noutput "winit" {{\n    scale {self.scale}\n}}\n')
        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})
        vk = {"VK_ICD_FILENAMES": base.LAVAPIPE}
        self.spawn([self.bin("wallpaper")], "wallpaper", vk)
        self.top_bar = self.spawn([str(Path(self.args.bin_dir) / "top-bar")], "top-bar", vk)
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(3)
        self.spawn([self.bin("files")], "files", vk)
        time.sleep(5)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    # GPUI reports AT-SPI extents in output pixels, which are also the host
    # Sway's pixels, so everything below works in pixels.
    def click_at(self, x: float, y: float, button: str = "left") -> bool:
        self.point(x, y)
        self.keys.button(True, button)
        time.sleep(0.03)
        self.keys.button(False, button)
        time.sleep(0.2)
        return True

    def point(self, x: float, y: float) -> None:
        self.keys.move(x, y, self.width, self.height)
        time.sleep(0.3)

    def shot(self, name: str) -> Image.Image:
        image = self.capture(name)
        if self.shots:
            self.shots.mkdir(parents=True, exist_ok=True)
            image.save(self.shots / f"{self.args.case}-{self.args.appearance}-{name}.png")
        return image

    def menu_items(self, menu) -> list:
        found, stack = [], [menu]
        while stack:
            node = stack.pop()
            try:
                for index in range(node.childCount):
                    child = node.getChildAtIndex(index)
                    if child is None:
                        continue
                    if child.getRoleName() == "menu item":
                        found.append(child)
                    else:
                        stack.append(child)
            except Exception:  # noqa: BLE001
                continue
        boxes = [box for box in (self.extents(item) for item in found) if box]
        return sorted(boxes, key=lambda box: box[1])

    def run(self) -> int:
        self.start()
        outputs = self.niri("outputs") or {}
        logical = next(iter(outputs.values()), {}).get("logical") or {}
        self.check("niri output has the case's logical size",
                   logical.get("height") == self.logical_h and logical.get("scale") == self.scale,
                   f"logical={logical}")
        files = self.wait_for(lambda: any("files" in (w.get("app_id") or "").lower()
                                          for w in self.niri("windows") or []), 20)
        self.check("Files is open", files)
        closed = self.shot("closed")
        button = self.wait_for(lambda: self.find_button("File menu"), 20, 0.5)
        self.check("Files' File menu title is in the bar", button is not None)
        if button is None:
            return self.finish()
        opened = self.retry_until(lambda: self.click_node(button),
                                  lambda: self.find_menu("File menu") is not None)
        self.check("File menu opens", opened, f"button={self.extents(button)}")
        if not opened:
            return self.finish()
        time.sleep(1.0)
        menu = self.find_menu("File menu")
        panel = self.extents(menu)
        items = self.menu_items(menu)
        self.check("File menu has rows", len(items) >= 10, f"rows={len(items)}")
        if not panel or not items:
            return self.finish()
        x, y, w, h = panel
        bottom = y + h
        self.check("panel stays 5 pt above the screen bottom",
                   bottom <= self.height - self.margin + 2, f"panel={panel} screen={self.height}")
        opened_shot = self.shot("open")
        last = items[-1]
        if self.args.case == "fits":
            self.check("every row is inside the panel",
                       all(row[1] >= y - 2 and row[1] + row[3] <= bottom + 2 for row in items),
                       f"panel={panel} last={last}")
            # The last row's label is really painted. When the bar's surface
            # was too short, the rows stopped part-way and only the plain
            # material backdrop (a flat light slab) showed where they belong.
            s = self.scale
            label = opened_shot.crop((int(x + 20 * s), int(last[1]),
                                      int(x + w / 2), int(last[1] + last[3]))).convert("L")
            values = list(label.getdata())
            spread = max(values) - min(values) if values else 0
            self.check("the last row is drawn", spread >= 80, f"spread={spread} row={last}")
            # Beside the panel's left edge, below the bar, nothing changes:
            # the backdrop is no wider or taller than the panel.
            beside = (max(0, int(x - 60 * s)), int(y + h / 2), max(1, int(x - 45 * s)), int(bottom - 20 * s))
            if beside[2] > beside[0]:
                changed = self.changed_pixels(closed, opened_shot, beside)
                self.check("nothing is drawn beside the panel", changed < 50, f"changed={changed} box={beside}")
            # The panel's material is near-white in light appearance.
            if self.args.appearance == "light":
                inner = opened_shot.crop((int(x + w - 12 * s), int(y + h / 2),
                                          int(x + w - 6 * s), int(y + h / 2 + 40 * s)))
                pixels = list(inner.getdata())
                mean = sum(sum(p) for p in pixels) / (3 * max(1, len(pixels)))
                self.check("light material is bright", mean >= 200, f"mean={mean:.0f}")
            self.keys.key("escape")
            return self.finish()

        hidden = [row for row in items if row[1] + row[3] > bottom + 2]
        self.check("rows are hidden below the panel", bool(hidden), f"panel={panel} last={last}")
        first_top = items[0][1]
        # Up with nothing highlighted highlights the last row: it scrolls
        # into view and the first rows go under the top arrow.
        self.keys.key("up")
        time.sleep(0.6)
        items = self.menu_items(self.find_menu("File menu"))
        self.check("Up scrolls the last row into view",
                   items and items[-1][1] + items[-1][3] <= bottom + 2, f"last={items[-1] if items else None}")
        self.check("the first rows scroll away", items and items[0][1] < first_top - self.arrow,
                   f"first={items[0] if items else None}")
        self.shot("scrolled-end")
        # The wheel scrolls back up a little.
        self.point(x + w / 2, y + h / 2)
        scrolled = items[0][1] if items else 0
        for _ in range(3):
            self.keys.scroll(-1)
            time.sleep(0.1)
        time.sleep(0.5)
        items = self.menu_items(self.find_menu("File menu"))
        self.check("the wheel scrolls the rows back", items and items[0][1] > scrolled,
                   f"before={scrolled} after={items[0] if items else None}")
        # Resting on the top arrow scrolls to the very top, then stops.
        self.point(x + w / 2, y + self.arrow / 2)
        time.sleep(2.5)
        items = self.menu_items(self.find_menu("File menu"))
        self.check("resting on the top arrow scrolls to the top",
                   items and abs(items[0][1] - first_top) <= 2,
                   f"first={items[0] if items else None} want={first_top}")
        self.shot("scrolled-top")
        self.keys.key("escape")
        return self.finish()


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", "busctl"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-menu-scroll-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)
        services = work / "dbus-services"
        services.mkdir()
        bus = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
        if bus.exists():
            (services / bus.name).write_text(bus.read_text())
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        with open(work / "logs" / "session.log", "w") as log:
            result = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if result:
            for name in ("top-bar", "files", "session"):
                log = work / "logs" / f"{name}.log"
                if log.exists():
                    print(f"{name} log tail:\n{log.read_text(errors='replace')[-1500:]}")
        return result
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True, help="directory with this branch's top-bar")
    parser.add_argument("--app-dir", action="append", default=[],
                        help="fallback directories for wallpaper and rmac-files (repeatable)")
    parser.add_argument("--shots", help="directory for screenshots (kept local)")
    parser.add_argument("--dark-too", action="store_true")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--case", choices=tuple(CASES), help=argparse.SUPPRESS)
    parser.add_argument("--appearance", choices=("light", "dark"), default="light",
                        help=argparse.SUPPRESS)
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.app_dir:
        args.app_dir = ["/usr/libexec/rmac", "/usr/bin"]
    if args.inner:
        return ScrollRun(args, args.inner).run()
    base_argv = [a for a in sys.argv[1:] if a not in ("--keep", "--dark-too")]
    runs = [("fits", "light"), ("scrolls", "light")]
    if args.dark_too:
        runs.append(("fits", "dark"))
    failed = 0
    for case, appearance in runs:
        print(f"== {case} ({appearance})", flush=True)
        failed |= outer(args, [*base_argv, "--case", case, "--appearance", appearance])
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
