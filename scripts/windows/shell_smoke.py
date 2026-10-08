"""The Lulo layer on Windows (ADR 0023 phase 3), checked on a real desktop.

`launch_smoke.py --shell` runs this after the apps' own checks. It starts
`lulo-session.exe` (which runs `lulo-shell.exe`) with `LULO_SHELL_TRACE=1`
and private profile folders, then checks, with real input:

1. the menu bar reserves the top of the work area and the Dock the bottom,
   and the taskbar is hidden while Lulo runs;
2. both processes stay idle (the same 20 s window as the apps; the result
   goes to `--results`, so `idle_gate.py` gates it);
3. a click on the Dock's Calculator tile opens Lulo's Calculator, and its
   tile gets the running dot;
4. the bar shows Calculator's own menus over the menu pipe, and choosing
   Calculator ▸ About Calculator in the bar opens Calculator's About panel;
5. a maximised window stops below the bar and above the Dock;
6. Alt+Space opens Spotlight, typing finds Text Editor and Return opens it
   (and, reported but not required, "notepad" finds Windows' Notepad
   through the Apps folder);
7. `lulo-session --stop` turns the layer off, and the work area, the
   taskbar's visibility and its auto-hide setting are as before.

Screenshots of each step go to `<screenshots>/shell-*.png`.
"""

from __future__ import annotations

import ctypes
import os
import re
import subprocess
import time
from ctypes import wintypes
from pathlib import Path

READY_TIMEOUT_SECONDS = 60.0
STEP_TIMEOUT_SECONDS = 30.0
SETTLE_SECONDS = 2.0

VK_MENU = 0x12
VK_SPACE = 0x20
VK_RETURN = 0x0D
VK_DOWN = 0x28
VK_ESCAPE = 0x1B
KEYEVENTF_KEYUP = 0x0002
SPI_GETWORKAREA = 0x0030
ABM_GETSTATE = 0x4
SW_MAXIMIZE = 3
DWMWA_EXTENDED_FRAME_BOUNDS = 9
TH32CS_SNAPPROCESS = 0x00000002
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_TERMINATE = 0x0001


def user32():
    return ctypes.windll.user32


class _APPBARDATA(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("hWnd", wintypes.HWND),
        ("uCallbackMessage", wintypes.UINT),
        ("uEdge", wintypes.UINT),
        ("rc", wintypes.RECT),
        ("lParam", wintypes.LPARAM),
    ]


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


def work_area() -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    user32().SystemParametersInfoW(SPI_GETWORKAREA, 0, ctypes.byref(rect), 0)
    return rect.left, rect.top, rect.right, rect.bottom


def taskbar_hwnd() -> int:
    return user32().FindWindowW("Shell_TrayWnd", None) or 0


def taskbar_state() -> tuple[bool, int | None]:
    """(visible, ABS_* state) of the primary taskbar."""
    hwnd = taskbar_hwnd()
    if not hwnd:
        return False, None
    data = _APPBARDATA()
    data.cbSize = ctypes.sizeof(_APPBARDATA)
    data.hWnd = hwnd
    shell32 = ctypes.windll.shell32
    shell32.SHAppBarMessage.restype = ctypes.c_size_t
    state = shell32.SHAppBarMessage(ABM_GETSTATE, ctypes.byref(data))
    return bool(user32().IsWindowVisible(hwnd)), int(state)


def processes_named(name: str) -> list[int]:
    kernel32 = ctypes.windll.kernel32
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    found: list[int] = []
    if not snapshot or snapshot == wintypes.HANDLE(-1).value:
        return found
    try:
        entry = _PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(_PROCESSENTRY32W)
        more = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            if entry.szExeFile.lower() == name.lower():
                found.append(entry.th32ProcessID)
            more = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return found


def terminate(pid: int) -> None:
    kernel32 = ctypes.windll.kernel32
    kernel32.OpenProcess.restype = wintypes.HANDLE
    handle = kernel32.OpenProcess(PROCESS_TERMINATE, False, pid)
    if handle:
        kernel32.TerminateProcess(handle, 1)
        kernel32.CloseHandle(handle)


def windows_of(pid: int, titled: bool = False) -> list[int]:
    """Visible top-level windows of `pid` (with a title, if `titled`)."""
    found: list[int] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32().GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32().IsWindowVisible(hwnd):
            if not titled or user32().GetWindowTextLengthW(hwnd) > 0:
                found.append(hwnd)
        return True

    user32().EnumWindows(collect, 0)
    return found


def frame_bounds(hwnd: int) -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    ctypes.windll.dwmapi.DwmGetWindowAttribute(
        wintypes.HWND(hwnd),
        DWMWA_EXTENDED_FRAME_BOUNDS,
        ctypes.byref(rect),
        ctypes.sizeof(rect),
    )
    return rect.left, rect.top, rect.right, rect.bottom


