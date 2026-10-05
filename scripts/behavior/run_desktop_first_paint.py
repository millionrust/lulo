#!/usr/bin/env python3
"""The desktop's icons paint on their own, with no input, in a nested niri.

    python3 scripts/behavior/run_desktop_first_paint.py --bin-dir DIR \\
        [--niri PATH] [--rounds N] [--capture-dir DIR] [--keep]

GPUI loads img() assets (the desktop's folder and document icons, image
previews) on the background executor and paints them from a next-frame
callback. The vendored Wayland backend parks an idle window's frame loop
(docs/decisions/0013-gpui-linux-patch.md); a parked inactive window once left
that callback unrun, so the owner saw a desktop folder's name but not its icon
until a click repainted the screen.

The wallpaper only sits idle under niri: elsewhere its output watcher fails
and it re-polls outputs every 500 ms, which hides the bug. So, as
run_niri_minimize.py does, this starts headless Sway only as a parent display,
runs niri nested in it with the shipped packaging/rmac-session/shell.kdl and
starts `wallpaper` (or `rmac-wallpaper`) from --bin-dir with a private HOME
whose ~/Desktop holds a folder, a text file and large PNG screenshots.

The login case comes first: the owner's desktop (a folder and four
full-screen screenshots) with nothing else open. Its later rounds drop the
frames after the desktop's first one (RMAC_GPUI_TEST_UNPRESENTED_DRAWS), as
the reference laptop's niri does through Mesa's suboptimal swapchain, and the
folder icon must still paint without input.

Each round launches the desktop, waits with no input at all and captures the
output, then clicks empty desktop (the only input, into this run's Sway) and
captures again. The icons must already be there before the click. A second
part of each round adds the first folder while the desktop sits idle.

Isolation is run_lulo.py's: a private dbus-run-session, a temporary HOME and
XDG_RUNTIME_DIR, `wayland-0`/`wayland-1` held so no socket is ever named like
the live session's, and input only through wlinput.py into this run's Sway.
"""

from __future__ import annotations

import argparse
import fcntl
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import run_lulo  # noqa: E402
import wlinput  # noqa: E402

LAVAPIPE = "/usr/share/vulkan/icd.d/lvp_icd.json"
OUTPUT_W, OUTPUT_H = 1280, 800
# Staggered sizes so each preview's decode finishes at its own moment.
SCREENSHOT_SIZES = ((2560, 1600), (1920, 1080), (1440, 900), (1280, 800), (960, 600), (640, 400))


# The owner's desktop when the bug was reported: one folder and four
# full-screen screenshots, with the icon positions their desktop.json held.
OWNER_FOLDER = "hi"
OWNER_SCREENSHOTS = (
    "Screenshot 2026-09-30 at 8.26.03\u202fPM.png",
    "Screenshot 2026-09-30 at 8.30.53\u202fPM.png",
    "Screenshot 2026-09-30 at 8.32.43\u202fPM.png",
    "Screenshot 2026-09-30 at 8.34.15\u202fPM.png",
)
OWNER_POSITIONS = {
    OWNER_FOLDER: (34.0, 41.0),
    OWNER_SCREENSHOTS[0]: (28.734375, 152.00781),
    OWNER_SCREENSHOTS[1]: (34.0, 265.0),
    OWNER_SCREENSHOTS[2]: (34.0, 377.0),
    OWNER_SCREENSHOTS[3]: (34.0, 489.0),
}

MAGENTA = (255, 0, 255)
PICTURES = 4
# A picture preview fits the 64-point icon slot (a 64 px square at scale 1;
# measured 2900 px or more once labels overlap); each new one must add this.
MIN_SWATCH_PIXELS = 2000
# Main-thread wake-ups an idle, inactive desktop may take in IDLE_SECONDS.
IDLE_SECONDS = 10
MAX_IDLE_WAKEUPS = 10
# The same bar compare() uses for "the folder icon paints at all".
FOLDER_ICON_THRESHOLD = 400
# How long time-to-icon polling waits before giving up, and how often it
# samples. 20 ms is below a 60 Hz frame (16.6 ms) so a capture rarely
# straddles two frames; grim itself costs a few ms per call.
TIME_TO_ICON_TIMEOUT = 5.0
TIME_TO_ICON_INTERVAL = 0.02


