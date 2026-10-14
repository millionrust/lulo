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
5. presses Ctrl+M and checks the window minimises (and, for Calculator,
   that Zoom leaves the fixed-size window alone);
6. for Text Editor, launches the app a second time with no arguments and
   checks that launch hands off to the running process (it exits and a
   window opens in the first one) instead of starting a second app.
6. for Calculator and Text Editor, before any key, puts a blue backdrop
   behind the window and checks its frame (ADR 0023 "Window chrome",
   WIN-OS-73): outside the arc of Lulo's corner radius (27 pt and 16 pt)
   the backdrop shows, just inside it the window does, and the Mac shadow
   beside the window fades with distance and ends at its margin.

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

Each app also runs with `gpui_windows`' own traces on
(docs/decisions/0025-vendor-gpui-windows.md): `RMAC_GPUI_STARTUP_TRACE`
prints the start-up phases (DirectX devices, DirectWrite, window
creation, first frame, first present), timed from process creation, and
`RMAC_GPUI_WAKE_TRACE` logs every wake-up; the ones that fall inside the
idle window are grouped by source and printed, so a regression names its
cause.

With `--world-tick-check`, Clock is launched once more with its World Clock
redraw period shortened to `WORLD_TICK_SECONDS` (`RMAC_CLOCK_WORLD_TICK_MS`,
otherwise a minute) and left alone through the same settle and idle window,
so the window holds many World Clock redraws instead of a one-in-three
chance of one. Its CPU over the window, divided by the redraws in it, is
what one minute tick costs (`world_tick` in `--results`, checked by
`idle_gate.py --max-world-tick-ticks`).

With `--foreground-check <app>`, after every app in `apps` has been tested
and closed, one more pair is launched back to back and left running (not
killed early like every other check): `<app>` first, then whichever of
`rmac-weather`/`rmac-terminal` sits later in `apps`, without killing the
first. The second app's window must become the foreground window — ADR
0023's "Foreground" gap, launched this way because `launch()`'s own loop
kills each app before the next starts, which cannot reproduce two windows
open at once.

