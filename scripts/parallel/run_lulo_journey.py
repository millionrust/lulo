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
FORBIDDEN = {"Shut Down", "Restart", "Log Out", "Sleep", "Empty Bin", "Empty Trash", "Wi-Fi On", "Wi-Fi Off"}


def private_bus(work: Path, env: dict, bins: Path) -> Path:
    services = work / "dbus-services"
    services.mkdir()
    for name in ("org.a11y.Bus.service", "org.freedesktop.portal.Desktop.service"):
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
        config.write_text(shell)
        run_window_move.REPO = private_root
        self.session.start()
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.session.env, capture_output=True, timeout=10, check=False)
        self.session.output = next(iter(self.session.niri("outputs") or {}), "winit")
        self.sampler = screencopy.Screencopy(self.session.pointer)
        bins = Path(self.args.bin_dir)
        for binary in ("rmac-top-bar", "rmac-wallpaper"):
            if (bins / binary).exists():
                self.session.spawn([str(bins / binary)], binary)
        if any(step.get("click") == "Control Centre" for step in self.data["steps"]):
            self.session.spawn([str(bins / "rmac-quick-settings")], "quick-settings")
            self.session.wait_for(lambda: (self.session.runtime / "rmac/shortcut-quick-settings.sock").exists(), 10)
        if any(step.get("key") == "cmd-space" for step in self.data["steps"]):
            self.session.spawn([str(bins / "rmac-launcher")], "launcher")
            self.session.wait_for(lambda: (self.session.runtime / "rmac/shortcut-launcher.sock").exists(), 10)
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

    def capture(self, destination: Path, full=False, fast=False):
        geom = self.capture_region(full)
        # Sway's wlroots screencopy captures the niri surface in one frame.
        # niri's own screencopy waits for its next software-rendered frame.
        # This session has exactly one Sway output. `-o` overrides `-g` in
        # the installed grim, so omit it to preserve window-region shots.
        cmd = ["grim"]
        if geom:
            cmd += ["-g", f"{geom[0]},{geom[1]} {geom[2]}x{geom[3]}"]
        if fast:
            cmd += ["-t", "ppm"]
        cmd.append(str(destination))
        env = {**self.session.env, "WAYLAND_DISPLAY": self.session.sway_display}
        result = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=10)
        if result.returncode:
            raise RuntimeError(f"grim failed: {result.stderr[-200:]}")

    def capture_region(self, full=False):
        if not full and self.window():
            x, y, w, h = self.session.geometry(self.window())
            x, y = max(0, int(x)), max(0, int(y))
            w, h = min(int(w), self.session.width - x), min(int(h), self.session.height - y)
            if w > 50 and h > 50:
                return (x + self.session.niri_rect[0], y + self.session.niri_rect[1], w, h)
        return (self.session.niri_rect[0], self.session.niri_rect[1], self.session.width, self.session.height)

    def launch(self, step: dict):
        app = step["launch"]
        self.current = app
        if app in self.apps:
            window = self.window()
            if window:
                subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(window["id"])],
                               env=self.session.env, check=True, capture_output=True)
            return
        command = [str(Path(self.args.bin_dir) / BINARIES[app])]
        if app == "files":
            command += ["--path", str(self.sandbox / step.get("path", "."))]
        elif app == "preview":
            command += [str(self.sandbox / step["file"])]
        if not Path(command[0]).is_file():
            raise RuntimeError(f"missing binary {command[0]}")
        process = self.session.spawn(command, app)
        self.apps[app] = process
        if not self.session.wait_for(lambda: self.window(), 30):
            raise RuntimeError(f"{app} did not open a nested niri window; see {self.session.logs / (app + '.log')}")

    def click(self, label: str, target: str | None = None, attempt: int = 0):
        if label in FORBIDDEN:
            raise RuntimeError(f"refusing destructive or toggle control {label!r}")
        if target == "Dock" and label == "Files":
            self.current = "files"
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
        window = self.window()
        dock = next((p for p in self.session.children
                     if isinstance(p.args, list) and Path(p.args[0]).name == "dock"), None)
        while stack:
            node = stack.pop()
            try:
                if node is None:
                    continue
                if node.name == label or (node.name or "").startswith(label + ","):
                    pid = node.getApplication().get_process_id()
                    if target == "Dock" and (dock is None or pid != dock.pid):
                        stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
                        continue
                    belongs = current is not None and pid == current.pid
                    box = node.queryComponent().getExtents(
                        pyatspi.WINDOW_COORDS if belongs else pyatspi.DESKTOP_COORDS)
                    if box.width > 2 and box.height > 2 and box.x >= 0 and box.y >= 0:
                        role = node.getRoleName()
                        rank = 0 if role in {"list item", "tree item", "table row", "table cell"} else 1
                        rank += 0 if belongs else 2
                        origin = self.session.geometry(window)[:2] if belongs and window else (0, 0)
                        candidates.append((rank, -box.width * box.height,
                                           box.x + origin[0], box.y + origin[1], box))
                stack.extend(node.getChildAtIndex(i) for i in range(node.childCount))
            except Exception:
                continue
        if candidates:
            _rank, _area, bx, by, box = min(candidates, key=lambda item: item[:2])
            x, y = self.session.parent_point(bx + box.width / 2, by + box.height / 2)
            self.session.pointer.click(x, y, self.session.parent_width, self.session.parent_height)
            return
        if target == "Dock" and label == "Files":
            x, y = self.session.parent_point(self.session.width / 2 - 303, self.session.height - 48)
            self.session.pointer.click(x, y, self.session.parent_width, self.session.parent_height)
            return
        status_x = {"Lulo": 26, "Battery": self.session.width - 296,
                    "Wi-Fi": self.session.width - 251,
                    "Control Centre": self.session.width - 182}
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
                self.click(label)
                time.sleep(0.15)
        elif kind == "key":
            if step[kind] in {"power", "ctrl-power", "cmd-alt-escape", "cmd-alt-s"}:
                raise RuntimeError("refusing session or device-control shortcut")
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
            self.session.drag((x + ax, y + ay), (x + ax + dx, y + ay + dy))
            moved = self.session.wait_for(
                lambda: (candidate := self.window())
                if candidate and (abs(self.session.geometry(candidate)[0] - x) > 20 or
                                  abs(self.session.geometry(candidate)[1] - y) > 20) else None, 4)
            after = self.session.geometry(self.window()) if self.window() else None
            if not moved and (after is None or
                              (abs(after[0] - x) <= 20 and abs(after[1] - y) <= 20)):
                raise RuntimeError(f"nested niri reported no window movement after drag: {(x, y)} -> {after}")

    def run(self, name: str) -> dict:
        target = self.out / name
        target.mkdir(parents=True, exist_ok=True)
        result = {"journey": name, "platform": "lulo", "steps": [], "status": "passed"}
        pending = None
        timing = None
        issues = []
        try:
            self.start()
            for index, step in enumerate(self.data["steps"]):
                kind = next(iter(journey.ACTIONS.intersection(step)))
                if kind == "wait":
                    time.sleep(float(step[kind]))
                elif kind == "shot":
                    destination = target / f"{len(result['steps']):02d}-{step[kind]}.png"
                    self.capture(destination, full=step.get("scope") == "full")
                    if self.current == "text-editor" and step[kind] == "saved" and not list(self.sandbox.glob("Parallel Journey Sandbox*")):
                        issues.append({"index": index, "action": pending,
                                       "error": "Save did not create the document in the journey sandbox"})
                    if step.get("expect_window") and not any(
                        w.get("app_id") == step["expect_window"] and w.get("is_focused")
                        for w in self.session.windows()
                    ):
                        issues.append({"index": index, "action": pending,
                                       "error": f"expected {step['expect_window']} window is not focused"})
                    result["steps"].append({"name": step[kind], "image": destination.name,
                                            "region": self.capture_region(step.get("scope") == "full"),
                                            "action": pending, **(timing or {})})
                else:
                    full = kind == "launch" or step.get("scope") == "full"
                    pending = {kind: step[kind], "index": index}
                    try:
                        timing = journey.measure(lambda p: self.capture(p, full=full, fast=True),
                                                 lambda: self.action(step), self.scratch,
                                                 probe=lambda: self.sampler.fingerprint(self.capture_region(full)))
                    except RuntimeError as error:
                        if kind not in {"click", "menu"}:
                            raise
                        issues.append({"index": index, "action": pending, "error": str(error)})
                        timing = {"first_change_ms": None, "settled_ms": None,
                                  "samples": 0, "actual_hz": None, "timed_out": False,
                                  "error": str(error)}
            if issues:
                result.update(status="partial", issues=issues)
        except (RuntimeError, OSError, subprocess.SubprocessError, wlinput.InjectorError) as error:
            result.update(status="failed", error=str(error))
        finally:
            if hasattr(self.session, "pointer"):
                self.session.finish()
            else:
                for process in reversed(self.session.children):
                    if process.poll() is None:
                        process.terminate()
            (target / "result.json").write_text(json.dumps(result, indent=2) + "\n")
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
                    if source.is_file():
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
                    (links / alias).symlink_to(bins / source)
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
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inner:
        data = journey.load(journey.JOURNEYS / f"{args.journeys[0]}.json")
        result = Driver(args, args.inner, data, args.output).run(args.journeys[0])
        print(f"{result['status']} {result['journey']}: {len(result['steps'])} shots", flush=True)
        return 0 if result["status"] == "passed" else 1
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
