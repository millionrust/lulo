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

For Preview, a tiny one-page PDF (a red square, written by this script —
no poppler or other PDF tool involved) is passed on the command line, and
the check waits for it to render (ADR 0023 phase 2c, `rmac-preview`'s
`winpdf` module) and looks for its colour in the window.

With --screenshots <dir>, captures are saved for each app: the window as
it opened, with its first menu open, and after Ctrl+N (Pillow is used when
it is installed; without it no capture is taken and the menu check is
skipped). Windows only: it calls user32 through ctypes.

Two more numbers print for every app, non-blocking (a slow or non-idle
result fails nothing — this job already runs with `continue-on-error`):

- **Launch time**, process start to a visible top-level window, polled at
  `LAUNCH_POLL_SECONDS` resolution.
- **Idle CPU**, `IDLE_WINDOW_SECONDS` of this process's own kernel+user
  time (`GetProcessTimes`), after `IDLE_SETTLE_SECONDS` with no input, as a
  share of one 15.6 ms scheduling tick. On Linux the apps are at true zero
  when idle; the target here is the same, below one tick over the window,
  except Terminal with a live shell.

With `--foreground-check <app>`, after every app in `apps` has been tested
and closed, one more pair is launched back to back and left running (not
killed early like every other check): `<app>` first, then whichever of
`rmac-weather`/`rmac-terminal` sits later in `apps`, without killing the
first. The second app's window must become the foreground window — ADR
0023's "Foreground" gap, launched this way because `launch()`'s own loop
kills each app before the next starts, which cannot reproduce two windows
open at once.
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

# How often `launch()` polls for the first visible window: the resolution
# of the launch-time number it prints, not a user-visible delay.
LAUNCH_POLL_SECONDS = 0.02
# No input for this long before the idle-CPU measurement starts.
IDLE_SETTLE_SECONDS = 10.0
# How long the idle-CPU measurement itself runs.
IDLE_WINDOW_SECONDS = 20.0
# Windows' default scheduling quantum: a process doing nothing should not
# show up as having used even one of these across the whole window.
TICK_SECONDS = 0.0156
TICK_100NS = int(TICK_SECONDS * 10_000_000)
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000


class _FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wintypes.DWORD), ("dwHighDateTime", wintypes.DWORD)]


def _filetime_to_100ns(value: _FILETIME) -> int:
    return (value.dwHighDateTime << 32) | value.dwLowDateTime


def process_cpu_time_100ns(pid: int) -> int | None:
    """This process's kernel+user CPU time so far, in 100 ns units, or
    `None` when it cannot be read (already exited, or access denied)."""
    handle = ctypes.windll.kernel32.OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION, False, pid
    )
    if not handle:
        return None
    try:
        creation, exited, kernel, user = (
            _FILETIME(),
            _FILETIME(),
            _FILETIME(),
            _FILETIME(),
        )
        ok = ctypes.windll.kernel32.GetProcessTimes(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exited),
            ctypes.byref(kernel),
            ctypes.byref(user),
        )
        if not ok:
            return None
        return _filetime_to_100ns(kernel) + _filetime_to_100ns(user)
    finally:
        ctypes.windll.kernel32.CloseHandle(handle)


def measure_idle_cpu(
    pid: int, settle: float = IDLE_SETTLE_SECONDS, window: float = IDLE_WINDOW_SECONDS
) -> tuple[float, float] | None:
    """(ticks, percent of one core) this process used over `window`
    seconds of no input, after `settle` seconds of no input. `None` when
    its CPU time could not be read (it may have exited)."""
    time.sleep(settle)
    before = process_cpu_time_100ns(pid)
    if before is None:
        return None
    time.sleep(window)
    after = process_cpu_time_100ns(pid)
    if after is None:
        return None
    delta = max(after - before, 0)
    return delta / TICK_100NS, (delta / (window * 10_000_000)) * 100


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


