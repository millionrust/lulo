#!/usr/bin/env python3
"""Play behaviour scenarios on Lulo inside a nested, headless compositor.

    python3 scripts/behavior/run_lulo.py --bin-dir DIR [--shell-bin-dir DIR] [SCENARIO…]

Every scenario with a recorded <name>.mac.json runs against the Lulo apps in
--bin-dir (rmac-files, rmac-text-editor, rmac-system-settings,
rmac-calculator; the desktop is the shell's `wallpaper` binary). Results go
to --output (JSON) and a readable report goes to stdout; see compare.py.

Isolation (docs/behavior-suite.md):
  * the runner re-executes itself under `dbus-run-session`, so the apps, the
    AT-SPI bus and gsettings (memory backend) are private to the run;
  * it starts its own headless Sway with a fresh XDG_RUNTIME_DIR, and HOME
    and every XDG_* directory point into a temporary directory;
  * input goes only to that Sway, through the virtual keyboard and pointer
    in wlinput.py, which refuses WAYLAND_DISPLAY=wayland-1 and any
    /run/user/* runtime directory;
  * one app instance at a time, killed by PID and waited for.
"""

from __future__ import annotations

import argparse
import errno
import json
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import scenario as sc  # noqa: E402
import wlinput  # noqa: E402

OUTPUT_W, OUTPUT_H = 1280, 800
APP_BINARIES = {
    "files": ["rmac-files"],
    "text-editor": ["rmac-text-editor"],
    "settings": ["rmac-system-settings"],
    "calculator": ["rmac-calculator"],
    "desktop": ["rmac-wallpaper", "wallpaper"],
    "preview": ["rmac-preview"],
    "terminal": ["rmac-terminal"],
}
KEEP_ENV = {"PATH", "LANG", "TERM", "USER", "LOGNAME", "SHELL", "CARGO_TARGET_DIR", "RUST_BACKTRACE", "RUST_LOG"}
TEXT_ROLES = {"text-field", "text-area", "search-field", "combo-box"}
DIALOG_ROLES = {"dialog", "alert", "file chooser"}
HELPER_APPS = {"rmac-file-chooser"}


def calculator_visible_size(width: Optional[int], height: Optional[int]) -> tuple[Optional[int], Optional[int]]:
    """Account for Sway versions that report GPUI's 12 px client frame."""

    if width in (254, 698):
        width -= 24
    if height in (430, 432):
        height -= 24
    return width, height


def find_file_chooser_binary(directories: list[Path]) -> Optional[Path]:
    """Find the portal backend among the app and helper binary directories."""

    for directory in directories:
        candidate = directory / "rmac-file-chooser"
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate.resolve()
    return None


def empty_viewport_point(box: tuple[int, int, int, int], origin: tuple[int, int]) -> tuple[int, int]:
    """Choose an inset point at the bottom-right of an accessible viewport."""
    x, y, width, height = box
    if width <= 48 or height <= 48:
        raise StepFailed("Files list viewport is too small to context-click safely")
    return origin[0] + x + width - 24, origin[1] + y + height - 24


