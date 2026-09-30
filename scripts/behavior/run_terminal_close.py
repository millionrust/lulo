#!/usr/bin/env python3
"""Terminal's red traffic light must close the window, in a nested niri.

    python3 scripts/behavior/run_terminal_close.py --niri PATH --bin-dir DIR [--keep]

--bin-dir must hold this branch's `rmac-terminal`.

The owner's report: clicking the red close button on a Terminal window does
nothing. Root cause was `RequestClose` only being handled on `#terminal-grid`
(a sibling of the title bar, Find panel and pickers in the render tree), so
the action never reached its handler unless the grid itself happened to hold
keyboard focus. This exercises the real fix end to end:

  1. no running foreground job: the close button closes the window outright,
     with no dialog (mirrors Terminal.app);
  2. a running foreground job (`sleep 30`): the close button shows the
     "Do you want to terminate running processes in this window?" review;
     Cancel leaves the window open and the job running; a second close +
     Terminate closes the window.

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, wayland-0/wayland-1 held so no socket can collide with the
live session, and no input is sent anywhere but this run's own headless Sway.
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
sys.path.insert(0, str(REPO / "scripts"))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402
import atspi_assert_support as atspi  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
TERMINAL_APP_ID = "org.rmac.Terminal"


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args, self.work = args, work
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.logs = work / "logs"
        self.logs.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []
        self.results: list[tuple[str, bool, str]] = []

    def check(self, name: str, ok, detail: str = "") -> None:
        self.results.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name} {detail}".rstrip(), flush=True)

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(argv, env=env, stdout=open(self.logs / f"{name}.log", "w"),
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

    # -- compositor ---------------------------------------------------------

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.logs / "sway.conf"
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
        config = self.logs / "niri.kdl"
        config.write_text(shell)
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
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, capture_output=True, timeout=10, check=False)
        time.sleep(4)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    def niri(self, *request: str):
        result = subprocess.run([self.args.niri, "msg", "-j", *request], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def terminal_window(self):
        return next((w for w in self.niri("windows") or []
                     if w.get("app_id") == TERMINAL_APP_ID), None)

    def close_button(self):
        return next(iter(atspi.nodes_with(
            atspi.pyatspi.Registry.getDesktop(0), "push button", "Close window")), None)

    def dialog_button(self, label: str):
        return next(iter(atspi.nodes_with(
            atspi.pyatspi.Registry.getDesktop(0), "push button", label)), None)

    def launch_terminal(self) -> subprocess.Popen:
        bins = Path(self.args.bin_dir)
        process = self.spawn([str(bins / "rmac-terminal")], f"terminal-{len(self.children)}",
                             {"VK_ICD_FILENAMES": LAVAPIPE})
        window = self.wait_for(self.terminal_window, 30)
        self.check("Terminal window mapped", window)
        if window:
            # Let the shell inside the PTY reach its prompt before any
            # input is sent to it.
            time.sleep(2)
        return process

    # -- the checks -----------------------------------------------------

    def run(self) -> int:
        self.start()

        # 1. No running job: the close button closes the window outright.
        terminal = self.launch_terminal()
        button = self.wait_for(self.close_button, 15, 0.5)
        self.check("the close (traffic-light) button is on AT-SPI", button, str(button))
        if button is not None:
            button.queryAction().doAction(0)
        self.check("closing with no running job closes the window immediately",
                   self.wait_for(lambda: self.terminal_window() is None, 10))
        if terminal.poll() is None:
            terminal.terminate()
            terminal.wait(5)

        # 2. A running foreground job: the close button reviews first.
        terminal = self.launch_terminal()
        self.keys.type_text("sleep 30")
        self.keys.key("enter")
        time.sleep(1.5)  # let bash exec `sleep`, so it owns the PTY's foreground process group

        button = self.wait_for(self.close_button, 15, 0.5)
        if button is not None:
            button.queryAction().doAction(0)
        terminate = self.wait_for(lambda: self.dialog_button("Terminate"), 10, 0.5)
        self.check("closing a window with a running job asks to terminate it first", terminate)
        self.check("the window is still open while the review is up", self.terminal_window())

        cancel = self.dialog_button("Cancel")
        self.check("the review offers Cancel", cancel, str(cancel))
        if cancel is not None:
            cancel.queryAction().doAction(0)
        time.sleep(0.5)
        self.check("Cancel leaves the window open and the job running", self.terminal_window())

        button = self.wait_for(self.close_button, 15, 0.5)
        if button is not None:
            button.queryAction().doAction(0)
        terminate = self.wait_for(lambda: self.dialog_button("Terminate"), 10, 0.5)
        if terminate is not None:
            terminate.queryAction().doAction(0)
        self.check("Terminate closes the window and ends the job",
                   self.wait_for(lambda: self.terminal_window() is None, 10))
        if terminal.poll() is None:
            terminal.terminate()
            terminal.wait(5)

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
    work = Path(tempfile.mkdtemp(prefix="lulo-terminal-close-"))
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
    parser.add_argument("--bin-dir", help="directory with this branch's rmac-terminal")
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
