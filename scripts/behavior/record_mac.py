#!/usr/bin/env python3
"""Record what macOS does for each behaviour scenario (owner's Mac only).

    python3 scripts/behavior/record_mac.py files/new-folder
    python3 scripts/behavior/record_mac.py --all
    python3 scripts/behavior/record_mac.py --all --missing   # only unrecorded

Each scenario runs in a fresh /tmp/lulo-behavior/<area>-<name>/sandbox
folder, driven through System Events (osascript). The facts it sees are
written to tests/behavior/<area>/<name>.mac.json: words and numbers only,
never captures, never absolute paths.

Safety (docs/behavior-suite.md):
  * holds /tmp/lulo-mac-gui.lock for the whole run, so no other agent drives
    the screen at the same time;
  * before every key press it checks the frontmost app is the scenario's
    app and its focused window is one the scenario opened (never one of
    the owner's windows); otherwise it stops;
  * never quits an app that was already running, and closes only the
    windows and documents it opened, without saving;
  * only ever creates files in its sandbox (the Desktop scenario creates
    one folder on ~/Desktop and removes it with rmdir, which only removes
    an empty folder).
"""

from __future__ import annotations

import argparse
import datetime
import fcntl
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

import scenario as sc  # noqa: E402

HERE = Path(__file__).resolve().parent
OBSERVER = HERE / "mac_observe.js"
LOCK = "/tmp/lulo-mac-gui.lock"
SANDBOX_ROOT = Path("/tmp/lulo-behavior")
DESKTOP = Path.home() / "Desktop"

APPS = {
    "files": {"process": "Finder", "bundle": "com.apple.finder"},
    "desktop": {"process": "Finder", "bundle": "com.apple.finder"},
    "text-editor": {"process": "TextEdit", "bundle": "com.apple.TextEdit"},
    "settings": {"process": "System Settings", "bundle": "com.apple.systempreferences"},
    "calculator": {"process": "Calculator", "bundle": "com.apple.calculator"},
    "preview": {"process": "Preview", "bundle": "com.apple.Preview"},
}
BIDI_MARKS = dict.fromkeys(map(ord, "‎‏‪‫‬‭‮⁦⁧⁨⁩"))


class Stop(RuntimeError):
    """A safety check failed; the scenario stops and cleans up."""


def background_context_point(window: tuple[int, int, int, int], viewport: tuple[int, int, int, int]) -> tuple[int, int]:
    """Return a safe empty point from the viewport's intersection with our window."""
    wx, wy, ww, wh = window
    vx, vy, vw, vh = viewport
    left, top = max(wx, vx), max(wy, vy)
    right, bottom = min(wx + ww, vx + vw), min(wy + wh, vy + vh)
    if right - left <= 48 or bottom - top <= 48:
        raise Stop(f"Finder viewport {viewport} is not a usable region inside scenario window {window}")
    return right - 24, bottom - 24


def osascript(*lines: str, js: bool = False, args: tuple[str, ...] = ()) -> str:
    command = ["osascript"]
    if js:
        command += ["-l", "JavaScript"]
    for line in lines:
        command += ["-e", line]
    command += list(args)
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
    except subprocess.TimeoutExpired as error:
        raise Stop("osascript timed out (is a menu or modal panel open?)") from error
    if result.returncode != 0:
        raise Stop(f"osascript failed: {result.stderr.strip()[:200]}")
    return result.stdout.strip()


def observe_raw(process: str, facts: list[str], baseline: list[str]) -> dict[str, Any]:
    try:
        result = subprocess.run(
            ["osascript", "-l", "JavaScript", str(OBSERVER), process, ",".join(facts), json.dumps(baseline)],
            capture_output=True, text=True, timeout=60,
        )
    except subprocess.TimeoutExpired as error:
        raise Stop("the AX observer timed out") from error
    if result.returncode != 0:
        raise Stop(f"observer failed: {result.stderr.strip()[:200]}")
    return json.loads(result.stdout)


