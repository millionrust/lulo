"""Plumbing between the robot and the owner's live Lulo session.

Everything here talks to the real session on the reference laptop:
niri IPC (``niri msg --json``), grim screenshots, the session D-Bus (only
``busctl --user list`` and the apps' own ``org.rmac.AppMenu2`` menus),
the user journal, ``session-health.json`` and synthetic input through
``scripts/behavior/wlinput.py``'s virtual keyboard and pointer.

That input is only allowed because the owner granted it for robot tests on
2026-10-13. ``robot_guard`` insists on that consent being stated on the
command line and in the environment before a single event is sent.
"""

from __future__ import annotations

import glob
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "behavior"))

import wlinput  # noqa: E402

CONSENT_ENV = "RMAC_ROBOT_LIVE_CONSENT"
CONSENT_VALUE = "owner-granted-2026-10-13"
UID = os.getuid()
RUNTIME = f"/run/user/{UID}"
SHELL_PROCESSES = ("niri", "rmac-top-bar", "rmac-dock")
LOGICAL = (1536, 864)


class RobotAbort(RuntimeError):
    """Stop the run now (the owner came back, the shell restarted, …)."""


def robot_guard(environ: dict[str, str]) -> None:
    if environ.get(CONSENT_ENV) != CONSENT_VALUE:
        raise wlinput.InjectorError(
            f"live input needs {CONSENT_ENV}={CONSENT_VALUE} (the owner's 2026-10-13 robot-test permission)")
    if not environ.get("WAYLAND_DISPLAY") or not environ.get("XDG_RUNTIME_DIR"):
        raise wlinput.InjectorError("WAYLAND_DISPLAY and XDG_RUNTIME_DIR must name the live session")


def live_env() -> dict[str, str]:
    env = dict(os.environ)
    env.setdefault("WAYLAND_DISPLAY", "wayland-1")
    env.setdefault("XDG_RUNTIME_DIR", RUNTIME)
    env.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={RUNTIME}/bus")
    if "NIRI_SOCKET" not in env:
        sockets = sorted(glob.glob(f"{RUNTIME}/niri.{env['WAYLAND_DISPLAY']}.*.sock"))
        if sockets:
            env["NIRI_SOCKET"] = sockets[0]
    return env


def run(cmd: list[str], env: dict[str, str], timeout: float = 10.0, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout, check=check)


# ---------------------------------------------------------------- niri ----

class Niri:
    def __init__(self, env: dict[str, str]) -> None:
        self.env = env

    def json(self, what: str) -> Any:
        result = run(["niri", "msg", "--json", what], self.env)
        if result.returncode != 0:
            raise RobotAbort(f"niri msg {what} failed: {result.stderr.strip()}")
        return json.loads(result.stdout)

    def windows(self) -> list[dict]:
        return self.json("windows")

    def layers(self) -> list[dict]:
        return self.json("layers")

    def focused(self) -> Optional[dict]:
        return self.json("focused-window")

    def action(self, *args: str) -> None:
        run(["niri", "msg", "action", *args], self.env)

    def spawn(self, command: list[str]) -> None:
        self.action("spawn", "--", *command)


def window_box(window: dict, scale: float) -> Optional[tuple[int, int, int, int]]:
    """The window's box in screenshot pixels, from niri's layout data."""

    layout = window.get("layout") or {}
    pos = layout.get("tile_pos_in_workspace_view")
    size = layout.get("window_size") or layout.get("tile_size")
    if not pos or not size:
        return None
    offset = layout.get("window_offset_in_tile") or [0, 0]
    x, y = pos[0] + offset[0], pos[1] + offset[1]
    return (int(x * scale), int(y * scale), int((x + size[0]) * scale), int((y + size[1]) * scale))


# --------------------------------------------------------------- input ----

