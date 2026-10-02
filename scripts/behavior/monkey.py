#!/usr/bin/env python3
"""Seeded random ("monkey") testing for Lulo apps and the shell.

    python3 scripts/behavior/monkey.py --bin-dir DIR [--shell-bin-dir DIR] \\
        --niri PATH --app files --duration 1800 [--seed N] \\
        [--findings-dir DIR] [--max-findings N]

--app is one of files, text-editor, settings, calculator, clock, weather,
preview, notes, system-monitor, terminal, or shell (Dock, the menu bar,
Spotlight, Control Centre and Mission Control, with no document window).

Each run drives the target for --duration seconds with a seeded stream of
actions: weighted random clicks (preferring a real accessible control when
one is on screen), menu-bar items opened through the AT-SPI tree, keyboard
shortcuts drawn from tests/inventory/lulo/<App>.json, typed text, window
move/resize/minimise/zoom/close/new, and file operations confined to a
private sandbox (text, a PDF, a PNG, folders, a long name, a unicode name,
a 0-byte file and a large file). It watches for: a process crash (exit or
"panicked at" in the log), a hang (no AT-SPI response, or no new frame, for
more than 5 s), an error dialog, runaway CPU (>50% for >5 s while idle),
memory growth (Pss_Anon+SwapPss across the run), a stuck window (the niri
window list disagreeing with the AT-SPI tree), and journal warnings.

On a finding, the run freezes: the seed, the full action log, the app's
stderr/stdout tail and a grim screenshot go to --findings-dir (never
committed -- it defaults outside the repository). It then replays shrinking
prefixes of the action log against a freshly relaunched app (binary search:
if a shorter prefix still reproduces the same kind of finding, keep
shrinking) to find a minimal repro, and writes one markdown report per
finding.

Isolation is run_window_move.Run's (itself run_lulo.py's): a private
dbus-run-session, temporary HOME/XDG, headless Sway with a nested niri
running the shipped shell.kdl, the Dock and Mission Control services, and
input only through wlinput.py's virtual keyboard/pointer, which refuse the
live session. This script takes /tmp/lulo-journey.lock itself, the same as
every other scripts/behavior runner: never wrap it in an outer flock.
"""

from __future__ import annotations

import argparse
import fcntl
import importlib.util
import json
import os
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import run_window_move  # noqa: E402
import wlinput  # noqa: E402

_soak_spec = importlib.util.spec_from_file_location("run_memory_soak", HERE / "run_memory_soak.py")
assert _soak_spec is not None and _soak_spec.loader is not None
run_memory_soak = importlib.util.module_from_spec(_soak_spec)
sys.modules.setdefault(_soak_spec.name, run_memory_soak)
_soak_spec.loader.exec_module(run_memory_soak)

# --------------------------------------------------------------------------
# App identity: binary names (run_lulo), niri app_id, and the inventory file.
# --------------------------------------------------------------------------

APPS = ("files", "text-editor", "settings", "calculator", "clock", "weather",
        "preview", "notes", "system-monitor", "terminal")
TARGETS = APPS + ("shell",)

APP_IDS = {
    "files": "org.rmac.Files",
    "text-editor": "org.rmac.TextEditor",
    "settings": "org.rmac.SystemSettings",
    "calculator": "org.rmac.Calculator",
    "clock": "org.rmac.Clock",
    "weather": "org.rmac.Weather",
    "preview": "org.rmac.Preview",
    "notes": "org.rmac.Notes",
    "system-monitor": "org.rmac.SystemMonitor",
    "terminal": "org.rmac.Terminal",
}

INVENTORY_LABELS = {
    "files": "Finder",
    "text-editor": "Text Editor",
    "settings": "System Settings",
    "calculator": "Calculator",
    "clock": "Clock",
    "weather": "Weather",
    "preview": "Preview",
    "notes": "Notes",
    "system-monitor": "System Monitor",
    "terminal": "Terminal",
}

INVENTORY_DIR = REPO / "tests" / "inventory" / "lulo"

# Best-effort extra shell surfaces started alongside the Dock and Mission
# Control that run_window_move.Run already brings up. A missing binary is
# skipped (logged), never a hard failure: these are opportunistic coverage
# for the "shell" target, not load-bearing for app testing.
SHELL_EXTRAS = (("menubar", "top-bar"), ("launcher", "rmac-launcher"),
                ("quick-settings", "rmac-quick-settings"),
                ("notification-center", "rmac-notification-center-panel"))

CRASH_MARKERS = ("panicked at", "fatal runtime error", "stack overflow",
                  "RUST_BACKTRACE=1 was not", "memory allocation of")
DIALOG_ROLES = {"dialog", "alert", "file chooser"}
ERROR_KEYWORDS = ("error", "panic", "crash", "failed to", "unexpected", "unreachable")
# Printing needs a real printer/portal and cannot produce a meaningful
# finding in the private headless compositor.
UNSAFE_MENU_WORDS = ("quit process", "force quit", "shut down", "restart", "sleep",
                     "log out", "lock screen", "print")
SAFE_NAV_APPS = {"settings", "system-monitor"}
SAFE_NAV_ROLES = {"list item", "page tab", "tab", "tree item"}


# --------------------------------------------------------------------------
# Binary lookup across one or more directories.
# --------------------------------------------------------------------------


def find_binary(dirs: list[Path], names: list[str]) -> Optional[Path]:
    for directory in dirs:
        for candidate in names:
            path = directory / candidate
            if path.is_file() and os.access(path, os.X_OK):
                return path.resolve()
    return None


# --------------------------------------------------------------------------
# The sandbox: a small, deterministic fixture of sample files/folders.
# --------------------------------------------------------------------------

_MINIMAL_PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000020000000208020000007213"
    "e9e40000001849444154789c6360606006061646060616464606000000ffff"
    "03000309010a9a4b5a9f0000000049454e44ae426082"
)
# A tiny, valid single-page PDF (no external deps): enough for Preview/Finder
# to recognise and render something rather than fail to open.
_MINIMAL_PDF = b"""%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<<>>>>endobj
xref
0 4
0000000000 65535 f
trailer<</Size 4/Root 1 0 R>>
startxref
0
%%EOF
"""


