#!/usr/bin/env python3
"""Automated Orca screen-reader audit (todo.md journey 9) in a private session.

    python3 scripts/a11y/orca_audit.py --bin-dir DIR [--shell-bin-dir DIR] \\
        [--output tests/accessibility/orca-run.json] [JOURNEY ...]

With no JOURNEY every app and shell journey runs. It drives each app and
shell surface with the keyboard only (Tab/Shift-Tab, arrows, Space, Return
into menus, Escape) and, for every key, records what keyboard focus landed
on over AT-SPI (role, name, states, value) and what a real Orca, running in
the same private session, decided to say. Then it flags unnamed controls,
generic roles, missing checked/expanded/selected states, focus going
nowhere, Tab traps, controls Tab never reaches, and changes Orca stayed
silent about (scripts/a11y/orca_checks.py has the rules).

Isolation is scripts/behavior/run_lulo.py's, plus:
  * Orca runs only inside the private dbus-run-session, against the run's
    own AT-SPI bus, with HOME/XDG_* in a temporary directory and GSettings on
    the memory backend - the owner's Orca settings and gsettings are never
    read or written, and screen-reader-enabled is never touched;
  * Orca has no speech server at all (scripts/a11y/orca_customizations.py):
    speech-dispatcher is never spawned, so nothing can reach a sound card;
    what Orca would say is written to a file instead;
  * the system bus is unreachable for everything in the run
    (DBUS_SYSTEM_BUS_ADDRESS points at a socket that does not exist), so a
    Space on a Wi-Fi or Bluetooth switch cannot reach NetworkManager/BlueZ;
  * no key that could activate something destructive is pressed: Return only
    opens menus in the shell, and push buttons are never activated.

Shell journeys (menu bar, Dock, Spotlight, Control Centre, Notification
Centre) run in nested niri with the shipped shell.kdl, nested inside the
same Sway - scripts/interaction/lulo_probe.py's ShellSession.
"""

from __future__ import annotations

import argparse
import datetime
import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import zlib
import struct
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO / "scripts" / "behavior"))
sys.path.insert(0, str(REPO / "scripts" / "interaction"))
sys.path.insert(0, str(REPO / "scripts"))

import orca_checks as checks  # noqa: E402
import run_lulo  # noqa: E402
import wlinput  # noqa: E402
from run_lulo import StepFailed, atspi, has_state, name, pump, role  # noqa: E402

DEFAULT_OUTPUT = REPO / "tests" / "accessibility" / "orca-run.json"
MAX_TAB = 45
QUIET = 0.6
MIN_SETTLE = 0.35
MAX_SETTLE = 3.0

STATE_NAMES = [
    "focused", "focusable", "checkable", "checked", "indeterminate", "expandable", "expanded",
    "selectable", "selected", "sensitive", "enabled", "editable", "multi_line", "pressed",
    "showing", "visible", "modal", "read_only", "has_popup", "active",
]
NO_DESCEND = {"list", "list box", "tree", "table", "tree table", "menu", "terminal", "document text",
              "document frame", "document web"}


