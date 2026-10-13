#!/usr/bin/env python3
"""The live Lulo robot: drives the owner's real session to find problems.

    RMAC_ROBOT_LIVE_CONSENT=owner-granted-2026-10-13 \\
        python3 scripts/robot/robot.py --live --out ~/lulo-robot-runs/RUN \\
        [--apps Calculator,Clock | --apps all] [--shell] [--max-seconds 540]
    python3 scripts/robot/robot.py --probe      # read-only checks, no input

Run it on the reference laptop, in the owner's session, only with the
owner's permission (granted 2026-10-13 for robot tests). See
docs/behavior-suite.md, "Live robot", for the design and the safety rules;
``safety.py`` holds the deny lists.

For each app it: launches it (timing the first window), screenshots it,
walks every menu-bar menu and submenu through the real menu bar (⌃F2 and
the arrow keys), activates every allowed menu item through the app's own
``org.rmac.AppMenu2`` endpoint (the same call the menu bar makes) and
dismisses what opened, presses the app's main shortcuts, resizes the window
small and large, then quits it. For the shell it walks the Lulo menu, the
status menus, Control Centre, Spotlight, Notification Centre, the Apps
grid, the Dock's right-click menus and the desktop's right-click menu.

It records findings (crashes, panics, hangs, slow opens, items with no
effect, logged errors and screenshot heuristics) in ``report.json`` beside
the screenshots in --out. Screenshots of the owner's screen stay local:
never commit them.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import signal
import subprocess
import sys
import time
import traceback
from pathlib import Path
from typing import Any, Callable, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import live  # noqa: E402
import safety  # noqa: E402
import vision  # noqa: E402

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    Image = None  # type: ignore[assignment]

SCALE = 1.25
SLOW_OPEN_S = 1.0
HANG_S = 2.0

# Per-app plans: how to launch it, its niri app id, and the keyboard
# shortcuts worth pressing (all non-destructive; each is followed by Esc).
APPS: dict[str, dict[str, Any]] = {
    "Files": {"exec": ["/usr/bin/rmac-files", "{scratch}"], "app_id": "org.rmac.Files", "bin": "rmac-files",
              "keys": ["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-1", "cmd-f", "type:robot", "escape", "escape"]},
    "Text Editor": {"exec": ["/usr/bin/rmac-text-editor", "{scratch}/robot.txt"], "app_id": "org.rmac.TextEditor",
                    "bin": "rmac-text-editor", "scratch_edit": True,
                    "keys": ["cmd-end", "type:Robot was here. ", "cmd-a", "cmd-b", "cmd-b", "right",
                             "cmd-f", "type:Robot", "escape", "cmd-=", "cmd--", "cmd-0", "cmd-s"]},
    "Terminal": {"exec": ["/usr/bin/rmac-terminal"], "app_id": "org.rmac.Terminal", "bin": "rmac-terminal",
                 "keys": ["type:echo lulo-robot", "return", "cmd-=", "cmd--", "cmd-0", "cmd-t", "cmd-w", "cmd-k"]},
    "Settings": {"exec": ["/usr/bin/rmac-system-settings"], "app_id": "org.rmac.SystemSettings",
                 "bin": "rmac-system-settings",
                 "keys": ["cmd-f", "type:display", "escape", "escape", "cmd-f", "type:sound", "escape", "escape"]},
    "Notes": {"exec": ["/usr/bin/rmac-notes"], "app_id": "org.rmac.Notes", "bin": "rmac-notes",
              "keys": ["cmd-alt-f", "type:zzz-robot", "escape", "escape"], "notes_flow": True},
    "Weather": {"exec": ["/usr/bin/rmac-weather"], "app_id": "org.rmac.Weather", "bin": "rmac-weather",
                "keys": ["cmd-r"]},
    "Calculator": {"exec": ["/usr/bin/rmac-calculator"], "app_id": "org.rmac.Calculator", "bin": "rmac-calculator",
                   "keys": ["escape", "type:12+30=", "type:7*6=", "escape", "cmd-c"]},
    "Mail": {"exec": ["/usr/bin/rmac-mail"], "app_id": "org.rmac.Mail", "bin": "rmac-mail", "keys": []},
    "Clock": {"exec": ["/usr/bin/rmac-clock"], "app_id": "org.rmac.Clock", "bin": "rmac-clock", "keys": []},
    "Preview": {"exec": ["/usr/bin/rmac-preview", "{scratch}/robot.png"], "app_id": "org.rmac.Preview",
                "bin": "rmac-preview", "keys": ["cmd-=", "cmd--", "cmd-0", "cmd-alt-1", "cmd-alt-2", "cmd-alt-1"]},
    "Calendar": {"exec": ["/usr/bin/rmac-calendar"], "app_id": "org.rmac.Calendar", "bin": "rmac-calendar",
                 "keys": ["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-t", "cmd-3"]},
    "System Monitor": {"exec": ["/usr/bin/rmac-system-monitor"], "app_id": "org.rmac.SystemMonitor",
                       "bin": "rmac-system-monitor", "keys": ["cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-1"]},
}
SPECIAL_MENUS = {"Application", "Window", "Help"}

# niri runs its key binds only for physical keyboards: keys from a
# zwp_virtual_keyboard_v1 go straight to the focused client. So the robot
# reaches the shell's global shortcuts by spawning exactly what the bind
# spawns (packaging/rmac-session/shell.kdl and the generated shortcuts).
DISPATCH = "/usr/libexec/rmac/rmac-shortcut-dispatch"
COMPOSITOR_BINDS = {
    "ctrl-f2": [DISPATCH, "menu-bar-focus"],
    "cmd-space": [DISPATCH, "launcher"],
    "cmd-ctrl-n": [DISPATCH, "notification-center"],
    "ctrl-f3": ["/usr/libexec/rmac/rmac-dock", "focus"],
    "ctrl-up": ["/usr/libexec/rmac/rmac-mission-control", "mission-control"],
}
# Apps the robot may launch with no arguments to push the Dock's recent-apps
# section back into shape (quick to open, no personal data, no Open panel).
RECENTS_FILLERS = ("System Monitor", "Clock", "Calculator", "Weather")  # not kept in the Dock
# Items whose effect is invisible by design (the clipboard).
INVISIBLE_ITEMS = {"Copy", "Copy Path", "Copy Link"}


def now_iso() -> str:
    return dt.datetime.now().strftime("%Y-%m-%d %H:%M:%S")


class Robot:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.env = live.live_env()
        self.niri = live.Niri(self.env)
        self.out = Path(args.out).expanduser()
        self.out.mkdir(parents=True, exist_ok=True)
        self.home = Path.home()
        self.scratch = self.home / "RobotScratch"
        self.findings: list[dict] = []
        self.steps: list[dict] = []
        self.timings: dict[str, float] = {}
        self.started_wall = now_iso()
        self.started = time.monotonic()
        self.deadline = self.started + args.max_seconds
        self.input: Optional[live.Input] = None
        self.shot_index = 0
        self.robot_pids: set[int] = set()
        self.shell_pids: dict[str, Optional[int]] = {}
        self.health: dict[str, int] = {}
        self.owner_count = 0
        self.current_app = "shell"

    # ------------------------------------------------------------ log ----

    def log(self, message: str) -> None:
        line = f"[{time.monotonic() - self.started:7.1f}s] {message}"
        print(line, flush=True)
        with open(self.out / "robot.log", "a") as handle:
            handle.write(line + "\n")

    def step(self, app: str, what: str, **extra: Any) -> None:
        self.steps.append({"t": round(time.monotonic() - self.started, 2), "app": app, "step": what, **extra})

    def finding(self, severity: str, app: str, kind: str, title: str, detail: str = "",
                shot: Optional[Path] = None) -> None:
        entry = {"id": f"R{len(self.findings) + 1:03d}", "severity": severity, "app": app, "kind": kind,
                 "title": title, "detail": detail, "screenshot": str(shot) if shot else None,
                 "t": round(time.monotonic() - self.started, 2)}
        # One finding per (app, kind, title): the robot repeats itself a lot.
        if any(f["app"] == app and f["kind"] == kind and f["title"] == title for f in self.findings):
            return
        self.findings.append(entry)
        self.log(f"FINDING {severity} {app} {kind}: {title} {detail}")
        self.write_report()

    def write_report(self) -> None:
        report = {
            "run": self.out.name, "started": self.started_wall, "finished": now_iso(),
            "build": live.run(["dpkg-query", "-W", "-f", "${Version}", "rmac-apps"], self.env).stdout.strip(),
            "findings": self.findings, "timings_s": self.timings, "steps": self.steps,
        }
        (self.out / "report.json").write_text(json.dumps(report, indent=2))

    # ------------------------------------------------------- watching ----

    def shot(self, name: str):
        self.shot_index += 1
        safe = "".join(c if c.isalnum() or c in "-_" else "_" for c in name)[:80]
        path = live.screenshot(self.env, self.out / f"{self.shot_index:04d}-{safe}.png")
        image = Image.open(path)
        image.load()
        return path, image

    def check_session(self) -> None:
        """Abort on owner input, a shell restart or the deadline."""

        count = live.owner_input_count()
        if count != self.owner_count:
            raise live.RobotAbort("the owner touched the keyboard, touchpad or screen")
        for name, pid in self.shell_pids.items():
            if pid is not None and not Path(f"/proc/{pid}").exists():
                self.finding("critical", "shell", "crash", f"{name} exited or restarted", f"pid {pid} is gone")
                raise live.RobotAbort(f"{name} (pid {pid}) is gone")
        health = live.session_health()
        for unit, restarts in health.items():
            if restarts > self.health.get(unit, restarts):
                self.finding("critical", "shell", "crash", f"{unit} restarted", f"restarts {self.health.get(unit)} -> {restarts}")
                raise live.RobotAbort(f"{unit} restarted")

    def over_time(self) -> bool:
        return time.monotonic() > self.deadline

    def key(self, chord: str, pause: float = live.Input.PAUSE) -> None:
        self.check_session()
        assert self.input is not None
        if chord.startswith("type:"):
            self.input.type(chord[5:])
        elif chord in COMPOSITOR_BINDS:
            self.niri.spawn(COMPOSITOR_BINDS[chord])
            time.sleep(max(pause, 0.5))
        else:
            self.input.key(chord, pause)

    # ------------------------------------------------------- baseline ----

    def baseline(self) -> None:
        self.shell_pids = {name: live.live_session_pid(name, self.env) for name in live.SHELL_PROCESSES}
        self.health = live.session_health()
        self.bus_start = live.bus_count(self.env)
        self.windows_start = {w["id"] for w in self.niri.windows()}
        self.volume_start = live.volume(self.env)
        self.brightness_start = live.brightness()
        self.scheme_start = live.color_scheme(self.env)
        self.state_snapshot = live.snapshot_state(self.home)
        self.recents_start = live.dock_recents(self.home)
        if self.args.dock_recents:
            # The list as the owner left it, when an earlier run changed it.
            self.recents_start = [d for d in self.args.dock_recents.split(",") if d]
        self.log(f"baseline: shell {self.shell_pids}, D-Bus names {self.bus_start}, windows {sorted(self.windows_start)}, "
                 f"volume {self.volume_start!r}, brightness {self.brightness_start}, scheme {self.scheme_start}")
        if any(pid is None for pid in self.shell_pids.values()):
            raise live.RobotAbort(f"could not find the live shell processes: {self.shell_pids}")
        if any("lock" in layer.get("namespace", "") for layer in self.niri.layers()):
            raise live.RobotAbort("the screen is locked")

    def make_scratch(self) -> None:
        self.scratch.mkdir(exist_ok=True)
        (self.scratch / "robot.txt").write_text("Lulo robot scratch file.\nSafe to delete.\n")
        if Image is not None:
            image = Image.new("RGB", (640, 400), (40, 120, 200))
            for x in range(0, 640, 40):
                for y in range(0, 400, 40):
                    if (x // 40 + y // 40) % 2:
                        image.paste((240, 200, 60), (x, y, x + 40, y + 40))
            image.save(self.scratch / "robot.png")
        (self.scratch / "Robot Folder").mkdir(exist_ok=True)

    def finish(self) -> None:
        """Put the session back as it was found."""

        for pid in sorted(self.robot_pids):
            if Path(f"/proc/{pid}").exists():
                self.log(f"stopping robot-launched pid {pid}")
                try:
                    os.kill(pid, signal.SIGTERM)
                except OSError:
                    pass
        time.sleep(1.0)
        self.restore_dock_recents()
        touched = live.restore_state(self.home, self.state_snapshot)
        if touched:
            self.log(f"restored state files: {touched}")
        if live.volume(self.env) != self.volume_start:
            self.finding("high", "shell", "state", "volume changed during the run",
                         f"{self.volume_start} -> {live.volume(self.env)}")
        if live.brightness() != self.brightness_start:
            self.finding("high", "shell", "state", "brightness changed during the run",
                         f"{self.brightness_start} -> {live.brightness()}")
        if live.color_scheme(self.env) != self.scheme_start:
            self.finding("high", "shell", "state", "appearance changed during the run",
                         f"{self.scheme_start} -> {live.color_scheme(self.env)}")
        left = {w["id"] for w in self.niri.windows()} - self.windows_start
        if left:
            self.finding("medium", "shell", "state", "robot windows left open", f"window ids {sorted(left)}")
        bus_end = live.bus_count(self.env)
        if bus_end > self.bus_start + 3:
            self.finding("high", "shell", "dbus-growth", "D-Bus connections grew over the run",
                         f"{self.bus_start} -> {bus_end}")
        self.scan_journal()
        self.write_report()

    def restore_dock_recents(self) -> None:
        """Launching apps changes the Dock's recent-apps section. Put it
        back by relaunching the original apps in order (the Dock only reads
        its state file at start), unless the owner is back."""

        by_desktop = {plan["app_id"] + ".desktop": plan for plan in APPS.values()}
        fillers = [APPS[name]["app_id"] + ".desktop" for name in RECENTS_FILLERS]
        for _round in range(2):
            current = live.dock_recents(self.home)
            if current == self.recents_start:
                return
            order = live.recents_launch_order(self.recents_start, current, fillers)
            if not order or live.owner_input_count() != self.owner_count or self.input is None:
                self.finding("medium", "shell", "state", "Dock recent apps changed and were not restored",
                             f"{self.recents_start} -> {current}")
                return
            self.log(f"restoring the Dock's recent apps {self.recents_start} from {current} by launching {order}")
            for desktop in order:
                plan = by_desktop.get(desktop)
                if plan is None:
                    self.finding("medium", "shell", "state", f"cannot relaunch {desktop} to restore Dock recents")
                    return
                if desktop in live.dock_recents(self.home):
                    continue  # launching it would change nothing
                self.relaunch_for_recents(plan, desktop)
        final = live.dock_recents(self.home)
        if final != self.recents_start:
            self.finding("medium", "shell", "state", "Dock recent apps could not be restored",
                         f"{self.recents_start} -> {final}")

    def relaunch_for_recents(self, plan: dict, desktop: str) -> None:
        """Open one app until the Dock has recorded it, then quit it."""

        known = {w["id"] for w in self.niri.windows()}
        self.niri.spawn([plan["exec"][0]])
        window = self.wait_window(plan["app_id"], known)
        if window is None:
            return
        end = time.monotonic() + 5
        while desktop not in live.dock_recents(self.home) and time.monotonic() < end:
            time.sleep(0.2)
        time.sleep(0.5)
        self.focus(window)
        pid = window.get("pid")
        self.input.key("cmd-w" if plan["bin"] == "rmac-files" else "cmd-q", 0.8)
        end = time.monotonic() + 5
        while pid and Path(f"/proc/{pid}").exists() and time.monotonic() < end:
            time.sleep(0.2)
        if pid and Path(f"/proc/{pid}").exists():
            os.kill(pid, signal.SIGTERM)
        time.sleep(1.0)

    def scan_journal(self) -> None:
        lines = live.journal_since(self.env, self.started_wall)
        (self.out / "journal.txt").write_text("\n".join(lines))
        for kind, line in live.classify_journal(lines):
            severity = {"panic": "critical", "crash": "critical", "dbus-limit": "critical"}.get(kind, "low")
            app = "journal"
            for name, plan in APPS.items():
                if plan["bin"] in line:
                    app = name
            self.finding(severity, app, f"journal-{kind}", line.split(": ", 1)[-1][:160], line)

    # ---------------------------------------------------------- apps ----

    def app_windows(self, app_id: str) -> list[dict]:
        return [w for w in self.niri.windows() if w.get("app_id") == app_id]

    def wait_window(self, app_id: str, known: set[int], timeout: float = 12.0) -> Optional[dict]:
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for window in self.app_windows(app_id):
                if window["id"] not in known:
                    return window
            time.sleep(0.03)
        return None

    def focus(self, window: dict) -> None:
        self.niri.action("focus-window", "--id", str(window["id"]))
        time.sleep(0.25)

    def window_shot(self, app: str, name: str, window: Optional[dict]) -> Path:
        path, image = self.shot(f"{app}-{name}")
        if window is not None:
            fresh = next((w for w in self.niri.windows() if w["id"] == window["id"]), window)
            box = live.window_box(fresh, SCALE)
            if box:
                for problem in vision.analyse_panel(image, box):
                    severity = "high" if problem["kind"] in {"black-window", "blank-window"} else "low"
                    self.finding(severity, app, problem["kind"], f"{name}: {problem['kind']}", problem["detail"], path)
        return path

    def run_app(self, name: str) -> None:
        plan = APPS[name]
        self.current_app = name
        app_id = plan["app_id"]
        if live.pids_of(plan["bin"]):
            self.log(f"{name} is already running (the owner's instance?); skipping it")
            self.step(name, "skipped: already running")
            return
        if self.app_windows(app_id):
            self.log(f"{name} already has windows; skipping it")
            return
        bus_before = live.bus_count(self.env)
        known = {w["id"] for w in self.niri.windows()}
        command = [part.replace("{scratch}", str(self.scratch)) for part in plan["exec"]]
        self.check_session()
        started = time.monotonic()
        self.niri.spawn(command)
        window = self.wait_window(app_id, known)
        elapsed = time.monotonic() - started
        if window is None:
            path, _ = self.shot(f"{name}-no-window")
            self.finding("critical", name, "no-window", "no window within 12 s of launch", "", path)
            for pid in live.pids_of(plan["bin"]):
                self.robot_pids.add(pid)
            return
        self.timings[f"{name} first window"] = round(elapsed, 3)
        pid = window.get("pid")
        if pid:
            self.robot_pids.add(pid)
        self.log(f"{name}: first window after {elapsed * 1000:.0f} ms (pid {pid})")
        if elapsed > SLOW_OPEN_S:
            self.finding("medium", name, "slow-open", f"first window after {elapsed * 1000:.0f} ms", "budget 1000 ms")
        time.sleep(1.0)
        self.focus(window)
        self.window_shot(name, "open", window)
        self.window_pid = window.get("pid")
        phases: list[Callable[[], None]] = [
            lambda: self.walk_menus(name, window),
            lambda: self.activate_items(name, window),
            lambda: self.press_keys(name, window, plan.get("keys", [])),
        ]
        if plan.get("notes_flow"):
            phases.append(lambda: self.notes_flow(window))
        phases.append(lambda: self.resize(name, window))
        try:
            for phase in phases:
                if self.over_time() or not self.alive(name, window):
                    break
                phase()
        finally:
            self.quit_app(name, plan, window)
        # Names held by helpers an app started (portals, activated services)
        # take a few seconds to go after it quits: re-sample before judging.
        bus_after = live.bus_count(self.env)
        end = time.monotonic() + 8
        while bus_after > bus_before + 1 and time.monotonic() < end:
            time.sleep(1.0)
            bus_after = live.bus_count(self.env)
        self.step(name, "D-Bus names", before=bus_before, after=bus_after)
        if bus_after > bus_before + 1:
            self.finding("high", name, "dbus-growth", "D-Bus names grew after quitting", f"{bus_before} -> {bus_after}")
        if bus_after > self.bus_start + 6:
            raise live.RobotAbort("D-Bus connections are climbing")

    def alive(self, name: str, window: dict) -> bool:
        pid = window.get("pid")
        if pid and not Path(f"/proc/{pid}").exists():
            path, _ = self.shot(f"{name}-gone")
            self.finding("critical", name, "crash", "the app exited unexpectedly", f"pid {pid}", path)
            return False
        return True

    # -------------------------------------------------------- menus ----

    def bar_titles(self, layout: list[dict]) -> list[dict]:
        """The menu bar's titles in order: Lulo, the app menu, the app's own
        menus, Window, Help (rmac-menubar's `bar_menus`)."""

        middle = [m for m in layout if m["label"] not in SPECIAL_MENUS]
        by_label = {m["label"]: m for m in layout}
        return ([{"label": "Lulo", "items": None}, {"label": "Application", "items": (by_label.get("Application") or {}).get("items")}]
                + middle + [{"label": "Window", "items": (by_label.get("Window") or {}).get("items")},
                            {"label": "Help", "items": None}])

    def walk_menus(self, name: str, window: dict) -> None:
        bus = live.MENU_BUS.get(name)
        if not bus:
            return
        result, elapsed = live.read_layout(self.env, bus)
        if result is None:
            self.finding("high", name, "menu-endpoint", "AppMenu2.Layout did not answer", "")
            return
        _revision, layout = result
        (self.out / f"{name}-menus.json").write_text(json.dumps(layout, indent=1))
        titles = self.bar_titles(layout)
        self.step(name, "menus", titles=[t["label"] for t in titles])
        before, before_image = self.shot(f"{name}-before-menus")
        for index, title in enumerate(titles):
            if self.over_time():
                return
            self.focus(window)
            self.key("ctrl-f2", 0.4)
            for _ in range(index):
                self.key("right", 0.12)
            self.key("down", 0.35)
            opened, opened_image = self.shot(f"{name}-menu-{title['label']}")
            if vision.changed_fraction(before_image, opened_image, (0, 0, opened_image.size[0], opened_image.size[1])) < 0.0005:
                self.finding("high", name, "menu-did-not-open", f"menu {title['label']} did not open from ⌃F2", "", opened)
            items = title.get("items")
            rows = [i for i in items if i["depth"] == 0 and not i["alternate"]] if items else []
            if items is None:
                rows = [{"enabled": True, "submenu": False}] * 6
            for row_index, row in enumerate(rows):
                if not row.get("enabled"):
                    continue
                self.key("down", 0.12)
                if row.get("submenu"):
                    self.key("right", 0.35)
                    self.shot(f"{name}-submenu-{title['label']}-{row.get('label', row_index)}")
                    self.key("left", 0.15)
            self.key("escape", 0.2)
            self.key("escape", 0.3)
            self.check_hang(name, bus, f"after menu {title['label']}")
            if not self.alive(name, window):
                return
        time.sleep(0.4)
        after, after_image = self.shot(f"{name}-after-menus")
        ignore = [(int(1300 * SCALE), 0, after_image.size[0], int(40 * SCALE))]  # the clock
        if name in {"System Monitor", "Clock", "Weather", "Terminal"}:
            box = live.window_box(window, SCALE)
            if box:
                ignore.append(box)
        for problem in vision.residue(before_image, after_image, ignore, min_fraction=0.01):
            self.finding("low", name, "menu-residue", "screen differs after walking the menus", problem["detail"], after)

    def check_hang(self, name: str, bus: Optional[str], when: str) -> None:
        if not bus:
            return
        pid = getattr(self, "window_pid", None)
        if pid and not Path(f"/proc/{pid}").exists():
            return  # gone, not hung: `alive` reports it
        try:
            result, elapsed = live.read_layout(self.env, bus, timeout=10)
        except subprocess.TimeoutExpired:
            result, elapsed = None, 10.0
        if pid and not Path(f"/proc/{pid}").exists():
            return
        if (result is None and elapsed > HANG_S) or elapsed > HANG_S:
            path, _ = self.shot(f"{name}-hang")
            self.finding("critical", name, "hang", f"app did not answer within {HANG_S:.0f} s {when}",
                         f"Layout took {elapsed:.2f} s", path)

    def activate_items(self, name: str, window: dict) -> None:
        bus = live.MENU_BUS.get(name)
        if not bus:
            return
        result, _ = live.read_layout(self.env, bus)
        if result is None:
            return
        _rev, layout = result
        for menu in layout:
            for item in menu["items"]:
                if self.over_time():
                    return
                if item["submenu"]:
                    continue
                allowed, reason = safety.item_allowed(name, menu["label"], item["label"], item["action"], item["enabled"])
                label = " > ".join(item["path"])
                if not allowed:
                    self.step(name, "skip item", item=label, reason=reason)
                    continue
                self.activate_one(name, bus, window, label, item)
                if not self.alive(name, window):
                    return

    def checked_state(self, bus: str) -> dict[str, tuple[bool, str]]:
        result, _ = live.read_layout(self.env, bus)
        if result is None:
            return {}
        return {i["action"]: (i["checked"], i["label"]) for m in result[1] for i in m["items"]}

    def activate_one(self, name: str, bus: str, window: dict, label: str, item: dict) -> None:
        self.check_session()
        self.focus(window)
        known = {w["id"] for w in self.niri.windows()}
        state_before = self.checked_state(bus)
        _, before = self.shot(f"{name}-pre-{label}")
        error = live.activate(self.env, bus, item["action"])
        if error:
            self.finding("medium", name, "activate-error", f"{label}: activation refused", error)
            return
        time.sleep(0.9)
        path, after = self.shot(f"{name}-item-{label}")
        windows = self.niri.windows()
        new = [w for w in windows if w["id"] not in known]
        state_after = self.checked_state(bus)
        self.check_hang(name, bus, f"after {label}")
        box = live.window_box(next((w for w in windows if w["id"] == window["id"]), window), SCALE)
        changed = vision.changed_fraction(before, after)
        toggled = [a for a in state_after if a in state_before and state_after[a] != state_before[a]]
        self.step(name, "item", item=label, changed=round(changed, 4), new_windows=[w["app_id"] for w in new],
                  toggled=toggled)
        already_checked = state_before.get(item["action"], (False, ""))[0]
        if not new and not toggled and changed < 0.0008 and not already_checked \
                and item["label"] not in INVISIBLE_ITEMS:
            self.finding("medium", name, "no-effect", f"{label} did nothing visible",
                         f"changed {changed * 100:.3f}% of the screen, no window, no state change", path)
        if box:
            for problem in vision.blank_window(after, box):
                self.finding("high", name, problem["kind"], f"after {label}", problem["detail"], path)
        for extra in new:
            self.inspect_new_window(name, label, extra)
        self.restore_toggles(name, bus, item, state_before)
        # Whatever the item opened inside the window (a sheet, a find bar,
        # a popover) gets one Esc, which Lulo maps to Cancel.
        if changed > 0.0008 and not toggled:
            self.focus(window)
            self.key("escape", 0.3)
        # Keep the scratch document's text for the items that follow: a
        # Select All left in place would be replaced by the next insert.
        if item["label"] == "Select All":
            self.focus(window)
            self.key("right", 0.2)
        # Tabs the robot opened (Terminal's New Tab ▸ profile) close again.
        if "New Tab" in item["path"][:-1] or item["label"] == "New Tab":
            self.focus(window)
            self.key("cmd-w", 0.5)

    def inspect_new_window(self, name: str, label: str, extra: dict) -> None:
        time.sleep(0.4)
        box = live.window_box(extra, SCALE)
        path, image = self.shot(f"{name}-opened-{label}")
        if box:
            for problem in vision.analyse_panel(image, box):
                severity = "high" if problem["kind"] in {"black-window", "blank-window"} else "low"
                self.finding(severity, name, problem["kind"], f"{label} opened {extra.get('title')!r}: {problem['kind']}",
                             problem["detail"], path)
        self.close_window(name, extra, f"{label} window")

    def close_window(self, name: str, window: dict, what: str) -> None:
        for chord in ("escape", "cmd-w"):
            if not any(w["id"] == window["id"] for w in self.niri.windows()):
                return
            self.focus(window)
            self.key(chord, 0.5)
        if any(w["id"] == window["id"] for w in self.niri.windows()):
            path, _ = self.shot(f"{name}-stuck-{what}")
            self.finding("medium", name, "would-not-close", f"{what} did not close on Esc or ⌘W", "", path)
            self.niri.action("close-window", "--id", str(window["id"]))
            time.sleep(0.5)

    def restore_toggles(self, name: str, bus: str, item: dict, before: dict[str, tuple[bool, str]]) -> None:
        after = self.checked_state(bus)
        lost = [a for a, (checked, _l) in before.items() if checked and a in after and not after[a][0]]
        for action in lost:
            live.activate(self.env, bus, action)
            time.sleep(0.4)
        if not lost:
            mine = item["action"]
            if mine in before and mine in after and before[mine] != after[mine]:
                live.activate(self.env, bus, mine)
                time.sleep(0.4)
        final = self.checked_state(bus)
        drift = [a for a in before if a in final and final[a][0] != before[a][0]]
        if drift:
            self.finding("low", name, "toggle-not-restored", f"could not restore {drift} after {' > '.join(item['path'])}")

    # --------------------------------------------------------- keys ----

    def press_keys(self, name: str, window: dict, keys: list[str]) -> None:
        bus = live.MENU_BUS.get(name)
        self.focus(window)
        _, before = self.shot(f"{name}-before-keys")
        for chord in keys:
            known = {w["id"] for w in self.niri.windows()}
            self.key(chord, 0.35)
            self.step(name, "key", chord=chord)
            for extra in [w for w in self.niri.windows() if w["id"] not in known]:
                self.inspect_new_window(name, chord, extra)
                self.focus(window)
            if chord.startswith("type:"):
                path, after = self.shot(f"{name}-typed")
                box = live.window_box(window, SCALE)
                if box and vision.changed_fraction(before, after, box) < 0.0002:
                    self.finding("high", name, "no-redraw", f"typing {chord[5:]!r} did not redraw within 2 s", "", path)
                before = after
            if not self.alive(name, window):
                return
        self.check_hang(name, bus, "after the shortcuts")
        self.window_shot(name, "after-keys", window)

    def notes_flow(self, window: dict) -> None:
        """Create one note titled "Robot test", then move it to Recently
        Deleted. Deleting is allowed only for the note the robot just made,
        confirmed in the notes store first."""

        self.focus(window)
        self.key("cmd-n", 0.8)
        self.key("type:Robot test")
        time.sleep(1.5)
        path, _ = self.shot("Notes-robot-note")
        if not self.newest_note_is_ours():
            self.finding("medium", "Notes", "robot-note", "could not confirm the robot's note in the store; left it", "", path)
            return
        # TrashOrRestore (⌘⌫) acts on the selected note: the robot's own.
        self.focus(window)
        self.key("escape", 0.3)
        self.input.wl.key("cmd-backspace")  # type: ignore[union-attr]
        time.sleep(1.0)
        path, _ = self.shot("Notes-robot-note-deleted")
        self.step("Notes", "robot note moved to Recently Deleted")

    def newest_note_is_ours(self) -> bool:
        root = Path(os.environ.get("XDG_DATA_HOME", self.home / ".local/share")) / "rmac" / "notes"
        newest: Optional[Path] = None
        for path in root.rglob("*"):
            if path.is_file() and time.time() - path.stat().st_mtime < 20:
                if newest is None or path.stat().st_mtime > newest.stat().st_mtime:
                    newest = path
        if newest is None:
            return False
        try:
            return b"Robot test" in newest.read_bytes()[:65536]
        except OSError:
            return False

    # ------------------------------------------------------- resize ----

    def resize(self, name: str, window: dict) -> None:
        fresh = next((w for w in self.niri.windows() if w["id"] == window["id"]), None)
        if fresh is None:
            return
        size = (fresh.get("layout") or {}).get("window_size")
        wid = str(window["id"])
        for label, (width, height) in (("small", (420, 300)), ("large", (1400, 780))):
            self.check_session()
            self.niri.action("set-window-width", "--id", wid, str(width))
            self.niri.action("set-window-height", "--id", wid, str(height))
            time.sleep(1.0)
            self.window_shot(name, f"resize-{label}", window)
            if not self.alive(name, window):
                return
        if size:
            self.niri.action("set-window-width", "--id", wid, str(size[0]))
            self.niri.action("set-window-height", "--id", wid, str(size[1]))
            time.sleep(0.6)

    # --------------------------------------------------------- quit ----

    def quit_app(self, name: str, plan: dict, window: dict) -> None:
        pid = window.get("pid")
        if pid and Path(f"/proc/{pid}").exists():
            self.focus(window)
            chord = "cmd-w" if name == "Files" else "cmd-q"
            self.key(chord, 0.1)
            started = time.monotonic()
            end = started + 12
            while time.monotonic() < end and Path(f"/proc/{pid}").exists():
                if name == "Files" and not self.app_windows(plan["app_id"]):
                    break
                time.sleep(0.05)
            quit_s = time.monotonic() - started
            if name != "Files" and not Path(f"/proc/{pid}").exists():
                self.timings[f"{name} quit"] = round(quit_s, 3)
                if quit_s > 2.0:
                    self.finding("medium", name, "slow-quit", f"⌘Q took {quit_s:.1f} s to exit", "budget 2 s")
            if Path(f"/proc/{pid}").exists() and name != "Files":
                path, _ = self.shot(f"{name}-quit-stuck")
                gone = not self.app_windows(plan["app_id"])
                detail = ("its windows closed but the process kept running" if gone
                          else "windows are still open (a prompt?)")
                self.finding("high", name, "quit-hung", "⌘Q did not exit within 12 s", detail, path)
                if not gone:
                    self.key("escape", 0.4)
            if Path(f"/proc/{pid}").exists():
                os.kill(pid, signal.SIGTERM)
                time.sleep(1.0)
        for window_left in self.app_windows(plan["app_id"]):
            self.niri.action("close-window", "--id", str(window_left["id"]))
        self.log(f"{name}: done")

    # --------------------------------------------------------- shell ----

    def run_shell(self) -> None:
        import shell_tour

        shell_tour.tour(self)


def probe(env: dict[str, str]) -> int:
    import socket
    import struct

    niri = live.Niri(env)
    print("windows:", [(w["id"], w["app_id"]) for w in niri.windows()])
    print("layers:", [l["namespace"] for l in niri.layers()])
    print("D-Bus names:", live.bus_count(env))
    print("owner input count:", live.owner_input_count())
    print("health:", live.session_health())
    print("shell pids:", {n: live.live_session_pid(n, env) for n in live.SHELL_PROCESSES})
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(os.path.join(env["XDG_RUNTIME_DIR"], env["WAYLAND_DISPLAY"]))
    sock.sendall(struct.pack("<III", 1, (12 << 16) | 1, 2) + struct.pack("<III", 1, (12 << 16) | 0, 3))
    data = b""
    names = set()
    done = False
    sock.settimeout(3)
    while not done:
        chunk = sock.recv(65536)
        if not chunk:
            break
        data += chunk
        while len(data) >= 8:
            obj, word = struct.unpack_from("<II", data, 0)
            size = word >> 16
            if len(data) < size:
                break
            if obj == 2 and (word & 0xFFFF) == 0:
                length = struct.unpack_from("<I", data, 12)[0]
                names.add(data[16:16 + length - 1].decode())
            if obj == 3:
                done = True
            data = data[size:]
    sock.close()
    for interface in ("zwp_virtual_keyboard_manager_v1", "zwlr_virtual_pointer_manager_v1"):
        print(interface, "offered" if interface in names else "MISSING")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--live", action="store_true", help="drive the live session (needs the owner's consent)")
    parser.add_argument("--probe", action="store_true", help="read-only checks, no input")
    parser.add_argument("--out", default=f"~/lulo-robot-runs/{dt.datetime.now():%Y%m%d-%H%M%S}")
    parser.add_argument("--apps", default="", help="comma-separated app names, or 'all'")
    parser.add_argument("--shell", action="store_true", help="also tour the shell")
    parser.add_argument("--max-seconds", type=float, default=540)
    parser.add_argument("--dock-recents", default="",
                        help="comma-separated desktop IDs the Dock's recent apps should end as "
                             "(default: as found at the start)")
    parser.add_argument("--idle-seconds", type=float, default=120)
    parser.add_argument("--idle-give-up", type=float, default=900)
    args = parser.parse_args(argv)
    env = live.live_env()
    if args.probe:
        return probe(env)
    if not args.live:
        parser.error("pass --live (with the consent variable) to drive the session, or --probe")
    live.robot_guard(env)
    apps = list(APPS) if args.apps == "all" else [a for a in args.apps.split(",") if a]
    unknown = [a for a in apps if a not in APPS]
    if unknown:
        parser.error(f"unknown apps {unknown}; known: {sorted(APPS)}")
    robot = Robot(args)
    robot.log(f"robot run {robot.out}: apps {apps}, shell {args.shell}")
    live.wait_for_owner_idle(args.idle_seconds, args.idle_give_up, robot.log)
    robot.owner_count = live.owner_input_count()
    status = 0
    try:
        robot.baseline()
        robot.make_scratch()
        robot.input = live.Input(env)
        for name in apps:
            if robot.over_time():
                robot.log("time budget used up; stopping before " + name)
                break
            robot.run_app(name)
        if args.shell and not robot.over_time():
            robot.run_shell()
    except live.RobotAbort as abort:
        robot.log(f"ABORT: {abort}")
        robot.step(robot.current_app, "abort", reason=str(abort))
        status = 3
    except Exception:  # noqa: BLE001 - always restore the session
        robot.log("robot error:\n" + traceback.format_exc())
        status = 4
    finally:
        try:
            robot.finish()
        finally:
            if robot.input is not None:
                try:
                    robot.input.close()
                except Exception:  # noqa: BLE001
                    pass
        robot.log(f"{len(robot.findings)} findings; report {robot.out / 'report.json'}")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
