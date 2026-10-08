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

and, for the fixes after the first real-PC test (ADR 0023, WIN-OS-30 on):

- each exe names itself (`FileDescription`: Files is "Files", not
  Preview's) and, when `LULO_EXPECT_EXE_ICONS=1`, carries an icon;
- the desktop's icon list view moves below the bar while Lulo runs and is
  back where it was afterwards;
- the bar and the Dock get the acrylic backdrop (when Windows'
  transparency effects are on);
- the Dock has the Recycle Bin tile, which turns full when a file is
  recycled and opens its menu on a right-click, and Files shows with its
  own icon even run from another executable name;
- lulo-shell's working set and private bytes at idle and after Spotlight
  let go of its window (`idle_gate.py --shell-memory-mb` gates the idle
  working set);
- with Alt+Space held by another process: the fallback hotkey is traced,
  a one-time notice names it, it opens Spotlight with Spotlight in front,
  the bar's magnifier opens Spotlight, Alt+Space still reaches the other
  process, and the notice does not show again on the next start.

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


VK_LWIN = 0x5B
VK_CONTROL = 0x11
VK_SHIFT = 0x10
GWL_STYLE = -16
LVS_AUTOARRANGE = 0x0100
WCA_ACCENT_POLICY = 19
ACCENT_ENABLE_ACRYLICBLURBEHIND = 4
PROCESS_VM_READ = 0x0010
MOD_ALT = 0x0001
MOD_NOREPEAT = 0x4000
WM_HOTKEY = 0x0312
MEGABYTE = 1024 * 1024
#: The Lulo layer's working set at idle, on an 8 GB PC (ADR 0023).
SHELL_MEMORY_BUDGET_MB = 60.0


def find_window_ex(parent: int | None, after: int | None, cls: str) -> int:
    function = user32().FindWindowExW
    function.argtypes = [wintypes.HWND, wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR]
    function.restype = wintypes.HWND
    return function(parent, after, cls, None) or 0


def desktop_list_view() -> int:
    """Explorer's desktop icon list view (in Progman, or in a WorkerW)."""

    def in_parent(parent: int) -> int:
        view = find_window_ex(parent, None, "SHELLDLL_DefView")
        return find_window_ex(view, None, "SysListView32") if view else 0

    found = 0
    progman = user32().FindWindowW("Progman", None)
    if progman:
        found = in_parent(progman)
    worker = None
    while not found:
        worker = find_window_ex(None, worker, "WorkerW")
        if not worker:
            break
        found = in_parent(worker)
    return found


def window_rect(hwnd: int) -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    user32().GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(rect))
    return rect.left, rect.top, rect.right, rect.bottom


def auto_arranged(list_view: int) -> bool:
    return bool(user32().GetWindowLongW(wintypes.HWND(list_view), GWL_STYLE) & LVS_AUTOARRANGE)


class _ACCENTPOLICY(ctypes.Structure):
    _fields_ = [
        ("state", ctypes.c_uint),
        ("flags", ctypes.c_uint),
        ("gradient", ctypes.c_uint),
        ("animation", ctypes.c_uint),
    ]


class _COMPOSITIONDATA(ctypes.Structure):
    _fields_ = [
        ("attribute", ctypes.c_uint),
        ("data", ctypes.c_void_p),
        ("size", ctypes.c_size_t),
    ]


def accent_state(hwnd: int) -> int | None:
    """The acrylic/blur accent Windows draws behind `hwnd`, as
    `GetWindowCompositionAttribute` reports it, or `None` when it cannot be
    read."""
    function = getattr(user32(), "GetWindowCompositionAttribute", None)
    if function is None:
        return None
    accent = _ACCENTPOLICY()
    data = _COMPOSITIONDATA(
        WCA_ACCENT_POLICY, ctypes.cast(ctypes.pointer(accent), ctypes.c_void_p), ctypes.sizeof(accent)
    )
    function.argtypes = [wintypes.HWND, ctypes.POINTER(_COMPOSITIONDATA)]
    function.restype = wintypes.BOOL
    if not function(wintypes.HWND(hwnd), ctypes.byref(data)):
        return None
    return int(accent.state)


