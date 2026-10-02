#!/usr/bin/env python3
"""Record interaction-probe facts on Lulo, inside a private nested compositor.

    python3 scripts/interaction/lulo_probe.py --bin-dir DIR [--shell-bin-dir DIR] [SURFACE_ID...]
    python3 scripts/interaction/lulo_probe.py --bin-dir DIR --shell-bin-dir DIR --all

Against the installed build:

    python3 scripts/interaction/lulo_probe.py --bin-dir /usr/bin \\
        --bin-dir /usr/libexec/rmac --shell-bin-dir /usr/bin --all

Surfaces with `lulo.harness == "shell"` (a top-bar menu, Control Centre) run
inside nested niri with the shipped packaging/rmac-session/shell.kdl, the
Dock and the top bar from --shell-bin-dir/--bin-dir, nested *inside*
scripts/behavior/run_lulo.py's own headless Sway (reusing its isolation,
its private D-Bus/AT-SPI session, and wlinput.py for input) - the same
niri-inside-Sway skeleton scripts/behavior/run_window_move.py and
run_niri_minimize.py use, just with AT-SPI wired up as well. Surfaces with
`lulo.harness == "app"` reuse run_lulo.py's Nested/LuloRun directly, no
shell.

Writes tests/interaction/lulo/<surface>.json: the exact same fact shapes
mac_probe.py writes, so interaction_diff.py can compare them. Never a screenshot.
"""

from __future__ import annotations

import argparse
import datetime
import fcntl
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "behavior"))
sys.path.insert(0, str(HERE.parent))

import surfaces as sf  # noqa: E402
import probes as pr  # noqa: E402
import run_lulo  # noqa: E402
import wlinput  # noqa: E402
import atspi_assert_support as support  # noqa: E402

from run_lulo import (  # noqa: E402
    StepFailed, Unsupported, atspi, descendants, extents, has_state, name, pump, role,
)

OUT_DIR = HERE.parent.parent / "tests" / "interaction" / "lulo"
REPO = HERE.parent.parent
# The parent Sway surface's size: run_lulo.Nested's own headless output mode.
OUTPUT_W, OUTPUT_H = run_lulo.OUTPUT_W, run_lulo.OUTPUT_H


def find_bin(directories: list[Path], *candidates: str) -> Path:
    for directory in directories:
        for candidate in candidates:
            found = directory / candidate
            if found.is_file() and os.access(found, os.X_OK):
                return found.resolve()
    raise StepFailed(f"none of {candidates} found under {[str(d) for d in directories]}")


