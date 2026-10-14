#!/usr/bin/env python3
"""Walk the Mac robot's states on Lulo, in a private nested session (Lulo side).

    python3 scripts/robot/mac/lulo_walk.py --bin-dir DIR --out DIR [--apps Finder,Notes | --apps all]

Runs on the reference laptop or CI, never in the live session: it reuses
scripts/behavior/run_lulo.py's isolation (dbus-run-session, a private
headless Sway, temporary HOME and XDG dirs, AT-SPI on the private bus, input
only into that Sway). The state list comes from tests/robot/mac/<app>.json,
the Mac walker's summaries, so both sides use the same (app, path) keys.

For each app it launches the Lulo clone floating at its own default size and
records, per state, the same summary the Mac side does (AT-SPI tree, window
size, a JPEG screenshot):

  window/main, menu/…    from the app's own org.rmac.AppMenu2 layout (Lulo's
                         menus draw in the top bar, so they have no shot here)
  item/…, settings       by activating that menu item through AppMenu2
  settings/<tab>, tab/<label>, toolbar/<label>, pane/<label>
                         by clicking the control with that label
  context/<surface>      by right-clicking the matching surface

Everything it opens is dismissed with Escape, Cancel or by closing the new
window. Output: DIR/<Mac app>/states.jsonl and the JPEGs beside it.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import traceback
from pathlib import Path
from typing import Any, Callable, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent.parent / "behavior"))
sys.path.insert(0, str(HERE.parent.parent / "inventory"))

import common as cm  # noqa: E402
import run_lulo as rl  # noqa: E402

RL_KEYS = {
    "Finder": "files", "TextEdit": "text-editor", "Preview": "preview", "Notes": "notes", "Terminal": "terminal",
    "Calculator": "calculator", "Clock": "clock", "Weather": "weather", "System Settings": "settings",
    "Activity Monitor": "system-monitor", "Mail": "mail", "Calendar": "calendar",
    "Archive Utility": "archive-utility", "QuickTime Player": "player",
}
rl.APP_BINARIES.setdefault("archive-utility", ["rmac-archive-utility"])
rl.APP_BINARIES.setdefault("player", ["rmac-player"])
MENU_PATH, MENU_IFACE = "/org/rmac/AppMenu1", "org.rmac.AppMenu2"
# Mac pane name -> Lulo Settings `--pane` id (crates/system-settings/src/navigation.rs PANE_ROUTES).
PANE_IDS = {cm.norm_label(k): v for k, v in {
    "Wi-Fi": "wifi", "Wi‑Fi": "wifi", "Bluetooth": "bluetooth", "Network": "network",
    "Internet Accounts": "internet-accounts", "VPN": "vpn", "Battery": "battery", "General": "general",
    "Date & Time": "date-time", "Language & Region": "language-region", "Login Items & Extensions": "login-items",
    "Login Items": "login-items", "Sharing": "sharing", "Accessibility": "accessibility", "Appearance": "appearance",
    "Desktop & Dock": "desktop-dock", "Displays": "displays", "Menu Bar": "menu-bar", "Spotlight": "spotlight",
    "Wallpaper": "wallpaper", "Notifications": "notifications", "Sound": "sound", "Keyboard": "keyboard",
    "Mouse": "mouse", "Trackpad": "trackpad", "Focus": "focus", "Lock Screen": "lock-screen",
    "Privacy & Security": "privacy-security", "Touch ID & Password": "login-password",
    "Users & Groups": "users-groups", "Printers & Scanners": "printers-scanners",
}.items()}
PANE_TITLES = {
    "wifi": "Wi-Fi", "internet-accounts": "Internet Accounts", "date-time": "Date & Time",
    "language-region": "Language & Region", "login-items": "Login Items", "desktop-dock": "Desktop & Dock",
    "menu-bar": "Menu Bar", "lock-screen": "Lock Screen", "privacy-security": "Privacy & Security",
    "login-password": "Login Password", "users-groups": "Users & Groups", "printers-scanners": "Printers & Scanners",
    "vpn": "VPN",
}
ENABLED, SEPARATOR_BEFORE, CHECKED, SUBMENU, ALTERNATE = 1, 2, 4, 16, 32
CONTAINERS = {"list", "list box", "table", "tree", "tree table"}
POPUP_ROLES = {"dialog", "alert", "popup menu", "menu", "file chooser", "window"}


def parse_layout(data: Any) -> list[dict[str, Any]]:
    """AppMenu2.Layout as nested rows: {label, action, key, enabled,
    checked, alternate, separator_before, children}."""

    payload = data["data"] if isinstance(data, dict) else data
    menus = []
    for label, items in payload[1]:
        root: list[dict[str, Any]] = []
        stack: list[list[dict[str, Any]]] = [root]
        for item_label, action, shortcut, bits, depth in items:
            del stack[depth + 1:]
            row = {"label": item_label, "action": action, "key": shortcut, "enabled": bool(bits & ENABLED),
                   "checked": bool(bits & CHECKED), "alternate": bool(bits & ALTERNATE),
                   "separator_before": bool(bits & SEPARATOR_BEFORE), "children": []}
            stack[-1].append(row)
            if bits & SUBMENU:
                stack.append(row["children"])
        menus.append({"label": label, "items": root})
    return menus


def as_menu_bar(app: str, menus: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """What the top bar shows: the app's menus with the Application, Window
    and Help menus synthesized as the menu bar does (lulo_inventory.py
    mirrors shell/bins/rmac-menubar)."""

    import lulo_inventory as li
    import rust_menu_parser as rmp

    live: dict[int, dict[str, Any]] = {}

    def to_rmp(row: dict[str, Any]) -> "rmp.MenuItem":
        item = rmp.MenuItem(label=row["label"], action=row["action"], shortcut=row["key"],
                            separator_before=row["separator_before"], children=[to_rmp(c) for c in row["children"]])
        live[id(item)] = row
        return item

    def from_rmp(item: "rmp.MenuItem") -> dict[str, Any]:
        row = live.get(id(item), {})
        return {"label": item.label, "action": item.action, "key": item.shortcut,
                "enabled": row.get("enabled", True), "checked": row.get("checked", False),
                "alternate": row.get("alternate", False), "separator_before": item.separator_before,
                "children": [from_rmp(c) for c in item.children]}

    bar = [rmp.Menu(m["label"], [to_rmp(r) for r in m["items"]]) for m in menus]
    name = {"Activity Monitor": "System Monitor"}.get(app, app)
    application = li._take_menu(bar, li.APPLICATION_MENU)
    window = li._take_menu(bar, li.WINDOW_MENU)
    helps = li._take_menu(bar, li.HELP_MENU)
    bar.insert(0, li._synthesize_app_menu(name, application))
    bar.append(li._synthesize_window_menu(name, window))
    bar.append(li._synthesize_help_menu(name, helps))
    return [{"label": m.label, "items": [from_rmp(i) for i in m.items]} for m in bar]


def flatten(rows: list[dict[str, Any]], trail: list[str]) -> list[dict[str, Any]]:
    out = []
    for row in rows:
        out.append({**row, "trail": trail, "submenu": bool(row["children"])})
        out += flatten(row["children"], trail + [row["label"]])
    return out


class Walker:
    def __init__(self, nested: rl.Nested, bins: list[Path], out: Path, settle: float) -> None:
        self.nested = nested
        self.bins = bins
        self.out = out
        self.settle = settle
        self.app = ""
        self.records: list[dict[str, Any]] = []
        self.run: Optional[rl.LuloRun] = None
        self.layout: list[dict[str, Any]] = []

    # ------------------------------------------------------------ helpers

    def windows(self) -> list[dict[str, Any]]:
        return self.nested.windows()

    def rect_of(self, frame) -> Optional[tuple[int, int, int, int]]:
        title = rl.name(frame)
        wins = self.windows()
        mine = [w for w in wins if w.get("name") == title] or [w for w in wins if w.get("focused")]
        if not mine:
            return None
        rect, inner = mine[0]["rect"], mine[0].get("window_rect") or {}
        x = rect["x"] + inner.get("x", 0)
        y = rect["y"] + inner.get("y", 0)
        return x, y, inner.get("width", rect["width"]), inner.get("height", rect["height"])

    def shoot(self, path: str, rect: Optional[tuple[int, int, int, int]]) -> Optional[str]:
        if rect is None:
            return None
        from PIL import Image

        safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in path)[:120]
        target = self.out / self.app / f"{safe}.jpg"
        raw = target.with_suffix(".png")
        x, y, w, h = rect
        if w <= 0 or h <= 0:
            return None
        result = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", str(raw)], env=self.nested.env,
                                capture_output=True, timeout=20)
        if result.returncode or not raw.exists():
            return None
        Image.open(raw).convert("RGB").save(target, "JPEG", quality=80)
        raw.unlink()
        return f"{self.app}/{target.name}"

    def summarize(self, root) -> list[dict[str, Any]]:
        pyatspi = rl.atspi()
        out = []
        stack = [(root, 0)]
        count = 0
        while stack and count < 3000:
            node, depth = stack.pop()
            count += 1
            r = rl.role(node)
            showing = rl.has_state(node, pyatspi.STATE_SHOWING) or depth == 0
            if not showing:
                continue
            common = cm.ATSPI_ROLES.get(r)
            label = rl.name(node)
            if common in {"text", "textfield", "textarea"} and not label:
                value, _s, _e = rl.text_of(node)
                if value and common == "text":
                    label = value[:80]
            if common in cm.CONTROL_ROLES and (label or common not in {"text", "row"}):
                box = rl.extents(node)
                enabled = rl.has_state(node, pyatspi.STATE_ENABLED) or rl.has_state(node, pyatspi.STATE_SENSITIVE)
                out.append({"role": common, "label": label or "", "enabled": enabled,
                            "f": list(box) if box else None, "atspi": r})
            try:
                kids = [node.getChildAtIndex(i) for i in range(node.childCount)]
            except Exception:  # noqa: BLE001
                kids = []
            if r in CONTAINERS:
                kids = kids[:8]
            for kid in reversed(kids):
                stack.append((kid, depth + 1))
        return out

    def record(self, state: dict[str, Any]) -> None:
        state["app"] = self.app
        self.records.append(state)
        print(f"  {state['path']}  [{state['kind']}] "
              f"{len(state.get('items') or state.get('controls') or [])} rows", flush=True)

    def capture_frame(self, path: str, kind: str, frame, extra: Optional[dict] = None) -> None:
        rect = self.rect_of(frame)
        state = {"path": path, "kind": kind, "title": rl.name(frame),
                 "window": [rect[2], rect[3]] if rect else None, "controls": self.summarize(frame),
                 "shot": self.shoot(path, rect)}
        state.update(extra or {})
        self.record(state)

    def all_frames(self) -> list:
        assert self.run is not None
        return self.run.frames() + self.run.helper_frames()

    def popup_nodes(self) -> list:
        pyatspi = rl.atspi()
        found = []
        for frame in self.all_frames():
            for node in rl.descendants(frame, limit=3000):
                if node is not frame and rl.role(node) in POPUP_ROLES and rl.has_state(node, pyatspi.STATE_SHOWING):
                    found.append(node)
        return found

    def menu_items(self, node) -> list[dict[str, Any]]:
        pyatspi = rl.atspi()
        items = []
        for index in range(node.childCount):
            item = node.getChildAtIndex(index)
            r = rl.role(item)
            if r == "separator":
                items.append({"label": "-"})
            elif r.endswith("menu item") or r == "menu":
                items.append({"label": rl.name(item),
                              "enabled": rl.has_state(item, pyatspi.STATE_ENABLED) or rl.has_state(item, pyatspi.STATE_SENSITIVE),
                              "checked": rl.has_state(item, pyatspi.STATE_CHECKED), "submenu": r == "menu", "key": ""})
        return items

    CLICKABLE = {"list item", "table row", "tree item", "table cell", "label", "static", "push button", "button",
                 "combo box", "menu item", "radio button", "check box", "page tab", "toggle button", "link"}

    def click_label(self, label: str, button: str = "left") -> None:
        """Click the control whose label matches `label` the way diff.py
        compares labels (case, quotes, hyphens, app name, spelling)."""

        assert self.run is not None
        want = cm.norm_label(label, self.app)
        frame = self.run.active_frame()
        frames = ([frame] if frame is not None else []) + [f for f in self.run.frames() if f is not frame]
        for candidate in frames:
            for node in rl.descendants(candidate, limit=4000):
                if rl.role(node) in self.CLICKABLE and cm.norm_label(rl.name(node), self.app) == want:
                    box = rl.extents(node)
                    if not box or box[2] <= 0:
                        continue
                    ox, oy = self.run.window_origin(candidate)
                    self.nested.input.click(ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2,
                                            rl.OUTPUT_W, rl.OUTPUT_H, button=button)
                    return
        raise rl.StepFailed(f"no accessible item named {label!r} to click")

    # ------------------------------------------------------- probe/dismiss

    def snapshot(self) -> dict[str, Any]:
        return {"wins": {w["id"] for w in self.windows()}, "popups": len(self.popup_nodes())}

    def probe(self, path: str, act: Callable[[], Any], kind_hint: str = "window") -> None:
        assert self.run is not None
        before = self.snapshot()
        act()
        time.sleep(self.settle + 0.8)
        rl.pump()
        new = [w for w in self.windows() if w["id"] not in before["wins"]]
        popups = self.popup_nodes()
        if new:
            title = new[0].get("name")
            frame = next((f for f in self.all_frames() if rl.name(f) == title), None)
            if frame is not None:
                self.capture_frame(path, "dialog" if rl.role(frame) in {"dialog", "alert", "file chooser"} else kind_hint,
                                   frame)
            else:
                rect = new[0]["rect"]
                self.record({"path": path, "kind": kind_hint, "title": title, "window": [rect["width"], rect["height"]],
                             "controls": [], "shot": self.shoot(path, (rect["x"], rect["y"], rect["width"], rect["height"]))})
        elif len(popups) > before["popups"]:
            node = popups[-1]
            frame = self.run.active_frame() or (self.run.frames() or [None])[0]
            rect = self.rect_of(frame) if frame is not None else None
            r = rl.role(node)
            state: dict[str, Any] = {"path": path, "kind": "sheet" if r in {"dialog", "alert"} else kind_hint,
                                     "title": rl.name(node), "shot": self.shoot(path, rect)}
            box = rl.extents(node)
            state["window"] = [box[2], box[3]] if box else None
            if r in {"menu", "popup menu"}:
                state["items"] = self.menu_items(node)
                state["kind"] = "context" if kind_hint == "context" else "menu"
            else:
                state["controls"] = self.summarize(node)
            self.record(state)
        else:
            frame = self.run.active_frame() or (self.run.frames() or [None])[0]
            if frame is not None:
                self.capture_frame(path, "in-window", frame, {"note": "no new window, sheet or popup"})
        self.dismiss(before)

    def dismiss(self, before: dict[str, Any]) -> None:
        assert self.run is not None
        for attempt in range(4):
            rl.pump()
            new = [w for w in self.windows() if w["id"] not in before["wins"]]
            popups = len(self.popup_nodes()) > before["popups"]
            if not new and not popups:
                return
            if attempt == 0:
                self.nested.input.key("escape")
            elif attempt == 1:
                try:
                    self.run.click_item("Cancel", "left")
                except rl.StepFailed:
                    self.nested.input.key("escape")
            else:
                for window in new:
                    self.nested.swaymsg(f"[con_id={window['id']}]", "kill")
                if popups and not new:
                    self.nested.input.key("escape")
            time.sleep(0.6)

    # ---------------------------------------------------------------- menus

    def read_layout(self) -> None:
        assert self.run is not None
        bus = cm.LULO_APPS[self.app]["bus"]
        for _ in range(20):
            result = subprocess.run(["busctl", "--user", "--json=short", "call", bus, MENU_PATH, MENU_IFACE, "Layout"],
                                    env=self.run.env, capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                self.layout = as_menu_bar(self.app, parse_layout(json.loads(result.stdout)))
                return
            time.sleep(0.5)
        self.layout = []
        print(f"  ! no AppMenu2 layout on {bus}", flush=True)

    def menu_states(self) -> None:
        for index, menu in enumerate(self.layout):
            key = "menu/" + cm.menu_key(index, menu["label"])

            def emit(path: str, rows: list[dict[str, Any]]) -> None:
                items = []
                for row in rows:
                    if row.get("alternate"):
                        continue  # the Mac hides Option-key alternates too
                    if row["separator_before"] and items:
                        items.append({"label": "-"})
                    items.append({"label": row["label"], "enabled": row["enabled"], "key": row["key"],
                                  "checked": row["checked"], "submenu": bool(row["children"])})
                self.record({"path": path, "kind": "menu", "window": None, "shot": None, "items": items})
                for row in rows:
                    if row["children"] and not row.get("alternate"):
                        emit(f"{path}/{row['label']}", row["children"])

            emit(key, menu["items"])

    def find_item(self, mac_path: str) -> Optional[dict[str, Any]]:
        parts = mac_path.split("/")[1:]
        if not parts:
            return None
        menu_title, *trail, label = parts
        for index, menu in enumerate(self.layout):
            if cm.norm_label(cm.menu_key(index, menu["label"]), self.app) != cm.norm_label(menu_title, self.app):
                continue
            for item in flatten(menu["items"], []):
                if (cm.norm_label(item["label"], self.app) == cm.norm_label(label, self.app)
                        and [cm.norm_label(t, self.app) for t in item["trail"]] == [cm.norm_label(t, self.app) for t in trail]):
                    return item
        return None

    def activate(self, action: str) -> None:
        assert self.run is not None
        bus = cm.LULO_APPS[self.app]["bus"]
        subprocess.run(["busctl", "--user", "call", bus, MENU_PATH, MENU_IFACE, "Activate", "s", action],
                       env=self.run.env, capture_output=True, timeout=10)

    # ------------------------------------------------------------- app walk

    def main_frame(self):
        assert self.run is not None
        rl.pump()
        return self.run.active_frame() or (self.run.frames() or [None])[0]

    def walk(self, mac_app: str, mac_paths: list[str]) -> None:
        self.app = mac_app
        self.records = []
        (self.out / mac_app).mkdir(parents=True, exist_ok=True)
        key = RL_KEYS[mac_app]
        print(f"== {mac_app} (Lulo {cm.LULO_APPS[mac_app]['lulo']})", flush=True)
        launch = {"files": {"folder": "."}, "text-editor": {"file": "notes.txt"},
                  "preview": {"file": "picture.png"}}.get(key, {})
        scenario = {"app": key, "launch": launch,
                    "setup": {"files": {"notes.txt": "Lulo robot sample text.\nSecond line.\n", "Folder A/": ""}}}
        self.run = rl.LuloRun(self.nested, f"macrobot/{key}", scenario, self.bins, self.settle)
        try:
            self.run.setup()
            from PIL import Image

            Image.new("RGB", (512, 512), (90, 140, 220)).save(self.run.sandbox / "picture.png")
            if key in {"archive-utility", "player"}:
                self.launch_loose()
            else:
                self.run.launch()
            time.sleep(1.5)
            frame = self.main_frame() or next(iter(self.run.helper_frames()), None)
            if frame is not None:
                self.capture_frame("window/main", "window", frame)
            if RL_KEYS[self.app] == "player":
                self.nested.input.key("escape")  # as on the Mac: cancel the Open panel first
                time.sleep(1.0)
            self.read_layout()
            self.menu_states()
            for path in mac_paths:
                if os.environ.get("MACROBOT_DEBUG"):
                    print(f"  > {path}", flush=True)
                try:
                    self.walk_state(path)
                    if self.run.process.poll() is not None:
                        log = Path(self.run.log.name).read_text(errors="replace")[-600:] if self.run.log else ""
                        print(f"  ! {path}: the app exited ({self.run.process.returncode})\n{log}", flush=True)
                        first_line = next((line for line in log.splitlines() if "panicked" in line or "cannot" in line), "")
                        self.record({"path": path, "kind": "crash", "window": None, "shot": None,
                                     "note": f"app exited {self.run.process.returncode}: {first_line.strip()}"})
                        # Start it again and carry on with the next state.
                        self.run.stop()
                        if RL_KEYS[self.app] in {"archive-utility", "player"}:
                            self.launch_loose()
                        else:
                            self.run.launch()
                        time.sleep(1.5)
                        self.read_layout()
                except BaseException as error:  # noqa: BLE001
                    if not isinstance(error, Exception):
                        print(f"  ! {path}: {type(error).__name__} {error}", flush=True)
                        raise
                    print(f"  ! {path}: {error}", flush=True)
                    if self.run.process.poll() is not None:
                        self.record({"path": path, "kind": "crash", "window": None, "shot": None,
                                     "note": f"app exited {self.run.process.returncode}"})
                        break
        except Exception as error:  # noqa: BLE001
            print(f"  ! {mac_app} failed: {error}", flush=True)
            traceback.print_exc()
        finally:
            self.run.stop()
            self.run.restore_shared_state()
            with open(self.out / mac_app / "states.jsonl", "w") as handle:
                for state in self.records:
                    handle.write(json.dumps(state, ensure_ascii=False) + "\n")

    def open_pane(self, pane: str) -> None:
        """`rmac-system-settings --pane ID` hands the running instance the
        pane; wait until its window title says it got there."""

        assert self.run is not None
        want = PANE_TITLES.get(pane, pane).casefold()
        for _attempt in range(3):
            subprocess.run([str(self.run.binary()), "--pane", pane], env=self.run.env,
                           capture_output=True, timeout=20)
            deadline = time.monotonic() + 6
            while time.monotonic() < deadline:
                rl.pump()
                frame = self.main_frame()
                if frame is not None and rl.name(frame).casefold().startswith(want):
                    time.sleep(self.settle + 0.6)
                    return
                time.sleep(0.3)
        raise rl.StepFailed(f"Settings did not open pane {pane}")

    def launch_loose(self) -> None:
        """Start an app that shows no window of its own at launch (Archive
        Utility) or opens straight on the Open panel (Player), and wait for
        its menu instead of a window."""

        assert self.run is not None
        run = self.run
        run.log = open(self.nested.logs / f"macrobot-{RL_KEYS[self.app]}.log", "w")
        run.process = subprocess.Popen([str(run.binary())], env=run.env, stdout=run.log,
                                       stderr=subprocess.STDOUT, close_fds=True, cwd=str(run.sandbox))
        bus = cm.LULO_APPS[self.app]["bus"]
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if run.process.poll() is not None:
                raise rl.StepFailed(f"{self.app} exited with {run.process.returncode}")
            probe = subprocess.run(["busctl", "--user", "status", bus], env=run.env, capture_output=True, timeout=5)
            if probe.returncode == 0:
                break
            time.sleep(0.3)
        time.sleep(2.0)

    def walk_state(self, path: str) -> None:
        assert self.run is not None
        head = path.split("/")[0]
        label = path.split("/")[-1]
        if head == "item":
            item = self.find_item(path)
            if item and item["enabled"] and item["action"]:
                self.probe(path, lambda: self.activate(item["action"]))
        elif path == "settings":
            item = next((i for m in self.layout[:1] for i in flatten(m["items"], [])
                         if i["label"].startswith(("Settings", "Preferences"))), None)
            if item is None:
                return
            before = self.snapshot()
            self.activate(item["action"])
            time.sleep(self.settle + 1.2)
            new = [w for w in self.windows() if w["id"] not in before["wins"]]
            frame = None
            if new:
                frame = next((f for f in self.run.frames() if rl.name(f) == new[0].get("name")), None)
            frame = frame or self.main_frame()
            if frame is None:
                return
            self.capture_frame("settings", "settings", frame)
            for tab in self.settings_tabs:
                try:
                    self.click_label(tab)
                except rl.StepFailed:
                    continue
                time.sleep(self.settle + 0.4)
                self.capture_frame(f"settings/{tab}", "settings", self.main_frame() or frame)
            for window in new:
                self.nested.swaymsg(f"[con_id={window['id']}]", "kill")
            time.sleep(0.6)
        elif head in {"tab", "pane"}:
            parts = path.split("/")[1:]
            try:
                if head == "pane":
                    # Sidebar rows below the fold are not on screen to click:
                    # open the pane the way a deep link does (`--pane`).
                    pane = PANE_IDS.get(cm.norm_label(parts[0]))
                    if pane is None:
                        return
                    self.open_pane(pane)
                    parts = parts[1:]
                for part in parts:
                    sub = PANE_IDS.get(cm.norm_label(part))
                    if sub is not None:
                        self.open_pane(sub)
                    else:
                        self.click_label(part)
                    time.sleep(self.settle + 0.6)
            except rl.StepFailed:
                return
            frame = self.main_frame()
            if frame is not None:
                self.capture_frame(path, head, frame)
        elif head == "toolbar":
            try:
                self.probe(path, lambda: self.click_label(label), "popover")
            except rl.StepFailed:
                return
        elif head == "context":
            self.probe(path, lambda: self.context(label), "context")

    settings_tabs: list[str] = []

    def context(self, surface: str) -> None:
        assert self.run is not None
        if surface == "background":
            self.run.context_background()
            return
        if surface == "item":
            self.run.click_item("notes.txt", "right")
            return
        frame = self.main_frame()
        rect = self.rect_of(frame)
        if rect is None:
            raise rl.StepFailed("no window")
        x, y, w, h = rect
        if surface in {"note-list", "process"}:
            row = next((n for n in rl.descendants(frame, limit=4000)
                        if rl.role(n) in {"list item", "table row", "tree item"} and rl.extents(n)), None)
            if row is not None:
                box = rl.extents(row)
                self.nested.input.click(x + box[0] + box[2] // 2, y + box[1] + box[3] // 2,
                                        rl.OUTPUT_W, rl.OUTPUT_H, button="right")
                return
        self.nested.input.click(x + w // 2, y + h // 2, rl.OUTPUT_W, rl.OUTPUT_H, button="right")


def mac_paths(app: str, states_dir: Path) -> tuple[list[str], list[str]]:
    data = json.loads((states_dir / f"{app}.json").read_text())
    paths = [s["path"] for s in data["states"] if "#" not in s["path"]]
    tabs = [p.split("/", 1)[1] for p in paths if p.startswith("settings/")]
    order = {"tab": 0, "pane": 1, "toolbar": 2, "context": 3, "settings": 4, "item": 5}
    walk = [p for p in paths if p.split("/")[0] in order and not p.startswith("settings/")]
    walk.sort(key=lambda p: order[p.split("/")[0]])
    return walk, tabs


def inner(args: argparse.Namespace) -> int:
    work = Path(args.inner)
    nested = rl.Nested(work)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    try:
        # Every window floats at the size the app asks for, as on the desktop.
        nested.swaymsg("for_window", '[app_id=".*"]', "floating", "enable")
        bins = [Path(p) for p in args.bin_dir]
        walker = Walker(nested, bins, out, args.settle)
        states_dir = Path(args.states)
        apps = list(RL_KEYS) if args.apps == "all" else args.apps.split(",")
        for app in apps:
            if not (states_dir / f"{app}.json").exists():
                print(f"== {app}: no Mac states, skipped", flush=True)
                continue
            paths, tabs = mac_paths(app, states_dir)
            walker.settings_tabs = tabs
            walker.walk(app, paths)
    finally:
        nested.close()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", action="append", default=[])
    parser.add_argument("--out", required=True)
    parser.add_argument("--apps", default="all")
    parser.add_argument("--states", default=str(cm.SUMMARIES))
    parser.add_argument("--settle", type=float, default=0.8)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        parser.error("lulo_walk.py runs on Linux (the reference laptop or CI)")
    if args.inner:
        return inner(args)
    args.bin_dir = [str(Path(p).resolve()) for p in args.bin_dir]
    args.out = str(Path(args.out).resolve())
    args.states = str(Path(args.states).resolve())
    args.shell_bin_dir, args.output, args.fake_hardware = [], None, True
    argv = ["--out", args.out, "--apps", args.apps, "--states", args.states, "--settle", str(args.settle)]
    for directory in args.bin_dir:
        argv += ["--bin-dir", directory]
    # run_lulo.outer builds the private session and re-runs its own file
    # with --inner; point it at this file instead.
    rl.__file__ = str(Path(__file__).resolve())
    return rl.outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