def transparency_effects() -> bool:
    import winreg

    try:
        with winreg.OpenKey(
            winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
        ) as key:
            return winreg.QueryValueEx(key, "EnableTransparency")[0] != 0
    except OSError:
        return True


def shell_window(name: str) -> int:
    """`BarWindow` or `DockWindow`, as lulo-shell records them."""
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Lulo\Shell") as key:
            return int(winreg.QueryValueEx(key, name)[0])
    except OSError:
        return 0


class _PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
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


def memory_mb(pid: int) -> dict[str, float] | None:
    """Working set, peak working set and private bytes of `pid`, in MB."""
    kernel32 = ctypes.windll.kernel32
    kernel32.OpenProcess.restype = wintypes.HANDLE
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, False, pid)
    if not handle:
        return None
    try:
        counters = _PROCESS_MEMORY_COUNTERS_EX()
        counters.cb = ctypes.sizeof(counters)
        if not kernel32.K32GetProcessMemoryInfo(
            wintypes.HANDLE(handle), ctypes.byref(counters), counters.cb
        ):
            return None
        return {
            "working_set_mb": round(counters.WorkingSetSize / MEGABYTE, 1),
            "peak_working_set_mb": round(counters.PeakWorkingSetSize / MEGABYTE, 1),
            "private_mb": round(counters.PrivateUsage / MEGABYTE, 1),
        }
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(handle))


def file_description(path: Path) -> str | None:
    """The executable's `FileDescription` version string."""
    version = ctypes.windll.version
    size = version.GetFileVersionInfoSizeW(str(path), None)
    if not size:
        return None
    data = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoW(str(path), 0, size, data):
        return None
    pointer = ctypes.c_void_p()
    length = wintypes.UINT()
    if not version.VerQueryValueW(
        data, "\\VarFileInfo\\Translation", ctypes.byref(pointer), ctypes.byref(length)
    ) or length.value < 4:
        return None
    language, code_page = ctypes.cast(pointer, ctypes.POINTER(ctypes.c_ushort * 2)).contents
    query = f"\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"
    if not version.VerQueryValueW(data, query, ctypes.byref(pointer), ctypes.byref(length)):
        return None
    return ctypes.wstring_at(pointer.value, max(length.value - 1, 0))


def icon_count(path: Path) -> int:
    shell32 = ctypes.windll.shell32
    shell32.ExtractIconExW.restype = wintypes.UINT
    return int(shell32.ExtractIconExW(str(path), -1, None, None, 0))


class _SHFILEOPSTRUCTW(ctypes.Structure):
    _fields_ = [
        ("hwnd", wintypes.HWND),
        ("wFunc", wintypes.UINT),
        ("pFrom", wintypes.LPCWSTR),
        ("pTo", wintypes.LPCWSTR),
        ("fFlags", wintypes.WORD),
        ("fAnyOperationsAborted", wintypes.BOOL),
        ("hNameMappings", ctypes.c_void_p),
        ("lpszProgressTitle", wintypes.LPCWSTR),
    ]


def recycle(path: Path) -> bool:
    """Move `path` to the Recycle Bin, as Explorer's Delete does."""
    operation = _SHFILEOPSTRUCTW()
    operation.wFunc = 3  # FO_DELETE
    operation.pFrom = str(path) + "\0"
    # FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI
    operation.fFlags = 0x40 | 0x10 | 0x4 | 0x400
    return ctypes.windll.shell32.SHFileOperationW(ctypes.byref(operation)) == 0


def foreground_pid_and_class() -> tuple[int, str]:
    hwnd = user32().GetForegroundWindow()
    owner = wintypes.DWORD()
    user32().GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
    buffer = ctypes.create_unicode_buffer(256)
    user32().GetClassNameW(hwnd, buffer, 256)
    return owner.value, buffer.value