With `--shell`, the Lulo layer (ADR 0023 phase 3: `lulo-session.exe` and
`lulo-shell.exe` from the same folder) is checked last, by `shell_smoke.py`:
the bar's and the Dock's AppBar strips, the Dock opening Calculator, the
bar showing Calculator's own menus, a maximised window between the bar and
the Dock, Spotlight on Alt+Space, and the desktop given back on exit. Both
processes get an idle reading in `--results`, so `idle_gate.py` gates them
like the apps.
"""

from __future__ import annotations

import argparse
import ctypes
import json
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
VK_SHIFT = 0x10
VK_F11 = 0x7A
KEYEVENTF_KEYUP = 0x0002

# How often `launch()` polls for the first visible window: the resolution
# of the launch-time number it prints, not a user-visible delay.
LAUNCH_POLL_SECONDS = 0.02
# No input for this long before the idle-CPU measurement starts.
IDLE_SETTLE_SECONDS = 10.0
# How long the idle-CPU measurement itself runs.
IDLE_WINDOW_SECONDS = 20.0
# `--world-tick-check`: the shortened World Clock redraw period.
WORLD_TICK_SECONDS = 2.0
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


TH32CS_SNAPTHREAD = 0x00000004
THREAD_QUERY_LIMITED_INFORMATION = 0x0800


class _THREADENTRY32(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ThreadID", wintypes.DWORD),
        ("th32OwnerProcessID", wintypes.DWORD),
        ("tpBasePri", wintypes.LONG),
        ("tpDeltaPri", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
    ]


def thread_cpu_times_100ns(pid: int) -> dict[int, tuple[str, int]]:
    """Each thread of `pid`: its description (Rust names its threads with
    it) and kernel+user CPU time so far, in 100 ns units."""
    kernel32 = ctypes.windll.kernel32
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.OpenThread.restype = wintypes.HANDLE
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    if not snapshot or snapshot == wintypes.HANDLE(-1).value:
        return {}
    threads: dict[int, tuple[str, int]] = {}
    try:
        entry = _THREADENTRY32()
        entry.dwSize = ctypes.sizeof(_THREADENTRY32)
        more = kernel32.Thread32First(snapshot, ctypes.byref(entry))
        while more:
            if entry.th32OwnerProcessID == pid:
                tid = entry.th32ThreadID
                handle = kernel32.OpenThread(THREAD_QUERY_LIMITED_INFORMATION, False, tid)
                if handle:
                    try:
                        creation, exited, kernel, user = (
                            _FILETIME(),
                            _FILETIME(),
                            _FILETIME(),
                            _FILETIME(),
                        )
                        name = ""
                        description = ctypes.c_wchar_p()
                        if kernel32.GetThreadDescription(handle, ctypes.byref(description)) >= 0:
                            name = description.value or ""
                            kernel32.LocalFree(description)
                        if kernel32.GetThreadTimes(
                            handle,
                            ctypes.byref(creation),
                            ctypes.byref(exited),
                            ctypes.byref(kernel),
                            ctypes.byref(user),
                        ):
                            threads[tid] = (
                                name,
                                _filetime_to_100ns(kernel) + _filetime_to_100ns(user),
                            )
                    finally:
                        kernel32.CloseHandle(handle)
            more = kernel32.Thread32Next(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return threads


class _MODULEINFO(ctypes.Structure):
    _fields_ = [
        ("lpBaseOfDll", ctypes.c_void_p),
        ("SizeOfImage", wintypes.DWORD),
        ("EntryPoint", ctypes.c_void_p),
    ]


def thread_start_modules(pid: int, tids) -> dict[int, str]:
    """The module (lower-case file name) each of `tids` started in, from
    its Win32 start address; threads that cannot be read are left out."""
    kernel32 = ctypes.windll.kernel32
    psapi = ctypes.windll.psapi
    ntdll = ctypes.windll.ntdll
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenThread.restype = wintypes.HANDLE
    process = kernel32.OpenProcess(0x0400 | 0x0010, False, pid)  # QUERY_INFORMATION | VM_READ
    if not process:
        return {}
    ranges: list[tuple[int, int, str]] = []
    try:
        handles = (ctypes.c_void_p * 1024)()
        needed = wintypes.DWORD()
        if not psapi.EnumProcessModulesEx(
            wintypes.HANDLE(process), handles, ctypes.sizeof(handles), ctypes.byref(needed), 3
        ):
            return {}
        count = min(needed.value // ctypes.sizeof(ctypes.c_void_p), len(handles))
        for module in handles[:count]:
            info = _MODULEINFO()
            if not psapi.GetModuleInformation(
                wintypes.HANDLE(process),
                ctypes.c_void_p(module),
                ctypes.byref(info),
                ctypes.sizeof(info),
            ):
                continue
            name = ctypes.create_unicode_buffer(260)
            psapi.GetModuleBaseNameW(wintypes.HANDLE(process), ctypes.c_void_p(module), name, 260)
            ranges.append((info.lpBaseOfDll or 0, info.SizeOfImage, name.value.lower()))
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(process))
    modules: dict[int, str] = {}
    for tid in tids:
        handle = kernel32.OpenThread(0x0040, False, tid)  # THREAD_QUERY_INFORMATION
        if not handle:
            continue
        try:
            start = ctypes.c_void_p()
            status = ntdll.NtQueryInformationThread(
                wintypes.HANDLE(handle), 9, ctypes.byref(start), ctypes.sizeof(start), None
            )  # ThreadQuerySetWin32StartAddress
            if status != 0 or not start.value:
                continue
            for base, size, name in ranges:
                if base <= start.value < base + size:
                    modules[tid] = name
                    break
        finally:
            kernel32.CloseHandle(wintypes.HANDLE(handle))
    return modules


def busy_threads(
    before: dict[int, tuple[str, int]],
    after: dict[int, tuple[str, int]],
    modules: dict[int, str] | None = None,
) -> list[tuple[str, float]]:
    """The threads that used CPU between two `thread_cpu_times_100ns`
    snapshots, as ("name (tid)", ticks), busiest first, with
    " [module]" added where `modules` knows the thread's start module. A
    thread that started in between counts all its time."""
    modules = modules or {}
    busy = []
    for tid, (name, time_after) in after.items():
        time_before = before.get(tid, (name, 0))[1]
        delta = time_after - time_before
        if delta > 0:
            label = f"{name or 'unnamed'} ({tid})"
            if tid in modules:
                label += f" [{modules[tid]}]"
            busy.append((label, delta / TICK_100NS))
    return sorted(busy, key=lambda item: -item[1])


CPU_PREFIX = "gpui_windows cpu: "
# Threads of the Windows thread pool start in ntdll. GPUI's background
# tasks run there, and so do the workers of WARP, Direct3D's software
# rasteriser that stands in for a GPU on a machine without one (GitHub's
# runners): on a real PC that work is the GPU's.
THREAD_POOL_MODULE = "ntdll.dll"


def pool_task_ticks(trace: str) -> float:
    """The CPU the app's own thread-pool tasks used, from the
    `gpui_windows cpu:` lines (`RMAC_GPUI_WAKE_TRACE`) in `trace`."""
    total = 0
    for line in trace.splitlines():
        if not line.startswith(CPU_PREFIX):
            continue
        fields = line[len(CPU_PREFIX) :].rsplit(" at ", 1)[0].split()
        if fields and fields[-1].isdigit():
            total += int(fields[-1])
    return total / TICK_100NS


def software_renderer_ticks(threads: list[tuple[str, float]], trace: str) -> float:
    """The part of `busy_threads` the software rasteriser used: the thread
    pool's time less what the app's own pool tasks report using."""
    pool = sum(
        ticks for label, ticks in threads if label.endswith(f"[{THREAD_POOL_MODULE}]")
    )
    return max(pool - pool_task_ticks(trace), 0.0)


def measure_idle_cpu(
    pid: int,
    settle: float = IDLE_SETTLE_SECONDS,
    window: float = IDLE_WINDOW_SECONDS,
    log: Path | None = None,
) -> tuple[float, float, str, list[tuple[str, float]]] | None:
    """(ticks, percent of one core, trace, busy threads) for the CPU this
    process used over `window` seconds of no input, after `settle` seconds
    of no input, where `trace` is what the app wrote to `log` during the
    window. `None` when its CPU time could not be read (it may have
    exited)."""
    time.sleep(settle)
    threads_before = thread_cpu_times_100ns(pid)
    before = process_cpu_time_100ns(pid)
    if before is None:
        return None
    start = log.stat().st_size if log is not None and log.exists() else 0
    time.sleep(window)
    after = process_cpu_time_100ns(pid)
    if after is None:
        return None
    threads_after = thread_cpu_times_100ns(pid)
    threads = busy_threads(
        threads_before, threads_after, thread_start_modules(pid, threads_after.keys())
    )
    trace = ""
    if log is not None and log.exists():
        with log.open("rb") as output:
            output.seek(start)
            trace = output.read().decode("utf-8", errors="replace")
    delta = max(after - before, 0)
    return delta / TICK_100NS, (delta / (window * 10_000_000)) * 100, trace, threads


class _ProcessMemory(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


def process_memory_mb(pid: int) -> dict[str, float] | None:
    """Working set and private bytes of `pid`, in MB (None if unreadable)."""
    kernel32 = ctypes.windll.kernel32
    kernel32.OpenProcess.restype = wintypes.HANDLE
    handle = kernel32.OpenProcess(0x1000 | 0x0010, False, pid)
    if not handle:
        return None
    try:
        counters = _ProcessMemory()
        counters.cb = ctypes.sizeof(counters)
        if not kernel32.K32GetProcessMemoryInfo(
            wintypes.HANDLE(handle), ctypes.byref(counters), counters.cb
        ):
            return None
        return {
            "working_set_mb": round(counters.WorkingSetSize / 1048576, 1),
            "private_mb": round(counters.PrivateUsage / 1048576, 1),
        }
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(handle))


# Apps whose window redraws on a timer while it is active (and only then).
LIVE_APPS = ("rmac-system-monitor",)

# Per app: launch time, idle ticks and wake-ups, start-up phases; written
# with --results for `idle_gate.py` and the job summary.
MEASUREMENTS: dict[str, dict] = {}

WAKE_PREFIX = "gpui_windows wake: "
STARTUP_PREFIX = "gpui_windows startup: "


def summarize_wakes(trace: str, limit: int = 8) -> list[tuple[int, str]]:
    """The wake-up sources in a `RMAC_GPUI_WAKE_TRACE` excerpt, most
    frequent first, as (count, "source detail")."""
    counts: dict[str, int] = {}
    for line in trace.splitlines():
        if not line.startswith(WAKE_PREFIX):
            continue
        body = line[len(WAKE_PREFIX) :].rsplit(" at ", 1)[0]
        counts[body] = counts.get(body, 0) + 1
    ranked = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
    return [(count, body) for body, count in ranked[:limit]]


def startup_phases(log_text: str) -> list[str]:
    """Each `RMAC_GPUI_STARTUP_TRACE` phase, first occurrence only, as
    "phase at N ms"."""
    seen: set[str] = set()
    phases = []
    for line in log_text.splitlines():
        if not line.startswith(STARTUP_PREFIX):
            continue
        body = line[len(STARTUP_PREFIX) :]
        phase = body.split(" at ", 1)[0]
        if phase not in seen:
            seen.add(phase)
            phases.append(body)
    return phases


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


#: Lulo's window corner radius (pt) for the apps whose frame is checked:
#: niri's unified-toolbar radius for Calculator, the title-bar radius for
#: Text Editor (`rmac_design::Radii`, `packaging/rmac-session/shell.kdl`).
FRAME_RADIUS = {"rmac-calculator": 27, "rmac-text-editor": 16}
#: A saturated blue no Lulo window is, put behind the window for the check.
BACKDROP = (32, 96, 255)
#: The backdrop reaches this far (pt) past the window on every side.
BACKDROP_MARGIN = 110


def window_scale(hwnd: int) -> float:
    try:
        dpi = user32().GetDpiForWindow(wintypes.HWND(hwnd))
    except AttributeError:
        dpi = 96
    return (dpi or 96) / 96


def client_rect_on_screen(hwnd: int) -> tuple[int, int, int, int]:
    """The window's client area, the visible window (its swap chain)."""
    rect = wintypes.RECT()
    user32().GetClientRect(hwnd, ctypes.byref(rect))
    left, top = client_origin(hwnd)
    return left, top, left + rect.right - rect.left, top + rect.bottom - rect.top