def content_viewport(candidates: list[tuple[int, int, int, int]]) -> tuple[int, int, int, int]:
    """Choose the inner folder list, excluding the sidebar and status bar."""
    wide = [box for box in candidates if box[2] >= OUTPUT_W // 2]
    if not wide:
        raise StepFailed("no on-screen Files content viewport to context-click")
    return min(wide, key=lambda box: box[2] * box[3])


class Unsupported(RuntimeError):
    pass


class StepFailed(RuntimeError):
    pass


# --------------------------------------------------------------------------
# Outer process: build the isolated environment, then re-run inside it.
# --------------------------------------------------------------------------


def isolated_environment(work: Path) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if k in KEEP_ENV}
    home = work / "home"
    runtime = work / "runtime"
    for path in (home, runtime, home / ".config", home / ".local/share", home / ".local/state",
                 home / ".cache", home / "Desktop", home / "Documents"):
        path.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    (home / ".config/user-dirs.dirs").write_text(
        'XDG_DESKTOP_DIR="$HOME/Desktop"\nXDG_DOCUMENTS_DIR="$HOME/Documents"\n'
    )
    env.update({
        "HOME": str(home),
        "XDG_RUNTIME_DIR": str(runtime),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_DATA_HOME": str(home / ".local/share"),
        "XDG_STATE_HOME": str(home / ".local/state"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "XDG_SESSION_TYPE": "wayland",
        "XDG_CURRENT_DESKTOP": "rmac:niri",  # as rmac-wayland-session sets it
        # The Mac reference is en-GB (docs/parity.md), so Lulo runs in en-GB.
        "LANG": "en_GB.UTF-8",
        "GSETTINGS_BACKEND": "memory",
        "WLR_BACKENDS": "headless",
        "WLR_HEADLESS_OUTPUTS": "1",
        "WLR_LIBINPUT_NO_DEVICES": "1",
        "WLR_RENDERER": "pixman",
        "LIBGL_ALWAYS_SOFTWARE": "1",
        "RMAC_BEHAVIOR_NESTED": "1",
    })
    for icd in sorted(Path("/usr/share/vulkan/icd.d").glob("*lvp*.json")):
        env["VK_ICD_FILENAMES"] = str(icd)
        break
    return env


def refuse_live_session(environ: dict[str, str]) -> None:
    """The hard guard: the inner runner never starts in the live session."""

    runtime = environ.get("XDG_RUNTIME_DIR", "")
    if environ.get("WAYLAND_DISPLAY") == "wayland-1" or runtime.startswith("/run/user/"):
        raise SystemExit("refusing to run: this environment is the live session (wayland-1 or /run/user)")


def reap(runtime: Path) -> list[int]:
    """Stop every process still using this run's runtime dir: D-Bus services
    the private bus activated (rmac-focus-service, rmac-notification-center,
    portals) outlive dbus-run-session, and each holds a system-bus
    connection."""

    needle = f"XDG_RUNTIME_DIR={runtime}".encode()
    killed = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            environ = (entry / "environ").read_bytes().split(b"\0")
        except OSError:
            continue
        if needle in environ:
            try:
                os.kill(int(entry.name), signal.SIGTERM)
                killed.append(int(entry.name))
            except OSError:
                pass
    return killed


def remove_tree(path: Path) -> None:
    # The document portal leaves read-only directories in the runtime dir.
    for root, dirs, _files in os.walk(path):
        for name in dirs:
            try:
                os.chmod(os.path.join(root, name), 0o700)
            except OSError:
                pass
    shutil.rmtree(path, ignore_errors=True)


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    import fcntl

    for tool in ("sway", "swaymsg", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-behavior-"))
    try:
        binary_directories = [Path(p) for p in args.bin_dir + args.shell_bin_dir]
        chooser = find_file_chooser_binary(binary_directories)
        env = isolated_environment(work)
        refuse_live_session(env)
        # A session bus that can activate only the AT-SPI bus launcher: the
        # installed rmac services (focus, notifications) and portals stay
        # out of the run, so nothing it starts holds a system-bus connection.
        services = work / "dbus-services"
        services.mkdir()
        for name in ("org.a11y.Bus.service", "org.freedesktop.portal.Desktop.service"):
            source = Path("/usr/share/dbus-1/services") / name
            if source.exists():
                shutil.copy(source, services / name)
        # Save and Open panels: xdg-desktop-portal with only the branch's own
        # rmac-file-chooser behind it (GPUI asks the portal for them).
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
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable, str(Path(__file__).resolve()), "--inner", str(work), *argv]
        # The private bus daemon and the services it activates are chatty on
        # stderr; the inner runner reports on stdout.
        (work / "logs").mkdir(exist_ok=True)
        with open(work / "logs" / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if status not in (0, 1):
            print((work / "logs" / "session.log").read_text()[-3000:], file=sys.stderr)
        return status
    finally:
        if reap(work / "runtime"):
            time.sleep(1.0)
            reap(work / "runtime")
        if not args.keep:
            remove_tree(work)
        else:
            print(f"kept {work}", file=sys.stderr)
        journey_lock.close()


# --------------------------------------------------------------------------
# Inner process: Sway, AT-SPI, one scenario at a time.
# --------------------------------------------------------------------------


class Nested:
    def __init__(self, work: Path) -> None:
        self.work = work
        self.env = dict(os.environ)
        refuse_live_session(self.env)
        self.logs = work / "logs"
        self.logs.mkdir(exist_ok=True)
        config = work / "sway.conf"
        config.write_text(
            "xwayland disable\n"
            "default_border none\n"
            "default_floating_border none\n"
            'for_window [app_id="org.rmac.Calculator"] floating enable\n'
            f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H} position 0 0\n"
            "seat seat0 fallback true\n"
            "focus_follows_mouse no\n"
        )
        # Hold wayland-0/1's lock files so Sway's automatic socket name is
        # never "wayland-1": the injector refuses that name outright, as it
        # is the live session's.
        import fcntl

        self.name_locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(Path(self.env["XDG_RUNTIME_DIR"]) / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.name_locks.append(handle)
        self.sway = subprocess.Popen(
            ["sway", "--unsupported-gpu", "--config", str(config)],
            stdout=open(self.logs / "sway.log", "w"), stderr=subprocess.STDOUT, env=self.env, close_fds=True,
        )
        try:
            self._connect()
        except BaseException:
            self.sway.kill()
            self.sway.wait(5)
            raise

    def _connect(self) -> None:
        runtime = Path(self.env["XDG_RUNTIME_DIR"])
        deadline = time.monotonic() + 20
        display = ipc = None
        while time.monotonic() < deadline:
            display = next((p for p in runtime.glob("wayland-*") if not p.name.endswith(".lock")), None)
            ipc = next(iter(runtime.glob("sway-ipc.*.sock")), None)
            if display and ipc:
                break
            if self.sway.poll() is not None:
                raise SystemExit("sway exited early; see its log with --keep")
            time.sleep(0.1)
        if not (display and ipc):
            raise SystemExit("sway did not create its sockets")
        self.env["WAYLAND_DISPLAY"] = display.name
        self.env["SWAYSOCK"] = str(ipc)
        os.environ.update({"WAYLAND_DISPLAY": display.name, "SWAYSOCK": str(ipc)})
        wlinput.assert_nested(self.env)
        # Services the private bus starts later (the portal's file chooser)
        # must reach this Sway too.
        subprocess.run(
            ["busctl", "--user", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
             "UpdateActivationEnvironment", "a{ss}", "2", "WAYLAND_DISPLAY", display.name, "SWAYSOCK", str(ipc)],
            env=self.env, check=False, capture_output=True, timeout=10,
        )
        self.input = wlinput.Wayland(self.env)
        self.enable_accessibility()
        import pyatspi  # noqa: F401  (imported only on the private bus)

    def enable_accessibility(self) -> None:
        # Private session bus: flipping IsEnabled here reaches only this run's
        # at-spi-bus-launcher. AccessKit registers each window when it sees it.
        subprocess.run(
            ["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "IsEnabled", "b", "true"],
            env=self.env, check=False, capture_output=True, timeout=10,
        )

    def swaymsg(self, *args: str) -> Any:
        out = subprocess.run(["swaymsg", "-r", *args], env=self.env, capture_output=True, text=True, timeout=10)
        return json.loads(out.stdout) if out.stdout.strip() else None

    def windows(self) -> list[dict[str, Any]]:
        found = []

        def walk(node: dict[str, Any]) -> None:
            if node.get("pid") and node.get("type") in {"con", "floating_con"}:
                found.append(node)
            for child in node.get("nodes", []) + node.get("floating_nodes", []):
                walk(child)

        tree = self.swaymsg("-t", "get_tree")
        if tree:
            walk(tree)
        return found

    def close(self) -> None:
        try:
            self.input.close()
        except Exception:
            pass
        self.sway.terminate()
        try:
            self.sway.wait(5)
        except subprocess.TimeoutExpired:
            self.sway.kill()


# ---- AT-SPI helpers --------------------------------------------------------


def atspi():
    import pyatspi

    return pyatspi


def pump() -> None:
    """Let libatspi process pending D-Bus signals (new apps, children and
    state changes); without a main loop its cache goes stale."""

    from gi.repository import GLib

    context = GLib.MainContext.default()
    for _ in range(200):
        if not context.iteration(False):
            break


def descendants(node, limit: int = 4000, depth: int = 40):
    stack = [(node, 0)]
    seen = 0
    while stack and seen < limit:
        current, level = stack.pop()
        if current is None:
            continue
        seen += 1
        yield current
        if level >= depth:
            continue
        try:
            count = current.childCount
            children = [current.getChildAtIndex(i) for i in range(count)]
        except Exception:
            continue
        for child in reversed(children):
            stack.append((child, level + 1))


def role(node) -> str:
    try:
        return node.getRoleName()
    except Exception:
        return ""


def name(node) -> str:
    try:
        return node.name or ""
    except Exception:
        return ""


def has_state(node, state) -> bool:
    try:
        return node.getState().contains(state)
    except Exception:
        return False


def text_of(node) -> tuple[Optional[str], Optional[int], Optional[int]]:
    try:
        text = node.queryText()
    except Exception:
        return None, None, None
    try:
        value = text.getText(0, -1)
    except Exception:
        value = None
    start = end = None
    try:
        if text.getNSelections() > 0:
            start, end = text.getSelection(0)
        else:
            start = end = text.caretOffset
    except Exception:
        pass
    return value, start, end


def extents(node) -> Optional[tuple[int, int, int, int]]:
    pyatspi = atspi()
    try:
        box = node.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
        return box.x, box.y, box.width, box.height
    except Exception:
        return None


def png_pixel_rgb(path: Path, x: int, y: int) -> tuple[int, int, int]:
    """Read one 8-bit RGB/RGBA PNG pixel without a third-party image package."""
    data = path.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise StepFailed(f"{path} is not a PNG screenshot")
    pos = 8
    compressed = bytearray()
    width = height = bit_depth = color_type = interlace = None
    while pos + 12 <= len(data):
        length = struct.unpack_from(">I", data, pos)[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, bit_depth, color_type, _compression, _filter, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
        elif kind == b"IDAT":
            compressed.extend(chunk)
        elif kind == b"IEND":
            break
    if bit_depth != 8 or color_type not in {2, 6} or interlace != 0:
        raise StepFailed(
            f"unsupported PNG format: depth={bit_depth}, type={color_type}, interlace={interlace}"
        )
    if width is None or height is None or not (0 <= x < width and 0 <= y < height):
        raise StepFailed(f"screenshot pixel ({x}, {y}) is outside {width}x{height}")
    channels = 4 if color_type == 6 else 3
    row_bytes = width * channels
    raw = zlib.decompress(compressed)
    previous = bytearray(row_bytes)
    offset = 0
    for row_index in range(height):
        filter_type = raw[offset]
        offset += 1
        row = bytearray(raw[offset : offset + row_bytes])
        offset += row_bytes
        for index in range(row_bytes):
            left = row[index - channels] if index >= channels else 0
            above = previous[index]
            upper_left = previous[index - channels] if index >= channels else 0
            if filter_type == 1:
                row[index] = (row[index] + left) & 0xFF
            elif filter_type == 2:
                row[index] = (row[index] + above) & 0xFF
            elif filter_type == 3:
                row[index] = (row[index] + ((left + above) // 2)) & 0xFF
            elif filter_type == 4:
                estimate = left + above - upper_left
                distances = (
                    abs(estimate - left),
                    abs(estimate - above),
                    abs(estimate - upper_left),
                )
                predictor = (
                    left if distances[0] <= distances[1] and distances[0] <= distances[2]
                    else above if distances[1] <= distances[2]
                    else upper_left
                )
                row[index] = (row[index] + predictor) & 0xFF
            elif filter_type != 0:
                raise StepFailed(f"unsupported PNG row filter: {filter_type}")
        if row_index == y:
            start = x * channels
            return tuple(row[start : start + 3])
        previous = row
    raise StepFailed(f"screenshot row {y} is missing")


class LuloRun:
    def __init__(self, nested: Nested, sid: str, scenario: dict[str, Any], bins: list[Path], settle: float,
                 capture_dir: Optional[Path] = None) -> None:
        self.nested = nested
        self.sid = sid
        self.scenario = scenario
        self.app = scenario["app"]
        self.settle = settle
        self.bins = bins
        self.capture_dir = capture_dir
        # A fresh home per scenario: apps keep state (Text Editor's unsaved
        # work, Files' window state) that must not leak into the next one.
        home = nested.work / "homes" / sid.replace("/", "-")
        if home.exists():
            shutil.rmtree(home)
        for sub in (".config", ".local/share", ".local/state", ".cache", "Desktop", "Documents"):
            (home / sub).mkdir(parents=True, exist_ok=True)
        (home / ".config/user-dirs.dirs").write_text(
            'XDG_DESKTOP_DIR="$HOME/Desktop"\nXDG_DOCUMENTS_DIR="$HOME/Documents"\n'
        )
        self.env = dict(nested.env)
        self.env.update({
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_STATE_HOME": str(home / ".local/state"),
            "XDG_CACHE_HOME": str(home / ".cache"),
        })
        self.sandbox = home / "lulo-behavior" / sid.replace("/", "-") / "sandbox"
        self.files_root = home / "Desktop" if self.app == "desktop" else self.sandbox
        self.before: set[str] = set()
        self.process: Optional[subprocess.Popen] = None
        self.log = None

    # -- lifecycle ---------------------------------------------------------

    def binary(self) -> Path:
        for directory in self.bins:
            for candidate in APP_BINARIES[self.app]:
                if (directory / candidate).is_file():
                    return directory / candidate
        raise StepFailed(f"no binary for {self.app} in {', '.join(map(str, self.bins))}")

    def setup(self) -> None:
        if self.sandbox.exists():
            shutil.rmtree(self.sandbox)
        self.sandbox.mkdir(parents=True)
        for entry, content in self.scenario.get("setup", {}).get("files", {}).items():
            target = self.sandbox / entry
            if entry.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content or "")
        if self.app == "desktop":
            self.before = {p.name + ("/" if p.is_dir() else "") for p in self.files_root.iterdir()}

    def launch(self) -> None:
        launch = self.scenario.get("launch", {})
        command = [str(self.binary())]
        if self.app == "files":
            if "reveal" in launch:
                command += ["--reveal", str(self.sandbox / launch["reveal"])]
            else:
                command += ["--path", str(self.sandbox / launch.get("folder", "."))]
        elif self.app == "preview":
            command += [str(self.sandbox / launch["file"])]
        self.log = open(self.nested.logs / f"{self.sid.replace('/', '-')}.log", "w")
        self.process = subprocess.Popen(
            command, env=self.env, stdout=self.log, stderr=subprocess.STDOUT, close_fds=True,
            cwd=str(self.sandbox),
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise StepFailed(f"{command[0]} exited with {self.process.returncode}")
            if self.app == "desktop":
                if self.application() is not None:
                    break
            elif any(w.get("pid") == self.process.pid for w in self.nested.windows()) and self.application() is not None:
                break
            time.sleep(0.2)
        else:
            pyatspi = atspi()
            desktop = pyatspi.Registry.getDesktop(0)
            apps = []
            for index in range(desktop.childCount):
                try:
                    app = desktop.getChildAtIndex(index)
                    apps.append((name(app), app.get_process_id()))
                except Exception as error:  # pragma: no cover - diagnostics only
                    apps.append(("?", str(error)))
            windows = [(w.get("name"), w.get("pid")) for w in self.nested.windows()]
            raise StepFailed(
                f"the app (pid {self.process.pid}) showed no window with an accessible tree within 30 s; "
                f"sway windows {windows}, AT-SPI apps {apps}"
            )
        if self.app == "calculator":
            # The Calculator has a fixed, mode-dependent size. Sway's default
            # tiled container fills the whole headless output and obscures
            # its requested size. The pre-launch rule keeps it floating like
            # the real desktop; wait for its Basic-sized surface before the
            # first observation or shortcut.
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                windows = [w for w in self.nested.windows() if w.get("pid") == self.process.pid]
                rect = (windows[0].get("window_rect") or {}) if windows else {}
                visible_width, visible_height = calculator_visible_size(
                    rect.get("width"), rect.get("height")
                )
                if (visible_width is not None and visible_height is not None
                        and 228 <= visible_width <= 232 and 404 <= visible_height <= 410):
                    break
                time.sleep(0.1)
            else:
                raise StepFailed(f"Calculator did not settle to Basic size: {windows}")
        time.sleep(max(self.settle, 1.0))

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(8)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(5)
        if self.log:
            self.log.close()

    # -- AT-SPI ------------------------------------------------------------

    def application(self):
        pyatspi = atspi()
        pump()
        desktop = pyatspi.Registry.getDesktop(0)
        for index in range(desktop.childCount):
            try:
                app = desktop.getChildAtIndex(index)
                if app is not None and app.get_process_id() == self.process.pid:
                    return app
            except Exception:
                continue
        return None

    def frames(self) -> list:
        app = self.application()
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

    def helper_frames(self) -> list:
        """Windows of the portal's file chooser: a Save panel on Lulo is its
        own process, where the Mac shows a sheet on the document window."""

        pyatspi = atspi()
        desktop = pyatspi.Registry.getDesktop(0)
        out = []
        for index in range(desktop.childCount):
            try:
                app = desktop.getChildAtIndex(index)
                if app is None or name(app) not in HELPER_APPS:
                    continue
                out.extend(app.getChildAtIndex(i) for i in range(app.childCount))
            except Exception:
                continue
        return [frame for frame in out if frame is not None]

    def active_frame(self):
        pyatspi = atspi()
        for frame in self.helper_frames():
            if has_state(frame, pyatspi.STATE_ACTIVE):
                return frame
        frames = self.frames()
        for frame in frames:
            if has_state(frame, pyatspi.STATE_ACTIVE):
                return frame
        focused = [w for w in self.nested.windows() if w.get("focused") and w.get("pid") == self.process.pid]
        if focused:
            for frame in frames:
                if name(frame) == focused[0].get("name"):
                    return frame
        return frames[0] if len(frames) == 1 else None

    def focused_node(self):
        pyatspi = atspi()
        frame = self.active_frame()
        roots = [frame] if frame is not None else self.frames()
        best = None
        for root in roots:
            for node in descendants(root):
                if node is not root and has_state(node, pyatspi.STATE_FOCUSED):
                    best = node  # the deepest focused node wins (depth-first order)
        return best

    # -- facts -------------------------------------------------------------

    def fact_focus(self) -> dict[str, Any]:
        node = self.focused_node()
        if node is None:
            return {"role": None, "value": None, **sc.selection_facts(None, None, None), "label": None}
        normalized = sc.normalize_atspi_role(role(node))
        value, start, end = text_of(node) if normalized in TEXT_ROLES else (None, None, None)
        return {"role": normalized, "value": value, **sc.selection_facts(value, start, end), "label": name(node) or None}

    def fact_windows(self) -> dict[str, Any]:
        pyatspi = atspi()
        plain = [f for f in self.frames() if role(f) not in DIALOG_ROLES
                 and (role(f) == "frame" or has_state(f, pyatspi.STATE_SHOWING))]
        active = self.active_frame()
        titles = [sc.lulo_window_title(name(f)) for f in plain]
        front = sc.lulo_window_title(name(active)) if active is not None and role(active) not in DIALOG_ROLES else None
        if front in titles:
            titles.remove(front)
            titles.insert(0, front)
        return {"count": len(plain), "front": front, "titles": titles}

    def fact_info(self) -> dict[str, Any]:
        """Check the accessible Size row in the frontmost Get Info window."""
        frame = self.active_frame()
        if frame is None or not name(frame).endswith(" Info"):
            return {"size_bytes_present": False, "item_count_present": False}
        values = []
        for node in descendants(frame, limit=3000):
            label = name(node)
            if label:
                values.append(label)
            value, _, _ = text_of(node)
            if value:
                values.append(value)
        joined = " ".join(values)
        return {
            "size_bytes_present": bool(re.search(r"\b\d[\d,]*\s+bytes?\b", joined, re.I)),
            "item_count_present": bool(re.search(r"\b\d[\d,]*\s+items?\b", joined, re.I)),
        }

    def fact_window_size(self) -> dict[str, Any]:
        """Visible compositor bounds for runtime-sized calculator windows."""
        windows = [w for w in self.nested.windows() if w.get("pid") == self.process.pid]
        focused = [w for w in windows if w.get("focused")] or windows
        if not focused:
            return {"width": None, "height": None}
        # Sway's xdg geometry can lag a client-driven resize. The compositor's
        # current window rectangle matches the visible pixels in captures.
        rect = focused[0].get("window_rect") or focused[0].get("rect") or {}
        width, height = rect.get("width"), rect.get("height")
        if self.app == "calculator":
            width, height = calculator_visible_size(width, height)
        return {
            "width": width,
            "height": height,
        }

    def dialog_node(self):
        pyatspi = atspi()
        frame = self.active_frame()
        if frame is None:
            return None
        if role(frame) in DIALOG_ROLES or any(frame == helper for helper in self.helper_frames()):
            return frame
        for node in descendants(frame, limit=3000):
            if node is not frame and role(node) in DIALOG_ROLES and has_state(node, pyatspi.STATE_SHOWING):
                return node
        return None

    def fact_dialog(self) -> dict[str, Any]:
        pyatspi = atspi()
        node = self.dialog_node()
        if node is None:
            return {"present": False}
        texts, buttons, default = [], [], None
        focused_button = None
        for child in descendants(node, limit=2000):
            r = role(child)
            if r in {"label", "static", "heading", "paragraph"} and name(child):
                texts.append(name(child))
            elif r in {"push button", "button"} and name(child):
                box = extents(child) or (0, 0, 0, 0)
                buttons.append((round(box[1] / 6), box[0], name(child)))
                if has_state(child, pyatspi.STATE_IS_DEFAULT):
                    default = name(child)
                if has_state(child, pyatspi.STATE_FOCUSED):
                    focused_button = name(child)
        buttons.sort()
        # AccessKit's AT-SPI adapter does not publish IS_DEFAULT. For a
        # one-button alert, its focused button is also its Return action.
        if default is None and role(node) == "alert" and len(buttons) == 1:
            default = focused_button
        sentence = next((t for t in texts if len(t.split()) >= 3), None)
        result = {"present": True, "title": name(node) or sentence, "texts": texts,
                  "buttons": [b[2] for b in buttons]}
        if default is not None:
            result["default"] = default
        return result

    def fact_menu(self) -> dict[str, Any]:
        pyatspi = atspi()
        for frame in self.frames():
            for node in descendants(frame, limit=3000):
                if role(node) in {"menu", "popup menu"} and has_state(node, pyatspi.STATE_SHOWING):
                    items = []
                    for index in range(node.childCount):
                        item = node.getChildAtIndex(index)
                        r = role(item)
                        if r == "separator":
                            items.append("-")
                        elif r.endswith("menu item") or r == "menu":
                            title = name(item)
                            mark = "✓ " if has_state(item, pyatspi.STATE_CHECKED) else ""
                            off = "" if has_state(item, pyatspi.STATE_ENABLED) or has_state(item, pyatspi.STATE_SENSITIVE) else " [disabled]"
                            items.append(f"{mark}{title}{off}")
                    return {"present": True, "items": items}
        return {"present": False}

    def fact_selection(self) -> dict[str, Any]:
        pyatspi = atspi()
        frame = self.active_frame()
        names = []
        for node in descendants(frame, limit=4000) if frame is not None else []:
            if role(node) in {"list item", "table row", "tree item", "table cell"} and has_state(node, pyatspi.STATE_SELECTED):
                label = name(node)
                if not label:
                    for child in descendants(node, limit=20):
                        if child is not node and name(child):
                            label = name(child)
                            break
                if label and label not in names:
                    names.append(label)
        return {"items": names}

    def fact_tabs(self) -> dict[str, Any]:
        frame = self.active_frame()
        tabs = [name(n) for n in descendants(frame, limit=3000) if role(n) == "page tab"] if frame is not None else []
        if not tabs and frame is not None:
            tabs = [name(frame)]
        return {"count": len(tabs), "titles": tabs}

    def fact_display(self) -> dict[str, Any]:
        frame = self.active_frame()
        for node in descendants(frame, limit=2000) if frame is not None else []:
            label = name(node)
            try:
                description = node.description or ""
            except Exception:
                description = ""
            if "display" in (label + " " + description).lower() or "result" in description.lower():
                value, _s, _e = text_of(node)
                if value is None:
                    try:
                        value = node.queryValue().currentValue
                    except Exception:
                        value = None
                if value is not None:
                    return {"value": str(value)}
        return {"value": None}

    def fact_files(self) -> dict[str, Any]:
        entries = []
        root = self.files_root
        for path in sorted(root.rglob("*")):
            rel = path.relative_to(root)
            if any(part.startswith(".") for part in rel.parts):
                continue
            top = rel.parts[0] + ("/" if (root / rel.parts[0]).is_dir() else "")
            if top in self.before:
                continue
            entries.append(rel.as_posix() + ("/" if path.is_dir() else ""))
        return {"entries": entries}

    def fact_saved_documents(self) -> dict[str, Any]:
        """Read the isolated Documents folder after a Text Editor Save sheet."""
        directory = Path(self.env["HOME"]) / "Documents"
        names = sorted(path.name for path in directory.iterdir() if path.is_file())
        contents = {
            name: (directory / name).read_bytes()[:4096].decode("utf-8", "replace")
            for name in names
        }
        return {"entries": names, "contents": contents}

    # -- steps -------------------------------------------------------------

    def ensure_alive(self) -> None:
        if self.process.poll() is not None:
            raise StepFailed(f"the app exited ({self.process.returncode}) during the scenario")

    def window_origin(self, frame=None) -> tuple[int, int]:
        helpers = self.helper_frames() if frame is not None else []
        if any(frame == helper for helper in helpers):
            windows = [w for w in self.nested.windows() if w.get("focused")]
        else:
            windows = [w for w in self.nested.windows() if w.get("pid") == self.process.pid]
        matching = [w for w in windows if w.get("name") == name(frame)] if frame is not None else []
        focused = matching or [w for w in windows if w.get("focused")] or windows
        if not focused:
            return 0, 0
        rect = focused[0]["rect"]
        inner = focused[0].get("window_rect") or {"x": 0, "y": 0}
        return rect["x"] + inner.get("x", 0), rect["y"] + inner.get("y", 0)

    def click_item(self, label: str, button: str, count: int = 1, modifiers: Optional[list[str]] = None) -> None:
        frame = self.active_frame()
        target = None
        # A non-modal utility window can be active while its owning document
        # remains clickable (Finder View Options). Search that document too.
        frames = ([frame] if frame is not None else []) + [
            other for other in self.frames() if other is not frame
        ]
        for candidate in frames:
            for node in descendants(candidate, limit=4000):
                if name(node) == label and role(node) in {"list item", "table row", "tree item", "table cell", "label", "static", "push button", "button", "combo box", "menu item"}:
                    frame, target = candidate, node
                    break
            if target is not None:
                break
        if target is None:
            raise StepFailed(f"no accessible item named {label!r} to click")
        box = extents(target)
        if not box:
            raise StepFailed(f"{label!r} has no on-screen extents")
        ox, oy = self.window_origin(frame)
        x, y = ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2
        self.nested.input.click(x, y, OUTPUT_W, OUTPUT_H, button=button, count=count, modifiers=modifiers)

    def context_background(self) -> None:
        """Right-click an empty point in the Files list viewport."""
        frame = self.active_frame()
        candidates = []
        for node in descendants(frame, limit=4000) if frame is not None else []:
            if role(node) in {"list", "table", "tree", "tree table", "list box"}:
                box = extents(node)
                if box and box[2] > 80 and box[3] > 80:
                    candidates.append(box)
        if not candidates:
            raise StepFailed("no on-screen Files list viewport to context-click")
        box = content_viewport(candidates)
        # The lower-right corner of the viewport is below the listed rows in
        # the fixture and avoids activating a file or folder.
        x, y = empty_viewport_point(box, self.window_origin())
        self.nested.input.click(x, y, OUTPUT_W, OUTPUT_H, button="right")

    def run_steps(self, limit: Optional[int] = None) -> dict[str, Any]:
        observations: dict[str, Any] = {}
        for index, step in enumerate(self.scenario["steps"]):
            if limit is not None and index >= limit:
                break
            self.ensure_alive()
            if "key" in step:
                self.nested.input.key(step["key"])
            elif "type" in step:
                self.nested.input.type_text(step["type"])
            elif "wait" in step:
                time.sleep(float(step["wait"]))
                continue
            elif "select" in step:
                # A plain select (no modifiers/double) sends exactly the
                # same single unmodified left click as before. modifiers
                # (shift/cmd) and double are a real shift-click,
                # command-click or double-click, matching mac_click.py.
                self.click_item(
                    step["select"], "left",
                    count=2 if step.get("double") else 1,
                    modifiers=step.get("modifiers"),
                )
            elif "context" in step:
                if step["context"] == "background":
                    self.context_background()
                else:
                    self.click_item(step["context"], "left")
                    time.sleep(0.3)
                    self.click_item(step["context"], "right")
            elif "focus_desktop" in step:
                self.nested.input.click(OUTPUT_W // 4, OUTPUT_H // 2, OUTPUT_W, OUTPUT_H)
            elif "menu" in step:
                raise Unsupported("menu-bar steps need the top bar, which the nested runner does not start yet")
            elif "click_key" in step:
                # The Mac side clicks by measured grid position (real macOS
                # Calculator exposes no usable accessible name for these
                # keys); Lulo's keys do carry a stable aria-label
                # (`scientific_keypad::key_name`), so click by that instead.
                self.click_item(step["click_key"], "left")
            elif "capture" in step:
                if self.capture_dir is not None:
                    self.capture_dir.mkdir(parents=True, exist_ok=True)
                    destination = self.capture_dir / f"{self.sid.replace('/', '-')}-{step['capture']}.png"
                    subprocess.run(["grim", str(destination)], env=self.env, check=True)
                continue
            elif "observe" in step:
                facts = {}
                for fact in step["facts"]:
                    facts[fact] = getattr(self, f"fact_{fact}")()
                observations[step["observe"]] = sc.finish_observation(self.scenario, step["observe"], facts)
                continue
            time.sleep(float(step.get("settle", self.settle)))
        return observations


def explore(run: LuloRun) -> None:
    """Print the app and portal helper trees (for writing new scenarios)."""

    pyatspi = atspi()
    app = run.application()
    frames = run.frames() + run.helper_frames()
    print(f"== {run.sid}: application {name(app) if app is not None else None!r}, "
          f"{len(frames)} top-level nodes, sway windows "
          f"{[(w.get('name'), w.get('focused')) for w in run.nested.windows()]}")
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
            states = [s for s, flag in (("focused", pyatspi.STATE_FOCUSED), ("selected", pyatspi.STATE_SELECTED),
                                         ("active", pyatspi.STATE_ACTIVE), ("default", pyatspi.STATE_IS_DEFAULT))
                      if has_state(node, flag)]
            value, start, end = text_of(node)
            extra = f" text={value!r}[{start},{end}]" if value is not None else ""
            try:
                description = node.description
            except Exception:
                description = ""
            print(f"{'  ' * depth}{role(node)} {name(node)!r}{' desc=' + repr(description) if description else ''}"
                  f"{' ' + ','.join(states) if states else ''}{extra}")


def check_files_context_submenus(nested: Nested, bins: list[Path], settle: float) -> None:
    """Private nested UI check for real View/Sort By flyout interaction."""
    scenario = {"app": "files", "launch": {"folder": "."}, "steps": []}
    run = LuloRun(nested, "private/context-submenus", scenario, bins, settle, None)
    try:
        run.setup()
        run.launch()

        def visible_menu_items() -> dict[str, Any]:
            pyatspi = atspi()
            frame = run.active_frame()
            return {
                name(node): node
                for node in (descendants(frame, limit=4000) if frame is not None else [])
                if role(node) in {"menu item", "check menu item"}
                and has_state(node, pyatspi.STATE_SHOWING)
            }

        def click_menu_item(label: str) -> None:
            items = visible_menu_items()
            target = items.get(label)
            if target is None:
                raise StepFailed(f"no visible menu item named {label!r}")
            box = extents(target)
            if not box:
                raise StepFailed(f"menu item {label!r} has no on-screen extents")
            ox, oy = run.window_origin()
            x, y = ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2
            nested.input.click(x, y, OUTPUT_W, OUTPUT_H)

        def open_menu(label: str) -> dict[str, Any]:
            run.context_background()
            time.sleep(settle)
            items = visible_menu_items()
            if label not in items:
                raise StepFailed(f"background menu is missing {label!r}; found {sorted(items)}")
            click_menu_item(label)
            time.sleep(settle)
            return visible_menu_items()

        view_items = open_menu("View")
        for label in ("Icons", "List", "Columns", "Gallery"):
            if label not in view_items:
                raise StepFailed(f"View submenu is missing {label!r}; found {sorted(view_items)}")
        print("PASS  View submenu opened with Icons, List, Columns, Gallery", flush=True)
        click_menu_item("Columns")
        time.sleep(settle)

        sort_items = open_menu("Sort By")
        for label in ("Name", "Date Modified", "Size", "Kind"):
            if label not in sort_items:
                raise StepFailed(f"Sort By submenu is missing {label!r}; found {sorted(sort_items)}")
        pyatspi = atspi()
        if not has_state(sort_items["Name"], pyatspi.STATE_CHECKED):
            raise StepFailed("Sort By submenu did not mark the initial Name sort as checked")
        print("PASS  Sort By submenu opened with Name checked", flush=True)
        click_menu_item("Size")
        time.sleep(settle)

        sort_items = open_menu("Sort By")
        if not has_state(sort_items.get("Size"), pyatspi.STATE_CHECKED):
            raise StepFailed("choosing Size did not update the checked sort state")
        if has_state(sort_items.get("Name"), pyatspi.STATE_CHECKED):
            raise StepFailed("Name remained checked after choosing Size")
        print("PASS  choosing Size updates the checked sort state", flush=True)
    finally:
        run.stop()


def capture_preview_markup(nested: Nested, bins: list[Path], settle: float, destination: Path) -> None:
    """Draw and save an annotation in private Sway, then capture its overlay."""
    scenario = sc.load(sc.REPO / "docs" / "behavior-pending" / "preview" / "markup-toggle.json")
    run = LuloRun(nested, "private/preview-markup", scenario, bins, settle, None)
    try:
        run.setup()
        run.launch()
        nested.input.key("shift-cmd-a")
        time.sleep(settle)
        run.click_item("Rectangle", "left")
        x, y = run.window_origin()
        nested.input.drag((x + 300, y + 260), (x + 480, y + 370), OUTPUT_W, OUTPUT_H)
        run.click_item("Line", "left")
        nested.input.drag((x + 540, y + 260), (x + 700, y + 350), OUTPUT_W, OUTPUT_H)
        run.click_item("Sketch", "left")
        nested.input.drag((x + 300, y + 430), (x + 450, y + 480), OUTPUT_W, OUTPUT_H)
        run.click_item("Text", "left")
        nested.input.click(x + 540, y + 430, OUTPUT_W, OUTPUT_H)
        time.sleep(settle)
        nested.input.key("cmd-a")
        nested.input.type_text("Review")
        run.click_item("Sign", "left")
        nested.input.drag((x + 540, y + 500), (x + 700, y + 550), OUTPUT_W, OUTPUT_H)
        time.sleep(settle)
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["grim", str(destination)], env=run.env, check=True)
        nested.input.key("cmd-s")
        time.sleep(max(settle, 1.0))
        pdf = (run.sandbox / "guide.pdf").read_bytes()
        if any(kind not in pdf for kind in (b"/Annots", b"/Square", b"/Line", b"/Ink", b"/FreeText")):
            raise StepFailed("saved PDF lacks a drawn or text annotation")
        if pdf.count(b"/Subtype/Ink") != 2:
            raise StepFailed("saved PDF lacks sketch and signature annotations")
        if b"/Contents/Review" not in pdf:
            raise StepFailed("text box edit was not saved")
        saved_pdf = destination.with_suffix(".pdf")
        saved_pdf.write_bytes(pdf)
        print(json.dumps({"capture": str(destination), "saved_pdf": str(saved_pdf),
                          "pdf_annotation": True}), flush=True)
    finally:
        run.stop()


def check_files_tag_swatches(nested: Nested, bins: list[Path], settle: float) -> None:
    """Verify the tag dots, stable accessible names, and real tag toggling."""
    filename = "tag-swatch.txt"
    scenario = {
        "app": "files",
        "setup": {"files": {filename: "Private tag swatch fixture.\n"}},
        "launch": {"reveal": filename},
        "steps": [],
    }
    run = LuloRun(nested, "private/file-tag-swatches", scenario, bins, settle, None)
    colors = ("Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Gray")
    try:
        run.setup()
        run.launch()
        path = run.sandbox / filename

        def tag_value() -> Optional[str]:
            try:
                return os.getxattr(path, "user.rmac.tag").decode("utf-8")
            except OSError as error:
                if error.errno in {errno.ENODATA, getattr(errno, "ENOATTR", errno.ENODATA)}:
                    return None
                raise

        def wait_for_tag(expected: Optional[str], timeout: float = 8.0) -> bool:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if tag_value() == expected:
                    return True
                time.sleep(0.2)
            return tag_value() == expected

        def visible_items() -> dict[str, Any]:
            pyatspi = atspi()
            frame = run.active_frame()
            return {
                name(node): node
                for node in (descendants(frame, limit=5000) if frame is not None else [])
                if role(node) in {"menu item", "check menu item"}
                and has_state(node, pyatspi.STATE_SHOWING)
            }

        def open_item_menu() -> dict[str, Any]:
            run.click_item(filename, "right")
            time.sleep(settle)
            return visible_items()

        def click_menu_item(label: str, items: dict[str, Any]) -> None:
            target = items.get(label)
            if target is None:
                raise StepFailed(f"no visible item-menu row named {label!r}")
            box = extents(target)
            if not box:
                raise StepFailed(f"tag row {label!r} has no visible bounds")
            ox, oy = run.window_origin()
            x, y = ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2
            nested.input.click(x, y, OUTPUT_W, OUTPUT_H)

        rows = open_item_menu()
        missing = [color for color in colors if color not in rows]
        if missing:
            raise StepFailed(f"item menu is missing tag colors {missing}; found {sorted(rows)}")
        expected_rgb = {
            "Red": ((0xFF, 0x3B, 0x30), (0xFF, 0x45, 0x3A)),
            "Orange": ((0xFF, 0x95, 0x00), (0xFF, 0x9F, 0x0A)),
            "Yellow": ((0xFF, 0xCC, 0x00), (0xFF, 0xD6, 0x0A)),
            "Green": ((0x34, 0xC7, 0x59), (0x30, 0xD1, 0x58)),
            "Blue": ((0x13, 0x72, 0xF9), (0x13, 0x72, 0xF9)),
            "Purple": ((0xAF, 0x52, 0xDE), (0xBF, 0x5A, 0xF2)),
            "Gray": ((0x8E, 0x8E, 0x93), (0x8E, 0x8E, 0x93)),
        }
        screenshot = nested.work / "file-tag-swatches.png"
        if shutil.which("grim", path=run.env.get("PATH")) is None:
            raise StepFailed("grim is required to verify the private tag swatch colors")
        subprocess.run(["grim", str(screenshot)], env=run.env, check=True)
        ox, oy = run.window_origin()
        for color in colors:
            swatch_name = f"{color} tag color"
            swatches = [
                child
                for child in descendants(rows[color], limit=40)
                if role(child) == "image" and name(child) == swatch_name
            ]
            if not swatches:
                raise StepFailed(f"{color} row has no accessible color swatch named {swatch_name!r}")
            box = extents(swatches[0])
            if not box or box[2] < 3 or box[3] < 3:
                raise StepFailed(f"{color} swatch has no measurable visible bounds: {box}")
            actual = png_pixel_rgb(
                screenshot, ox + box[0] + box[2] // 2, oy + box[1] + box[3] // 2
            )
            expected = expected_rgb[color]
            if not any(
                all(abs(actual[channel] - palette[channel]) <= 16 for channel in range(3))
                for palette in expected
            ):
                raise StepFailed(
                    f"{color} swatch center pixel is {actual!r}, expected one of {expected!r}"
                )
        print("PASS  seven visible tag dots match their colors and keep Red–Gray accessible names", flush=True)

        click_menu_item("Red", rows)
        if not wait_for_tag("red"):
            raise StepFailed(f"selecting Red did not write user.rmac.tag=red (value={tag_value()!r})")

        rows = open_item_menu()
        pyatspi = atspi()
        if not has_state(rows.get("Red"), pyatspi.STATE_CHECKED):
            raise StepFailed("Red did not appear checked after applying the tag")
        print("PASS  applying Red checks the row and persists user.rmac.tag=red", flush=True)
        click_menu_item("Red", rows)
        if not wait_for_tag(None):
            raise StepFailed(f"selecting Red again did not remove the tag (value={tag_value()!r})")
        print("PASS  selecting Red again removes the tag xattr", flush=True)
    finally:
        run.stop()


def check_storage_deep_link(nested: Nested, bins: list[Path], settle: float) -> None:
    """A second `--pane storage` launch (the deep-link/relaunch path a system
    launcher or `--pane storage` capture uses, dispatched through SET-57's
    single-window reuse as `NavigateToPane`) must measure Storage categories
    the same way clicking "Storage" in the sidebar already does. Regression
    coverage for a bug where `navigate_to_pane()` set the pane's nav stack
    directly but never called `measure_storage_categories()`, leaving every
    category stuck at "0.0 GB" forever when Settings was reached this way."""
    scenario = {"app": "settings", "launch": {}, "steps": []}
    run = LuloRun(nested, "private/storage-deep-link", scenario, bins, settle, None)
    try:
        run.setup()
        run.launch()

        def refresh_y() -> Optional[int]:
            frame = run.active_frame()
            if frame is None:
                return None
            refresh = next((node for node in descendants(frame, limit=3000) if name(node) == "Refresh"), None)
            box = extents(refresh) if refresh is not None else None
            return box[1] if box else None

        second = subprocess.Popen(
            [str(run.binary()), "--pane", "storage"], env=run.env,
            stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT, close_fds=True,
        )
        try:
            second.wait(10)
        except subprocess.TimeoutExpired:
            second.kill()
            second.wait(5)
        if second.returncode not in (0, None):
            raise StepFailed(f"second `--pane storage` launch exited {second.returncode}")

        def on_storage_pane() -> bool:
            frame = run.active_frame()
            return frame is not None and "Storage" in name(frame)

        reached_storage = False
        reach_deadline = time.monotonic() + 10.0
        while time.monotonic() < reach_deadline:
            if on_storage_pane():
                reached_storage = True
                break
            time.sleep(0.1)
        if not reached_storage:
            raise StepFailed(
                "the second launch's --pane storage deep link never reached "
                "the Storage pane at all (navigate_to_pane/SET-57 dispatch itself "
                "is broken, not just the measurement)"
            )

        # Once on Storage, `categories` is `None` (unmeasured) until
        # `measure_storage_categories()` runs; only then does the render add
        # a second card (System Data's own row, plus any non-empty category)
        # below the usage bar, pushing Refresh down from where it sits right
        # under the bar alone. AT-SPI exposes no text for the GB figures
        # themselves (`icon_row`/`storage_legend` are plain, unnamed divs),
        # so this geometry shift -- confirmed by hand against a fixed build,
        # where the categories card puts Refresh at y=263 -- is the only
        # available signal that a scan actually completed.
        deadline = time.monotonic() + 15
        measured = False
        while time.monotonic() < deadline:
            y = refresh_y()
            if y is not None and y >= 220:
                measured = True
                break
            time.sleep(0.1)
        if not measured:
            frame = run.active_frame()
            names = [f"{role(n)} {name(n)!r}" for n in descendants(frame, limit=400)] if frame else []
            shot = nested.work / "debug-storage-deep-link.png"
            subprocess.run(["grim", str(shot)], env=run.env, check=False)
            raise StepFailed(
                "Storage never measured categories after a --pane storage deep link "
                "(the sidebar-click path already works; this one regressed); "
                f"refresh_y={refresh_y()}, screenshot={shot}, tree={names}"
            )
        print(
            "PASS  a --pane storage deep link (second launch, SET-57 reuse) "
            "measures Storage categories, not just a sidebar click",
            flush=True,
        )
    finally:
        run.stop()


def inner(args: argparse.Namespace) -> int:
    work = Path(args.inner)
    nested = Nested(work)
    bins = [Path(p) for p in args.bin_dir] + [Path(p) for p in args.shell_bin_dir]
    results = []
    try:
        if args.benchmark_storage:
            benchmark_storage(nested, bins, args.benchmark_storage)
            return 0
        if args.check_context_submenus:
            check_files_context_submenus(nested, bins, args.settle)
            return 0
        if args.check_file_tag_swatches:
            check_files_tag_swatches(nested, bins, args.settle)
            return 0
        if args.check_storage_deep_link:
            check_storage_deep_link(nested, bins, args.settle)
            return 0
        if args.check_terminal_profiles:
            check_terminal_profiles(nested, bins, args.settle)
            return 0
        if args.preview_markup_capture:
            capture_preview_markup(nested, bins, args.settle, Path(args.preview_markup_capture))
            return 0
        for path in sc.scenario_paths(only=args.scenarios):
            sid = sc.scenario_id(path)
            scenario = sc.load(path)
            expected_path = sc.expectation_path(path)
            if not expected_path.exists() and not args.explore:
                continue
            run = LuloRun(nested, sid, scenario, bins, args.settle,
                          Path(args.capture_dir) if args.capture_dir else None)
            actual: dict[str, Any] = {"format": sc.FORMAT, "scenario": sid, "observations": {}}
            try:
                run.setup()
                run.launch()
                if args.explore:
                    run.run_steps(limit=args.explore_steps)
                    explore(run)
                    continue
                actual["observations"] = run.run_steps()
            except Unsupported as error:
                actual["unsupported"] = str(error)
            except (StepFailed, wlinput.InjectorError) as error:
                actual["error"] = str(error)
            finally:
                run.stop()
            if args.explore:
                if "error" in actual:
                    print(f"== {sid}: {actual['error']}")
                continue
            expected = json.loads(expected_path.read_text())
            mismatches = sc.compare(scenario, expected, actual)
            status = "unsupported" if "unsupported" in actual else ("pass" if not mismatches else "fail")
            results.append({"scenario": sid, "title": scenario["title"], "status": status,
                            "mismatches": mismatches, "lulo": actual})
            if status == "unsupported":
                print(f"SKIP  {sid}  {actual['unsupported']}")
            else:
                print("\n".join(sc.report_lines(sid, scenario, mismatches)), flush=True)
            time.sleep(0.5)
    finally:
        nested.close()
    if args.output:
        Path(args.output).write_text(json.dumps({"format": sc.FORMAT, "results": results}, indent=2, ensure_ascii=False) + "\n")
    passed = sum(r["status"] == "pass" for r in results)
    print(f"\n{passed}/{len(results)} scenarios match the Mac")
    return 0 if passed == len(results) else 1


def check_terminal_profiles(nested: Nested, bins: list[Path], settle: float) -> None:
    """Exercise Terminal's Settings shortcut and profile persistence in nested niri."""
    scenario = sc.load(sc.REPO / "docs" / "behavior-pending" / "terminal" / "profile-settings.json")
    run = LuloRun(nested, "private/terminal-profiles", scenario, bins, settle, None)
    try:
        run.setup()
        run.launch()
        run.run_steps()
        windows = [w for w in nested.windows() if w.get("pid") == run.process.pid]
        if len(windows) != 2:
            raise StepFailed(f"⌘, should open one Settings window; found {len(windows)} windows")
        print("PASS  ⌘, opens one Terminal Settings window", flush=True)

        names = {name(node) for frame in run.frames() for node in descendants(frame, limit=6000)}
        profiles = {
            "Basic", "Clear Dark", "Clear Light", "Grass", "Homebrew",
            "Man Page", "Novel", "Ocean", "Pro", "Red Sands",
            "Silver Aerogel", "Solid Colors",
        }
        missing = profiles - names
        if missing:
            raise StepFailed(f"Settings does not expose profile rows: {sorted(missing)}")
        print("PASS  all 12 Mac profile names are exposed in Settings", flush=True)

        run.click_item("Clear Dark", "left")
        path = Path(run.env["XDG_CONFIG_HOME"]) / "rmac-terminal" / "profile.txt"
        if not path.exists() or path.read_text().strip() != "Clear Dark":
            raise StepFailed("choosing Clear Dark did not persist the selected profile")
        print("PASS  choosing Clear Dark persists the profile", flush=True)
    finally:
        run.stop()


def benchmark_storage(nested: Nested, bins: list[Path], count: int) -> None:
    """Measure one Settings instance against a private synthetic home tree."""
    scenario = {"app": "settings", "launch": {}, "steps": []}
    run = LuloRun(nested, "private/storage-benchmark", scenario, bins, 0.1)
    documents = Path(run.env["HOME"]) / "Documents"
    for directory_number in range((count + 999) // 1000):
        directory = documents / f"many-{directory_number:03d}"
        directory.mkdir()
        for number in range(min(1000, count - directory_number * 1000)):
            (directory / f"file-{number:04d}").touch()
    (documents / "measured.txt").write_bytes(b"x" * 4096)

    ticks = os.sysconf("SC_CLK_TCK")

    def cpu_seconds() -> float:
        stat = Path(f"/proc/{run.process.pid}/stat").read_text()
        fields = stat.rsplit(")", 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / ticks

    try:
        run.setup()
        run.launch()

        def refresh_y() -> Optional[int]:
            frame = run.active_frame()
            if frame is None:
                return None
            refresh = next((node for node in descendants(frame, limit=3000) if name(node) == "Refresh"), None)
            box = extents(refresh) if refresh is not None else None
            return box[1] if box else None

        def storage_click_point() -> tuple[int, int]:
            frame = run.active_frame()
            if frame is None:
                raise StepFailed("Settings has no active window")
            storage = next((node for node in descendants(frame, limit=3000) if name(node) == "Storage"), None)
            box = extents(storage) if storage is not None else None
            if box is None:
                raise StepFailed("Storage row is not accessible from General")
            ox, oy = run.window_origin(frame)
            return ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2

        def measure_open() -> dict[str, Any]:
            x, y = storage_click_point()
            before_cpu = cpu_seconds()
            started = time.monotonic()
            run.nested.input.click(x, y, OUTPUT_W, OUTPUT_H)
            first = complete = capacity = None
            deadline = started + 90
            while time.monotonic() < deadline:
                y = refresh_y()
                elapsed = time.monotonic() - started
                # Text inside the Storage cards is not exported through
                # AT-SPI. The Refresh button follows the bar, category rows,
                # and footnote in this fixed-size nested Settings window.
                if capacity is None and y is not None and y >= 260:
                    capacity = elapsed
                if first is None and y is not None and y >= (320 if count >= 64_000 else 300):
                    first = elapsed
                finished = y is not None and (y >= 340 if count >= 64_000 else 300 <= y < 320)
                if first is not None and finished:
                    complete = elapsed
                    break
                time.sleep(0.05)
            completed_cpu = cpu_seconds()
            time.sleep(5.0)
            idle_start_cpu = cpu_seconds()
            time.sleep(1.0)
            result = {
                "capacity_seconds": capacity,
                "first_category_seconds": first,
                "complete_seconds": complete,
                "cpu_seconds": round(completed_cpu - before_cpu, 3),
                "idle_cpu_seconds_per_second": round(cpu_seconds() - idle_start_cpu, 3),
            }
            if first is None or complete is None:
                raise StepFailed(f"Storage categories did not complete: {result}")
            return result

        cold = measure_open()
        run.click_item("General", "left")
        time.sleep(0.2)
        x, y = storage_click_point()
        before_cpu = cpu_seconds()
        started = time.monotonic()
        run.nested.input.click(x, y, OUTPUT_W, OUTPUT_H)
        first_reopen = None
        while time.monotonic() - started < 2 and first_reopen is None:
            refresh_position = refresh_y()
            if refresh_position is not None and refresh_position >= (320 if count >= 64_000 else 300):
                first_reopen = time.monotonic() - started
            time.sleep(0.05)
        remaining = 2 - (time.monotonic() - started)
        if remaining > 0:
            time.sleep(remaining)
        reopen = {
            "first_category_seconds": first_reopen,
            "cpu_seconds_in_two_seconds": round(cpu_seconds() - before_cpu, 3),
        }
        print(json.dumps({
            "files": count,
            "cold": cold,
            "reopen": reopen,
            "binary": str(run.binary()),
        }), flush=True)
    finally:
        run.stop()


def main(argv: Optional[list[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="area/name ids (default: every scenario with a .mac.json)")
    parser.add_argument("--bin-dir", action="append", default=[], help="directory with rmac-files etc. (repeatable)")
    parser.add_argument("--shell-bin-dir", action="append", default=[], help="directory with the shell's wallpaper binary")
    parser.add_argument("--output", help="write results JSON here (compare.py reads it)")
    parser.add_argument("--capture-dir", help="save scenario capture steps with grim into this directory")
    parser.add_argument("--settle", type=float, default=0.8)
    parser.add_argument("--keep", action="store_true", help="keep the temporary directory and logs")
    parser.add_argument("--explore", action="store_true", help="print each scenario app's accessible tree instead")
    parser.add_argument("--explore-steps", type=int, default=0, help="with --explore: play this many steps first")
    parser.add_argument("--check-context-submenus", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check-file-tag-swatches", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check-storage-deep-link", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check-terminal-profiles", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--benchmark-storage", type=int, metavar="FILES", help="measure Storage against a synthetic home")
    parser.add_argument("--preview-markup-capture", help=argparse.SUPPRESS)
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if not sys.platform.startswith("linux"):
        parser.error("run_lulo.py runs on Linux (the reference laptop or CI)")
    # A timeout's SIGTERM still runs the cleanup (reap, then remove the tree).
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    args.bin_dir = [str(Path(p).resolve()) for p in args.bin_dir]
    args.shell_bin_dir = [str(Path(p).resolve()) for p in args.shell_bin_dir]
    if args.output:
        args.output = str(Path(args.output).resolve())
    if args.capture_dir:
        args.capture_dir = str(Path(args.capture_dir).resolve())
    rebuilt = list(args.scenarios)
    for directory in args.bin_dir:
        rebuilt += ["--bin-dir", directory]
    for directory in args.shell_bin_dir:
        rebuilt += ["--shell-bin-dir", directory]
    if args.output:
        rebuilt += ["--output", args.output]
    if args.capture_dir:
        rebuilt += ["--capture-dir", args.capture_dir]
    rebuilt += ["--settle", str(args.settle), "--explore-steps", str(args.explore_steps)]
    if args.explore:
        rebuilt.append("--explore")
    if args.check_context_submenus:
        rebuilt.append("--check-context-submenus")
    if args.check_file_tag_swatches:
        rebuilt.append("--check-file-tag-swatches")
    if args.check_storage_deep_link:
        rebuilt.append("--check-storage-deep-link")
    if args.check_terminal_profiles:
        rebuilt.append("--check-terminal-profiles")
    if args.benchmark_storage:
        rebuilt += ["--benchmark-storage", str(args.benchmark_storage)]
    if args.preview_markup_capture:
        rebuilt += ["--preview-markup-capture", str(Path(args.preview_markup_capture).resolve())]
    return outer(args, rebuilt)


if __name__ == "__main__":
    sys.exit(main())
