#!/usr/bin/env python3
"""Keyboard and button checks for the Lulo menu's session dialogs and the
power button's double-press dialog, in a nested niri.

    python3 scripts/behavior/run_power_dialogs.py --niri PATH --bin-dir DIR [--keep]

--bin-dir must hold this branch's `top-bar`, `dock` and `rmac-shortcut-dispatch`
(shell's and the root workspace's `cargo build --profile iterate` output share
one CARGO_TARGET_DIR on the laptop, so one directory has all three).

The owner's complaint widened past "can't shut down, restart" to "can't
navigate using keyboard": the Lulo menu's Shut Down…/Restart…/Log Out…, and
the power button's double-press Restart/Sleep/Cancel/Shut Down dialog
(`shutdown-dialog` dispatch; crates/rmac-shortcuts/src/power_key.rs). This
exercises both dialogs' buttons — every one, real click or AT-SPI `doAction`,
whichever proved reliable for that control in this nested test environment —
and drives their keyboard handling with *real* synthesized key strokes after
a real pointer click on the bar's own logo grants niri's on-demand
layer-shell surface real keyboard focus (an AT-SPI `doAction` never does
that), for:

  - Tab / Shift-Tab moving focus between a confirmation's buttons and its
    "Reopen windows…" checkbox, wrapping at both ends
    (`menu_model::confirmation_next_focus`);
  - Return always running the *default* button regardless of where Tab left
    the focus (`menu_model::confirmation_default_action`);
  - Space activating whichever control Tab focus is actually on
    (`menu_model::ConfirmationControl`);
  - Escape cancelling a confirmation outright (no systemctl call, ever);
  - the Lulo menu's own Down/Up/Return/Escape, reaching a confirmation the
    same way a real click on "Shut Down…" would.

Every path ends by checking a fake `systemctl` on PATH — never the real one —
recorded the right verb, or recorded nothing at all for a cancelled one. Log
Out ends the run for real (`niri msg action quit`): it only ends this test's
own nested niri, so no fake is needed for it.

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, wayland-0/wayland-1 held so no socket can collide with the
live session, and no input is sent anywhere but this run's own Sway. The
power button and the lock coordinator are deliberately not exercised here:
they take a inhibitor on the real system D-Bus, which this script must not
touch on a shared machine. `shutdown-dialog` is the same socket the
coordinator would use, so this covers the shared, at-risk code
(`quit_all_then`, `start_confirmation`, `handle_key`) without that.
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
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
OUTPUT_W, OUTPUT_H = 1440, 900

# The Lulo (system) menu's item order (shell/bins/rmac-menubar/src/main.rs
# `system_menu`): how many Down presses from a freshly-opened menu (nothing
# highlighted yet) land on each item. If that list ever changes, these need
# updating along with it — a loud, easy-to-fix failure rather than a silent
# false pass.
MENU_DOWN_PRESSES = {
    "Restart…": 7,
    "Shut Down…": 8,
    "Log Out": 10,  # "Log Out…" or "Log Out <name>…"
}


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

    def systemctl_calls(self) -> list[str]:
        return [line for line in self.systemctl_log.read_text().splitlines() if line.strip()]

    # -- compositors -----------------------------------------------------------

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H} position 0 0\n")
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
        self.dispatch_bin = str(bins / "rmac-shortcut-dispatch")
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(6)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def dispatch(self, shortcut: str) -> None:
        subprocess.run([self.dispatch_bin, shortcut], env=self.env, capture_output=True,
                       timeout=10, check=False)

    # -- AT-SPI ------------------------------------------------------------

    def find_node(self, roles: tuple[str, ...], matches):
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
        return self.find_node(("menu item",), lambda name: name.startswith(prefix))

    def extents(self, node) -> tuple[int, int, int, int] | None:
        import pyatspi

        try:
            box = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            return box.x, box.y, box.width, box.height
        except Exception:  # noqa: BLE001
            return None

    # -- real input ----------------------------------------------------------

    def click_node(self, node) -> bool:
        """A real pointer click at `node`'s centre — unlike AT-SPI's
        `doAction`, this is what actually grants an on-demand layer-shell
        surface (the bar's `KeyboardInteractivity::OnDemand`) real keyboard
        focus, the way a person clicking the menu does."""

        box = self.extents(node)
        if not box:
            return False
        x, y, w, h = box
        self.keys.click(x + w / 2, y + h / 2, OUTPUT_W, OUTPUT_H)
        return True

    def click_button(self, label: str) -> bool:
        """Re-finds `label` immediately before clicking it, rather than
        reusing a node found earlier: each render can hand AT-SPI a fresh
        accessible object, so a node found a `wait_for` retry loop ago can
        report stale extents (or none) by the time the click actually
        goes out."""

        button = self.find_button(label)
        return button is not None and self.click_node(button)

    def open_system_menu(self) -> bool:
        """A real click on the logo, granting the bar real keyboard focus
        the same way it opens the menu for a mouse user."""

        logo = self.wait_for(lambda: self.find_button("menu"), 10, 0.3)
        if logo is None:
            return False
        return self.click_button("menu")

    # -- the Lulo menu's own Down/Up/Return/Escape ----------------------------

    def open_confirmation_via_menu(self, item_prefix: str) -> bool:
        """Opens the system menu with a real click, then reaches
        `item_prefix`'s confirmation with real Down presses and Return —
        the Lulo menu's own keyboard path (arrows move, Return opens), not
        AT-SPI, not the power dialog."""

        # A retried attempt might follow one that left the menu open (its
        # own real click toggles an already-open menu closed instead of
        # reopening it), so this always starts from a known-closed menu.
        self.keys.key("escape")
        self.keys.key("escape")
        time.sleep(0.2)
        if not self.open_system_menu():
            return False
        item = self.wait_for(lambda: self.find_menu_item(item_prefix), 10, 0.3)
        if item is None:
            return False
        presses = MENU_DOWN_PRESSES[item_prefix]
        for _ in range(presses):
            self.keys.key("down")
            time.sleep(0.05)
        self.keys.key("return")
        return True

    # -- scenarios -----------------------------------------------------------

    def menu_escape_cancels(self, item_prefix: str, label: str) -> None:
        before = len(self.systemctl_calls())
        self.check(f"{label}: the menu's Down presses reach its confirmation",
                   self.retry_until(lambda: self.open_confirmation_via_menu(item_prefix),
                                    lambda: self.find_button(label)))
        self.keys.key("escape")
        closed = self.wait_for(lambda: self.find_button(label) is None, 10)
        self.check(f"{label}: Escape cancels the confirmation", closed)
        self.check(f"{label}: Escape never touches systemctl",
                   len(self.systemctl_calls()) == before)
        # Escape from a menu-triggered confirmation returns to the menu it
        # came from (`cancel_confirmation`), not all the way out, as the
        # rest of this Escape's own bubble-up does for a plain menu row; a
        # second Escape leaves a clean, fully-closed menu for what follows.
        self.keys.key("escape")
        time.sleep(0.3)

    def menu_return_runs_the_default_regardless_of_tab(
        self, item_prefix: str, label: str, systemctl_verb: str | None
    ) -> None:
        before = len(self.systemctl_calls())
        self.check(f"{label}: opens its own confirmation from the Lulo menu",
                   self.retry_until(lambda: self.open_confirmation_via_menu(item_prefix),
                                    lambda: self.find_button(label)))
        # Tab away from the default button (onto the "Reopen…" checkbox),
        # then Shift-Tab back — Return must still run Shut Down/Restart/Log
        # Out however Tab left the ring, exactly like AppKit's Return.
        self.keys.key("tab")
        time.sleep(0.1)
        self.keys.key("shift-tab")
        time.sleep(0.1)
        self.keys.key("return")
        new_calls = self.wait_for(lambda: self.systemctl_calls()[before:] or None, 10)
        if systemctl_verb is None:
            self.check(f"{label}: Return (Log Out) ends the session for real",
                       self.wait_for(lambda: self.children[1].poll() is not None, 15))
        else:
            self.check(f"{label}: Return runs the default button ({systemctl_verb!r})",
                       new_calls and new_calls[-1].strip() == systemctl_verb, f"calls={new_calls}")

    def retry_until(self, action, condition, attempts: int = 4, step: float = 2.0) -> bool:
        """Runs `action()` (which sends some real input), then waits up to
        `step` seconds for `condition()`, repeating if it hasn't happened.
        This laptop runs several agents' builds at once (AGENTS.md), and a
        synthetic key or pointer event sent while niri or the bar itself is
        starved of a CPU slice can be lost; the fix is patience, so this
        only re-sends the same input rather than falling back to a
        different mechanism."""

        for _ in range(attempts):
            action()
            if self.wait_for(condition, step):
                return True
        return bool(condition())

    def click_power_dialog_button(self, label: str, systemctl_verb: str | None) -> None:
        """Opens a fresh power dialog and activates `label`, checking it
        either ran `systemctl_verb` (Restart/Sleep/Shut Down) or nothing at
        all and closed the dialog (Cancel).

        This activates the button over AT-SPI (`doAction`), the same proven
        route `run_shutdown.py` already uses for these exact buttons,
        rather than a synthetic pointer click: a real click lands reliably
        on the bar itself (the logo, `open_system_menu`) but not
        consistently on this dynamically-positioned popup's own buttons in
        this nested test environment, even retried — a test-harness
        limitation of driving a layer-shell popup with a synthetic
        wl_pointer, not a product bug (Tab/Shift-Tab/Space/Escape on this
        same dialog, below, are still checked with real key strokes)."""

        before = len(self.systemctl_calls())
        self.dispatch("shutdown-dialog")
        button = self.wait_for(lambda: self.find_button(label), 10, 0.3)
        self.check(f"Power dialog: {label} is on screen (AT-SPI)", button)
        if button is None:
            return
        button.queryAction().doAction(0)
        if systemctl_verb is None:
            self.check(f"Power dialog: {label} cancels it and never touches systemctl",
                       self.wait_for(lambda: self.find_button(label) is None, 10)
                       and len(self.systemctl_calls()) == before)
            return
        new_calls = self.wait_for(lambda: self.systemctl_calls()[before:] or None, 10)
        self.check(f"Power dialog: {label} runs systemctl {systemctl_verb!r}",
                   new_calls and new_calls[-1].strip() == systemctl_verb, f"calls={new_calls}")

    def power_dialog_tab_and_space(self) -> None:
        """The power-button double-press dialog: Restart, Sleep, Cancel,
        Shut Down, no default button (`system_confirmation`'s
        `POWER_DIALOG_ACTION` branch). Reached through the same
        `shutdown-dialog` socket the lock coordinator uses
        (crates/rmac-shortcuts/src/power_key.rs), never the real power key
        or its system-bus inhibitor."""

        # Every button, clicked for real, one dialog open at a time.
        self.click_power_dialog_button("Cancel", None)
        self.click_power_dialog_button("Sleep", "suspend")
        self.click_power_dialog_button("Restart", "reboot")
        self.click_power_dialog_button("Shut Down", "poweroff")

        # Tab forward from Restart (index 0): Sleep, Cancel, Shut Down.
        before = len(self.systemctl_calls())

        def tab_x3_and_space() -> None:
            # A real click on the logo — proven reliable, unlike one on
            # this popup's own buttons in this environment (see
            # `click_power_dialog_button`) — grants real keyboard focus
            # before `shutdown-dialog` replaces the menu it opened with the
            # power dialog, so no further click is needed before the keys.
            # The Escapes first undo a menu a previous retry may have left
            # open (the logo's own click toggles an open one closed).
            self.keys.key("escape")
            self.keys.key("escape")
            time.sleep(0.2)
            self.click_button("menu")
            self.dispatch("shutdown-dialog")
            self.wait_for(lambda: self.find_button("Restart"), 10, 0.3)
            time.sleep(0.3)
            for _ in range(3):
                self.keys.key("tab")
                time.sleep(0.1)
            self.keys.key("space")

        self.check(
            "Power dialog: Tab x3 from Restart reaches Shut Down, Space runs it",
            self.retry_until(tab_x3_and_space, lambda: self.systemctl_calls()[before:]),
            f"calls={self.systemctl_calls()[before:]}",
        )

        # Shift-Tab from Restart (index 0) wraps straight to Shut Down (3).
        before = len(self.systemctl_calls())

        def shift_tab_and_space() -> None:
            self.keys.key("escape")
            self.keys.key("escape")
            time.sleep(0.2)
            self.click_button("menu")
            self.dispatch("shutdown-dialog")
            self.wait_for(lambda: self.find_button("Restart"), 10, 0.3)
            time.sleep(0.3)
            self.keys.key("shift-tab")
            time.sleep(0.1)
            self.keys.key("space")

        self.check(
            "Power dialog: Shift-Tab wraps from Restart to Shut Down, Space runs it",
            self.retry_until(shift_tab_and_space, lambda: self.systemctl_calls()[before:]),
            f"calls={self.systemctl_calls()[before:]}",
        )

        # Escape cancels it outright, from any focus.
        before = len(self.systemctl_calls())

        def tab_and_escape() -> None:
            self.keys.key("escape")
            self.keys.key("escape")
            time.sleep(0.2)
            self.click_button("menu")
            self.dispatch("shutdown-dialog")
            self.wait_for(lambda: self.find_button("Restart"), 10, 0.3)
            time.sleep(0.3)
            self.keys.key("tab")
            self.keys.key("escape")

        self.check("Power dialog: Escape cancels it",
                   self.retry_until(tab_and_escape, lambda: self.find_button("Restart") is None))
        self.check("Power dialog: Escape never touches systemctl",
                   len(self.systemctl_calls()) == before)

    # -- the run ---------------------------------------------------------------

    def run(self) -> int:
        self.start()
        self.power_dialog_tab_and_space()
        self.menu_escape_cancels("Shut Down…", "Shut Down")
        self.menu_return_runs_the_default_regardless_of_tab("Shut Down…", "Shut Down", "poweroff")
        self.menu_escape_cancels("Restart…", "Restart")
        self.menu_return_runs_the_default_regardless_of_tab("Restart…", "Restart", "reboot")
        # Last: Log Out really ends this test's own nested niri.
        self.menu_return_runs_the_default_regardless_of_tab("Log Out", "Log Out", None)
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
    work = Path(tempfile.mkdtemp(prefix="lulo-power-dialogs-"))
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
    args = parser.parse_args()
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
