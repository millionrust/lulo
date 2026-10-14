#!/usr/bin/env python3
"""Walk the real macOS apps and shell and record every state (Mac side).

    python3 scripts/robot/mac/mac_walk.py [--apps Finder,Notes | --apps all] [--shell] [--settle 0.6]

Only with the owner's say-so that the Mac is idle (it moves the pointer and
presses keys). Writes, per app, .claude/mac-captures/<app>/states.jsonl and
JPEG screenshots beside it (local, never committed), then the scrubbed,
image-free summaries to tests/robot/mac/<app>.json.

Safety:
  * before every key, click or pointer move it reads HIDIdleTime; input the
    robot did not make stops the whole run at once (HumanInput);
  * it starts only when the Mac has been idle for 60 s;
  * menu items are activated only when common.may_activate allows it (an
    ellipsis item or a known panel opener, never a destructive, outward or
    account word), and whatever they open is cancelled with Escape, Cancel
    or the window's close button, never confirmed;
  * toolbar buttons are pressed only when they open a menu or popover;
  * apps it launched are quit without saving; windows of apps that were
    already running are left alone, only the ones it opened are closed;
  * it never changes a System Settings value: panes are opened by selecting
    their sidebar row.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Callable, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent.parent / "inventory"))

import common as cm  # noqa: E402

MACAX = cm.CAPTURES / "bin" / "macax"
SANDBOX = Path("/tmp/lulo-mac-robot/sandbox")
LOCK = "/tmp/lulo-mac-gui.lock"
LAST_INPUT = cm.CAPTURES / "last-input"
ESC, TAB, SPACE, RETURN = 53, 48, 49, 36
POPUP_LAYERS = range(1, 2000)  # menus (101), popovers, panels above windows

MAX_SUB_DEPTH = 2


class HumanInput(RuntimeError):
    """Someone else is using the Mac: stop now."""


class Stuck(RuntimeError):
    """Something the robot opened would not close."""


def build_macax() -> None:
    source = HERE / "macax.swift"
    if MACAX.exists() and MACAX.stat().st_mtime >= source.stat().st_mtime:
        return
    MACAX.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", "-O", "-o", str(MACAX), str(source)], check=True)


def shortcut(node: dict[str, Any]) -> str:
    key = node.get("key")
    if not key:
        return ""
    mods = node.get("mods", 0)
    glyphs = ""
    if mods & 16:
        glyphs += "🌐"
    if mods & 4:
        glyphs += "⌃"
    if mods & 2:
        glyphs += "⌥"
    if mods & 1:
        glyphs += "⇧"
    if not mods & 8:
        glyphs += "⌘"
    names = {"\x7f": "⌫", "\x08": "⌫", "\x1b": "⎋", "\r": "↩", "\t": "⇥", " ": "Space"}
    return glyphs + names.get(key, key.upper())


def menu_items(nodes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Rows of an open menu from `menuat`/`tree` nodes at depth 1."""

    items = []
    for node in nodes:
        if node.get("d") != 1 or node.get("role") != "AXMenuItem":
            continue
        frame = node.get("f") or [0, 0, 0, 0]
        if frame[3] == 0:
            continue  # an Option-key alternate shares its primary's row
        label = node.get("title")
        if not label:
            items.append({"label": "-"})
            continue
        item = {"label": label, "enabled": not node.get("disabled"), "key": shortcut(node),
                "checked": bool(node.get("mark")), "submenu": node.get("n", 0) > 0, "f": frame, "p": node["p"]}
        items.append(item)
    return items