def build_sandbox(root: Path) -> None:
    """A fixed fixture (not seed-dependent, so shrink replay stays stable):
    plain text, a 0-byte file, a ~12 MiB "large" file, a minimal PNG and
    PDF, nested folders, a long file name and a unicode file name."""

    root.mkdir(parents=True, exist_ok=True)
    (root / "notes.txt").write_text("The quick brown fox jumps over the lazy dog.\n" * 20)
    (root / "empty.txt").write_bytes(b"")
    with (root / "large.bin").open("wb") as handle:
        chunk = bytes(range(256)) * 4096
        for _ in range(12):
            handle.write(chunk)
    (root / "picture.png").write_bytes(_MINIMAL_PNG)
    (root / "document.pdf").write_bytes(_MINIMAL_PDF)
    folder = root / "Folder"
    (folder / "Nested Folder").mkdir(parents=True, exist_ok=True)
    (folder / "inside.txt").write_text("inside the folder\n")
    long_name = ("a-very-long-file-name-that-stresses-truncation-and-" * 4 + ".txt")[:240]
    (root / long_name).write_text("long name\n")
    unicode_name = "café 🎉 日本語 файл.txt"
    (root / unicode_name).write_text("unicode name\n", encoding="utf-8")


# --------------------------------------------------------------------------
# Shortcut inventory: flatten tests/inventory/lulo/<App>.json's menu bar.
# --------------------------------------------------------------------------


def load_shortcuts(app: str) -> list[tuple[str, str]]:
    """[(menu > item label, shortcut glyph string), ...] for every menu item
    with a non-empty shortcut, recursing into submenus."""

    label = INVENTORY_LABELS.get(app)
    if label is None:
        return []
    path = INVENTORY_DIR / f"{label}.json"
    if not path.is_file():
        return []
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return []
    out: list[tuple[str, str]] = []

    def walk(menu_label: str, items: list[dict[str, Any]]) -> None:
        for item in items:
            shortcut = item.get("shortcut") or ""
            item_label = item.get("label") or ""
            if shortcut and not any(word in item_label.lower() for word in UNSAFE_MENU_WORDS):
                out.append((f"{menu_label} > {item_label}", shortcut))
            children = item.get("children") or []
            if children:
                walk(f"{menu_label} > {item_label}", children)

    for menu in data.get("menu_bar", []):
        walk(menu.get("label", ""), menu.get("items", []))
    return out


def load_settings_sidebar_labels() -> set[str]:
    path = INVENTORY_DIR / "System Settings.json"
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return set()
    return {item["label"] for item in data.get("sidebar", []) if isinstance(item.get("label"), str)}


def shortcut_to_chord(shortcut: str) -> Optional[str]:
    """wlinput.parse_chord already accepts the Mac glyph form directly; this
    just validates it up front so unparseable inventory entries (rare, e.g.
    a function key glyph the injector does not model) are skipped quietly
    rather than raising mid-run."""

    try:
        wlinput.parse_chord(shortcut)
    except wlinput.InjectorError:
        return None
    return shortcut


# --------------------------------------------------------------------------
# Random text generation.
# --------------------------------------------------------------------------

_WORDS = ("hello", "world", "test", "lulo", "finder", "the", "quick", "brown",
          "fox", "folder", "document", "quit", "undo", "redo", "select")
_UNICODE_BITS = "café 日本語 файл 🎉 — “quoted” €£¥ \t\n"


def random_text(rng: random.Random, max_len: int = 24) -> str:
    kind = rng.random()
    if kind < 0.5:
        return " ".join(rng.choice(_WORDS) for _ in range(rng.randint(1, 4)))
    if kind < 0.8:
        return "".join(rng.choice(_UNICODE_BITS) for _ in range(rng.randint(1, max_len)))
    # ASCII punctuation/control-ish stress.
    return "".join(chr(rng.randint(32, 126)) for _ in range(rng.randint(1, max_len)))


def sanitize_for_typing(text: str) -> str:
    """Drop characters wlinput.text_to_strokes cannot map (mirrors its own
    acceptance test exactly -- notably, plain `char.isascii()` is NOT a
    substitute: tab and other C0 controls are ASCII but unmapped, and
    text_to_strokes raises InjectorError on them)."""

    out = []
    for char in text:
        if char == "\n" or (char.isascii() and char.isalpha()) or char in wlinput.SHIFTED or char in wlinput.KEYCODES:
            out.append(char)
    return "".join(out)


# --------------------------------------------------------------------------
# Recorded, replayable actions.
# --------------------------------------------------------------------------


@dataclass
class ActionRecord:
    index: int
    kind: str
    params: dict[str, Any]
    note: str = ""


@dataclass
class Finding:
    kind: str  # crash | hang | error-dialog | runaway-cpu | memory-growth | stuck-window | journal-warning
    detail: str
    action_count: int
    evidence: dict[str, Any] = field(default_factory=dict)


class MonkeyError(RuntimeError):
    pass


def call_with_timeout(fn: Callable[[], Any], timeout: float) -> tuple[bool, Any]:
    """Run fn() off-thread with a hard deadline. Returns (finished, value).
    Used to detect an AT-SPI call (or anything else) that never returns --
    the hang signature this tool must catch."""

    box: dict[str, Any] = {}

    def runner() -> None:
        try:
            box["value"] = fn()
        except Exception as error:  # noqa: BLE001
            box["error"] = error

    thread = threading.Thread(target=runner, daemon=True)
    thread.start()
    thread.join(timeout)
    if thread.is_alive():
        return False, None
    if "error" in box:
        raise box["error"]
    return True, box.get("value")


# --------------------------------------------------------------------------
# The monkey itself.
# --------------------------------------------------------------------------


