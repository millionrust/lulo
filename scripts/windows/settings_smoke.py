"""System Settings on Windows: every pane shows content, and every way out quits.

Usage: python scripts/windows/settings_smoke.py <bin-dir> [--screenshots <dir>]

Settings runs with private %APPDATA%/%LOCALAPPDATA% folders and
`LULO_SETTINGS_TRACE=1`, under which it prints the panes its sidebar lists
on this PC once its hardware scan is in (`settings: panes <id>=<name>;…`):
Windows-only gaps and absent hardware (no battery, no Bluetooth radio…) are
already left out there, so the list is what the person sees.

1. For each listed pane, a second launch `rmac-system-settings --pane <id>`
   hands the pane to the running window (one process, as on the Mac). The
   window title must become "<pane> - Settings", and the detail column — right of the sidebar, below the toolbar —
   must show content: at least `MIN_CONTENT_SHARE` of its pixels differ
   from its background. An empty pane (only the background) fails.
2. Each way to close the window must also end the process, within
   `EXIT_TIMEOUT_SECONDS`: a click on the red traffic light, Ctrl+W, Ctrl+Q
   (the Windows spelling of ⌘W and ⌘Q) and Alt+F4. Settings is relaunched
   for each.

Windows only: it calls user32 through ctypes and Pillow for the captures.
"""

from __future__ import annotations

import argparse
import ctypes
import os
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes
from pathlib import Path

WINDOW_TIMEOUT_SECONDS = 60.0
PANES_TIMEOUT_SECONDS = 30.0
PANE_SETTLE_SECONDS = 2.0
EXIT_TIMEOUT_SECONDS = 8.0
#: The share of the detail column that must differ from its background.
MIN_CONTENT_SHARE = 0.008
#: Settings' sidebar column and toolbar, in points (settings_style.rs).
SIDEBAR_WIDTH = 244
TOOLBAR_HEIGHT = 52
APP = "rmac-system-settings"

VK_MENU = 0x12
VK_CONTROL = 0x11
VK_F4 = 0x73
KEYEVENTF_KEYUP = 0x0002


def user32():
    return ctypes.windll.user32


def visible_windows(pid: int) -> list[tuple[int, str]]:
    found: list[tuple[int, str]] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32().GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value != pid or not user32().IsWindowVisible(hwnd):
            return True
        length = user32().GetWindowTextLengthW(hwnd)
        buffer = ctypes.create_unicode_buffer(length + 1)
        user32().GetWindowTextW(hwnd, buffer, length + 1)
        if buffer.value:
            found.append((hwnd, buffer.value))
        return True

    user32().EnumWindows(collect, 0)
    return found


def window_title(hwnd: int) -> str:
    length = user32().GetWindowTextLengthW(hwnd)
    buffer = ctypes.create_unicode_buffer(length + 1)
    user32().GetWindowTextW(hwnd, buffer, length + 1)
    return buffer.value


def client_rect_on_screen(hwnd: int) -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    user32().GetClientRect(hwnd, ctypes.byref(rect))
    point = wintypes.POINT(0, 0)
    user32().ClientToScreen(hwnd, ctypes.byref(point))
    return point.x, point.y, point.x + rect.right - rect.left, point.y + rect.bottom - rect.top


def window_scale(hwnd: int) -> float:
    try:
        dpi = user32().GetDpiForWindow(wintypes.HWND(hwnd))
    except AttributeError:
        dpi = 96
    return (dpi or 96) / 96


def grab(bbox):
    from PIL import ImageGrab  # type: ignore[import-not-found]

    return ImageGrab.grab(bbox=bbox)


def key(code: int, up: bool = False) -> None:
    user32().keybd_event(code, 0, KEYEVENTF_KEYUP if up else 0, 0)
    time.sleep(0.05)


def chord(modifier: int, code: int) -> None:
    key(modifier)
    key(code)
    key(code, up=True)
    key(modifier, up=True)


def click(x: int, y: int) -> None:
    user32().SetCursorPos(x, y)
    time.sleep(0.2)
    user32().mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.05)
    user32().mouse_event(0x0004, 0, 0, 0, 0)


def bring_forward(hwnd: int) -> None:
    user32().ShowWindow(hwnd, 9)  # SW_RESTORE
    user32().SetForegroundWindow(hwnd)
    time.sleep(0.5)


def content_share(image) -> float:
    """The share of pixels that differ from the most common colour."""
    rgb = image.convert("RGB")
    width, height = rgb.size
    samples = [rgb.getpixel((x, y)) for y in range(0, height, 2) for x in range(0, width, 2)]
    if not samples:
        return 0.0
    counts: dict[tuple[int, int, int], int] = {}
    for colour in samples:
        counts[colour] = counts.get(colour, 0) + 1
    background = max(counts, key=counts.get)
    differing = sum(
        1 for colour in samples if sum(abs(a - b) for a, b in zip(colour, background)) > 48
    )
    return differing / len(samples)


def red_light(hwnd: int) -> tuple[int, int] | None:
    """The centre of the red traffic light, found by its colour in the
    window's top-left corner."""
    left, top, _, _ = client_rect_on_screen(hwnd)
    scale = window_scale(hwnd)
    box = (left, top, left + int(120 * scale), top + int(90 * scale))
    image = grab(box).convert("RGB")
    xs: list[int] = []
    ys: list[int] = []
    width, height = image.size
    for y in range(height):
        for x in range(width):
            red, green, blue = image.getpixel((x, y))
            if red > 200 and green < 120 and blue < 120:
                xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return left + (min(xs) + max(xs)) // 2, top + (min(ys) + max(ys)) // 2