# --------------------------------------------------------------------------
# Outer process: isolate, then re-run inside a private D-Bus session.
# --------------------------------------------------------------------------


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "swaymsg", "dbus-run-session", "niri", "grim"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-interaction-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        services = work / "dbus-services"
        services.mkdir()
        source = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
        if source.exists():
            shutil.copy(source, services / source.name)
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
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        (work / "logs").mkdir(exist_ok=True)
        with open(work / "logs" / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if status != 0:
            print((work / "logs" / "session.log").read_text()[-1500:], file=sys.stderr)
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
# The "shell" harness: niri + the shipped shell, nested inside run_lulo's Sway.
# --------------------------------------------------------------------------


class ShellSession:
    def __init__(self, nested: "run_lulo.Nested", bins: list[Path], niri_bin: Optional[Path] = None) -> None:
        self.nested = nested
        self.bins = bins
        self.niri_bin = niri_bin or Path(shutil.which("niri"))
        self.children: list[subprocess.Popen] = []
        self.logs = nested.work / "logs"
        self.env = dict(nested.env)  # inherits Sway's WAYLAND_DISPLAY, SWAYSOCK, HOME, XDG_*, the private D-Bus address

    def _spawn(self, argv: list[str], label: str, extra: Optional[dict[str, str]] = None) -> subprocess.Popen:
        process = subprocess.Popen(argv, env={**self.env, **(extra or {})},
                                   stdout=open(self.logs / f"{label}.log", "w"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    def _wait_for(self, predicate, timeout: float = 20):
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

    def start(self) -> None:
        runtime = Path(self.env["XDG_RUNTIME_DIR"])
        sway_display = self.env["WAYLAND_DISPLAY"]
        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        dock = find_bin(self.bins, "rmac-dock", "dock")
        mission_control = find_bin(self.bins, "rmac-mission-control", "mission-control")
        shell = shell.replace("/usr/libexec/rmac/rmac-dock", str(dock))
        shell = shell.replace("/usr/libexec/rmac/rmac-mission-control", str(mission_control))
        niri_config = self.logs / "niri.kdl"
        niri_config.write_text(shell)
        validate = subprocess.run([str(self.niri_bin), "validate", "-c", str(niri_config)], env=self.env,
                                  capture_output=True, text=True, timeout=15)
        if validate.returncode != 0:
            raise StepFailed(f"shipped shell.kdl does not validate: {validate.stderr[-500:]}")
        existing = {p.name for p in runtime.glob("wayland-*")}
        self.niri = self._spawn([str(self.niri_bin), "-c", str(niri_config)], "niri",
                                {"WAYLAND_DISPLAY": sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        self.socket = self._wait_for(lambda: next(iter(runtime.glob("niri.*.sock")), None))
        self.display = self._wait_for(lambda: next((p.name for p in runtime.glob("wayland-*")
                                                     if not p.name.endswith(".lock") and p.name not in existing), None))
        if not self.socket or not self.display:
            raise StepFailed("nested niri did not start under Sway")
        self.env.update({"WAYLAND_DISPLAY": self.display, "NIRI_SOCKET": str(self.socket)})
        # Input is injected into the *parent* Sway (niri is just one of its
        # clients), exactly like run_window_move.py's self.pointer. niri's
        # own surface is not necessarily at the parent's (0, 0): find its
        # rect in Sway's tree so clicks land at the right screen point.
        self.input = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": sway_display})
        tree = self.nested.swaymsg("-t", "get_tree") or {}
        stack = [tree]
        niri_node = None
        while stack:
            node = stack.pop()
            if node.get("pid") == self.niri.pid:
                niri_node = node
                break
            stack.extend(node.get("nodes", []))
            stack.extend(node.get("floating_nodes", []))
        rect = (niri_node or {}).get("rect") or {"x": 0, "y": 0}
        self.niri_origin = (rect.get("x", 0), rect.get("y", 0))
        self._spawn([str(dock)], "dock")
        self.top_bar = find_bin(self.bins, "rmac-top-bar", "top-bar")
        self.top_bar_process = self._spawn([str(self.top_bar)], "top-bar")
        if not self.wait_for_populated_frame(self.top_bar_process.pid):
            raise StepFailed("the top bar did not publish an accessible tree in time")

    def wait_for_populated_frame(self, pid: int, timeout: float = 15.0) -> bool:
        """AT-SPI briefly reports an app with one empty root frame right
        after it maps; wait until it actually has content, rather than
        guessing a fixed settle time."""

        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            frames = self.frames_by_pid(pid)
            if any(sum(1 for _ in descendants(frame, limit=50)) > 1 for frame in frames):
                return True
            time.sleep(0.3)
        return False

    def parent_point(self, x: float, y: float) -> tuple[float, float]:
        """niri's nested surface is not necessarily at the parent Sway
        output's (0, 0); translate one of niri's own logical-pixel
        coordinates (what AT-SPI's extents() reports) into the parent's."""

        return self.niri_origin[0] + x, self.niri_origin[1] + y

    def click(self, x: float, y: float, **kwargs: Any) -> None:
        self.input.click(*self.parent_point(x, y), OUTPUT_W, OUTPUT_H, **kwargs)

    def move(self, x: float, y: float) -> None:
        self.input.move(*self.parent_point(x, y), OUTPUT_W, OUTPUT_H)

    def niri_query(self, key: str) -> Any:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(2.0)
            connection.connect(self.env["NIRI_SOCKET"])
            connection.sendall(json.dumps(key).encode() + b"\n")
            with connection.makefile("rb") as stream:
                reply = json.loads(stream.readline(1024 * 1024))
        if "Err" in reply:
            raise StepFailed(f"niri {key}: {reply['Err']}")
        return reply["Ok"][key]

    def start_quick_settings(self) -> None:
        quick_settings = find_bin(self.bins, "rmac-quick-settings")
        self.quick_settings_process = self._spawn([str(quick_settings)], "quick-settings")
        endpoint = Path(self.env["XDG_RUNTIME_DIR"]) / "rmac" / "shortcut-quick-settings.sock"
        if not self._wait_for(endpoint.exists, 20):
            raise StepFailed("Quick Settings did not register its shortcut endpoint")

    def start_notification_center(self) -> None:
        panel = find_bin(self.bins, "rmac-notification-center-panel")
        self.notification_center_process = self._spawn([str(panel)], "notification-center")
        endpoint = Path(self.env["XDG_RUNTIME_DIR"]) / "rmac" / "shortcut-notification-center.sock"
        if not self._wait_for(endpoint.exists, 20):
            raise StepFailed("Notification Center did not register its shortcut endpoint")

    def dispatch(self, shortcut: str) -> None:
        dispatcher = find_bin(self.bins, "rmac-shortcut-dispatch")
        result = subprocess.run([str(dispatcher), shortcut], env=self.env,
                                capture_output=True, text=True, timeout=25)
        if result.returncode != 0:
            raise StepFailed(f"rmac-shortcut-dispatch {shortcut} failed: {result.stderr.strip()[:200]}")

    def app_by_pid(self, pid: int):
        pyatspi = atspi()
        pump()
        desktop = pyatspi.Registry.getDesktop(0)
        for index in range(desktop.childCount):
            try:
                app = desktop.getChildAtIndex(index)
                if app is not None and app.get_process_id() == pid:
                    return app
            except Exception:
                continue
        return None

    def frames_by_pid(self, pid: int) -> list:
        app = self.app_by_pid(pid)
        if app is None:
            return []
        out = []
        for index in range(app.childCount):
            try:
                child = app.getChildAtIndex(index)
            except Exception:
                continue
            if child is not None:
                out.append(child)
        return out

    def close(self) -> None:
        for process in self.children:
            try:
                process.terminate()
            except Exception:
                pass
        for process in self.children:
            try:
                process.wait(5)
            except Exception:
                try:
                    process.kill()
                except Exception:
                    pass


# --------------------------------------------------------------------------
# Probe runners
# --------------------------------------------------------------------------


def find_showing(frames: list, target_name: str, roles: set[str]) -> Optional[Any]:
    pyatspi = atspi()
    for frame in frames:
        for node in descendants(frame, limit=4000):
            if role(node) in roles and name(node) == target_name and has_state(node, pyatspi.STATE_SHOWING):
                return node
    return None


def any_showing(frames: list, roles: set[str]) -> bool:
    pyatspi = atspi()
    for frame in frames:
        for node in descendants(frame, limit=4000):
            if role(node) in roles and has_state(node, pyatspi.STATE_SHOWING):
                return True
    return False


def click_node(shell: ShellSession, node) -> None:
    """Activate a node through the accessibility API's own default action,
    not a synthetic pointer click at its extents - the same technique
    run_terminal_close.py uses for the real `rmac-terminal` close button,
    and the one that actually works here: a wlinput pointer click at the
    node's WINDOW_COORDS extents (the convention run_lulo.py's extents()
    uses, meant to be added to a niri window's own origin) does not open a
    menu on a layer-shell surface like the top bar live-tested against the
    installed build, 2026-10-02)."""

    try:
        action = node.queryAction()
    except Exception as error:
        raise StepFailed(f"{name(node)!r} has no accessible action") from error
    action.doAction(0)


def run_menu_surface(shell: ShellSession, item: dict[str, Any]) -> dict[str, Any]:
    lulo = item["lulo"]
    label = lulo["label"]
    neighbor_label = lulo.get("neighbor_label")
    out: dict[str, Any] = {}
    frames = shell.frames_by_pid(shell.top_bar_process.pid)
    if not frames:
        raise StepFailed("the top bar exposed no accessible frame over AT-SPI")

    menu_roles = {"menu item", "check menu item", "radio menu item"}
    # The item that opened whichever menu is currently showing (if any), so
    # a fallback close can toggle exactly that item - needed because this
    # probe's own job is to find out whether outside-click/Escape close the
    # menu, so neither can be assumed to work as the *cleanup* mechanism
    # between probes.
    state: dict[str, Optional[str]] = {"open_label": None}

    def bar_item(target_label: str):
        for frame in frames:
            for node in descendants(frame, limit=2000):
                if role(node) in {"menu", "menu bar", "push button", "button", "label"} and name(node) == target_label:
                    return node
        return None

    def open_menu(target_label: str) -> None:
        target = bar_item(target_label)
        if target is None:
            raise StepFailed(f"no top-bar item named {target_label!r}")
        for _attempt in range(3):
            click_node(shell, target)
            time.sleep(0.6)
            if any_showing(frames, menu_roles):
                state["open_label"] = target_label
                return
            target = bar_item(target_label) or target
        dump_tree(frames, f"open_menu({target_label!r}) failed; top-bar tree at the last attempt")
        raise StepFailed(f"{target_label!r} menu did not open")

    def close_safety_net() -> None:
        for _ in range(2):
            if not any_showing(frames, menu_roles):
                state["open_label"] = None
                return
            shell.input.key("escape")
            time.sleep(0.3)
        # Escape alone is exactly what this probe may find broken; fall
        # back to toggling the item that opened it closed again (proven by
        # the reopen_same_title probe: clicking an already-open item's
        # trigger closes it) so the next probe still starts from closed.
        if state["open_label"] is not None:
            target = bar_item(state["open_label"])
            if target is not None:
                click_node(shell, target)
                time.sleep(0.4)
        state["open_label"] = None

    try:
        open_menu(label)
        shell.click(OUTPUT_W // 2, OUTPUT_H // 2)
        time.sleep(0.4)
        out["outside_click"] = {"closed": not any_showing(frames, menu_roles)}
        close_safety_net()

        open_menu(label)
        shell.input.key("escape")
        time.sleep(0.4)
        out["escape"] = {"closed": not any_showing(frames, menu_roles)}
        close_safety_net()

        open_menu(label)
        target = bar_item(label)
        click_node(shell, target)
        time.sleep(0.4)
        closed = not any_showing(frames, menu_roles)
        out["reopen_same_title"] = {"closed": closed}
        if closed:
            state["open_label"] = None
        close_safety_net()

        if neighbor_label:
            open_menu(label)
            neighbor = bar_item(neighbor_label)
            if neighbor is None:
                out["switch_neighbor"] = {"switched": None}
            else:
                click_node(shell, neighbor)
                time.sleep(0.4)
                switched = any_showing(frames, menu_roles)
                out["switch_neighbor"] = {"switched": switched}
                if switched:
                    state["open_label"] = neighbor_label
            close_safety_net()
    finally:
        close_safety_net()
    return out


def run_popover_surface(shell: ShellSession, item: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    shell.start_quick_settings()
    # Quick Settings starts hidden and (live-verified) registers no AT-SPI
    # application at all until the *first* dispatch actually shows it, so
    # frames cannot be fetched once up front the way the top bar's can.
    opened_once = False

    def current_frames() -> list:
        return shell.frames_by_pid(shell.quick_settings_process.pid)

    def is_open() -> bool:
        pyatspi = atspi()
        for frame in current_frames():
            if has_state(frame, pyatspi.STATE_SHOWING) or has_state(frame, pyatspi.STATE_VISIBLE):
                return True
        return False

    def open_popover() -> None:
        nonlocal opened_once
        if is_open():
            return
        shell.dispatch("quick-settings")
        timeout = 25.0 if not opened_once else 5.0
        if not shell._wait_for(is_open, timeout):
            raise StepFailed("Quick Settings did not open from its shortcut endpoint")
        if not opened_once and not shell.wait_for_populated_frame(shell.quick_settings_process.pid, timeout=25):
            raise StepFailed("Quick Settings opened but its AT-SPI tree stayed empty")
        if not opened_once:
            from assert_control_centre_accessibility import assert_tree
            def populated_tree():
                try:
                    return assert_tree(shell.app_by_pid(shell.quick_settings_process.pid))
                except AssertionError:
                    return None
            count = shell._wait_for(populated_tree, 10)
            if count is None:
                try:
                    assert_tree(shell.app_by_pid(shell.quick_settings_process.pid))
                except AssertionError as error:
                    raise StepFailed(f"Control Centre accessibility assertion failed: {error}") from error
            print(f"Control Centre: {count} accessible nodes", flush=True)
            def painted_panel():
                shot = capture_full(shell.env, shell.nested.work / "panel-ready.png")
                low, high = shot.crop((OUTPUT_W - 320, 28, OUTPUT_W - 2, 380)).convert("L").getextrema()
                return high - low > 40
            if not shell._wait_for(painted_panel, 8):
                raise StepFailed("Quick Settings exposed controls but did not paint its panel")
        opened_once = True

    def close_safety_net() -> None:
        for _ in range(2):
            if not is_open():
                return
            shell.input.key("escape")
            time.sleep(0.4)
            if is_open():
                shell.dispatch("quick-settings")
                time.sleep(0.4)

    try:
        for control in item.get("hover_controls", []):
            label = control["label"]
            open_popover()
            target = shell._wait_for(
                lambda: find_showing(current_frames(), control["lulo_name"], {"slider"}), 20)
            if target is None:
                out[f"hover:{label}"] = {"changed": None, "reason": "slider not present"}
                continue
            if "focusable" not in support.states(target):
                out[f"hover:{label}"] = {"changed": None, "reason": "slider unavailable"}
                continue
            # SCREEN_COORDS normally locate the layer surface on niri's
            # output. Some AT-SPI adapters return window-relative coordinates
            # there too; the panel is anchored at the measured 316 pt width,
            # 28 pt top and 2 pt right margins (surface.rs/layout.rs).
            try:
                rect = target.queryComponent().getExtents(atspi().SCREEN_COORDS)
                box = (rect.x, rect.y, rect.width, rect.height)
            except Exception:
                box = None
            if box is None or box[0] < OUTPUT_W // 2:
                local = extents(target)
                box = ((OUTPUT_W - 316 - 2 + local[0], 28 + local[1], local[2], local[3])
                       if local else None)
            if box is None or box[2] <= 0 or box[3] <= 0:
                out[f"hover:{label}"] = {"changed": None, "reason": "slider bounds unavailable"}
                continue
            shell.move(OUTPUT_W // 2, OUTPUT_H // 2)
            time.sleep(0.3)
            origin_x, origin_y = shell.niri_origin
            region = (max(0, box[0] + origin_x - 14),
                      max(0, box[1] + origin_y - 14), box[2] + 28, box[3] + 28)
            rest = capture_full(shell.env, shell.nested.work / f"hover-rest-{label}.png")
            cx, cy = box[0] + box[2] / 2, box[1] + box[3] / 2
            shell.move(cx - 20, cy)
            time.sleep(0.1)
            shell.move(cx, cy)
            time.sleep(0.5)
            hovered = capture_full(shell.env, shell.nested.work / f"hover-on-{label}.png")
            out[f"hover:{label}"] = {"changed": region_changed(rest, hovered, region)}
        open_popover()
        from assert_control_centre_accessibility import SCENARIO, assert_detail
        trigger = shell._wait_for(
            lambda: find_showing(current_frames(), SCENARIO["detail_trigger"], {"push button", "button"}), 5)
        if trigger is None:
            dump_tree(current_frames(), "Control Centre before Wi-Fi AT-SPI click")
            raise StepFailed("Wi-Fi detail button is missing from the AT-SPI tree")
        click_node(shell, trigger)
        if not shell._wait_for(lambda: find_showing(current_frames(), SCENARIO["detail_panel"], {"panel", "group"}), 5):
            dump_tree(current_frames(), "Control Centre after Wi-Fi AT-SPI click")
            raise StepFailed("Wi-Fi detail view did not open through AT-SPI")
        def populated_detail():
            try:
                return assert_detail(shell.app_by_pid(shell.quick_settings_process.pid))
            except AssertionError:
                return None
        detail_count = shell._wait_for(populated_detail, 5)
        if detail_count is None:
            try:
                assert_detail(shell.app_by_pid(shell.quick_settings_process.pid))
            except AssertionError as error:
                raise StepFailed(f"Control Centre detail assertion failed: {error}") from error
        print(f"Control Centre Wi-Fi detail: {detail_count} accessible nodes", flush=True)
        close_safety_net()
        open_popover()
        shell.click(OUTPUT_W // 2, OUTPUT_H - 100)
        time.sleep(0.8)
        out["outside_click"] = {"closed": not is_open()}
        close_safety_net()

        # Live-verified flaky across repeated runs even with this 0.8s
        # settle (roughly 2 of 3 trials: still open; the rest: closed) -
        # recorded as-is rather than retried into a single answer, since
        # the flakiness itself (Escape not reliably dismissing Control
        # Centre) is worth surfacing, not hiding.
        open_popover()
        shell.input.key("escape")
        time.sleep(0.8)
        out["escape"] = {"closed": not is_open()}
        close_safety_net()

    finally:
        close_safety_net()
    return out


def capture_full(env: dict[str, str], destination: Path):
    from PIL import Image

    result = subprocess.run(["grim", str(destination)], env=env, capture_output=True, text=True, timeout=15)
    if result.returncode != 0:
        raise StepFailed(f"grim failed: {result.stderr[-300:]}")
    try:
        return Image.open(destination).convert("RGB")
    finally:
        destination.unlink(missing_ok=True)


def region_changed(before, after, box: tuple[float, float, float, float], threshold: int = 40) -> bool:
    from PIL import ImageChops

    if before.size != after.size:
        return True
    crop_box = (round(box[0]), round(box[1]), round(box[0] + box[2]), round(box[1] + box[3]))
    difference = ImageChops.difference(before.crop(crop_box), after.crop(crop_box)).convert("L")
    changed_pixels = sum(difference.histogram()[25:])
    return changed_pixels > threshold


RUNNERS = {
    "menu": run_menu_surface,
    "popover": run_popover_surface,
}


def desktop_app_count() -> int:
    pyatspi = atspi()
    pump()
    return pyatspi.Registry.getDesktop(0).childCount


def dump_tree(frames: list, label: str) -> None:
    pyatspi = atspi()
    print(f"== {label}: {len(frames)} top-level frame(s)")
    for frame in frames:
        for node in descendants(frame, limit=1500):
            depth = 0
            parent = node
            while parent is not None and parent is not frame and depth < 30:
                try:
                    parent = parent.parent
                except Exception:
                    break
                depth += 1
            states = [s for s, flag in (("showing", pyatspi.STATE_SHOWING), ("visible", pyatspi.STATE_VISIBLE),
                                         ("focused", pyatspi.STATE_FOCUSED), ("checked", pyatspi.STATE_CHECKED))
                      if has_state(node, flag)]
            print(f"{'  ' * depth}{role(node)} {name(node)!r}"
                  f"{' ' + ','.join(states) if states else ''} extents={extents(node)}")


def explore(shell: ShellSession, items: list[dict[str, Any]]) -> None:
    for item in items:
        lulo = item["lulo"]
        if item["kind"] == "menu":
            frames = shell.frames_by_pid(shell.top_bar_process.pid)
            dump_tree(frames, f"{item['id']} (top bar, before opening)")
            target = None
            for frame in frames:
                for node in descendants(frame, limit=2000):
                    if name(node) == lulo["label"]:
                        target = node
                        break
            if target is not None:
                click_node(shell, target)
                time.sleep(0.6)
                dump_tree(frames, f"{item['id']} (top bar, after clicking {lulo['label']!r})")
            else:
                print(f"{item['id']}: no node named {lulo['label']!r} found before opening")
            shell.input.key("escape")
            time.sleep(0.3)
        elif item["kind"] == "popover":
            shell.start_quick_settings()
            frames = shell.frames_by_pid(shell.quick_settings_process.pid)
            print(f"{item['id']}: {len(frames)} frame(s) before any dispatch "
                  f"(desktop has {desktop_app_count()} app(s) registered)")
            shell.dispatch(lulo["shortcut"])
            populated = shell.wait_for_populated_frame(shell.quick_settings_process.pid, timeout=25)
            frames = shell.frames_by_pid(shell.quick_settings_process.pid)
            dump_tree(frames, f"{item['id']} (after dispatch, populated={populated}, "
                              f"desktop has {desktop_app_count()} app(s) registered)")


def record_shell_surfaces(nested: "run_lulo.Nested", bins: list[Path], niri_bin: Optional[Path],
                          items: list[dict[str, Any]], assert_notification_center: bool = False) -> list[dict[str, Any]]:
    results = []
    shell = ShellSession(nested, bins, niri_bin)
    try:
        shell.start()
        for item in items:
            runner = RUNNERS[item["kind"]]
            result: dict[str, Any] = {"format": 1, "surface": item["id"], "platform": "Lulo",
                                      "recorded": datetime.date.today().isoformat(), "probes": {}}
            try:
                result["probes"] = runner(shell, item)
            except (StepFailed, Unsupported, wlinput.InjectorError) as error:
                result["error"] = str(error)
            results.append(result)
            # Preserve completed surface measurements if a later, separate
            # panel assertion fails in this same private session.
            OUT_DIR.mkdir(parents=True, exist_ok=True)
            (OUT_DIR / f"{result['surface']}.json").write_text(json.dumps(result, indent=2) + "\n")
            print(f"{'ERROR' if 'error' in result else 'ok   '} {item['id']}: "
                  f"{result.get('error') or json.dumps(result['probes'])}", flush=True)
        if assert_notification_center:
            shell.start_notification_center()
            shell.dispatch("notification-center")
            if not shell.wait_for_populated_frame(shell.notification_center_process.pid, timeout=25):
                raise StepFailed("Notification Center opened but its AT-SPI tree stayed empty")
            from assert_notification_center_accessibility import assert_tree
            def populated_tree():
                try:
                    return assert_tree(shell.app_by_pid(shell.notification_center_process.pid))
                except AssertionError:
                    return None
            count = shell._wait_for(populated_tree, 10)
            if count is None:
                try:
                    assert_tree(shell.app_by_pid(shell.notification_center_process.pid))
                except AssertionError as error:
                    raise StepFailed(f"Notification Center accessibility assertion failed: {error}") from error
            print(f"Notification Center: {count} accessible nodes", flush=True)
    finally:
        shell.close()
    return results


def inner(args: argparse.Namespace) -> int:
    work = Path(args.inner)
    nested = run_lulo.Nested(work)
    bins = [Path(p) for p in args.bin_dir] + [Path(p) for p in args.shell_bin_dir]
    niri_bin = Path(args.niri) if args.niri else None
    wanted = [item for item in sf.SURFACES if item["status"] == "automated"
              and item.get("lulo", {}).get("harness") == "shell"]
    if not args.all:
        chosen = set(args.surfaces)
        wanted = [item for item in wanted if item["id"] in chosen]
    if args.explore:
        wanted_all = [item for item in sf.SURFACES if item.get("lulo", {}).get("harness") == "shell"
                      and (args.all or item["id"] in set(args.surfaces))]
        shell = ShellSession(nested, bins, niri_bin)
        try:
            shell.start()
            explore(shell, wanted_all)
        finally:
            shell.close()
            nested.close()
        return 0
    results = []
    try:
        if wanted:
            results.extend(record_shell_surfaces(nested, bins, niri_bin, wanted,
                                                 args.assert_notification_center))
    finally:
        nested.close()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for result in results:
        (OUT_DIR / f"{result['surface']}.json").write_text(json.dumps(result, indent=2) + "\n")
    failures = sum("error" in r for r in results)
    return 1 if failures else 0


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("surfaces", nargs="*")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--bin-dir", action="append", default=[])
    parser.add_argument("--shell-bin-dir", action="append", default=[])
    parser.add_argument("--niri", default=None, help="path to niri (default: $PATH)")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--explore", action="store_true", help="dump the AT-SPI tree instead of probing")
    parser.add_argument("--assert-notification-center", action="store_true",
                        help="also assert Notification Center's populated AT-SPI tree")
    parser.add_argument("--inner", default=None, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    if not args.all and not args.surfaces:
        parser.error("name surfaces or pass --all")
    args.bin_dir = [str(Path(p).resolve()) for p in args.bin_dir]
    args.shell_bin_dir = [str(Path(p).resolve()) for p in args.shell_bin_dir]
    rebuilt = ["--bin-dir=" + d for d in args.bin_dir] + ["--shell-bin-dir=" + d for d in args.shell_bin_dir]
    if args.niri:
        rebuilt += ["--niri", args.niri]
    if args.keep:
        rebuilt.append("--keep")
    if args.explore:
        rebuilt.append("--explore")
    if args.assert_notification_center:
        rebuilt.append("--assert-notification-center")
    if args.all:
        rebuilt.append("--all")
    rebuilt += args.surfaces
    return outer(args, rebuilt)


if __name__ == "__main__":
    sys.exit(main())