def key(code: int, up: bool = False) -> None:
    user32().keybd_event(code, 0, KEYEVENTF_KEYUP if up else 0, 0)
    time.sleep(0.04)


def tap(code: int) -> None:
    key(code)
    key(code, up=True)


def type_text(text: str) -> None:
    for character in text:
        if character == " ":
            tap(VK_SPACE)
        else:
            tap(ord(character.upper()))
        time.sleep(0.03)


def click(x: int, y: int) -> None:
    user32().SetCursorPos(x, y)
    time.sleep(0.15)
    user32().mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.05)
    user32().mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.05)


def grab():
    try:
        from PIL import ImageGrab  # type: ignore[import-not-found]
    except ImportError:
        return None
    return ImageGrab.grab()


def save(screenshots: Path | None, name: str) -> None:
    if screenshots is None:
        return
    image = grab()
    if image is not None:
        screenshots.mkdir(parents=True, exist_ok=True)
        image.save(screenshots / f"shell-{name}.png")


class Log:
    """The session's and the shell's stderr (and that of the apps the
    shell opens, which inherit it)."""

    def __init__(self, path: Path):
        self.path = path

    def text(self) -> str:
        if not self.path.exists():
            return ""
        return self.path.read_text(encoding="utf-8", errors="replace")

    def lines(self, prefix: str = "lulo-shell: ") -> list[str]:
        return [line[len(prefix) :] for line in self.text().splitlines() if line.startswith(prefix)]

    def wait_for(self, pattern: str, timeout: float = STEP_TIMEOUT_SECONDS, after: int = 0):
        """The first shell trace line from index `after` on that matches
        `pattern`, as a `re.Match`, or `None` after `timeout`."""
        expression = re.compile(pattern)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for line in self.lines()[after:]:
                match = expression.search(line)
                if match:
                    return match
            time.sleep(0.05)
        return None

    def last(self, pattern: str):
        expression = re.compile(pattern)
        for line in reversed(self.lines()):
            match = expression.search(line)
            if match:
                return match
        return None


def wait_until(predicate, timeout: float = STEP_TIMEOUT_SECONDS, interval: float = 0.05):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    return None


def app_window(exe: str) -> tuple[int, int] | None:
    """(pid, hwnd) of the first titled window of a process named `exe`."""
    for pid in processes_named(exe):
        windows = windows_of(pid, titled=True)
        if windows:
            return pid, windows[0]
    return None


