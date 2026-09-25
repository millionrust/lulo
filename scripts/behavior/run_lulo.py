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
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
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
}
KEEP_ENV = {"PATH", "LANG", "LC_ALL", "TERM", "USER", "LOGNAME", "SHELL", "CARGO_TARGET_DIR", "RUST_BACKTRACE", "RUST_LOG"}
TEXT_ROLES = {"text-field", "text-area", "search-field", "combo-box"}
DIALOG_ROLES = {"dialog", "alert", "file chooser"}


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
        "XDG_CURRENT_DESKTOP": "Lulo",
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


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "swaymsg", "dbus-run-session"):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    work = Path(tempfile.mkdtemp(prefix="lulo-behavior-"))
    try:
        env = isolated_environment(work)
        refuse_live_session(env)
        command = ["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()), "--inner", str(work), *argv]
        # The private bus daemon and the services it activates are chatty on
        # stderr; the inner runner reports on stdout.
        (work / "logs").mkdir(exist_ok=True)
        with open(work / "logs" / "session.log", "w") as log:
            status = subprocess.call(command, env=env, close_fds=True, stderr=log)
        if status not in (0, 1):
            print((work / "logs" / "session.log").read_text()[-3000:], file=sys.stderr)
        return status
    finally:
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"kept {work}", file=sys.stderr)


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