def backdrop_share(colour: tuple[int, int, int]) -> float | None:
    """How much of the backdrop shows in `colour`, if `colour` is the
    backdrop darkened by a black shadow (a share of 1.0 is the bare
    backdrop); `None` when it is anything else, such as the window."""
    red, green, blue = colour[:3]
    share = blue / BACKDROP[2]
    if blue < 40:
        return None
    if abs(red - BACKDROP[0] * share) > 14 or abs(green - BACKDROP[1] * share) > 14:
        return None
    return share


def open_backdrop(rect: tuple[int, int, int, int]) -> subprocess.Popen | None:
    """A borderless window of `BACKDROP` covering `rect`, from Tk."""
    left, top, right, bottom = rect
    script = (
        "import tkinter\n"
        "root = tkinter.Tk()\n"
        "root.overrideredirect(True)\n"
        f"root.geometry('{right - left}x{bottom - top}+{left}+{top}')\n"
        f"root.configure(background='#{BACKDROP[0]:02x}{BACKDROP[1]:02x}{BACKDROP[2]:02x}')\n"
        "root.mainloop()\n"
    )
    try:
        return subprocess.Popen([sys.executable, "-c", script])
    except OSError:
        return None


def check_window_frame(app: str, hwnd: int, screenshots: Path | None) -> str | None:
    """ADR 0023 "Window chrome" (WIN-OS-73): the window is drawn as on Lulo
    OS and macOS, over a contrasting backdrop. Outside the arc of Lulo's
    corner radius the backdrop shows through (darkened only by the shadow),
    just inside it the window does; and beside the window the shadow fades
    out with distance, back to the bare backdrop past its margin."""
    radius_pt = FRAME_RADIUS.get(app)
    if radius_pt is None:
        return None
    if grab((0, 0, 1, 1)) is None:
        print(f"{app}: window frame check skipped (Pillow is not installed)")
        return None
    scale = window_scale(hwnd)
    left, top, right, bottom = client_rect_on_screen(hwnd)
    screen_width = user32().GetSystemMetrics(0)
    screen_height = user32().GetSystemMetrics(1)
    margin = round(BACKDROP_MARGIN * scale)
    area = (
        max(left - margin, 0),
        max(top - margin, 0),
        min(right + margin, screen_width),
        min(bottom + margin, screen_height),
    )
    backdrop = open_backdrop(area)
    if backdrop is None:
        return "could not open the backdrop window for the frame check"
    try:
        time.sleep(1.5)
        if not bring_forward(hwnd):
            return "could not bring the window over the frame check's backdrop"
        time.sleep(1.0)
        left, top, right, bottom = client_rect_on_screen(hwnd)
        image = grab(area)
        if image is None:
            return None
        image = image.convert("RGB")

        def at(x: int, y: int) -> tuple[int, int, int]:
            return image.getpixel((x - area[0], y - area[1]))

        radius = radius_pt * scale
        # The arc meets the diagonal r(1 - 1/sqrt 2) in from the corner.
        diagonal = radius * (1 - 2**-0.5)
        outside = max(int(diagonal - 3 * scale), 0)
        inside = int(diagonal + 3 * scale) + 1
        corners = {
            "top left": ((left + outside, top + outside), (left + inside, top + inside)),
            "top right": ((right - 1 - outside, top + outside), (right - 1 - inside, top + inside)),
        }
        if bottom + 2 < screen_height:
            corners["bottom left"] = (
                (left + outside, bottom - 1 - outside),
                (left + inside, bottom - 1 - inside),
            )
        errors: list[str] = []
        for name, (clear, filled) in corners.items():
            clear_colour, filled_colour = at(*clear), at(*filled)
            print(
                f"{app}: {name} corner (radius {radius_pt} pt): {clear_colour} outside the arc, "
                f"{filled_colour} just inside it"
            )
            if backdrop_share(clear_colour) is None:
                errors.append(
                    f"its {name} corner is not clear outside a {radius_pt} pt arc ({clear_colour})"
                )
            if backdrop_share(filled_colour) is not None:
                errors.append(f"its {name} corner is clear inside a {radius_pt} pt arc ({filled_colour})")
        # The window's own top row beside the corner: clear too (no line of
        # DWM's frame along the top edge).
        top_row = at(left + outside, top)
        print(f"{app}: top row beside the corner: {top_row}")
        if backdrop_share(top_row) is None:
            errors.append(f"its top edge shows a line beside the corner ({top_row})")
        diagonal_colours = [at(left + step, top + step) for step in range(0, int(diagonal) + 4)]
        print(f"{app}: top-left diagonal {diagonal_colours}")
        # The shadow beside a side edge (the right one, or the left one if
        # the window is near the screen's right), half way down: darkest at
        # the edge, fading with distance, gone past the margin.
        y = (top + bottom) // 2
        distances = [2, 10, 20, 32]
        room = round(60 * scale)
        if right + room < area[2]:
            side, outward = "right", [right - 1 + round(d * scale) for d in distances]
            past = min(right + margin - 4, area[2] - 1)
        else:
            side, outward = "left", [left - round(d * scale) for d in distances]
            past = max(left - margin + 4, area[0])
        shares = [backdrop_share(at(x, y)) for x in outward]
        beyond = backdrop_share(at(past, y))
        print(
            f"{app}: beside its {side} edge the backdrop shows {shares} at {distances} pt, "
            f"{beyond} past the shadow"
        )
        if any(share is None for share in shares) or beyond is None:
            errors.append(f"beside its {side} edge the screen is not the shadowed backdrop ({shares})")
        else:
            if not all(a < b for a, b in zip(shares, shares[1:])):
                errors.append(f"its shadow does not fade with distance ({shares})")
            if shares[0] > 0.85:
                errors.append(f"it casts no shadow beside its edge ({shares[0]:.2f} of the backdrop shows)")
            if beyond < 0.95:
                errors.append(f"its shadow reaches past its margin ({beyond:.2f} of the backdrop shows)")
        if screenshots is not None:
            save(image, screenshots / f"{app}-frame.png")
            corner = image.crop(
                (
                    left - area[0] - round(16 * scale),
                    top - area[1] - round(16 * scale),
                    left - area[0] + round(48 * scale),
                    top - area[1] + round(48 * scale),
                )
            )
            save(corner.resize((corner.width * 6, corner.height * 6), 0), screenshots / f"{app}-corner.png")
        if errors:
            return "window frame: " + "; ".join(errors)
        print(f"{app}: window frame OK: {radius_pt} pt corners clear outside the arc, Mac shadow")
        return None
    finally:
        backdrop.kill()
        backdrop.wait(timeout=30)
        time.sleep(0.3)


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