def tiny_png(width: int = 64, height: int = 48) -> bytes:
    rows = b"".join(b"\0" + b"".join(bytes([(x * 4) % 256, (y * 5) % 256, 160]) for x in range(width))
                    for y in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


# --------------------------------------------------------------------------
# Journeys
# --------------------------------------------------------------------------
#
# Each app journey: launch, record what was said, run `extras` (scripted keys
# from the launch focus, each sequence closing what it opened), then a Tab
# cycle with a role probe at every stop, then three Shift-Tabs.
# Extra steps: (chord or "type:TEXT", expect, note).

APP_JOURNEYS: list[dict[str, Any]] = [
    {
        "id": "files", "title": "Files", "app": "files",
        "setup": {"files": {"Budget.txt": "numbers\n", "Letter.txt": "Dear\n", "Photos/": None}},
        "extras": [
            ("down", "move", "next item"), ("down", "move", "next item"), ("up", "move", "previous item"),
            ("cmd-shift-n", None, "New Folder: rename field"), ("escape", None, "leave rename"),
            ("cmd-f", None, "Search field"), ("escape", None, "leave search"),
        ],
    },
    {
        "id": "text-editor", "title": "Text Editor", "app": "text-editor",
        "extras": [
            ("type:Hello", None, "type into the document"),
            ("cmd-f", None, "Find field"), ("escape", None, "close Find"),
            ("cmd-w", None, "unsaved-changes alert"), ("tab", None, "alert: next button"),
            ("tab", None, "alert: next button"), ("tab", None, "alert: next button"),
            ("tab", None, "alert: wraps inside the alert"), ("escape", None, "cancel the alert"),
        ],
        "alt_tab": "ctrl-tab",
    },
    {
        "id": "settings", "title": "System Settings", "app": "settings",
        "extras": [
            ("down", "move", "sidebar: next pane"), ("down", "move", "sidebar: next pane"),
            ("up", "move", "sidebar: previous pane"),
            ("cmd-f", None, "Search field"), ("escape", None, "leave search"),
        ],
    },
    {
        "id": "terminal", "title": "Terminal", "app": "terminal",
        "extras": [
            ("type:echo audit", None, "type a command"), ("return", None, "run it"),
            ("cmd-f", None, "Find overlay"), ("escape", None, "close Find"),
        ],
        "alt_tab": "ctrl-tab",
    },
    {
        "id": "notes", "title": "Notes", "app": "notes",
        "extras": [
            ("cmd-n", None, "New Note"), ("type:Audit note", None, "type the title"),
            ("cmd-f", None, "Search field"), ("escape", None, "leave search"),
        ],
    },
    {
        "id": "preview", "title": "Preview", "app": "preview",
        "setup": {"png": "Sample.png"}, "launch": {"file": "Sample.png"},
        "extras": [("cmd-=", None, "zoom in"), ("cmd--", None, "zoom out")],
    },
    {
        "id": "calculator", "title": "Calculator", "app": "calculator",
        "extras": [("type:12+3", None, "type a sum"), ("return", None, "equals"), ("escape", None, "clear")],
        "watch": {"roles": ["text", "label", "static", "entry"], "max_width_fraction": 1.0},
    },
    {
        "id": "system-monitor", "title": "System Monitor", "app": "system-monitor",
        "idle_seconds": 15,
        "extras": [
            ("down", "move", "next process"), ("down", "move", "next process"), ("up", "move", "previous process"),
        ],
    },
]

SHELL_JOURNEYS: list[dict[str, Any]] = [
    {
        "id": "menu-bar", "title": "Top-bar menus (Control-F2)",
        "steps": [
            ("ctrl-f2", None, "focus the menu bar"), ("right", "move", "next title"), ("right", "move", "next title"),
            ("left", "move", "previous title"), ("down", None, "open the menu"), ("down", "move", "next item"),
            ("down", "move", "next item"), ("up", "move", "previous item"), ("right", None, "next menu / submenu"),
            ("escape", None, "back out"), ("escape", None, "back out"), ("escape", None, "leave the menu bar"),
        ],
    },
    {
        "id": "dock", "title": "Dock (Control-F3)",
        "steps": [
            ("ctrl-f3", None, "focus the Dock"), ("right", "move", "next app"), ("right", "move", "next app"),
            ("right", "move", "next app"), ("left", "move", "previous app"), ("escape", None, "leave the Dock"),
        ],
    },
    {
        "id": "spotlight", "title": "Spotlight", "open": "launcher", "resident": "rmac-launcher",
        "steps": [
            ("type:calc", None, "type a query"), ("down", "move", "next result"), ("down", "move", "next result"),
            ("up", "move", "previous result"), ("escape", None, "close Spotlight"),
        ],
    },
    {
        "id": "control-centre", "title": "Control Centre", "open": "quick-settings",
        "resident": "rmac-quick-settings", "background": "rmac-calculator", "cycle": True,
        "steps": [("escape", None, "close Control Centre")],
    },
    {
        "id": "notification-centre", "title": "Notification Centre", "open": "notification-center",
        "resident": "rmac-notification-center-panel", "background": "rmac-calculator",
        "steps": [("tab", None, "next control"), ("tab", None, "next control"), ("tab", None, "next control"),
                  ("escape", None, "close Notification Centre")],
    },
]


# --------------------------------------------------------------------------
# Outer process
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "swaymsg", "dbus-run-session", "orca"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-orca-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        env.update({
            # Nothing in the run can reach NetworkManager, BlueZ or logind.
            "DBUS_SYSTEM_BUS_ADDRESS": f"unix:path={work}/no-system-bus",
            # Belt and braces: the private runtime dir has no sound server.
            "PULSE_SERVER": f"unix:{work}/no-pulse",
            "PIPEWIRE_REMOTE": str(work / "no-pipewire"),
            "LULO_ORCA_SPEECH": str(work / "logs" / "orca-speech.jsonl"),
        })
        services = work / "dbus-services"
        services.mkdir()
        for service in ("org.a11y.Bus.service", "org.freedesktop.portal.Desktop.service"):
            source = Path("/usr/share/dbus-1/services") / service
            if source.exists():
                shutil.copy(source, services / service)
        chooser = run_lulo.find_file_chooser_binary([Path(p) for p in args.bin_dir + args.shell_bin_dir])
        if chooser is not None:
            (services / "org.freedesktop.impl.portal.desktop.rmac.filechooser.service").write_text(
                "[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.rmac.filechooser\n"
                f"Exec={chooser}\n"
            )
            portals = work / "portals"
            portals.mkdir()
            (portals / "rmac-file-chooser.portal").write_text(
                "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.rmac.filechooser\n"
                "Interfaces=org.freedesktop.impl.portal.FileChooser;\nUseIn=rmac\n"
            )
            env["XDG_DESKTOP_PORTAL_DIR"] = str(portals)
            conf = Path(env["XDG_CONFIG_HOME"]) / "xdg-desktop-portal"
            conf.mkdir(parents=True, exist_ok=True)
            (conf / "rmac-portals.conf").write_text(
                "[preferred]\ndefault=none\norg.freedesktop.impl.portal.FileChooser=rmac-file-chooser\n"
            )
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
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        with open(work / "logs" / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if status not in (0, 1):
            print((work / "logs" / "session.log").read_text()[-3000:], file=sys.stderr)
        return status
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if not args.keep:
            run_lulo.remove_tree(work)
        else:
            print(f"kept {work}", file=sys.stderr)
        journey_lock.close()


# --------------------------------------------------------------------------
# Orca and what it says
# --------------------------------------------------------------------------


class Orca:
    def __init__(self, env: dict[str, str], work: Path, debug: bool) -> None:
        self.speech_path = Path(env["LULO_ORCA_SPEECH"])
        prefs = Path(env["XDG_DATA_HOME"]) / "orca"
        prefs.mkdir(parents=True, exist_ok=True)
        shutil.copy(HERE / "orca_customizations.py", prefs / "orca-customizations.py")
        command = ["orca"]
        if debug:
            command += ["--debug-file", str(work / "logs" / "orca-debug.log")]
        self.log = open(work / "logs" / "orca.log", "w")
        self.process = subprocess.Popen(command, env=env, stdout=self.log, stderr=subprocess.STDOUT, close_fds=True)
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise StepFailed(f"Orca exited with {self.process.returncode}; see orca.log with --keep")
            if any(entry["kind"] == "speech" for entry in self.since(0)):
                break
            time.sleep(0.2)
        else:
            raise StepFailed("Orca started but never produced its start-up announcement")
        version = subprocess.run(["dpkg-query", "-W", "-f=${Version}", "orca"], capture_output=True, text=True)
        self.version = version.stdout.strip() or "unknown"

    def offset(self) -> int:
        try:
            return self.speech_path.stat().st_size
        except OSError:
            return 0

    def since(self, offset: int) -> list[dict[str, Any]]:
        try:
            with open(self.speech_path, "rb") as handle:
                handle.seek(offset)
                data = handle.read()
        except OSError:
            return []
        out = []
        for line in data.decode("utf-8", "replace").splitlines():
            try:
                out.append(json.loads(line))
            except ValueError:
                continue
        return out

    def alive(self) -> bool:
        return self.process.poll() is None

    def stop(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(8)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(5)
        self.log.close()


# --------------------------------------------------------------------------
# AT-SPI: focus tracking and snapshots
# --------------------------------------------------------------------------


EVENT_TYPES = (
    "object:announcement",
    "object:property-change:accessible-name",
    "object:state-changed:focused", "object:state-changed:checked", "object:state-changed:expanded",
    "object:state-changed:selected", "object:state-changed:pressed", "object:property-change:accessible-value",
    "object:value-changed", "object:selection-changed", "object:text-changed", "object:active-descendant-changed",
    "window:activate",
)


class Tracker:
    def __init__(self) -> None:
        pyatspi = atspi()
        self.focused = None
        self.last_event = time.monotonic()
        self.events: list[str] = []
        pyatspi.Registry.registerEventListener(self._on_event, *EVENT_TYPES)

    def _on_event(self, event) -> None:
        self.last_event = time.monotonic()
        try:
            kind = str(event.type)
        except Exception:
            kind = "?"
        self.events.append(kind)
        if kind.startswith("object:state-changed:focused") and event.detail1:
            self.focused = event.source

    def reset(self) -> None:
        self.events = []


STATES: list[tuple[str, Any]] = []


def state_constants() -> list[tuple[str, Any]]:
    if not STATES:
        pyatspi = atspi()
        for nick in STATE_NAMES:
            constant = getattr(pyatspi, "STATE_" + nick.upper(), None)
            if constant is not None:
                STATES.append((nick.replace("_", " "), constant))
    return STATES


def node_key(node) -> str:
    try:
        return f"{node.get_process_id()}:{node.path}"
    except Exception:
        pass
    parts = []
    current = node
    for _ in range(40):
        try:
            parent = current.parent
            index = current.getIndexInParent()
        except Exception:
            break
        parts.append(str(index))
        if parent is None:
            break
        current = parent
    return "/".join(reversed(parts))


def content_text(node, limit: int = 8) -> str:
    texts = []
    for child in run_lulo.descendants(node, limit=limit, depth=3):
        if child is node:
            continue
        label = name(child).strip()
        if label and label not in texts:
            texts.append(label)
    return " ".join(texts)[:120]


def snapshot(node) -> Optional[dict[str, Any]]:
    if node is None:
        return None
    try:
        state_set = node.getState()
        states = [nick for nick, constant in state_constants() if state_set.contains(constant)]
    except Exception:
        return None
    result: dict[str, Any] = {
        "key": node_key(node), "role": role(node), "name": name(node).strip(), "states": states,
        "description": "", "value": None, "text": None, "labelled_by": "", "content": "",
    }
    try:
        result["description"] = (node.description or "").strip()
    except Exception:
        pass
    try:
        result["app"] = node.getApplication().name
    except Exception:
        result["app"] = ""
    try:
        value = node.queryValue()
        result["value"] = float(value.currentValue)
        result["range"] = [float(value.minimumValue), float(value.maximumValue)]
    except Exception:
        pass
    if result["role"] in checks.TEXT_ROLES or result["role"] in {"label", "static"}:
        try:
            text = node.queryText()
            result["text"] = text.getText(0, min(text.characterCount, 160))
        except Exception:
            pass
    try:
        pyatspi = atspi()
        for relation in node.getRelationSet():
            if relation.getRelationType() == pyatspi.RELATION_LABELLED_BY:
                labels = [name(relation.getTarget(i)) for i in range(relation.getNTargets())]
                result["labelled_by"] = " ".join(label for label in labels if label).strip()
    except Exception:
        pass
    try:
        result["parent"] = node_key(node.parent) if node.parent is not None else ""
    except Exception:
        result["parent"] = ""
    if result["role"] in checks.CONTENT_NAMED_ROLES and not result["name"]:
        result["content"] = content_text(node)
    return result


def controls_in(frame, limit: int = 2500) -> list[dict[str, Any]]:
    """Showing controls in a window, with their ancestors' roles."""

    pyatspi = atspi()
    out = []
    stack = [(frame, [])]
    seen = 0
    while stack and seen < limit:
        node, ancestors = stack.pop()
        seen += 1
        node_role = role(node)
        if node is not frame and node_role in checks.REACHABLE_ROLES and has_state(node, pyatspi.STATE_SHOWING):
            snap = snapshot(node)
            if snap:
                snap["ancestors"] = ancestors
                out.append(snap)
        if node_role in NO_DESCEND and node is not frame:
            continue
        try:
            children = [node.getChildAtIndex(i) for i in range(node.childCount)]
        except Exception:
            continue
        for child in reversed(children):
            if child is not None:
                stack.append((child, ancestors + [node_role]))
    return out


# --------------------------------------------------------------------------
# The audit
# --------------------------------------------------------------------------


class Audit:
    def __init__(self, nested: run_lulo.Nested, orca: Orca, keyboard: wlinput.Wayland) -> None:
        self.nested = nested
        self.orca = orca
        self.keyboard = keyboard
        self.tracker = Tracker()
        self.frames_provider = None  # callable returning the frames to search for focus

    # -- input and observation ----------------------------------------------

    def drain(self, quiet: float = 2.0, limit: float = 20.0) -> None:
        """Let Orca work through events a closed app left queued, so one
        journey's backlog is not blamed on the next."""

        started = time.monotonic()
        time.sleep(1.0)
        last = self.orca.offset()
        last_change = time.monotonic()
        while time.monotonic() - started < limit:
            pump()
            size = self.orca.offset()
            if size != last:
                last, last_change = size, time.monotonic()
            if time.monotonic() - max(last_change, self.tracker.last_event) >= quiet:
                return
            time.sleep(0.1)

    def settle(self, offset: int) -> None:
        started = time.monotonic()
        last_size = offset
        last_change = started
        while True:
            pump()
            size = self.orca.offset()
            now = time.monotonic()
            if size != last_size:
                last_size = size
                last_change = now
            quiet_since = max(last_change, self.tracker.last_event)
            if now - started >= MIN_SETTLE and now - quiet_since >= QUIET:
                return
            if now - started >= MAX_SETTLE:
                return
            time.sleep(0.05)

    def current_focus(self):
        pyatspi = atspi()
        pump()
        node = self.tracker.focused
        if node is not None and has_state(node, pyatspi.STATE_FOCUSED):
            return node, "event"
        frames = self.frames_provider() if self.frames_provider else []
        best = None
        for frame in frames:
            if not (has_state(frame, pyatspi.STATE_ACTIVE) or len(frames) == 1):
                continue
            for candidate in run_lulo.descendants(frame, limit=1500):
                if candidate is not frame and has_state(candidate, pyatspi.STATE_FOCUSED):
                    best = candidate
        return best, ("walk" if best is not None else "none")

    def press(self, chord: str, expect: Optional[str], note: str, before: Optional[dict[str, Any]],
              watch=None) -> dict[str, Any]:
        offset = self.orca.offset()
        self.tracker.reset()
        watched_before = snapshot(watch) if watch is not None else None
        if chord.startswith("type:"):
            self.keyboard.type_text(chord[5:])
        else:
            self.keyboard.key(chord)
        self.settle(offset)
        node, via = self.current_focus()
        after = snapshot(node)
        speech = [entry["text"] for entry in self.orca.since(offset) if entry["kind"] in {"speech", "character"}]
        step: dict[str, Any] = {
            "key": chord, "note": note, "expect": expect, "focus": after, "via": via,
            "heard": checks.describe(after), "speech": speech, "events": sorted(set(self.tracker.events)),
            "name_events": sum(event.startswith("object:property-change:accessible-name")
                               for event in self.tracker.events),
        }
        flags = []
        if expect is not None or chord in {"tab", "shift-tab"} or (before or {}).get("key") != (after or {}).get("key"):
            flags += checks.step_flags(before, after, speech, expect)
        if (before or {}).get("key") != (after or {}).get("key"):
            flags += checks.focus_flags(after)
        if after is not None and via == "walk" and (before or {}).get("key") != after.get("key"):
            flags.append(checks.flag("silent-focus", f"focus moved to {checks.describe(after)} without an "
                                     "AT-SPI focus event, so Orca cannot follow it"))
        if watch is not None:
            watched_after = snapshot(watch)
            step["watched"] = {"before": checks.describe(watched_before), "after": checks.describe(watched_after)}
            changed = (watched_before or {}).get("text") != (watched_after or {}).get("text") or \
                (watched_before or {}).get("name") != (watched_after or {}).get("name")
            if changed and not speech:
                flags.append(checks.flag("silent-change", f"{checks.describe(watched_after)} changed but Orca "
                                         "said nothing"))
        step["flags"] = flags
        mark = "!" if flags else " "
        print(f"   {mark} {chord:<12} -> {checks.describe(after)[:70]:<70} | {' / '.join(speech)[:90]}", flush=True)
        for item in flags:
            print(f"       [{item['severity']}] {item['kind']}: {item['detail'][:150]}", flush=True)
        return step

    # -- generic parts --------------------------------------------------------

    def probe(self, stop: dict[str, Any], steps: list[dict[str, Any]], allow_toggle: bool) -> None:
        """Exercise one Tab stop by its role, then put it back."""

        role_name = stop.get("role")
        states = set(stop.get("states") or [])
        if role_name in {"check box", "toggle button", "switch"} and allow_toggle:
            step = self.press("space", "toggle", f"toggle {stop.get('name')!r}", stop)
            steps.append(step)
            if (step["focus"] or {}).get("key") == stop.get("key"):
                steps.append(self.press("space", "toggle", "toggle back", step["focus"]))
        elif role_name == "combo box" or ("expandable" in states and role_name in {"button", "push button", "push button menu"}):
            step = self.press("space", None, f"open {stop.get('name')!r}", stop)
            steps.append(step)
            opened = (step["focus"] or {}).get("key") != stop.get("key") or \
                "expanded" in ((step["focus"] or {}).get("states") or [])
            if not opened:
                step["flags"].append(checks.flag("no-effect", f"Space did not open {checks.describe(stop)}"))
            else:
                if "expandable" not in states:
                    step["flags"].append(checks.flag("missing-state", f"{checks.describe(stop)} opens a popup "
                                                     "but reports no expandable state"))
                steps.append(self.press("down", None, "move in the popup", step["focus"]))
                back = self.press("escape", None, "close the popup", steps[-1]["focus"])
                if (back["focus"] or {}).get("key") != stop.get("key"):
                    back["flags"].append(checks.flag("escape-focus", "Escape did not return focus to "
                                                     f"{checks.describe(stop)}"))
                steps.append(back)
        elif role_name in {"slider", "spin button"}:
            step = self.press("right" if role_name == "slider" else "up", "value", "raise the value", stop)
            steps.append(step)
            steps.append(self.press("left" if role_name == "slider" else "down", "value", "lower it back",
                                    step["focus"]))
        elif role_name in {"page tab"}:
            step = self.press("right", "move", "next tab", stop)
            steps.append(step)
            steps.append(self.press("left", "move", "previous tab", step["focus"]))
        elif role_name in {"list item", "tree item", "table row", "table cell", "list", "list box", "tree", "table"}:
            step = self.press("down", None, "next item", stop)
            steps.append(step)
            steps.append(self.press("up", None, "previous item", step["focus"]))

    def tab_cycle(self, start: Optional[dict[str, Any]], steps: list[dict[str, Any]], key: str = "tab",
                  allow_toggle: bool = True, probe: bool = True) -> tuple[list[dict[str, Any]], str]:
        stops: list[dict[str, Any]] = []
        keys: list[str] = []
        previous = start
        outcome = "limit"
        for _ in range(MAX_TAB):
            step = self.press(key, "move", "next control", previous)
            steps.append(step)
            focus = step["focus"]
            focus_key = (focus or {}).get("key")
            if focus is None:
                outcome = "lost"
                break
            if focus_key == (previous or {}).get("key"):
                outcome = "stuck"
                if not stops:
                    stops.append(focus)
                break
            if start is not None and focus_key == start.get("key") and stops:
                outcome = "cycle"
                break
            if focus_key in keys:
                # Back on a stop already seen without passing the start
                # again: fine if the start was the window itself (it is not
                # a Tab stop), a trap if it was a control Tab cannot reach.
                came_from_window = start is None or start.get("role") in {"frame", "window", "dialog"} | checks.GENERIC_ROLES
                outcome = "cycle" if focus_key == keys[0] and came_from_window else "subcycle"
                if len(keys) == 1:
                    outcome = "single"
                break
            stops.append(focus)
            keys.append(focus_key)
            if probe:
                self.probe(focus, steps, allow_toggle)
                after_probe = steps[-1]["focus"]
                if (after_probe or {}).get("key") != focus_key:
                    # The probe left focus somewhere else (a popup that did
                    # not close): stop rather than walk a different window.
                    outcome = "probe-left-focus"
                    break
            previous = steps[-1]["focus"]
        return stops, outcome

    # -- apps -----------------------------------------------------------------

    def launch(self, run, steps: list[dict[str, Any]], note: str) -> Optional[dict[str, Any]]:
        offset = self.orca.offset()
        run.launch()
        self.frames_provider = run.frames
        self.settle(offset)
        node, via = self.current_focus()
        first = snapshot(node)
        speech = [e["text"] for e in self.orca.since(offset) if e["kind"] == "speech"]
        step = {"key": "launch", "note": note, "focus": first, "via": via,
                "heard": checks.describe(first), "speech": speech, "flags": []}
        if not speech:
            step["flags"].append(checks.flag("silent-focus", "Orca said nothing when the window opened"))
        if first is None:
            step["flags"].append(checks.flag("focus-lost", "no initial keyboard focus in the new window"))
        else:
            step["flags"] += checks.focus_flags(first)
        steps.append(step)
        print(f"   launch -> {checks.describe(first)[:70]} | {' / '.join(speech)[:90]}", flush=True)
        return first

    def run_app(self, journey: dict[str, Any], bins: list[Path]) -> dict[str, Any]:
        scenario: dict[str, Any] = {"app": journey["app"], "setup": {}, "launch": journey.get("launch", {})}
        setup = dict(journey.get("setup", {}))
        png = setup.pop("png", None)
        scenario["setup"] = setup
        result: dict[str, Any] = {"id": journey["id"], "title": journey["title"], "kind": "app", "steps": []}
        steps = result["steps"]
        run = run_lulo.LuloRun(self.nested, f"orca/{journey['id']}", scenario, bins, 0.8)
        try:
            run.setup()
            if png:
                (run.sandbox / png).write_bytes(tiny_png())
            # 1. Scripted keys from the window's own initial focus.
            previous = self.launch(run, steps, "window opens (scripted keys)")
            if idle_seconds := journey.get("idle_seconds"):
                offset = self.orca.offset()
                self.tracker.reset()
                deadline = time.monotonic() + idle_seconds
                while time.monotonic() < deadline:
                    pump()
                    time.sleep(0.1)
                speech = [entry["text"] for entry in self.orca.since(offset)
                          if entry["kind"] in {"speech", "character"}]
                name_events = sum(event.startswith("object:property-change:accessible-name")
                                  for event in self.tracker.events)
                steps.append({"key": "idle", "note": f"{idle_seconds}s without input",
                              "focus": previous, "heard": checks.describe(previous),
                              "speech": speech, "name_events": name_events,
                              "events": sorted(set(self.tracker.events)), "flags": []})
                print(f"     idle {idle_seconds}s -> {name_events} accessible-name events",
                      flush=True)
            watch = self.find_watch(run, journey.get("watch"))
            for chord, expect, note in journey.get("extras", []):
                if run.process.poll() is not None:
                    raise StepFailed(f"{journey['title']} exited during the scripted keys "
                                     f"(status {run.process.returncode})")
                step = self.press(chord, expect, note, previous, watch)
                steps.append(step)
                previous = step["focus"]
            if run.process.poll() is not None:
                raise StepFailed(f"{journey['title']} exited during the scripted keys (status {run.process.returncode})")
            run.stop()
            run.restore_shared_state()
            time.sleep(0.5)
            # 2. A fresh window in a fresh home (so nothing from the scripted
            # keys, such as Text Editor's unsaved-changes recovery, carries
            # over): the full Tab cycle with a probe at every stop.
            run = run_lulo.LuloRun(self.nested, f"orca/{journey['id']}-tab", scenario, bins, 0.8)
            run.setup()
            if png:
                (run.sandbox / png).write_bytes(tiny_png())
            first = self.launch(run, steps, "window opens (Tab cycle)")
            stops, outcome = self.tab_cycle(first, steps)
            if outcome == "stuck" and journey.get("alt_tab") and stops and stops[-1].get("role") in checks.TEXT_ROLES:
                result["alt_tab"] = journey["alt_tab"]
                stops, outcome = self.tab_cycle(stops[-1], steps, key=journey["alt_tab"])
            result["cycle"] = {"outcome": outcome, "start": checks.describe(first),
                               "stops": [checks.describe(s) for s in stops]}
            cycle_flags = checks.cycle_flags(stops, outcome, first) + checks.item_stop_flags(stops)
            if outcome in {"cycle", "subcycle"} and len(stops) >= 2:
                reverse = []
                previous = steps[-1]["focus"]
                for _ in range(min(3, len(stops))):
                    step = self.press("shift-tab", "move", "previous control", previous)
                    steps.append(step)
                    reverse.append(step["focus"])
                    previous = step["focus"]
                ring = stops if outcome == "subcycle" or first is None or first.get("key") in {s.get("key") for s in stops} \
                    else [first] + stops
                cycle_flags += checks.reverse_flags(ring, reverse)
            if outcome in {"cycle", "subcycle", "stuck", "single"}:
                pyatspi = atspi()
                frames = [f for f in run.frames() if has_state(f, pyatspi.STATE_ACTIVE)] or run.frames()[:1]
                controls = controls_in(frames[0]) if frames else []
                visited = {s.get("key") for s in stops} | {(s.get("focus") or {}).get("key") for s in steps}
                cycle_flags += checks.reachability_flags(controls, visited)
            if run.process.poll() is not None:
                cycle_flags.append(checks.flag("focus-lost", f"{journey['title']} exited during the Tab cycle "
                                               f"(status {run.process.returncode})"))
            result["cycle_flags"] = cycle_flags
        except (StepFailed, wlinput.InjectorError) as error:
            result["error"] = str(error)
            print(f"   ERROR {error}", flush=True)
        finally:
            self.frames_provider = None
            run.stop()
            run.restore_shared_state()
        return finish(result)

    def find_watch(self, run, spec: Optional[dict[str, Any]]):
        """Calculator's display: the widest showing text/label in the window."""

        if not spec:
            return None
        pyatspi = atspi()
        best = None
        best_width = 0
        for frame in run.frames():
            for node in run_lulo.descendants(frame, limit=800):
                if role(node) in spec["roles"] and has_state(node, pyatspi.STATE_SHOWING):
                    box = run_lulo.extents(node)
                    if box and box[2] > best_width:
                        best, best_width = node, box[2]
        return best

    # -- shell ----------------------------------------------------------------

    def run_shell(self, journey: dict[str, Any], shell) -> dict[str, Any]:
        result: dict[str, Any] = {"id": journey["id"], "title": journey["title"], "kind": "shell", "steps": []}
        steps = result["steps"]
        try:
            if journey.get("open"):
                offset = self.orca.offset()
                shell.dispatch(journey["open"])
                resident = shell.residents.get(journey.get("resident", ""))
                if resident is not None:
                    shell.session.wait_for_populated_frame(resident.pid, timeout=20)
                    time.sleep(1.0)
                self.settle(offset)
                time.sleep(0.5)
                self.settle(self.orca.offset())
                node, via = self.current_focus()
                first = snapshot(node)
                speech = [e["text"] for e in self.orca.since(offset) if e["kind"] == "speech"]
                step = {"key": f"open {journey['open']}", "note": "open the surface", "focus": first, "via": via,
                        "heard": checks.describe(first), "speech": speech, "flags": checks.focus_flags(first)}
                if not speech:
                    step["flags"].append(checks.flag("silent-focus", "Orca said nothing when the surface opened"))
                steps.append(step)
                print(f"   open -> {checks.describe(first)[:70]} | {' / '.join(speech)[:90]}", flush=True)
            previous = steps[-1]["focus"] if steps else None
            if journey.get("cycle"):
                stops, outcome = self.tab_cycle(previous, steps, allow_toggle=False)
                result["cycle"] = {"outcome": outcome, "stops": [checks.describe(s) for s in stops]}
                result["cycle_flags"] = checks.cycle_flags(stops, outcome) + checks.item_stop_flags(stops)
                resident = shell.residents.get(journey.get("resident", ""))
                frames = shell.session.frames_by_pid(resident.pid) if resident is not None else []
                pyatspi = atspi()
                showing = [f for f in frames if has_state(f, pyatspi.STATE_SHOWING)] or frames[:1]
                if showing:
                    visited = {s.get("key") for s in stops} | {(s.get("focus") or {}).get("key") for s in steps}
                    result["cycle_flags"] += checks.reachability_flags(controls_in(showing[0]), visited)
                previous = steps[-1]["focus"] if steps else None
            for chord, expect, note in journey["steps"]:
                step = self.press(chord, expect, note, previous)
                if chord == "escape" and journey.get("background"):
                    focused_app = (step.get("focus") or {}).get("app")
                    if focused_app != journey["background"]:
                        step["flags"].append(checks.flag(
                            "focus-lost", f"Escape returned focus to {focused_app!r}, not "
                            f"{journey['background']!r}"))
                steps.append(step)
                previous = step["focus"]
        except (StepFailed, wlinput.InjectorError) as error:
            result["error"] = str(error)
            print(f"   ERROR {error}", flush=True)
        return finish(result)


def finish(result: dict[str, Any]) -> dict[str, Any]:
    flags = []
    for step in result["steps"]:
        for item in step.get("flags", []):
            flags.append({**item, "key": step["key"], "focus": step.get("heard")})
    flags += result.get("cycle_flags", [])
    if "error" in result:
        flags.append(checks.flag("focus-lost", f"journey stopped: {result['error']}"))
    result["flags"] = flags
    result["summary"] = checks.summarise(flags)
    return result


# --------------------------------------------------------------------------
# Inner process
# --------------------------------------------------------------------------


class Shell:
    """lulo_probe.ShellSession with every /usr/libexec/rmac binary the shipped
    shell.kdl spawns pointed at this run's build, and the residents a
    journey needs started on demand."""

    def __init__(self, nested: run_lulo.Nested, bins: list[Path], niri: Optional[str]) -> None:
        import lulo_probe

        self.lulo_probe = lulo_probe
        session = lulo_probe.ShellSession(nested, bins, Path(niri) if niri else None)
        original = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        patched = original
        import re

        for path in sorted(set(re.findall(r'"/usr/libexec/rmac/([a-z0-9-]+)"', original))):
            short = path[5:] if path.startswith("rmac-") else path
            for directory in bins:
                for candidate in (path, short):
                    if (directory / candidate).is_file():
                        patched = patched.replace(f'"/usr/libexec/rmac/{path}"', f'"{directory / candidate}"')
                        break
                else:
                    continue
                break
        self.kdl = nested.work / "shell-audit.kdl"
        self.kdl.write_text(patched)
        self.session = session
        self.residents: dict[str, subprocess.Popen] = {}

    def start(self) -> None:
        # ShellSession.start reads the shipped shell.kdl; hand it ours.
        real = self.lulo_probe.REPO
        staged = self.session.nested.work / "repo-view"
        (staged / "packaging" / "rmac-session").mkdir(parents=True, exist_ok=True)
        shutil.copy(self.kdl, staged / "packaging" / "rmac-session" / "shell.kdl")
        self.lulo_probe.REPO = staged
        try:
            self.session.start()
        finally:
            self.lulo_probe.REPO = real
        # The Dock registers its accessible tree a moment after it maps.
        for process in self.session.children[1:]:
            self.session.wait_for_populated_frame(process.pid, timeout=20)

    def resident(self, binary: str) -> None:
        if binary in self.residents:
            return
        path = self.lulo_probe.find_bin(self.session.bins, binary)
        self.residents[binary] = self.session._spawn([str(path)], binary)
        time.sleep(2.0)

    def dispatch(self, shortcut: str) -> None:
        self.session.dispatch(shortcut)

    def close(self) -> None:
        self.session.close()


def inner(args: argparse.Namespace) -> int:
    work = Path(args.inner)
    nested = run_lulo.Nested(work)
    bins = [Path(p) for p in args.bin_dir] + [Path(p) for p in args.shell_bin_dir]
    wanted = set(args.journeys)
    results: list[dict[str, Any]] = []
    orca = None
    shell = None
    try:
        orca = Orca(nested.env, work, args.orca_debug)
        print(f"Orca {orca.version} is running in the private session (speech to a file, no audio)", flush=True)
        audit = Audit(nested, orca, nested.input)
        for journey in APP_JOURNEYS:
            if wanted and journey["id"] not in wanted:
                continue
            print(f"== {journey['title']}", flush=True)
            results.append(audit.run_app(journey, bins))
            audit.drain()
        shell_journeys = [j for j in SHELL_JOURNEYS if not wanted or j["id"] in wanted]
        if shell_journeys and shutil.which(args.niri or "niri"):
            shell = Shell(nested, bins, args.niri)
            shell.start()
            audit.keyboard = shell.session.input
            audit.frames_provider = None
            for journey in shell_journeys:
                print(f"== {journey['title']}", flush=True)
                if journey.get("background"):
                    shell.resident(journey["background"])
                if journey.get("resident"):
                    try:
                        shell.resident(journey["resident"])
                    except StepFailed as error:
                        results.append(finish({"id": journey["id"], "title": journey["title"], "kind": "shell",
                                               "steps": [], "error": str(error)}))
                        continue
                results.append(audit.run_shell(journey, shell))
                time.sleep(0.5)
        if not orca.alive():
            print("WARNING: Orca exited during the run", flush=True)
    finally:
        if shell is not None:
            shell.close()
        if orca is not None:
            orca.stop()
        nested.close()
    flags = [flag for result in results for flag in result["flags"]]
    report = {
        "format": 1,
        "recorded": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git": args.git or "",
        "orca": orca.version if orca else "unavailable",
        "harness": "scripts/a11y/orca_audit.py",
        "summary": checks.summarise(flags),
        "journeys": results,
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n")
    print(f"\n{len(results)} journeys, {len(flags)} flags: {report['summary']}\nwrote {output}", flush=True)
    return 0 if not any("error" in r for r in results) else 1


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("journeys", nargs="*", help="journey ids (default: all)")
    parser.add_argument("--bin-dir", action="append", default=[])
    parser.add_argument("--shell-bin-dir", action="append", default=[])
    parser.add_argument("--niri", default=None)
    parser.add_argument("--output", default=str(DEFAULT_OUTPUT))
    parser.add_argument("--orca-debug", action="store_true", help="also keep Orca's own debug log")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--git", default=None, help=argparse.SUPPRESS)
    parser.add_argument("--inner", default=None, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    known = {j["id"] for j in APP_JOURNEYS + SHELL_JOURNEYS}
    unknown = [j for j in args.journeys if j not in known]
    if unknown:
        parser.error(f"unknown journeys {unknown}; known: {sorted(known)}")
    git = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
    rebuilt = ["--bin-dir=" + str(Path(p).resolve()) for p in args.bin_dir]
    rebuilt += ["--shell-bin-dir=" + str(Path(p).resolve()) for p in args.shell_bin_dir]
    rebuilt += ["--output", str(Path(args.output).resolve()), "--git", git.stdout.strip()]
    if args.niri:
        rebuilt += ["--niri", args.niri]
    if args.orca_debug:
        rebuilt.append("--orca-debug")
    rebuilt += args.journeys
    args.bin_dir = [str(Path(p).resolve()) for p in args.bin_dir]
    args.shell_bin_dir = [str(Path(p).resolve()) for p in args.shell_bin_dir]
    return outer(args, rebuilt)


if __name__ == "__main__":
    sys.exit(main())
