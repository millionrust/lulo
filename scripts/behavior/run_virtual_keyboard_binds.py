#!/usr/bin/env python3
"""Virtual-keyboard keys reach niri's key binds (ROBOT-13), in a nested niri.

    python3 scripts/behavior/run_virtual_keyboard_binds.py --niri PATH [--keep]

An on-screen keyboard types through `zwp_virtual_keyboard_v1`. Upstream niri
(through smithay) sends those keys straight to the focused client, so no
compositor shortcut works from one: not Spotlight (⌘Space), not ⌃F2 menu-bar
focus, not ⌃F3 Dock focus. Lulo's niri (26.04+lulo1-3,
packaging/third-party/niri/debian/patches/0002-virtual-keyboard-binds.patch)
runs the configured bind for a matching virtual key press instead.

This runner starts headless Sway only as a parent display and runs niri
nested inside it with a small config whose binds touch marker files. A GTK 4
window logs the keys it receives. Unlike every other runner here, the
virtual keyboard connects to *niri itself* (not to Sway, which would turn the
keys into real keyboard input for niri). Checks:

1. ⌃F2 from the virtual keyboard runs its bind (the marker appears);
2. the window never receives that F2 press or its release;
3. Mod+Shift+T (niri reads Mod as Alt when nested) runs its bind too;
4. an unbound key ("a") still reaches the focused window.

Upstream niri fails 1-3. Isolation is run_lulo.py's (private dbus-run-session,
temporary HOME and XDG_RUNTIME_DIR, `wayland-0`/`wayland-1` held), and the
input goes only to this run's compositors through wlinput.py.
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

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402

CLIENT_APP_ID = "org.example.KeyLog"

CONFIG = """
input {{ keyboard {{ xkb {{ layout "us"; }} }} }}
hotkey-overlay {{ skip-at-startup; }}
binds {{
    Ctrl+F2 {{ spawn "touch" "{marker_f2}"; }}
    Mod+Shift+T {{ spawn "touch" "{marker_t}"; }}
}}
"""


def key_log_window(log: str) -> int:
    """A GTK 4 window that appends every key it receives to `log`."""

    import gi

    gi.require_version("Gtk", "4.0")
    gi.require_version("Gdk", "4.0")
    from gi.repository import Gdk, Gtk

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title="Key Log")
        window.set_default_size(400, 300)
        controller = Gtk.EventControllerKey()

        def record(kind):
            def handler(_controller, keyval, _keycode, _state):
                with open(log, "a") as handle:
                    handle.write(f"{kind} {Gdk.keyval_name(keyval)}\n")
                return False
            return handler

        controller.connect("key-pressed", record("press"))
        controller.connect("key-released", record("release"))
        window.add_controller(controller)
        window.present()

    app = Gtk.Application(application_id=CLIENT_APP_ID)
    app.connect("activate", activate)
    return app.run([])


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.work = work
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
        process = subprocess.Popen(argv, env={**self.env, **(extra or {})},
                                   stdout=open(self.out / f"{name}.log", "w"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    @staticmethod
    def wait_for(predicate, timeout: float = 20, step: float = 0.1):
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

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             "output HEADLESS-1 mode 1280x800 position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not sway_display:
            raise SystemExit("sway did not start")

        self.marker_f2 = self.work / "ctrl-f2.marker"
        self.marker_t = self.work / "mod-shift-t.marker"
        config = self.out / "niri.kdl"
        config.write_text(CONFIG.format(marker_f2=self.marker_f2, marker_t=self.marker_t))
        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})

        self.key_log = self.work / "keys.log"
        self.key_log.touch()
        self.spawn([sys.executable, str(Path(__file__).resolve()), "--key-log-window", str(self.key_log)],
                   "client", {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"})
        focused = self.wait_for(lambda: (self.niri("focused-window") or {}).get("app_id") == CLIENT_APP_ID, 30)
        self.check("the key-log window has keyboard focus", focused)
        # Straight into niri: this is the on-screen keyboard's path.
        self.keys = wlinput.Wayland({**self.env, "RMAC_BEHAVIOR_NESTED": "1"})

    def niri(self, request: str):
        out = subprocess.run([self.args.niri, "msg", "-j", request], env=self.env,
                             capture_output=True, text=True, timeout=10)
        return json.loads(out.stdout) if out.returncode == 0 and out.stdout.strip() else None

    def received(self) -> list[str]:
        return self.key_log.read_text().splitlines()

    def run(self) -> int:
        try:
            self.start()
            self.keys.key("ctrl-f2")
            self.check("Ctrl+F2 from a virtual keyboard runs its niri bind",
                       self.wait_for(self.marker_f2.exists, 5))
            time.sleep(0.5)
            leaked = [line for line in self.received() if line.endswith(" F2")]
            self.check("the bound key never reaches the focused window", not leaked, f"{leaked}")

            # niri reads Mod as Alt when it runs nested.
            self.keys.key("alt-shift-t")
            self.check("Mod+Shift+T from a virtual keyboard runs its niri bind",
                       self.wait_for(self.marker_t.exists, 5))

            self.keys.key("a")
            self.check("an unbound key still reaches the focused window",
                       self.wait_for(lambda: "press a" in self.received(), 5), f"{self.received()[-4:]}")
        except SystemExit as error:
            self.check("setup", False, str(error))
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


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "dbus-run-session"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-vk-binds-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE"):
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--key-log-window", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.key_log_window:
        return key_log_window(args.key_log_window)
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
