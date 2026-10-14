#!/usr/bin/env python3
"""Touchscreen input reaches GPUI as clicks, scrolls, pinches and drags (ADR 0028).

    python3 scripts/behavior/run_touch_input.py --probe PATH [--keep]

PATH is the touch probe (`shell/probes/touch`, built with
`cargo build --features wayland -p rmac-shell-touch-probe` in shell/). It is
a full-screen GPUI window on the vendored Wayland backend whose regions log
what each touch became.

No wlroots compositor can inject touch without a real device (niri nested,
Sway and cage have no virtual-touch protocol, and /dev/uinput is root's), so
this runner starts a private headless GNOME Shell (Mutter) and injects
touches through Mutter's RemoteDesktop D-Bus API, which delivers real
`wl_touch` events to the window. Everything runs on a private session bus
with a temporary HOME and XDG_RUNTIME_DIR, never in the live session.

Checks, each on the window's own log:

1. a tap is a left click, and a second quick tap a double click;
2. a long press is a right click, with no left click;
3. after a touch, a small button answers a tap just outside its edge;
4. a real pointer move ends touch mode: the same spot no longer clicks it;
5. a swipe scrolls the list naturally, then glides on (kinetic momentum);
6. a resting finger then a move drags a list row instead of scrolling;
7. a finger on a `touch_drag` slider drags it straight away;
8. two fingers moving together scroll, and glide on after lifting;
9. two fingers spreading pinch (scale grows), closing pinch back;
10. a quick two-finger tap is a right click;
11. three fingers do nothing.

The Dock, menu bar and other shell surfaces are layer-shell surfaces, which
Mutter does not offer, but they take touch through this same backend code;
window moves by touch (title bars) go through niri's own touch move grab.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402

WIDTH, HEIGHT = 1280, 800
DISPLAY = "wayland-touch"
RD = "org.gnome.Mutter.RemoteDesktop"
SC = "org.gnome.Mutter.ScreenCast"
BTN_LEFT = 0x110


class Touch:
    """Mutter RemoteDesktop: touch and pointer into the monitor's stream."""

    def __init__(self) -> None:
        import dbus

        self.dbus = dbus
        bus = dbus.SessionBus()
        remote = bus.get_object(RD, "/org/gnome/Mutter/RemoteDesktop")
        self.session = bus.get_object(RD, remote.CreateSession(dbus_interface=RD))
        session_id = self.session.Get(RD + ".Session", "SessionId",
                                      dbus_interface="org.freedesktop.DBus.Properties")
        cast = bus.get_object(SC, "/org/gnome/Mutter/ScreenCast")
        cast_session = bus.get_object(
            SC, cast.CreateSession({"remote-desktop-session-id": session_id}, dbus_interface=SC))
        config = bus.get_object("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig")
        state = config.GetCurrentState(dbus_interface="org.gnome.Mutter.DisplayConfig")
        connector = str(state[1][0][0][0])
        self.stream = cast_session.RecordMonitor(connector, {}, dbus_interface=SC + ".Session")
        self.iface = RD + ".Session"
        self.session.Start(dbus_interface=self.iface)
        time.sleep(1.5)

    def down(self, slot: int, x: float, y: float) -> None:
        self.session.NotifyTouchDown(self.stream, self.dbus.UInt32(slot), float(x), float(y),
                                     dbus_interface=self.iface)

    def move(self, slot: int, x: float, y: float) -> None:
        self.session.NotifyTouchMotion(self.stream, self.dbus.UInt32(slot), float(x), float(y),
                                       dbus_interface=self.iface)

    def up(self, slot: int) -> None:
        self.session.NotifyTouchUp(self.dbus.UInt32(slot), dbus_interface=self.iface)

    def pointer(self, x: float, y: float) -> None:
        self.session.NotifyPointerMotionAbsolute(self.stream, float(x), float(y),
                                                 dbus_interface=self.iface)

    def button(self, pressed: bool) -> None:
        self.session.NotifyPointerButton(BTN_LEFT, pressed, dbus_interface=self.iface)

    def tap(self, x: float, y: float, hold: float = 0.05) -> None:
        self.down(0, x, y)
        time.sleep(hold)
        self.up(0)

    def swipe(self, fingers: list[tuple[float, float, float, float]], steps: int = 12,
              step_time: float = 0.016, rest: float = 0.0, lift: bool = True) -> None:
        """Each finger moves from (x0, y0) to (x1, y1) together."""
        for slot, (x0, y0, _x1, _y1) in enumerate(fingers):
            self.down(slot, x0, y0)
        time.sleep(rest or step_time)
        for step in range(1, steps + 1):
            for slot, (x0, y0, x1, y1) in enumerate(fingers):
                self.move(slot, x0 + (x1 - x0) * step / steps, y0 + (y1 - y0) * step / steps)
            time.sleep(step_time)
        if lift:
            for slot in range(len(fingers)):
                self.up(slot)


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
        self.log = work / "touch.log"
        self.seen = 0

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

    def lines(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def step(self, settle: float = 0.6) -> list[dict[str, str]]:
        """The probe's events since the last step, parsed into fields."""
        time.sleep(settle)
        lines = self.lines()
        new, self.seen = lines[self.seen:], len(lines)
        events = []
        for line in new:
            parts = line.split()
            event = {"t": parts[0], "event": parts[1]}
            for part in parts[2:]:
                key, _, value = part.partition("=")
                event[key] = value
            events.append(event)
        with open(self.out / "steps.log", "a") as handle:
            handle.write("\n".join(new) + "\n--\n")
        return events

    @staticmethod
    def named(events, name: str, **fields) -> list[dict[str, str]]:
        return [e for e in events if e["event"] == name
                and all(e.get(key) == value for key, value in fields.items())]

    def start(self) -> None:
        self.spawn(["pipewire"], "pipewire")
        time.sleep(1)
        self.spawn(["wireplumber"], "wireplumber")
        time.sleep(1)
        self.spawn(["gnome-shell", "--headless", "--no-x11", "--virtual-monitor",
                    f"{WIDTH}x{HEIGHT}", "--wayland-display", DISPLAY], "gnome-shell",
                   {"XDG_CURRENT_DESKTOP": "GNOME", "XDG_SESSION_DESKTOP": "gnome"})
        if not self.wait_for(lambda: (self.runtime / DISPLAY).exists(), 60):
            raise SystemExit("gnome-shell did not start")
        time.sleep(3)
        self.spawn([self.args.probe], "probe",
                   {"WAYLAND_DISPLAY": DISPLAY, "RMAC_TOUCH_PROBE_LOG": str(self.log),
                    "RMAC_TOUCH_TRACE": "1"})
        ready = self.wait_for(lambda: any(" ready " in f" {line} " for line in self.lines()), 40)
        if not ready:
            raise SystemExit("the touch probe did not draw")
        ready_line = next(line for line in self.lines() if " ready " in f" {line} ")
        self.check("the probe fills the screen", f"width={WIDTH}" in ready_line
                   and f"height={HEIGHT}" in ready_line, ready_line)
        time.sleep(1)
        self.touch = Touch()
        self.step(0.3)
        # Mutter adds its virtual touchscreen, and the seat's touch
        # capability, on the first injected touch; the window binds wl_touch
        # only then, so the first few touches are lost. Tap until one lands.
        landed = None
        for attempt in range(1, 21):
            self.touch.tap(140, 90)
            if self.named(self.step(0.5), "click", target="tap"):
                landed = attempt
                break
        if not landed:
            raise SystemExit("no touch reached the probe")
        print(f"touch reached the probe after {landed} tap(s)", flush=True)
        time.sleep(0.6)

    def run(self) -> int:
        try:
            self.start()
            self.scenarios()
        except SystemExit as error:
            self.check("setup", False, str(error))
        return self.finish()

    def scenarios(self) -> None:
        touch = self.touch
        # 1. Tap, then a quick second tap: click count 1, then 2.
        touch.tap(140, 90)
        time.sleep(0.15)
        touch.tap(142, 92)
        events = self.step()
        clicks = self.named(events, "click", target="tap")
        self.check("a tap is a left click, a second tap a double click",
                   [c.get("count") for c in clicks] == ["1", "2"]
                   and all(c.get("touch") == "true" for c in clicks), f"{clicks}")
        time.sleep(0.6)
        # 2. Long press: right click only.
        touch.tap(140, 90, hold=0.8)
        events = self.step()
        self.check("a long press is a right click, not a left click",
                   self.named(events, "right", target="tap")
                   and not self.named(events, "click"), f"{events}")
        # 3. The small button (120,200 16x16) answers 4 px past its right edge.
        touch.tap(140, 208)
        events = self.step()
        self.check("after a touch, a small button answers a tap just outside it",
                   self.named(events, "click", target="small"), f"{events}")
        # 4. A pointer that moves ends touch mode: the same spot misses.
        touch.pointer(900, 700)
        time.sleep(0.1)
        touch.pointer(140, 208)
        time.sleep(0.4)
        touch.button(True)
        time.sleep(0.05)
        touch.button(False)
        events = self.step()
        self.check("a pointer click there misses: targets shrink back for the mouse",
                   not self.named(events, "click", target="small"), f"{events}")
        touch.pointer(128, 208)
        time.sleep(0.2)
        touch.button(True)
        time.sleep(0.05)
        touch.button(False)
        events = self.step()
        self.check("the pointer still clicks the button itself, with no slop",
                   self.named(events, "click", target="small", slop="0", touch="false"), f"{events}")
        touch.pointer(1200, 780)
        # 5. A one-finger swipe up the list scrolls down, then glides on.
        touch.swipe([(520, 560, 520, 260)], steps=12)
        events = self.step(1.5)
        scrolls = self.named(events, "scroll", target="list")
        phases = [e["phase"] for e in scrolls]
        ended = phases.index("Ended") if "Ended" in phases else -1
        glide = [e for e in scrolls[ended + 1:] if e["phase"] == "Moved"] if ended >= 0 else []
        offset = max((float(e["offset"]) for e in scrolls), default=0.0)
        self.check("a swipe scrolls the list with the finger (natural)",
                   phases[:1] == ["Started"] and ended > 0 and all(float(e["dy"]) <= 0 for e in scrolls),
                   f"phases={phases[:4]}... dy={[e['dy'] for e in scrolls[:3]]}")
        self.check("the lifted swipe glides on (kinetic momentum)", len(glide) >= 3,
                   f"{len(glide)} momentum steps, offset {offset}")
        self.check("no row drag starts from a quick swipe", not self.named(events, "drag-start"))
        # 6. Rest, then move: the row under the finger drags.
        touch.swipe([(520, 300, 520, 420)], steps=10, rest=0.3)
        events = self.step()
        self.check("a resting finger then a move drags a list row",
                   self.named(events, "drag-start")
                   and not self.named(events, "scroll"), f"{events[:4]}")
        # 7. The slider drags straight away.
        touch.swipe([(60, 320, 220, 320)], steps=10)
        events = self.step()
        moves = self.named(events, "slider-move")
        self.check("a finger drags a touch_drag slider straight away",
                   self.named(events, "slider-down") and moves and self.named(events, "slider-up")
                   and float(moves[-1]["x"]) >= 200 and not self.named(events, "scroll"),
                   f"{events[:2]} ... {events[-2:]}")
        # 8. Two fingers moving up together scroll and glide.
        touch.swipe([(470, 560, 470, 300), (570, 560, 570, 300)], steps=12)
        events = self.step(1.5)
        scrolls = self.named(events, "scroll", target="list")
        phases = [e["phase"] for e in scrolls]
        ended = phases.index("Ended") if "Ended" in phases else -1
        glide = [e for e in scrolls[ended + 1:] if e["phase"] == "Moved"] if ended >= 0 else []
        self.check("two fingers moving together scroll the list",
                   phases[:1] == ["Started"] and ended > 0
                   and not self.named(events, "drag-start") and not self.named(events, "click"),
                   f"phases={phases[:3]}...")
        self.check("a lifted two-finger scroll glides on", len(glide) >= 3, f"{len(glide)} steps")
        # 9. Spread, then close: the pinch grows by about the spread ratio.
        touch.swipe([(900, 240, 840, 240), (1020, 240, 1080, 240)], steps=12)
        events = self.step()
        pinches = self.named(events, "pinch", target="pinch")
        grown = sum(float(e["delta"]) for e in pinches)
        self.check("two fingers spreading pinch out",
                   [e["phase"] for e in pinches][:1] == ["Started"]
                   and pinches and pinches[-1]["phase"] == "Ended" and 0.8 <= grown <= 1.2,
                   f"scale change {grown:.3f} over {len(pinches)} events")
        touch.swipe([(840, 240, 900, 240), (1080, 240, 1020, 240)], steps=12)
        events = self.step()
        shrunk = sum(float(e["delta"]) for e in self.named(events, "pinch", target="pinch"))
        self.check("two fingers closing pinch back in", -0.6 <= shrunk <= -0.4, f"{shrunk:.3f}")
        # 10. A quick two-finger tap right-clicks.
        touch.down(0, 900, 300)
        touch.down(1, 960, 300)
        time.sleep(0.08)
        touch.up(0)
        touch.up(1)
        events = self.step()
        self.check("a quick two-finger tap is a right click",
                   self.named(events, "right", target="pinch")
                   and not self.named(events, "click"), f"{events}")
        # 11. Three fingers: nothing.
        touch.swipe([(420, 500, 420, 300), (520, 500, 520, 300), (620, 500, 620, 300)], steps=10)
        events = self.step()
        self.check("three fingers do nothing", not [e for e in events if e["event"] != "ready"],
                   f"{events[:3]}")

    def finish(self) -> int:
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
    for tool in ("gnome-shell", "pipewire", "wireplumber", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    probe = Path(args.probe).resolve(strict=True)
    work = Path(tempfile.mkdtemp(prefix="lulo-touch-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "WAYLAND_DISPLAY", "DISPLAY"):
            env.pop(key, None)
        # A private bus that activates nothing: gnome-shell's portals, gvfs
        # and calendar helpers stay out of the run.
        services = work / "dbus-services"
        services.mkdir()
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            "<busconfig><type>session</type>"
            f"<listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        argv = [a if a != args.probe else str(probe) for a in argv]
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
    parser.add_argument("--probe", required=True, help="the built touch-probe binary")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