def controls(nodes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = []
    for node in nodes:
        if node.get("sub") in cm.CHROME_SUBROLES:
            continue
        role = cm.AX_ROLES.get(node.get("role", ""))
        if node.get("sub") == "AXSearchField":
            role = "textfield"
        if node.get("sub") == "AXSwitch":
            role = "checkbox"
        if role not in cm.CONTROL_ROLES:
            continue
        label = node.get("title") or node.get("desc")
        if role == "text":
            label = label or node.get("value")
            if not label:
                continue
        if role in {"textfield", "textarea"}:
            label = label or node.get("ph")
        if role == "row" and not label:
            continue
        out.append({"role": role, "label": label or "", "enabled": not node.get("disabled"),
                    "f": node.get("f"), "ax": node.get("role"), "p": node.get("p")})
    return out


class Mac:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.settle = args.settle
        # Input from an earlier robot run counts as the robot's own.
        try:
            self.last_input = float(LAST_INPUT.read_text())
        except (OSError, ValueError):
            self.last_input = 0.0
        self.pid = 0
        self.app = ""
        self.out: Optional[Path] = None
        self.records: list[dict[str, Any]] = []
        self.seen: set[str] = set()
        self.tokens = cm.personal_tokens()
        self.started_pointer = None

    # ------------------------------------------------------------- plumbing

    def ax(self, *args: Any, timeout: float = 30) -> Any:
        result = subprocess.run([str(MACAX), *map(str, args)], capture_output=True, text=True, timeout=timeout)
        try:
            return json.loads(result.stdout or "{}")
        except json.JSONDecodeError:
            return {"error": result.stderr.strip()[:200]}

    def mark(self) -> None:
        self.last_input = time.time()
        try:
            LAST_INPUT.write_text(str(self.last_input))
        except OSError:
            pass

    def guard(self) -> None:
        idle = self.ax("idle").get("idle", 0)
        since_mine = time.time() - self.last_input
        if idle + 2.0 < since_mine and idle < 60:
            raise HumanInput(f"HID input {idle:.1f}s ago that the robot did not make (its last was {since_mine:.1f}s ago)")

    def frontmost(self) -> int:
        return self.ax("frontmost").get("pid", 0)

    def ensure_front(self, pid: Optional[int] = None) -> None:
        pid = pid or self.pid
        if self.frontmost() == pid:
            return
        self.ax("activate", pid)
        time.sleep(0.5)
        if self.frontmost() != pid:
            raise Stuck(f"could not bring pid {pid} to the front")

    def key(self, code: int, mods: str = "", front: bool = True) -> None:
        self.guard()
        if front:
            self.ensure_front()
        self.ax("key", code, mods)
        self.mark()

    def escape(self) -> None:
        self.key(ESC, front=False)

    def move(self, x: float, y: float) -> None:
        self.guard()
        self.ax("move", x, y)
        self.mark()

    def click(self, button: str, x: float, y: float) -> None:
        self.guard()
        self.ax("click", button, x, y)
        self.mark()

    def windows(self, pid: Optional[int] = None) -> dict[str, Any]:
        return self.ax("windows", pid or self.pid)

    def popups(self, pid: Optional[int] = None) -> list[dict[str, Any]]:
        return [w for w in self.ax("cgwindows", pid or self.pid) if w["layer"] in POPUP_LAYERS]

    def wait_popups_closed(self, pid: Optional[int] = None, tries: int = 4) -> bool:
        for _ in range(tries):
            if not [w for w in self.popups(pid) if w["layer"] >= 100]:
                return True
            self.escape()
            time.sleep(0.25)
        return not [w for w in self.popups(pid) if w["layer"] >= 100]

    # -------------------------------------------------------------- capture

    def shot_file(self, path: str) -> Path:
        assert self.out is not None
        safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in path)[:120]
        return self.out / f"{safe}.jpg"

    def shoot(self, path: str, *, cg: Optional[int] = None, rect: Optional[list[int]] = None) -> Optional[str]:
        target = self.shot_file(path)
        raw = target.with_suffix(".png")
        if cg:
            command = ["screencapture", "-x", "-o", "-l", str(cg), str(raw)]
            width = None
        elif rect:
            x, y, w, h = rect
            command = ["screencapture", "-x", "-R", f"{x},{y},{w},{h}", str(raw)]
            width = w
        else:
            return None
        subprocess.run(command, capture_output=True, timeout=20)
        if not raw.exists():
            return None
        if width is None:
            sizes = [w["f"][2] for w in self.ax("cgwindows") if w["id"] == cg]
            width = sizes[0] if sizes else 1200
        self.ax("shrink", raw, target, width)
        raw.unlink(missing_ok=True)
        return f"{self.app}/{target.name}" if target.exists() else None

    def record(self, state: dict[str, Any]) -> None:
        state.setdefault("app", self.app)
        key = state["path"]
        if key in self.seen:
            n = 2
            while f"{key} #{n}" in self.seen:
                n += 1
            state["path"] = f"{key} #{n}"
        self.seen.add(state["path"])
        self.records.append(state)
        print(f"  {state['path']}  [{state['kind']}] "
              f"{len(state.get('items') or state.get('controls') or [])} rows", flush=True)

    def capture_window(self, path: str, kind: str, index: int, extra: Optional[dict] = None) -> None:
        info = self.windows()
        win = next((w for w in info["windows"] if w["i"] == index), None)
        if win is None:
            return
        tree = self.ax("tree", self.pid, f"w{index}", "--max", 3000)
        state = {"path": path, "kind": kind, "title": win.get("title"), "window": win.get("f", [0, 0, 0, 0])[2:],
                 "subrole": win.get("sub"), "controls": controls(tree.get("nodes", [])),
                 "shot": self.shoot(path, cg=win.get("cg"))}
        state.update(extra or {})
        self.record(state)

    def capture_sheet(self, path: str, window_index: int, sheet_path: str) -> None:
        info = self.windows()
        win = next((w for w in info["windows"] if w["i"] == window_index), {})
        tree = self.ax("tree", self.pid, sheet_path, "--max", 2000)
        nodes = tree.get("nodes", [])
        frame = nodes[0].get("f") if nodes else None
        self.record({"path": path, "kind": "sheet", "title": nodes[0].get("title") if nodes else None,
                     "window": frame[2:] if frame else None, "controls": controls(nodes),
                     "shot": self.shoot(path, cg=win.get("cg"))})

    def capture_popup(self, path: str, kind: str, new: list[dict[str, Any]], pid: Optional[int] = None) -> None:
        x0 = min(w["f"][0] for w in new)
        y0 = min(w["f"][1] for w in new)
        x1 = max(w["f"][0] + w["f"][2] for w in new)
        y1 = max(w["f"][1] + w["f"][3] for w in new)
        top = max(new, key=lambda w: (w["layer"], w["f"][2] * w["f"][3]))
        found: dict[str, Any] = {}
        for fy in (min(12, top["f"][3] / 2), 30, top["f"][3] / 2):
            found = self.ax("menuat", top["f"][0] + top["f"][2] / 2, top["f"][1] + fy)
            if found.get("found"):
                break
        rect = [x0, y0, x1 - x0, y1 - y0]
        window_nodes: list[dict[str, Any]] = []
        if not found.get("found"):
            # A SwiftUI popover (Control Centre, menu-bar extras) is its own
            # AX window in the owning process, larger than what it draws.
            owner = top["pid"]
            for win in self.ax("windows", owner).get("windows", []):
                if win.get("f") == top["f"] or win.get("cg") == top["id"]:
                    tree = self.ax("tree", owner, f"w{win['i']}", "--max", 2500, "--abs")
                    window_nodes = tree.get("nodes", [])
                    break
            boxes = [n["f"] for n in window_nodes if n.get("d") in (1, 2) and n.get("f") and n["f"][2] > 20
                     and n["f"][3] > 20]
            if boxes:
                bx0 = min(b[0] for b in boxes)
                by0 = min(b[1] for b in boxes)
                bx1 = max(b[0] + b[2] for b in boxes)
                by1 = max(b[1] + b[3] for b in boxes)
                rect = [bx0, by0, bx1 - bx0, by1 - by0]
                for node in window_nodes:
                    if node.get("f"):
                        node["f"] = [node["f"][0] - bx0, node["f"][1] - by0, node["f"][2], node["f"][3]]
        state: dict[str, Any] = {"path": path, "kind": kind, "window": rect[2:],
                                 "shot": self.shoot(path, rect=rect)}
        if window_nodes:
            state["controls"] = controls(window_nodes)
            state["kind"] = "popover"
        elif found.get("found"):
            nodes = found["nodes"]
            if nodes and nodes[0].get("role") == "AXMenu":
                state["items"] = menu_items(nodes)
                state["kind"] = "menu" if kind != "context" else "context"
            else:
                state["controls"] = controls(nodes)
                state["kind"] = "popover"
        self.record(state)

    # ---------------------------------------------------- probe and dismiss

    def snapshot(self) -> dict[str, Any]:
        info = self.windows()
        return {
            "keys": {(w["title"], w["sub"], tuple(w.get("f", []))) for w in info["windows"]},
            "sheets": {(w["title"], w["sub"], tuple(w.get("f", []))): w["sheets"] for w in info["windows"]},
            "cg": {w["id"] for w in info["cg"]},
            "front": self.frontmost(),
            "all_cg": {w["id"] for w in self.ax("cgwindows")},
            "apps": {a["bundle"] for a in self.ax("apps")},
        }

    def probe(self, path: str, act: Callable[[], Any], kind_hint: str = "window") -> str:
        """Do `act`, record whatever it opened under `path`, then cancel it."""

        before = self.snapshot()
        act()
        time.sleep(self.settle + 0.4)
        info = self.windows()
        new_windows = [w for w in info["windows"]
                       if (w["title"], w["sub"], tuple(w.get("f", []))) not in before["keys"] and not w["minimized"]]
        new_sheets = [w for w in info["windows"] if w["sheets"] > before["sheets"].get((w["title"], w["sub"], tuple(w.get("f", []))), 0)]
        popups = [w for w in info["cg"] if w["id"] not in before["cg"] and w["layer"] in POPUP_LAYERS]
        front = self.frontmost()
        outcome = "none"
        if front != before["front"] and front != self.pid:
            # Another app came forward (Help, Accounts…). Record it, cancel
            # anything our own app may have opened, and go back.
            self.escape()
            time.sleep(0.3)
            other = [w for w in self.ax("cgwindows") if w["id"] not in before["all_cg"] and w["layer"] == 0]
            self.record({"path": path, "kind": "other-app", "title": self.ax("frontmost").get("name"),
                         "window": other[0]["f"][2:] if other else None, "controls": [],
                         "shot": self.shoot(path, cg=other[0]["id"]) if other else None})
            front_bundle = self.ax("frontmost").get("bundle", "")
            if front_bundle and front_bundle not in before["apps"]:
                # The robot started it: quit it again.
                subprocess.run(["osascript", "-e", f'tell application id "{front_bundle}" to quit'],
                               capture_output=True, timeout=30)
            self.ax("activate", self.pid)
            time.sleep(0.6)
            return "other-app"
        if new_sheets:
            win = new_sheets[0]
            tree = self.ax("tree", self.pid, f"w{win['i']}", "--depth", 1)
            sheet = next((n for n in tree.get("nodes", []) if n.get("role") == "AXSheet"), None)
            if sheet:
                self.capture_sheet(path, win["i"], sheet["p"])
                outcome = "sheet"
        elif new_windows:
            win = new_windows[0]
            self.capture_window(path, "panel" if win["sub"] in {"AXFloatingWindow", "AXSystemFloatingWindow"} else
                                ("dialog" if win["sub"] in {"AXDialog", "AXSystemDialog"} else kind_hint), win["i"])
            outcome = "window"
        elif popups:
            self.capture_popup(path, "popover" if kind_hint != "context" else "context", popups)
            outcome = "popup"
        else:
            focused = next((w for w in info["windows"] if w.get("focused")), None)
            if focused:
                self.capture_window(path, "in-window", focused["i"], {"note": "no new window, sheet or popup"})
            outcome = "in-window"
        self.dismiss(before)
        # A system prompt from another process (Dictation, permissions)
        # is never answered by the robot: stop this app and report it.
        strays = [w for w in self.ax("cgwindows") if w["id"] not in before["all_cg"] and w["pid"] != self.pid
                  and w["owner"] not in {"Window Server", "Dock"} and w["layer"] > 0]
        if strays:
            self.escape()
            time.sleep(0.5)
            strays = [w for w in self.ax("cgwindows") if w["id"] in {s["id"] for s in strays}]
            if strays:
                raise Stuck(f"a prompt from {strays[0]['owner']} is open; left for a human")
        return outcome

    def dismiss(self, before: dict[str, Any]) -> None:
        for attempt in range(5):
            info = self.windows()
            new_windows = [w for w in info["windows"]
                           if (w["title"], w["sub"], tuple(w.get("f", []))) not in before["keys"] and not w["minimized"]]
            new_sheets = [w for w in info["windows"] if w["sheets"] > before["sheets"].get((w["title"], w["sub"], tuple(w.get("f", []))), 0)]
            frames = {tuple(w.get("f", [])) for w in new_windows}
            popups = [w for w in info["cg"] if w["id"] not in before["cg"] and w["layer"] in POPUP_LAYERS
                      and tuple(w["f"]) not in frames]
            if not (new_windows or new_sheets or popups):
                return
            if attempt == 0 or popups:
                self.escape()
                time.sleep(0.4)
                continue
            target = new_sheets[0] if new_sheets else (new_windows[0] if new_windows else None)
            if target is None:
                self.escape()
                time.sleep(0.4)
                continue
            tree = self.ax("tree", self.pid, f"w{target['i']}", "--max", 2500, "--rows", 0)
            nodes = tree.get("nodes", [])
            if new_sheets:
                sheet = next((n for n in nodes if n.get("role") == "AXSheet"), None)
                prefix = sheet["p"] if sheet else f"w{target['i']}"
                nodes = [n for n in nodes if n["p"].startswith(prefix)]
            cancel = next((n for n in nodes if n.get("role") == "AXButton"
                           and (n.get("title") or n.get("desc")) in {"Cancel", "Close", "Don’t Save", "Don't Save"}
                           and (n.get("title") or n.get("desc")) != "Don’t Save"), None)
            close = next((n for n in nodes if n.get("sub") == "AXCloseButton" and not n.get("disabled")), None)
            if cancel:
                self.ax("press", self.pid, cancel["p"])
            elif close and not new_sheets:
                self.ax("press", self.pid, close["p"])
            else:
                self.escape()
            time.sleep(0.6)
        raise Stuck("could not dismiss what the robot opened")

    # ---------------------------------------------------------------- menus

    def walk_menus(self, apple: bool = False) -> None:
        bar = self.ax("menubar", self.pid, "--deep").get("menus", [])
        for index, menu in enumerate(bar):
            if index == 0 and not apple:
                continue
            if index != 0 and apple:
                break
            title = menu["label"] or menu.get("desc") or f"#{index}"
            key = "menu/Apple" if apple else "menu/" + cm.menu_key(index - 1, title)
            self.open_menu(key, menu["p"])
        if not apple:
            self.walk_menu_items(bar)

    def open_menu(self, key: str, path: str) -> None:
        self.ensure_front()
        self.guard()
        pops: list[dict[str, Any]] = []
        for _attempt in range(2):
            self.ax("press", self.pid, path)
            time.sleep(0.6)
            pops = [w for w in self.popups() if w["layer"] >= 100]
            if pops:
                break
            self.ensure_front()
        if not pops:
            print(f"  ! {key}: the menu did not open", flush=True)
            self.escape()
            return
        try:
            self.capture_popup(key, "menu", pops)
            state = self.records[-1]
            if self.args.no_submenus:
                return
            self.walk_submenus(key, state.get("items") or [], pops, 1)
        finally:
            self.wait_popups_closed()

    def walk_submenus(self, key: str, items: list[dict[str, Any]], open_windows: list[dict[str, Any]], depth: int) -> None:
        for item in items:
            if not item.get("submenu") or not item.get("enabled") or item["label"] in cm.DYNAMIC_SUBMENUS:
                continue
            x, y, w, h = item["f"]
            self.move(x + min(w - 8, 60), y + h / 2)
            new: list[dict[str, Any]] = []
            for _ in range(3):
                time.sleep(0.45)
                # AppKit reuses submenu windows, so find the one beside this
                # row rather than a new window id.
                new = [p for p in self.popups() if p["layer"] >= 100 and abs(p["f"][1] - y) <= 16
                       and (p["f"][0] >= x + w - 12 or p["f"][0] + p["f"][2] <= x + 12)]
                if new:
                    break
            if not new:
                print(f"    (no submenu window for {item['label']})", flush=True)
                continue
            new = new[:1]
            sub_key = f"{key}/{item['label']}"
            self.capture_popup(sub_key, "menu", new)
            if depth < MAX_SUB_DEPTH:
                self.walk_submenus(sub_key, self.records[-1].get("items") or [], new, depth + 1)
            # Back onto the parent row so the next hover starts from there.
            self.move(x + 10, y + h / 2)
            time.sleep(0.2)

    def walk_menu_items(self, bar: list[dict[str, Any]]) -> None:
        """Activate the menu items that open a sheet, panel or window."""

        opened: dict[tuple[str, str], dict[str, Any]] = {}
        for state in self.records:
            for row in state.get("items") or []:
                opened[(state["path"], row.get("label"))] = row

        def visit(menu_title: str, items: list[dict[str, Any]], trail: list[str]) -> None:
            for item in items:
                label = item.get("label") or ""
                if item.get("items"):
                    if label not in cm.DYNAMIC_SUBMENUS:
                        visit(menu_title, item["items"], trail + [label])
                    continue
                if item.get("submenu"):
                    continue
                # A closed menu's enabled flags are stale: use the ones the
                # open menu showed.
                seen = opened.get(("menu/" + "/".join([menu_title, *trail]), label))
                if seen is None or not seen.get("enabled"):
                    continue
                if label.startswith("Settings") or label.startswith("Preferences"):
                    self.settings(item["p"])
                    continue
                if not cm.may_activate(label):
                    continue
                # The whole path counts: "Erase Deleted Items ▸ On My Mac…".
                if any(cm.DENY.search(part) for part in [menu_title, *trail]):
                    continue
                path = "item/" + "/".join([menu_title, *trail, label])
                try:
                    self.ensure_front()
                    self.guard()
                    self.probe(path, lambda p=item["p"]: self.ax("press", self.pid, p))
                except Stuck as error:
                    print(f"  ! {path}: {error}", flush=True)
                    raise

        for index, menu in enumerate(bar):
            if index == 0:
                continue
            title = cm.menu_key(index - 1, menu["label"])
            if title == "Help":
                continue
            visit(title, menu.get("items") or [], [])

    # ------------------------------------------------------------- settings

    def settings(self, item_path: str) -> None:
        before = self.snapshot()
        self.ensure_front()
        self.ax("press", self.pid, item_path)
        time.sleep(self.settle + 0.8)
        info = self.windows()
        new = [w for w in info["windows"] if (w["title"], w["sub"], tuple(w.get("f", []))) not in before["keys"]]
        if not new:
            focused = next((w for w in info["windows"] if w.get("focused")), None)
            if focused and focused["title"] != "":
                new = [focused]
        if not new:
            print("  ! Settings opened no window", flush=True)
            return
        win = new[0]
        self.capture_window("settings", "settings", win["i"])
        labels = self.tab_labels(win["i"], settings=True)
        first = self.selected_tab(win["i"], settings=True)
        for label in labels:
            if cm.DENY.search(label) and label not in {"Accounts"}:
                continue
            self.guard()
            if not self.press_tab(win["i"], label, settings=True):
                continue
            time.sleep(self.settle + 0.3)
            index = next((w["i"] for w in self.windows()["windows"]
                          if w.get("focused") or w["title"] == label), win["i"])
            self.capture_window(f"settings/{label}", "settings", index)
        if first:
            self.press_tab(win["i"], first, settings=True)
            time.sleep(0.3)
        self.close_new_windows(before)

    def close_new_windows(self, before: dict[str, Any]) -> None:
        for _ in range(4):
            info = self.windows()
            new = [w for w in info["windows"] if (w["title"], w["sub"], tuple(w.get("f", []))) not in before["keys"]]
            if not new:
                return
            tree = self.ax("tree", self.pid, f"w{new[0]['i']}", "--depth", 3, "--rows", 0)
            close = next((n for n in tree.get("nodes", []) if n.get("sub") == "AXCloseButton"), None)
            if close:
                self.ax("press", self.pid, close["p"])
            else:
                self.key(13, "cmd")  # ⌘W on our own window
            time.sleep(0.5)

    # ---------------------------------------------------- window surfaces

    def main_window(self) -> Optional[dict[str, Any]]:
        info = self.windows()
        wins = [w for w in info["windows"] if w.get("cg") and not w["minimized"]]
        return next((w for w in wins if w.get("focused")), wins[0] if wins else None)

    def walk_toolbar(self, win: dict[str, Any], safe_buttons: set[str]) -> None:
        tree = self.ax("tree", self.pid, f"w{win['i']}", "--max", 3000)
        nodes = tree.get("nodes", [])
        bars = [n for n in nodes if n.get("role") == "AXToolbar"]
        if not bars:
            return
        prefix = bars[0]["p"] + "/"
        for node in [n for n in nodes if n["p"].startswith(prefix)]:
            label = node.get("title") or node.get("desc") or ""
            if node.get("help") and (not label or ("." in label and " " not in label)):
                label = node["help"]  # an SF Symbol name is not a label
            role = node.get("role")
            if node.get("disabled") or not label or cm.DENY.search(label):
                continue
            if role in {"AXMenuButton", "AXPopUpButton"} or (role == "AXButton" and label in safe_buttons):
                self.ensure_front()
                self.probe(f"toolbar/{label}", lambda p=node["p"]: self.ax("press", self.pid, p), "popover")

    def tab_nodes(self, index: int, settings: bool = False) -> list[dict[str, Any]]:
        """Tab selectors only: a settings window's toolbar buttons, a tab
        view's tabs, or a segmented view switch inside a toolbar. Radio
        buttons elsewhere are setting values and are never pressed."""

        nodes = self.ax("tree", self.pid, f"w{index}", "--max", 3000).get("nodes", [])
        bars = [n for n in nodes if n.get("role") == "AXToolbar"]
        tabgroups = [n for n in nodes if n.get("role") == "AXTabGroup"]
        out: list[dict[str, Any]] = []
        if settings and bars:
            prefix = bars[0]["p"] + "/"
            out = [n for n in nodes if n["p"].startswith(prefix)
                   and n.get("role") in {"AXButton", "AXRadioButton"} and (n.get("title") or n.get("desc"))]
        if not out:
            groups = tabgroups + ([] if settings else [n for n in nodes if n.get("role") == "AXRadioGroup"
                                                        and any(n["p"].startswith(b["p"] + "/") for b in bars)])
            for group in groups[:1] if settings else groups:
                out += [n for n in nodes if n.get("role") == "AXRadioButton" and n["p"].rsplit("/", 1)[0] == group["p"]]
        return out

    @staticmethod
    def tab_label(node: dict[str, Any]) -> str:
        return node.get("title") or node.get("desc") or ""

    def tab_labels(self, index: int, settings: bool = False) -> list[str]:
        return [self.tab_label(n) for n in self.tab_nodes(index, settings) if self.tab_label(n)]

    def selected_tab(self, index: int, settings: bool = False) -> Optional[str]:
        for node in self.tab_nodes(index, settings):
            if node.get("selected") or node.get("value") == "1":
                return self.tab_label(node)
        return None

    def press_tab(self, index: int, label: str, settings: bool = False) -> bool:
        node = next((n for n in self.tab_nodes(index, settings) if self.tab_label(n) == label), None)
        if node is None:
            return False
        if self.ax("press", self.pid, node["p"]).get("ok"):
            return True
        # Some segmented controls refuse AXPress: click the segment.
        frame = self.ax("frame", self.pid, node["p"]).get("f")
        if not frame:
            return False
        self.ensure_front()
        self.click("left", frame[0] + frame[2] / 2, frame[1] + frame[3] / 2)
        return True

    def walk_tabs(self, win: dict[str, Any], labels: Optional[set[str]] = None) -> None:
        first = self.selected_tab(win["i"])
        for label in self.tab_labels(win["i"]):
            if (labels and label not in labels) or cm.DENY.search(label):
                continue
            self.guard()
            if not self.press_tab(win["i"], label):
                continue
            time.sleep(self.settle + 0.4)
            self.capture_window(f"tab/{label}", "tab", win["i"])
        if first:
            self.press_tab(win["i"], first)
            time.sleep(0.3)

    def context_at(self, path: str, x: float, y: float) -> None:
        self.ensure_front()
        before = self.snapshot()
        self.click("right", x, y)
        time.sleep(self.settle)
        pops = [w for w in self.popups() if w["id"] not in before["cg"] and w["layer"] >= 100]
        if pops:
            self.capture_popup(path, "context", pops)
        self.wait_popups_closed()
        self.dismiss(before)

    def context_on(self, path: str, win: dict[str, Any], match: Callable[[dict[str, Any]], bool], where: str = "center") -> None:
        tree = self.ax("tree", self.pid, f"w{win['i']}", "--max", 3000, "--abs")
        node = next((n for n in tree.get("nodes", []) if match(n) and n.get("f") and n["f"][2] > 8), None)
        if node is None:
            # Fall back on the middle of the window's content.
            frame = win.get("f")
            if not frame:
                print(f"  ! {path}: no target", flush=True)
                return
            node = {"f": [frame[0], frame[1] + 60, frame[2], max(frame[3] - 80, 40)]}
        x, y, w, h = node["f"]
        if where == "empty":
            px, py = x + w - 30, y + h - 30
        elif where == "top":
            px, py = x + min(w / 2, 80), y + min(h / 2, 10)
        else:
            px, py = x + w / 2, y + h / 2
        self.context_at(path, px, py)

    # ------------------------------------------------------------ app walk

    def walk_app(self, name: str, spec: dict[str, Any]) -> None:
        self.app = name
        self.out = cm.CAPTURES / name
        if self.out.exists():
            shutil.rmtree(self.out)
        self.out.mkdir(parents=True)
        self.records, self.seen = [], set()
        bundle = spec["bundle"]
        print(f"== {name}", flush=True)
        self.guard()
        was_running = bool(self.ax("pidof", bundle).get("pid"))
        baseline_windows: set = set()
        if was_running:
            pid = self.ax("pidof", bundle)["pid"]
            baseline_windows = {(w["title"], w["sub"], tuple(w.get("f", []))) for w in self.ax("windows", pid)["windows"]}
        subprocess.run(spec["open"], check=False, capture_output=True, timeout=30)
        deadline = time.time() + 20
        while time.time() < deadline:
            self.pid = self.ax("pidof", bundle).get("pid", 0)
            if self.pid and (spec.get("windowless") or self.windows()["windows"]):
                break
            time.sleep(0.5)
        if not self.pid:
            print(f"  ! {name} did not start", flush=True)
            return
        time.sleep(spec.get("startup", 2.0))
        self.ensure_front()
        try:
            win = self.main_window()
            if win:
                self.capture_window("window/main", "window", win["i"])
            if spec.get("before"):
                spec["before"](self)
                win = self.main_window()
            # Window surfaces first, while the window is as it opened.
            win = self.main_window()
            if win:
                if spec.get("tabs", True):
                    self.walk_tabs(win, spec.get("tab_labels"))
                self.walk_toolbar(win, spec.get("toolbar_safe", {"Search"}))
                for path, match, where in spec.get("contexts", []):
                    win = self.main_window() or win
                    self.context_on(path, win, match, where)
            else:
                print("  ! no main window for tabs, toolbar and context menus", flush=True)
            if spec.get("extra"):
                spec["extra"](self)
            self.walk_menus()
        finally:
            self.wait_popups_closed()
            self.finish_app(name, bundle, was_running, baseline_windows, spec)

    def finish_app(self, name: str, bundle: str, was_running: bool, baseline: set, spec: dict[str, Any]) -> None:
        if not was_running:
            saving = " saving no" if spec.get("documents") else ""
            subprocess.run(["osascript", "-e", f'tell application id "{bundle}" to quit{saving}'],
                           capture_output=True, timeout=30)
            time.sleep(1.0)
        else:
            self.close_new_windows({"keys": baseline, "sheets": {}, "cg": set(), "front": 0, "all_cg": set()})
        self.write_app()

    def write_app(self) -> None:
        assert self.out is not None
        with open(self.out / "states.jsonl", "w") as handle:
            for state in self.records:
                handle.write(json.dumps(state, ensure_ascii=False) + "\n")
        cm.SUMMARIES.mkdir(parents=True, exist_ok=True)
        vocab = cm.menu_vocabulary(self.records) | getattr(self, "vocab", set())
        summary = [cm.summary_for_commit(s, self.tokens, vocab) for s in self.records]
        # One state per line: compact, and a re-run diffs state by state.
        rows = ",\n".join(json.dumps(state, ensure_ascii=False, separators=(",", ":")) for state in summary)
        (cm.SUMMARIES / f"{self.app}.json").write_text(
            f'{{"app":{json.dumps(self.app)},"source":"scripts/robot/mac/mac_walk.py","states":[\n{rows}\n]}}\n')
        print(f"  wrote {len(self.records)} states", flush=True)