class Monkey:
    def __init__(self, run: run_window_move.Run, app: str, app_dirs: list[Path],
                 home_root: Path, logger: Callable[[str], None]) -> None:
        self.run = run
        self.app = app
        self.app_dirs = app_dirs
        self.log = logger
        self.process: Optional[subprocess.Popen] = None
        self.app_log: Optional[Path] = None
        self._app_log_offset = 0
        self.shortcuts = load_shortcuts(app) if app != "shell" else []
        self.settings_sidebar_labels = load_settings_sidebar_labels() if app == "settings" else set()
        self.extra_processes: dict[str, subprocess.Popen] = {}
        self._shell_log_offsets: dict[Path, int] = {}
        self.home = home_root
        self.sandbox = home_root / "sandbox"
        self.hertz = os.sysconf("SC_CLK_TCK")
        self._baseline_mem_kib: Optional[int] = None
        self._mem_samples: list[tuple[float, int]] = []
        self._window_missing_since: Optional[float] = None
        self._launched_at = 0.0

    # -- process lifecycle --------------------------------------------------

    def binary(self) -> Path:
        if self.app == "shell":
            raise MonkeyError("shell target has no single binary")
        names = run_lulo.APP_BINARIES[self.app]
        found = find_binary(self.app_dirs, names)
        if found is None:
            raise MonkeyError(f"no binary for {self.app} in {self.app_dirs}")
        return found

    def launch(self) -> None:
        for sub in (".config", ".local/share", ".local/state", ".cache", "Desktop", "Documents"):
            (self.home / sub).mkdir(parents=True, exist_ok=True)
        env = dict(self.run.env)
        env.update({
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "XDG_DATA_HOME": str(self.home / ".local/share"),
            "XDG_STATE_HOME": str(self.home / ".local/state"),
            "XDG_CACHE_HOME": str(self.home / ".cache"),
        })
        self.env = env
        if not self.sandbox.exists():
            build_sandbox(self.sandbox)
        if self.app == "shell":
            self._launch_shell_extras()
            self._launched_at = time.monotonic()
            return
        command = [str(self.binary())]
        if self.app == "files":
            command += ["--path", str(self.sandbox)]
        elif self.app == "preview":
            images = sorted(self.sandbox.glob("*.png"))
            if images:
                command += [str(images[0])]
        log_path = self.run.logs / f"monkey-{self.app}-{int(time.time() * 1000)}.log"
        self.app_log = log_path
        self._app_log_offset = 0
        handle = open(log_path, "w")
        self.process = subprocess.Popen(command, env=self.env, stdout=handle, stderr=subprocess.STDOUT,
                                        close_fds=True, cwd=str(self.sandbox))
        handle.close()
        self.log(f"launched pid={self.process.pid} binary={command[0]}")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise MonkeyError(f"{command[0]} exited with {self.process.returncode} before showing a window; "
                                  f"log: {self.tail_log()}")
            if self.application() is not None:
                break
            time.sleep(0.2)
        else:
            raise MonkeyError(f"{command[0]} (pid {self.process.pid}) never grew an accessible tree")
        if self.app == "calculator":
            self._wait_calculator_settled()
        time.sleep(1.0)
        self._launched_at = time.monotonic()

    def _launch_shell_extras(self) -> None:
        for logical, binary_name in SHELL_EXTRAS:
            path = find_binary(self.app_dirs, [binary_name])
            if path is None:
                self.log(f"shell extra {logical!r} ({binary_name}) not found, skipping")
                continue
            handle = open(self.run.logs / f"monkey-shell-{logical}.log", "w")
            process = subprocess.Popen([str(path)], env=self.env, stdout=handle, stderr=subprocess.STDOUT,
                                       close_fds=True)
            handle.close()
            self.extra_processes[logical] = process
        time.sleep(1.5)

    def _wait_calculator_settled(self) -> None:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            window = self.run.window(APP_IDS["calculator"])
            if window is not None:
                _x, _y, width, height = self.run.geometry(window)
                visible_width, visible_height = run_lulo.calculator_visible_size(int(width), int(height))
                if visible_width and 228 <= visible_width <= 232 and visible_height and 404 <= visible_height <= 410:
                    return
            time.sleep(0.1)

    def relaunch(self, reset_state: bool = False) -> None:
        """Restart the app, resetting its private HOME only for a fresh replay."""

        self.stop()
        if reset_state:
            shutil.rmtree(self.home, ignore_errors=True)
        self._window_missing_since = None
        self._baseline_mem_kib = None
        self._mem_samples.clear()
        self.launch()

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                try:
                    self.process.wait(5)
                except subprocess.TimeoutExpired:
                    pass
        for process in self.extra_processes.values():
            if process.poll() is None:
                process.terminate()
        for process in self.extra_processes.values():
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.kill()
        self.extra_processes.clear()

    def alive(self) -> bool:
        if self.app == "shell":
            return True
        return self.process is not None and self.process.poll() is None

    def root_pid(self) -> Optional[int]:
        if self.app == "shell":
            return None
        return self.process.pid if self.process else None

    def shell_processes(self) -> dict[str, subprocess.Popen]:
        processes = dict(self.extra_processes)
        for process in self.run.children:
            name = Path(process.args[0]).name
            if name in ("dock", "mission-control"):
                processes[name] = process
        return processes

    def tail_log(self, n: int = 2000) -> str:
        if self.app == "shell":
            parts = []
            for path in sorted(self.run.logs.glob("*.log")):
                if path.name.startswith("monkey-shell-") or path.name in ("dock.log", "mission-control.log"):
                    parts.append(f"== {path.name} ==\n{path.read_bytes()[-n:].decode('utf-8', 'replace')}")
            return "\n".join(parts)[-n:]
        if self.app_log is None or not self.app_log.exists():
            return ""
        data = self.app_log.read_bytes()
        return data[-n:].decode("utf-8", "replace")

    # -- AT-SPI --------------------------------------------------------------

    def application(self):
        pid = self.root_pid()
        if pid is None:
            return None
        pyatspi = run_lulo.atspi()
        run_lulo.pump()
        desktop = pyatspi.Registry.getDesktop(0)
        for index in range(desktop.childCount):
            try:
                app = desktop.getChildAtIndex(index)
                if app is not None and app.get_process_id() == pid:
                    return app
            except Exception:  # noqa: BLE001
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
            except Exception:  # noqa: BLE001
                continue
            if child is not None:
                out.append(child)
        return out

    def active_frame(self):
        pyatspi = run_lulo.atspi()
        frames = self.frames()
        for frame in frames:
            if run_lulo.has_state(frame, pyatspi.STATE_ACTIVE):
                return frame
        return frames[0] if frames else None

    # -- geometry -------------------------------------------------------------

    def app_window(self) -> Optional[dict[str, Any]]:
        if self.app == "shell":
            return None
        return self.run.window(APP_IDS[self.app])

    def app_rect(self) -> Optional[tuple[float, float, float, float]]:
        window = self.app_window()
        if window is None:
            return 0.0, 0.0, float(self.run.width), float(self.run.height)
        return self.run.geometry(window)

    def random_point(self, rng: random.Random) -> tuple[float, float]:
        x, y, width, height = self.app_rect()
        margin = 4
        if width <= 2 * margin or height <= 2 * margin:
            return x + width / 2, y + height / 2
        return (x + rng.uniform(margin, width - margin), y + rng.uniform(margin, height - margin))

    def pick_accessible_point(self, rng: random.Random) -> Optional[tuple[float, float, str, str]]:
        """A random on-screen leaf with a name, for a more realistic click
        than a uniform point. Returns (x, y, role, name) in niri-local
        coords, or None if nothing usable is on screen."""

        frame = self.active_frame()
        if frame is None:
            return None
        nodes = [n for n in run_lulo.descendants(frame, limit=600) if n is not frame]
        rng.shuffle(nodes)
        wx, wy, _w, _h = self.app_rect()
        for node in nodes[:60]:
            box = run_lulo.extents(node)
            if not box or box[2] < 4 or box[3] < 4:
                continue
            label = run_lulo.name(node)
            if not label:
                continue
            role = run_lulo.role(node)
            if self.app in SAFE_NAV_APPS and role not in SAFE_NAV_ROLES:
                continue
            if self.app == "settings" and role in ("list item", "tree item") and label not in self.settings_sidebar_labels:
                continue
            return (wx + box[0] + box[2] / 2, wy + box[1] + box[3] / 2, role, label)
        return None

    # -- low-level input, shared by decide() and replay -----------------------

    def _bounded_point(self, point: tuple[float, float]) -> tuple[float, float]:
        x, y = point
        return (max(1.0, min(float(self.run.width - 1), x)),
                max(1.0, min(float(self.run.height - 1), y)))

    def _click_point(self, x: float, y: float, button: str = "left", count: int = 1) -> None:
        x, y = self._bounded_point((x, y))
        sx, sy = self.run.parent_point(x, y)
        self.run.pointer.click(sx, sy, self.run.parent_width, self.run.parent_height, button=button, count=count)

    def _drag(self, start: tuple[float, float], end: tuple[float, float]) -> None:
        self.run.drag(self._bounded_point(start), self._bounded_point(end))

    def _key(self, chord: str) -> None:
        self.run.pointer.key(chord)

    def _type(self, text: str) -> None:
        typed = sanitize_for_typing(text)
        if self.app == "terminal":
            typed = typed.replace("\n", "")
        self.run.pointer.type_text(typed, delay=0.02)

    # -- decide: produce a concrete, replayable ActionRecord -----------------

    ACTION_WEIGHTS_APP = (
        ("click-accessible", 30), ("click-point", 10), ("right-click", 10),
        ("double-click", 8), ("type-text", 12), ("shortcut", 18),
        ("window-move", 4), ("window-resize", 4), ("window-toggle", 6),
        ("fileop", 6), ("open-close-window", 4),
    )
    ACTION_WEIGHTS_SHELL = (
        ("spotlight-toggle", 20), ("spotlight-type", 10), ("notification-center", 10),
        ("mission-control", 15), ("desktop-click", 10), ("dock-menu", 15),
        ("menu-bar", 10), ("control-centre", 5), ("fileop", 5),
    )
    ACTION_WEIGHTS_SAFE_NAV = (
        ("click-accessible", 40), ("type-text", 15), ("shortcut", 20),
        ("window-move", 7), ("window-resize", 7), ("window-toggle", 6),
        ("fileop", 5),
    )

    def decide(self, index: int, rng: random.Random) -> ActionRecord:
        table = (self.ACTION_WEIGHTS_SHELL if self.app == "shell" else
                 self.ACTION_WEIGHTS_SAFE_NAV if self.app in SAFE_NAV_APPS else self.ACTION_WEIGHTS_APP)
        kinds = [k for k, _ in table]
        weights = [w for _, w in table]
        kind = rng.choices(kinds, weights=weights, k=1)[0]
        params: dict[str, Any] = {}
        note = ""

        if kind == "click-accessible":
            picked = self.pick_accessible_point(rng)
            if picked is None:
                if self.app in SAFE_NAV_APPS:
                    kind, params = "type-text", {"text": random_text(rng)}
                else:
                    kind, params = "click-point", {"xy": self.random_point(rng)}
            else:
                x, y, role, label = picked
                params = {"xy": (x, y)}
                note = f"{role} {label!r}"
        elif kind in ("click-point", "right-click", "double-click"):
            params = {"xy": self.random_point(rng)}
        elif kind == "type-text":
            params = {"text": random_text(rng)}
        elif kind == "shortcut":
            if self.shortcuts:
                label, shortcut = rng.choice(self.shortcuts)
                chord = shortcut_to_chord(shortcut)
                if chord is None:
                    kind, params, note = "type-text", {"text": random_text(rng)}, "unparseable shortcut skipped"
                else:
                    params, note = {"chord": chord}, label
            else:
                kind, params = "type-text", {"text": random_text(rng)}
        elif kind in ("window-move", "window-resize"):
            x, y, width, height = self.app_rect()
            if kind == "window-move":
                start = (x + min(80, width / 2), y + 10)
            else:
                start = (x + width - 6, y + height - 6)
            end = (start[0] + rng.uniform(-60, 60), start[1] + rng.uniform(-60, 60))
            params = {"start": start, "end": end}
        elif kind == "window-toggle":
            params = {"chord": rng.choice(["cmd-m", "cmd-ctrl-shift-f", "cmd-w", "cmd-n"])}
        elif kind == "fileop":
            params = {"op": rng.choice(["create", "rename", "delete", "mkdir"]), "seed": rng.random()}
        elif kind == "open-close-window":
            params = {"chord": rng.choice(["cmd-n", "cmd-w", "cmd-t"])}
        elif kind == "spotlight-toggle":
            params = {"chord": "cmd-space"}
        elif kind == "spotlight-type":
            params = {"text": rng.choice(["cal", "term", "files", "notes", str(rng.random())])}
        elif kind == "notification-center":
            params = {"chord": "cmd-ctrl-n"}
        elif kind == "mission-control":
            params = {"chord": rng.choice(["ctrl-up", "ctrl-down", "ctrl-left", "ctrl-right"])}
        elif kind == "desktop-click":
            params = {"xy": (rng.uniform(40, self.run.width - 40), rng.uniform(40, self.run.height - 120)),
                     "button": rng.choice(["left", "right"])}
        elif kind == "dock-menu":
            params = {"xy": (rng.uniform(self.run.width * 0.31, self.run.width * 0.68),
                             self.run.height - 40)}
        elif kind == "menu-bar":
            params = {"xy": (rng.uniform(20, self.run.width * 0.28), 15)}
        elif kind == "control-centre":
            params = {"xy": (self.run.width - 75, 15)}
        return ActionRecord(index=index, kind=kind, params=params, note=note)

    # -- execute: pure given params, used both live and on replay -----------

    def execute(self, action: ActionRecord) -> None:
        kind, params = action.kind, action.params
        if kind in ("click-accessible", "click-point"):
            self._click_point(*params["xy"])
        elif kind == "right-click":
            self._click_point(*params["xy"], button="right")
        elif kind == "double-click":
            self._click_point(*params["xy"], count=2)
        elif kind == "type-text":
            self._type(params["text"])
        elif kind in ("shortcut", "window-toggle", "open-close-window", "spotlight-toggle", "notification-center",
                     "mission-control"):
            self._key(params["chord"])
        elif kind in ("window-move", "window-resize"):
            self._drag(params["start"], params["end"])
        elif kind == "fileop":
            self._file_op(params["op"], params["seed"])
        elif kind == "spotlight-type":
            self._type(params["text"])
        elif kind == "desktop-click":
            button = params.get("button", "left")
            self._click_point(*params["xy"], button=button)
            if button == "right":
                time.sleep(0.2)
                self._key("escape")
        elif kind in ("dock-menu", "menu-bar", "control-centre"):
            self._click_point(*params["xy"], button="right" if kind == "dock-menu" else "left")
            time.sleep(0.2)
            self._key("escape")
        elif kind == "relaunch":
            self.relaunch()
        elif kind == "wait":
            time.sleep(min(120.0, max(0.0, float(params["seconds"]))))
        else:
            raise MonkeyError(f"unknown action kind {kind!r}")

    def _file_op(self, op: str, seed: float) -> None:
        rng = random.Random(seed)
        entries = sorted(self.sandbox.iterdir()) if self.sandbox.exists() else []
        try:
            if op == "mkdir":
                (self.sandbox / f"dir-{int(rng.random() * 1e6)}").mkdir(exist_ok=True)
            elif op == "create":
                (self.sandbox / f"file-{int(rng.random() * 1e6)}.txt").write_text("monkey\n")
            elif op == "rename" and entries:
                target = rng.choice(entries)
                target.rename(target.with_name(f"renamed-{target.name}"))
            elif op == "delete" and entries:
                target = rng.choice(entries)
                if target.is_dir():
                    shutil.rmtree(target, ignore_errors=True)
                else:
                    target.unlink(missing_ok=True)
        except OSError:
            pass  # a concurrent app operation on the same path is expected

    # -- health checks ---------------------------------------------------------

    def check_crashed(self) -> Optional[Finding]:
        if self.app == "shell":
            for name, process in self.shell_processes().items():
                code = process.poll()
                if code == 0 and name in ("dock", "mission-control", "menubar"):
                    return Finding("stuck-window", f"shell {name} exited while its surface should stay resident", -1,
                                   {"log_tail": self.tail_log()})
                if code not in (None, 0):
                    return Finding("crash", f"shell {name} exited with {process.returncode}", -1,
                                   {"log_tail": self.tail_log()})
            return None
        if self.process is not None and self.process.poll() not in (None, 0):
            return Finding("crash", f"process exited with {self.process.returncode}", -1,
                           {"log_tail": self.tail_log()})
        return None

    def check_panic_in_log(self) -> Optional[Finding]:
        if self.app == "shell":
            for path in sorted(self.run.logs.glob("*.log")):
                if not (path.name.startswith("monkey-shell-") or path.name in ("dock.log", "mission-control.log")):
                    continue
                data = path.read_bytes()
                new = data[self._shell_log_offsets.get(path, 0):]
                self._shell_log_offsets[path] = len(data)
                for marker in CRASH_MARKERS:
                    if marker.encode() in new:
                        return Finding("crash", f"{path.name} contains {marker!r}", -1,
                                       {"log_excerpt": new[-2000:].decode("utf-8", "replace")})
            return None
        if self.app_log is None or not self.app_log.exists():
            return None
        data = self.app_log.read_bytes()
        new = data[self._app_log_offset:]
        self._app_log_offset = len(data)
        text = new.decode("utf-8", "replace")
        for marker in CRASH_MARKERS:
            if marker in text:
                return Finding("crash", f"log contains {marker!r}", -1, {"log_excerpt": text[-2000:]})
        return None

    def check_error_dialog(self) -> Optional[Finding]:
        for frame in self.frames():
            for node in run_lulo.descendants(frame, limit=1500):
                role = run_lulo.role(node)
                if role not in DIALOG_ROLES:
                    continue
                if role == "alert":
                    return Finding("error-dialog", f"alert dialog {run_lulo.name(node)!r}", -1, {})
                texts = [run_lulo.name(node)]
                for child in run_lulo.descendants(node, limit=200):
                    value, _s, _e = run_lulo.text_of(child)
                    texts.append(value or run_lulo.name(child))
                blob = " ".join(t for t in texts if t).lower()
                if any(keyword in blob for keyword in ERROR_KEYWORDS):
                    return Finding("error-dialog", f"dialog text suggests an error: {blob[:200]!r}", -1, {})
        return None

    def sample(self) -> Optional[dict[str, Any]]:
        if self.app == "shell":
            samples = [run_memory_soak.sample_tree(process.pid, self.hertz)
                       for process in self.shell_processes().values() if process.poll() is None]
            samples = [sample for sample in samples if sample is not None]
            if not samples:
                return None
            return {key: sum(sample[key] for sample in samples)
                    for key in ("cpu_seconds", "pss_anon_kib", "swap_pss_kib")}
        pid = self.root_pid()
        if pid is None:
            return None
        return run_memory_soak.sample_tree(pid, self.hertz)

    def thread_cpu(self) -> dict[int, tuple[str, float]]:
        """Read per-thread CPU once, to locate an idle wake source."""
        pid = self.root_pid()
        if pid is None:
            return {}
        result = {}
        for task in (Path(f"/proc/{pid}/task")).glob("[0-9]*"):
            try:
                stat = (task / "stat").read_text()
                end = stat.rfind(")")
                fields = stat[end + 2:].split()
                cpu = (int(fields[11]) + int(fields[12])) / self.hertz
                result[int(task.name)] = (stat[stat.find("(") + 1:end], cpu)
            except (OSError, ValueError, IndexError):
                continue
        return result

    def check_idle_cpu(self, idle_seconds: float = 5.0,
                       threshold_percent: float = 50.0) -> Optional[Finding]:
        """Must be called with no actions in flight: samples CPU for
        idle_seconds and flags sustained >50% usage."""

        # Startup snapshots and a relaunch's first paint are active work.
        # Sampling them as "idle" produced repeatable false positives.
        if time.monotonic() - self._launched_at < 15.0:
            return None
        before = self.sample()
        if before is None:
            return None
        trace_threads = getattr(self, "app", None) not in (None, "shell")
        before_threads = self.thread_cpu() if trace_threads else {}
        time.sleep(idle_seconds)
        after = self.sample()
        if after is None:
            return None
        after_threads = self.thread_cpu() if trace_threads else {}
        percent = (after["cpu_seconds"] - before["cpu_seconds"]) / idle_seconds * 100
        self.log(f"idle CPU {percent:.1f}% over {idle_seconds:.0f}s")
        thread_usage = sorted(
            ((round((cpu - before_threads[tid][1]) / idle_seconds * 100, 2), name)
             for tid, (name, cpu) in after_threads.items() if tid in before_threads),
            reverse=True,
        )
        self.log(f"idle CPU by thread: {thread_usage[:8]}")
        if percent > threshold_percent:
            return Finding("runaway-cpu", f"{percent:.0f}% CPU over {idle_seconds:.0f}s idle", -1,
                           {"percent": percent})
        return None

    def record_memory(self) -> None:
        sample = self.sample()
        if sample is None:
            return
        value = sample["pss_anon_kib"] + sample["swap_pss_kib"]
        now = time.monotonic()
        if self._baseline_mem_kib is None:
            self._baseline_mem_kib = value
        self._mem_samples.append((now, value))
        if len(self._mem_samples) > 64:
            self._mem_samples = self._mem_samples[-64:]

    def check_memory_growth(self, threshold_kib: int = 150 * 1024) -> Optional[Finding]:
        if self._baseline_mem_kib is None or len(self._mem_samples) < 4:
            return None
        recent = [v for _t, v in self._mem_samples[-3:]]
        growth = recent[-1] - self._baseline_mem_kib
        monotonic_recent = recent == sorted(recent)
        if growth > threshold_kib and monotonic_recent:
            return Finding("memory-growth", f"Pss_Anon+SwapPss grew {growth / 1024:.0f} MiB above baseline", -1,
                           {"baseline_kib": self._baseline_mem_kib, "samples": self._mem_samples[-8:]})
        return None

    def check_stuck_window(self) -> Optional[Finding]:
        if self.app == "shell" or self.process is None or self.process.poll() is not None:
            self._window_missing_since = None
            return None
        niri_has_window = self.app_window() is not None
        pyatspi = run_lulo.atspi()
        atspi_showing = any(run_lulo.has_state(frame, pyatspi.STATE_SHOWING) for frame in self.frames())
        # A hidden, minimized or fullscreen window may disappear from niri's
        # current window list while its AT-SPI frame still says SHOWING. That
        # direction produced false findings for almost every app in the first
        # campaign. The opposite direction means a mapped window has no
        # accessible surface; require five seconds before calling it stuck.
        if niri_has_window and not atspi_showing:
            if self._window_missing_since is None:
                self._window_missing_since = time.monotonic()
            elif time.monotonic() - self._window_missing_since > 5:
                return Finding("stuck-window", "mapped window has no showing AT-SPI frame for >5 s", -1, {})
        else:
            self._window_missing_since = None
        return None

    def check_journal(self, since_epoch: float) -> Optional[Finding]:
        pid = self.root_pid()
        if shutil.which("journalctl") is None or pid is None:
            return None
        try:
            result = subprocess.run(
                ["journalctl", "--no-pager", "-q", "--since", f"@{int(since_epoch)}",
                 "-p", "warning", f"_PID={pid}"],
                env=self.env, capture_output=True, text=True, timeout=5,
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if result.returncode != 0 or not result.stdout.strip():
            return None
        return Finding("journal-warning", result.stdout.strip()[-500:], -1, {})

    def run_checks(self) -> Optional[Finding]:
        """The cheap checks, run after every single action. check_journal is
        not here: it spawns journalctl and belongs in the periodic idle
        check alongside the CPU/memory samples, not the hot per-action loop."""

        for check in (self.check_panic_in_log, self.check_crashed, self.check_stuck_window, self.check_error_dialog):
            finding = check()
            if finding:
                return finding
        return None

    def health_check(self) -> Optional[Finding]:
        finished, finding = call_with_timeout(self.run_checks, 5.0)
        if not finished:
            return Finding("hang", "health check did not respond within 5 s", -1, {})
        return finding


# --------------------------------------------------------------------------
# Shrink: binary search over the recorded action prefix.
# --------------------------------------------------------------------------


def reproduces(monkey: Monkey, actions: list[ActionRecord], count: int, expect_kind: str,
               since_epoch: float) -> bool:
    del since_epoch  # journal findings are never shrunk (see _handle_finding); kept for a stable signature
    monkey.relaunch(reset_state=True)
    try:
        for action in actions[:count]:
            try:
                monkey.execute(action)
            except Exception:  # noqa: BLE001
                pass
            time.sleep(0.05)
            finding = monkey.health_check()
            if finding and finding.kind == expect_kind:
                return True
        finding = monkey.health_check()
        if finding and finding.kind == expect_kind:
            return True
        if expect_kind == "runaway-cpu":
            time.sleep(max(0.0, 15.0 - (time.monotonic() - monkey._launched_at)))
            finding = monkey.check_idle_cpu(5.0)
            return bool(finding and finding.kind == expect_kind)
        return False
    except Exception:  # noqa: BLE001
        return False


def shrink(monkey: Monkey, actions: list[ActionRecord], failing_count: int, expect_kind: str,
           since_epoch: float, log: Callable[[str], None], max_attempts: int = 14) -> Optional[int]:
    """Binary search for the shortest prefix (1..failing_count) that still
    reproduces `expect_kind`. Assumes (as monkey tools generally do) that a
    finding, once triggered by a prefix, keeps triggering for any longer
    prefix built the same way -- not guaranteed, but a reasonable heuristic
    that a bounded number of relaunches can afford to test."""

    if not reproduces(monkey, actions, failing_count, expect_kind, since_epoch):
        log("shrink: full action log did not reproduce; preserving original evidence")
        return None
    if expect_kind == "runaway-cpu" and reproduces(monkey, actions, 0, expect_kind, since_epoch):
        return 0
    lo, hi = 0, failing_count
    attempts = 0
    while hi - lo > 1 and attempts < max_attempts:
        attempts += 1
        mid = (lo + hi) // 2
        log(f"shrink: trying prefix of {mid} actions ({attempts}/{max_attempts})")
        if reproduces(monkey, actions, mid, expect_kind, since_epoch):
            hi = mid
        else:
            lo = mid
    return hi


# --------------------------------------------------------------------------
# Reports.
# --------------------------------------------------------------------------


def write_report(findings_dir: Path, app: str, seed: int, finding: Finding, actions: list[ActionRecord],
                 minimal_count: Optional[int], stderr_tail: str, screenshot: Optional[Path]) -> Path:
    findings_dir.mkdir(parents=True, exist_ok=True)
    stamp = int(time.time() * 1000)
    stem = f"{app}-{finding.kind}-{seed}-{stamp}"
    actions_path = findings_dir / f"{stem}.actions.json"
    actions_path.write_text(json.dumps(
        [{"index": a.index, "kind": a.kind, "params": _jsonable(a.params), "note": a.note} for a in actions],
        indent=2,
    ))
    report_path = findings_dir / f"{stem}.md"
    minimal_text = (f"{minimal_count} action(s)" if minimal_count is not None else "not shrunk")
    report_path.write_text(
        f"# {app} — {finding.kind}\n\n"
        f"- Seed: {seed}\n"
        f"- Total actions before the finding: {len(actions)}\n"
        f"- Minimal repro: {minimal_text}\n"
        f"- Detail: {finding.detail}\n"
        f"- Action log: `{actions_path.name}`\n"
        f"- Screenshot: `{screenshot.name if screenshot else 'none'}`\n\n"
        "## Evidence\n\n```\n"
        f"{json.dumps(_jsonable(finding.evidence), indent=2)[:4000]}\n"
        "```\n\n## stderr/stdout tail\n\n```\n"
        f"{stderr_tail[-4000:]}\n"
        "```\n\n## Repro\n\n"
        "python3 scripts/behavior/monkey.py --bin-dir <dir> --niri <niri> \\\n"
        f"    --app {app} --seed {seed} --duration 0 \\\n"
        f"    --replay {actions_path}"
        + (f" --replay-count {minimal_count}" if minimal_count is not None else "")
        + (" --check-idle-cpu" if finding.kind == "runaway-cpu" else "")
        + "\n"
    )
    return report_path


def _jsonable(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: _jsonable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(v) for v in value]
    if isinstance(value, float):
        return round(value, 3)
    return value


# --------------------------------------------------------------------------
# The main loop.
# --------------------------------------------------------------------------


def monkey_session(run: run_window_move.Run, app: str, app_dirs: list[Path], seed: int, duration: float,
                   findings_dir: Path, max_findings: int, log: Callable[[str], None]) -> list[Path]:
    rng = random.Random(seed)
    home_root = run.work / "monkey-home"
    monkey = Monkey(run, app, app_dirs, home_root, log)
    try:
        monkey.launch()
    except MonkeyError as error:
        if monkey.process is None:
            raise  # missing binary is a setup failure, not an app finding
        finding = monkey.check_panic_in_log() or monkey.check_crashed()
        if finding is None:
            finding = Finding("hang", f"startup failed: {error}", 0, {})
        log(f"FINDING {finding.kind}: {finding.detail}")
        try:
            return [_handle_finding(run, monkey, app, [], seed, finding, findings_dir, log)]
        finally:
            monkey.stop()
    since_epoch = time.time()
    actions: list[ActionRecord] = []
    reports: list[Path] = []
    deadline = time.monotonic() + duration
    next_idle_check = time.monotonic() + 10
    index = 0
    try:
        while time.monotonic() < deadline and len(reports) < max_findings:
            if time.monotonic() >= next_idle_check and monkey.alive():
                monkey.record_memory()
                idle_finding = monkey.check_idle_cpu(5.0)
                next_idle_check = time.monotonic() + 15
                growth_finding = monkey.check_memory_growth()
                journal_finding = monkey.check_journal(since_epoch)
                for finding in (idle_finding, growth_finding, journal_finding):
                    if finding:
                        finding.action_count = len(actions)
                        log(f"FINDING {finding.kind}: {finding.detail}")
                        reports.append(_handle_finding(run, monkey, app, actions, seed, finding, findings_dir, log))
            if not monkey.alive() and app != "shell":
                finding = monkey.check_panic_in_log() or monkey.check_crashed()
                if finding:
                    finding.action_count = len(actions)
                    log(f"FINDING {finding.kind}: {finding.detail}")
                    reports.append(_handle_finding(run, monkey, app, actions, seed, finding, findings_dir, log))
                    break
                # Quit and closing the last window are ordinary user actions.
                # Record the restart so replay can cross the same boundary.
                log("app exited normally; relaunching")
                relaunch = ActionRecord(index=index, kind="relaunch", params={})
                actions.append(relaunch)
                index += 1
                monkey.relaunch()
                continue
            finished, action = call_with_timeout(lambda: monkey.decide(index, rng), 5.0)
            if not finished:
                finding = Finding("hang", "action selection did not respond within 5 s", len(actions), {})
                log(f"FINDING {finding.kind}: {finding.detail}")
                reports.append(_handle_finding(run, monkey, app, actions, seed, finding, findings_dir, log))
                break
            index += 1
            try:
                monkey.execute(action)
            except Exception as error:  # noqa: BLE001
                log(f"action {action.index} ({action.kind}) raised {error!r} (not itself a finding)")
            actions.append(action)
            time.sleep(rng.uniform(0.08, 0.3))
            finding = monkey.health_check()
            if finding:
                finding.action_count = len(actions)
                log(f"FINDING {finding.kind}: {finding.detail}")
                reports.append(_handle_finding(run, monkey, app, actions, seed, finding, findings_dir, log))
                if finding.kind in ("crash", "hang"):
                    break
    finally:
        monkey.stop()
    return reports


def replay_session(run: run_window_move.Run, app: str, app_dirs: list[Path], replay: Path, count: Optional[int],
                   check_idle_cpu: bool, idle_seconds: float, max_idle_cpu: float,
                   log: Callable[[str], None]) -> bool:
    """Replay recorded concrete actions in the same private compositor."""

    records = json.loads(replay.read_text())
    actions = [ActionRecord(**record) for record in records]
    if count is not None:
        actions = actions[:count]
    monkey = Monkey(run, app, app_dirs, run.work / "monkey-home", log)
    monkey.launch()
    try:
        for action in actions:
            monkey.execute(action)
            time.sleep(0.1)
            finding = monkey.health_check()
            if finding:
                log(f"REPLAY FINDING after action {action.index}: {finding.kind}: {finding.detail}")
                return True
        if check_idle_cpu:
            time.sleep(max(0.0, 15.0 - (time.monotonic() - monkey._launched_at)))
            finding = monkey.check_idle_cpu(idle_seconds, max_idle_cpu)
            if finding:
                log(f"REPLAY FINDING after idle sample: {finding.kind}: {finding.detail}")
                return True
        log(f"REPLAY NO FINDING after {len(actions)} actions")
        return False
    finally:
        monkey.stop()


def idle_session(run: run_window_move.Run, app: str, app_dirs: list[Path],
                 idle_seconds: float, max_idle_cpu: float, log: Callable[[str], None]) -> bool:
    """Measure a fresh, untouched app with its normal initial keyboard focus."""

    monkey = Monkey(run, app, app_dirs, run.work / "monkey-home", log)
    monkey.launch()
    try:
        time.sleep(max(0.0, 15.0 - (time.monotonic() - monkey._launched_at)))
        if monkey.sample() is None:
            raise MonkeyError("no process sample available for idle CPU check")
        finding = monkey.check_idle_cpu(idle_seconds, max_idle_cpu)
        if finding:
            log(f"IDLE FINDING: {finding.kind}: {finding.detail}")
            return True
        log("IDLE PASS")
        return False
    finally:
        monkey.stop()


def _handle_finding(run: run_window_move.Run, monkey: Monkey, app: str, actions: list[ActionRecord], seed: int,
                    finding: Finding, findings_dir: Path, log: Callable[[str], None]) -> Path:
    stderr_tail = monkey.tail_log()
    screenshot = None
    try:
        screenshot_path = findings_dir / f"{app}-{finding.kind}-{seed}-{int(time.time() * 1000)}.png"
        findings_dir.mkdir(parents=True, exist_ok=True)
        result = subprocess.run(["grim", str(screenshot_path)], env=run.env, capture_output=True, timeout=10)
        if result.returncode == 0:
            screenshot = screenshot_path
    except (OSError, subprocess.TimeoutExpired):
        pass
    minimal_count = None
    if app != "shell" and actions and finding.kind in ("crash", "hang", "error-dialog", "stuck-window", "runaway-cpu"):
        try:
            minimal_count = shrink(monkey, actions, len(actions), finding.kind, time.time(), log)
        except Exception as error:  # noqa: BLE001
            log(f"shrink failed: {error!r}")
    return write_report(findings_dir, app, seed, finding, actions, minimal_count, stderr_tail, screenshot)


# --------------------------------------------------------------------------
# Outer/inner process split, matching run_cold_surfaces.py / run_lulo.py.
# --------------------------------------------------------------------------


def inner(args: argparse.Namespace) -> int:
    args.frame_only = False
    args.geometry_only = False
    args.extra_zoom = False
    # run_window_move.Run.start() looks up dock/mission-control/wallpaper
    # under a single args.bin_dir; point it at --shell-bin-dir when the
    # caller keeps the shell surfaces separate from the app binaries, and
    # search both for the app-under-test and the extra shell surfaces below.
    run_args = argparse.Namespace(**vars(args))
    run_args.bin_dir = args.shell_bin_dir or args.bin_dir
    run = run_window_move.Run(run_args, args.inner)
    run.start()
    # run_window_move.Run itself tests window geometry, not accessibility,
    # so unlike run_lulo.py/run_niri_minimize.py it never flips this on --
    # without it the private a11y-bus launcher never forwards an app's
    # AccessKit registration, and every launch() times out waiting for one.
    subprocess.run(
        ["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "IsEnabled",
         "b", "true"],
        env=run.env, check=False, capture_output=True, timeout=10,
    )
    app_dirs = [Path(args.bin_dir)]
    if args.shell_bin_dir:
        app_dirs.append(Path(args.shell_bin_dir))

    def log(message: str) -> None:
        print(f"[{args.app} seed={args.seed}] {message}", flush=True)

    findings_dir = Path(args.findings_dir)
    try:
        if args.idle_only:
            return 1 if idle_session(run, args.app, app_dirs, args.idle_seconds,
                                     args.max_idle_cpu, log) else 0
        if args.replay:
            return 1 if replay_session(run, args.app, app_dirs, args.replay, args.replay_count,
                                       args.check_idle_cpu, args.idle_seconds, args.max_idle_cpu, log) else 0
        reports = monkey_session(run, args.app, app_dirs, args.seed, args.duration,
                                 findings_dir, args.max_findings, log)
        for report in reports:
            print(f"REPORT {report}", flush=True)
        return 1 if reports else 0
    finally:
        run.finish()


def outer(args: argparse.Namespace) -> int:
    for binary in ("sway", "dbus-run-session"):
        if not shutil.which(binary):
            raise SystemExit(f"{binary} is required")
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-monkey-"))
    env = run_lulo.isolated_environment(work)
    run_lulo.refuse_live_session(env)
    try:
        argv = ["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                "--inner", str(work), "--niri", args.niri, "--bin-dir", args.bin_dir,
                "--app", args.app, "--seed", str(args.seed), "--duration", str(args.duration),
                "--findings-dir", args.findings_dir, "--max-findings", str(args.max_findings)]
        if args.shell_bin_dir:
            argv += ["--shell-bin-dir", args.shell_bin_dir]
        if args.idle_only:
            argv += ["--idle-only"]
        if args.replay:
            argv += ["--replay", str(args.replay.resolve())]
        if args.replay_count is not None:
            argv += ["--replay-count", str(args.replay_count)]
        if args.check_idle_cpu:
            argv += ["--check-idle-cpu"]
        if args.idle_seconds != 5.0:
            argv += ["--idle-seconds", str(args.idle_seconds)]
        if args.max_idle_cpu != 50.0:
            argv += ["--max-idle-cpu", str(args.max_idle_cpu)]
        return subprocess.call(argv, env=env)
    finally:
        runtime = Path(env["XDG_RUNTIME_DIR"])
        if run_lulo.reap(runtime):
            time.sleep(1)
            run_lulo.reap(runtime)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)
        try:
            fcntl.flock(lock, fcntl.LOCK_UN)
        finally:
            lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--shell-bin-dir", default=None)
    parser.add_argument("--app", required=True, choices=TARGETS)
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--duration", type=float, default=1800.0, help="seconds to run (default 30 minutes)")
    parser.add_argument("--findings-dir", default=str(Path.home() / "lulo-monkey-findings"))
    parser.add_argument("--max-findings", type=int, default=5)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--replay", type=Path, help="replay an action log inside the private compositor")
    parser.add_argument("--replay-count", type=int, help="replay only the first N actions")
    parser.add_argument("--check-idle-cpu", action="store_true", help="sample CPU for five idle seconds after replay")
    parser.add_argument("--idle-only", action="store_true", help="measure an untouched app with its initial focus")
    parser.add_argument("--idle-seconds", type=float, default=5.0, help="idle CPU sample length during replay")
    parser.add_argument("--max-idle-cpu", type=float, default=50.0, help="CPU percent above which replay fails")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.idle_only and args.replay:
        parser.error("--idle-only and --replay cannot be combined")
    if args.seed is None:
        args.seed = random.SystemRandom().randrange(1, 2**31 - 1)
    print(f"seed={args.seed}", flush=True)
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    sys.exit(main())
