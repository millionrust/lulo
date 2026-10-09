#!/usr/bin/env python3
"""Lulo OS's half of the cross-platform shell check: the fixed scene.

    python3 scripts/behavior/run_shell_scene.py --bin-dir DIR --output PNG \\
        [--niri PATH] [--keep]

ADR 0023, "Phase 3 revised: shared shell views": the menu bar, the Dock and
the desktop are one view on Lulo OS and on Windows. This draws the scene
`scripts/windows/shell_scene.py` draws on Windows, so
`scripts/compare_shell_scenes.py` can hold the two to the same pixels:

- a 1024 x 768 output at scale 1 (the Windows runner's screen at 100 %);
- the built-in Lulo wallpaper in the light appearance;
- the clock at Thursday 8 October, 9:41 AM (`RMAC_SHELL_SCENE_TIME`);
- the status items' fixed readings (`RMAC_SHELL_SCENE=1`): Wi-Fi at full
  signal, the battery at 80 %, sound at half volume;
- the Dock pinning the nine Lulo apps that also build for Windows, from the
  same desktop entries and icons, nothing running, the Trash empty;
- an empty Desktop folder.

Headless Sway is only the parent display; niri runs nested in it with the
shipped packaging/rmac-session/shell.kdl, as run_desktop_first_paint.py
does, and `wallpaper`, `top-bar` and `dock` start from --bin-dir. Nothing
touches a live session: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, and `wayland-0`/`wayland-1` held.
"""

from __future__ import annotations

import argparse
import fcntl
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
OUTPUT_W, OUTPUT_H = 1024, 768
SCENE_TIME = "2026-10-08T09:41"
# The Lulo apps that also run on Windows, in the Dock's order.
SCENE_PINS = [
    "org.rmac.Files",
    "org.rmac.Notes",
    "org.rmac.TextEditor",
    "org.rmac.Terminal",
    "org.rmac.SystemSettings",
    "org.rmac.Calculator",
    "org.rmac.Clock",
    "org.rmac.Weather",
    "org.rmac.Preview",
]
HICOLOR_INDEX = (
    "[Icon Theme]\nName=Hicolor\nComment=Fallback icon theme\nDirectories=scalable/apps\n\n"
    "[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=1024\nContext=Applications\nType=Scalable\n"
)


def scene_entry(text: str) -> str:
    """A Lulo app's desktop entry for the scene: the same name and icon,
    no TryExec (the scene draws the Dock, it starts nothing)."""
    return "".join(line + "\n" for line in text.splitlines() if not line.startswith("TryExec="))


def prepare_profile(home: Path) -> None:
    """The scene's settings, desktop entries, icons and empty Desktop."""
    data = home / ".local/share"
    applications = data / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    icons = data / "icons/hicolor"
    (icons / "scalable/apps").mkdir(parents=True, exist_ok=True)
    (icons / "index.theme").write_text(HICOLOR_INDEX)
    for app in SCENE_PINS:
        entry = REPO / "packaging/rmac-apps/applications" / f"{app}.desktop"
        (applications / entry.name).write_text(scene_entry(entry.read_text(encoding="utf-8")))
    for icon in (REPO / "packaging/rmac-apps/icons").glob("*.svg"):
        shutil.copy(icon, icons / "scalable/apps" / icon.name)
    dock = data / "rmac/dock/icons"
    dock.mkdir(parents=True, exist_ok=True)
    for icon in (REPO / "crates/rmac-dock/assets/icons").glob("*.svg"):
        shutil.copy(icon, dock / icon.name)
    settings = home / ".config/rmac"
    settings.mkdir(parents=True, exist_ok=True)
    pins = ",".join(f'"{app}"' for app in SCENE_PINS)
    (settings / "shell.json").write_text(f'{{"version": 3, "settings": {{"pinned_apps": [{pins}]}}}}\n')
    for item in (home / "Desktop").iterdir():
        if item.is_dir():
            shutil.rmtree(item)
        else:
            item.unlink()


