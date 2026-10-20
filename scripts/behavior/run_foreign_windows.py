#!/usr/bin/env python3
"""Third-party windows stay in the Mac's visible frame, in a nested niri.

    python3 scripts/behavior/run_foreign_windows.py --niri PATH --bin-dir DIR [--keep]

macOS constrains every new window to its screen's visible frame (below the
menu bar, above the Dock). niri floats Lulo OS windows but only keeps 75 px of
one on screen, so third-party apps opening at a remembered huge size or at
0,0 sat under the top bar and the Dock. Mission Control's window placer
(crates/rmac-compositor/src/placement.rs) fixes that from niri's event stream.

This runner starts headless Sway as a parent display, the shipped shell.kdl
in a nested niri, and this branch's `top-bar`, `dock` and `mission-control
--service` from --bin-dir, then checks:

1. a GTK 4 window asking for 4000 × 4000 ends inside the visible frame;
2. foot asking for 4000 × 4000 pixels ends inside it;
3. an XWayland GTK 3 window at 0,0 and 4000 × 4000 ends inside it (only when
   xwayland-satellite is installed; reported, not required elsewhere);
4. the green button (xdg maximize) fills the visible frame exactly, not the
   screen;
5. a window moved with its title bar under the menu bar comes back below
   it, while one moved partly under the Dock stays there, as on the Mac;
6. a bigger Dock (Desktop & Dock ▸ Size) moves a window that now sits under
   it back above it;
7. Calculator, a Lulo app, opens where it put itself (the placer leaves
   Lulo's own windows alone), when --bin-dir has rmac-calculator;
8. the menu bar names a third-party app by its desktop entry's Name, found
   by desktop ID and by `StartupWMClass`;
9. ⌘W (niri reads Mod as Alt when nested) closes the frontmost third-party
   window, and ⌘Q closes every window of a third-party app, through the
   menu bar, without keyd;
10. Zoom toggles: once fills the visible frame, again puts the window back.

Isolation is run_lulo.py's: private dbus-run-session, HOME and
XDG_RUNTIME_DIR, `wayland-0`/`wayland-1` held, nothing sent to the live
session. Windows are moved over the nested niri's own IPC socket; the ⌘Q/⌘W keys
go only to this run's headless Sway, whose one window is niri (wlinput.py).
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
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
MENU_BAR = 29.0
# The Dock's reservation at 64 pt tiles: shelf plus margin (rmac-dock
# reserved_thickness), and at 128 pt.
DOCK = 89.0
BIG_DOCK = 178.0
SLACK = 2.0


def gtk4_window(app_id: str, title: str, maximize_after: float | None) -> int:
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import GLib, Gtk

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title=title)
        window.set_default_size(4000, 4000)
        window.set_child(Gtk.Label(label=title))
        window.present()
        if maximize_after is not None:
            # Exactly what the title bar's green (maximise) button does.
            GLib.timeout_add(int(maximize_after * 1000), lambda: window.maximize() and False)

    app = Gtk.Application(application_id=app_id)
    app.connect("activate", activate)
    return app.run([])


def gtk4_pair(app_id: str) -> int:
    """Two ordinary windows of one app, for ⌘Q."""
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import Gtk

    def activate(app):
        for index in (1, 2):
            window = Gtk.ApplicationWindow(application=app, title=f"Pair {index}")
            window.set_default_size(500, 300)
            window.set_child(Gtk.Label(label=f"Pair {index}"))
            window.present()

    app = Gtk.Application(application_id=app_id)
    app.connect("activate", activate)
    return app.run([])


def gtk3_window(title: str) -> int:
    import gi

    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk

    window = Gtk.Window(title=title)
    window.set_default_size(4000, 4000)
    window.move(0, 0)
    window.connect("destroy", Gtk.main_quit)
    window.show_all()
    Gtk.main()
    return 0


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

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def action(self, *words: str) -> None:
        subprocess.run([self.args.niri, "msg", "action", *words], env=self.env,
                       capture_output=True, timeout=10, check=False)

    def window(self, app_id: str):
        return next((w for w in self.niri("windows") or [] if w.get("app_id") == app_id), None)

    def window_by_id(self, window_id: int):
        return next((w for w in self.niri("windows") or [] if w["id"] == window_id), None)

    @staticmethod
    def frame(window: dict) -> tuple[float, float, float, float]:
        layout = window.get("layout") or {}
        x, y = layout.get("tile_pos_in_workspace_view") or (0.0, 0.0)
        width, height = layout.get("tile_size") or (0.0, 0.0)
        return float(x), float(y), float(width), float(height)

    def visible(self, dock: float = DOCK) -> tuple[float, float, float, float]:
        width, height = self.output_size
        return 0.0, MENU_BAR, float(width), float(height) - MENU_BAR - dock

    @staticmethod
    def inside(frame, area) -> bool:
        x, y, width, height = frame
        ax, ay, aw, ah = area
        return (x >= ax - SLACK and y >= ay - SLACK and x + width <= ax + aw + SLACK
                and y + height <= ay + ah + SLACK)

    def settled_inside(self, app_id: str, area, timeout: float = 15):
        def placed():
            window = self.window(app_id)
            return window if window and self.inside(self.frame(window), area) else None
        return self.wait_for(placed, timeout)

    # -- the session --------------------------------------------------------

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
        sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not sway_display:
            raise SystemExit("sway did not start")

        bins = Path(self.args.bin_dir)
        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        config = self.out / "niri.kdl"
        config.write_text(shell.replace("/usr/libexec/rmac/rmac-mission-control", str(bins / "mission-control"))
                          .replace("/usr/libexec/rmac/rmac-shortcut-dispatch", str(bins / "rmac-shortcut-dispatch"))
                          .replace("/usr/libexec/rmac/rmac-dock", str(bins / "dock")))
        self.sway_display = sway_display
        validate = subprocess.run([self.args.niri, "validate", "-c", str(config)], env=self.env,
                                  capture_output=True, text=True)
        self.check("niri validate accepts shell.kdl", validate.returncode == 0,
                   "" if validate.returncode == 0 else validate.stderr[-300:])

        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket),
                         "RMAC_BEHAVIOR_NESTED": "1"})
        self.env.pop("DISPLAY", None)
        # Desktop entries for the third-party apps: one by desktop ID, one
        # claiming foot's windows through StartupWMClass.
        applications = Path(self.env["XDG_DATA_HOME"]) / "applications"
        applications.mkdir(parents=True, exist_ok=True)
        (applications / "org.example.ForeignBig.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Foreign Big App\nExec=true\n")
        (applications / "foreign-terminal.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Foreign Terminal\nExec=foot\n"
            "StartupWMClass=org.example.ForeignFoot\n")
        gpu = {"VK_ICD_FILENAMES": LAVAPIPE}
        for name in ("top-bar", "dock"):
            if (bins / name).exists():
                self.spawn([str(bins / name)], name, gpu)
        self.spawn([str(bins / "mission-control"), "--service"], "mission-control", gpu)
        # The bar and the Dock reserve their edges (layer-shell exclusive zones).
        time.sleep(8)
        # The nested output takes Sway's window size once niri's own window
        # is configured; read it after that.
        outputs = self.niri("outputs") or {}
        logical = (next(iter(outputs.values()), {}) or {}).get("logical") or {}
        self.output_size = (logical.get("width", 1440), logical.get("height", 900))
        print(f"nested output {self.output_size}", flush=True)

    # -- the checks ---------------------------------------------------------

    def run(self) -> int:
        self.start()
        area = self.visible()
        me = [sys.executable, str(Path(__file__).resolve())]
        gtk_env = {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"}

        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

        # The visible frame as the Dock changes below.
        current_area = area

        # 1. GTK 4 at 4000 × 4000.
        gtk = self.spawn([*me, "--gtk4", "org.example.ForeignBig"], "gtk4", gtk_env)
        window = self.wait_for(lambda: self.window("org.example.ForeignBig"))
        self.check("GTK 4 window mapped", window)
        placed = self.settled_inside("org.example.ForeignBig", area)
        last = self.window("org.example.ForeignBig")
        self.check("a 4000 × 4000 GTK 4 window ends inside the visible frame", placed,
                   f"frame {self.frame(last) if last else None} visible {area}")
        self.check("the menu bar names it by its desktop entry",
                   self.wait_for(lambda: self.logged("top-bar", "org.example.ForeignBig is named Foreign Big App"), 10))
        gtk.terminate()
        gtk.wait(10)

        # 2. foot at 4000 × 4000 pixels.
        if shutil.which("foot"):
            foot = self.spawn(["foot", "--app-id=org.example.ForeignFoot", "--override=pad=0x0",
                               "--window-size-pixels=4000x4000", "sleep", "600"], "foot")
            window = self.wait_for(lambda: self.window("org.example.ForeignFoot"))
            self.check("foot mapped", window)
            placed = self.settled_inside("org.example.ForeignFoot", area)
            last = self.window("org.example.ForeignFoot")
            self.check("a 4000 × 4000 foot window ends inside the visible frame", placed,
                       f"frame {self.frame(last) if last else None}")
            first = window["id"] if window else None
            if first is not None:
                self.action("focus-window", "--id", str(first))
            self.check("the menu bar names foot's window by StartupWMClass",
                       self.wait_for(lambda: self.logged("top-bar", "org.example.ForeignFoot is named Foreign Terminal"), 10))
        else:
            print("SKIP foot is not installed", flush=True)
            foot, first = None, None

        # 3. XWayland, where niri can start xwayland-satellite.
        if shutil.which("xwayland-satellite"):
            self.action("spawn", "--", "env", "GDK_BACKEND=x11", sys.executable,
                        str(Path(__file__).resolve()), "--gtk3", "Foreign X11")
            window = self.wait_for(lambda: next((w for w in self.niri("windows") or []
                                                 if w.get("title") == "Foreign X11"), None), 30)
            if window:
                def x11_placed():
                    current = self.window_by_id(window["id"])
                    return current if current and self.inside(self.frame(current), area) else None
                placed = self.wait_for(x11_placed, 15)
                current = self.window_by_id(window["id"])
                self.check("a 4000 × 4000 XWayland window at 0,0 ends inside the visible frame", placed,
                           f"frame {self.frame(current) if current else None}")
                self.action("close-window", "--id", str(window["id"]))
            else:
                print("SKIP no XWayland window appeared (xwayland-satellite did not start)", flush=True)
        else:
            print("SKIP xwayland-satellite is not installed", flush=True)

        # 4. The green button fills the visible frame, not the screen.
        gtk = self.spawn([*me, "--gtk4", "org.example.ForeignZoom", "--maximize-after", "3"], "zoom", gtk_env)

        def maximized():
            window = self.window("org.example.ForeignZoom")
            if not window or window.get("is_floating", True):
                return None
            return window
        window = self.wait_for(maximized, 20)
        frame = self.frame(window) if window else None
        # A maximised window is tiled; niri reports its position inside the
        # working area, so the size is what tells whether it fills the
        # visible frame or the whole screen.
        filled = frame and abs(frame[2] - area[2]) <= SLACK and abs(frame[3] - area[3]) <= SLACK
        self.check("the green button fills the visible frame exactly", filled, f"frame {frame} visible {area}")
        gtk.terminate()
        gtk.wait(10)

        # 5. Title bar below the menu bar; the Dock may cover a window.
        if first is not None:
            start = self.frame(self.window_by_id(first))
            self.action("move-floating-window", "--id", str(first), "-x", "+0", "-y", f"-{start[1] + 100:.0f}")

            def below_bar():
                current = self.window_by_id(first)
                return current if current and self.frame(current)[1] >= MENU_BAR - SLACK else None
            time.sleep(1.0)
            self.check("a window moved under the menu bar comes back below it", self.wait_for(below_bar, 10),
                       f"frame {self.frame(self.window_by_id(first))}")
            # Resize it smaller so it can go partly under the Dock and stay on screen.
            self.action("set-window-height", "--id", str(first), "300")
            self.action("set-window-width", "--id", str(first), "500")
            time.sleep(1.0)
            current = self.frame(self.window_by_id(first))
            target_y = self.output_size[1] - DOCK - 150
            self.action("move-floating-window", "--id", str(first), "-x", "+0", "-y",
                        f"{target_y - current[1]:+.0f}")
            time.sleep(2.0)
            after = self.frame(self.window_by_id(first))
            self.check("a window moved partly under the Dock stays there", after[1] + after[3] > area[1] + area[3] + SLACK,
                       f"frame {after}")

            # 6. A bigger Dock moves the window now under it. First bring the
            # window just above the 89 pt Dock.
            self.action("move-floating-window", "--id", str(first), "-x", "+0", "-y",
                        f"{(area[1] + area[3] - after[3]) - after[1] - 5:+.0f}")
            time.sleep(2.0)
            before_dock = self.frame(self.window_by_id(first))
            self.check("the window sits just above the Dock", self.inside(before_dock, area), f"frame {before_dock}")
            self.set_dock_tile_size(128.0)
            big = self.visible(BIG_DOCK)
            current_area = big

            def above_big_dock():
                current = self.window_by_id(first)
                return current if current and self.inside(self.frame(current), big) else None
            self.check("a bigger Dock moves the window above it", self.wait_for(above_big_dock, 15),
                       f"frame {self.frame(self.window_by_id(first))} visible {big}")
        # 9a. ⌘W closes the frontmost third-party window.
        if foot is not None and first is not None:
            self.action("focus-window", "--id", str(first))
            time.sleep(1.0)
            self.keys.key("alt-w")
            self.check("⌘W closes a third-party window without keyd",
                       self.wait_for(lambda: self.window_by_id(first) is None, 10))
            self.check("closing its only window ends foot", self.wait_for(lambda: foot.poll() is not None, 10))
            if foot.poll() is None:
                foot.terminate()
                foot.wait(10)

        # 10 and 9b. Zoom toggles; ⌘Q closes every window of the app.
        pair = self.spawn([*me, "--gtk4-pair", "org.example.ForeignPair"], "pair", gtk_env)
        windows = self.wait_for(lambda: [w for w in self.niri("windows") or []
                                         if w.get("app_id") == "org.example.ForeignPair"
                                         and len([v for v in self.niri("windows") or []
                                                  if v.get("app_id") == "org.example.ForeignPair"]) == 2], 20)
        self.check("two windows of one third-party app mapped", windows)
        if windows:
            target = windows[0]["id"]
            time.sleep(1.5)
            self.action("focus-window", "--id", str(target))
            time.sleep(0.5)
            before = self.frame(self.window_by_id(target))
            mission_control = str(Path(self.args.bin_dir) / "mission-control")
            subprocess.run([mission_control, "zoom"], env=self.env, capture_output=True, timeout=10, check=False)

            def filled():
                current = self.window_by_id(target)
                frame = self.frame(current) if current else None
                return frame if frame and all(abs(a - b) <= SLACK for a, b in zip(frame, current_area)) else None
            self.check("Zoom fills the visible frame", self.wait_for(filled, 10),
                       f"frame {self.frame(self.window_by_id(target))} visible {current_area}")
            subprocess.run([mission_control, "zoom"], env=self.env, capture_output=True, timeout=10, check=False)

            def restored():
                current = self.window_by_id(target)
                frame = self.frame(current) if current else None
                return frame if frame and all(abs(a - b) <= SLACK for a, b in zip(frame, before)) else None
            self.check("Zoom again puts the window back", self.wait_for(restored, 10),
                       f"frame {self.frame(self.window_by_id(target))} before {before}")
            self.action("focus-window", "--id", str(target))
            time.sleep(0.5)
            self.keys.key("alt-q")
            self.check("⌘Q closes every window of a third-party app without keyd",
                       self.wait_for(lambda: not [w for w in self.niri("windows") or []
                                                  if w.get("app_id") == "org.example.ForeignPair"], 10))
            self.check("the app exits once its windows are closed", self.wait_for(lambda: pair.poll() is not None, 10))
        if pair.poll() is None:
            pair.terminate()
            pair.wait(10)

        # 7. A Lulo app is left where it put itself.
        calculator = Path(self.args.bin_dir) / "rmac-calculator"
        if calculator.exists():
            process = self.spawn([str(calculator)], "calculator", {"VK_ICD_FILENAMES": LAVAPIPE})
            window = self.wait_for(lambda: self.window("org.rmac.Calculator"), 40)
            self.check("Calculator mapped", window)
            if window:
                time.sleep(3)
                settled = self.frame(self.window("org.rmac.Calculator"))
                time.sleep(2)
                again = self.frame(self.window("org.rmac.Calculator"))
                self.check("Calculator stays where it opened", settled == again, f"{settled} then {again}")
            process.terminate()
            process.wait(10)
        return self.finish()

    def logged(self, name: str, text: str) -> bool:
        path = self.out / f"{name}.log"
        return path.exists() and text in path.read_text(errors="replace")

    def set_dock_tile_size(self, size: float) -> None:
        """Desktop & Dock ▸ Size, through the shell settings document the
        Dock and Mission Control watch. `dock toggle-hide` twice writes a
        complete document first."""
        path = Path(self.env.get("XDG_CONFIG_HOME") or Path(self.env["HOME"]) / ".config") / "rmac" / "shell.json"
        if not path.exists():
            for _ in range(2):
                subprocess.run([str(Path(self.args.bin_dir) / "dock"), "toggle-hide"], env=self.env,
                               capture_output=True, timeout=10, check=False)
                time.sleep(1.0)
        document = json.loads(path.read_text())
        settings = document.get("settings", document)
        settings["dock"]["tile_size"] = size
        settings["dock"]["autohide"] = False
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(document))
        temporary.replace(path)

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


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "dbus-run-session"):
        if not shutil.which(tool):
            raise SystemExit(f"{tool} is required")
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-foreign-windows-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES", "DISPLAY"):
            env.pop(key, None)
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                   "--inner", str(work), *argv]
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
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", help="directory with this branch's top-bar, dock and mission-control")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--gtk4", help=argparse.SUPPRESS)
    parser.add_argument("--gtk3", help=argparse.SUPPRESS)
    parser.add_argument("--gtk4-pair", help=argparse.SUPPRESS)
    parser.add_argument("--maximize-after", type=float, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.gtk4:
        return gtk4_window(args.gtk4, args.gtk4.rsplit(".", 1)[-1], args.maximize_after)
    if args.gtk3:
        return gtk3_window(args.gtk3)
    if args.gtk4_pair:
        return gtk4_pair(args.gtk4_pair)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