def check_window_keys(app: str, process: subprocess.Popen, hwnd: int) -> str | None:
    """Ctrl+M minimises the window, and Calculator ignores Zoom.

    Found by the robot (scripts/robot/windows): a window key's action ran
    while GPUI had the window leased for key dispatch, so Ctrl+M, Ctrl+H,
    Ctrl+Q and Ctrl+` did nothing from the keyboard (the menu items worked).
    And Window ▸ Zoom (⌃⌘Z, Alt+Shift+Z here) maximised the fixed-size
    Calculator to the whole monitor, over the Lulo bar and Dock."""
    if not bring_forward(hwnd):
        return "could not bring the window forward to test Ctrl+M"
    if app != "rmac-calculator":
        focus_content(hwnd)
    if app == "rmac-calculator":
        before = window_rect(hwnd)
        key(VK_MENU)
        key(VK_SHIFT)
        tap(ord("Z"))
        key(VK_SHIFT, up=True)
        key(VK_MENU, up=True)
        time.sleep(KEY_SETTLE_SECONDS)
        after = window_rect(hwnd)
        print(f"{app}: Alt+Shift+Z (Zoom): {before} -> {after}")
        if after != before:
            user32().ShowWindow(hwnd, 9)  # SW_RESTORE
            return f"Zoom resized the fixed-size Calculator window from {before} to {after}"
    if app != "rmac-calculator":
        # F11 enters full screen and leaves it again (the robot found a
        # full-screen window had no way out on Windows: the Mac's Globe+F
        # has no Windows key, and the window covers the Lulo bar).
        before = window_rect(hwnd)
        tap(VK_F11)
        time.sleep(KEY_SETTLE_SECONDS)
        full = window_rect(hwnd)
        tap(VK_F11)
        time.sleep(KEY_SETTLE_SECONDS)
        after = window_rect(hwnd)
        width = user32().GetSystemMetrics(0)
        height = user32().GetSystemMetrics(1)
        print(f"{app}: F11: {before} -> {full} -> {after}")
        if full[2] - full[0] < width or full[3] - full[1] < height:
            diagnose_keys(app, process, hwnd, "F11")
            return f"F11 did not make the window full screen ({full})"
        if after != before:
            user32().ShowWindow(hwnd, 9)  # SW_RESTORE
            return f"a second F11 did not bring the window back to {before} ({after})"
    if app == "rmac-terminal":
        # Ctrl+letter keys belong to the shell in Terminal (Ctrl+M is Return).
        return None
    chord(VK_CONTROL, "m")
    deadline = time.monotonic() + KEY_SETTLE_SECONDS * 2
    minimised = False
    while time.monotonic() < deadline and not minimised:
        time.sleep(0.1)
        minimised = bool(user32().IsIconic(hwnd))
    print(f"{app}: Ctrl+M: minimised={minimised}")
    bring_forward(hwnd)
    if process.poll() is not None:
        return f"exited with {process.returncode} after Ctrl+M"
    if not minimised:
        return "Ctrl+M did not minimise the window"
    return None


def describe_window(hwnd: int) -> str:
    if not hwnd:
        return "none"
    owner = wintypes.DWORD()
    user32().GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
    name = ctypes.create_unicode_buffer(64)
    user32().GetClassNameW(hwnd, name, 64)
    length = user32().GetWindowTextLengthW(hwnd)
    title = ctypes.create_unicode_buffer(length + 1)
    user32().GetWindowTextW(hwnd, title, length + 1)
    return f"{hwnd:#x} pid {owner.value} class {name.value!r} title {title.value!r}"


def diagnose_keys(app: str, process: subprocess.Popen, hwnd: int, what: str) -> None:
    """Where keys went when `what` had no effect: the foreground window,
    the modifiers Windows holds down, and every window of the process."""
    held = [
        name
        for name, code in (("Ctrl", VK_CONTROL), ("Alt", VK_MENU), ("Shift", VK_SHIFT), ("Win", 0x5B))
        if user32().GetAsyncKeyState(code) & 0x8000
    ]
    foreground = user32().GetForegroundWindow()
    print(f"{app}: {what}: foreground {describe_window(foreground)}; window {hwnd:#x}; held {held}")

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(window, _lparam):
        owner = wintypes.DWORD()
        user32().GetWindowThreadProcessId(window, ctypes.byref(owner))
        if owner.value == process.pid:
            visible = bool(user32().IsWindowVisible(window))
            print(f"{app}: {what}:   {describe_window(window)} visible={visible}")
        return True

    user32().EnumWindows(collect, 0)