HOTKEY_HOLDER = """
import ctypes, sys
from ctypes import wintypes
user32 = ctypes.windll.user32
ok = user32.RegisterHotKey(None, 1, 0x0001 | 0x4000, 0x20)
print("registered" if ok else "refused", flush=True)
message = wintypes.MSG()
while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
    if message.message == 0x0312:
        print("hotkey", flush=True)
"""


class HotkeyHolder:
    """Another app holding Alt+Space, as the ChatGPT app or PowerToys Run
    do: a process that registered it first and prints each press."""

    def __init__(self):
        import sys
        import threading

        self.lines: list[str] = []
        self.process = subprocess.Popen(
            [sys.executable, "-c", HOTKEY_HOLDER],
            stdout=subprocess.PIPE,
            text=True,
        )

        def read():
            assert self.process.stdout is not None
            for line in self.process.stdout:
                self.lines.append(line.strip())

        threading.Thread(target=read, daemon=True).start()

    def registered(self) -> bool:
        return wait_until(lambda: self.lines, 10.0) is not None and self.lines[0] == "registered"

    def presses(self) -> int:
        return self.lines.count("hotkey")

    def stop(self) -> None:
        self.process.kill()


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


def start_session(bin_dir: Path, environment: dict, log_path: Path) -> subprocess.Popen:
    with log_path.open("ab") as output:
        return subprocess.Popen(
            [str(bin_dir / "lulo-session.exe")],
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
        )