def as_string(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def running(process: str) -> bool:
    out = osascript(f'tell application "System Events" to exists process {as_string(process)}')
    return out == "true"


def listing(root: Path, before: set[str]) -> list[str]:
    entries = []
    if not root.exists():
        return entries
    for path in sorted(root.rglob("*")):
        rel = path.relative_to(root)
        if any(part.startswith(".") for part in rel.parts):
            continue
        name = rel.as_posix() + ("/" if path.is_dir() else "")
        top = rel.parts[0] + ("/" if (root / rel.parts[0]).is_dir() else "")
        if top in before:
            continue
        entries.append(name)
    return entries


def normalize(raw: dict[str, Any], facts: list[str], files_root: Path, before: set[str]) -> dict[str, Any]:
    """Turn raw AX readings into the portable facts scenario.py compares."""

    out: dict[str, Any] = {}
    for fact in facts:
        if fact == "files":
            out["files"] = {"entries": listing(files_root, before)}
        elif fact == "focus":
            focus = raw.get("focus")
            if not focus:
                out["focus"] = {"role": None, "value": None, **sc.selection_facts(None, None, None)}
                continue
            value = focus.get("value")
            value = value.translate(BIDI_MARKS) if isinstance(value, str) else None
            start = end = None
            rng = focus.get("range")
            if isinstance(rng, list) and len(rng) == 2 and value is not None:
                # JXA reports an NSRange as AppleScript's 1-based inclusive
                # [first, last]; a caret is [n + 1, n].
                start, end = rng[0] - 1, rng[1]
            out["focus"] = {
                "role": sc.normalize_ax_role(focus.get("ax_role"), focus.get("ax_subrole")),
                "value": value,
                **sc.selection_facts(value, start, end),
                "label": focus.get("label"),
            }
        elif fact == "windows":
            windows = raw.get("windows") or {}
            out["windows"] = {k: windows.get(k) for k in ("count", "front", "titles")}
        elif fact == "display":
            value = (raw.get("display") or {}).get("value")
            out["display"] = {"value": value.translate(BIDI_MARKS) if value else value}
        elif fact == "dialog":
            dialog = dict(raw.get("dialog") or {"present": False})
            dialog.pop("kind", None)
            if dialog.get("default") is None:
                # AX rarely names a sheet's default button; unknown is not
                # "none", so leave it out rather than expect none on Lulo.
                dialog.pop("default", None)
            out["dialog"] = dialog
        elif fact == "saved_documents":
            # Mirrors run_lulo.py's fact_saved_documents (the nested run's
            # isolated $HOME/Documents): read plain files under this run's
            # own sandbox, never the owner's real Documents folder. Any
            # scenario that actually commits a save must have already
            # steered TextEdit's Save sheet into files_root first.
            names = sorted(p.name for p in files_root.iterdir() if p.is_file()) if files_root.exists() else []
            contents = {}
            for name in names:
                try:
                    contents[name] = (files_root / name).read_bytes()[:4096].decode("utf-8", "replace")
                except OSError:
                    pass
            out["saved_documents"] = {"entries": names, "contents": contents}
        elif fact in raw:
            out[fact] = raw[fact]
        else:
            out[fact] = None
    return out


class MacRun:
    def __init__(self, sid: str, scenario: dict[str, Any], settle: float) -> None:
        self.sid = sid
        self.scenario = scenario
        self.settle = settle
        self.app = scenario["app"]
        self.process = APPS[self.app]["process"]
        self.work = SANDBOX_ROOT / sid.replace("/", "-")
        self.sandbox = self.work / "sandbox"
        self.files_root = DESKTOP if self.app == "desktop" else self.sandbox
        self.before: set[str] = set()
        self.baseline: list[str] = []
        self.was_running = False
        self.finder_ids: list[int] = []
        self.documents: list[str] = []
        self.menu_open = False
        self.started = time.time()

    # -- setup -------------------------------------------------------------

    def setup(self) -> None:
        if self.work.exists():
            shutil.rmtree(self.work)
        self.sandbox.mkdir(parents=True)
        for name, content in self.scenario.get("setup", {}).get("files", {}).items():
            target = self.sandbox / name
            if name.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content or "")
        if self.app == "desktop":
            self.before = {p.name + ("/" if p.is_dir() else "") for p in DESKTOP.iterdir()}
        self.was_running = running(self.process)
        if self.app == "calculator" and self.was_running:
            raise Stop("Calculator is already running; quit it to record this scenario")
        if self.process == "Finder":
            ids = osascript('tell application "Finder" to get id of every window')
            self.finder_ids = [int(x) for x in ids.replace(",", " ").split()] if ids else []
        if self.process == "TextEdit" and self.was_running:
            names = osascript('tell application "TextEdit" to get name of every document')
            self.documents = [n.strip() for n in names.split(",")] if names else []
        # System Settings has one window and no documents: when it is already
        # open, the scenario uses that window (a search and a pane change),
        # and leaves the app running afterwards.
        if self.was_running and self.app != "settings":
            self.baseline = observe_raw(self.process, ["baseline"], []).get("baseline", [])
        if any(key.split("|")[0] in {"sandbox", SANDBOX_ROOT.name} for key in self.baseline):
            raise Stop("a window from an earlier run is still open; close it first")
        if self.app == "desktop" and self.finder_ids:
            raise Stop("Finder has windows open; the Desktop scenario needs the desktop focused")

    def launch(self) -> None:
        launch = self.scenario.get("launch", {})
        if self.app == "files":
            if "reveal" in launch:
                subprocess.run(["open", "-R", str(self.sandbox / launch["reveal"])], check=True)
            else:
                subprocess.run(["open", str(self.sandbox / launch.get("folder", "."))], check=True)
        elif self.app == "desktop":
            osascript('tell application "Finder" to activate')
        elif self.app == "text-editor":
            osascript('tell application "TextEdit" to make new document', 'tell application "TextEdit" to activate')
        elif self.app == "preview":
            document = self.sandbox / launch["file"]
            subprocess.run(["open", "-b", APPS[self.app]["bundle"], str(document)], check=True)
            osascript(f'tell application id {as_string(APPS[self.app]["bundle"])} to activate')
        else:
            subprocess.run(["open", "-b", APPS[self.app]["bundle"]], check=True)
            osascript(f'tell application id {as_string(APPS[self.app]["bundle"])} to activate')
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            raw = observe_raw(self.process, [], self.baseline)
            guard = raw.get("guard") or {}
            if raw.get("frontmost") == self.process and (self.app == "desktop" or guard.get("focused_window_is_ours")):
                time.sleep(self.settle)
                return
            time.sleep(0.3)
        raise Stop("the app did not come to the front with the scenario's window")

    # -- steps -------------------------------------------------------------

    def check_target(self) -> None:
        raw = observe_raw(self.process, ["menu"] if self.menu_open else [], self.baseline)
        guard = raw.get("guard") or {}
        if raw.get("frontmost") != self.process:
            raise Stop(f"frontmost app is {raw.get('frontmost')!r}, not {self.process}; stopped before typing")
        if self.menu_open and (raw.get("menu") or {}).get("present"):
            return  # the context menu this scenario opened has the keyboard
        self.menu_open = False
        if self.app == "desktop":
            if guard.get("windows"):
                raise Stop("a Finder window has focus; stopped before typing on the desktop")
        elif guard.get("focused_window") is None and guard.get("ours"):
            # Finder reports no focused window while renaming (AXFocusedWindow
            # goes briefly null for the window whose inline field we just
            # typed into). frontmost is already confirmed to be Finder above,
            # and "ours" being non-zero means the window we opened is still
            # open, so this is safe even when the owner has other Finder
            # windows open too (requiring every window to be ours made every
            # Return-based rename scenario fail whenever a second Finder
            # window existed, e.g. tests/behavior/files/rename-file.json
            # against a Finder with a Downloads and an Applications window
            # open, 2026-09-29).
            return
        elif not guard.get("focused_window_is_ours"):
            raise Stop(f"the focused window is not one this scenario opened ({guard}); stopped before typing")

    def key(self, chord: str) -> None:
        char, code, mods = sc.mac_keystroke(chord)
        using = f" using {{{', '.join(mods)}}}" if mods else ""
        self.check_target()
        if code is not None:
            osascript(f'tell application "System Events" to key code {code}{using}')
        else:
            osascript(f'tell application "System Events" to keystroke {as_string(char)}{using}')

    def type_text(self, text: str) -> None:
        self.check_target()
        osascript(f'tell application "System Events" to keystroke {as_string(text)}')

    def _locate_item(self, name: str) -> tuple[float, float]:
        """The on-screen centre of a row, icon, button or pop-up named
        `name` in the focused window (which, for a sheet, includes the
        sheet's own children), found the same way `context()` finds the
        selected Finder item, but by title instead of selection state. Used
        for a real click (shift-click, command-click, double-click), which a
        Finder "select" Apple Event cannot produce, and, for any process,
        for a plain click on a dialog/sheet control (a Save panel's "Where"
        pop-up, "Show Details", a System Settings sidebar row) that has no
        such Apple Event at all."""

        script = f"""
function run() {{
  var se = Application("System Events");
  var p = se.processes.byName({json.dumps(self.process)});
  var w = p.attributes.byName("AXFocusedWindow").value();
  function A(el, n) {{ try {{ return el.attributes.byName(n).value(); }} catch (e) {{ return undefined; }} }}
  function nameOf(el) {{
    var t = A(el, "AXTitle");
    if (t) return t;
    // List view nests the name a couple of levels down (AXRow > AXCell >
    // AXStaticText/AXTextField); icon view puts it closer to the top.
    var found = null;
    function dig(node, depth) {{
      if (found || depth < 0) return;
      var role = A(node, "AXRole");
      if (role === "AXStaticText" || role === "AXTextField") {{
        var v = A(node, "AXValue") || A(node, "AXTitle");
        if (v) {{ found = v; return; }}
      }}
      var kids = A(node, "AXChildren") || [];
      for (var j = 0; j < kids.length; j++) dig(kids[j], depth - 1);
    }}
    dig(el, 4);
    if (found) return found;
    // A pop-up button (e.g. a Save sheet's "Where" control) has a fixed
    // AXTitle label ("Where:") and no children at all; the folder it
    // currently shows lives in AXValue instead. A disclosure triangle
    // (the Save sheet's expand/collapse control) has neither a title nor
    // children; its accessible name is AXDescription. Both are a last
    // resort, after an exact title match.
    var av = A(el, "AXValue");
    if (typeof av === "string") return av;
    return A(el, "AXDescription") || null;
  }}
  var hit = null;
  function walk(el, d) {{
    if (hit || d < 0) return;
    var role = A(el, "AXRole");
    if ((role === "AXRow" || role === "AXImage" || role === "AXGroup" || role === "AXButton" || role === "AXPopUpButton" || role === "AXMenuButton" || role === "AXRadioButton" || role === "AXCell" || role === "AXDisclosureTriangle") && nameOf(el) === {json.dumps(name)}) {{ hit = el; return; }}
    var k = A(el, "AXChildren") || [];
    for (var i = 0; i < k.length; i++) walk(k[i], d - 1);
  }}
  walk(w, 14);
  if (!hit) return "none";
  var pos = A(hit, "AXPosition"), size = A(hit, "AXSize");
  if (!pos || !size) return "none";
  return [pos[0] + Math.min(40, size[0] / 2), pos[1] + size[1] / 2].join(" ");
}}"""
        where = osascript(script, js=True)
        if where == "none":
            raise Stop(f"no on-screen item named {name!r} to click in {self.process}")
        x, y = (float(value) for value in where.split())
        return x, y

    def select(self, name: str, modifiers: Optional[list[str]] = None, double: bool = False) -> None:
        if self.process != "Finder" and not self.menu_open and not (modifiers or double):
            # Not Finder, and no open context menu: this is a plain click on
            # a dialog/sheet control (Save panel "Where" pop-up button, menu
            # item once that pop-up is open, "Show Details", a System
            # Settings sidebar row) rather than a Finder item. There is no
            # Apple Event for this outside Finder, so click the control's
            # measured centre directly, the same way a real Finder click
            # does below, then see whether the click opened a menu so the
            # *next* select() knows to look for a menu item instead of
            # another button.
            self.check_target()
            x, y = self._locate_item(name)
            self.check_target()
            subprocess.run([sys.executable, str(HERE / "mac_click.py"), "left", str(x), str(y)], check=True, timeout=10)
            time.sleep(0.3)
            raw = observe_raw(self.process, ["menu"], self.baseline)
            self.menu_open = bool((raw.get("menu") or {}).get("present"))
            return
        if self.process != "Finder" and (modifiers or double):
            raise Stop("select with modifiers or double-click is only defined for Finder scenarios")
        if not self.menu_open and (modifiers or double):
            # A real click, not a Finder "select" Apple Event: shift-click
            # extends the selection, command-click toggles an item into it,
            # and a double-click opens the item — none of which "select"
            # (which only sets AX selection state) can produce.
            self.check_target()
            x, y = self._locate_item(name)
            self.check_target()
            args = ["left", str(x), str(y)]
            if double:
                args += ["--count", "2"]
            if modifiers and "shift" in modifiers:
                args.append("--shift")
            if modifiers and "cmd" in modifiers:
                args.append("--cmd")
            subprocess.run([sys.executable, str(HERE / "mac_click.py"), *args], check=True, timeout=10)
            return
        if self.menu_open:
            self.check_target()
            script = f"""
function run() {{
  var se = Application("System Events");
  var p = se.processes.byName({json.dumps(self.process)});
  var hit = null;
  function A(el, n) {{ try {{ return el.attributes.byName(n).value(); }} catch (e) {{ return undefined; }} }}
  function walk(el, d) {{
    if (hit || d < 0) return;
    if (A(el, "AXRole") === "AXMenuItem" && A(el, "AXTitle") === {json.dumps(name)}) {{ hit = el; return; }}
    var k = A(el, "AXChildren") || [];
    for (var i = 0; i < k.length; i++) walk(k[i], d - 1);
  }}
  walk(p, 12);
  if (!hit) return "none";
  var pos = A(hit, "AXPosition"), size = A(hit, "AXSize");
  if (!pos || !size || size[0] <= 0 || size[1] <= 0) return "none";
  return [pos[0], pos[1], size[0], size[1]].join(" ");
}}"""
            where = osascript(script, js=True)
            if where == "none":
                raise Stop(f"no open context-menu item named {name!r}")
            try:
                x, y, width, height = (float(value) for value in where.split())
            except (TypeError, ValueError) as error:
                raise Stop(f"Finder returned invalid bounds for context-menu item {name!r}") from error
            self.check_target()
            subprocess.run([sys.executable, str(HERE / "mac_click.py"), "left", str(x + width / 2), str(y + height / 2)], check=True, timeout=10)
            self.menu_open = False
            return
        self.check_target()
        target = self.files_root / name
        osascript(f'tell application "Finder" to select (POSIX file {as_string(str(target))} as alias)')

    def context(self, name: str) -> None:
        if name == "background":
            self.check_target()
            script = f"""
function run() {{
  var se = Application("System Events");
  var p = se.processes.byName({json.dumps(self.process)});
  var w = p.attributes.byName("AXFocusedWindow").value();
  function A(el, n) {{ try {{ return el.attributes.byName(n).value(); }} catch (e) {{ return undefined; }} }}
  var hits = [];
  function walk(el, d) {{
    if (d < 0) return;
    var role = A(el, "AXRole");
    if (role === "AXOutline" || role === "AXTable" || role === "AXList") {{
      var vp = A(el, "AXPosition"), vs = A(el, "AXSize");
      if (vp && vs) hits.push([vp[0], vp[1], vs[0], vs[1]]);
    }}
    var k = A(el, "AXChildren") || [];
    for (var i = 0; i < k.length; i++) walk(k[i], d - 1);
  }}
  walk(w, 12);
  var wp = A(w, "AXPosition"), ws = A(w, "AXSize");
  if (!wp || !ws) return "none";
  return JSON.stringify([[wp[0], wp[1], ws[0], ws[1]], hits]);
}}"""
            where = osascript(script, js=True)
            if where == "none":
                raise Stop("Finder did not expose an AX list viewport for background context-click")
            try:
                window_values, viewport_values = json.loads(where)
                window = tuple(float(value) for value in window_values)
                viewports = [tuple(float(value) for value in values) for values in viewport_values]
            except (TypeError, ValueError, json.JSONDecodeError) as error:
                raise Stop("Finder returned invalid AX bounds for background context-click")
            usable = []
            for box in viewports:
                try:
                    background_context_point(window, box)
                    left, top = max(window[0], box[0]), max(window[1], box[1])
                    right, bottom = min(window[0] + window[2], box[0] + box[2]), min(window[1] + window[3], box[1] + box[3])
                    usable.append(((right - left) * (bottom - top), box))
                except Stop:
                    continue
            if not usable:
                raise Stop(f"Finder list viewports {viewports} are not inside scenario window {window}")
            _, viewport = max(usable)
            x, y = background_context_point(window, viewport)
            self.check_target()
            subprocess.run([sys.executable, str(HERE / "mac_click.py"), "right", str(x), str(y)], check=True, timeout=10)
            self.menu_open = True
            return
        self.select(name)
        time.sleep(0.4)
        self.check_target()
        # Finder's AXShowMenu action opens no menu the AX API can read, so
        # right-click the selected row or icon through Quartz instead.
        script = f"""
function run() {{
  var se = Application("System Events");
  var p = se.processes.byName({json.dumps(self.process)});
  var w = p.attributes.byName("AXFocusedWindow").value();
  function A(el, n) {{ try {{ return el.attributes.byName(n).value(); }} catch (e) {{ return undefined; }} }}
  var hit = null;
  function walk(el, d) {{
    if (hit || d < 0) return;
    var role = A(el, "AXRole");
    if ((role === "AXRow" || role === "AXImage" || role === "AXGroup") && A(el, "AXSelected") === true) {{ hit = el; return; }}
    var k = A(el, "AXChildren") || [];
    for (var i = 0; i < k.length; i++) walk(k[i], d - 1);
  }}
  walk(w, 10);
  if (!hit) return "none";
  var p0 = A(hit, "AXPosition"), s = A(hit, "AXSize");
  return (p0[0] + Math.min(60, s[0] / 2)) + " " + (p0[1] + s[1] / 2);
}}"""
        where = osascript(script, js=True)
        if where == "none":
            raise Stop(f"no selected item to right-click for {name!r}")
        x, y = where.split()
        subprocess.run([sys.executable, str(HERE / "mac_click.py"), "right", x, y], check=True, timeout=10)
        self.menu_open = True

    def menu(self, path: list[str]) -> None:
        self.check_target()
        menu_bar_item, *items = path
        target = f"menu bar item {as_string(menu_bar_item)} of menu bar 1"
        for item in items[:-1]:
            target = f"menu item {as_string(item)} of menu 1 of {target}"
        osascript(
            f'tell application "System Events" to tell process {as_string(self.process)} to '
            f"click menu item {as_string(items[-1])} of menu 1 of {target}"
        )

    def click_key(self, row: int, col: int) -> None:
        """Click a Scientific-mode calculator key by its (row, column) in
        `crates/calculator/src/scientific_keypad.rs::LAYOUT`. Real macOS
        Calculator does not expose a usable accessible name for these keys
        (their AXTitle/AXDescription are generic), so `select` (an
        accessible-name click) cannot find them; this instead reads the
        focused window's own position live and clicks the key's measured
        offset from it (`KEYPAD_LEFT`/`KEYPAD_TOP`/`KEY_PITCH_X/Y` in that
        same file), the same way this scenario was originally recorded by
        hand with `click at`."""

        if self.process != "Calculator":
            raise Stop("click_key is only defined for calculator scenarios")
        self.check_target()
        origin = osascript(
            f'tell application "System Events" to tell process {as_string(self.process)} '
            "to get position of window 1"
        )
        window_x, window_y = (float(value.strip().rstrip(",")) for value in origin.split())
        keypad_left, keypad_top = 10.0, 132.0
        pitch_x, pitch_y = 66.0, 54.0
        key_width, key_height = 60.0, 48.0
        x = window_x + keypad_left + col * pitch_x + key_width / 2
        y = window_y + keypad_top + row * pitch_y + key_height / 2
        subprocess.run(
            [sys.executable, str(HERE / "mac_click.py"), "left", str(x), str(y)],
            check=True,
            timeout=10,
        )

    def _where_popup_title(self) -> Optional[str]:
        """The folder currently shown by the Save sheet's "Where" pop-up
        (its AXTitle is the constant label "Where:"; the folder name is
        its AXValue), or None if that pop-up isn't in the focused window."""

        script = f"""
function run() {{
  var se = Application("System Events");
  var p = se.processes.byName({json.dumps(self.process)});
  var w = p.attributes.byName("AXFocusedWindow").value();
  function A(el, n) {{ try {{ return el.attributes.byName(n).value(); }} catch (e) {{ return undefined; }} }}
  var hit = null;
  function walk(el, d) {{
    if (hit || d < 0) return;
    if (A(el, "AXRole") === "AXPopUpButton" && A(el, "AXTitle") === "Where:") {{ hit = el; return; }}
    var k = A(el, "AXChildren") || [];
    for (var i = 0; i < k.length; i++) walk(k[i], d - 1);
  }}
  walk(w, 14);
  if (!hit) return "none";
  var v = A(hit, "AXValue");
  return (typeof v === "string" && v) ? v : "none";
}}"""
        where = osascript(script, js=True)
        return None if where == "none" else where

    def _run_close_unsaved_save(self) -> dict[str, Any]:
        """The same interaction as text-editor/close-unsaved-save.json
        (new document, edit, close, name and confirm), except the Save
        sheet's "Where" is steered to this run's own sandbox with Go to
        Folder before anything is typed or confirmed. TextEdit's sheet
        otherwise defaults "Where" to the owner's real Documents folder (or
        iCloud Drive), and a save there must never happen
        (docs/behavior-suite.md safety)."""

        self.key("cmd-n")
        time.sleep(1.2)
        self.type_text("Hello")
        time.sleep(0.8)
        self.key("cmd-w")
        time.sleep(1.2)
        self.key("cmd-shift-g")
        time.sleep(0.6)
        self.type_text(str(self.sandbox))
        self.key("return")
        time.sleep(0.8)
        title = self._where_popup_title()
        if title != self.sandbox.name:
            raise Stop(f"Save sheet's Where did not land on the sandbox (saw {title!r}); refusing to save")
        self.type_text("SheetCheck")
        self.key("return")
        time.sleep(1.2)
        facts = ["windows", "saved_documents", "dialog"]
        raw = observe_raw(self.process, facts, self.baseline)
        return {
            "saved": sc.finish_observation(
                self.scenario, "saved", normalize(raw, facts, self.files_root, self.before)
            )
        }

    def run_steps(self) -> dict[str, Any]:
        if self.sid == "text-editor/close-unsaved-save":
            return self._run_close_unsaved_save()
        observations: dict[str, Any] = {}
        for step in self.scenario["steps"]:
            if "key" in step:
                self.key(step["key"])
            elif "type" in step:
                self.type_text(step["type"])
            elif "wait" in step:
                time.sleep(float(step["wait"]))
                continue
            elif "select" in step:
                self.select(step["select"], modifiers=step.get("modifiers"), double=bool(step.get("double")))
            elif "context" in step:
                self.context(step["context"])
            elif "menu" in step:
                self.menu(step["menu"])
            elif "click_key" in step:
                self.click_key(step["row"], step["col"])
            elif "focus_desktop" in step:
                osascript('tell application "Finder" to activate')
                self.check_target()
            elif "observe" in step:
                facts = step["facts"]
                raw = observe_raw(self.process, facts, self.baseline)
                observations[step["observe"]] = sc.finish_observation(
                    self.scenario, step["observe"], normalize(raw, facts, self.files_root, self.before)
                )
                continue
            time.sleep(float(step.get("settle", self.settle)))
        return observations

    # -- cleanup -----------------------------------------------------------

    def discard_autosaved(self, path: str) -> list[str]:
        """Move an Untitled document this run autosaved to the Bin (Finder,
        so it stays recoverable). Anything else is left alone."""

        name = os.path.basename(path)
        folder = os.path.dirname(path)
        if not re.fullmatch(r"Untitled( \d+)?\.(rtf|txt)", name) or not folder.endswith("com~apple~TextEdit/Documents"):
            return [f"left {name}: not an autosaved Untitled document"]
        minutes = max(1, int((time.time() - self.started) / 60) + 1)
        osascript(
            f'tell application "Finder" to delete (every file of folder (POSIX file {as_string(folder)} as alias) '
            f"whose name is {as_string(name)} and creation date > (current date) - {minutes * 60})"
        )
        return []

    def cleanup(self) -> list[str]:
        notes = []
        try:
            for _ in range(3):
                raw = observe_raw(self.process, ["dialog", "menu"], self.baseline)
                ours = (raw.get("guard") or {}).get("focused_window_is_ours") or self.app == "desktop"
                menu = (raw.get("menu") or {}).get("present") and self.menu_open
                dialog = (raw.get("dialog") or {}).get("present") and ours
                if raw.get("frontmost") == self.process and (menu or dialog):
                    osascript('tell application "System Events" to key code 53')
                    time.sleep(0.5)
                else:
                    break
        except Stop as error:
            notes.append(str(error))
        try:
            if self.process == "Finder":
                if self.app == "desktop":
                    raw = observe_raw(self.process, ["focus"], self.baseline)
                    if raw.get("frontmost") == "Finder" and not (raw.get("guard") or {}).get("windows") and \
                            (raw.get("focus") or {}).get("ax_role") in {"AXTextField", "AXTextArea"}:
                        osascript('tell application "System Events" to key code 53')
                        time.sleep(0.5)
                ids = osascript('tell application "Finder" to get id of every window')
                now = [int(x) for x in ids.replace(",", " ").split()] if ids else []
                for window_id in now:
                    if window_id not in self.finder_ids:
                        osascript(f'tell application "Finder" to close window id {window_id}')
                # Wait for the windows to go, so the next scenario's baseline
                # never mistakes a closing window for one of the owner's.
                deadline = time.monotonic() + 4
                while time.monotonic() < deadline:
                    if (observe_raw(self.process, [], []).get("guard") or {}).get("windows", 0) <= len(self.finder_ids):
                        break
                    time.sleep(0.2)
                # Quick Look's panel is not a Finder window; Escape closes it
                # when it (one of ours) has focus.
                raw = observe_raw(self.process, [], self.baseline)
                if raw.get("frontmost") == "Finder" and (raw.get("guard") or {}).get("focused_window_is_ours"):
                    osascript('tell application "System Events" to key code 53')
            elif self.process == "TextEdit":
                # With iCloud Documents on, TextEdit autosaves an edited
                # Untitled document into its iCloud folder, and closing it
                # "saving no" keeps that copy. Note each of our documents'
                # file, close it, then move a copy this run created to the Bin.
                names = osascript('tell application "TextEdit" to get name of every document')
                created = []
                for name in [n.strip() for n in names.split(",")] if names else []:
                    if name in self.documents:
                        continue
                    doc = f"(first document whose name is {as_string(name)})"
                    path = osascript(f'tell application "TextEdit" to get path of {doc}')
                    if path and path != "missing value":
                        created.append(path)
                    osascript(f'tell application "TextEdit" to close {doc} saving no')
                if not self.was_running:
                    osascript('tell application "TextEdit" to quit saving no')
                    time.sleep(1.0)
                for path in created:
                    notes.extend(self.discard_autosaved(path))
            elif self.app == "preview":
                # Preview may already be open on the owner's documents. Close
                # only this run's focused window, proven by the AX baseline.
                raw = observe_raw(self.process, [], self.baseline)
                guard = raw.get("guard") or {}
                if raw.get("frontmost") == self.process and guard.get("focused_window_is_ours"):
                    osascript('tell application "System Events" to key code 13 using {command down}')
                    time.sleep(0.5)
                if not self.was_running:
                    osascript(f'tell application id {as_string(APPS[self.app]["bundle"])} to quit')
                    time.sleep(0.5)
            elif not self.was_running:
                osascript(f'tell application id {as_string(APPS[self.app]["bundle"])} to quit')
        except Stop as error:
            notes.append(str(error))
        if self.app == "desktop":
            for path in DESKTOP.iterdir():
                entry = path.name + ("/" if path.is_dir() else "")
                if entry not in self.before and path.is_dir():
                    try:
                        path.rmdir()  # only ever removes an empty folder
                    except OSError:
                        notes.append("left a non-empty new folder on the Desktop")
        shutil.rmtree(self.work, ignore_errors=True)
        return notes


