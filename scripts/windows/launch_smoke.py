"""Launch each Lulo app on Windows, check its window, menu strip and keys.

Usage: python scripts/windows/launch_smoke.py <bin-dir> <app> [<app> ...]

Each app starts with private %APPDATA%/%LOCALAPPDATA% folders, so the run
never reads or writes a real profile. For each app the check:

1. waits for a visible top-level window with a non-empty title;
2. taps Alt, which opens the first menu of the window's menu strip
   (ADR 0023 phase 2), and checks that the pixels below the strip changed
   (a menu panel appeared), then closes it with Esc;
3. chooses App ▸ About with Alt, Down and Return and checks the About
   panel opens (a menu command reaches the app);
4. presses Ctrl+N, the Windows spelling of ⌘N, and checks the app answers:
   Text Editor opens a second window, the others stay running;
5. for Text Editor, launches the app a second time with no arguments and
   checks that launch hands off to the running process (it exits and a
   window opens in the first one) instead of starting a second app.

With --screenshots <dir>, captures are saved for each app: the window as
it opened, with its first menu open, and after Ctrl+N (Pillow is used when
it is installed; without it no capture is taken and the menu check is
skipped). Windows only: it calls user32 through ctypes.
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
SETTLE_SECONDS = 3.0
KEY_SETTLE_SECONDS = 1.5
# The strip is 24 px tall at 100 % scale (the runner's display).
STRIP_HEIGHT = 24
# Where the first menu's panel opens: just below the strip, at the left.
MENU_PROBE = (12, STRIP_HEIGHT + 8, 180, STRIP_HEIGHT + 110)

VK_MENU = 0x12
VK_CONTROL = 0x11
VK_ESCAPE = 0x1B
VK_DOWN = 0x28
VK_RETURN = 0x0D
KEYEVENTF_KEYUP = 0x0002


def user32():
    return ctypes.windll.user32


def visible_windows(pid: int) -> list[tuple[int, str]]:
    """The visible top-level windows owned by `pid`, with their titles."""
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


def client_origin(hwnd: int) -> tuple[int, int]:
    """The screen position of the window's content (its strip's top left)."""
    point = wintypes.POINT(0, 0)
    user32().ClientToScreen(hwnd, ctypes.byref(point))
    return point.x, point.y


def click(x: int, y: int) -> None:
    user32().SetCursorPos(x, y)
    time.sleep(0.1)
    user32().mouse_event(0x0002, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTDOWN
    time.sleep(0.05)
    user32().mouse_event(0x0004, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTUP
    time.sleep(0.05)


def window_rect(hwnd: int) -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    user32().GetWindowRect(hwnd, ctypes.byref(rect))
    return rect.left, rect.top, rect.right, rect.bottom


def grab(bbox: tuple[int, int, int, int] | None = None):
    try:
        from PIL import ImageGrab  # type: ignore[import-not-found]
    except ImportError:
        return None
    return ImageGrab.grab(bbox=bbox)


def save(image, path: Path) -> None:
    if image is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path)


def key(code: int, up: bool = False) -> None:
    user32().keybd_event(code, 0, KEYEVENTF_KEYUP if up else 0, 0)
    time.sleep(0.05)


def tap(code: int) -> None:
    key(code)
    key(code, up=True)


def chord(modifier: int, letter: str) -> None:
    key(modifier)
    tap(ord(letter.upper()))
    key(modifier, up=True)


def bring_forward(hwnd: int) -> bool:
    user32().ShowWindow(hwnd, 9)  # SW_RESTORE
    user32().SetForegroundWindow(hwnd)
    time.sleep(0.5)
    return user32().GetForegroundWindow() == hwnd


def changed_fraction(before, after) -> float:
    """The share of pixels that differ noticeably between two captures."""
    if before is None or after is None or before.size != after.size:
        return 0.0
    before = before.convert("L")
    after = after.convert("L")
    width, height = before.size
    differing = 0
    total = 0
    for y in range(0, height, 2):
        for x in range(0, width, 2):
            total += 1
            if abs(before.getpixel((x, y)) - after.getpixel((x, y))) > 24:
                differing += 1
    return differing / max(total, 1)


def check_menu_strip(app: str, hwnd: int, screenshots: Path | None) -> str | None:
    """Open the strip's first menu by Alt, Alt+letter and a click.

    Each way must open a menu over the content below the strip; Alt is the
    one the check requires, the others say where a failure lies."""
    left, top = client_origin(hwnd)
    probe = (
        left + MENU_PROBE[0],
        top + MENU_PROBE[1],
        left + MENU_PROBE[2],
        top + MENU_PROBE[3],
    )
    closed = grab(probe)
    if closed is None:
        print(f"{app}: menu strip check skipped (Pillow is not installed)")
        return None
    initial = "e"  # Edit (Notes, Text Editor) or the app's Edit menu

    def opened(way: str, open_menu) -> bool:
        open_menu()
        time.sleep(KEY_SETTLE_SECONDS)
        after = grab(probe)
        if screenshots is not None:
            save(grab(window_rect(hwnd)), screenshots / f"{app}-menu-{way}.png")
        tap(VK_ESCAPE)
        time.sleep(0.7)
        changed = changed_fraction(closed, after)
        print(f"{app}: {way} -> {changed:.0%} of the area below the strip changed")
        return changed >= 0.02

    by_alt = opened("alt", lambda: tap(VK_MENU))
    opened("alt-letter", lambda: chord(VK_MENU, initial))
    opened("click", lambda: click(left + 20, top + STRIP_HEIGHT // 2))
    bring_forward(hwnd)
    if not by_alt:
        return "tapping Alt did not open the menu strip's first menu"
    return None


def check_menu_command(app: str, process: subprocess.Popen, hwnd: int) -> str | None:
    """Choose App ▸ About from the keyboard: Alt, Down, Return.

    The About panel is a window of its own, so the command reached the app
    when the process gains a window. Ctrl+W closes it again."""
    windows_before = len(visible_windows(process.pid))
    tap(VK_MENU)
    time.sleep(KEY_SETTLE_SECONDS)
    tap(VK_DOWN)
    time.sleep(0.3)
    tap(VK_RETURN)
    time.sleep(KEY_SETTLE_SECONDS * 2)
    windows_after = len(visible_windows(process.pid))
    print(f"{app}: Alt, Down, Return: {windows_before} -> {windows_after} windows")
    if windows_after > windows_before:
        chord(VK_CONTROL, "w")
        time.sleep(KEY_SETTLE_SECONDS)
    bring_forward(hwnd)
    if windows_after <= windows_before:
        return "choosing About from the menu strip opened no About panel"
    return None


def check_new_shortcut(
    app: str, process: subprocess.Popen, screenshots: Path | None
) -> str | None:
    """Press Ctrl+N and check the app answered it."""
    windows_before = len(visible_windows(process.pid))
    chord(VK_CONTROL, "n")
    time.sleep(KEY_SETTLE_SECONDS * 2)
    if screenshots is not None:
        save(grab(), screenshots / f"{app}-ctrl-n.png")
    if process.poll() is not None:
        return f"exited with {process.returncode} after Ctrl+N"
    windows_after = len(visible_windows(process.pid))
    print(f"{app}: Ctrl+N: {windows_before} -> {windows_after} windows")
    if app == "rmac-text-editor" and windows_after <= windows_before:
        return "Ctrl+N did not open a new Text Editor window"
    return None


def check_single_instance(
    binary: Path, environment: dict[str, str], process: subprocess.Popen, log: Path
) -> str | None:
    """A second launch must hand off to the running app and exit."""
    windows_before = len(visible_windows(process.pid))
    with log.open("ab") as output:
        second = subprocess.Popen(
            [str(binary), "--new-document"],
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
        )
        try:
            second.wait(timeout=30)
        except subprocess.TimeoutExpired:
            second.kill()
            second.wait(timeout=30)
            return "a second launch kept running instead of handing off"
    time.sleep(KEY_SETTLE_SECONDS * 2)
    windows_after = len(visible_windows(process.pid))
    print(
        f"{binary.stem}: second launch exited {second.returncode}; "
        f"running app {windows_before} -> {windows_after} windows"
    )
    if windows_after <= windows_before:
        return "a second launch did not open a window in the running app"
    return None


def launch(binary: Path, profile: Path, screenshots: Path | None) -> str | None:
    """Run one app; return an error message, or None when every check passed."""
    app = binary.stem
    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    environment["RMAC_MENU_STRIP_TRACE"] = "1"
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    log = profile / "output.log"
    with log.open("wb") as output:
        process = subprocess.Popen(
            [str(binary)], env=environment, stdout=output, stderr=subprocess.STDOUT
        )
    try:
        deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
        windows: list[tuple[int, str]] = []
        while time.monotonic() < deadline:
            if process.poll() is not None:
                break
            windows = visible_windows(process.pid)
            if windows:
                break
            time.sleep(0.5)
        if windows:
            # Still running a moment later: the first frame did not crash it.
            time.sleep(SETTLE_SECONDS)
            if screenshots is not None:
                save(grab(), screenshots / f"{app}.png")
        exit_code = process.poll()
        if exit_code is not None:
            return f"exited with {exit_code} before or just after opening a window"
        if not windows:
            return f"opened no visible window within {WINDOW_TIMEOUT_SECONDS:.0f} s"
        hwnd, title = windows[0]
        print(f"{app}: window {title!r}")

        if not bring_forward(hwnd):
            return "could not bring the window forward to test its keys"
        errors = [
            error
            for error in (
                check_menu_strip(app, hwnd, screenshots),
                check_menu_command(app, process, hwnd),
                check_new_shortcut(app, process, screenshots),
            )
            if error
        ]
        if app == "rmac-text-editor" and not errors:
            error = check_single_instance(binary, environment, process, log)
            if error:
                errors.append(error)
        return "; ".join(errors) or None
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    parser.add_argument("apps", nargs="+")
    parser.add_argument("--screenshots", type=Path)
    arguments = parser.parse_args()
    if sys.platform != "win32":
        print("launch_smoke.py runs only on Windows", file=sys.stderr)
        return 2
    # Physical pixels, so window rectangles and captures agree.
    ctypes.windll.user32.SetProcessDPIAware()

    failures = 0
    for app in arguments.apps:
        binary = arguments.bin_dir / f"{app}.exe"
        with tempfile.TemporaryDirectory(prefix=f"{app}-") as directory:
            profile = Path(directory)
            error = launch(binary, profile, arguments.screenshots)
            log = profile / "output.log"
            if log.exists():
                trace = [
                    line
                    for line in log.read_text(encoding="utf-8", errors="replace").splitlines()
                    if line.startswith("menu strip:")
                ]
                for line in trace[:40]:
                    print(f"{app}: {line}")
            if error is not None:
                failures += 1
                print(f"{app}: FAIL: {error}")
                log = profile / "output.log"
                if log.exists():
                    print(log.read_text(encoding="utf-8", errors="replace")[-4000:])
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