def check_shell(
    bin_dir: Path,
    profile: Path,
    screenshots: Path | None,
    measurements: dict[str, dict],
    measure_idle_cpu,
    process_cpu_time_100ns,
    summarize_wakes,
    startup_phases,
    wake_prefix: str,
    tick_100ns: int,
    software_renderer_ticks=None,
) -> list[str]:
    """Run every check; return the failures."""
    failures: list[str] = []
    baseline_area = work_area()
    baseline_taskbar = taskbar_state()
    print(f"shell: before Lulo: work area {baseline_area}, taskbar (visible, state) {baseline_taskbar}")
    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    environment["LULO_SHELL_TRACE"] = "1"
    environment["RMAC_GPUI_STARTUP_TRACE"] = "1"
    environment["RMAC_GPUI_WAKE_TRACE"] = "1"
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    log_path = profile / "shell.log"
    log = Log(log_path)
    started = time.monotonic()
    with log_path.open("wb") as output:
        session = subprocess.Popen(
            [str(bin_dir / "lulo-session.exe")],
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
        )
    opened: list[str] = []
    try:
        starting = log.wait_for(r"^starting pid (\d+)", READY_TIMEOUT_SECONDS)
        if starting is None:
            return [f"lulo-shell did not start: {log.text()[-2000:]}"]
        shell_pid = int(starting.group(1))
        bar = wait_until(lambda: windows_of(shell_pid), READY_TIMEOUT_SECONDS, 0.02)
        launch_ms = (time.monotonic() - started) * 1000
        ready = log.wait_for(r"^ready at (\d+) ms", READY_TIMEOUT_SECONDS)
        if not bar or ready is None:
            return [f"the Lulo layer did not come up: {log.text()[-3000:]}"]
        print(f"shell: launch ~{launch_ms:.0f} ms (session start to the first shell window)")
        print(f"shell: ready (bar and Dock placed) {ready.group(1)} ms after lulo-shell started")
        placed = log.wait_for(r"^bar at (-?\d+),(-?\d+),(-?\d+),(-?\d+) dock strip at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
        time.sleep(SETTLE_SECONDS)
        save(screenshots, "desktop")

        # 1. The work area.
        area = work_area()
        visible, state = taskbar_state()
        print(f"shell: while Lulo runs: work area {area}, taskbar visible {visible}, state {state}")
        if placed is None:
            failures.append("the shell did not report where it placed the bar")
        else:
            bar_bottom = int(placed.group(4))
            dock_top = int(placed.group(6))
            if area[1] != bar_bottom:
                failures.append(f"the work area starts at y={area[1]}, not below the bar (y={bar_bottom})")
            if area[3] != dock_top:
                failures.append(f"the work area ends at y={area[3]}, not above the Dock (y={dock_top})")
        if visible:
            failures.append("the Windows taskbar is still visible while Lulo runs")

        # 2. Idle, before any input.
        phases = startup_phases(log.text())
        for name, pid in (("lulo-shell", shell_pid), ("lulo-session", session.pid)):
            measurement: dict = {"launch_ms": round(launch_ms), "startup": phases if name == "lulo-shell" else []}
            if name == "lulo-shell":
                measurement["ready_ms"] = int(ready.group(1))
            measurements[name] = measurement
        # The session is measured over the same window as the shell.
        session_before = process_cpu_time_100ns(session.pid)
        idle_shell = measure_idle_cpu(shell_pid, log=log_path)
        session_after = process_cpu_time_100ns(session.pid)
        idle_session = None
        if session_before is not None and session_after is not None:
            idle_session = (max(session_after - session_before, 0) / tick_100ns, 0.0, "", [])
        for name, idle in (("lulo-shell", idle_shell), ("lulo-session", idle_session)):
            if idle is None:
                print(f"shell: {name}: idle CPU skipped (could not read its CPU time)")
                continue
            ticks, percent, trace, threads = idle
            wakes = summarize_wakes(trace) if name == "lulo-shell" else []
            total = sum(1 for line in trace.splitlines() if line.startswith(wake_prefix))
            measurements[name].update(
                {
                    "idle_ticks": round(ticks, 3),
                    # WARP, the runner's software GPU (launch_smoke.py).
                    "idle_renderer_ticks": round(software_renderer_ticks(threads, trace), 3)
                    if software_renderer_ticks and name == "lulo-shell"
                    else 0.0,
                    "idle_wakes": total if name == "lulo-shell" else 0,
                    "idle_wake_sources": [{"count": count, "source": source} for count, source in wakes],
                    "idle_busy_threads": [
                        {"thread": thread, "ticks": round(thread_ticks, 3)} for thread, thread_ticks in threads[:6]
                    ],
                }
            )
            print(f"shell: {name}: idle CPU = {ticks:.2f} ticks ({percent:.2f}% of one core), {total} wake-ups")
            for count, source in wakes:
                print(f"shell: {name}:   {count:5d} x {source}")
            if name == "lulo-shell" and total:
                # Every wake-up in order, to name what woke an idle shell.
                for line in [line for line in trace.splitlines() if line.startswith(wake_prefix)][:150]:
                    print(f"shell: lulo-shell idle: {line[len(wake_prefix):]}")
            for thread, thread_ticks in threads[:6]:
                print(f"shell: {name}:   thread {thread} used {thread_ticks:.2f} ticks")

        # 3. The Dock opens Calculator.
        tile = log.last(r"^dock tile rmac-calculator\.exe Calculator at (\d+),(\d+)")
        if tile is None:
            failures.append("the Dock has no Calculator tile")
        else:
            before = len(log.lines())
            click(int(tile.group(1)), int(tile.group(2)))
            calculator = wait_until(lambda: app_window("rmac-calculator.exe"))
            if calculator is None:
                failures.append("clicking the Dock's Calculator tile opened no Calculator window")
            else:
                opened.append("rmac-calculator.exe")
                print("shell: the Dock's Calculator tile opened Calculator")
                if log.wait_for(r"^dock tile rmac-calculator\.exe Calculator at \d+,\d+ running", after=before) is None:
                    failures.append("Calculator's Dock tile shows no running dot")
                time.sleep(SETTLE_SECONDS)
                save(screenshots, "dock-opened-calculator")

                # 4. Calculator's own menus in the bar, and About from them.
                menus = log.wait_for(r"^menus from org\.rmac\.Calculator", after=before)
                title = log.wait_for(r"^bar title 1 Calculator at (-?\d+),(-?\d+),(\d+),(\d+)", after=before)
                if menus is None or title is None:
                    failures.append("the bar does not show Calculator's menus")
                else:
                    pid, _ = calculator
                    windows_before = len(windows_of(pid, titled=True))
                    x = int(title.group(1)) + int(title.group(3)) // 2
                    y = int(title.group(2)) + int(title.group(4)) // 2
                    click(x, y)
                    if log.wait_for(r"^menu 1 open", after=before) is None:
                        failures.append("clicking Calculator in the bar opened no menu")
                    time.sleep(1.0)
                    save(screenshots, "menu-calculator")
                    tap(VK_DOWN)
                    time.sleep(0.3)
                    tap(VK_RETURN)
                    about = wait_until(lambda: len(windows_of(pid, titled=True)) > windows_before, 10.0)
                    time.sleep(1.0)
                    save(screenshots, "calculator-about")
                    if not about:
                        failures.append("Calculator > About Calculator in the bar opened no About panel")
                    else:
                        print("shell: the bar's Calculator > About Calculator opened Calculator's About panel")

        # 5. A maximised window stays between the bar and the Dock.
        subprocess.Popen(["notepad.exe"])
        found = wait_until(lambda: app_window("notepad.exe"))
        window = found[1] if found else None
        if window is None:
            failures.append("Notepad opened no window to maximise")
        else:
            user32().ShowWindow(window, SW_MAXIMIZE)
            time.sleep(SETTLE_SECONDS)
            bounds = frame_bounds(window)
            area = work_area()
            save(screenshots, "maximised")
            print(f"shell: maximised Notepad: {bounds}, work area {area}")
            if bounds[1] < area[1] or bounds[3] > area[3]:
                failures.append(f"a maximised window ({bounds}) goes under the bar or the Dock ({area})")
        for pid in processes_named("notepad.exe"):
            terminate(pid)
        time.sleep(1.0)

        # 6. Spotlight.
        before = len(log.lines())
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        if log.wait_for(r"^spotlight shown", after=before, timeout=10.0) is None:
            failures.append("Alt+Space did not open Spotlight")
        else:
            time.sleep(0.8)
            type_text("text editor")
            results = log.wait_for(r'^spotlight "text editor": (\d+) results', after=before, timeout=10.0)
            time.sleep(0.8)
            save(screenshots, "spotlight")
            if results is None or int(results.group(1)) == 0:
                failures.append("Spotlight found nothing for \"text editor\"")
            tap(VK_RETURN)
            if wait_until(lambda: app_window("rmac-text-editor.exe")) is None:
                failures.append("Return in Spotlight did not open Text Editor")
            else:
                opened.append("rmac-text-editor.exe")
                print("shell: Spotlight opened Text Editor")
                time.sleep(SETTLE_SECONDS)
                save(screenshots, "spotlight-opened-text-editor")
        # The Start menu's apps through the Apps folder: reported only.
        before = len(log.lines())
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        if log.wait_for(r"^spotlight shown", after=before, timeout=10.0):
            time.sleep(0.8)
            type_text("notepad")
            log.wait_for(r'^spotlight "notepad"', after=before, timeout=10.0)
            time.sleep(0.5)
            save(screenshots, "spotlight-notepad")
            tap(VK_RETURN)
            found = wait_until(lambda: app_window("notepad.exe"), 15.0)
            print(f"shell: Spotlight {'opened' if found else 'did not open'} Notepad from the Apps folder")
            if found:
                opened.append("notepad.exe")
        catalog = log.last(r"^catalog: (\d+) apps")
        print(f"shell: Spotlight's catalogue: {catalog.group(1) if catalog else '?'} apps")

        # 7. Turning Lulo off gives the desktop back.
        subprocess.run([str(bin_dir / "lulo-session.exe"), "--stop"], env=environment, timeout=30)
        try:
            session.wait(timeout=STEP_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            failures.append("lulo-session kept running after --stop")
        time.sleep(SETTLE_SECONDS)
        save(screenshots, "restored")
        restored_area = work_area()
        restored_taskbar = taskbar_state()
        print(f"shell: after Lulo: work area {restored_area}, taskbar (visible, state) {restored_taskbar}")
        if restored_area != baseline_area:
            failures.append(f"the work area is {restored_area} after Lulo, not {baseline_area} as before")
        if restored_taskbar != baseline_taskbar:
            failures.append(f"the taskbar is {restored_taskbar} after Lulo, not {baseline_taskbar} as before")
        if processes_named("lulo-shell.exe"):
            failures.append("lulo-shell kept running after --stop")
    finally:
        for exe in ["lulo-shell.exe", *opened]:
            for pid in processes_named(exe):
                terminate(pid)
        if session.poll() is None:
            session.kill()
        trace = [
            line
            for line in log.text().splitlines()
            if line.startswith("lulo-") and not line.startswith(wake_prefix)
        ]
        for line in trace[:200]:
            print(f"shell log: {line}")
    return failures