# ------------------------------------------------------------ app specs ----


def sandbox() -> None:
    if SANDBOX.exists():
        shutil.rmtree(SANDBOX)
    (SANDBOX / "Folder A").mkdir(parents=True)
    (SANDBOX / "notes.txt").write_text("Lulo robot sample text.\nSecond line.\n")
    icon = "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns"
    subprocess.run(["sips", "-s", "format", "png", "-Z", "512", icon, "--out", str(SANDBOX / "picture.png")],
                   capture_output=True, timeout=30)


def role_is(*roles: str) -> Callable[[dict[str, Any]], bool]:
    return lambda n: n.get("role") in roles


def finder_extra(mac: Mac) -> None:
    win = mac.main_window()
    if not win:
        return
    mac.context_on("context/item", win, lambda n: n.get("role") in {"AXRow", "AXImage", "AXStaticText"}
                   and (n.get("title") == "notes.txt" or n.get("value") == "notes.txt"), "center")


def settings_extra(mac: Mac) -> None:
    """Every System Settings pane, opened by selecting its sidebar row."""

    win = mac.main_window()
    if not win:
        return
    tree = mac.ax("tree", mac.pid, f"w{win['i']}", "--max", 4000, "--rows", 200)
    nodes = tree.get("nodes", [])
    outline = next((n for n in nodes if n.get("role") == "AXOutline"), None)
    if outline is None:
        print("  ! no sidebar outline", flush=True)
        return
    depth = outline["p"].count("/") + 1
    rows = [n for n in nodes if n.get("role") == "AXRow" and n["p"].startswith(outline["p"] + "/")
            and n["p"].count("/") == depth]
    labels = {}
    for row in rows:
        texts = [n.get("value") or n.get("title") for n in nodes
                 if n["p"].startswith(row["p"] + "/") and n.get("role") == "AXStaticText"]
        texts = [t for t in texts if t]
        if texts:
            labels[row["p"]] = texts[0]
    first_label = None
    for index, row in enumerate(rows):
        label = labels.get(row["p"])
        if not label:
            continue
        if index < 3 and any(token.casefold() in label.casefold() for token in mac.tokens):
            label = "Apple Account"
        first_label = first_label or row
        mac.guard()
        result = mac.ax("select", mac.pid, row["p"])
        if not result.get("ok"):
            continue
        time.sleep(mac.settle + 1.0)
        info = mac.main_window()
        if info:
            mac.capture_window(f"pane/{label}", "pane", info["i"])
            if label == "General":
                general_subpages(mac, info)
                mac.ax("select", mac.pid, row["p"])
                time.sleep(0.8)