class Input:
    """The virtual keyboard and pointer, with the minimum 150 ms pause the
    robot rules ask for after every action."""

    PAUSE = 0.18

    def __init__(self, env: dict[str, str]) -> None:
        self.wl = wlinput.Wayland(env, guard=robot_guard)

    def key(self, chord: str, pause: float = PAUSE) -> None:
        from safety import chord_allowed

        if not chord_allowed(chord):
            raise RobotAbort(f"refusing denied chord {chord}")
        self.wl.key(chord)
        time.sleep(pause)

    def type(self, text: str) -> None:
        self.wl.type_text(text, delay=0.05)
        time.sleep(self.PAUSE)

    def click(self, x: float, y: float, button: str = "left", count: int = 1) -> None:
        self.wl.click(x, y, *LOGICAL, button=button, count=count)
        time.sleep(self.PAUSE)

    def move(self, x: float, y: float) -> None:
        self.wl.move(x, y, *LOGICAL)
        time.sleep(self.PAUSE)

    def close(self) -> None:
        self.wl.close()


# ------------------------------------------------------------ watching ----

def screenshot(env: dict[str, str], path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    result = run(["grim", str(path)], env, timeout=15)
    if result.returncode != 0:
        raise RobotAbort(f"grim failed: {result.stderr.strip()}")
    return path


def bus_count(env: dict[str, str]) -> int:
    return sum(bus_owners(env).values())


def bus_owners(env: dict[str, str]) -> dict[str, int]:
    """Session-bus names per owning process name (unowned activatable names
    under "-"), so growth can be pinned on a process."""

    result = run(["busctl", "--user", "list", "--no-legend"], env)
    owners: dict[str, int] = {}
    for line in result.stdout.splitlines():
        fields = line.split()
        if len(fields) >= 3:
            owners[fields[2]] = owners.get(fields[2], 0) + 1
    return owners


def owner_growth(before: dict[str, int], after: dict[str, int]) -> dict[str, int]:
    return {name: after[name] - before.get(name, 0) for name in after if after[name] > before.get(name, 0)}


def pids_of(name: str) -> list[int]:
    """PIDs whose /proc comm matches `name` (comm is cut at 15 bytes)."""

    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            comm = Path(f"/proc/{entry}/comm").read_text().strip()
            exe = os.readlink(f"/proc/{entry}/exe")
        except OSError:
            continue
        if comm == name[:15] or os.path.basename(exe) == name:
            found.append(int(entry))
    return sorted(found)


def live_session_pid(name: str, env: dict[str, str]) -> Optional[int]:
    """The live session's own copy of a shell process, not another agent's
    nested one: it has the live WAYLAND_DISPLAY (or, for niri, owns the live
    NIRI_SOCKET)."""

    if name == "niri":
        match = re.search(r"\.(\d+)\.sock$", env.get("NIRI_SOCKET", ""))
        return int(match.group(1)) if match else None
    for pid in pids_of(name):
        try:
            environ = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
        except OSError:
            continue
        if f"WAYLAND_DISPLAY={env['WAYLAND_DISPLAY']}".encode() in environ and \
                f"XDG_RUNTIME_DIR={RUNTIME}".encode() in environ:
            return pid
    return None


def session_health() -> dict[str, int]:
    """Restart counts per unit from the session supervisor's health file."""

    try:
        data = json.loads(Path(f"{RUNTIME}/rmac/session-health.json").read_text())
    except (OSError, ValueError):
        return {}
    return {c.get("unit", "?"): int(c.get("restarts", 0)) for c in data.get("components", [])}


INPUT_IRQ = re.compile(r"i8042|atmel|mxt|i2c_designware|i2c-designware|synaptics|rmi4", re.I)


def owner_input_count() -> int:
    """Interrupts from the laptop's own keyboard, touchpad and touchscreen.

    The robot's virtual devices raise none, so a change means a human
    touched the machine.
    """

    total = 0
    for line in Path("/proc/interrupts").read_text().splitlines()[1:]:
        if not INPUT_IRQ.search(line):
            continue
        for field in line.split()[1:]:
            if field.isdigit():
                total += int(field)
            else:
                break
    return total


def wait_for_owner_idle(seconds: float, give_up: float, log) -> None:
    deadline = time.monotonic() + give_up
    last = owner_input_count()
    quiet_since = time.monotonic()
    while True:
        now = time.monotonic()
        if now - quiet_since >= seconds:
            return
        if now > deadline:
            raise RobotAbort(f"the owner kept using the laptop for {give_up:.0f}s; not starting")
        time.sleep(5)
        current = owner_input_count()
        if current != last:
            log(f"owner input seen; waiting for {seconds:.0f}s of quiet")
            last = current
            quiet_since = time.monotonic()


def journal_since(env: dict[str, str], since: str) -> list[str]:
    result = run(["journalctl", "--user", "--since", since, "--no-pager", "-o", "short-iso"], env, timeout=30)
    return result.stdout.splitlines()


ERROR_PATTERNS = (
    (re.compile(r"panicked at"), "panic"),
    (re.compile(r"\bSIGSEGV\b|segfault|core dumped|Aborted", re.I), "crash"),
    (re.compile(r"\b(ERROR|error)\b"), "error"),
    (re.compile(r"max_connections_per_user"), "dbus-limit"),
)


def classify_journal(lines: list[str]) -> list[tuple[str, str]]:
    """(kind, line) for each journal line from a Lulo process that looks
    like a crash, panic or logged error."""

    found = []
    for line in lines:
        if not re.search(r"rmac|lulo|niri", line):
            continue
        for pattern, kind in ERROR_PATTERNS:
            if pattern.search(line):
                found.append((kind, line))
                break
    return found


# ------------------------------------------------------------- menus ----

MENU_BUS = {
    "Files": "org.rmac.Files.Menu", "Terminal": "org.rmac.Terminal.Menu",
    "Notes": "org.rmac.Notes.Menu", "Text Editor": "org.rmac.TextEditor.Menu",
    "System Monitor": "org.rmac.SystemMonitor.Menu", "Settings": "org.rmac.SystemSettings.Menu",
    "Calculator": "org.rmac.Calculator.Menu", "Calendar": "org.rmac.Calendar.Menu",
    "Mail": "org.rmac.Mail.Menu", "Preview": "org.rmac.Preview.Menu",
    "Clock": "org.rmac.Clock.Menu", "Weather": "org.rmac.Weather.Menu",
}
MENU_PATH = "/org/rmac/AppMenu1"
MENU_IFACE = "org.rmac.AppMenu2"

ENABLED, SEPARATOR, CHECKED, MIXED, SUBMENU, ALTERNATE = 1, 2, 4, 8, 16, 32


def parse_layout(data: Any) -> tuple[int, list[dict]]:
    """Decode ``busctl --json=short`` output of AppMenu2.Layout into
    ``(revision, [{"label", "items": [{label, action, shortcut, flags,
    depth, path}]}])``. ``path`` is the item's labels from the menu down."""

    payload = data["data"] if isinstance(data, dict) else data
    revision, menus = payload[0], payload[1]
    decoded = []
    for label, items in menus:
        stack: list[str] = []
        rows = []
        for item_label, action, shortcut, bits, depth in items:
            del stack[depth:]
            path = [label, *stack, item_label]
            rows.append({"label": item_label, "action": action, "shortcut": shortcut,
                         "flags": bits, "depth": depth, "path": path,
                         "enabled": bool(bits & ENABLED), "checked": bool(bits & CHECKED),
                         "submenu": bool(bits & SUBMENU), "alternate": bool(bits & ALTERNATE)})
            if bits & SUBMENU:
                stack.append(item_label)
        decoded.append({"label": label, "items": rows})
    return revision, decoded


def read_layout(env: dict[str, str], bus: str, timeout: float = 5.0) -> tuple[Optional[tuple[int, list[dict]]], float]:
    """The app's live menus and how long the app took to answer (it
    validates them on its UI thread, so a slow answer means a busy UI)."""

    started = time.monotonic()
    result = run(["busctl", "--user", "--json=short", "call", bus, MENU_PATH, MENU_IFACE, "Layout"], env, timeout=timeout)
    elapsed = time.monotonic() - started
    if result.returncode != 0:
        return None, elapsed
    return parse_layout(json.loads(result.stdout)), elapsed


def activate(env: dict[str, str], bus: str, action: str) -> Optional[str]:
    result = run(["busctl", "--user", "call", bus, MENU_PATH, MENU_IFACE, "Activate", "s", action], env)
    return None if result.returncode == 0 else result.stderr.strip()


# --------------------------------------------------------- state files ----

STATE_GLOBS = (
    ".config/rmac/*.json",
    ".local/state/rmac/*.json",
    ".local/state/rmac/windows/*",
    ".local/state/rmac/system-settings/*",
    ".local/share/recently-used.xbel",
)
# The Dock keeps its recent-apps section in memory and only rewrites this
# file, so it is put back by relaunching apps (Robot.restore_dock_recents),
# never by writing the file.
DOCK_RECENTS = ".local/state/rmac/dock-recents"


def dock_recents(home: Path) -> list[str]:
    try:
        return [line.strip() for line in (home / DOCK_RECENTS).read_text().splitlines() if line.strip()]
    except OSError:
        return []


def recents_launch_order(original: list[str], current: list[str], fillers: list[str] = ()) -> list[str]:
    """Apps to launch, one at a time, to turn the Dock's recent-apps list
    back into `original` (rmac_dock::recents::advance: a launched app joins
    at the end unless already listed, and the oldest is dropped past
    three). First tries replaying `original`; if that cannot get there
    (an original app still listed in the wrong place is never moved), it
    pushes everything out with `fillers` (apps not in `original`) first.
    Empty if neither works (then the robot reports it instead)."""

    if original == current:
        return []
    spare = [app for app in fillers if app not in original and app not in current]
    for prefix in ([], spare[:3]):
        state = list(current)
        order: list[str] = []
        for app in [*prefix, *original]:
            state = simulate_recent_launch(state, app)
            order.append(app)
        if state == original:
            return order
    return []


def simulate_recent_launch(state: list[str], app: str, limit: int = 3) -> list[str]:
    if app in state:
        return list(state)
    state = [*state, app]
    while len(state) > limit:
        state.pop(0)
    return state


def snapshot_state(home: Path) -> dict[str, Optional[bytes]]:
    """Small app preference and window-state files, to put back afterwards."""

    snap: dict[str, Optional[bytes]] = {}
    for pattern in STATE_GLOBS:
        for path in glob.glob(str(home / pattern)):
            try:
                if os.path.getsize(path) <= 512 * 1024:
                    snap[path] = Path(path).read_bytes()
            except OSError:
                pass
    return snap


def restore_state(home: Path, snap: dict[str, Optional[bytes]]) -> list[str]:
    """Write back changed files and remove ones the run created. Returns
    the paths it touched."""

    touched = []
    now = snapshot_state(home)
    for path, data in snap.items():
        if now.get(path) != data and data is not None:
            tmp = path + ".robot-restore"
            Path(tmp).write_bytes(data)
            os.replace(tmp, path)
            touched.append(path)
    for path in now:
        if path not in snap and not path.endswith(".last-good.json"):
            os.remove(path)
            touched.append(path)
    return touched


def volume(env: dict[str, str]) -> str:
    return run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], env).stdout.strip()


def brightness() -> str:
    values = []
    for path in glob.glob("/sys/class/backlight/*/brightness"):
        try:
            values.append(Path(path).read_text().strip())
        except OSError:
            pass
    return ",".join(values)


def color_scheme(env: dict[str, str]) -> str:
    return run(["gsettings", "get", "org.gnome.desktop.interface", "color-scheme"], env).stdout.strip()


def free_bytes(path: str) -> int:
    return shutil.disk_usage(path).free