def write_minimal_pdf(path: Path) -> None:
    """A tiny, valid one-page PDF: a solid red square, no external tool."""
    stream_data = b"1 0 0 rg 20 20 160 160 re f\n"
    object_bodies = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
        b"/Contents 4 0 R /Resources << >> >>",
        b"<< /Length " + str(len(stream_data)).encode("ascii") + b" >>\nstream\n"
        + stream_data
        + b"endstream",
    ]
    body = bytearray(b"%PDF-1.4\n")
    offsets = []
    for index, content in enumerate(object_bodies, start=1):
        offsets.append(len(body))
        body += f"{index} 0 obj\n".encode("ascii") + content + b"\nendobj\n"
    xref_offset = len(body)
    count = len(object_bodies) + 1
    body += f"xref\n0 {count}\n".encode("ascii")
    body += b"0000000000 65535 f \n"
    for offset in offsets:
        body += f"{offset:010d} 00000 n \n".encode("ascii")
    body += (
        b"trailer\n<< /Size "
        + str(count).encode("ascii")
        + b" /Root 1 0 R >>\nstartxref\n"
        + str(xref_offset).encode("ascii")
        + b"\n%%EOF\n"
    )
    path.write_bytes(bytes(body))


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


def has_reddish_pixel(image) -> bool:
    """A coarse scan for the fixture PDF's red square (loose on exact
    colour: WARP's software rasteriser and PNG recompression both shift
    values slightly)."""
    if image is None:
        return False
    rgb = image.convert("RGB")
    width, height = rgb.size
    for y in range(0, height, 3):
        for x in range(0, width, 3):
            red, green, blue = rgb.getpixel((x, y))
            if red > 140 and red - green > 50 and red - blue > 50:
                return True
    return False


def check_preview_pdf(hwnd: int, screenshots: Path | None) -> str | None:
    """Preview opened with the fixture PDF on its command line: wait for
    Windows.Data.Pdf (`rmac-preview`'s `winpdf` module, ADR 0023 phase 2c)
    to rasterise the page and look for the fixture's red square anywhere
    in the window."""
    time.sleep(KEY_SETTLE_SECONDS * 2)
    capture = grab(window_rect(hwnd))
    if screenshots is not None:
        save(capture, screenshots / "rmac-preview-pdf.png")
    if capture is None:
        print("rmac-preview: PDF render check skipped (Pillow is not installed)")
        return None
    if not has_reddish_pixel(capture):
        return "the fixture PDF's page did not render (no red pixels found)"
    print("rmac-preview: the fixture PDF's page rendered")
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


def launch(
    binary: Path,
    profile: Path,
    screenshots: Path | None,
    arguments: list[str] | None = None,
) -> str | None:
    """Run one app; return an error message, or None when every check passed."""
    app = binary.stem
    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    environment["RMAC_MENU_STRIP_TRACE"] = "1"
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    log = profile / "output.log"
    launch_started = time.monotonic()
    with log.open("wb") as output:
        process = subprocess.Popen(
            [str(binary), *(arguments or [])],
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
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
            time.sleep(LAUNCH_POLL_SECONDS)
        launch_ms = (time.monotonic() - launch_started) * 1000
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
        # ADR 0023 task 2 (launch time): process start to a visible
        # top-level window. Not the same as a presented first frame
        # (`RMAC_FRAME_TRACE` does not run on Windows, see the module
        # docstring), but the closest number this script can take without
        # it, and on the same clock every run.
        print(f"{app}: launch time ~{launch_ms:.0f} ms (process start to a visible window)")

        # ADR 0023 task 1 (idle CPU): before any input reaches this
        # window, not after — `check_menu_strip`/`check_menu_command`/
        # `check_new_shortcut` below all inject keys and mouse clicks.
        idle = measure_idle_cpu(process.pid)
        if idle is None:
            print(f"{app}: idle CPU skipped (could not read its CPU time)")
        else:
            ticks, percent = idle
            verdict = "OK" if ticks < 1.0 else "ABOVE TARGET"
            print(
                f"{app}: idle CPU over {IDLE_WINDOW_SECONDS:.0f} s = "
                f"{ticks:.2f} ticks ({percent:.2f}% of one core) [{verdict}]"
            )

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
        if app == "rmac-preview" and arguments:
            error = check_preview_pdf(hwnd, screenshots)
            if error:
                errors.append(error)
        if app == "rmac-text-editor" and not errors:
            error = check_single_instance(binary, environment, process, log)
            if error:
                errors.append(error)
        return "; ".join(errors) or None
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)


