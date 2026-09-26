#!/usr/bin/env python3
"""End-to-end Shut Down / Restart completion check, in a nested niri.

    python3 scripts/behavior/run_shutdown.py --niri PATH --bin-dir DIR [--keep]

--bin-dir must hold this branch's `top-bar`, `dock` and `rmac-shortcut-dispatch`
(shell's and the root workspace's `cargo build --profile iterate` output share
one CARGO_TARGET_DIR on the laptop, so one directory has all three).

The owner's complaint was "can't shut down, restart": clicking through to Shut
Down or Restart never actually powers the machine off. Opening the
confirmation dialog was live-tested before (docs/parity.md SESSION-01..04),
but nobody had exercised the far end — whether the menu bar's `quit_all_then`
(shell/bins/rmac-menubar/src/main.rs) really asks every window to close, waits
for a parked (minimised) one exactly like a visible one, and then hands off to
`systemctl`. This runs that whole path for real, with a fake `systemctl` on
PATH that only records what it was asked to do, so nothing is powered off.

It:
  1. starts headless Sway, a nested niri (packaging/rmac-session/shell.kdl),
     the Dock (so a GTK window's own minimise really gets parked) and this
     branch's `top-bar`;
  2. opens a plain GTK window and a second one that minimises itself, so a
     parked window is on the board when Shut Down is asked for;
  3. fires the `shutdown-dialog` dispatch socket, exactly as a second press of
     the power button would (rmac_shortcuts::power_key::SHUTDOWN_DIALOG_SHORTCUT),
     and clicks the dialog's Shut Down button over AT-SPI;
  4. checks both windows (the parked one included) were asked to close and
     that the fake `systemctl poweroff` ran;
  5. repeats for Restart (`systemctl reboot`).

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, wayland-0/wayland-1 held so no socket can collide with the
live session, and no input is sent anywhere but this run's own Sway. The
power button and the lock coordinator are deliberately not exercised here:
they take a inhibitor on the real system D-Bus, which this script must not
touch on a shared machine. `shutdown-dialog` is the same socket the
coordinator would use, so this covers the shared, at-risk code
(`quit_all_then`, `spawn_command`) without that.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import stat
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
GTK_APP_ID = "org.example.ShutdownTest"


def app_id_for(title: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9]+", "", title)
    return f"{GTK_APP_ID}.{slug}"


def gtk_window(title: str, minimize_after: float | None, refuse_close: bool = False) -> int:
    import gi

    gi.require_version("Gtk", "4.0")
    from gi.repository import GLib, Gtk

    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title=title)
        window.set_default_size(360, 240)
        window.set_child(Gtk.Label(label=title))
        window.present()
        if minimize_after is not None:
            GLib.timeout_add(int(minimize_after * 1000), lambda: window.minimize() and False)
        if refuse_close:
            # Stands in for an app with an unsaved-changes alert: it stays
            # open no matter how many times niri asks it to close.
            window.connect("close-request", lambda *_: True)

    app = Gtk.Application(application_id=app_id_for(title))
    app.connect("activate", activate)
    return app.run([])


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
        self.systemctl_log = work / "systemctl-calls.log"
        self.systemctl_log.touch()

    def check(self, name: str, ok, detail: str = "") -> None:
        self.results.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name} {detail}".rstrip(), flush=True)

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(argv, env=env, stdout=open(self.out / f"{name}.log", "a"),
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

    # -- fake systemctl ----------------------------------------------------

    def install_fake_systemctl(self, work: Path) -> None:
        fakebin = work / "fakebin"
        fakebin.mkdir(exist_ok=True)
        script = fakebin / "systemctl"
        script.write_text(
            "#!/bin/sh\n"
            f'echo "$@" >> "{self.systemctl_log}"\n'
            "exit 0\n"
        )
        script.chmod(script.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        self.env["PATH"] = f"{fakebin}:{self.env['PATH']}"

    def systemctl_calls(self) -> list[str]:
        return [line for line in self.systemctl_log.read_text().splitlines() if line.strip()]

    # -- compositors ---------------------------------------------------------

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
        outputs = self.niri("outputs") or {}
        self.output = next(iter(outputs), "winit")

        bins = Path(self.args.bin_dir)
        self.dock = self.spawn([str(bins / "dock")], "dock", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.top_bar = self.spawn([str(bins / "top-bar")], "top-bar", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.spawn([mission_control, "--service"], "mission-control", {"VK_ICD_FILENAMES": LAVAPIPE})
        self.dispatch_bin = str(bins / "rmac-shortcut-dispatch")
        self.events = open(self.out / "events.jsonl", "w")
        self.children.append(subprocess.Popen([self.args.niri, "msg", "-j", "event-stream"], env=self.env,
                                              stdout=self.events, stderr=subprocess.DEVNULL))
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(6)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    # -- niri state ----------------------------------------------------------

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def windows(self):
        return self.niri("windows") or []

    def window(self, app_id: str):
        return next((w for w in self.windows() if w.get("app_id") == app_id), None)

    def parking(self):
        return next((w["id"] for w in self.niri("workspaces") or [] if w.get("name") == "rmac-parking"), None)

    def parked(self, window_id: int) -> bool:
        window = next((w for w in self.windows() if w["id"] == window_id), None)
        return bool(window and window.get("workspace_id") == self.parking())

    # -- AT-SPI ---------------------------------------------------------------

    def find_node(self, roles: tuple[str, ...], matches) -> object | None:
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        stack = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        while stack:
            node = stack.pop()
            try:
                if node is None:
                    continue
                if node.getRoleName() in roles and matches(node.name or ""):
                    return node
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:  # noqa: BLE001
                continue
        return None

    def find_button(self, label: str):
        return self.find_node(("push button", "button"), lambda name: name == label)

    def find_menu_item(self, prefix: str):
        # Log Out's own item reads "Log Out {account name}…" (main.rs
        # `system_menu`'s `logout_label`), so this matches by prefix.
        return self.find_node(("menu item",), lambda name: name.startswith(prefix))

    def dispatch(self, shortcut: str) -> None:
        subprocess.run([self.dispatch_bin, shortcut], env=self.env, capture_output=True,
                       timeout=10, check=False)

    # -- one Shut Down / Restart cycle ----------------------------------------

    def shutdown_cycle(self, verb: str, systemctl_verb: str) -> None:
        """Opens two windows (one minimising itself), asks for `verb`
        ("Shut Down" or "Restart") through the power dialog, as a second
        press of the power button would, and checks both windows were asked
        to close and the fake systemctl ran."""

        plain_title = f"{verb} Plain"
        parking_title = f"{verb} Parking"
        plain = self.spawn([sys.executable, __file__, "--gtk-window", plain_title], f"{verb}-plain",
                           {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"})
        parking = self.spawn([sys.executable, __file__, "--gtk-window", parking_title], f"{verb}-parking",
                             {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"})
        plain_window = self.wait_for(lambda: self.window(app_id_for(plain_title)), 20)
        parking_window = self.wait_for(lambda: self.window(app_id_for(parking_title)), 20)
        self.check(f"{verb}: both windows mapped", plain_window and parking_window)
        if not (plain_window and parking_window):
            plain.terminate()
            parking.terminate()
            return
        # A GTK window's own minimise button only reaches the Dock through
        # niri's WindowMinimizeRequested, which needs Lulo's patched niri
        # (docs/decisions/0021-niri-minimize-request.md); this laptop's
        # /usr/bin/niri is the stock build. ⌘M (Mission Control's synthetic
        # minimize) parks a window on any niri, so it stands in here for a
        # minimised/parked window on the board.
        subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(parking_window["id"])],
                       env=self.env, capture_output=True, check=False)
        time.sleep(1)
        self.keys.key("alt-m")
        parked = self.wait_for(lambda: self.parked(parking_window["id"]), 15)
        self.check(f"{verb}: the second window minimised (parked)", parked,
                   f"dock alive={self.dock.poll() is None} window={self.window(app_id_for(parking_title))} "
                   f"parking_id={self.parking()}")

        before_calls = len(self.systemctl_calls())
        started = time.monotonic()
        self.dispatch("shutdown-dialog")
        button = self.wait_for(lambda: self.find_button(verb), 10, 0.3)
        self.check(f"{verb}: the power dialog shows a {verb!r} button (AT-SPI)", button)
        if button is None:
            plain.terminate()
            parking.terminate()
            return
        button.queryAction().doAction(0)

        closed = self.wait_for(
            lambda: not self.window(app_id_for(plain_title))
            and not self.window(app_id_for(parking_title)), 35)
        self.check(f"{verb}: both windows (parked one included) were asked to close", closed)

        new_calls = self.wait_for(lambda: self.systemctl_calls()[before_calls:] or None, 10)
        elapsed = time.monotonic() - started
        self.check(f"{verb}: the fake systemctl ran {systemctl_verb!r}",
                   new_calls and new_calls[-1].strip() == systemctl_verb,
                   f"calls={new_calls} elapsed={elapsed:.1f}s")
        self.check(f"{verb}: it did not wait for the full 30 s grace period", elapsed < 20,
                   f"elapsed={elapsed:.1f}s")

        for process in (plain, parking):
            if process.poll() is None:
                process.terminate()
                process.wait(5)

    # -- an app that refuses to close ------------------------------------------

    def blocked_then_recovers(self) -> None:
        """An app that ignores every close request (an unsaved-changes
        alert, say) must cancel Shut Down instead of leaving it stuck; once
        that app is dealt with, asking again must still work. This is the
        Mac's "<App> interrupted shutdown" behaviour
        (`quit_all_interrupted_copy`), checked for real rather than only in
        the pure `quit_all_progress` unit tests."""

        title = "Stuck App"
        blocker = self.spawn([sys.executable, __file__, "--gtk-window", title, "--refuse-close"],
                             "blocker", {"GDK_BACKEND": "wayland", "GSK_RENDERER": "cairo"})
        window = self.wait_for(lambda: self.window(app_id_for(title)), 20)
        self.check("Blocked: the stuck window mapped", window)
        if not window:
            blocker.terminate()
            return

        before_calls = len(self.systemctl_calls())
        self.dispatch("shutdown-dialog")
        button = self.wait_for(lambda: self.find_button("Shut Down"), 10, 0.3)
        self.check("Blocked: the power dialog shows a Shut Down button", button)
        if button is None:
            blocker.terminate()
            return
        button.queryAction().doAction(0)

        # QUIT_ALL_GRACE is 30 s (menu_model.rs); give it a margin either way.
        time.sleep(33)
        self.check("Blocked: the stuck window is still open (cancelled, not forced closed)",
                   self.window(app_id_for(title)))
        self.check("Blocked: systemctl was never asked to power off",
                   len(self.systemctl_calls()) == before_calls,
                   f"calls={self.systemctl_calls()[before_calls:]}")

        # Deal with the "unsaved changes": close it for real, then ask again.
        blocker.terminate()
        blocker.wait(10)
        self.check("Blocked: the window is gone once the app is dealt with",
                   self.wait_for(lambda: not self.window(app_id_for(title)), 10))

        self.dispatch("shutdown-dialog")
        button = self.wait_for(lambda: self.find_button("Shut Down"), 10, 0.3)
        self.check("Blocked: a fresh Shut Down still shows the dialog", button)
        if button is None:
            return
        button.queryAction().doAction(0)
        new_calls = self.wait_for(lambda: self.systemctl_calls()[before_calls:] or None, 10)
        self.check("Blocked: Shut Down completes normally once nothing blocks it",
                   new_calls and new_calls[-1].strip() == "poweroff", f"calls={new_calls}")

    # -- the Apple-menu path (as most users actually shut down) --------------

    def menu_shutdown(self) -> None:
        """The everyday route: the logo menu's own "Shut Down…" item, not
        the power-key dialog. It opens a 60 s-countdown confirmation
        (`start_confirmation`) instead of dispatching immediately; clicking
        its own "Shut Down" button must still reach `quit_all_then`."""

        before_calls = len(self.systemctl_calls())
        logo = self.wait_for(lambda: self.find_button("menu"), 10, 0.3)
        self.check("Menu: the logo menu is on the bar (AT-SPI)", logo)
        if logo is None:
            return
        logo.queryAction().doAction(0)
        item = self.wait_for(lambda: self.find_menu_item("Shut Down…"), 10, 0.3)
        self.check("Menu: the system menu lists Shut Down…", item)
        if item is None:
            return
        item.queryAction().doAction(0)
        button = self.wait_for(lambda: self.find_button("Shut Down"), 10, 0.3)
        self.check("Menu: the confirmation shows its own Shut Down button", button)
        if button is None:
            return
        button.queryAction().doAction(0)
        new_calls = self.wait_for(lambda: self.systemctl_calls()[before_calls:] or None, 10)
        self.check("Menu: Shut Down… reaches the fake systemctl too",
                   new_calls and new_calls[-1].strip() == "poweroff", f"calls={new_calls}")

    def menu_logout(self) -> None:
        """Log Out takes the same `quit_all_then` path but ends by running
        `niri msg action quit --skip-confirmation` for real instead of a
        fake systemctl (there is nothing to fake it with, and doing so is
        harmless here: it only ends this test's own nested niri). Must be
        the last scenario in a run — niri exiting ends everything after it."""

        logo = self.wait_for(lambda: self.find_button("menu"), 10, 0.3)
        self.check("Log Out: the logo menu is on the bar (AT-SPI)", logo)
        if logo is None:
            return
        logo.queryAction().doAction(0)
        item = self.wait_for(lambda: self.find_menu_item("Log Out"), 10, 0.3)
        self.check("Log Out: the system menu lists Log Out…", item)
        if item is None:
            return
        item.queryAction().doAction(0)
        button = self.wait_for(lambda: self.find_button("Log Out"), 10, 0.3)
        self.check("Log Out: the confirmation shows its own Log Out button", button)
        if button is None:
            return
        button.queryAction().doAction(0)
        self.check("Log Out: niri actually quits (no systemctl involved)",
                   self.wait_for(lambda: self.children[1].poll() is not None, 15))

    # -- the run ---------------------------------------------------------------

    def run(self) -> int:
        self.start()
        self.shutdown_cycle("Shut Down", "poweroff")
        self.shutdown_cycle("Restart", "reboot")
        self.blocked_then_recovers()
        self.menu_shutdown()
        self.menu_logout()
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
    for tool in ("sway", "grim", "dbus-run-session", "busctl"):
        if subprocess.run(["which", tool], capture_output=True).returncode != 0:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-shutdown-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)
        fakebin = work / "fakebin"
        fakebin.mkdir(exist_ok=True)
        systemctl_log = work / "systemctl-calls.log"
        systemctl_log.touch()
        script = fakebin / "systemctl"
        script.write_text(f'#!/bin/sh\necho "$@" >> "{systemctl_log}"\nexit 0\n')
        script.chmod(0o755)
        env["PATH"] = f"{fakebin}:{env['PATH']}"
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
        print(f"\nsystemctl calls: {systemctl_log.read_text().splitlines()}")
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
    parser.add_argument("--bin-dir", help="directory with this branch's top-bar, dock, rmac-shortcut-dispatch")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--gtk-window", help=argparse.SUPPRESS)
    parser.add_argument("--minimize-after", type=float, help=argparse.SUPPRESS)
    parser.add_argument("--refuse-close", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.gtk_window:
        return gtk_window(args.gtk_window, args.minimize_after, args.refuse_close)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