def focus_content(hwnd: int) -> None:
    """Click the window's lower content area, as a user would before typing,
    so keys reach the view and not whatever last held focus."""
    left, top, right, bottom = window_rect(hwnd)
    click((left + right) // 2, bottom - max((bottom - top) // 4, 20))
    time.sleep(0.4)


def process_dialogs(pid: int) -> dict[int, bool]:
    """Every top-level common dialog (#32770) of `pid`, shown or not."""
    found: dict[int, bool] = {}

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(window, _lparam):
        owner = wintypes.DWORD()
        user32().GetWindowThreadProcessId(window, ctypes.byref(owner))
        if owner.value == pid:
            name = ctypes.create_unicode_buffer(64)
            user32().GetClassNameW(window, name, 64)
            if name.value == "#32770":
                found[window] = bool(user32().IsWindowVisible(window))
        return True

    user32().EnumWindows(collect, 0)
    return found


#: How long the Open dialog may take to appear. Windows creates it hidden
#: and shows it once its folder view is filled; a hosted runner's first
#: dialog has been seen hidden for over 35 s while it holds the keyboard.
OPEN_DIALOG_SECONDS = 60.0


def check_open_panel(app: str, process: subprocess.Popen, hwnd: int) -> str | None:
    """Ctrl+O shows Windows' Open dialog, and Esc cancels it.

    Found by the robot: Preview's File ▸ Open… asked the Linux desktop
    portal on every platform, so on Windows it did nothing.

    The dialog is created hidden and takes the keyboard at once, so it is
    waited for until it shows, and closed whatever happens: a dialog left
    hidden kept every later key (F11, Ctrl+M) from reaching the window."""
    if not bring_forward(hwnd):
        return "could not bring the window forward to test Ctrl+O"
    focus_content(hwnd)
    before = set(process_dialogs(process.pid))
    chord(VK_CONTROL, "o")
    pressed = time.monotonic()
    created = None
    shown = None
    deadline = pressed + OPEN_DIALOG_SECONDS
    while time.monotonic() < deadline and shown is None:
        time.sleep(0.2)
        for window, visible in process_dialogs(process.pid).items():
            if window in before:
                continue
            if created is None:
                created = time.monotonic() - pressed
            if visible:
                shown = time.monotonic() - pressed
                dialog = window
    created_text = "never" if created is None else f"after {created:.1f} s"
    shown_text = "never" if shown is None else f"after {shown:.1f} s"
    print(f"{app}: Ctrl+O: open dialog created {created_text}, shown {shown_text}")
    if shown is None:
        diagnose_keys(app, process, hwnd, "Ctrl+O")
        for window in set(process_dialogs(process.pid)) - before:
            user32().PostMessageW(window, WM_CLOSE, 0, 0)
        time.sleep(KEY_SETTLE_SECONDS)
        bring_forward(hwnd)
        if created is None:
            return "Ctrl+O showed no Open dialog"
        return f"the Open dialog stayed hidden for {OPEN_DIALOG_SECONDS:.0f} s after Ctrl+O"
    user32().SetForegroundWindow(dialog)
    time.sleep(0.3)
    tap(VK_ESCAPE)
    time.sleep(KEY_SETTLE_SECONDS)
    bring_forward(hwnd)
    if process.poll() is not None:
        return f"exited with {process.returncode} after the Open dialog was cancelled"
    return None


WM_NCHITTEST = 0x0084
HTCLIENT = 1
#: Clock's tab capsule (crates/clock/src/metrics.rs): centred, 396 pt wide,
#: 7 pt from the top, 38 pt tall.
CLOCK_TABS_CENTRE_Y = 7 + 38 / 2


def check_clock_tabs(app: str, hwnd: int) -> str | None:
    """Clock's tabs take clicks: the pointer over a tab hit-tests as client
    area, not as the caption of the title-area drag region beneath them.

    Found by the robot: clicking World Clock, Alarms, Stopwatch or Timers
    did nothing on Windows (the click started a window move instead)."""
    if not bring_forward(hwnd):
        return "could not bring Clock forward to test its tabs"
    left, top, right, _bottom = window_rect(hwnd)
    origin_x, origin_y = client_origin(hwnd)
    try:
        scale = (user32().GetDpiForWindow(hwnd) or 96) / 96.0
    except AttributeError:
        scale = 1.0
    x = (left + right) // 2 - int(120 * scale)  # the first tab, World Clock
    # The in-window menu strip sits above the content here (no Lulo bar).
    y = origin_y + int((STRIP_HEIGHT + CLOCK_TABS_CENTRE_Y) * scale)
    user32().SetCursorPos(x, y)
    time.sleep(0.4)
    user32().SendMessageW.restype = ctypes.c_ssize_t
    result = user32().SendMessageW(
        wintypes.HWND(hwnd), WM_NCHITTEST, 0, wintypes.LPARAM(((y & 0xFFFF) << 16) | (x & 0xFFFF))
    )
    print(f"{app}: hit test over the World Clock tab at ({x}, {y}): {result} (origin {origin_x}, {origin_y})")
    if result != HTCLIENT:
        return f"Clock's tabs hit-test as {result}, not client area: clicks on them move the window"
    return None


#: Calculator's History button in its title area, from the window's left
#: edge and the content's top (pt), as UI Automation reports it at 100 %.
CALCULATOR_HISTORY_BUTTON = (125, 25)


def check_title_area_button(app: str, hwnd: int) -> str | None:
    """A click on a button inside a title-area drag region reaches it.

    Found by the robot: on Windows the press started the system window move,
    which swallowed the release, so Calculator's History and Mode buttons,
    Text Editor's format bar and every toolbar control in a drag region did
    nothing. Clicking History opens Calculator's history panel."""
    if not bring_forward(hwnd):
        return "could not bring Calculator forward to test its History button"
    origin_x, origin_y = client_origin(hwnd)
    left, top, right, bottom = window_rect(hwnd)
    try:
        scale = (user32().GetDpiForWindow(hwnd) or 96) / 96.0
    except AttributeError:
        scale = 1.0
    x = origin_x + int(CALCULATOR_HISTORY_BUTTON[0] * scale)
    y = origin_y + int((STRIP_HEIGHT + CALCULATOR_HISTORY_BUTTON[1]) * scale)
    before = grab((left, top, right, bottom))
    if before is None:
        print(f"{app}: title-area button check skipped (Pillow is not installed)")
        return None
    rect_before = window_rect(hwnd)
    click(x, y)
    time.sleep(KEY_SETTLE_SECONDS)
    after = grab((left, top, right, bottom))
    changed = changed_fraction(before, after)
    moved = window_rect(hwnd) != rect_before
    print(f"{app}: click on History at ({x}, {y}): {changed:.1%} changed, window moved {moved} (origin {origin_x})")
    # Close the history panel again.
    click(x, y)
    time.sleep(KEY_SETTLE_SECONDS)
    if changed < 0.02 or moved:
        return "a click on Calculator's History button (in its title area) did nothing"
    return None


WM_CLOSE = 0x0010
CONSOLE_HOSTS = ("conhost.exe", "openconsole.exe")
TH32CS_SNAPPROCESS = 0x00000002


class _PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", wintypes.DWORD),
        ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD),
        ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


def process_tree() -> dict[int, tuple[int, str]]:
    """Every process: pid -> (parent pid, exe name)."""
    kernel32 = ctypes.windll.kernel32
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    tree: dict[int, tuple[int, str]] = {}
    if not snapshot or snapshot == wintypes.HANDLE(-1).value:
        return tree
    try:
        entry = _PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(_PROCESSENTRY32W)
        more = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            tree[entry.th32ProcessID] = (entry.th32ParentProcessID, entry.szExeFile)
            more = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return tree


