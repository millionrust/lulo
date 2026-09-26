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
import subprocess
import sys
import tempfile
import time
from pathlib import Path

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
    window.set_child(Gtk.Label(label="Drag this GTK title bar"))
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

    def _track(self, process: subprocess.Popen) -> subprocess.Popen:
        self.children.append(process)
        return process

    def wait_for(self, predicate, timeout: float = 30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except Exception:  # noqa: BLE001
                value = None
            if value:
                return value
            time.sleep(0.2)
        return None

    def check(self, name: str, ok: bool, detail: str = "") -> None:
        self.results.append((ok, name))
        print(f"{'PASS' if ok else 'FAIL'} {name}{(': ' + detail) if detail else ''}", flush=True)

    def niri(self, *args: str):
        proc = subprocess.run([self.args.niri, "msg", "--json", *args], env=self.env,
                              capture_output=True, text=True, timeout=10)
        return json.loads(proc.stdout) if proc.stdout.strip() else None

    def windows(self):
        return self.niri("windows") or []

    def window(self, app_id: str):
        return next((w for w in self.windows() if w.get("app_id") == app_id), None)

    @staticmethod
    def geometry(window: dict) -> tuple[float, float, float, float]:
        layout = window.get("layout") or {}
        pos = layout.get("pos_in_scrolling_layout") or layout.get("pos_in_workspace_view") or [0, 0]
        size = layout.get("window_size") or layout.get("tile_size") or [500, 400]
        return float(pos[0]), float(pos[1]), float(size[0]), float(size[1])

    def start(self) -> None:
        self.locks = []
        for name in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / name, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        config = self.logs / "sway.conf"
        config.write_text("xwayland disable\ndefault_border none\noutput HEADLESS-1 mode 1440x900 position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(config)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise RuntimeError("headless Sway did not start")

        shell = (REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
        # Keep the shipped session rules and replace only the two packaged service paths.
        bins = Path(self.args.bin_dir)
        shell = shell.replace("/usr/libexec/rmac/rmac-dock", str(bins / "dock"))
        shell = shell.replace("/usr/libexec/rmac/rmac-mission-control", str(bins / "mission-control"))
        niri_config = self.logs / "niri.kdl"
        niri_config.write_text(shell)
        validate = subprocess.run([self.args.niri, "validate", "-c", str(niri_config)], env=self.env,
                                  capture_output=True, text=True)
        self.check("shipped shell.kdl validates", validate.returncode == 0, validate.stderr[-300:])
        existing = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(niri_config)], "niri",
                   {"WAYLAND_DISPLAY": self.sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
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
        # The shipped shell starts these services. Their HOME/XDG state is private to this run.
        self.spawn([str(bins / "dock")], "dock", {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
        self.spawn([str(bins / "mission-control"), "--service"], "mission-control",
                   {"VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
        time.sleep(3)
        self.pointer = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display})

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
        start, end = (x + width * .5, y + 18), (x + width * .5 + 150, y + 100)
        self.pointer.drag(start, end, self.width, self.height)
        moved = self.wait_for(lambda: self.window(app_id), 3)
        new_geometry = self.geometry(moved) if moved else (x, y, width, height)
        changed = abs(new_geometry[0] - x) > 30 or abs(new_geometry[1] - y) > 30
        self.check(f"{title} title-bar drag changes niri position", changed,
                   f"{(x, y)} -> {new_geometry[:2]}")
        time.sleep(2)
        settled = self.window(app_id)
        settled_geometry = self.geometry(settled) if settled else (0, 0, 0, 0)
        stays = abs(settled_geometry[0] - new_geometry[0]) < 2 and abs(settled_geometry[1] - new_geometry[1]) < 2
        self.check(f"{title} remains at dragged position", stays, str(settled_geometry[:2]))
        process.terminate()
        process.wait(10)

    def resize_settings(self) -> None:
        process = self.spawn([str(Path(self.args.bin_dir) / "rmac-system-settings")], "settings")
        window = self.wait_for(lambda: self.window("org.rmac.SystemSettings"), 40)
        self.check("Settings mapped floating", bool(window and window.get("is_floating")))
        if window:
            time.sleep(1)
            x, y, width, height = self.geometry(window)
            # Grab just inside the lower right edge and move inward to shrink.
            self.pointer.drag((x + width - 3, y + height - 3),
                              (x + width - 240, y + height - 200), self.width, self.height)
            resized = self.wait_for(lambda: self.window("org.rmac.SystemSettings"), 4)
            resized_geometry = self.geometry(resized) if resized else (x, y, width, height)
            shrunk = resized_geometry[2] < width - 30 and resized_geometry[3] < height - 30
            self.check("Settings resizes from its lower-right edge", shrunk,
                       f"{(width, height)} -> {resized_geometry[2:]}")
            sx, sy, sw, sh = resized_geometry
            self.pointer.drag((sx + sw * .5, sy + 18), (sx + sw * .5 + 140, sy + 90), self.width, self.height)
            moved = self.wait_for(lambda: self.window("org.rmac.SystemSettings"), 4)
            mg = self.geometry(moved) if moved else resized_geometry
            changed = abs(mg[0] - sx) > 30 or abs(mg[1] - sy) > 30
            self.check("shrunk Settings window can still move", changed, f"{(sx, sy)} -> {mg[:2]}")
        process.terminate()
        process.wait(10)

    def run(self) -> int:
        self.start()
        self.assert_move("org.rmac.Calculator", "Calculator", [str(Path(self.args.bin_dir) / "rmac-calculator")])
        self.resize_settings()
        self.assert_move("org.example.WindowMoveTest", "GTK", [sys.executable, str(Path(__file__).resolve()), "--gtk-window"])
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
    work = Path(tempfile.mkdtemp(prefix="lulo-window-move-"))
    env = run_lulo.isolated_environment(work)
    run_lulo.refuse_live_session(env)
    try:
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                "--inner", str(work), "--niri", args.niri, "--bin-dir", args.bin_dir], env=env)
    finally:
        runtime = Path(env["XDG_RUNTIME_DIR"])
        if run_lulo.reap(runtime):
            time.sleep(1)
            run_lulo.reap(runtime)
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--gtk-window", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.gtk_window:
        return gtk_window()
    if args.inner:
        return Run(args, args.inner).run()
    return outer(args)


if __name__ == "__main__":
    sys.exit(main())