def general_subpages(mac: Mac, win: dict[str, Any]) -> None:
    tree = mac.ax("tree", mac.pid, f"w{win['i']}", "--max", 4000)
    buttons = [n for n in tree.get("nodes", []) if n.get("role") == "AXButton" and (n.get("title") or n.get("desc"))
               and n.get("f", [0])[0] > 200]
    skip = {"Transfer or Reset", "Device Management", "Back", "Forward"}
    for button in buttons:
        label = button.get("title") or button.get("desc")
        if label in skip or cm.DENY.search(label):
            continue
        mac.guard()
        mac.ax("press", mac.pid, button["p"])
        time.sleep(mac.settle + 1.0)
        mac.capture_window(f"pane/General/{label}", "pane", win["i"])
        back = next((n for n in mac.ax("tree", mac.pid, f"w{win['i']}", "--max", 600).get("nodes", [])
                     if n.get("role") == "AXButton" and (n.get("desc") == "Back" or n.get("title") == "Back")), None)
        if back:
            mac.ax("press", mac.pid, back["p"])
        else:
            mac.key(33, "cmd")  # ⌘[
        time.sleep(0.8)


def textedit_before(mac: Mac) -> None:
    pass


def quicktime_before(mac: Mac) -> None:
    # It opens on an Open panel: that is window/main; cancel it for the menus.
    mac.escape()
    time.sleep(0.6)