def check_terminal_closes(app: str, process: subprocess.Popen, hwnd: int) -> str | None:
    """Closing Terminal's window at an idle prompt closes it without asking.

    Found by the robot: on Windows every close asked "Do you want to
    terminate running processes in this window?", the shell itself
    counting as a running program."""
    # The shell (Terminal's child that is not its console host) must be idle:
    # a program it runs is a real reason to ask.
    tree = process_tree()
    shells = [pid for pid, (parent, name) in tree.items() if parent == process.pid and name.lower() not in CONSOLE_HOSTS]
    busy = {
        pid: name
        for pid, (parent, name) in tree.items()
        if parent in shells and name.lower() not in CONSOLE_HOSTS
    }
    print(f"{app}: shells {[tree[pid][1] for pid in shells]}, their programs {sorted(busy.values())}")
    if busy:
        print(f"{app}: the shell is running {sorted(busy.values())}; the idle-close check is skipped")
        return None
    user32().PostMessageW(wintypes.HWND(hwnd), WM_CLOSE, 0, 0)
    deadline = time.monotonic() + 10.0
    while time.monotonic() < deadline:
        time.sleep(0.2)
        if process.poll() is not None or hwnd not in {w for w, _ in visible_windows(process.pid)}:
            print(f"{app}: the idle window closed without asking")
            return None
    return "closing Terminal at an idle prompt asked to terminate running processes"


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
    environment["RMAC_GPUI_STARTUP_TRACE"] = "1"
    environment["RMAC_GPUI_WAKE_TRACE"] = "1"
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
        phases = startup_phases(log.read_text(encoding="utf-8", errors="replace"))
        for phase in phases:
            print(f"{app}: startup {phase}")
        measurement: dict = {"launch_ms": round(launch_ms), "startup": phases}
        MEASUREMENTS[app] = measurement
        idle = measure_idle_cpu(process.pid, log=log)
        if app in LIVE_APPS and idle is not None:
            # A process monitor redraws on its refresh timer while its window
            # is the active one (that is its job), and parks the moment the
            # window is not: nothing is shown, so nothing is sampled. The
            # reading above is what watching it costs; the gate's idle
            # reading is taken with the window minimised.
            active_ticks = idle[0]
            measurement["active_ticks"] = round(active_ticks, 3)
            print(f"{app}: CPU over {IDLE_WINDOW_SECONDS:.0f} s while it is the active window = {active_ticks:.2f} ticks")
            ctypes.windll.user32.ShowWindow(hwnd, 6)  # SW_MINIMIZE
            # Longer than its slowest refresh period (5 s), so a refresh that
            # was already due when the window went away has run.
            time.sleep(7)
            idle = measure_idle_cpu(process.pid, log=log)
            ctypes.windll.user32.ShowWindow(hwnd, 9)  # SW_RESTORE
        memory = process_memory_mb(process.pid)
        if memory is not None:
            measurement["idle_working_set_mb"] = memory["working_set_mb"]
            measurement["idle_private_mb"] = memory["private_mb"]
            print(
                f"{app}: memory at idle: working set {memory['working_set_mb']} MB, "
                f"private {memory['private_mb']} MB"
            )
        if idle is None:
            print(f"{app}: idle CPU skipped (could not read its CPU time)")
        else:
            ticks, percent, trace, threads = idle
            verdict = "OK" if ticks < 1.0 else "ABOVE TARGET"
            print(
                f"{app}: idle CPU over {IDLE_WINDOW_SECONDS:.0f} s = "
                f"{ticks:.2f} ticks ({percent:.2f}% of one core) [{verdict}]"
            )
            wakes = summarize_wakes(trace)
            total = sum(1 for line in trace.splitlines() if line.startswith(WAKE_PREFIX))
            measurement["idle_ticks"] = round(ticks, 3)
            measurement["idle_renderer_ticks"] = round(software_renderer_ticks(threads, trace), 3)
            measurement["idle_wakes"] = total
            measurement["idle_wake_sources"] = [
                {"count": count, "source": source} for count, source in wakes
            ]
            print(f"{app}: idle wake-ups over {IDLE_WINDOW_SECONDS:.0f} s = {total}")
            for count, source in wakes:
                print(f"{app}:   {count:5d} x {source}")
            for thread, thread_ticks in threads[:6]:
                print(f"{app}:   thread {thread} used {thread_ticks:.2f} ticks")
            measurement["idle_busy_threads"] = [
                {"thread": thread, "ticks": round(thread_ticks, 3)}
                for thread, thread_ticks in threads[:6]
            ]

        # Before the keys and menus below, which open panels over it.
        frame_error = check_window_frame(app, hwnd, screenshots)
        if not bring_forward(hwnd):
            return "could not bring the window forward to test its keys"
        errors = [
            error
            for error in (
                frame_error,
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
        if app == "rmac-calculator":
            error = check_title_area_button(app, hwnd)
            if error:
                errors.append(error)
        if app == "rmac-clock":
            error = check_clock_tabs(app, hwnd)
            if error:
                errors.append(error)
        if app == "rmac-preview":
            error = check_open_panel(app, process, hwnd)
            if error:
                errors.append(error)
        # After the checks above: it toggles full screen and minimises.
        error = check_window_keys(app, process, hwnd)
        if error:
            errors.append(error)
        if app == "rmac-terminal" and not errors:
            # Last: it closes the window.
            error = check_terminal_closes(app, process, hwnd)
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


FILE_ATTRIBUTE_HIDDEN = 0x2
FILE_ATTRIBUTE_SYSTEM = 0x4


def explorer_advanced(name: str) -> int | None:
    """A DWORD under Explorer's Advanced key in this user's registry."""
    import winreg

    try:
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        ) as key:
            value, _ = winreg.QueryValueEx(key, name)
            return int(value)
    except OSError:
        return None


def create_shortcut(link: Path, target: Path) -> bool:
    """A real shell link, as Explorer records an opened file under Recent."""
    script = (
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:LULO_LINK); "
        "$s.TargetPath = $env:LULO_TARGET; $s.Save()"
    )
    environment = dict(os.environ, LULO_LINK=str(link), LULO_TARGET=str(target))
    result = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
        env=environment,
        capture_output=True,
        timeout=60,
    )
    return result.returncode == 0 and link.exists()


def run_files_until(
    binary: Path, profile: Path, arguments: list[str], pattern: str, name: str
) -> tuple[str, "re.Match | None"]:
    """Run Files with its traces on until a stderr line matches `pattern`
    (or the window timeout passes); return the log and the match."""
    import re

    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    environment["RMAC_FILES_TRACE"] = "1"
    environment["RMAC_MENU_STRIP_TRACE"] = "1"
    log = profile / f"files-{name}.log"
    expression = re.compile(pattern, re.MULTILINE)
    with log.open("wb") as output:
        process = subprocess.Popen(
            [str(binary), *arguments], env=environment, stdout=output, stderr=subprocess.STDOUT
        )
    match = None
    try:
        deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
        while time.monotonic() < deadline and process.poll() is None:
            match = expression.search(log.read_text(encoding="utf-8", errors="replace"))
            if match:
                break
            time.sleep(0.1)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)
    return log.read_text(encoding="utf-8", errors="replace"), match