def wait_for_window(pid: int, timeout: float = WINDOW_TIMEOUT_SECONDS) -> tuple[int, str] | None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        windows = visible_windows(pid)
        if windows:
            return windows[0]
        time.sleep(LAUNCH_POLL_SECONDS)
    return None


def check_foreground_order(
    bin_dir: Path, first_app: str, second_app: str, screenshots: Path | None
) -> str | None:
    """ADR 0023's "Foreground" gap: launch `first_app`, then `second_app`
    without closing the first, and check the second's window — not the
    first's — ends up as the foreground window. `launch()`'s own loop
    cannot reproduce this: it kills each app before starting the next."""
    processes: list[subprocess.Popen] = []
    directories: list[tempfile.TemporaryDirectory] = []
    try:
        hwnds: dict[str, int] = {}
        for app in (first_app, second_app):
            directory = tempfile.TemporaryDirectory(prefix=f"{app}-fg-")
            directories.append(directory)
            profile = Path(directory.name)
            environment = dict(os.environ)
            environment["APPDATA"] = str(profile / "Roaming")
            environment["LOCALAPPDATA"] = str(profile / "Local")
            for folder in ("Roaming", "Local"):
                (profile / folder).mkdir(parents=True, exist_ok=True)
            process = subprocess.Popen(
                [str(bin_dir / f"{app}.exe")],
                env=environment,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            processes.append(process)
            found = wait_for_window(process.pid)
            if found is None:
                return f"{app} opened no visible window within {WINDOW_TIMEOUT_SECONDS:.0f} s"
            hwnds[app] = found[0]
            if app == first_app:
                # Let the first app settle into the foreground on its own
                # before the second one starts, the same gap a person
                # opening one app after another leaves.
                time.sleep(SETTLE_SECONDS)
        time.sleep(KEY_SETTLE_SECONDS)
        foreground = user32().GetForegroundWindow()
        if screenshots is not None:
            save(grab(), screenshots / f"foreground-order-{first_app}-then-{second_app}.png")
        print(
            f"foreground order: opened {first_app}, then {second_app} without "
            f"closing it; foreground window is now "
            f"{'the second app' if foreground == hwnds[second_app] else 'something else'}"
        )
        if foreground != hwnds[second_app]:
            return (
                f"{second_app} opened behind {first_app} instead of becoming "
                "the foreground window"
            )
        return None
    finally:
        for process in processes:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=30)
        for directory in directories:
            directory.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    parser.add_argument("apps", nargs="+")
    parser.add_argument("--screenshots", type=Path)
    parser.add_argument(
        "--foreground-check",
        metavar="APP",
        help=(
            "Open APP, then whichever of rmac-weather/rmac-terminal is "
            "later in `apps`, without closing APP first, and check the "
            "second app's window becomes the foreground window."
        ),
    )
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
            app_arguments: list[str] = []
            if app == "rmac-preview":
                fixture = profile / "fixture.pdf"
                write_minimal_pdf(fixture)
                app_arguments = [str(fixture)]
            error = launch(binary, profile, arguments.screenshots, app_arguments)
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

    if arguments.foreground_check:
        second_app = next(
            (app for app in arguments.apps if app in ("rmac-weather", "rmac-terminal")),
            None,
        )
        if second_app is None:
            print(
                "foreground order: skipped (no rmac-weather or rmac-terminal in `apps`)"
            )
        else:
            error = check_foreground_order(
                arguments.bin_dir,
                arguments.foreground_check,
                second_app,
                arguments.screenshots,
            )
            if error is not None:
                failures += 1
                print(f"foreground order: FAIL: {error}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