def record(path: Path, settle: float) -> dict[str, Any]:
    scenario = sc.load(path)
    sid = sc.scenario_id(path)
    run = MacRun(sid, scenario, settle)
    result: dict[str, Any] = {
        "format": sc.FORMAT,
        "scenario": sid,
        "platform": f"macOS {platform.mac_ver()[0]}",
        "recorded": datetime.date.today().isoformat(),
        "observations": {},
    }
    try:
        run.setup()
        run.launch()
        result["observations"] = run.run_steps()
    except Stop as error:
        result["error"] = str(error)
    finally:
        notes = run.cleanup()
        if notes:
            result["cleanup"] = notes
    return result


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="area/name ids (default: none; see --all)")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--missing", action="store_true", help="skip scenarios that already have a .mac.json")
    parser.add_argument("--settle", type=float, default=0.8, help="seconds to wait after each step")
    parser.add_argument("--dry-run", action="store_true", help="print results instead of writing .mac.json")
    args = parser.parse_args(argv)
    if sys.platform != "darwin":
        parser.error("record_mac.py runs on the owner's Mac")
    if not args.all and not args.scenarios:
        parser.error("name scenarios or pass --all")
    paths = sc.scenario_paths(only=[] if args.all else args.scenarios)
    paths = [path for path in paths if not sc.load(path).get("lulo_only")]
    if args.missing:
        paths = [p for p in paths if not sc.expectation_path(p).exists()]
    lock = open(LOCK, "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    failures = 0
    for path in paths:
        sid = sc.scenario_id(path)
        result = record(path, args.settle)
        if "error" in result:
            failures += 1
            print(f"ERROR {sid}: {result['error']}")
        else:
            print(f"ok    {sid}")
        if args.dry_run or "error" in result:
            print(json.dumps(result, indent=2, ensure_ascii=False))
        else:
            sc.expectation_path(path).write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
        time.sleep(0.5)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