def stop_session(bin_dir: Path, environment: dict, session: subprocess.Popen, failures: list[str]) -> None:
    subprocess.run([str(bin_dir / "lulo-session.exe"), "--stop"], env=environment, timeout=30)
    try:
        session.wait(timeout=STEP_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired:
        failures.append("lulo-session kept running after --stop")


def check_resources(bin_dir: Path, failures: list[str]) -> None:
    """Each exe carries its own name and icon: Files is not Preview (its
    resources once leaked into Files through Quick Look)."""
    expected = {
        "rmac-files.exe": "Files",
        "rmac-preview.exe": "Preview",
        "lulo-shell.exe": "Lulo",
        "lulo-session.exe": "Lulo",
    }
    for exe, name in expected.items():
        path = bin_dir / exe
        if not path.exists():
            continue
        description = file_description(path)
        icons = icon_count(path)
        print(f"shell: {exe}: FileDescription {description!r}, {icons} icon(s)")
        if description != name:
            failures.append(f"{exe} describes itself as {description!r}, not {name!r}")
        if os.environ.get("LULO_EXPECT_EXE_ICONS") == "1" and icons < 1:
            failures.append(f"{exe} carries no icon of its own")


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
) -> list[str]:
    """Run every check; return the failures."""
    failures: list[str] = []
    check_resources(bin_dir, failures)
    baseline_area = work_area()
    baseline_taskbar = taskbar_state()
    list_view = desktop_list_view()
    baseline_icons = window_rect(list_view) if list_view else None
    print(
        f"shell: before Lulo: work area {baseline_area}, taskbar (visible, state) {baseline_taskbar}, "
        f"desktop icons {baseline_icons}"
        + (" (auto-arranged)" if list_view and auto_arranged(list_view) else "")
    )
    environment = dict(os.environ)
    environment["APPDATA"] = str(profile / "Roaming")
    environment["LOCALAPPDATA"] = str(profile / "Local")
    environment["LULO_SHELL_TRACE"] = "1"
    environment["RMAC_GPUI_STARTUP_TRACE"] = "1"
    environment["RMAC_GPUI_WAKE_TRACE"] = "1"
    # Closed panels let go of their windows after 3 s instead of 30, so the
    # memory they give back can be measured here.
    environment["LULO_PANEL_RELEASE_SECONDS"] = "3"
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    log_path = profile / "shell.log"
    log = Log(log_path)
    started = time.monotonic()
    session = start_session(bin_dir, environment, log_path)
    opened: list[str] = []
    holder: HotkeyHolder | None = None
    try:
        starting = log.wait_for(r"^starting pid (\d+)", READY_TIMEOUT_SECONDS)
        if starting is None:
            return failures + [f"lulo-shell did not start: {log.text()[-2000:]}"]
        shell_pid = int(starting.group(1))
        bar = wait_until(lambda: windows_of(shell_pid), READY_TIMEOUT_SECONDS, 0.02)
        launch_ms = (time.monotonic() - started) * 1000
        ready = log.wait_for(r"^ready at (\d+) ms", READY_TIMEOUT_SECONDS)
        if not bar or ready is None:
            return failures + [f"the Lulo layer did not come up: {log.text()[-3000:]}"]
        print(f"shell: launch ~{launch_ms:.0f} ms (session start to the first shell window)")
        print(f"shell: ready (bar and Dock placed) {ready.group(1)} ms after lulo-shell started")
        placed = log.wait_for(r"^bar at (-?\d+),(-?\d+),(-?\d+),(-?\d+) dock strip at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
        time.sleep(SETTLE_SECONDS)
        save(screenshots, "desktop")

        # 1. The work area.
        area = work_area()
        visible, state = taskbar_state()
        print(f"shell: while Lulo runs: work area {area}, taskbar visible {visible}, state {state}")
        bar_bottom = None
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

        # 1b. The desktop's icons clear of the bar.
        if list_view and bar_bottom is not None:
            if auto_arranged(list_view):
                print("shell: desktop icons are auto-arranged; Explorer flows them into the work area")
            else:
                moved = wait_until(lambda: window_rect(list_view)[1] >= bar_bottom, 8.0)
                print(f"shell: desktop icons while Lulo runs: list view at {window_rect(list_view)}")
                if not moved:
                    failures.append(
                        f"the desktop's icons start at y={window_rect(list_view)[1]}, under the bar (y={bar_bottom})"
                    )
        elif not list_view:
            print("shell: no desktop icon list view on this desktop; icon check skipped")

        # 1c. The frosted backdrop behind the bar and the Dock.
        frosted_expected = transparency_effects()
        for surface, name in (("Bar", "BarWindow"), ("Dock", "DockWindow")):
            traced = log.wait_for(rf"^backdrop {surface}: (acrylic|tint only)", 10.0)
            hwnd = shell_window(name)
            accent = accent_state(hwnd) if hwnd else None
            print(
                f"shell: {surface} backdrop: {traced.group(1) if traced else 'not reported'}, "
                f"accent state {accent} (transparency effects {'on' if frosted_expected else 'off'})"
            )
            if traced is None:
                failures.append(f"the {surface} reported no backdrop")
            elif frosted_expected and traced.group(1) != "acrylic":
                failures.append(f"the {surface} has no acrylic backdrop although transparency effects are on")
            if frosted_expected and accent is not None and accent != ACCENT_ENABLE_ACRYLICBLURBEHIND:
                failures.append(f"Windows reports accent {accent} behind the {surface}, not acrylic")

        # 1d. The Spotlight hotkey (free here) and the Dock's tiles.
        hotkey = log.wait_for(r"^spotlight hotkey (.+) \((first choice|fallback)\)", 5.0)
        print(f"shell: Spotlight hotkey: {hotkey.group(0) if hotkey else 'not reported'}")
        if hotkey is None or hotkey.group(1) != "Alt+Space":
            failures.append("Alt+Space was free but is not Spotlight's hotkey")
        if log.wait_for(r"^notice shown", 0.5) is not None:
            failures.append("a hotkey notice showed although Alt+Space was free")
        bin_tile = log.last(r"^dock tile recycle-bin Recycle Bin at (\d+),(\d+) (full|empty)")
        files_tile = log.last(r"^dock tile rmac-files\.exe Files at (\d+),(\d+).* icon (\S+)")
        print(
            f"shell: Dock: Recycle Bin tile {bin_tile.group(0) if bin_tile else 'missing'}; "
            f"Files tile {files_tile.group(0) if files_tile else 'missing'}"
        )
        if bin_tile is None:
            failures.append("the Dock has no Recycle Bin tile")
        if files_tile is None or files_tile.group(3) != "apps/org.rmac.Files.svg":
            failures.append("the Dock's Files tile does not show Files' own icon")

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

        # 2b. Memory at idle (gated by idle_gate.py --shell-memory-mb).
        for line in log.lines():
            if line.startswith("memory "):
                print(f"shell: lulo-shell {line}")
        for name, pid in (("lulo-shell", shell_pid), ("lulo-session", session.pid)):
            memory = memory_mb(pid)
            print(f"shell: {name}: memory at idle {memory}")
            if memory is not None:
                measurements[name]["idle_working_set_mb"] = memory["working_set_mb"]
                measurements[name]["idle_private_mb"] = memory["private_mb"]
                measurements[name]["peak_working_set_mb"] = memory["peak_working_set_mb"]

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

        # 4b. Files under another executable name is still Files in the
        # Dock (known by the app id it gives the menu bar), with Files' icon.
        files_exe = bin_dir / "rmac-files.exe"
        if files_exe.exists():
            import shutil

            renamed = profile / "files-dev.exe"
            shutil.copyfile(files_exe, renamed)
            before = len(log.lines())
            renamed_process = subprocess.Popen([str(renamed)], env=environment)
            try:
                hello = log.wait_for(r"^menus from org\.rmac\.Files", after=before)
                tile = log.wait_for(
                    r"^dock tile rmac-files\.exe Files at \d+,\d+ running icon apps/org\.rmac\.Files\.svg",
                    after=before,
                )
                stray = log.wait_for(r"^dock tile files-dev\.exe", 0.5, after=before)
                time.sleep(1.0)
                save(screenshots, "dock-files-icon")
                print(
                    f"shell: Files as files-dev.exe: menus {'yes' if hello else 'no'}, "
                    f"Dock {tile.group(0) if tile else 'no running Files tile'}"
                )
                if hello is None or tile is None or stray is not None:
                    failures.append("Files run as files-dev.exe does not show as Files, with Files' icon, in the Dock")
            finally:
                renamed_process.kill()
                renamed_process.wait(timeout=10)
            time.sleep(1.0)

        # 4c. The Recycle Bin tile follows the bin and has its menu.
        if bin_tile is not None:
            before = len(log.lines())
            initial = log.wait_for(r"^recycle bin (full|empty)", 5.0)
            junk = profile / "lulo-recycle-check.txt"
            junk.write_text("Lulo's Recycle Bin check\n", encoding="utf-8")
            recycled = recycle(junk)
            full = log.wait_for(r"^recycle bin full", 10.0, after=0 if initial is None else before)
            if initial is not None and initial.group(1) == "full":
                full = initial
            print(
                f"shell: the Dock's bin started {initial.group(1) if initial else 'unreported'}; "
                f"recycled a file: {recycled}; the bin {'shows full' if full else 'did not turn full'}"
            )
            if not recycled or full is None:
                failures.append("the Dock's Recycle Bin did not show full after a file was recycled")
            right = log.last(r"^dock tile recycle-bin Recycle Bin at (\d+),(\d+)")
            if right is not None:
                user32().SetCursorPos(int(right.group(1)), int(right.group(2)))
                time.sleep(0.15)
                user32().mouse_event(0x0008, 0, 0, 0, 0)
                time.sleep(0.05)
                user32().mouse_event(0x0010, 0, 0, 0, 0)
                menu = log.wait_for(r"^menu dock open", 5.0, after=before)
                time.sleep(0.8)
                save(screenshots, "dock-recycle-bin-menu")
                if menu is None:
                    failures.append("right-clicking the Recycle Bin tile opened no menu")
                tap(VK_ESCAPE)
                log.wait_for(r"^menu closed", 5.0, after=before)

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

        # 6b. A closed Spotlight lets go of its window and file list.
        released = log.wait_for(r"^spotlight released", 15.0, after=before)
        time.sleep(2.0)
        memory = memory_mb(shell_pid)
        print(
            f"shell: lulo-shell memory after Spotlight closed: {memory} "
            f"({'released' if released else 'not released'})"
        )
        if memory is not None:
            measurements["lulo-shell"]["after_spotlight_working_set_mb"] = memory["working_set_mb"]
            measurements["lulo-shell"]["after_spotlight_private_mb"] = memory["private_mb"]
        if released is None:
            failures.append("Spotlight kept its window after it closed")

        # 7. Turning Lulo off gives the desktop back.
        stop_session(bin_dir, environment, session, failures)
        time.sleep(SETTLE_SECONDS)
        save(screenshots, "restored")
        restored_area = work_area()
        restored_taskbar = taskbar_state()
        print(f"shell: after Lulo: work area {restored_area}, taskbar (visible, state) {restored_taskbar}")
        if restored_area != baseline_area:
            failures.append(f"the work area is {restored_area} after Lulo, not {baseline_area} as before")
        if restored_taskbar != baseline_taskbar:
            failures.append(f"the taskbar is {restored_taskbar} after Lulo, not {baseline_taskbar} as before")
        if list_view and baseline_icons is not None:
            restored = wait_until(lambda: window_rect(list_view) == baseline_icons, 5.0)
            print(f"shell: desktop icons after Lulo: list view at {window_rect(list_view)}")
            if not restored:
                failures.append(
                    f"the desktop's icons are at {window_rect(list_view)} after Lulo, not {baseline_icons} as before"
                )
        if processes_named("lulo-shell.exe"):
            failures.append("lulo-shell kept running after --stop")

        # 8. Alt+Space held by another app: a fallback, a one-time notice,
        # the menu bar's icon, and the other app keeps its hotkey.
        for exe in opened:
            for pid in processes_named(exe):
                terminate(pid)
        opened.clear()
        holder = HotkeyHolder()
        if not holder.registered():
            failures.append("the test's own Alt+Space holder could not register it")
        else:
            failures.extend(check_hotkey_fallback(bin_dir, environment, log, screenshots, holder))
    finally:
        if holder is not None:
            holder.stop()
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
        for line in trace[:300]:
            print(f"shell log: {line}")
    return failures


def check_hotkey_fallback(
    bin_dir: Path, environment: dict, log: Log, screenshots: Path | None, holder: HotkeyHolder
) -> list[str]:
    failures: list[str] = []
    before = len(log.lines())
    session = start_session(bin_dir, environment, log.path)
    try:
        starting = log.wait_for(r"^starting pid (\d+)", READY_TIMEOUT_SECONDS, after=before)
        ready = log.wait_for(r"^ready at (\d+) ms", READY_TIMEOUT_SECONDS, after=before)
        if starting is None or ready is None:
            return [f"the Lulo layer did not come up again: {log.text()[-2000:]}"]
        shell_pid = int(starting.group(1))
        hotkey = log.wait_for(r"^spotlight hotkey (.+) \((first choice|fallback)\)", 10.0, after=before)
        taken = log.wait_for(r"^keyboard layouts (\d+); hotkeys taken by other apps: (.*)$", 1.0, after=before)
        print(
            f"shell: with Alt+Space taken: {hotkey.group(0) if hotkey else 'no hotkey reported'}; "
            f"{taken.group(0) if taken else ''}"
        )
        if hotkey is None or hotkey.group(2) != "fallback":
            return failures + ["Lulo did not fall back from a taken Alt+Space"]
        label = hotkey.group(1)
        notice = log.wait_for(r"^notice shown: (.+)$", 10.0, after=before)
        time.sleep(1.0)
        save(screenshots, "hotkey-notice")
        print(f"shell: notice: {notice.group(1) if notice else 'none'}")
        if notice is None or label not in notice.group(1):
            failures.append(f"no one-time notice named the hotkey in effect ({label})")

        # The fallback opens Spotlight, and Win+Space does not open Start.
        keys = {
            "Win+Space": [VK_LWIN],
            "Ctrl+Alt+Space": [VK_CONTROL, VK_MENU],
            "Alt+Shift+Space": [VK_MENU, VK_SHIFT],
        }.get(label)
        if keys is None:
            failures.append(f"unknown fallback hotkey {label}")
        else:
            mark = len(log.lines())
            for code in keys:
                key(code)
            tap(VK_SPACE)
            for code in reversed(keys):
                key(code, up=True)
            shown = log.wait_for(r"^spotlight shown", 10.0, after=mark)
            time.sleep(0.8)
            front_pid, front_class = foreground_pid_and_class()
            save(screenshots, "spotlight-fallback")
            print(f"shell: {label} -> Spotlight {'shown' if shown else 'not shown'}; in front: pid {front_pid} {front_class}")
            if shown is None:
                failures.append(f"{label} did not open Spotlight")
            elif front_pid != shell_pid:
                failures.append(f"Spotlight opened on {label} but {front_class} (pid {front_pid}) has the keyboard")
            tap(VK_ESCAPE)
            log.wait_for(r"^spotlight hidden", 5.0, after=mark)
            time.sleep(0.5)

        # The menu bar's magnifier always opens Spotlight.
        icon = log.last(r"^bar spotlight at (-?\d+),(-?\d+),(\d+),(\d+)")
        if icon is None:
            failures.append("the bar did not report its Spotlight icon")
        else:
            mark = len(log.lines())
            click(int(icon.group(1)) + int(icon.group(3)) // 2, int(icon.group(2)) + int(icon.group(4)) // 2)
            shown = log.wait_for(r"^spotlight shown", 10.0, after=mark)
            time.sleep(0.5)
            save(screenshots, "spotlight-icon")
            print(f"shell: the bar's Spotlight icon {'opened' if shown else 'did not open'} Spotlight")
            if shown is None:
                failures.append("clicking the bar's Spotlight icon did not open Spotlight")
            tap(VK_ESCAPE)
            log.wait_for(r"^spotlight hidden", 5.0, after=mark)

        # The other app still has Alt+Space: Lulo never took it.
        presses = holder.presses()
        mark = len(log.lines())
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        got = wait_until(lambda: holder.presses() > presses, 5.0)
        stolen = log.wait_for(r"^spotlight shown", 1.0, after=mark)
        print(f"shell: Alt+Space reached {'the app that holds it' if got else 'nothing'}")
        if not got or stolen is not None:
            failures.append("Alt+Space did not reach the app that registered it first")

        # A tap of Win alone still opens Start (reported: the runner's
        # Start may not show).
        if label == "Win+Space":
            tap(VK_LWIN)
            time.sleep(1.5)
            _, front_class = foreground_pid_and_class()
            print(f"shell: a tap of Win alone: {front_class} in front")
            save(screenshots, "win-tap")
            tap(VK_ESCAPE)
            time.sleep(0.5)

        # A second start says nothing more: the notice was one-time.
        stop_session(bin_dir, environment, session, failures)
        mark = len(log.lines())
        session = start_session(bin_dir, environment, log.path)
        if log.wait_for(r"^ready at", READY_TIMEOUT_SECONDS, after=mark) is None:
            failures.append("the Lulo layer did not come up a third time")
        elif log.wait_for(r"^notice shown", 3.0, after=mark) is not None:
            failures.append("the hotkey notice showed again on the next start")
        else:
            print("shell: the hotkey notice did not show again")
    finally:
        stop_session(bin_dir, environment, session, failures)
    return failures