def write_solid_png(path: Path, width: int, height: int, rgb: tuple[int, int, int]) -> None:
    row = b"\x00" + bytes(rgb) * width
    body = zlib.compress(row * height)

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                     + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", body) + chunk(b"IEND", b""))


def write_screenshot_png(path: Path, width: int, height: int) -> None:
    """A full-screen screenshot stand-in: incompressible, so it decodes as
    slowly as a real one, but with no blue, so its preview never counts as
    folder-blue pixels."""
    noise = os.urandom(width * height * 2)
    rows = []
    for y in range(height):
        start = y * width * 2
        chunk = noise[start:start + width * 2]
        pixels = bytearray(width * 3)
        pixels[0::3] = chunk[0::2]
        pixels[1::3] = chunk[1::2]
        rows.append(b"\x00" + bytes(pixels))

    def chunk_of(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                     + chunk_of(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk_of(b"IDAT", zlib.compress(b"".join(rows), 1)) + chunk_of(b"IEND", b""))


def magenta_pixels(pixels: bytes) -> int:
    return sum(
        1
        for index in range(0, len(pixels), 3)
        if pixels[index] >= 235 and pixels[index + 1] <= 30 and pixels[index + 2] >= 235
    )


FOCUS_HOLDER_RULE = """
window-rule {
    match app-id="^lulo-focus-holder$"
    open-floating true
    default-column-width { fixed 160; }
    default-window-height { fixed 90; }
    default-floating-position x=16 y=16 relative-to="bottom-left"
}
"""


class Run:
    def __init__(self, args: argparse.Namespace, work: Path) -> None:
        self.args = args
        self.work = work
        self.env = dict(os.environ)
        run_lulo.refuse_live_session(self.env)
        self.runtime = Path(self.env["XDG_RUNTIME_DIR"])
        self.out = work / "logs"
        self.out.mkdir(exist_ok=True)
        self.capture_dir = Path(args.capture_dir) if args.capture_dir else None
        self.children: list[subprocess.Popen] = []
        self.results: list[tuple[str, bool, str]] = []
        self.desktop: subprocess.Popen | None = None
        # Time from spawn to the folder icon's blue crossing FOLDER_ICON_THRESHOLD,
        # one entry per login round that reached it at all.
        self.time_to_icon_ms: list[float] = []
        self.home = work / "home"
        self.desktop_dir = self.home / "Desktop"
        for sub in (".config", ".local/share", ".local/state", ".cache", "Desktop", "Documents"):
            (self.home / sub).mkdir(parents=True, exist_ok=True)
        (self.home / ".config/user-dirs.dirs").write_text(
            'XDG_DESKTOP_DIR="$HOME/Desktop"\nXDG_DOCUMENTS_DIR="$HOME/Documents"\n'
        )

    def check(self, name: str, ok, detail: str = "") -> None:
        self.results.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name} {detail}".rstrip(), flush=True)

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
                             f"output HEADLESS-1 mode {OUTPUT_W}x{OUTPUT_H}@120Hz position 0 0\n")
        self.spawn(["sway", "--unsupported-gpu", "--config", str(sway_conf)], "sway",
                   {"WLR_BACKENDS": "headless", "WLR_HEADLESS_OUTPUTS": "1",
                    "WLR_LIBINPUT_NO_DEVICES": "1",
                    # --gpu: the parent renders on the GPU too, so niri gets
                    # hardware EGL and offers dmabuf feedback with a main
                    # device, as the reference laptop's niri does.
                    **({"WLR_RENDERER": "gles2", "WLR_RENDER_DRM_DEVICE": "/dev/dri/renderD128"}
                       if self.args.gpu else {"WLR_RENDERER": "pixman"})})
        self.sway_display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None))
        if not self.sway_display:
            raise SystemExit("sway did not start")
        config = self.out / "niri.kdl"
        # The reference laptop runs its panel at scale 2.
        config.write_text((REPO / "packaging/rmac-session/shell.kdl").read_text(encoding="utf-8")
                          + f'\noutput "winit" {{\n    scale {self.args.scale}\n}}\n'
                          + FOCUS_HOLDER_RULE)
        before = {p.name for p in self.runtime.glob("wayland-*")}
        self.spawn([self.args.niri, "-c", str(config)], "niri",
                   {"WAYLAND_DISPLAY": self.sway_display,
                    **({} if self.args.gpu else {"LIBGL_ALWAYS_SOFTWARE": "1"})})
        socket = self.wait_for(lambda: next(iter(self.runtime.glob("niri.*.sock")), None))
        display = self.wait_for(lambda: next(
            (p.name for p in self.runtime.glob("wayland-*")
             if not p.name.endswith(".lock") and p.name not in before), None))
        if not (socket and display):
            raise SystemExit("niri did not start")
        self.env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": str(socket)})
        outputs = self.wait_for(lambda: self.niri_outputs()) or {}
        self.output = next(iter(outputs), "winit")
        subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus",
                        "org.a11y.Status", "IsEnabled", "b", "true"],
                       env=self.env, check=False, capture_output=True, timeout=10)
        self.keys = wlinput.Wayland({**self.env, "WAYLAND_DISPLAY": self.sway_display,
                                     "RMAC_BEHAVIOR_NESTED": "1"})

    def start_focus_holder(self) -> None:
        # In a real session an app window holds keyboard focus and the
        # desktop is inactive; GPUI paces an inactive window's next-frame
        # callbacks at 30 fps, which is where the idle frame loop once lost
        # them. A small floating terminal plays that app.
        if shutil.which("foot") is None:
            raise SystemExit("foot is required to hold keyboard focus off the desktop")
        self.spawn(["foot", "--app-id=lulo-focus-holder", "sleep", "3600"], "foot")
        if not self.wait_for(self.focus_holder_id):
            raise SystemExit("the focus-holding terminal did not map")

    def focus_holder_id(self):
        import json

        result = subprocess.run([self.args.niri, "msg", "-j", "windows"], env=self.env,
                                capture_output=True, text=True, timeout=10)
        windows = json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else []
        return next((w["id"] for w in windows if w.get("app_id") == "lulo-focus-holder"), None)

    def focus_terminal(self) -> None:
        window = self.focus_holder_id()
        if window is not None:
            subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(window)],
                           env=self.env, capture_output=True, timeout=10, check=False)

    def niri_outputs(self):
        import json

        result = subprocess.run([self.args.niri, "msg", "-j", "outputs"], env=self.env,
                                capture_output=True, text=True, timeout=10)
        return json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else None

    def vulkan_env(self) -> dict[str, str]:
        # --gpu leaves GPUI on the machine's own Vulkan driver (the laptop's
        # Intel iGPU); otherwise lavapipe, as on CI.
        return {} if self.args.gpu else {"VK_ICD_FILENAMES": LAVAPIPE}

    def binary(self) -> Path:
        for name in ("wallpaper", "rmac-wallpaper"):
            candidate = Path(self.args.bin_dir) / name
            if candidate.is_file():
                return candidate
        raise SystemExit(f"no wallpaper binary in {self.args.bin_dir}")

    # -- the desktop -----------------------------------------------------------

    def launch_desktop(self, focus_terminal: bool = True, extra: dict[str, str] | None = None) -> None:
        if focus_terminal:
            self.focus_terminal()
        self.desktop = self.spawn([str(self.binary())], "wallpaper", {**self.desktop_env(), **(extra or {})})

    def desktop_env(self) -> dict[str, str]:
        return {
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "XDG_DATA_HOME": str(self.home / ".local/share"),
            "XDG_STATE_HOME": str(self.home / ".local/state"),
            "XDG_CACHE_HOME": str(self.home / ".cache"),
            **self.vulkan_env(),
        }

    def stop_desktop(self) -> None:
        if self.desktop is not None and self.desktop.poll() is None:
            self.desktop.terminate()
            try:
                self.desktop.wait(8)
            except subprocess.TimeoutExpired:
                self.desktop.kill()
                self.desktop.wait(5)
        self.desktop = None

    def capture(self, label: str) -> bytes | None:
        path = self.out / f"desktop-{label}.ppm"
        result = subprocess.run(["grim", "-t", "ppm", "-o", str(self.output), str(path)],
                                env=self.env, capture_output=True, check=False)
        if result.returncode != 0:
            self.check(f"capture {label}", False, result.stderr.decode(errors="replace")[-200:])
            return None
        if self.capture_dir is not None:
            self.capture_dir.mkdir(parents=True, exist_ok=True)
            subprocess.run(["grim", "-o", str(self.output), str(self.capture_dir / f"desktop-{label}.png")],
                           env=self.env, capture_output=True, check=False)
        return run_lulo.read_ppm(path)[2]

    def click_empty_desktop(self) -> None:
        # Sway's only window is niri, filling the output; the lower-left
        # quarter of the desktop is empty (icons sit at the top right).
        self.keys.click(OUTPUT_W // 4, OUTPUT_H - 80, OUTPUT_W, OUTPUT_H)

    def compare(self, label: str, idle: bytes | None, clicked: bytes | None) -> None:
        if idle is None or clicked is None or len(idle) != len(clicked):
            self.check(f"{label}: icons paint without input", False, "missing or mismatched captures")
            return
        blue_idle = run_lulo.folder_blue_pixels(idle)
        blue_clicked = run_lulo.folder_blue_pixels(clicked)
        detail = f"(folder-blue pixels {blue_idle} before input, {blue_clicked} after a click)"
        if blue_clicked < 400:
            self.check(f"{label}: the folder icon paints at all", False, detail)
            return
        self.check(f"{label}: icons paint without input",
                   blue_idle >= blue_clicked * 0.9, detail)

    def settle_desktop(self, wait: Optional[float] = None) -> None:
        """Wait --settle (or `wait`) seconds, then until the desktop process has used no
        CPU for a while: every picture and icon has finished loading (on a
        busy CI runner the folder artwork alone can take five seconds), so
        a capture taken now shows what the desktop will show with no input."""
        time.sleep(self.args.settle if wait is None else wait)
        if self.desktop is None:
            return

        def cpu_ticks() -> int:
            fields = Path(f"/proc/{self.desktop.pid}/stat").read_text().rsplit(")", 1)[1].split()
            return int(fields[11]) + int(fields[12])  # utime + stime

        deadline = time.monotonic() + 30.0
        try:
            history = [cpu_ticks()]
            while time.monotonic() < deadline:
                time.sleep(0.5)
                history.append(cpu_ticks())
                # Quiet for 1.5 s: at most 2 ticks (20 ms) of CPU.
                if len(history) >= 4 and history[-1] - history[-4] <= 2:
                    return
        except (OSError, IndexError, ValueError):
            return

    def check_idle_wakeups(self, label: str, pid: Optional[int]) -> None:
        """No polling: an idle, inactive surface's main thread sleeps."""
        if pid is None:
            return

        def switches() -> int:
            status = Path(f"/proc/{pid}/task/{pid}/status").read_text()
            return sum(int(line.split()[1]) for line in status.splitlines() if "ctxt_switches" in line)

        time.sleep(2.0)
        before = switches()
        time.sleep(IDLE_SECONDS)
        wakeups = switches() - before
        self.check(f"idle {label} main thread stays asleep", wakeups <= MAX_IDLE_WAKEUPS,
                   f"({wakeups} context switches in {IDLE_SECONDS} s, want <= {MAX_IDLE_WAKEUPS})")

    def measure_time_to_icon(self, start: float) -> tuple[Optional[float], Optional[bool], int]:
        """Polls captures from `start` (the desktop's spawn time) until the
        folder icon's blue crosses FOLDER_ICON_THRESHOLD, with no settle
        wait and no input: the time-to-icon the owner actually sees. Returns
        (milliseconds to the icon, whether the very first capture already
        had it, captures taken). macOS shows desktop icons instantly; this
        polls tightly enough (TIME_TO_ICON_INTERVAL, under one frame) that a
        folder icon queued even one frame behind the rest of the desktop
        shows up as a non-zero, later sample rather than vanishing into a
        single settled capture."""
        deadline = time.monotonic() + TIME_TO_ICON_TIMEOUT
        first_frame_had_icon: Optional[bool] = None
        sample = 0
        while time.monotonic() < deadline:
            if self.desktop is None or self.desktop.poll() is not None:
                return None, first_frame_had_icon, sample
            frame = self.capture(f"time-to-icon-{sample}")
            if frame is not None:
                blue = run_lulo.folder_blue_pixels(frame)
                has_icon = blue >= FOLDER_ICON_THRESHOLD
                if first_frame_had_icon is None:
                    first_frame_had_icon = has_icon
                if has_icon:
                    return (time.monotonic() - start) * 1000.0, first_frame_had_icon, sample + 1
                sample += 1
            time.sleep(TIME_TO_ICON_INTERVAL)
        return None, first_frame_had_icon, sample

    def reset_desktop_folder(self, with_folder: bool) -> None:
        for entry in self.desktop_dir.iterdir():
            shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
        if with_folder:
            (self.desktop_dir / "Projects").mkdir()
        (self.desktop_dir / "Notes.txt").write_text("Private desktop fixture.\n")
        # Like the owner's desktop: full-size PNG screenshots, each its own
        # slow img() decode finishing at a different moment.
        for index, (width, height) in enumerate(SCREENSHOT_SIZES):
            run_lulo.write_noise_png(self.desktop_dir / f"Screenshot {index}.png", width, height)

    def reset_owner_desktop(self) -> None:
        """The owner's desktop: the folder "hi" and four screenshots, copied
        from --fixture-dir when given (read only), else generated at the
        laptop's 1920x1080, with their saved icon positions."""
        import json

        for entry in self.desktop_dir.iterdir():
            shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
        (self.desktop_dir / OWNER_FOLDER).mkdir()
        source = Path(self.args.fixture_dir) if self.args.fixture_dir else None
        for name in OWNER_SCREENSHOTS:
            if source is not None and (source / name).is_file():
                shutil.copyfile(source / name, self.desktop_dir / name)
            else:
                write_screenshot_png(self.desktop_dir / name, 1920, 1080)
        settings = self.home / ".config/rmac/desktop.json"
        settings.parent.mkdir(parents=True, exist_ok=True)
        settings.write_text(json.dumps({
            "arrangement": "none",
            "use_stacks": False,
            "view": {"icon_size": 64.0, "grid_spacing": 54.0, "text_size": 12.0},
            "positions": {name: {"from_right": right, "top": top}
                          for name, (right, top) in OWNER_POSITIONS.items()},
            "widgets": [],
            "next_widget_id": 0,
        }, indent=2))

    def run_login(self, attempt: int) -> None:
        """Login: the owner's desktop and nothing else open, so no window
        holds keyboard focus. The folder must paint with no input.

        Every round after the first also drops the frames right after the
        desktop's first one, as the reference laptop does: niri sends a
        surface its scan-out dmabuf feedback once it has composited it, Mesa
        then reports the swapchain suboptimal and the next frame is drawn but
        never presented. That frame is the one that adds the folder icon,
        which finishes loading just after the first frame; unless the
        backend redraws a frame it could not present, the icon stays blank
        until a click."""
        self.reset_owner_desktop()
        companions = [self.spawn([binary], Path(binary).name, self.desktop_env())
                      for binary in filter(None, (self.args.companions or "").split(","))]
        dropped = self.args.unpresented_draws if attempt > 0 else 0
        label = f"login {attempt}" + (f" ({dropped} unpresented frames)" if dropped else "")
        start = time.monotonic()
        self.launch_desktop(focus_terminal=False,
                            extra={"RMAC_GPUI_TEST_UNPRESENTED_DRAWS": str(dropped)} if dropped else None)
        icon_ms, first_frame_had_icon, samples = self.measure_time_to_icon(start)
        if icon_ms is not None:
            self.time_to_icon_ms.append(icon_ms)
            print(f"{label}: time to icon {icon_ms:.1f} ms ({samples} captures)", flush=True)
        self.check(f"{label}: the folder icon is present in the very first captured frame",
                   bool(first_frame_had_icon),
                   f"(time to icon {icon_ms:.1f} ms)" if icon_ms is not None
                   else f"(icon never painted in {TIME_TO_ICON_TIMEOUT:.1f}s, {samples} captures)")
        self.settle_desktop()
        if self.desktop is None or self.desktop.poll() is not None:
            self.check("desktop starts", False, (self.out / "wallpaper.log").read_text()[-400:])
            return
        idle = self.capture(f"login-{attempt}")
        self.click_empty_desktop()
        time.sleep(1.5)
        self.compare(label, idle, self.capture(f"login-clicked-{attempt}"))
        if dropped and attempt == self.args.login_rounds - 1:
            # Redrawing a dropped frame must not leave the loop running.
            self.check_idle_wakeups("desktop after unpresented frames", self.desktop.pid)
        settings = self.home / ".config/rmac/desktop.json"
        self.stop_desktop()
        for companion in companions:
            companion.terminate()
            companion.wait(8)
        settings.unlink(missing_ok=True)

    def run_probe(self) -> None:
        """Deterministic part: the img-paint probe's swatches load a fixed
        delay after the frame that requested them, on a surface that never
        takes keyboard focus (like the Dock and the menu bar)."""
        delays = [int(delay) for delay in self.args.probe_delays.split(",")]
        probe = subprocess.Popen(
            [self.args.probe, "--delays", self.args.probe_delays],
            env={**self.env, **self.vulkan_env()}, stdin=subprocess.PIPE,
            stdout=open(self.out / "img-paint.log", "w"), stderr=subprocess.STDOUT, close_fds=True,
        )
        self.children.append(probe)
        time.sleep(3.0)
        # A 48-point square; allow a quarter of one for scaled edges, so a
        # missing swatch always fails.
        full = int(48 * 48 * self.args.scale * self.args.scale)
        try:
            for index, delay in enumerate(delays):
                if probe.poll() is not None:
                    self.check("img-paint probe runs", False, f"exited {probe.returncode}")
                    return
                probe.stdin.write(b"\n")
                probe.stdin.flush()
                time.sleep(1.0)
                shown = self.capture(f"probe-{index}")
                count = magenta_pixels(shown) if shown is not None else 0
                expected = (index + 1) * full - full // 4
                self.check(f"probe swatch {index} ({delay} ms load): paints without input",
                           count >= expected, f"({count} magenta pixels, want >= {expected})")
            if probe.poll() is None:
                self.check_idle_wakeups("img-paint probe", probe.pid)
        finally:
            probe.stdin.close()
            probe.terminate()
            probe.wait(5)

    def run(self) -> int:
        try:
            self.start()
            for attempt in range(self.args.login_rounds):
                self.run_login(attempt)
            if self.args.login_only:
                return self.finish()
            self.start_focus_holder()
            if self.args.probe:
                self.run_probe()
            for attempt in range(self.args.rounds):
                self.reset_desktop_folder(with_folder=True)
                self.launch_desktop()
                self.settle_desktop()
                if self.desktop is None or self.desktop.poll() is not None:
                    self.check("desktop starts", False, (self.out / "wallpaper.log").read_text()[-400:])
                    break
                idle = self.capture(f"startup-{attempt}")
                self.click_empty_desktop()
                time.sleep(1.5)
                self.compare(f"startup {attempt}", idle, self.capture(f"startup-clicked-{attempt}"))
                self.stop_desktop()

                # The first folder that appears while the desktop sits idle.
                self.reset_desktop_folder(with_folder=False)
                self.launch_desktop()
                time.sleep(self.args.settle)
                self.focus_terminal()
                time.sleep(1.0)
                (self.desktop_dir / "Projects").mkdir()
                self.settle_desktop(3.0)
                idle = self.capture(f"new-folder-{attempt}")
                self.click_empty_desktop()
                time.sleep(1.5)
                self.compare(f"new folder {attempt}", idle, self.capture(f"new-folder-clicked-{attempt}"))

                # Small pictures decode in a millisecond or two, so each
                # preview's load finishes right after the frame that drew
                # its label: the tightest race with the idle frame loop.
                self.focus_terminal()
                time.sleep(1.0)
                previous = magenta_pixels(self.capture(f"swatch-{attempt}-none") or b"")
                for picture in range(PICTURES):
                    path = self.desktop_dir / f"Swatch {attempt}-{picture}.png"
                    write_solid_png(path, 64, 64, MAGENTA)
                    time.sleep(2.5)
                    shown = self.capture(f"swatch-{attempt}-{picture}")
                    count = magenta_pixels(shown) if shown is not None else 0
                    expected = previous + int(MIN_SWATCH_PIXELS * self.args.scale * self.args.scale)
                    self.check(f"picture {attempt}-{picture}: preview paints without input",
                               count >= expected, f"({count} magenta pixels, want >= {expected})")
                    previous = count
                if attempt == self.args.rounds - 1:
                    self.focus_terminal()
                    self.check_idle_wakeups("desktop", self.desktop.pid if self.desktop else None)
                self.stop_desktop()
        finally:
            self.stop_desktop()
        return self.finish()

    def finish(self) -> int:
        try:
            self.keys.close()
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
        if self.time_to_icon_ms:
            times = self.time_to_icon_ms
            print(f"time to icon: min {min(times):.1f} ms, max {max(times):.1f} ms, "
                  f"mean {sum(times) / len(times):.1f} ms, over {len(times)} round(s)", flush=True)
        failed = [result for result in self.results if not result[1]]
        print(f"\n{len(self.results) - len(failed)}/{len(self.results)} checks passed", flush=True)
        return 1 if failed or not self.results else 0


def outer(args: argparse.Namespace, argv: list[str]) -> int:
    for tool in ("sway", "grim", "dbus-run-session", args.niri):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is required")
    journey_lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(journey_lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-desktop-paint-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        for key in ("WLR_BACKENDS", "WLR_HEADLESS_OUTPUTS", "WLR_LIBINPUT_NO_DEVICES", "WLR_RENDERER",
                    "LIBGL_ALWAYS_SOFTWARE", "VK_ICD_FILENAMES"):
            env.pop(key, None)  # set per process instead: niri, Sway and GPUI differ
        services = work / "dbus-services"
        services.mkdir()
        # The AT-SPI bus, as in a Lulo session: every GPUI window's AccessKit
        # adapter is active there (org.a11y.Status.IsEnabled is true).
        a11y_service = Path("/usr/share/dbus-1/services/org.a11y.Bus.service")
        if a11y_service.exists():
            shutil.copy(a11y_service, services / a11y_service.name)
        config = work / "session.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>unix:dir={work}</listen><auth>EXTERNAL</auth>"
            f"<servicedir>{services}</servicedir>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        (work / "logs").mkdir(exist_ok=True)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), *argv]
        with open(work / "logs" / "session.log", "w") as log:
            return subprocess.call(command, env=env, close_fds=True, stderr=log)
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
    parser.add_argument("--bin-dir", required=True, help="directory with this branch's wallpaper binary")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--probe", help="the shell's img-paint probe binary (deterministic load delays)")
    parser.add_argument("--probe-delays", default="0,10,25,40,50,60,80,120",
                        help="comma-separated asset load delays in ms, one swatch each")
    parser.add_argument("--scale", type=float, default=2.0, help="niri output scale (the laptop uses 2)")
    parser.add_argument("--settle", type=float, default=5.0, help="idle seconds before each capture")
    parser.add_argument("--capture-dir", help="also save each capture as PNG here")
    parser.add_argument("--login-rounds", type=int, default=3,
                        help="rounds of the login case: the owner's desktop, nothing else open")
    parser.add_argument("--unpresented-draws", type=int, default=20,
                        help="frames after the first that login rounds after the first draw without presenting")
    parser.add_argument("--login-only", action="store_true", help="run only the login case")
    parser.add_argument("--companions", help="comma-separated shell binaries started with the desktop "
                        "in the login case (the menu bar, the Dock)")
    parser.add_argument("--fixture-dir", help="copy the owner's screenshots from here (read only)")
    parser.add_argument("--gpu", action="store_true",
                        help="use the machine's GPU for niri and GPUI instead of llvmpipe/lavapipe")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.capture_dir:
        args.capture_dir = str(Path(args.capture_dir).resolve())
    if args.probe:
        args.probe = str(Path(args.probe).resolve())
    if args.inner:
        return Run(args, args.inner).run()
    argv = [a for a in sys.argv[1:] if a != "--keep"]
    return outer(args, argv)


if __name__ == "__main__":
    sys.exit(main())
