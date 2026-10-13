"""Windows' half of the cross-platform shell check: the fixed scene.

    python scripts/windows/shell_scene.py --bin-dir target/debug --output PNG

ADR 0023, "Phase 3 revised: shared shell views": the menu bar, the Dock and
the desktop are one view on Lulo OS and on Windows. This draws, on the
runner's desktop, the scene `scripts/behavior/run_shell_scene.py` draws in a
nested niri, so `scripts/compare_shell_scenes.py` can hold the two to the
same pixels: the screen at 100 % (1024 x 768 on the runner), the built-in
Lulo wallpaper, the clock at Thursday 8 October 9:41 AM, the status items'
fixed readings, the Dock pinning the nine Lulo apps that build for Windows,
nothing running, the Recycle Bin empty and an empty Desktop folder.

The runner's own windows (its console) are hidden for the capture and shown
again afterwards: nothing runs in the Lulo OS scene either.

`lulo-session.exe` starts the Lulo layer with private profile folders and
`LULO_SHELL_TRACE=1`; once lulo-shell says it is ready and its surfaces had
time to decode their pictures, the screen is captured and `lulo-session
--stop` gives the desktop back.
"""

from __future__ import annotations

import argparse
import ctypes
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SCENE_TIME = "2026-10-08T09:41"
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
READY_TIMEOUT_SECONDS = 90.0
SHERB_NOCONFIRMATION = 0x1
SHERB_NOPROGRESSUI = 0x2
SHERB_NOSOUND = 0x4
SW_HIDE = 0
SW_SHOWNOACTIVATE = 4
GWL_EXSTYLE = -20
WS_EX_TOOLWINDOW = 0x80


def empty_recycle_bin() -> None:
    shell32 = ctypes.windll.shell32
    shell32.SHEmptyRecycleBinW(None, None, SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)


def hide_other_windows() -> list[int]:
    """Hide every visible, titled top-level window and return them, so the
    scene shows only the desktop, the bar and the Dock."""
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    hidden: list[int] = []
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def visit(hwnd, _lparam):
        if user32.IsWindowVisible(hwnd) and user32.GetWindowTextLengthW(hwnd) > 0:
            name = ctypes.create_unicode_buffer(64)
            user32.GetClassNameW(hwnd, name, 64)
            if name.value not in ("Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"):
                hidden.append(int(hwnd))
        return True

    user32.EnumWindows(callback_type(visit), 0)
    for hwnd in hidden:
        user32.ShowWindow(wintypes.HWND(hwnd), SW_HIDE)
    print(f"hid {len(hidden)} window(s) for the scene")
    return hidden


