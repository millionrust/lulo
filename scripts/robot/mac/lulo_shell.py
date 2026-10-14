#!/usr/bin/env python3
"""Walk the Mac robot's shell states on Lulo's shell, in a private nested niri.

    python3 scripts/robot/mac/lulo_shell.py --bin-dir DIR --out DIR [--niri PATH]

--bin-dir holds the CI `lulo-binaries` (rmac-top-bar, rmac-dock, …); the
nested session (scripts/behavior/run_menu_dismiss.py's: headless Sway, niri
with the shipped shell.kdl, a private D-Bus and AT-SPI, fake hardware) wants
the cargo names `top-bar`, `dock`, `wallpaper` and `mission-control`, so a
sibling directory of links is made for it.

Paths come from tests/robot/mac/Shell.json: menu/Apple (the Lulo menu),
shell/extra/<status item>, shell/spotlight, shell/dock-menu/<app> and
shell/desktop-context. ⌘Tab is not walked: niri runs its key binds only for
physical keyboards. Output: DIR/Shell/states.jsonl and JPEGs.
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent.parent / "behavior"))

import common as cm  # noqa: E402
import run_menu_dismiss as md  # noqa: E402

CARGO_NAMES = {"top-bar": "rmac-top-bar", "dock": "rmac-dock", "wallpaper": "rmac-wallpaper",
               "mission-control": "rmac-mission-control"}
DOCK_NAMES = {"Finder": "Files", "System Settings": "Settings", "Trash": "Bin"}
EXTRA_ALIASES = {"Clock": ["clock", "date", "notification"], "Control Centre": ["control centre", "control center"],
                 "Wi‑Fi": ["wi-fi", "wifi"], "Wi-Fi": ["wi-fi", "wifi"], "Battery": ["battery"],
                 "Spotlight": ["spotlight", "search"], "Sound": ["sound", "volume"]}


class ShellWalk(md.Run):
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        super().__init__(args, work)
        self.dest = Path(args.out) / "Shell"
        self.dest.mkdir(parents=True, exist_ok=True)
        self.records: list[dict[str, Any]] = []

    # ------------------------------------------------------------ AT-SPI

    def nodes(self):
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        for index in range(desktop.childCount):
            app = desktop.getChildAtIndex(index)
            if app is None:
                continue
            stack = [(app, 0)]
            while stack:
                node, depth = stack.pop()
                try:
                    yield app, node, depth
                    if depth < 30:
                        stack.extend((node.getChildAtIndex(i), depth + 1) for i in range(node.childCount))
                except Exception:  # noqa: BLE001
                    continue

    def showing(self, node) -> bool:
        import pyatspi

        try:
            return node.getState().contains(pyatspi.STATE_SHOWING)
        except Exception:  # noqa: BLE001
            return False

    def top_frames(self) -> dict[tuple[str, str], tuple]:
        out = {}
        for app, node, depth in self.nodes():
            if depth == 1 and self.showing(node):
                box = self.extents(node)
                if box and box[2] > 4 and box[3] > 4:
                    out[(app.name or "", node.name or "", box)] = node
        return out

    def open_menu(self):
        for _app, node, _depth in self.nodes():
            try:
                if node.getRoleName() in {"menu", "popup menu"} and self.showing(node) and node.childCount:
                    if any(node.getChildAtIndex(i).getRoleName().endswith("menu item") for i in range(node.childCount)):
                        return node
            except Exception:  # noqa: BLE001
                continue
        return None

    def items(self, menu) -> list[dict[str, Any]]:
        import pyatspi

        out = []
        for index in range(menu.childCount):
            item = menu.getChildAtIndex(index)
            role = item.getRoleName()
            if role == "separator":
                out.append({"label": "-"})
            elif role.endswith("menu item") or role == "menu":
                state = item.getState()
                out.append({"label": item.name or "", "key": "", "checked": state.contains(pyatspi.STATE_CHECKED),
                            "enabled": state.contains(pyatspi.STATE_ENABLED) or state.contains(pyatspi.STATE_SENSITIVE),
                            "submenu": role == "menu"})
        return out

    def controls(self, root) -> list[dict[str, Any]]:
        import pyatspi

        out = []
        origin = self.extents(root) or (0, 0, 0, 0)
        stack = [root]
        while stack and len(out) < 1500:
            node = stack.pop()
            try:
                role = cm.ATSPI_ROLES.get(node.getRoleName())
                if role in cm.CONTROL_ROLES and node.name and self.showing(node):
                    box = self.extents(node)
                    state = node.getState()
                    out.append({"role": role, "label": node.name,
                                "enabled": state.contains(pyatspi.STATE_ENABLED) or state.contains(pyatspi.STATE_SENSITIVE),
                                "f": [box[0] - origin[0], box[1] - origin[1], box[2], box[3]] if box else None})
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:  # noqa: BLE001
                continue
        return out

    # ------------------------------------------------------------ capture

    def shot(self, path: str, box: Optional[tuple]) -> Optional[str]:
        if not box:
            return None
        from PIL import Image

        safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in path)[:120]
        target = self.dest / f"{safe}.jpg"
        raw = target.with_suffix(".png")
        x, y, w, h = (int(v) for v in box)
        x, y = max(0, x), max(0, y)
        w, h = min(w, md.OUTPUT_W - x), min(h, md.OUTPUT_H - y)
        if w <= 0 or h <= 0:
            return None
        result = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", str(raw)], env=self.env, capture_output=True)
        if result.returncode or not raw.exists():
            return None
        Image.open(raw).convert("RGB").save(target, "JPEG", quality=80)
        raw.unlink()
        return f"Shell/{target.name}"

    def record(self, state: dict[str, Any]) -> None:
        state["app"] = "Shell"
        self.records.append(state)
        print(f"  {state['path']}  [{state['kind']}] "
              f"{len(state.get('items') or state.get('controls') or [])} rows", flush=True)

    def probe(self, path: str, act, kind: str = "popover") -> None:
        before = self.top_frames()
        layers_before = self.layer_names()
        act()
        time.sleep(1.5)
        menu = self.open_menu()
        if menu is not None:
            box = self.extents(menu)
            self.record({"path": path, "kind": "menu" if kind != "context" else "context",
                         "window": [box[2], box[3]] if box else None, "items": self.items(menu),
                         "shot": self.shot(path, box)})
        else:
            after = self.top_frames()
            new = [(k, v) for k, v in after.items() if k not in before]
            if new:
                (app, name, box), node = max(new, key=lambda kv: kv[0][2][2] * kv[0][2][3])
                self.record({"path": path, "kind": kind, "title": name, "window": [box[2], box[3]],
                             "controls": self.controls(node), "shot": self.shot(path, box), "owner": app})
            else:
                # A layer-shell popover (Control Centre, Spotlight) keeps its
                # accessible frame while hidden: find it by its new layer.
                fresh = [n for n in self.layer_names() - layers_before if not n.endswith("click-catcher")]
                node = None
                for namespace in fresh:
                    short = namespace.replace("rmac-", "")
                    for (app, name, box), frame in self.top_frames().items():
                        if short in app or namespace in app:
                            if node is None or box[2] * box[3] > node[1][2] * node[1][3]:
                                node = (frame, box, name, app)
                if node is not None:
                    frame, box, name, app = node
                    self.record({"path": path, "kind": kind, "title": name, "window": [box[2], box[3]],
                                 "controls": self.controls(frame), "shot": self.shot(path, box), "owner": app,
                                 "layer": fresh})
                elif fresh:
                    self.record({"path": path, "kind": kind, "window": None, "controls": [], "shot": None,
                                 "layer": fresh, "note": "layer opened; no accessible frame"})
                else:
                    print(f"  ! {path}: nothing opened", flush=True)
        self.close_everything()
        time.sleep(0.5)

    def layer_names(self) -> set[str]:
        names: set[str] = set()

        def visit(value) -> None:
            if isinstance(value, dict):
                if isinstance(value.get("namespace"), str):
                    names.add(value["namespace"])
                for child in value.values():
                    visit(child)
            elif isinstance(value, list):
                for child in value:
                    visit(child)

        visit(self.niri("layers"))
        return names

    # --------------------------------------------------------------- walk

    def top_bar_buttons(self) -> dict[str, Any]:
        out = {}
        for app, node, _depth in self.nodes():
            try:
                if "bar" in (app.name or "") and node.getRoleName() in {"push button", "button", "toggle button"}:
                    box = self.extents(node)
                    if node.name and box and box[1] < 40:
                        out[node.name] = node
            except Exception:  # noqa: BLE001
                continue
        return out

    def walk(self, paths: list[str]) -> None:
        buttons = self.top_bar_buttons()
        print(f"top bar buttons: {sorted(buttons)}", flush=True)
        for path in paths:
            try:
                self.walk_one(path, buttons)
            except Exception as error:  # noqa: BLE001
                print(f"  ! {path}: {error}", flush=True)
                self.close_everything()

    def walk_one(self, path: str, buttons: dict[str, Any]) -> None:
        if path == "menu/Apple":
            self.probe(path, self.open_system_menu, "menu")
        elif path.startswith("shell/extra/"):
            label = path.split("/", 2)[2]
            wanted = EXTRA_ALIASES.get(label, [cm.norm_label(label)])
            name = next((n for n in buttons if any(w in cm.norm_label(n) for w in wanted)), None)
            if name is None:
                close = difflib.get_close_matches(cm.norm_label(label), [cm.norm_label(n) for n in buttons], 1, 0.6)
                name = next((n for n in buttons if close and cm.norm_label(n) == close[0]), None)
            if name is None:
                print(f"  ! {path}: no top-bar item like {label!r}", flush=True)
                return
            self.probe(path, lambda: self.click_node(buttons[name]))
        elif path == "shell/spotlight":
            self.probe(path, lambda: self.dispatch("launcher"))
        elif path.startswith("shell/dock-menu/"):
            title = path.split("/", 2)[2]
            name = DOCK_NAMES.get(title, title)
            node = self.find_node(("push button", "button", "list item", "toggle button"), lambda n: n == name)
            if node is None:
                print(f"  ! {path}: no Dock item {name!r}", flush=True)
                return
            box = self.extents(node)
            self.probe(path, lambda: self.click_at(box[0] + box[2] / 2, box[1] + box[3] / 2, "right"), "menu")
        elif path == "shell/desktop-context":
            self.probe(path, lambda: self.click_at(md.OUTPUT_W * 0.45, md.OUTPUT_H * 0.5, "right"), "context")

    def run(self) -> int:  # type: ignore[override]
        self.start()
        try:
            data = json.loads((Path(self.args.states) / "Shell.json").read_text())
            paths = [s["path"] for s in data["states"] if "#" not in s["path"]]
            self.walk(paths)
        finally:
            with open(self.dest / "states.jsonl", "w") as handle:
                for state in self.records:
                    handle.write(json.dumps(state, ensure_ascii=False) + "\n")
            for child in reversed(self.children):
                child.terminate()
        return 0


def link_bins(source: Path) -> Path:
    target = source.parent / (source.name + "-shell")
    target.mkdir(exist_ok=True)
    for entry in source.iterdir():
        link = target / entry.name
        if not link.exists():
            link.symlink_to(entry)
    for cargo, shipped in CARGO_NAMES.items():
        link = target / cargo
        if not link.exists() and (source / shipped).exists():
            link.symlink_to(source / shipped)
    return target


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--states", default=str(cm.SUMMARIES))
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.audio, args.no_backlight, args.fake_hardware = "fake", False, True
    if args.inner:
        return ShellWalk(args, args.inner).run()
    args.bin_dir = str(link_bins(Path(args.bin_dir).resolve()))
    args.out = str(Path(args.out).resolve())
    args.states = str(Path(args.states).resolve())
    argv = ["--bin-dir", args.bin_dir, "--out", args.out, "--states", args.states, "--niri", args.niri]
    # run_menu_dismiss.outer re-runs its own file with --inner; point it here.
    md.__file__ = str(Path(__file__).resolve())
    return md.outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
