#!/usr/bin/env python3
"""Play parallel journeys in a private headless Sway + nested niri shell."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "behavior"))
import run_lulo  # noqa: E402
import run_window_move  # noqa: E402
import wlinput  # noqa: E402

import fixtures  # noqa: E402
import journey  # noqa: E402
import screencopy  # noqa: E402

BINARIES = {
    "files": "rmac-files", "text-editor": "rmac-text-editor",
    "settings": "rmac-system-settings", "calculator": "rmac-calculator",
    "preview": "rmac-preview", "notes": "rmac-notes", "terminal": "rmac-terminal",
}
APP_IDS = {
    "files": "org.rmac.Files", "text-editor": "org.rmac.TextEditor",
    "settings": "org.rmac.SystemSettings", "calculator": "org.rmac.Calculator",
    "preview": "org.rmac.Preview", "notes": "org.rmac.Notes", "terminal": "org.rmac.Terminal",
}
DOCK_NAMES = {
    "files": "Files", "text-editor": "Text Editor", "settings": "System Settings",
    "calculator": "Calculator", "preview": "Preview", "notes": "Notes", "terminal": "Terminal",
}
# The resident shell the real session starts (crates/rmac-session/units).
# Each is started only when --bin-dir has it; the socket, when named, is
# awaited so the first shortcut of a journey reaches a ready service.
RESIDENT = (
    ("rmac-top-bar", [], None),
    ("rmac-wallpaper", [], None),
    ("rmac-quick-settings", [], "rmac/shortcut-quick-settings.sock"),
    ("rmac-launcher", [], "rmac/shortcut-launcher.sock"),
    ("rmac-app-switcher", ["--service"], "rmac/app-switcher.sock"),
)
# Lulo names for controls a journey names after the Mac. Control Centre's
# Sound module title is the volume slider on Lulo; its detail (the output
# list the Mac opens) is behind the module's Sound Outputs button.
LULO_LABELS = {"Sound": "Sound Outputs"}
FORBIDDEN = {"Shut Down", "Restart", "Log Out", "Sleep", "Empty Bin", "Empty Trash", "Wi-Fi On", "Wi-Fi Off"}


def private_bus(work: Path, env: dict, bins: Path) -> Path:
    services = work / "dbus-services"
    services.mkdir()
    for name in ("org.a11y.Bus.service", "org.freedesktop.portal.Desktop.service",
                 "org.freedesktop.impl.portal.PermissionStore.service"):
        source = Path("/usr/share/dbus-1/services") / name
        if source.exists():
            shutil.copy(source, services / name)
    chooser = bins / "rmac-file-chooser"
    if chooser.is_file():
        (services / "org.freedesktop.impl.portal.desktop.rmac.filechooser.service").write_text(
            "[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.rmac.filechooser\n"
            f"Exec={chooser}\n")
        portals = work / "portals"
        portals.mkdir()
        (portals / "rmac-file-chooser.portal").write_text(
            "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.rmac.filechooser\n"
            "Interfaces=org.freedesktop.impl.portal.FileChooser;\nUseIn=rmac\n")
        env["XDG_DESKTOP_PORTAL_DIR"] = str(portals)
        config = Path(env["XDG_CONFIG_HOME"]) / "xdg-desktop-portal"
        config.mkdir(parents=True)
        (config / "rmac-portals.conf").write_text(
            "[preferred]\ndefault=none\norg.freedesktop.impl.portal.FileChooser=rmac-file-chooser\n")
    config = work / "session.conf"
    config.write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" '
        '"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        f'<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>'
        f'<servicedir>{services}</servicedir><policy context="default">'
        '<allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/>'
        '</policy></busconfig>\n')
    return config


class Driver:
    def __init__(self, args, work: Path, data: dict, out: Path):
        self.args, self.work, self.data, self.out = args, work, data, out
        self.session = run_window_move.Run(args, work)
        self.sandbox = work / "sandbox"
        self.sandbox.mkdir()
        fixtures.prepare(self.sandbox, data.get("setup", {}))
        self.apps = {}
        self.current = None
        self.scratch = work / "capture"
        self.scratch.mkdir()
        self.shot_serial = 0

    def start(self):
        # run_window_move supplies the proven Sway -> niri setup. Give it a
        # private copy of shell.kdl with every command bound to --bin-dir.
        original = run_window_move.REPO / "packaging/rmac-session/shell.kdl"
        private_root = self.work / "session-source"
        config = private_root / "packaging/rmac-session/shell.kdl"
        config.parent.mkdir(parents=True)
        shell = original.read_text()
        for binary in Path(self.args.bin_dir).glob("rmac-*"):
            shell = shell.replace(f"/usr/libexec/rmac/{binary.name}", str(binary))
        # niri makes Alt its Mod key when it runs nested in a window, so the
        # session's Mod (⌘ = Super) bindings such as ⌘Tab would otherwise
        # pass straight through to the focused app. Keep Super, as on a TTY.
        shell = shell.replace("input {\n", 'input {\n    mod-key-nested "Super"\n', 1)
        config.write_text(shell)
        run_window_move.REPO = private_root
        self.session.start()
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.session.env, capture_output=True, timeout=10, check=False)
        self.session.output = next(iter(self.session.niri("outputs") or {}), "winit")
        self.sampler = screencopy.Screencopy(self.session.pointer)
        bins = Path(self.args.bin_dir)
        for binary, arguments, socket in RESIDENT:
            if (bins / binary).exists():
                process = self.session.spawn([str(bins / binary), *arguments], binary)
                if socket and not self.session.wait_for(lambda: (self.session.runtime / socket).exists(), 10):
                    log = self.session.logs / f"{binary}.log"
                    detail = log.read_text(errors="replace")[-700:] if log.exists() else "no process log"
                    raise RuntimeError(f"{binary} did not open {socket} (exit={process.poll()}): {detail}")
        time.sleep(1)

    def window(self):
        if not self.current:
            return None
        process = self.apps.get(self.current)
        if process is None:
            return None
        windows = self.session.windows()
        return (next((w for w in windows if w.get("pid") == process.pid), None)
                or next((w for w in windows if w.get("app_id") == APP_IDS[self.current]), None))

    def refresh_output(self):
        # The nested niri output is resized by Sway after it starts, so the
        # logical size read once at start-up can be stale (1280×800 vs 900).
        try:
            outputs = self.session.niri("outputs") or {}
        except RuntimeError:
            return
        logical = next(iter(outputs.values()), {}).get("logical") or {}
        if logical.get("width") and logical.get("height"):
            self.session.width, self.session.height = logical["width"], logical["height"]

    def screenshot(self) -> Image.Image:
        """niri's own composition of its output.

        Shots come from niri, not from the parent Sway: nested niri's winit
        backend can stop presenting new frames to Sway for seconds (or for
        the rest of a run) after a window maps while its scene stays current,
        so Sway pixels showed stale screens. niri renders this from the same
        scene its DRM backend scans out on real hardware. The pointer is
        left out so a moved cursor never counts as a changed screen.
        """
        for _attempt in range(3):
            self.shot_serial += 1
            path = self.scratch / f"niri-{self.shot_serial}.png"
            subprocess.run([self.args.niri, "msg", "action", "screenshot-screen", "--write-to-disk", "true",
                            "--show-pointer", "false", "--path", str(path)],
                           env=self.session.env, check=True, capture_output=True, timeout=10)
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                try:
                    with Image.open(path) as source:
                        image = source.convert("RGB")
                    path.unlink()
                    return image
                except (OSError, ValueError):
                    time.sleep(0.03)
        raise RuntimeError("niri did not write its screenshot")

    def capture(self, destination: Path | None = None, full=False) -> Image.Image:
        self.refresh_output()
        image = self.screenshot()
        x, y, w, h = self.capture_region(full)
        scale = image.width / max(1, self.session.width)
        crop = image.crop(tuple(round(v * scale) for v in (x, y, x + w, y + h)))
        if destination is not None:
            crop.save(destination)
        return crop

    def settled(self, full=False, timeout=4.0) -> Image.Image:
        """Wait until two shots 250 ms apart show the same screen."""
        previous = self.capture(full=full)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            time.sleep(0.25)
            current = self.capture(full=full)
            if journey.changed_pixels(previous, current) < journey.SAME_SCREEN_PIXELS:
                return current
            previous = current
        return previous

    def capture_region(self, full=False):
        if not full and self.window():
            x, y, w, h = self.session.geometry(self.window())
            x, y = max(0, int(x)), max(0, int(y))
            w, h = min(int(w), self.session.width - x), min(int(h), self.session.height - y)
            if w > 50 and h > 50:
                return (x, y, w, h)
        return (0, 0, self.session.width, self.session.height)

    def sway_region(self, full=False):
        x, y, w, h = self.capture_region(full)
        return (x + self.session.niri_rect[0], y + self.session.niri_rect[1], w, h)

    def accessible(self, pid: int | None = None):
        """Yield (node, pid) for every accessible on screen, or one app's."""
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        for index in range(desktop.childCount):
            app = desktop.getChildAtIndex(index)
            try:
                app_pid = app.get_process_id()
            except Exception:  # noqa: BLE001
                continue
            if pid is not None and app_pid != pid:
                continue
            stack = [app]
            while stack:
                node = stack.pop()
                try:
                    if node is None:
                        continue
                    yield node, app_pid
                    stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                except Exception:  # noqa: BLE001
                    continue

    def dump_accessible(self, destination: Path) -> None:
        """Every named accessible with its role and window/desktop extents."""
        import pyatspi

        rows = []
        for node, pid in self.accessible():
            try:
                if not node.name:
                    continue
                component = node.queryComponent()
                window = component.getExtents(pyatspi.WINDOW_COORDS)
                desktop = component.getExtents(pyatspi.DESKTOP_COORDS)
                rows.append({"pid": pid, "role": node.getRoleName(), "name": node.name,
                             "window": [window.x, window.y, window.width, window.height],
                             "desktop": [desktop.x, desktop.y, desktop.width, desktop.height]})
            except Exception:  # noqa: BLE001
                continue
        destination.write_text(json.dumps(rows, indent=1) + "\n")

    @staticmethod
    def text_of(node) -> str:
        try:
            text = node.queryText()
            return text.getText(0, text.characterCount)
        except Exception:  # noqa: BLE001
            return node.name or ""

    def launch(self, step: dict):
        app = step["launch"]
        self.current = app
        command = [str(Path(self.args.bin_dir) / BINARIES[app])]
        if app == "files":
            command += ["--path", str(self.sandbox / step.get("path", "."))]
        elif app == "preview":
            command += [str(self.sandbox / step["file"])]
        if not Path(command[0]).is_file():
            raise RuntimeError(f"missing binary {command[0]}")
        running = self.apps.get(app)
        if running is not None and running.poll() is None:
            if "file" in step or "path" in step:
                # Opening a document in a running app: the second process
                # hands the request to the first, as a Files double-click or
                # `xdg-open` would.
                before = {w.get("id") for w in self.session.windows()}
                self.session.spawn(command, f"{app}-open")
                if not self.session.wait_for(lambda: any(
                        w.get("id") not in before and w.get("app_id") == APP_IDS[app]
                        for w in self.session.windows()), 30):
                    raise RuntimeError(f"{app} opened no window for {step.get('file') or step.get('path')}")
            else:
                # Bringing a running app forward is a Dock click, which also
                # restores its minimised windows like the Mac.
                self.click(DOCK_NAMES[app], "Dock")
                if not self.session.wait_for(lambda: (w := self.window()) and w.get("is_focused"), 10):
                    raise RuntimeError(f"clicking {DOCK_NAMES[app]} in the Dock did not focus it")
            return
        process = self.session.spawn(command, app)
        self.apps[app] = process
        if not self.session.wait_for(lambda: self.window(), 30):
            raise RuntimeError(f"{app} did not open a nested niri window; see {self.session.logs / (app + '.log')}")

    def click(self, label: str, target: str | None = None, attempt: int = 0):
        if label in FORBIDDEN:
            raise RuntimeError(f"refusing destructive or toggle control {label!r}")
        label = LULO_LABELS.get(label, label)
        if target == "Dock":
            self.current = next((app for app, name in DOCK_NAMES.items() if name == label), self.current)
        if label == "Save" and self.current == "text-editor":
            self.session.pointer.key("return")
            return
        if self.current == "calculator" and label == "2nd":
            window = self.window()
            if not window:
                raise RuntimeError("Calculator has no window for 2nd")
            x, y, _w, _h = self.session.geometry(window)
            px, py = self.session.parent_point(x + 40, y + 210)
            self.session.pointer.click(px, py, self.session.parent_width, self.session.parent_height)
            return
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
        stack = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        candidates = []
        current = self.apps.get(self.current)
        windows = self.session.windows()
        # AT-SPI on Wayland only knows coordinates inside a surface. A niri
        # window's surface sits at its niri geometry; the top bar is anchored
        # at the output origin; other layer surfaces (Dock, Control Centre,
        # Spotlight) have no known origin, so they are pressed through their
        # accessible action, which is what assistive tech does.
        top_bar = next((p.pid for p in self.session.children
                        if isinstance(p.args, list) and Path(p.args[0]).name == "rmac-top-bar"), None)
        top_bar_control = label in {"Wi-Fi", "Control Centre"}
        dock = next((p for p in self.session.children
                     if isinstance(p.args, list) and Path(p.args[0]).name == "dock"), None)
        while stack:
            node = stack.pop()
            try:
                if node is None:
                    continue
                name = node.name or ""
                if (name == label or name.startswith(label + ",") or
                        (top_bar_control and name.startswith(label + " "))):
                    pid = node.getApplication().get_process_id()
                    if top_bar_control and (pid != top_bar or
                                            node.getRoleName() not in {"push button", "button"}):
                        stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                        continue
                    if not top_bar_control and not node.getState().contains(pyatspi.STATE_SHOWING):
                        stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                        continue
                    if target == "Dock" and (dock is None or pid != dock.pid):
                        stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                        continue
                    owned = [w for w in windows if w.get("pid") == pid]
                    owner = (next((w for w in owned if w.get("is_focused")), None)
                             or (owned[0] if owned else None))
                    space = pyatspi.DESKTOP_COORDS if top_bar_control else pyatspi.WINDOW_COORDS
                    box = node.queryComponent().getExtents(space)
                    if box.width > 2 and box.height > 2 and box.x >= 0 and box.y >= 0:
                        role = node.getRoleName()
                        rank = 0 if role in {"list item", "tree item", "table row", "table cell"} else 1
                        rank += 0 if current is not None and pid == current.pid else 2
                        if owner is not None:
                            origin = self.session.geometry(owner)[:2]
                        elif pid == top_bar:
                            origin = (0, 0)
                        else:
                            origin = None
                        candidates.append((rank, -box.width * box.height, node, box, origin))
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:
                continue
        if candidates:
            _rank, _area, node, box, origin = min(candidates, key=lambda item: item[:2])
            if origin is None:
                try:
                    action = node.queryAction()
                    count = action.nActions
                except Exception:  # noqa: BLE001 - pyatspi raises NotImplementedError
                    count = 0
                if count < 1:
                    raise RuntimeError(f"{label!r} is on a layer surface and has no accessible action")
                action.doAction(0)
                return
            x, y = self.session.parent_point(origin[0] + box.x + box.width / 2,
                                             origin[1] + box.y + box.height / 2)
            self.session.pointer.click(x, y, self.session.parent_width, self.session.parent_height)
            return
        if top_bar_control:
            if attempt < 12:
                time.sleep(0.15)
                return self.click(label, target, attempt + 1)
            raise RuntimeError(f"no top-bar control with usable bounds named {label!r}")
        status_x = {"Lulo": 26, "Battery": self.session.width - 296}
        if label in status_x:
            x, y = self.session.parent_point(status_x[label], 15)
            self.session.pointer.click(x, y, self.session.parent_width, self.session.parent_height)
            return
        if label == "File" and self.current == "text-editor":
            x, y = self.session.parent_point(156, 15)
            self.session.pointer.click(x, y, self.session.parent_width, self.session.parent_height)
            return
        if attempt < 12:
            time.sleep(0.15)
            return self.click(label, target, attempt + 1)
        raise RuntimeError(f"no accessible control with usable bounds named {label!r}")

    def action(self, step: dict):
        kind = next(iter(journey.ACTIONS.intersection(step)))
        if kind == "launch":
            self.launch(step)
        elif kind == "click":
            self.click(step[kind], step.get("target"))
        elif kind == "menu":
            for label in step[kind]:
                if self.current == "text-editor" and label == "Parallel Journey Sandbox":
                    # Text Editor adds its format extension when saving; its
                    # Open Recent menu shows the resulting file name.
                    matches = list(self.sandbox.glob(f"{label}*"))
                    if len(matches) == 1:
                        label = matches[0].name
                self.click(label)
                time.sleep(0.15)
        elif kind == "key":
            if step[kind] in {"power", "ctrl-power", "cmd-alt-escape", "cmd-alt-s"}:
                raise RuntimeError("refusing session or device-control shortcut")
            if step[kind] == "cmd-m":
                focused = next((w.get("id") for w in self.session.windows()
                                if w.get("is_focused")), None)
                for _ in range(3):
                    self.session.pointer.key("cmd-m")
                    if focused is None or self.session.wait_for(
                            lambda: not any(w.get("id") == focused and w.get("is_focused")
                                            for w in self.session.windows()), 2.5):
                        break
            else:
                self.session.pointer.key(step[kind])
            if step[kind] == "cmd-space":
                # The private nested bus has no GlobalShortcuts portal. Route
                # the same shortcut to the resident launcher after injection.
                command = [str(Path(self.args.bin_dir) / "rmac-shortcut-dispatch"), "launcher"]
                outcome = subprocess.run(command, env=self.session.env, capture_output=True,
                                         text=True, timeout=10)
                if outcome.returncode:
                    raise RuntimeError(f"Spotlight dispatch failed: {outcome.stderr[-120:]}")
        elif kind == "type":
            self.session.pointer.type_text(step[kind].replace("$SANDBOX", str(self.sandbox)))
        elif kind == "drag_window":
            window = self.window()
            if not window:
                raise RuntimeError("no current window to drag")
            x, y, w, _h = self.session.geometry(window)
            dx, dy = step[kind]
            ax, ay = step.get("anchor", [w / 2, 18])
            after = None

            def moved():
                current = self.window()
                if not current:
                    return None
                cx, cy, _cw, _ch = self.session.geometry(current)
                return (cx, cy) if abs(cx - x) > 20 or abs(cy - y) > 20 else None

            for _ in range(3):
                candidate = self.window()
                if not candidate:
                    break
                cx, cy, _cw, _ch = self.session.geometry(candidate)
                self.session.drag((cx + ax, cy + ay), (cx + ax + dx, cy + ay + dy))
                after = self.session.wait_for(moved, 1.5)
                if after:
                    break
            if after is None:
                raise RuntimeError(f"nested niri reported no window movement after drag from {(x, y)}")

    def check_shot(self, step: dict, image: Image.Image, previous: Image.Image | None,
                   after_action: bool) -> list[str]:
        """Every assertion on a shot; an empty list means it held."""
        errors = []
        if after_action and step.get("expect_change", True) and previous is not None:
            changed = journey.changed_pixels(previous, image)
            if changed < journey.SAME_SCREEN_PIXELS:
                errors.append(f"the screen did not change ({changed} px differ from the previous shot)")
        windows = self.session.windows()
        if step.get("expect_window") and not any(
                w.get("app_id") == step["expect_window"] and w.get("is_focused") for w in windows):
            errors.append(f"expected {step['expect_window']} window is not focused")
        mapped = {w.get("app_id") for w in windows}
        errors += [f"{app} has no mapped window" for app in step.get("expect_mapped", []) if app not in mapped]
        errors += [f"{app} still has a mapped window" for app in step.get("expect_unmapped", []) if app in mapped]
        errors += [f"no sandbox file matches {pattern}" for pattern in step.get("expect_files", [])
                   if not list(self.sandbox.glob(pattern))]
        errors += [f"a sandbox file still matches {pattern}" for pattern in step.get("expect_no_files", [])
                   if list(self.sandbox.glob(pattern))]
        if "expect_text" in step:
            spec = step["expect_text"]
            process = self.apps.get(self.current)
            texts = [self.text_of(node) for node, _pid in
                     self.accessible(process.pid if process is not None else None)
                     if node.name == spec["label"]]
            if not texts:
                errors.append(f"no accessible named {spec['label']!r}")
            elif "equals" in spec and spec["equals"] not in texts:
                errors.append(f"{spec['label']!r} reads {texts!r}, expected {spec['equals']!r}")
            elif "contains" in spec and not any(spec["contains"] in text for text in texts):
                errors.append(f"{spec['label']!r} reads {texts!r}, expected it to contain {spec['contains']!r}")
        if step.get("expect_accessible"):
            import pyatspi

            names = set()
            for node, _pid in self.accessible():
                try:
                    if node.name and node.getState().contains(pyatspi.STATE_SHOWING):
                        names.add(node.name)
                except Exception:  # noqa: BLE001
                    continue
            errors += [f"nothing accessible named {name!r} is showing"
                       for name in step["expect_accessible"] if name not in names]
        return errors

    def run(self, name: str) -> dict:
        target = self.out / name
        target.mkdir(parents=True, exist_ok=True)
        result = {"journey": name, "platform": "lulo", "steps": [], "status": "passed"}
        pending = None
        timing = None
        issues = []
        previous = None
        try:
            self.start()
            for index, step in enumerate(self.data["steps"]):
                kind = next(iter(journey.ACTIONS.intersection(step)))
                if kind == "wait":
                    time.sleep(float(step[kind]))
                elif kind == "shot":
                    full = step.get("scope") == "full"
                    destination = target / f"{len(result['steps']):02d}-{step[kind]}.png"
                    # A slow first frame (a cold shell surface under nested
                    # software rendering) can land after the screen has
                    # settled; give the expected state a few seconds.
                    deadline = time.monotonic() + 6
                    while True:
                        image = self.settled(full)
                        errors = self.check_shot(step, image, previous, pending is not None)
                        if not errors or time.monotonic() > deadline:
                            break
                        time.sleep(0.5)
                    image.save(destination)
                    if self.args.full_too and not full:
                        self.capture(destination.with_suffix(".full.png"), full=True)
                    if self.args.dump_a11y:
                        self.dump_accessible(destination.with_suffix(".a11y.json"))
                    issues += [{"index": index, "shot": step[kind], "action": pending, "error": error}
                               for error in errors]
                    windows = [{"app_id": w.get("app_id"), "title": w.get("title"),
                                "focused": w.get("is_focused"), "geometry": self.session.geometry(w)}
                               for w in self.session.windows()]
                    result["steps"].append({"name": step[kind], "image": destination.name,
                                            "region": self.capture_region(full), "windows": windows,
                                            "errors": errors, "action": pending, **(timing or {})})
                    previous = image
                    pending = None
                else:
                    full = kind == "launch" or step.get("scope") == "full"
                    pending = {kind: step[kind], "index": index}
                    try:
                        # Timing samples the parent Sway output at a high rate.
                        # Nested niri can present late there, so these numbers
                        # are an upper bound; the shot itself comes from niri.
                        timing = journey.measure(
                            None, lambda: self.action(step), self.scratch,
                            probe=lambda: self.sampler.fingerprint(self.sway_region(full)))
                    except RuntimeError as error:
                        if kind not in {"click", "menu", "launch"}:
                            raise
                        issues.append({"index": index, "action": pending, "error": str(error)})
                        timing = {"first_change_ms": None, "settled_ms": None,
                                  "samples": 0, "actual_hz": None, "timed_out": False,
                                  "error": str(error)}
            if issues:
                result.update(status="failed", issues=issues)
        except Exception as error:  # noqa: BLE001 - any crash must fail the journey, never pass it
            result.update(status="failed", error=f"{type(error).__name__}: {error}")
        finally:
            if hasattr(self.session, "pointer"):
                self.session.finish()
            else:
                for process in reversed(self.session.children):
                    if process.poll() is None:
                        process.terminate()
            (target / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        for issue in issues:
            print(f"FAIL {name} step {issue['index']}: {issue['error']}", flush=True)
        if result.get("error"):
            print(f"FAIL {name}: {result['error']}", flush=True)
        return result


def outer(args) -> int:
    if journey.ROOT in args.output.resolve().parents:
        raise SystemExit("screenshots must be outside the repository")
    args.output.mkdir(parents=True, exist_ok=True)
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    failures = 0
    try:
        for path in journey.paths(args.journeys):
            work = Path(tempfile.mkdtemp(prefix="lulo-parallel-"))
            try:
                env = run_lulo.isolated_environment(work)
                run_lulo.refuse_live_session(env)
                bins = Path(args.bin_dir).expanduser().resolve()
                # run_window_move expects legacy dock/mission-control names.
                links = work / "bins"
                links.mkdir()
                for source in bins.iterdir():
                    if source.is_file() and source.name not in {"dock", "mission-control"}:
                        (links / source.name).symlink_to(source)
                if args.override_bin_dir:
                    overrides = Path(args.override_bin_dir).expanduser().resolve()
                    for source in overrides.glob("rmac-*"):
                        if source.is_file() and os.access(source, os.X_OK):
                            target = links / source.name
                            if target.exists() or target.is_symlink():
                                target.unlink()
                            target.symlink_to(source)
                for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
                    # Point at the link so --override-bin-dir also covers these.
                    (links / alias).symlink_to(links / source)
                run_lulo.install_shortcut_dispatcher(env, links)
                # The launcher discovers desktop entries through XDG. Populate
                # only this run's private data home, pointing Exec/TryExec at
                # the selected binaries so search can launch a real app.
                applications = Path(env["XDG_DATA_HOME"]) / "applications"
                applications.mkdir(parents=True, exist_ok=True)
                for source in (journey.ROOT / "packaging/rmac-apps/applications").glob("*.desktop"):
                    content = source.read_text()
                    binary = next((line.removeprefix("Exec=").split()[0]
                                   for line in content.splitlines() if line.startswith("Exec=")), None)
                    if binary and (links / Path(binary).name).exists():
                        content = content.replace(binary, str(links / Path(binary).name))
                        (applications / source.name).write_text(content)
                config = private_bus(work, env, links)
                command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                           str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                           "--niri", args.niri, "--output", str(args.output), path.stem]
                for flag in ("full_too", "dump_a11y"):
                    if getattr(args, flag):
                        command.insert(-1, "--" + flag.replace("_", "-"))
                status = subprocess.call(command, env=env, close_fds=True)
                failures += status != 0
            finally:
                if run_lulo.reap(work / "runtime"):
                    time.sleep(1)
                    run_lulo.reap(work / "runtime")
                if args.keep:
                    print(f"kept {work}", flush=True)
                else:
                    run_lulo.remove_tree(work)
    finally:
        lock.close()
    return int(bool(failures))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journeys", nargs="*")
    parser.add_argument("--bin-dir", default="~/rmac-release/inputs-20260929T1945")
    parser.add_argument("--override-bin-dir", help="prefer current app binaries here; shell falls back to --bin-dir")
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--dump-a11y", action="store_true",
                        help="save every named accessible and its extents beside each shot")
    parser.add_argument("--full-too", action="store_true",
                        help="also save the whole output beside each window shot (for review)")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    # run_window_move.Run uses this switch for its other two modes; parallel
    # journeys always use the ordinary 1440×900 nested session.
    parser.set_defaults(frame_only=False)
    args = parser.parse_args()
    if args.inner:
        data = journey.load(journey.JOURNEYS / f"{args.journeys[0]}.json")
        result = Driver(args, args.inner, data, args.output).run(args.journeys[0])
        print(f"{result['status']} {result['journey']}: {len(result['steps'])} shots", flush=True)
        return 0 if result["status"] == "passed" else 1
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