APPS: dict[str, dict[str, Any]] = {
    "Finder": {"bundle": "com.apple.finder", "open": ["open", str(SANDBOX)],
               "contexts": [("context/background", role_is("AXList", "AXOutline", "AXScrollArea"), "empty")],
               "extra": finder_extra, "toolbar_safe": {"Search", "Edit Tags"}},
    "TextEdit": {"bundle": "com.apple.TextEdit", "open": ["open", "-a", "TextEdit", str(SANDBOX / "notes.txt")],
                 "documents": True, "contexts": [("context/text", role_is("AXTextArea"), "center")]},
    "Preview": {"bundle": "com.apple.Preview", "open": ["open", "-a", "Preview", str(SANDBOX / "picture.png")],
                "documents": True, "contexts": [("context/image", role_is("AXImage", "AXScrollArea"), "center")]},
    "Notes": {"bundle": "com.apple.Notes", "open": ["open", "-a", "Notes"], "startup": 3.0,
              "toolbar_safe": {"Search", "Format", "Aa"},
              "contexts": [("context/note-list", role_is("AXTable", "AXOutline", "AXList"), "top")]},
    "Terminal": {"bundle": "com.apple.Terminal", "open": ["open", "-a", "Terminal"],
                 "contexts": [("context/terminal", role_is("AXTextArea"), "center")]},
    "Calculator": {"bundle": "com.apple.calculator", "open": ["open", "-a", "Calculator"]},
    "Clock": {"bundle": "com.apple.clock", "open": ["open", "-a", "Clock"]},
    "Weather": {"bundle": "com.apple.weather", "open": ["open", "-a", "Weather"], "startup": 4.0},
    "System Settings": {"bundle": "com.apple.systempreferences", "open": ["open", "-b", "com.apple.systempreferences"],
                        "startup": 3.0, "extra": settings_extra, "tabs": False},
    "Activity Monitor": {"bundle": "com.apple.ActivityMonitor", "open": ["open", "-a", "Activity Monitor"],
                         "contexts": [("context/process", role_is("AXOutline", "AXTable"), "top")]},
    "Mail": {"bundle": "com.apple.mail", "open": ["open", "-a", "Mail"], "startup": 4.0, "toolbar_safe": set()},
    "Calendar": {"bundle": "com.apple.iCal", "open": ["open", "-a", "Calendar"], "startup": 3.0, "toolbar_safe": set()},
    "Archive Utility": {"bundle": "com.apple.archiveutility",
                        "open": ["open", "-a", "/System/Library/CoreServices/Applications/Archive Utility.app"],
                        "windowless": True},
    "QuickTime Player": {"bundle": "com.apple.QuickTimePlayerX", "open": ["open", "-a", "QuickTime Player"],
                         "before": quicktime_before},
}


