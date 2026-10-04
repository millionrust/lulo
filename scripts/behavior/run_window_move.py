#!/usr/bin/env python3
"""Verify Wayland title-bar moves and edge resizing in the shipped nested niri session.

    python3 scripts/behavior/run_window_move.py --niri PATH --bin-dir DIR

Input is injected into this run's headless Sway parent only. Its private runtime,
held wayland socket names and wlinput safety guard keep it away from wayland-1.
"""
from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageChops

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import run_lulo  # noqa: E402
import wlinput  # noqa: E402


def gtk_window() -> int:
    import gi
    gi.require_version("Gtk", "4.0")
    from gi.repository import Gtk

    app = Gtk.Application(application_id="org.example.WindowMoveTest")
    app.connect("activate", lambda app: _show_gtk(app, Gtk))
    return app.run([])


def _show_gtk(app, Gtk) -> None:
    window = Gtk.ApplicationWindow(application=app, title="Window Move Test")
    window.set_default_size(420, 300)
    button = Gtk.Button(label="Click to check virtual pointer")
    button.connect("clicked", lambda *_: window.set_title("Pointer Works"))
    window.set_child(button)
    window.present()


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args, self.work = args, work
        self.env = {**run_lulo.isolated_environment(work), **os.environ}
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.logs = work / "logs"
        self.logs.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []
        self.results: list[tuple[bool, str]] = []

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        return self._track(subprocess.Popen(argv, env={**self.env, **(extra or {})},
                                             stdout=open(self.logs / f"{name}.log", "w"),
                                             stderr=subprocess.STDOUT, close_fds=True))

    def spawn_optional(self, argv: list[str], name: str,
                       extra: dict[str, str] | None = None) -> subprocess.Popen | None:
        """Start a shell session piece that need not be in every bin dir.

        `monkey.py`'s `_launch_shell_extras` already skips a missing shell
        binary instead of crashing (its bin dirs often hold only the one app
        under test); this mirrors that here, since `start()` always tries to
        bring up the Dock and Mission Control to match the shipped session.
        """
        if not Path(argv[0]).exists():
            print(f"{name} binary not found at {argv[0]}, skipping", flush=True)
            return None
        return self.spawn(argv, name, extra)

    def _track(self, process: subprocess.Popen) -> subprocess.Popen:
        self.children.append(process)
        return process

    def wait_for(self, predicate, timeout: float = 30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except (OSError, RuntimeError, ValueError, KeyError):
                value = None
            if value:
                return value
            time.sleep(0.2)
        return None

    def check(self, name: str, ok: bool, detail: str = "") -> None:
        self.results.append((ok, name))
        print(f"{'PASS' if ok else 'FAIL'} {name}{(': ' + detail) if detail else ''}", flush=True)

    def niri(self, *args: str):
        request = {("windows",): "Windows", ("outputs",): "Outputs"}.get(args)
        if request is None:
            raise ValueError(f"unsupported niri query: {args}")
        last_error = None
        for _ in range(3):
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                    connection.settimeout(1.0)
                    connection.connect(self.env["NIRI_SOCKET"])
                    connection.sendall(json.dumps(request).encode() + b"\n")
                    with connection.makefile("rb") as stream:
                        reply = json.loads(stream.readline(1024 * 1024))
                if "Err" in reply:
                    raise RuntimeError(f"niri {request}: {reply['Err']}")
                return reply["Ok"][request]
            except (OSError, ValueError, KeyError, RuntimeError) as error:
                last_error = error
        raise RuntimeError(f"niri {request} failed after 3 short attempts") from last_error

    def windows(self):
        return self.niri("windows") or []

    def window(self, app_id: str):
        return next((w for w in self.windows() if w.get("app_id") == app_id), None)

    def window_matching(self, app_id: str, predicate):
        candidate = self.window(app_id)
        return candidate if candidate and predicate(candidate) else None

    @staticmethod
    def geometry(window: dict) -> tuple[float, float, float, float]:
        layout = window.get("layout") or {}
        pos = (layout.get("tile_pos_in_workspace_view") or
               layout.get("pos_in_scrolling_layout") or [0, 0])
        size = layout.get("window_size") or layout.get("tile_size") or [500, 400]
        return float(pos[0]), float(pos[1]), float(size[0]), float(size[1])

    def swaymsg(self, *args: str):
        result = subprocess.run(["swaymsg", "-s", str(self.sway_socket), "-r", *args],
                                env=self.env, capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def parent_point(self, x: float, y: float) -> tuple[float, float]:
        # Wayland pointer coordinates are surface-local logical pixels; niri's
        # output can be smaller than the parent Sway surface in this nested setup.
        return self.niri_rect[0] + x, self.niri_rect[1] + y

    def drag(self, start: tuple[float, float], end: tuple[float, float],
             steps: int = 8, step_delay: float = 0.04, grab_delay: float = 0.0) -> None:
        self.pointer.drag(self.parent_point(*start), self.parent_point(*end),
                          self.parent_width, self.parent_height, steps=steps,
                          step_delay=step_delay, grab_delay=grab_delay)

    def start(self) -> None:
        self.locks = []
        for name in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / name, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        config = self.logs / "sway.conf"
        mode = "1920x1080" if self.args.frame_only else "1440x900"
        config.write_text(f"xwayland disable\ndefault_border none\noutput HEADLESS-1 mode {mode} position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(config)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise RuntimeError("headless Sway did not start")
        self.sway_socket = self.wait_for(lambda: next(self.runtime.glob("sway-ipc.*.sock"), None))
        if not self.sway_socket:
            raise RuntimeError("headless Sway IPC socket did not appear")
        outputs = self.swaymsg("-t", "get_outputs") or []
        output = next((o for o in outputs if o.get("active")), outputs[0] if outputs else {})
        if output:
            # Settings is taller than Sway's small default headless mode. Give
            # it enough vertical room to expose all four resize edges.
            self.swaymsg("output", output["name"], "mode", "1920x1080" if self.args.frame_only else "1280x900")
            time.sleep(0.5)
            outputs = self.swaymsg("-t", "get_outputs") or []
            output = next((o for o in outputs if o.get("active")), output)
        rect = output.get("rect") or {"width": 1440, "height": 900}
        self.parent_width, self.parent_height = rect["width"], rect["height"]

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        # Keep the shipped session rules and replace only the two packaged service paths.
        bins = Path(self.args.bin_dir)
        shell = shell.replace("/usr/libexec/rmac/rmac-dock", str(bins / "dock"))
        shell = shell.replace("/usr/libexec/rmac/rmac-mission-control", str(bins / "mission-control"))
        if getattr(self.args, "fallback_shortcuts", False):
            fallback = (REPO / "packaging/rmac-session/shortcuts-fallback.kdl")
            bindings = [line for line in fallback.read_text(encoding="utf-8").splitlines()
                        if line.lstrip().startswith("Mod+")]
            bindings = [line.replace("/usr/libexec/rmac/rmac-shortcut-dispatch",
                                     str(bins / "rmac-shortcut-dispatch")) for line in bindings]
            shell = shell.replace("binds {", "binds {\n" + "\n".join(bindings), 1)
        niri_config = self.logs / "niri.kdl"
        niri_config.write_text(shell)
        validate = subprocess.run([self.args.niri, "validate", "-c", str(niri_config)], env=self.env,
                                  capture_output=True, text=True)
        self.check("shipped shell.kdl validates", validate.returncode == 0, validate.stderr[-300:])
        existing = {p.name for p in self.runtime.glob("wayland-*")}
        self.niri_process = self.spawn([self.args.niri, "-c", str(niri_config)], "niri",
                                       {"WAYLAND_DISPLAY": self.sway_display,
                                        "LIBGL_ALWAYS_SOFTWARE": "1"})
        self.socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        self.display = self.wait_for(lambda: next((p.name for p in self.runtime.glob("wayland-*")
                                                    if not p.name.endswith(".lock") and p.name not in existing), None))
        if not self.socket or not self.display:
            raise RuntimeError("nested niri did not start")
        self.env.update({"WAYLAND_DISPLAY": self.display, "NIRI_SOCKET": str(self.socket),
                         "RMAC_BEHAVIOR_NESTED": "1"})
        outputs = self.niri("outputs") or {}
        output = next(iter(outputs.values()), {}).get("logical", {})
        self.width, self.height = output.get("width", 1440), output.get("height", 900)
        tree = self.swaymsg("-t", "get_tree") or {}
        stack = [tree]
        niri_node = None
        while stack:
            node = stack.pop()
            if node.get("pid") == self.niri_process.pid:
                niri_node = node
                break
            stack.extend(node.get("nodes", []))
            stack.extend(node.get("floating_nodes", []))
        node_rect = (niri_node or {}).get("rect") or {
            "x": 0, "y": 0, "width": self.parent_width, "height": self.parent_height,
        }
        self.niri_rect = (node_rect["x"], node_rect["y"], node_rect["width"], node_rect["height"])
        print(f"nested displays: niri={self.width}x{self.height}, Sway={self.parent_width}x{self.parent_height}, "
              f"niri surface={self.niri_rect}", flush=True)
        # The shipped shell starts these services. Their HOME/XDG state is private to this run.
        trash_home = Path(self.env["XDG_DATA_HOME"]) / "Trash"
        for folder in ("files", "info"):
            (trash_home / folder).mkdir(parents=True, exist_ok=True)
        (Path(self.env["HOME"]) / "Desktop" / "preexisting-frame-test.txt").write_text("first map\n")
        self.spawn_optional([str(bins / "dock")], "dock",
                            {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
        if self.args.frame_only:
            self.spawn_optional([str(bins / "wallpaper")], "wallpaper",
                                {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
        self.spawn_optional([str(bins / "mission-control"), "--service"], "mission-control",
                            {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
        time.sleep(3)
        if self.args.frame_only:
            def settled_output():
                logical = next(iter((self.niri("outputs") or {}).values()), {}).get("logical", {})
                if logical.get("width") == self.parent_width and logical.get("height") == self.parent_height:
                    return logical
                return None

            settled = self.wait_for(settled_output, 10)
            self.check("nested niri reaches the 1920×1080 output size", bool(settled))
            if settled:
                self.width, self.height = settled["width"], settled["height"]
        self.pointer = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display})

    def capture(self, name: str) -> Image.Image:
        path = self.work / f"{name}.png"
        result = subprocess.run(["grim", str(path)], env=self.env,
                                capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError(f"grim {name}: {result.stderr[-300:]}")
        return Image.open(path).convert("RGB")

    @staticmethod
    def changed_pixels(before: Image.Image, after: Image.Image, box: tuple[int, int, int, int]) -> int:
        if before.size != after.size:
            raise RuntimeError(f"output changed size between captures: {before.size} -> {after.size}")
        difference = ImageChops.difference(before.crop(box), after.crop(box)).convert("L")
        return sum(difference.histogram()[21:])

    def assert_repaint_without_input(self) -> None:
        """File watcher updates must commit while the wallpaper and Dock are idle."""
        before = self.capture("idle-before")
        preexisting = Path(self.env["HOME"]) / "Desktop" / "preexisting-frame-test.txt"
        preexisting.unlink()
        time.sleep(2)
        removed = self.capture("desktop-preexisting-removed")
        desktop_box = (max(0, before.width - 280), 35, before.width, min(before.height, 280))
        count = self.changed_pixels(before, removed, desktop_box)
        self.check("Wallpaper paints preexisting Desktop icon on first map", count > 100,
                   f"changed pixels after removal={count}")
        desktop_file = Path(self.env["HOME"]) / "Desktop" / "frame-repaint-test.txt"
        desktop_file.write_text("frame repaint\n")
        time.sleep(2)
        added = self.capture("desktop-added")
        count = self.changed_pixels(removed, added, desktop_box)
        self.check("Wallpaper paints a new Desktop icon without input", count > 100,
                   f"changed pixels={count}")

        moved = subprocess.run(["gio", "trash", str(desktop_file)], env=self.env,
                               capture_output=True, text=True, timeout=15)
        self.check("Private Desktop item moved to Trash", moved.returncode == 0,
                   moved.stderr[-200:])
        time.sleep(2)
        filled = self.capture("trash-filled")
        dock_box = (0, max(0, filled.height - 150), filled.width, filled.height)
        count = self.changed_pixels(added, filled, dock_box)
        self.check("Dock paints the full Bin without input", count > 100,
                   f"changed pixels={count}")
        trash_home = Path(self.env["XDG_DATA_HOME"]) / "Trash"
        for folder in ("files", "info"):
            for entry in (trash_home / folder).glob("frame-repaint-test.txt*"):
                entry.unlink()
        time.sleep(2)
        emptied = self.capture("trash-emptied")
        count = self.changed_pixels(filled, emptied, dock_box)
        self.check("Dock paints the empty Bin without input", count > 100,
                   f"changed pixels={count}")

    def assert_first_frame_geometry(self, window: dict, title: str) -> None:
        """The first mapped window image must start at niri's visible geometry."""
        for phase in ("first-frame", "idle-after-map"):
            if phase == "idle-after-map":
                time.sleep(2)
                window = self.window("org.rmac.SystemSettings") or window
            shot = self.capture(f"{title.lower()}-{phase}")
            x, y, width, height = map(round, self.geometry(window))
            # The selected General row can cross the midpoint after the
            # screen-fit resize; sample the plain sidebar near the bottom.
            sample_y = min(max(y + height - 24, 0), shot.height - 1)
            edge_x = min(max(x + 4, 0), shot.width - 1)
            content_x = min(max(x + 210, 0), shot.width - 1)
            edge = shot.getpixel((edge_x, sample_y))
            content = shot.getpixel((content_x, sample_y))
            distance = sum(abs(a - b) for a, b in zip(edge, content))
            self.check(f"{title} {phase} has no wallpaper inset", distance <= 35,
                       f"edge={edge}, content={content}, delta={distance}; geometry={(x, y, width, height)}")

    def assert_quick_settings_lifecycle(self) -> None:
        """Dismissing the popover must leave its shortcut endpoint alive."""
        bins = Path(self.args.bin_dir)
        process = self.spawn([str(bins / "rmac-quick-settings")], "quick-settings")
        endpoint = self.runtime / "rmac" / "shortcut-quick-settings.sock"
        ready = self.wait_for(endpoint.exists, 20)
        self.check("Quick Settings shortcut endpoint becomes ready", bool(ready))
        if not ready:
            process.terminate()
            process.wait(10)
            return

        def dispatch() -> bool:
            result = subprocess.run([str(bins / "rmac-shortcut-dispatch"), "quick-settings"],
                                    env=self.env, capture_output=True, text=True, timeout=10)
            return result.returncode == 0

        before = self.capture("quick-before")
        first = dispatch()
        time.sleep(1)
        opened = self.capture("quick-opened")
        region = (max(0, before.width - 500), 0, before.width, min(before.height, 650))
        painted = self.changed_pixels(before, opened, region)
        self.check("Quick Settings opens from private shortcut without input",
                   first and painted > 100, f"changed pixels={painted}")
        second = dispatch()
        time.sleep(1)
        closed = self.capture("quick-closed")
        dismissed = self.changed_pixels(opened, closed, region)
        self.check("Quick Settings dismisses from private shortcut without input",
                   second and dismissed > 100, f"changed pixels={dismissed}")
        alive = process.poll() is None and endpoint.exists()
        self.check("Quick Settings keeps its endpoint after popover close", alive)
        third = dispatch()
        time.sleep(1)
        reopened = self.capture("quick-reopened")
        painted_again = self.changed_pixels(closed, reopened, region)
        self.check("Quick Settings opens again without service restart",
                   third and painted_again > 100, f"changed pixels={painted_again}")
        process.terminate()
        process.wait(10)

    def assert_move(self, app_id: str, title: str, launch: list[str]) -> None:
        process = self.spawn(launch, app_id.replace(".", "-"))
        window = self.wait_for(lambda: self.window(app_id), 40)
        self.check(f"{title} mapped floating", bool(window and window.get("is_floating")))
        if not window:
            process.terminate()
            process.wait(5)
            return
        time.sleep(1)
        x, y, width, height = self.geometry(window)
        if app_id == "org.example.WindowMoveTest":
            self.pointer.click(*self.parent_point(x + width / 2, y + height / 2),
                               self.parent_width, self.parent_height)
            clicked = self.wait_for(lambda: (self.window(app_id) or {}).get("title") == "Pointer Works", 3)
            self.check("GTK content receives a virtual pointer click", bool(clicked))
        start, end = (x + width * .5, y + 18), (x + width * .5 + 150, y + 100)
        self.drag(start, end, steps=12, step_delay=0.06, grab_delay=0.15)
        moved = self.wait_for(
            lambda: self.window_matching(
                app_id, lambda candidate: abs(self.geometry(candidate)[0] - x) > 30
                or abs(self.geometry(candidate)[1] - y) > 30),
            8,
        )
        new_geometry = self.geometry(moved) if moved else (x, y, width, height)
        changed = abs(new_geometry[0] - x) > 30 or abs(new_geometry[1] - y) > 30
        self.check(f"{title} title-bar drag changes niri position", changed,
                   f"{(x, y)} -> {new_geometry[:2]}; layout={window.get('layout')}")
        time.sleep(2)
        settled = self.window(app_id)
        settled_geometry = self.geometry(settled) if settled else (0, 0, 0, 0)
        stays = abs(settled_geometry[0] - new_geometry[0]) < 2 and abs(settled_geometry[1] - new_geometry[1]) < 2
        self.check(f"{title} remains at dragged position", stays, str(settled_geometry[:2]))
        if app_id != "org.rmac.Calculator":
            self.assert_edge_resize(app_id, title)
        process.terminate()
        process.wait(10)

    def assert_edge_resize(self, app_id: str, title: str) -> None:
        window = self.window(app_id)
        if not window:
            return
        x, y, width, height = self.geometry(window)
        if app_id == "org.rmac.Calculator":
            # macOS Calculator's Basic window is fixed at 230x408: an edge
            # drag and a title-bar double-click both leave it unchanged.
            # CALC-13 made Lulo's surface fixed too. Testing for growth here
            # would report that intended behavior as an intermittent failure.
            self.drag((x, y + height / 2), (x - 160, y + height / 2))
            time.sleep(0.3)
            after = self.geometry(self.window(app_id) or window)
            fixed = abs(after[2] - width) < 2 and abs(after[3] - height) < 2
            self.check("Calculator left-edge drag preserves fixed size", fixed,
                       f"{(width, height)} -> {after[2:]}")
            return
        initial_width = width
        edge_y = y + height / 2
        resized = False
        offset_used = None
        for offset in (0, 1, -1, 2, -2, 4, -4, 8, -8):
            current = self.window(app_id)
            if not current:
                break
            x, y, width, height = self.geometry(current)
            if width > initial_width + 30:
                resized = True
                break
            edge_y = y + height / 2
            self.drag((x + offset, edge_y), (x + offset - 160, edge_y))
            candidate = self.wait_for(
                lambda: self.window_matching(
                    app_id, lambda candidate: self.geometry(candidate)[2] > initial_width + 30),
                3.0,
            )
            if candidate:
                resized = True
                offset_used = offset
                break
        final = self.window(app_id) or window
        after = self.geometry(final)
        expanded = resized or after[2] > initial_width + 30
        self.check(f"{title} left-edge resize changes niri width", expanded,
                   f"width {initial_width} -> {after[2]}; edge={(x, edge_y)}; grab offset={offset_used}")

    def resize_settings(self) -> None:
        process = self.spawn([str(Path(self.args.bin_dir) / "rmac-system-settings")], "settings")
        window = self.wait_for(lambda: self.window("org.rmac.SystemSettings"), 40)
        self.check("Settings mapped floating", bool(window and window.get("is_floating")))
        if window:
            self.assert_first_frame_geometry(window, "Settings")
            time.sleep(1)
            # The first-frame helper refreshes its own snapshot after the
            # screen-fit configure; the caller must refresh as well. Dragging
            # from the original, oversized geometry can miss the title bar.
            window = self.window("org.rmac.SystemSettings") or window
            x, y, width, height = self.geometry(window)
            # Move the mapped window into the visible area before asking niri
            # to resize it. This makes the same move request establish a
            # usable resize edge for oversized initial client bounds.
            self.drag_settings_title(x, y, width)
            placed = self.wait_for(
                lambda: self.window_matching(
                    "org.rmac.SystemSettings",
                    lambda candidate: abs(self.geometry(candidate)[0] - x) >= 30
                    or abs(self.geometry(candidate)[1] - y) >= 30),
                8,
            )
            placed_geometry = self.geometry(placed) if placed else (x, y, width, height)
            intersects_output = (
                placed_geometry[0] < self.width
                and placed_geometry[1] < self.height
                and placed_geometry[0] + placed_geometry[2] > 0
                and placed_geometry[1] + placed_geometry[3] > 0
            )
            self.check("Settings title-bar drag brings it into the output", bool(placed) and intersects_output,
                       f"{(x, y, width, height)} -> {placed_geometry}")
            # The first move can also resize Settings from its oversized
            # startup bounds. Let niri finish that configure and pointer
            # release before starting another drag from the new title bar.
            time.sleep(1)
            placed_geometry = self.geometry(self.window("org.rmac.SystemSettings") or window)
            x, y, width, height = placed_geometry
            sx, sy, sw, sh = x, y, width, height
            self.drag_settings_title(sx, sy, sw)
            moved = self.wait_for(
                lambda: self.window_matching(
                    "org.rmac.SystemSettings",
                    lambda candidate: abs(self.geometry(candidate)[0] - sx) >= 30
                    or abs(self.geometry(candidate)[1] - sy) >= 30),
                8,
            )
            mg = self.geometry(moved) if moved else (sx, sy, sw, sh)
            # Niri may clamp a 140 px grab to 30 px near an output edge.
            # That is still a real move, well beyond compositor jitter.
            changed = abs(mg[0] - sx) >= 30 or abs(mg[1] - sy) >= 30
            self.check("Settings title bar remains movable", changed, f"{(sx, sy)} -> {mg[:2]}")
            self.assert_edge_resize("org.rmac.SystemSettings", "Settings")
            time.sleep(2)
            self.assert_double_click_zoom("org.rmac.SystemSettings", "Settings")
        process.terminate()
        process.wait(10)

    def drag_settings_title(self, x: float, y: float, width: float) -> None:
        """Grab visible title chrome and move toward the room on the output.

        Settings may start wider than the nested output. The right end of its
        toolbar can then be outside the output, and a second rightward drag
        after the first move can be clamped at the same compositor position.
        """
        toolbar_x = x + width - 60
        grab_x = toolbar_x if 0 < toolbar_x < self.width - 30 else x + 160
        room_left = max(0.0, x)
        room_right = max(0.0, self.width - (x + width))
        delta_x = 140 if room_right >= room_left else -140
        # The first motion asks GPUI and then niri to begin the move grab.
        # Let that round trip finish before sending the rest of the path.
        self.drag((grab_x, y + 18), (grab_x + delta_x, y + 18),
                  steps=12, step_delay=0.06, grab_delay=0.15)

    def double_click(self, point: tuple[float, float]) -> None:
        """Send two stationary presses within GPUI's double-click interval.

        The Wayland backend now waits for pointer motion beyond its title-bar
        drag threshold before requesting a compositor move, so the matching
        releases must reach GPUI without synthetic drag motion.
        """
        self.pointer.click(*self.parent_point(*point),
                           self.parent_width, self.parent_height, count=2)

    def assert_double_click_zoom(self, app_id: str, title: str) -> None:
        """Double-clicking the title bar Zooms (SET-33), then Zooms back.

        The default double-click action is Zoom: a toggle between the
        window's user size and the working area, routed through
        `send_window_action` so it lands on niri as a real floating-frame
        request rather than GPUI's own no-op `zoom_window()`. The shipped
        Dock runs in this nested session (menubar does not), so the
        "never under the Dock" check below only covers the Dock's
        exclusive zone.

        niri applies the move, width and height as separate frame changes.
        Wait for the final frame before testing its bounds or clicking again.
        The nested output can resize after startup, so read its live size.
        """
        window = self.window(app_id)
        if not window:
            return
        x, y, width, height = self.geometry(window)
        # Same point `assert_move`'s title-bar drag already proves lands on
        # the drag region (not a shadow margin or a toolbar control).
        title_bar_point = (x + width * 0.5, y + 18)
        self.double_click(title_bar_point)

        if app_id == "org.rmac.Calculator":
            # The real Mac leaves its 230x408 Basic window unchanged after a
            # title-bar double-click. Its fixed surface has no Zoom target.
            time.sleep(0.5)
            after = self.geometry(self.window(app_id) or window)
            fixed = abs(after[2] - width) < 2 and abs(after[3] - height) < 2
            self.check("Calculator title-bar double-click preserves fixed size", fixed,
                       f"{(width, height)} -> {after[2:]}")
            outputs = self.niri("outputs") or {}
            logical = next(iter(outputs.values()), {}).get("logical", {})
            output_width = logical.get("width", self.width)
            output_height = logical.get("height", self.height)
            on_screen = (after[0] >= -1 and after[1] >= -1
                         and after[0] + after[2] <= output_width + 1
                         and after[1] + after[3] <= output_height + 1)
            self.check("Calculator stays on screen after double-click", on_screen, str(after))
            self.check("Calculator stays above the Dock after double-click",
                       after[1] + after[3] < output_height - 5,
                       f"bottom={after[1] + after[3]}, output height={output_height}")
            self.double_click((after[0] + after[2] * 0.5, after[1] + 18))
            time.sleep(0.5)
            again = self.geometry(self.window(app_id) or window)
            still_fixed = abs(again[2] - width) < 2 and abs(again[3] - height) < 2
            self.check("A second double-click keeps Calculator fixed", still_fixed,
                       f"{(width, height)} -> {again[2:]}")
            return

        def grew_substantially(candidate) -> bool:
            cw, ch = self.geometry(candidate)[2:]
            return cw > width + 80 or ch > height + 80

        # Mission Control applies Zoom out of process (SET-33), so this
        # window's own content can also settle by a few points right after
        # the click completely independently of it — a false "it changed"
        # signal well short of an actual Fill. Wait for a substantial size
        # change, the same threshold the check below uses, not any change.
        #
        # Direct socket queries have a short timeout, so one slow IPC reply
        # cannot consume the whole wait budget after the resize request.
        zoomed = self.wait_for(
            lambda: self.window_matching(app_id, grew_substantially),
            15.0,
        )
        if zoomed:
            time.sleep(2)
            zoomed = self.window(app_id) or zoomed
        zoomed_geometry = self.geometry(zoomed) if zoomed else (x, y, width, height)
        outputs = self.niri("outputs") or {}
        live_output = next(iter(outputs.values()), {}).get("logical", {})
        output_width = live_output.get("width", self.width)
        output_height = live_output.get("height", self.height)
        grew = (zoomed_geometry[2] > width + 80 or zoomed_geometry[3] > height + 80)
        self.check(f"{title} title-bar double-click Zooms", grew,
                   f"{(width, height)} -> {zoomed_geometry[2:]}")
        zx, zy, zw, zh = zoomed_geometry
        fits_output = (
            zx >= -1 and zy >= -1
            and zx + zw <= output_width + 1
            and zy + zh <= output_height + 1
        )
        # A real fill against the Dock's exclusive zone measurably stops
        # short of the full output height; reaching all the way down means
        # the Dock reservation was ignored (the owner's "goes under the
        # Dock" report).
        clears_dock = zy + zh < output_height - 5
        self.check(f"Zoomed {title} stays on screen", fits_output, str(zoomed_geometry))
        self.check(f"Zoomed {title} never goes under the Dock", clears_dock,
                   f"bottom={zy + zh}, output height={output_height}")

        # A second double-click Zooms back to the user's previous size.
        history_path = self.runtime / "rmac" / "tile-history.json"
        history_before = history_path.read_text() if history_path.exists() else "missing"
        restore_point = (zx + zw * 0.5, zy + 18)
        self.double_click(restore_point)
        restored = self.wait_for(
            lambda: self.window_matching(
                app_id, lambda candidate: abs(self.geometry(candidate)[2] - width) < 30
                and abs(self.geometry(candidate)[3] - height) < 40),
            15.0,
        )
        restored_geometry = self.geometry(restored or self.window(app_id) or zoomed or window)
        back = (abs(restored_geometry[2] - width) < 30 and abs(restored_geometry[3] - height) < 40)
        if not back:
            history_after = history_path.read_text() if history_path.exists() else "missing"
            print(f"restore diagnostic {title}: history before={history_before}; after={history_after}; "
                  f"window={self.window(app_id)}", flush=True)
        self.check(f"A second double-click restores {title}'s previous size", back,
                   f"{(width, height)} -> {restored_geometry[2:]}")

    def run(self) -> int:
        self.start()
        if self.args.frame_only or self.args.geometry_only:
            if self.args.frame_only:
                self.assert_repaint_without_input()
            process = self.spawn([str(Path(self.args.bin_dir) / "rmac-system-settings")],
                                 "settings-first-map")
            window = self.wait_for(lambda: self.window("org.rmac.SystemSettings"), 40)
            self.check("Settings mapped before input", bool(window))
            if window:
                self.assert_first_frame_geometry(window, "Settings")
            process.terminate()
            process.wait(10)
            if self.args.frame_only:
                self.assert_quick_settings_lifecycle()
            return self.finish()
        self.assert_move("org.rmac.Calculator", "Calculator", [str(Path(self.args.bin_dir) / "rmac-calculator")])
        self.resize_settings()
        self.assert_move("org.example.WindowMoveTest", "GTK", [sys.executable, str(Path(__file__).resolve()), "--gtk-window"])
        if self.args.extra_zoom:
            for app_id, title, binary in (
                ("org.rmac.Calculator", "Calculator", "rmac-calculator"),
                ("org.rmac.Files", "Files", "rmac-files"),
            ):
                process = self.spawn([str(Path(self.args.bin_dir) / binary)], f"zoom-{binary}")
                window = self.wait_for(lambda: self.window(app_id), 40)
                self.check(f"{title} Zoom window mapped", bool(window))
                if window:
                    # Files changes its initial client height by 30 px just
                    # after mapping. Capture the pre-Zoom size once settled.
                    time.sleep(1)
                    self.assert_double_click_zoom(app_id, title)
                if process.poll() is None:
                    process.terminate()
                process.wait(10)
        return self.finish()

    def finish(self) -> int:
        try:
            self.pointer.close()
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
        failed = [name for passed, name in self.results if not passed]
        print(f"\n{len(self.results) - len(failed)}/{len(self.results)} checks passed", flush=True)
        return 1 if failed else 0


def outer(args: argparse.Namespace) -> int:
    for binary in ("sway", "dbus-run-session"):
        if not shutil.which(binary):
            raise SystemExit(f"{binary} is required")
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-window-move-"))
    env = run_lulo.isolated_environment(work)
    run_lulo.refuse_live_session(env)
    try:
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--inner", str(work), "--niri", args.niri, "--bin-dir", args.bin_dir,
                                *(["--extra-zoom"] if args.extra_zoom else []),
                                *(["--frame-only"] if args.frame_only else []),
                                *(["--geometry-only"] if args.geometry_only else [])], env=env)
    finally:
        runtime = Path(env["XDG_RUNTIME_DIR"])
        if run_lulo.reap(runtime):
            time.sleep(1)
            run_lulo.reap(runtime)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--extra-zoom", action="store_true", help="also check Calculator and Files double-click Zoom")
    parser.add_argument("--frame-only", action="store_true", help="check idle repaint and first-map geometry")
    parser.add_argument("--geometry-only", action="store_true", help="capture Settings before and after its screen-fit resize")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--gtk-window", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.gtk_window:
        return gtk_window()
    if args.inner:
        return Run(args, args.inner).run()
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    return outer(args)


if __name__ == "__main__":
    sys.exit(main())