def check_files(binary: Path) -> list[str]:
    """Files on Windows (ADR 0023): hidden and protected items stay out as
    Explorer keeps them out, a fresh window opens on Recents (which lists a
    file Windows recorded as recently opened), and the menus use Windows'
    words and keys."""
    failures: list[str] = []
    kernel32 = ctypes.windll.kernel32
    with tempfile.TemporaryDirectory(prefix="rmac-files-check-", ignore_cleanup_errors=True) as directory:
        profile = Path(directory)
        for folder in ("Roaming", "Local"):
            (profile / folder).mkdir(parents=True, exist_ok=True)

        # 1. A fresh window opens on Recents, and the menus' wording.
        document = profile / "recent-report.txt"
        document.write_text("opened recently", encoding="utf-8")
        recent = profile / "Roaming" / "Microsoft" / "Windows" / "Recent"
        recent.mkdir(parents=True, exist_ok=True)
        if not create_shortcut(recent / "recent-report.txt.lnk", document):
            failures.append("could not make the Recent shortcut fixture")
        text, match = run_files_until(binary, profile, [], r"^files: recents (\d+): (.*)$", "default")
        if match is None:
            failures.append("a fresh Files window did not open on Recents")
        else:
            names = match.group(2).split(" | ")
            print(f"rmac-files: a fresh window opened on Recents with {match.group(1)} items: {names[:8]}")
            if "recent-report.txt" not in names:
                failures.append(f"Recents does not list the file Windows recorded as recent: {names}")
        menus = [line for line in text.splitlines() if line.startswith("menu strip: menu ")]
        for line in menus:
            print(f"rmac-files: {line}")
        file_menu = next((line for line in menus if line.startswith("menu strip: menu File: ")), "")
        go_menu = next((line for line in menus if line.startswith("menu strip: menu Go: ")), "")
        for expected in (
            "Move to Recycle Bin [Delete]",
            "Create Shortcut [Ctrl+L]",
            "Open File Location [Ctrl+R]",
            "Rename [F2]",
            "New Folder with Selection [Alt+Shift+N]",
            "Delete Immediately… [Shift+Delete]",
        ):
            if expected not in file_menu:
                failures.append(f"Files' File menu has no {expected!r}")
        for expected in ("Enclosing Folder [Alt+Up]", "This PC [Ctrl+Shift+C]", "Recycle Bin"):
            if expected not in go_menu:
                failures.append(f"Files' Go menu has no {expected!r}")
        for line in menus:
            for mac_word in ("Move to Trash", "Make Alias", "Show Original", "Win+"):
                if mac_word in line:
                    failures.append(f"a Files menu still says {mac_word!r}: {line}")

        # 2. Hidden and protected files, as Explorer's own setting shows them.
        fixture = profile / "fixture"
        fixture.mkdir()
        files = {
            "normal.txt": 0,
            "hidden.txt": FILE_ATTRIBUTE_HIDDEN,
            "protected.txt": FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM,
        }
        for name, attributes in files.items():
            (fixture / name).write_text(name, encoding="utf-8")
            if attributes and not kernel32.SetFileAttributesW(str(fixture / name), attributes):
                failures.append(f"could not set the attributes of {name}")
        (fixture / ".dotfile").write_text("dot", encoding="utf-8")
        show_hidden = explorer_advanced("Hidden") == 1
        show_protected = explorer_advanced("ShowSuperHidden") == 1
        expected = {"normal.txt"}
        if show_hidden:
            expected |= {"hidden.txt", ".dotfile"}
            if show_protected:
                expected.add("protected.txt")
        print(
            f"rmac-files: Explorer shows hidden items: {show_hidden}, protected files: "
            f"{show_protected}; expecting {sorted(expected)}"
        )
        _, match = run_files_until(
            binary, profile, ["--path", str(fixture)], r"^files: listed .*fixture: (.*)$", "fixture"
        )
        if match is None:
            failures.append("Files did not list the hidden-file fixture")
        else:
            listed = {name for name in match.group(1).split(" | ") if name}
            print(f"rmac-files: the fixture lists {sorted(listed)}")
            if listed != expected:
                failures.append(f"the fixture lists {sorted(listed)}, not {sorted(expected)}")
        for name, attributes in files.items():
            if attributes:
                kernel32.SetFileAttributesW(str(fixture / name), 0)
    if not failures:
        print("rmac-files: every Windows Files check passed")
    return failures


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


def measure_world_tick(bin_dir: Path, screenshots: Path | None) -> str | None:
    """Time Clock's World Clock redraw: launch it with the redraw period
    shortened to `WORLD_TICK_SECONDS`, leave it alone for the usual settle,
    then charge the CPU it used over the idle window to the redraws that
    fell inside it. The software rasteriser's share (WARP, the runner's
    stand-in for a GPU: about 9 ticks for any full frame of a 1024 x 768
    window, whatever the window shows) is reported apart from Clock's own."""
    app = "rmac-clock"
    with tempfile.TemporaryDirectory(prefix=f"{app}-tick-") as directory:
        profile = Path(directory)
        environment = dict(os.environ)
        environment["APPDATA"] = str(profile / "Roaming")
        environment["LOCALAPPDATA"] = str(profile / "Local")
        environment["RMAC_GPUI_WAKE_TRACE"] = "1"
        environment["RMAC_CLOCK_WORLD_TICK_MS"] = str(int(WORLD_TICK_SECONDS * 1000))
        for folder in ("Roaming", "Local"):
            (profile / folder).mkdir(parents=True, exist_ok=True)
        log = profile / "output.log"
        with log.open("wb") as output:
            process = subprocess.Popen(
                [str(bin_dir / f"{app}.exe")],
                env=environment,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
        try:
            found = wait_for_window(process.pid)
            if found is None:
                return f"{app} opened no visible window within {WINDOW_TIMEOUT_SECONDS:.0f} s"
            bring_forward(found[0])
            idle = measure_idle_cpu(process.pid, log=log)
            if idle is None:
                return f"{app}: could not read its CPU time"
            ticks, _percent, trace, threads = idle
            if screenshots is not None:
                save(grab(), screenshots / f"{app}-world-tick.png")
            redraws = IDLE_WINDOW_SECONDS / WORLD_TICK_SECONDS
            drawn = sum(
                1 for line in trace.splitlines() if line.startswith(WAKE_PREFIX + "frame drew")
            )
            renderer = software_renderer_ticks(threads, trace)
            own = max(ticks - renderer, 0.0)
            print(
                f"{app}: World Clock redraw every {WORLD_TICK_SECONDS:g} s: "
                f"{ticks:.2f} ticks over {IDLE_WINDOW_SECONDS:.0f} s, {drawn} frames drawn; "
                f"per minute tick {own / redraws:.2f} ticks of Clock's own and "
                f"{renderer / redraws:.2f} in the software rasteriser"
            )
            for count, source in summarize_wakes(trace):
                print(f"{app}:   {count:5d} x {source}")
            for thread, thread_ticks in threads[:8]:
                print(f"{app}:   thread {thread} used {thread_ticks:.2f} ticks")
            MEASUREMENTS.setdefault(app, {})["world_tick"] = {
                "period_ms": int(WORLD_TICK_SECONDS * 1000),
                "redraws": redraws,
                "frames_drawn": drawn,
                "ticks": round(ticks, 3),
                "ticks_per_redraw": round(ticks / redraws, 3),
                "renderer_ticks_per_redraw": round(renderer / redraws, 3),
                "own_ticks_per_redraw": round(own / redraws, 3),
                "busy_threads": [
                    {"thread": thread, "ticks": round(thread_ticks, 3)}
                    for thread, thread_ticks in threads[:8]
                ],
            }
            return None
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=30)


