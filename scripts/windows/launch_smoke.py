"""Launch each Lulo app on Windows and check that it opens a window.

Usage: python scripts/windows/launch_smoke.py <bin-dir> <app> [<app> ...]

Each app starts with private %APPDATA%/%LOCALAPPDATA% folders, so the run
never reads or writes a real profile. The check passes when the process is
still running and owns a visible top-level window with a non-empty title.
With --screenshots <dir>, a capture of the desktop is saved for each app
(Pillow is used when it is installed; otherwise no capture is taken).
Windows only: it calls user32 through ctypes.
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


def visible_windows(pid: int) -> list[str]:
    """Titles of the visible top-level windows owned by `pid`."""
    user32 = ctypes.windll.user32
    titles: list[str] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value != pid or not user32.IsWindowVisible(hwnd):
            return True
        length = user32.GetWindowTextLengthW(hwnd)
        buffer = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(hwnd, buffer, length + 1)
        titles.append(buffer.value)
        return True

    user32.EnumWindows(collect, 0)
    return titles


def screenshot(path: Path) -> None:
    try:
        from PIL import ImageGrab  # type: ignore[import-not-found]
    except ImportError:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    ImageGrab.grab().save(path)


def launch(binary: Path, profile: Path, screenshots: Path | None) -> str | None:
    """Run one app; return an error message, or None when it opened a window."""
    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    log = profile / "output.log"
    with log.open("wb") as output:
        process = subprocess.Popen(
            [str(binary)], env=environment, stdout=output, stderr=subprocess.STDOUT
        )
        try:
            deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
            titles: list[str] = []
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    break
                titles = [title for title in visible_windows(process.pid) if title]
                if titles:
                    break
                time.sleep(0.5)
            if titles:
                # Still running a moment later: the first frame did not crash it.
                time.sleep(SETTLE_SECONDS)
                if screenshots is not None:
                    screenshot(screenshots / f"{binary.stem}.png")
            exit_code = process.poll()
            if exit_code is not None:
                return f"exited with {exit_code} before or just after opening a window"
            if not titles:
                return f"opened no visible window within {WINDOW_TIMEOUT_SECONDS:.0f} s"
            print(f"{binary.stem}: window {titles[0]!r}")
            return None
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=30)
            output.flush()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    parser.add_argument("apps", nargs="+")
    parser.add_argument("--screenshots", type=Path)
    arguments = parser.parse_args()
    if sys.platform != "win32":
        print("launch_smoke.py runs only on Windows", file=sys.stderr)
        return 2

    failures = 0
    for app in arguments.apps:
        binary = arguments.bin_dir / f"{app}.exe"
        with tempfile.TemporaryDirectory(prefix=f"{app}-") as directory:
            profile = Path(directory)
            error = launch(binary, profile, arguments.screenshots)
            if error is not None:
                failures += 1
                print(f"{app}: FAIL: {error}")
                log = profile / "output.log"
                if log.exists():
                    print(log.read_text(encoding="utf-8", errors="replace")[-4000:])
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