def app_window(pid: int) -> tuple[int, int, int, int] | None:
    """The screen box of `pid`'s visible app window: its client area, the
    window as drawn, without any shadow round it. Shell surfaces (tool
    windows) are skipped."""
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    found: list[tuple[int, int, int, int]] = []
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def visit(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value != pid or not user32.IsWindowVisible(hwnd):
            return True
        if user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW:
            return True
        # The client area is the visible window (its swap chain): Lulo's
        # own frame keeps resize borders and its shadow outside it
        # (op/win-corners), so the frame bounds are larger than the window.
        rect = wintypes.RECT()
        user32.GetClientRect(hwnd, ctypes.byref(rect))
        origin = wintypes.POINT(0, 0)
        user32.ClientToScreen(hwnd, ctypes.byref(origin))
        width, height = rect.right - rect.left, rect.bottom - rect.top
        if width > 50 and height > 50:
            found.append((origin.x, origin.y, origin.x + width, origin.y + height))
        return True

    user32.EnumWindows(callback_type(visit), 0)
    return found[0] if found else None


def bring_forward(pid: int) -> None:
    """Make `pid`'s window the key window, as niri does with a new window in
    the Lulo OS scene."""
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def visit(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd) and not (
            user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW
        ):
            user32.SwitchToThisWindow(hwnd, True)
            return False
        return True

    user32.EnumWindows(callback_type(visit), 0)


def show_windows(windows: list[int]) -> None:
    from ctypes import wintypes

    for hwnd in windows:
        ctypes.windll.user32.ShowWindow(wintypes.HWND(hwnd), SW_SHOWNOACTIVATE)


def scene_profile(root: Path, default_dock: bool = False) -> dict[str, str]:
    """Private Lulo profile folders holding the scene's Dock pins."""
    folders = {
        "XDG_CONFIG_HOME": root / "config",
        "XDG_DATA_HOME": root / "data",
        "XDG_STATE_HOME": root / "state",
        "XDG_CACHE_HOME": root / "cache",
        "RMAC_DESKTOP_DIR": root / "Desktop",
    }
    for folder in folders.values():
        folder.mkdir(parents=True, exist_ok=True)
    settings = folders["XDG_CONFIG_HOME"] / "rmac"
    settings.mkdir(parents=True, exist_ok=True)
    if default_dock:
        # A fresh profile's own pins (`ShellSettings::default`).
        (settings / "shell.json").write_text('{"version": 3, "settings": {}}\n', encoding="utf-8")
    else:
        pins = ",".join(f'"{app}"' for app in SCENE_PINS)
        (settings / "shell.json").write_text(
            f'{{"version": 3, "settings": {{"pinned_apps": [{pins}]}}}}\n', encoding="utf-8"
        )
    return {name: str(path) for name, path in folders.items()}


def wait_for_ready(log: Path, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if log.exists() and "lulo-shell: ready at" in log.read_text(errors="replace"):
            return True
        time.sleep(0.25)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--settle", type=float, default=8.0)
    parser.add_argument("--normal", action="store_true",
                        help="a memory probe: start Lulo as a user would, not in the fixed scene")
    parser.add_argument("--no-windows-apps", action="store_true",
                        help="a memory probe: leave out Windows' own apps (LULO_NO_WINDOWS_APPS)")
    parser.add_argument("--launch", default="",
                        help="a probe: start this Lulo app (an exe in --bin-dir) once Lulo is ready")
    parser.add_argument("--drop-shadows", action="store_true",
                        help="a memory probe: Windows' window shadows on, as shell_smoke.py turns them")
    parser.add_argument("--desktop-items", action="store_true",
                        help="a memory probe: a folder and a note on the desktop, as shell_smoke.py has")
    parser.add_argument("--smoke-profile", action="store_true",
                        help="a memory probe: profile folders as shell_smoke.py sets them "
                        "(APPDATA and LOCALAPPDATA private, no XDG overrides)")
    parser.add_argument("--env", action="append", default=[],
                        help="a memory probe: KEY=VALUE added to the shell's environment")
    parser.add_argument("--keep-windows", action="store_true",
                        help="a memory probe: leave the runner's own windows showing")
    parser.add_argument("--real-status", action="store_true",
                        help="a memory probe: the machine's own Wi-Fi, sound and battery readings")
    parser.add_argument("--desktop", default="", help="a memory probe: show this folder on the desktop")
    parser.add_argument("--open", default="",
                        help="spotlight:<query>, control-centre, apps or notification-centre: "
                        "open that panel in the scene")
    parser.add_argument("--default-dock", action="store_true",
                        help="the Dock keeps a fresh profile's own pins")
    parser.add_argument("--window-output", default="",
                        help="with --launch: also save the app's window alone here")
    parser.add_argument("--scheme", choices=["", "light", "dark"], default="",
                        help="the appearance the scene's profile asks for")
    args = parser.parse_args()

    from PIL import ImageGrab

    bin_dir = Path(args.bin_dir).resolve()
    output = Path(args.output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    empty_recycle_bin()
    work = Path(tempfile.mkdtemp(prefix="lulo-scene-"))
    environment = {
        **os.environ,
        **scene_profile(work, args.default_dock),
        "RMAC_SHELL_SCENE": "1",
        "RMAC_SHELL_SCENE_TIME": SCENE_TIME,
        "LULO_SHELL_TRACE": "1",
    }
    if args.open:
        environment["RMAC_SHELL_SCENE_OPEN"] = args.open
    if args.scheme:
        # The same theme store file run_shell_scene.py writes on Lulo OS.
        theme = Path(environment["XDG_CONFIG_HOME"]) / "rmac" / "theme.json"
        theme.parent.mkdir(parents=True, exist_ok=True)
        theme.write_text(
            f'{{"version": 1, "preferences": {{"color_scheme": "{args.scheme}"}}}}\n', encoding="utf-8"
        )
    if args.smoke_profile:
        for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            environment.pop(name, None)
        environment["APPDATA"] = str(work / "Roaming")
        environment["LOCALAPPDATA"] = str(work / "Local")
        settings = work / "Roaming" / "Lulo" / "Config" / "rmac"
        settings.mkdir(parents=True, exist_ok=True)
        (work / "Local").mkdir(parents=True, exist_ok=True)
        pins = ",".join(f'"{app}"' for app in SCENE_PINS)
        (settings / "shell.json").write_text(
            f'{{"version": 3, "settings": {{"pinned_apps": [{pins}]}}}}\n', encoding="utf-8"
        )
    shadows_were = None
    if args.drop_shadows:
        value = ctypes.c_int(0)
        ctypes.windll.user32.SystemParametersInfoW(0x1024, 0, ctypes.byref(value), 0)
        shadows_were = value.value
        ctypes.windll.user32.SystemParametersInfoW(0x1025, 0, ctypes.c_void_p(1), 0)
        print(f"window shadows on (were {shadows_were})")
    if args.desktop_items:
        desk = Path(environment["RMAC_DESKTOP_DIR"])
        (desk / "Lulo Check Folder").mkdir(parents=True, exist_ok=True)
        (desk / "Lulo Check Note.txt").write_text("Lulo mode's desktop check\n", encoding="utf-8")
    for pair in args.env:
        key, _, value = pair.partition("=")
        environment[key] = value
    if args.normal:
        environment.pop("RMAC_SHELL_SCENE", None)
        environment.pop("RMAC_SHELL_SCENE_TIME", None)
    if args.no_windows_apps:
        environment["LULO_NO_WINDOWS_APPS"] = "1"
    if args.real_status:
        environment["RMAC_SHELL_SCENE_STATUS"] = "real"
    if args.desktop:
        environment["RMAC_DESKTOP_DIR"] = args.desktop
    log = work / "session.log"
    hidden = [] if args.keep_windows else hide_other_windows()
    session = subprocess.Popen(
        [str(bin_dir / "lulo-session.exe")],
        env=environment,
        stdout=open(log, "w"),
        stderr=subprocess.STDOUT,
    )
    try:
        if not wait_for_ready(log, READY_TIMEOUT_SECONDS):
            print(f"FAIL lulo-shell did not get ready:\n{log.read_text(errors='replace')[-4000:]}")
            return 1
        launched = None
        if args.launch:
            launched = subprocess.Popen([str(bin_dir / args.launch)], env=environment)
            if args.window_output:
                deadline = time.monotonic() + 60
                while app_window(launched.pid) is None and time.monotonic() < deadline:
                    time.sleep(0.25)
                if app_window(launched.pid) is None:
                    print(f"FAIL {args.launch} opened no window")
                    return 1
                bring_forward(launched.pid)
        time.sleep(args.settle)
        screen = ImageGrab.grab()
        screen.save(output)
        print(f"PASS captured the Windows scene to {output}")
        if launched is not None and args.window_output:
            box = app_window(launched.pid)
            if box is None:
                print(f"FAIL {args.launch}'s window went away")
                return 1
            window_output = Path(args.window_output).resolve()
            window_output.parent.mkdir(parents=True, exist_ok=True)
            screen.crop(box).save(window_output)
            print(f"PASS {args.launch}'s window {box} to {window_output}")
        return 0
    finally:
        subprocess.run([str(bin_dir / "lulo-session.exe"), "--stop"], env=environment, timeout=60, check=False)
        try:
            session.wait(30)
        except subprocess.TimeoutExpired:
            session.kill()
        if launched is not None:
            launched.kill()
        show_windows(hidden)
        if shadows_were == 0:
            ctypes.windll.user32.SystemParametersInfoW(0x1025, 0, ctypes.c_void_p(0), 0)
        text = log.read_text(errors='replace')
        for line in text.splitlines():
            if line.startswith(("lulo-shell: memory", "lulo-shell: bar title", "lulo-shell: menus from",
                                "lulo-shell: dock tile")):
                print(line)
        print(f"--- session log\n{text[-4000:]}")


if __name__ == "__main__":
    sys.exit(main())