# ---------------------------------------------------------------- shell ----


def walk_shell(mac: Mac) -> None:
    mac.app = "Shell"
    mac.out = cm.CAPTURES / "Shell"
    if mac.out.exists():
        shutil.rmtree(mac.out)
    mac.out.mkdir(parents=True)
    mac.records, mac.seen = [], set()
    print("== Shell", flush=True)
    finder = mac.ax("pidof", "com.apple.finder")["pid"]
    try:
        # The Apple menu, through Finder's menu bar.
        mac.pid = finder
        mac.ensure_front()
        mac.no_sub_backup = mac.args.no_submenus
        mac.args.no_submenus = True
        mac.walk_menus(apple=True)
        mac.args.no_submenus = mac.no_sub_backup
        # Menu-bar extras (Control Centre owns most of them).
        for bundle in ("com.apple.controlcenter", "com.apple.systemuiserver", "com.apple.Spotlight"):
            pid = mac.ax("pidof", bundle).get("pid")
            if not pid:
                continue
            extras = mac.ax("menubar", pid, "--extras").get("menus", [])
            for extra in extras:
                label = (extra.get("desc") or extra.get("label") or extra.get("id") or "extra").split(",")[0]
                if cm.DENY.search(label) and "Control" not in label:
                    continue
                path = "shell/extra/" + label
                known = {w["id"] for w in mac.ax("cgwindows")}
                mac.guard()
                mac.ax("press", pid, extra["p"])
                time.sleep(1.0)
                new = [w for w in mac.ax("cgwindows") if w["id"] not in known and w["layer"] > 0
                       and w["owner"] not in {"Window Server"}]
                if new:
                    mac.pid = new[0]["pid"]
                    mac.capture_popup(path, "popover", new)
                mac.escape()
                time.sleep(0.6)
                mac.escape()
                time.sleep(0.3)
        # Spotlight.
        mac.pid = finder
        mac.ensure_front()
        known = {w["id"] for w in mac.ax("cgwindows")}
        mac.key(SPACE, "cmd", front=False)
        time.sleep(1.2)
        new = [w for w in mac.ax("cgwindows") if w["id"] not in known and w["owner"] != "Window Server"]
        if new:
            mac.pid = new[0]["pid"]
            mac.capture_popup("shell/spotlight", "popover", new)
        mac.escape()
        time.sleep(0.6)
        # The app switcher (⌘Tab, cancelled with Escape before ⌘ is let go).
        mac.pid = finder
        mac.ensure_front()
        known = {w["id"] for w in mac.ax("cgwindows")}
        mac.guard()
        mac.ax("keydown", 55, "cmd")
        mac.ax("key", TAB, "cmd")
        mac.mark()
        time.sleep(1.0)
        new = [w for w in mac.ax("cgwindows") if w["id"] not in known and w["owner"] != "Window Server"]
        if new:
            mac.pid = new[0]["pid"]
            mac.capture_popup("shell/app-switcher", "popover", new)
        mac.ax("key", ESC, "cmd")
        mac.ax("keyup", 55, "")
        mac.mark()
        time.sleep(0.6)
        # Dock menus.
        dock = mac.ax("pidof", "com.apple.dock")["pid"]
        tree = mac.ax("tree", dock, "app", "--depth", 3, "--rows", 60, "--abs")
        items = [n for n in tree.get("nodes", []) if n.get("role") == "AXDockItem"]
        wanted = [n for n in items if n.get("title") in {"Finder", "Trash", "Bin", "Calculator", "System Settings",
                                                           "Downloads", "Launchpad", "Apps"}][:6]
        for item in wanted:
            known = {w["id"] for w in mac.ax("cgwindows")}
            mac.guard()
            mac.ax("press", dock, item["p"], "AXShowMenu")
            time.sleep(0.8)
            new = [w for w in mac.ax("cgwindows") if w["id"] not in known and w["layer"] > 0]
            if new:
                mac.pid = dock
                path = f"shell/dock-menu/{item['title']}"
                mac.capture_popup(path, "menu", new)
                # A Dock menu hangs off its Dock item, not a window.
                sub = mac.ax("tree", dock, item["p"], "--depth", 2, "--abs").get("nodes", [])
                menu = next((n for n in sub if n.get("role") == "AXMenu"), None)
                if menu is not None and mac.records and mac.records[-1]["path"] == path:
                    rows = mac.ax("tree", dock, menu["p"], "--depth", 1, "--abs").get("nodes", [])
                    mac.records[-1]["items"] = menu_items(rows)
                    mac.records[-1]["kind"] = "menu"
            mac.escape()
            time.sleep(0.5)
        # The desktop's context menu, on a point no window covers.
        point = uncovered_point(mac)
        if point:
            mac.pid = finder
            known = {w["id"] for w in mac.ax("cgwindows")}
            mac.click("right", *point)
            time.sleep(0.8)
            new = [w for w in mac.ax("cgwindows") if w["id"] not in known and w["layer"] > 0]
            if new:
                mac.capture_popup("shell/desktop-context", "context", new)
            mac.escape()
            time.sleep(0.4)
        else:
            print("  ! desktop context menu: no uncovered desktop point", flush=True)
    finally:
        mac.write_app()