def write_silent_wav(path: Path) -> None:
    """A quarter second of 8 kHz mono silence."""
    import wave

    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(1)
        output.setframerate(8000)
        output.writeframes(b"\x80" * 2000)


def check_archive_utility(binary: Path) -> str | None:
    """Archive Utility has no main window (as on the Mac): handed a zip it
    expands it beside the archive and exits; handed nothing it exits at once.
    Print how long the expansion took."""
    import zipfile

    with tempfile.TemporaryDirectory(prefix="rmac-archive-check-", ignore_cleanup_errors=True) as directory:
        root = Path(directory)
        profile = root / "profile"
        for folder in ("Roaming", "Local"):
            (profile / folder).mkdir(parents=True)
        environment = dict(os.environ)
        environment["APPDATA"] = str(profile / "Roaming")
        environment["LOCALAPPDATA"] = str(profile / "Local")
        archive = root / "Fixture.zip"
        with zipfile.ZipFile(archive, "w") as bundle:
            bundle.writestr("Fixture/one.txt", "one")
            bundle.writestr("Fixture/two/two.txt", "two")
            bundle.writestr("Fixture/CON", "refused: a Windows device name")
        started = time.monotonic()
        result = subprocess.run(
            [str(binary), str(archive)],
            env=environment,
            capture_output=True,
            timeout=60,
        )
        elapsed_ms = (time.monotonic() - started) * 1000
        print(f"rmac-archive-utility: expanded a zip and exited {result.returncode} in {elapsed_ms:.0f} ms")
        if result.returncode != 0:
            return f"exited {result.returncode} expanding a zip: {result.stderr.decode(errors='replace')[-400:]}"
        expanded = root / "Fixture"
        one = expanded / "one.txt"
        if not one.exists() or one.read_text() != "one":
            return "the archive was not expanded beside itself"
        if not (expanded / "two" / "two.txt").exists():
            return "a nested entry was not expanded"
        if (expanded / "CON").exists():
            return "a Windows device name was written"
        empty = subprocess.run([str(binary)], env=environment, capture_output=True, timeout=60)
        if empty.returncode != 0:
            return f"exited {empty.returncode} with nothing to expand"
    return None


def main() -> int:
    # Menu labels carry "…" and other characters the runner's console code
    # page cannot print.
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    # None with --shell: the Lulo layer job checks only lulo-session and
    # lulo-shell (the apps it opens come from bin_dir).
    parser.add_argument("apps", nargs="*")
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
    parser.add_argument(
        "--world-tick-check",
        action="store_true",
        help=(
            "Launch rmac-clock again with a shortened World Clock redraw "
            "period and report what one redraw costs."
        ),
    )
    parser.add_argument(
        "--shell",
        action="store_true",
        help="Also check the Lulo layer (lulo-session.exe and lulo-shell.exe in bin_dir).",
    )
    parser.add_argument(
        "--results",
        type=Path,
        help="Write each app's launch time, idle ticks and wake-ups here as JSON.",
    )
    arguments = parser.parse_args()
    if not arguments.apps and not arguments.shell:
        parser.error("name at least one app, or pass --shell")
    if sys.platform != "win32":
        print("launch_smoke.py runs only on Windows", file=sys.stderr)
        return 2
    # Physical pixels, so window rectangles and captures agree.
    ctypes.windll.user32.SetProcessDPIAware()

    failures = 0
    for app in arguments.apps:
        binary = arguments.bin_dir / f"{app}.exe"
        if app == "rmac-archive-utility":
            error = check_archive_utility(binary)
            if error is not None:
                failures += 1
                print(f"{app}: FAIL: {error}")
            continue
        with tempfile.TemporaryDirectory(prefix=f"{app}-") as directory:
            profile = Path(directory)
            app_arguments: list[str] = []
            if app == "rmac-preview":
                fixture = profile / "fixture.pdf"
                write_minimal_pdf(fixture)
                app_arguments = [str(fixture)]
            if app == "rmac-player":
                # Player has no window until it has a file (it asks for one
                # first); a quarter second of silence gives it one.
                fixture = profile / "fixture.wav"
                write_silent_wav(fixture)
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
                    text = log.read_text(encoding="utf-8", errors="replace")
                    lines = [
                        line for line in text.splitlines() if not line.startswith(WAKE_PREFIX)
                    ]
                    print("\n".join(lines)[-4000:])

    if "rmac-files" in arguments.apps:
        for failure in check_files(arguments.bin_dir / "rmac-files.exe"):
            failures += 1
            print(f"rmac-files: FAIL: {failure}")

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
    if arguments.world_tick_check:
        error = measure_world_tick(arguments.bin_dir, arguments.screenshots)
        if error is not None:
            failures += 1
            print(f"world tick: FAIL: {error}")
    if arguments.shell:
        import shell_smoke  # this script's own folder is on sys.path

        with tempfile.TemporaryDirectory(prefix="lulo-shell-", ignore_cleanup_errors=True) as directory:
            shell_failures = shell_smoke.check_shell(
                arguments.bin_dir,
                Path(directory),
                arguments.screenshots,
                MEASUREMENTS,
                measure_idle_cpu,
                process_cpu_time_100ns,
                summarize_wakes,
                startup_phases,
                WAKE_PREFIX,
                TICK_100NS,
                software_renderer_ticks,
            )
        for failure in shell_failures:
            failures += 1
            print(f"shell: FAIL: {failure}")
        if not shell_failures:
            print("shell: every Lulo layer check passed")
    if arguments.results is not None:
        arguments.results.write_text(json.dumps(MEASUREMENTS, indent=2), encoding="utf-8")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