def scene_environment() -> dict[str, str]:
    environment = {"RMAC_SHELL_SCENE": "1", "RMAC_SHELL_SCENE_TIME": SCENE_TIME}
    # The Lulo wallpaper's artwork, as the session package installs it.
    environment["RMAC_WALLPAPER_DIR"] = str(REPO / "packaging/rmac-session/wallpapers")
    return environment


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.work = work
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.out = work / "logs"
        self.out.mkdir(exist_ok=True)
        self.children: list[subprocess.Popen] = []
        self.home = Path(self.env["HOME"])

    def spawn(self, argv: list[str], name: str, extra: dict[str, str] | None = None) -> subprocess.Popen:
        env = {**self.env, **(extra or {})}
        process = subprocess.Popen(argv, env=env, stdout=open(self.out / f"{name}.log", "a"),
                                   stderr=subprocess.STDOUT, close_fds=True)
        self.children.append(process)
        return process

    @staticmethod
    def wait_for(predicate, timeout: float = 30, step: float = 0.2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except Exception:  # noqa: BLE001
                value = None
            if value:
                return value
            time.sleep(step)
        return None

    def start(self) -> None:
        self.locks = []
        for taken in ("wayland-0.lock", "wayland-1.lock"):
            handle = open(self.runtime / taken, "w")
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.locks.append(handle)
        sway_conf = self.out / "sway.conf"
        sway_conf.write_text("xwayland disable\ndefault_border none\n"
                             f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H}@60Hz position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"})
        sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not sway_display:
            raise SystemExit("sway did not start")
        config = self.out / "niri.kdl"
        config.write_text((REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
                          + '\noutput "winit" {\n    scale 1\n}\n')
        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": sway_display, "LIBGL_ALWAYS_SOFTWARE": "1"})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})
        self.output = self.wait_for(self.niri_output) or "winit"

    def niri_output(self):
        import json

        result = subprocess.run([self.args.niri, "msg", "-j", "outputs"], env=self.env,
                                capture_output=True, text=True, timeout=10)
        outputs = json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else {}
        return next(iter(outputs), None)

    def binary(self, *names: str) -> Path:
        for name in names:
            candidate = Path(self.args.bin_dir) / name
            if candidate.is_file():
                return candidate
        raise SystemExit(f"none of {', '.join(names)} in {self.args.bin_dir}")

    def run(self) -> int:
        prepare_profile(self.home)
        ready = self.work / "ready"
        ready.mkdir(exist_ok=True)
        try:
            self.start()
            surface_env = {**scene_environment(), "VK_ICD_FILENAMES": LAVAPIPE,
                           "RMAC_WALLPAPER_READY_FILE": str(ready / "wallpaper"),
                           "RMAC_TOP_BAR_READY_FILE": str(ready / "top-bar"),
                           "RMAC_DOCK_READY_FILE": str(ready / "dock")}
            self.spawn([str(self.binary("wallpaper", "rmac-wallpaper"))], "wallpaper", surface_env)
            self.spawn([str(self.binary("top-bar", "rmac-top-bar"))], "top-bar", surface_env)
            self.spawn([str(self.binary("dock", "rmac-dock"))], "dock", surface_env)
            for surface in ("wallpaper", "top-bar", "dock"):
                if not self.wait_for(lambda surface=surface: (ready / surface).exists(), 60):
                    print(f"FAIL {surface} did not report a configured surface", flush=True)
            if self.args.open:
                # The panel's own process opens it at start-up
                # (RMAC_SHELL_SCENE_OPEN), as lulo-shell does on Windows.
                # Only the profile's apps, as on Windows: not the runner's
                # own /usr/share/applications (vim and the like).
                empty = self.work / "no-system-data"
                empty.mkdir(exist_ok=True)
                panel_env = {**surface_env, "RMAC_SHELL_SCENE_OPEN": self.args.open,
                             "RMAC_SPOTLIGHT_FRAME_DIR": str(ready),
                             "XDG_DATA_DIRS": str(empty)}
                apps = Path(self.args.app_bin_dir or self.args.bin_dir)
                if self.args.open.startswith("spotlight:"):
                    self.spawn([str(apps / "rmac-launcher")], "launcher", panel_env)
                    if not self.wait_for(lambda: next(ready.glob("show-*.ready"), None), 60):
                        print("FAIL Spotlight did not draw", flush=True)
                elif self.args.open == "control-centre":
                    self.spawn([str(apps / "rmac-quick-settings")], "quick-settings", panel_env)
                else:
                    raise SystemExit(f"unknown --open {self.args.open!r}")
            # Icons and the wallpaper decode off the UI thread; let them land.
            time.sleep(self.args.settle)
            output = Path(self.args.output)
            output.parent.mkdir(parents=True, exist_ok=True)
            result = subprocess.run(["grim", "-o", str(self.output), str(output)],
                                    env=self.env, capture_output=True, check=False)
            if result.returncode != 0 or not output.is_file():
                print(f"FAIL capture: {result.stderr.decode(errors='replace')[-300:]}", flush=True)
                return 1
            print(f"PASS captured the Lulo OS scene to {output}", flush=True)
            return 0
        finally:
            for process in reversed(self.children):
                if process.poll() is None:
                    process.terminate()
            for process in reversed(self.children):
                try:
                    process.wait(5)
                except subprocess.TimeoutExpired:
                    process.kill()
            for log in sorted(self.out.glob("*.log")):
                if log.stat().st_size:
                    print(f"--- {log.name}\n{log.read_text(errors='replace')[-3000:]}", flush=True)


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", args.niri):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-shell-scene-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        return subprocess.call(command, env=env, close_fds=True)
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1.0)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            run_lulo.remove_tree(work)
        journey_lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--bin-dir", required=True, help="directory with wallpaper, top-bar and dock")
    parser.add_argument("--output", required=True, help="where the scene's PNG goes")
    parser.add_argument("--settle", type=float, default=6.0, help="seconds after the surfaces are up")
    parser.add_argument("--open", default="",
                        help="spotlight:<query> or control-centre: open that panel in the scene")
    parser.add_argument("--app-bin-dir", default="",
                        help="directory with rmac-launcher and rmac-quick-settings (for --open)")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.output = str(Path(args.output).resolve())
    args.bin_dir = str(Path(args.bin_dir).resolve())
    if args.app_bin_dir:
        args.app_bin_dir = str(Path(args.app_bin_dir).resolve())
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