def uncovered_point(mac: Mac) -> Optional[tuple[float, float]]:
    wins = [w for w in mac.ax("cgwindows") if 0 <= w["layer"] < 1000 and w["owner"] not in {"Window Server", "Dock"}
            and not (w["owner"] == "Finder" and w["layer"] < 0)]
    screen = max((w["f"] for w in mac.ax("cgwindows") if w["owner"] == "Dock" and w["layer"] < 0),
                 key=lambda f: f[2] * f[3], default=None)
    width, height = (screen[2], screen[3]) if screen else (1470, 956)
    for fy in (0.5, 0.35, 0.65, 0.8):
        for fx in (0.5, 0.3, 0.7, 0.15, 0.85):
            x, y = width * fx, height * fy
            if not any(f[0] <= x <= f[0] + f[2] and f[1] <= y <= f[1] + f[3]
                       for f in (w["f"] for w in wins) if f[1] >= 30 or f[3] > 40):
                return x, y
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apps", default="all")
    parser.add_argument("--shell", action="store_true")
    parser.add_argument("--settle", type=float, default=0.6)
    parser.add_argument("--no-submenus", action="store_true")
    parser.add_argument("--summaries-only", action="store_true",
                        help="rewrite tests/robot/mac/*.json from the local captures, no GUI")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("mac_walk.py runs on the reference Mac")
    if args.summaries_only:
        mac = Mac(args)
        everything = []
        for folder in sorted(cm.CAPTURES.iterdir()):
            if (folder / "states.jsonl").exists() and folder.name not in {"lulo", "diff", "Shell"}:
                everything += list(cm.load_states(folder / "states.jsonl").values())
        mac.vocab = cm.menu_vocabulary(everything)
        for folder in sorted(cm.CAPTURES.iterdir()):
            if (folder / "states.jsonl").exists() and folder.name not in {"lulo", "diff"}:
                mac.app, mac.out = folder.name, folder
                mac.records = list(cm.load_states(folder / "states.jsonl").values())
                mac.write_app()
        return 0
    build_macax()
    lock = open(LOCK, "w")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    sandbox()
    mac = Mac(args)
    names = list(APPS) if args.apps == "all" else [a for a in args.apps.split(",") if a]
    pointer = mac.ax("pointer")
    try:
        mac.guard()
        for name in names:
            try:
                mac.walk_app(name, APPS[name])
            except Stuck as error:
                print(f"  ! {name} stopped: {error}", flush=True)
        if args.shell:
            walk_shell(mac)
    except HumanInput as error:
        print(f"STOPPED: {error}", flush=True)
        return 2
    finally:
        if pointer.get("x") is not None:
            subprocess.run([str(MACAX), "move", str(pointer["x"]), str(pointer["y"])], capture_output=True)
            mac.mark()
        lock.close()
    total = subprocess.run(["du", "-sh", str(cm.CAPTURES)], capture_output=True, text=True).stdout.split()[0]
    print(f"captures: {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