class LuloRun:
    def __init__(self, nested: Nested, sid: str, scenario: dict[str, Any], bins: list[Path], settle: float) -> None:
        self.nested = nested
        self.sid = sid
        self.scenario = scenario
        self.app = scenario["app"]
        self.settle = settle
        self.bins = bins
        home = Path(nested.env["HOME"])
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
        self.log = open(self.nested.logs / f"{self.sid.replace('/', '-')}.log", "w")
        self.process = subprocess.Popen(
            command, env=self.nested.env, stdout=self.log, stderr=subprocess.STDOUT, close_fds=True,
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

    def active_frame(self):
        pyatspi = atspi()
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
        titles = [name(f) for f in plain]
        front = name(active) if active is not None and role(active) not in DIALOG_ROLES else None
        if front in titles:
            titles.remove(front)
            titles.insert(0, front)
        return {"count": len(plain), "front": front, "titles": titles}

    def dialog_node(self):
        pyatspi = atspi()
        frame = self.active_frame()
        if frame is None:
            return None
        if role(frame) in DIALOG_ROLES:
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
        for child in descendants(node, limit=2000):
            r = role(child)
            if r in {"label", "static", "heading", "paragraph"} and name(child):
                texts.append(name(child))
            elif r in {"push button", "button"} and name(child):
                box = extents(child) or (0, 0, 0, 0)
                buttons.append((round(box[1] / 6), box[0], name(child)))
                if has_state(child, pyatspi.STATE_IS_DEFAULT):
                    default = name(child)
        buttons.sort()
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

    # -- steps -------------------------------------------------------------

    def ensure_alive(self) -> None:
        if self.process.poll() is not None:
            raise StepFailed(f"the app exited ({self.process.returncode}) during the scenario")

    def window_origin(self) -> tuple[int, int]:
        windows = [w for w in self.nested.windows() if w.get("pid") == self.process.pid]
        focused = [w for w in windows if w.get("focused")] or windows
        if not focused:
            return 0, 0
        rect = focused[0]["rect"]
        inner = focused[0].get("window_rect") or {"x": 0, "y": 0}
        return rect["x"] + inner.get("x", 0), rect["y"] + inner.get("y", 0)

    def click_item(self, label: str, button: str) -> None:
        frame = self.active_frame()
        target = None
        for node in descendants(frame, limit=4000) if frame is not None else []:
            if name(node) == label and role(node) in {"list item", "table row", "tree item", "table cell", "label", "static", "push button", "button"}:
                target = node
                break
        if target is None:
            raise StepFailed(f"no accessible item named {label!r} to click")
        box = extents(target)
        if not box:
            raise StepFailed(f"{label!r} has no on-screen extents")
        ox, oy = self.window_origin()
        x, y = ox + box[0] + min(40, box[2] // 2), oy + box[1] + box[3] // 2
        self.nested.input.click(x, y, OUTPUT_W, OUTPUT_H, button=button)

    def run_steps(self) -> dict[str, Any]:
        observations: dict[str, Any] = {}
        for index, step in enumerate(self.scenario["steps"]):
            self.ensure_alive()
            if "key" in step:
                self.nested.input.key(step["key"])
            elif "type" in step:
                self.nested.input.type_text(step["type"])
            elif "wait" in step:
                time.sleep(float(step["wait"]))
                continue
            elif "select" in step:
                self.click_item(step["select"], "left")
            elif "context" in step:
                self.click_item(step["context"], "left")
                time.sleep(0.3)
                self.click_item(step["context"], "right")
            elif "focus_desktop" in step:
                self.nested.input.click(OUTPUT_W // 4, OUTPUT_H // 2, OUTPUT_W, OUTPUT_H)
            elif "menu" in step:
                raise Unsupported("menu-bar steps need the top bar, which the nested runner does not start yet")
            elif "observe" in step:
                facts = {}
                for fact in step["facts"]:
                    facts[fact] = getattr(self, f"fact_{fact}")()
                observations[step["observe"]] = sc.finish_observation(self.scenario, step["observe"], facts)
                continue
            time.sleep(float(step.get("settle", self.settle)))
        return observations


def explore(run: LuloRun) -> None:
    """Print the app's accessible tree (for writing new scenarios)."""

    pyatspi = atspi()
    app = run.application()
    print(f"== {run.sid}: application {name(app) if app is not None else None!r}, "
          f"{len(run.frames())} top-level nodes, sway windows "
          f"{[(w.get('name'), w.get('focused')) for w in run.nested.windows()]}")
    for frame in run.frames():
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


def inner(args: argparse.Namespace) -> int:
    work = Path(args.inner)
    nested = Nested(work)
    bins = [Path(p) for p in args.bin_dir] + [Path(p) for p in args.shell_bin_dir]
    results = []
    try:
        for path in sc.scenario_paths(only=args.scenarios):
            sid = sc.scenario_id(path)
            scenario = sc.load(path)
            expected_path = sc.expectation_path(path)
            if not expected_path.exists() and not args.explore:
                continue
            run = LuloRun(nested, sid, scenario, bins, args.settle)
            actual: dict[str, Any] = {"format": sc.FORMAT, "scenario": sid, "observations": {}}
            try:
                run.setup()
                run.launch()
                if args.explore:
                    for step in scenario["steps"][: args.explore_steps]:
                        if "key" in step:
                            nested.input.key(step["key"])
                        elif "type" in step:
                            nested.input.type_text(step["type"])
                        time.sleep(float(step.get("settle", args.settle)))
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


def main(argv: Optional[list[str]] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("scenarios", nargs="*", help="area/name ids (default: every scenario with a .mac.json)")
    parser.add_argument("--bin-dir", action="append", default=[], help="directory with rmac-files etc. (repeatable)")
    parser.add_argument("--shell-bin-dir", action="append", default=[], help="directory with the shell's wallpaper binary")
    parser.add_argument("--output", help="write results JSON here (compare.py reads it)")
    parser.add_argument("--settle", type=float, default=0.8)
    parser.add_argument("--keep", action="store_true", help="keep the temporary directory and logs")
    parser.add_argument("--explore", action="store_true", help="print each scenario app's accessible tree instead")
    parser.add_argument("--explore-steps", type=int, default=0, help="with --explore: play this many steps first")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if not sys.platform.startswith("linux"):
        parser.error("run_lulo.py runs on Linux (the reference laptop or CI)")
    if args.inner:
        return inner(args)
    if not args.bin_dir:
        parser.error("--bin-dir is required")
    args.bin_dir = [str(Path(p).resolve()) for p in args.bin_dir]
    args.shell_bin_dir = [str(Path(p).resolve()) for p in args.shell_bin_dir]
    if args.output:
        args.output = str(Path(args.output).resolve())
    rebuilt = list(args.scenarios)
    for directory in args.bin_dir:
        rebuilt += ["--bin-dir", directory]
    for directory in args.shell_bin_dir:
        rebuilt += ["--shell-bin-dir", directory]
    if args.output:
        rebuilt += ["--output", args.output]
    rebuilt += ["--settle", str(args.settle), "--explore-steps", str(args.explore_steps)]
    if args.explore:
        rebuilt.append("--explore")
    return outer(args, rebuilt)


if __name__ == "__main__":
    sys.exit(main())