class Settings:
    def __init__(self, binary: Path, profile: Path, name: str):
        self.binary = binary
        self.environment = dict(os.environ)
        self.environment["APPDATA"] = str(profile / "Roaming")
        self.environment["LOCALAPPDATA"] = str(profile / "Local")
        self.environment["LULO_SETTINGS_TRACE"] = "1"
        for folder in ("Roaming", "Local"):
            (profile / folder).mkdir(parents=True, exist_ok=True)
        self.log = profile / f"{name}.log"
        with self.log.open("wb") as output:
            self.process = subprocess.Popen(
                [str(binary)],
                env=self.environment,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
        self.hwnd = 0
        deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
        while time.monotonic() < deadline and self.process.poll() is None:
            windows = visible_windows(self.process.pid)
            if windows:
                self.hwnd = windows[0][0]
                break
            time.sleep(0.05)

    def panes(self) -> list[tuple[str, str]]:
        deadline = time.monotonic() + PANES_TIMEOUT_SECONDS
        while time.monotonic() < deadline and self.process.poll() is None:
            for line in self.log.read_text(encoding="utf-8", errors="replace").splitlines():
                if line.startswith("settings: panes "):
                    entries = line[len("settings: panes ") :].split(";")
                    return [tuple(entry.split("=", 1)) for entry in entries if "=" in entry]
            time.sleep(0.2)
        return []

    def open_pane(self, pane_id: str) -> None:
        subprocess.run(
            [str(self.binary), "--pane", pane_id],
            env=self.environment,
            timeout=30,
            check=False,
            capture_output=True,
        )

    def exited(self) -> bool:
        try:
            self.process.wait(timeout=EXIT_TIMEOUT_SECONDS)
            return True
        except subprocess.TimeoutExpired:
            return False

    def kill(self) -> None:
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=30)


def check_panes(binary: Path, profile: Path, screenshots: Path | None) -> list[str]:
    settings = Settings(binary, profile, "panes")
    errors: list[str] = []
    try:
        if not settings.hwnd:
            return ["Settings opened no window"]
        panes = settings.panes()
        if not panes:
            return ["Settings never listed its panes (LULO_SETTINGS_TRACE)"]
        print(f"{APP}: {len(panes)} panes: {', '.join(name for _, name in panes)}")
        scale = window_scale(settings.hwnd)
        for pane_id, name in panes:
            settings.open_pane(pane_id)
            time.sleep(PANE_SETTLE_SECONDS)
            title = window_title(settings.hwnd)
            expected = name
            left, top, right, bottom = client_rect_on_screen(settings.hwnd)
            detail = (left + int(SIDEBAR_WIDTH * scale), top + int(TOOLBAR_HEIGHT * scale), right, bottom)
            image = grab(detail)
            share = content_share(image)
            if screenshots is not None:
                screenshots.mkdir(parents=True, exist_ok=True)
                grab((left, top, right, bottom)).save(screenshots / f"settings-{pane_id}.png")
            print(f"{APP}: {name}: title {title!r}, {share:.1%} of the detail column drawn")
            if not title.startswith(expected):
                errors.append(f"--pane {pane_id} showed {title!r}, not {expected}")
            if share < MIN_CONTENT_SHARE:
                errors.append(f"the {name} pane is empty ({share:.1%} drawn)")
        if settings.process.poll() is not None:
            errors.append(f"Settings exited ({settings.process.returncode}) while panes were opened")
    finally:
        settings.kill()
    return errors


def check_close(binary: Path, profile: Path) -> list[str]:
    errors: list[str] = []

    def red(settings: Settings) -> str | None:
        position = red_light(settings.hwnd)
        if position is None:
            return "could not find the red traffic light"
        click(*position)
        return None

    ways = [
        ("the red traffic light", red),
        ("Ctrl+W", lambda settings: chord(VK_CONTROL, ord("W"))),
        ("Ctrl+Q", lambda settings: chord(VK_CONTROL, ord("Q"))),
        ("Alt+F4", lambda settings: chord(VK_MENU, VK_F4)),
    ]
    for index, (way, close) in enumerate(ways):
        settings = Settings(binary, profile, f"close-{index}")
        try:
            if not settings.hwnd:
                errors.append(f"{way}: Settings opened no window")
                continue
            time.sleep(2.0)
            bring_forward(settings.hwnd)
            problem = close(settings)
            if problem:
                errors.append(f"{way}: {problem}")
                continue
            if settings.exited():
                print(f"{APP}: {way} closed the window and the process exited")
            else:
                still = "open" if visible_windows(settings.process.pid) else "closed"
                errors.append(f"{way} did not end Settings (its window is {still})")
        finally:
            settings.kill()
    return errors


def main() -> int:
    if sys.platform != "win32":
        print("settings_smoke.py runs on Windows only", file=sys.stderr)
        return 2
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    parser.add_argument("--screenshots", type=Path)
    arguments = parser.parse_args()
    binary = arguments.bin_dir / f"{APP}.exe"
    with tempfile.TemporaryDirectory(prefix="lulo-settings-") as folder:
        profile = Path(folder)
        errors = check_panes(binary, profile, arguments.screenshots)
        errors += check_close(binary, profile)
    for error in errors:
        print(f"{APP}: FAIL: {error}")
    if not errors:
        print(f"{APP}: every pane has content and every way out quits")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
