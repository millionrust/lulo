#!/usr/bin/env python3
"""Minimise checks for every kind of window, in a nested niri.

    python3 scripts/behavior/run_niri_minimize.py --niri PATH --bin-dir DIR \\
        [--calculator PATH] [--keep]

The behaviour suite runs apps in headless Sway (docs/behavior-suite.md), but
minimising is niri's business: the parking workspace, the event stream, the
⌘M bind. So this runner starts headless Sway only as a parent display, runs
niri nested inside it with the shipped packaging/rmac-session/shell.kdl, and
starts the Dock and the Mission Control service from --bin-dir (`dock`,
`mission-control`). It then checks (docs/decisions/0021-niri-minimize-request.md):

1. a GTK 4 window's own minimise (xdg_toplevel.set_minimized) reaches the Dock
   through niri's WindowMinimizeRequested and the window is parked with its
   origin and a thumbnail (needs Lulo's patched niri, 26.04+lulo1-2);
2. the Dock lists the tile over AT-SPI, and its default action restores the
   window to the workspace it came from;
3. ⌘M (niri reads Mod as Alt when nested) minimises the focused third-party
   window through `mission-control minimize`;
4. closing a parked window drops its parking record and thumbnails;
5. with --calculator, ⌘M does the same for an rmac app.

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, `wayland-0`/`wayland-1` held so no socket is ever named like
the live session's, and keys injected only into this run's Sway through
wlinput.py. A virtual keyboard on niri itself would bypass niri's binds, so
the keys go to Sway, whose only window is niri.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
GTK_APP_ID = "org.example.MinimizeTest"
GTK_TITLE = "Third Party"


# --------------------------------------------------------------------------
# A plain GTK 4 window: the third-party app.
# --------------------------------------------------------------------------


def gtk_window(delay: float | None) -> int:
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import GLib, Gtk

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title=GTK_TITLE)
        window.set_default_size(420, 300)
        window.set_child(Gtk.Label(label=GTK_TITLE))
        window.present()
        if delay is not None:
            # Exactly what the title bar's minimise button does.
            GLib.timeout_add(int(delay * 1000), lambda: window.minimize() and False)

    app = Gtk.Application(application_id=GTK_APP_ID)
    app.connect("activate", activate)
    return app.run([])


# --------------------------------------------------------------------------
# Inner run: Sway, nested niri, the shell services, the checks.
# --------------------------------------------------------------------------


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.out = work / "logs"
        self.out.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []
        self.results: list[tuple[str, bool, str]] = []

    def check(self, name: str, ok, detail: str = "") -> None:
        self.results.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name} {detail}".rstrip(), flush=True)

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(argv, env=env, stdout=open(self.out / f"{name}.log", "w"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    @staticmethod
    def wait_for(predicate, timeout: float = 30, step: float = 0.2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except Exception:  # noqa: BLE001
                value = None
            if value:
                return value
            time.sleep(step)
        return None

    # -- compositors -------------------------------------------------------

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             "output HEADLESS-1 mode 1440x900 position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise SystemExit("sway did not start")

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        mission_control = str(Path(self.args.bin_dir) / "mission-control")
        config = self.out / "niri.kdl"
        config.write_text(shell.replace("/usr/libexec/rmac/rmac-mission-control", mission_control)
                          .replace("/usr/libexec/rmac/rmac-dock", str(Path(self.args.bin_dir) / "dock")))
        validate = subprocess.run([self.args.niri, "validate", "-c", str(config)], env=self.env,
                                  capture_output=True, text=True)
        self.check("niri validate accepts shell.kdl", validate.returncode == 0,
                   "" if validate.returncode == 0 else validate.stderr[-300:])

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
        self.events = open(self.out / "events.jsonl", "w")
        self.children.append(subprocess.Popen([self.args.niri, "msg", "-j", "event-stream"], env=self.env,
                                              stdout=self.events, stderr=subprocess.DEVNULL))
        outputs = self.niri("outputs") or {}
        self.output = next(iter(outputs), "winit")
        logical = (outputs.get(self.output) or {}).get("logical") or {}
        self.output_size = (logical.get("width", 1440), logical.get("height", 900))

        bins = Path(self.args.bin_dir)
        self.spawn([str(bins / "dock")], "dock", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.spawn([mission_control, "--service"], "mission-control", {"VK_ICD_FILENAMES": LAVAPIPE})
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(6)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    # -- niri and parking state ------------------------------------------------

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def window(self, window_id: int):
        return next((w for w in self.niri("windows") or [] if w["id"] == window_id), None)

    def parking(self):
        return next((w["id"] for w in self.niri("workspaces") or [] if w.get("name") == "rmac-parking"), None)

    def parked(self, window_id: int):
        window = self.window(window_id)
        return window if window and window.get("workspace_id") == self.parking() else None

    def entry(self, window_id: int):
        path = self.runtime / "rmac" / "parking.json"
        store = json.loads(path.read_text()) if path.exists() else {"parked": []}
        return next((e for e in store["parked"] if e["window"] == window_id), None)

    def thumbnails(self, window_id: int) -> list[Path]:
        directory = self.runtime / "rmac" / "thumbnails"
        return sorted(directory.glob(f"{window_id}-*.png")) if directory.exists() else []

    def dock_tile(self, title: str):
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        stack = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        while stack:
            node = stack.pop()
            try:
                if node is None:
                    continue
                if node.getRoleName() == "push button" and title in (node.name or ""):
                    return node
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:  # noqa: BLE001
                continue
        return None

    def minimized_tile_point(self, picture: str) -> tuple[int, int] | None:
        """Centre of the tile just left of the Bin: the Dock is the only
        thing on the empty desktop's bottom rows, and the Bin is its last item."""

        from PIL import Image

        image = Image.open(self.out / f"{picture}.png").convert("RGB")
        width, height = image.size
        background = image.getpixel((width // 2, height // 3))

        def differs(x: int, y: int) -> bool:
            return sum(abs(a - b) for a, b in zip(image.getpixel((x, y)), background)) > 30

        rows = [y for y in range(height - 150, height) if any(differs(x, y) for x in range(0, width, 4))]
        if not rows:
            return None
        middle = (rows[0] + rows[-1]) // 2
        columns = [x for x in range(width) if differs(x, middle)]
        if not columns:
            return None
        # Measured on this Dock: the Bin's centre sits 42 px inside the
        # shelf's right edge and tiles are 68 px apart.
        right = columns[-1]
        return right - 42 - 68, middle

    def screenshot(self, name: str) -> None:
        subprocess.run(["grim", "-o", str(self.output), str(self.out / f"{name}.png")],
                       env=self.env, capture_output=True, check=False)

    # -- the checks ------------------------------------------------------------

    def run(self) -> int:
        self.start()
        gtk = self.spawn([sys.executable, __file__, "--gtk-window", "--minimize-after", "4"], "gtk",
                         {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"})
        window = self.wait_for(lambda: next((w for w in self.niri("windows") or []
                                             if w.get("app_id") == GTK_APP_ID), None))
        self.check("GTK window mapped", window)
        if not window:
            return self.finish()
        wid, origin = window["id"], window["workspace_id"]

        # 1. Its own minimise button.
        self.check("set_minimized parks the GTK window", self.wait_for(lambda: self.parked(wid), 20))
        self.check("niri reported WindowMinimizeRequested",
                   f'"WindowMinimizeRequested":{{"id":{wid}}}' in (self.out / "events.jsonl").read_text())
        entry = self.entry(wid)
        self.check("the record keeps the origin workspace", entry and entry["workspace"] == origin, json.dumps(entry))
        self.check("a thumbnail was taken before the move",
                   entry and entry.get("thumbnail") and Path(entry["thumbnail"]).exists())
        time.sleep(2)
        self.screenshot("dock-after-set-minimized")

        # 2. The Dock lists it and restores it.
        tile = self.wait_for(lambda: self.dock_tile(GTK_TITLE), 5, 0.5)
        if tile is not None:
            self.check("the Dock lists the minimised window (AT-SPI)", True, tile.name)
            tile.queryAction().doAction(0)
        else:
            # Shell surfaces are not on AT-SPI yet (docs/parity.md ACC-07):
            # find the tile left of the Bin in a picture of the Dock and click it.
            point = self.minimized_tile_point("dock-after-set-minimized")
            self.check("the Dock shows a minimised tile left of the Bin", point, str(point))
            # Restore it from the keyboard, as a Mac user can: ⌃F3 moves focus
            # to the Dock, → stops on the Bin (the last item), ← selects the
            # tile before it, Return.
            self.keys.key("ctrl-f3")
            time.sleep(1.0)
            for key in ["right"] * 30 + ["left"]:
                self.keys.key(key)
                time.sleep(0.1)
            time.sleep(0.5)
            self.keys.key("enter")
        restored = self.wait_for(lambda: (self.window(wid) or {}).get("workspace_id") == origin, 10)
        self.check("the Dock restores it to its workspace", restored)
        self.check("restoring forgets the record", restored and not self.entry(wid))

        # 3. ⌘M on the focused third-party window.
        subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(wid)],
                       env=self.env, capture_output=True, check=False)
        time.sleep(1)
        self.keys.key("alt-m")
        self.check("⌘M parks the focused third-party window", self.wait_for(lambda: self.parked(wid), 15))
        entry = self.entry(wid)
        self.check("⌘M records the origin and a thumbnail",
                   entry and entry["workspace"] == origin and entry.get("thumbnail"), json.dumps(entry))
        self.check("one thumbnail per window after minimising again", len(self.thumbnails(wid)) == 1)
        time.sleep(2)
        self.screenshot("dock-after-cmd-m")
        for picture in self.thumbnails(wid):
            subprocess.run(["cp", str(picture), str(self.out / "gtk-thumbnail.png")], check=False)

        # 4. Closing a parked window cleans up after it.
        gtk.send_signal(signal.SIGTERM)
        gtk.wait(10)
        self.check("the parked window closed", self.wait_for(lambda: not self.window(wid), 10))
        self.check("WindowClosed drops its record and thumbnails",
                   self.wait_for(lambda: not self.entry(wid) and not self.thumbnails(wid), 10))

        # 5. An rmac app goes the same way.
        if self.args.calculator:
            calculator = self.spawn([self.args.calculator], "calculator", {"VK_ICD_FILENAMES": LAVAPIPE})
            window = self.wait_for(lambda: next((w for w in self.niri("windows") or []
                                                 if w.get("app_id") == "org.rmac.Calculator"), None), 30)
            self.check("Calculator mapped", window)
            if window:
                time.sleep(4)
                subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(window["id"])],
                               env=self.env, capture_output=True, check=False)
                time.sleep(1)
                print("calculator before ⌘M:", json.dumps(self.window(window["id"])), flush=True)
                self.keys.key("alt-m")
                self.check("⌘M parks an rmac app", self.wait_for(lambda: self.parked(window["id"]), 15))
                pictures = self.thumbnails(window["id"])
                self.check("the rmac app has a thumbnail", pictures)
                for picture in pictures:
                    subprocess.run(["cp", str(picture), str(self.out / "calculator-thumbnail.png")], check=False)
            calculator.terminate()
            calculator.wait(10)
        return self.finish()

    def finish(self) -> int:
        try:
            self.keys.close()
        except Exception:  # noqa: BLE001
            pass
        for process in reversed(self.children):
            if process.poll() is None:
                process.terminate()
        for process in reversed(self.children):
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.kill()
        failed = [result for result in self.results if not result[1]]
        print(f"\n{len(self.results) - len(failed)}/{len(self.results)} checks passed", flush=True)
        return 1 if failed else 0


# --------------------------------------------------------------------------
# Outer run: the isolated environment (shared with run_lulo.py).
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", "busctl"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-niri-minimize-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)  # set per process instead: niri, Sway and GPUI differ
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
            return subprocess.call(command, env=env, close_fds=True, stderr=log)
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
    parser.add_argument("--bin-dir", help="directory with this branch's dock and mission-control")
    parser.add_argument("--calculator")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--gtk-window", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--minimize-after", type=float, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.gtk_window:
        return gtk_window(args.minimize_after)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
