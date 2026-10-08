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

and, for Lulo mode (ADR 0023 "Lulo mode", WIN-OS-41 on):

- Lulo's desktop window covers the screen just above Explorer's desktop
  and below app windows (also after a click on it), Explorer's desktop
  icons are hidden while Lulo runs, and the user's Desktop folder shows as
  Lulo icons that open (a folder in Files), rename, drag, show a context
  menu and follow new files;
- a Lulo app window has a shadow (pixels just outside its edge darken) and
  DWM non-client rendering;
- Spotlight opens as the search bar alone, in the upper third, and grows
  with results;
- an app opened from the Dock or Spotlight comes to the front over a
  maximised File Explorer;
- lulo-shell's private memory 30 s after Spotlight and a menu closed
  (`idle_gate.py --shell-after-use-*` gates it);
- every way out gives the desktop back: Turn Off, sign-out
  (`WM_QUERYENDSESSION`/`WM_ENDSESSION`), a crash of the shell, a crash of
  the whole layer (restored at the next start and by
  `--restore-windows-desktop`) and `lulo-session --uninstall`, which also
  turns off Use Files for Folders;

and, for the fixes after the first real-PC test (ADR 0023, WIN-OS-32 on):

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


GWL_EXSTYLE = -20
WS_EX_TOPMOST = 0x00000008
SW_MINIMIZE = 6
SW_RESTORE = 9
WM_QUERYENDSESSION = 0x0011
WM_ENDSESSION = 0x0016
ENDSESSION_LOGOFF = 0x80000000
SMTO_ABORTIFHUNG = 0x0002
DWMWA_NCRENDERING_ENABLED = 1
SPI_GETDROPSHADOW = 0x1024
SPI_SETDROPSHADOW = 0x1025
#: Spotlight's panel as the bar alone (pt) and the clear margin round it.
SPOTLIGHT_FIELD = 56
SPOTLIGHT_MARGIN = 24


def z_order() -> list[int]:
    """Every top-level window, from the top of the z-order down."""
    found: list[int] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(hwnd, _lparam):
        found.append(hwnd)
        return True

    user32().EnumWindows(collect, 0)
    return found


def class_of(hwnd: int) -> str:
    buffer = ctypes.create_unicode_buffer(256)
    user32().GetClassNameW(wintypes.HWND(hwnd), buffer, 256)
    return buffer.value


def pid_of(hwnd: int) -> int:
    owner = wintypes.DWORD()
    user32().GetWindowThreadProcessId(wintypes.HWND(hwnd), ctypes.byref(owner))
    return owner.value


def screen_size() -> tuple[int, int]:
    return user32().GetSystemMetrics(0), user32().GetSystemMetrics(1)


def window_scale(hwnd: int) -> float:
    try:
        dpi = user32().GetDpiForWindow(wintypes.HWND(hwnd))
    except AttributeError:
        dpi = 96
    return (dpi or 96) / 96.0


def topmost(hwnd: int) -> bool:
    return bool(user32().GetWindowLongW(wintypes.HWND(hwnd), GWL_EXSTYLE) & WS_EX_TOPMOST)


def lulo_record(name: str):
    """A value Lulo keeps under `HKCU\\Software\\Lulo\\Shell`, or `None`."""
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Lulo\Shell") as key:
            return winreg.QueryValueEx(key, name)[0]
    except OSError:
        return None


def folder_verb() -> str | None:
    """The default verb for folders in the user's own classes."""
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Classes\Directory\shell") as key:
            return winreg.QueryValueEx(key, "")[0]
    except OSError:
        return None


def explorer_icons_visible() -> bool | None:
    list_view = desktop_list_view()
    if not list_view:
        return None
    return bool(user32().IsWindowVisible(wintypes.HWND(list_view)))


def desktop_state() -> dict:
    """What turning Lulo off must give back exactly."""
    return {
        "work area": work_area(),
        "taskbar": taskbar_state(),
        "explorer icons visible": explorer_icons_visible(),
        "lulo records": sorted(
            name
            for name in ("TaskbarState", "DesktopIconsHidden", "DesktopIconsOffset", "BarWindow", "DockWindow")
            if lulo_record(name) is not None
        ),
    }


def state_differences(before: dict, after: dict, keys=None) -> list[str]:
    return [
        f"{key}: {after[key]!r}, not {before[key]!r} as before Lulo"
        for key in (keys or before.keys())
        if before[key] != after[key]
    ]


def pixel(x: int, y: int) -> tuple[int, int, int]:
    gdi32 = ctypes.windll.gdi32
    gdi32.GetPixel.restype = wintypes.DWORD
    dc = user32().GetDC(None)
    try:
        value = gdi32.GetPixel(dc, x, y)
    finally:
        user32().ReleaseDC(None, dc)
    return value & 0xFF, (value >> 8) & 0xFF, (value >> 16) & 0xFF


def luminance(colour: tuple[int, int, int]) -> float:
    red, green, blue = colour
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue


def drop_shadows_on() -> bool:
    value = wintypes.BOOL()
    user32().SystemParametersInfoW(SPI_GETDROPSHADOW, 0, ctypes.byref(value), 0)
    return bool(value.value)


def nc_rendering(hwnd: int) -> bool | None:
    value = wintypes.BOOL()
    result = ctypes.windll.dwmapi.DwmGetWindowAttribute(
        wintypes.HWND(hwnd), DWMWA_NCRENDERING_ENABLED, ctypes.byref(value), ctypes.sizeof(value)
    )
    return bool(value.value) if result == 0 else None


def send_message(hwnd: int, message: int, wparam: int, lparam: int) -> None:
    result = ctypes.c_size_t()
    user32().SendMessageTimeoutW(
        wintypes.HWND(hwnd),
        message,
        wintypes.WPARAM(wparam),
        wintypes.LPARAM(lparam),
        SMTO_ABORTIFHUNG,
        5000,
        ctypes.byref(result),
    )


def double_click(x: int, y: int) -> None:
    click(x, y)
    time.sleep(0.05)
    user32().mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.03)
    user32().mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.05)


def right_click(x: int, y: int) -> None:
    user32().SetCursorPos(x, y)
    time.sleep(0.15)
    user32().mouse_event(0x0008, 0, 0, 0, 0)
    time.sleep(0.05)
    user32().mouse_event(0x0010, 0, 0, 0, 0)


def drag(start: tuple[int, int], end: tuple[int, int]) -> None:
    user32().SetCursorPos(*start)
    time.sleep(0.15)
    user32().mouse_event(0x0002, 0, 0, 0, 0)
    steps = 12
    for step in range(1, steps + 1):
        x = start[0] + (end[0] - start[0]) * step // steps
        y = start[1] + (end[1] - start[1]) * step // steps
        user32().SetCursorPos(x, y)
        time.sleep(0.03)
    time.sleep(0.1)
    user32().mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.2)


def bring_to_front(hwnd: int) -> bool:
    """Put a window in front from the test, as a user's click would (the
    test's own key press lets it take the foreground)."""
    key(VK_MENU)
    user32().SetForegroundWindow(wintypes.HWND(hwnd))
    key(VK_MENU, up=True)
    return wait_until(lambda: user32().GetForegroundWindow() == hwnd, 5.0) is not None


def explorer_window(known: set[int]) -> int:
    for hwnd in z_order():
        if hwnd not in known and class_of(hwnd) == "CabinetWClass" and user32().IsWindowVisible(hwnd):
            return hwnd
    return 0


def user_desktop() -> Path:
    import ctypes.wintypes as w

    buffer = ctypes.create_unicode_buffer(w.MAX_PATH)
    # CSIDL_DESKTOPDIRECTORY
    ctypes.windll.shell32.SHGetFolderPathW(None, 0x10, None, 0, buffer)
    return Path(buffer.value)


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
    software_renderer_ticks=None,
) -> list[str]:
    """Run every check; return the failures."""
    failures: list[str] = []
    check_resources(bin_dir, failures)
    baseline_area = work_area()
    baseline_taskbar = taskbar_state()
    list_view = desktop_list_view()
    baseline_icons = window_rect(list_view) if list_view else None
    baseline_state = desktop_state()
    # Lulo mode's desktop shows the user's Desktop folder: a folder and a
    # note to open, rename and drag.
    desk = user_desktop()
    desk.mkdir(parents=True, exist_ok=True)
    check_folder = desk / "Lulo Check Folder"
    check_folder.mkdir(exist_ok=True)
    check_note = desk / "Lulo Check Note.txt"
    check_note.write_text("Lulo mode's desktop check\n", encoding="utf-8")
    # Windows Server starts with window shadows off ("adjust for best
    # performance"); a PC has them on. Turn them on for the shadow check.
    if not drop_shadows_on():
        user32().SystemParametersInfoW(SPI_SETDROPSHADOW, 0, ctypes.c_void_p(1), 0)
        print(f"shell: window shadows were off on this runner; turned on: {drop_shadows_on()}")
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

        # 1b. Lulo mode's desktop: Lulo's own desktop window just above
        # Explorer's, Explorer's icons hidden, the Desktop folder as Lulo icons.
        desktop_hwnd = check_lulo_desktop(log, shell_pid, failures)
        if list_view:
            hidden = wait_until(lambda: not user32().IsWindowVisible(wintypes.HWND(list_view)), 8.0)
            print(
                f"shell: Explorer's desktop icons while Lulo runs: "
                f"{'hidden' if hidden else 'visible'}, record {lulo_record('DesktopIconsHidden')}"
            )
            if not hidden:
                failures.append("Explorer's desktop icons are still showing under Lulo's desktop")
        else:
            print("shell: no desktop icon list view on this desktop; Explorer icon check skipped")

        # 1c. The frosted backdrop behind the bar, and none behind the Dock.
        frosted_expected = transparency_effects()
        # The bar shows the wallpaper's colour through a plain blur. In Lulo
        # mode the Dock draws the wallpaper under its shelf itself and has
        # no accent: acrylic ignored its rounded window region on a real
        # Windows 11 PC and showed a dark box behind both ends (WIN-OS-49).
        for surface, name, kind, state in (
            ("Bar", "BarWindow", "blur", 3),
            ("Dock", "DockWindow", "none (Lulo mode)", None),
        ):
            traced = log.wait_for(
                rf"^backdrop {surface}: (blur|acrylic|tint only|none \(Lulo mode\))", 10.0
            )
            hwnd = shell_window(name)
            accent = accent_state(hwnd) if hwnd else None
            print(
                f"shell: {surface} backdrop: {traced.group(1) if traced else 'not reported'}, "
                f"accent state {accent} (transparency effects {'on' if frosted_expected else 'off'})"
            )
            if traced is None:
                failures.append(f"the {surface} reported no backdrop")
            elif (frosted_expected or state is None) and traced.group(1) != kind:
                failures.append(f"the {surface} backdrop is {traced.group(1)}, not {kind}")
            if state is not None and frosted_expected and accent is not None and accent != state:
                failures.append(f"Windows reports accent {accent} behind the {surface}, not {kind}")
            if state is None and accent in (3, ACCENT_ENABLE_ACRYLICBLURBEHIND):
                failures.append(f"Windows still blurs behind the {surface} (accent {accent})")
        text = log.wait_for(r"^wallpaper (\d+)x(\d+) for (\d+)x(\d+), bar luminance ([\d.]+)", 15.0)
        print(f"shell: Lulo's wallpaper: {text.group(0) if text else 'not reported'}")
        if text is None:
            failures.append("the desktop reported no wallpaper")
        else:
            check_dock_corners(failures)

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
                check_window_shadow(calculator[1], "Calculator", screenshots, failures)

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

        # 5a. The desktop's icons: open, rename, drag, menu, new files, and
        # the desktop staying under app windows.
        if desktop_hwnd:
            check_desktop_icons(log, desktop_hwnd, check_folder, check_note, screenshots, opened, failures)

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
            spotlight = user32().GetForegroundWindow()
            empty = window_rect(spotlight)
            save(screenshots, "spotlight-empty")
            check_spotlight_empty(spotlight, empty, shell_pid, failures)
            type_text("text editor")
            results = log.wait_for(r'^spotlight "text editor": (\d+) results', after=before, timeout=10.0)
            time.sleep(0.8)
            grown = window_rect(spotlight)
            print(f"shell: Spotlight with results: window {grown} (empty {empty})")
            if grown[3] - grown[1] <= empty[3] - empty[1]:
                failures.append(f"Spotlight did not grow for its results ({empty} -> {grown})")
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
            elif log.wait_for(r"^spotlight hidden", 0.5, after=before) is None:
                # Nothing to open (the runner's Apps folder may have no
                # Notepad): close Spotlight as the user would.
                tap(VK_ESCAPE)
        if log.wait_for(r"^spotlight hidden", 5.0, after=before) is None:
            failures.append("Spotlight did not close")
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

        # 6c. An app opened from the Dock or Spotlight comes to the front
        # over a maximised File Explorer.
        for exe in ("rmac-text-editor.exe", "notepad.exe"):
            for pid in processes_named(exe):
                terminate(pid)
        time.sleep(1.0)
        check_foreground(log, profile, screenshots, opened, failures)

        # 6d. Memory 30 s after one use of Spotlight and a menu.
        check_memory_after_use(log, shell_pid, measurements, failures)

        # 6e. Every Lulo app opens inside the work area the bar and the
        # Dock leave (WIN-OS-50).
        check_app_placement(bin_dir, environment, screenshots, failures)

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
        # The icon helper (also lulo-shell.exe) ends when the shell's pipe
        # to it closes, a moment after the shell itself.
        if not wait_until(lambda: not processes_named("lulo-shell.exe"), 5.0):
            failures.append("lulo-shell kept running after --stop")
        after_stop = wait_until(lambda: not state_differences(baseline_state, desktop_state()), 5.0)
        if not after_stop:
            failures.extend(
                f"after Turn Off: {difference}" for difference in state_differences(baseline_state, desktop_state())
            )
        else:
            print("shell: Turn Off gave back the work area, the taskbar, Explorer's icons and every record")

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
        holder.stop()
        holder = None

        # 9. Every other way out gives the desktop back.
        failures.extend(check_exit_paths(bin_dir, environment, log, screenshots, baseline_state))
    finally:
        for path in (check_note, check_folder):
            try:
                if path.is_dir():
                    import shutil

                    shutil.rmtree(path, ignore_errors=True)
                elif path.exists():
                    path.unlink()
            except OSError:
                pass
        for leftover in desk.glob("Lulo Check*"):
            try:
                if leftover.is_dir():
                    import shutil

                    shutil.rmtree(leftover, ignore_errors=True)
                else:
                    leftover.unlink()
            except OSError:
                pass
        for leftover in desk.glob("renamed*"):
            try:
                leftover.unlink()
            except OSError:
                pass
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


def check_lulo_desktop(log: Log, shell_pid: int, failures: list[str]) -> int:
    """Lulo's desktop window: visible, the whole screen, not topmost, just
    above Explorer's desktop. Returns its handle (0 when missing)."""
    traced = log.wait_for(r"^desktop window (\d+)", 15.0)
    if traced is None:
        failures.append("Lulo mode opened no desktop window")
        return 0
    hwnd = int(traced.group(1))
    placed = wait_until(lambda: user32().IsWindowVisible(wintypes.HWND(hwnd)), 10.0)
    rect = window_rect(hwnd)
    width, height = screen_size()
    order = z_order()
    progman = user32().FindWindowW("Progman", None) or 0
    above_explorer = hwnd in order and progman in order and order.index(hwnd) < order.index(progman)
    print(
        f"shell: Lulo's desktop window {hwnd}: visible {bool(placed)}, at {rect}, topmost {topmost(hwnd)}, "
        f"z-order {order.index(hwnd) if hwnd in order else '?'} of {len(order)}, "
        f"Progman at {order.index(progman) if progman in order else '?'}"
    )
    if pid_of(hwnd) != shell_pid:
        failures.append("the desktop window is not lulo-shell's")
    if not placed:
        failures.append("Lulo's desktop window is not showing")
    if rect != (0, 0, width, height):
        failures.append(f"Lulo's desktop window is at {rect}, not over the whole {width}x{height} screen")
    if topmost(hwnd):
        failures.append("Lulo's desktop window is topmost: it would cover app windows")
    if progman and not above_explorer:
        failures.append("Lulo's desktop window is not above Explorer's desktop")
    note = log.wait_for(r"^desktop icon Lulo Check Note\.txt at (\d+),(\d+)", 15.0)
    folder = log.wait_for(r"^desktop icon Lulo Check Folder at (\d+),(\d+)", 5.0)
    print(
        f"shell: the Desktop folder on Lulo's desktop: note {note.group(0) if note else 'missing'}, "
        f"folder {folder.group(0) if folder else 'missing'}"
    )
    if note is None or folder is None:
        failures.append("Lulo's desktop does not show the Desktop folder's items")
    return hwnd


def check_dock_corners(failures: list[str]) -> None:
    """Outside its rounded shelf the Dock's window must be clear: the
    pixel in each corner of the window matches the wallpaper just beside
    the window (WIN-OS-49: a dark box showed there on a real PC)."""
    hwnd = shell_window("DockWindow")
    if not hwnd:
        failures.append("no Dock window recorded for the corner check")
        return
    time.sleep(1.0)
    left, top, right, bottom = window_rect(hwnd)
    corners = {
        "top left": ((left + 1, top + 1), (left - 3, top + 1)),
        "top right": ((right - 2, top + 1), (right + 2, top + 1)),
        "bottom left": ((left + 1, bottom - 2), (left - 3, bottom - 2)),
        "bottom right": ((right - 2, bottom - 2), (right + 2, bottom - 2)),
    }
    for name, (inside, outside) in corners.items():
        corner, beside = pixel(*inside), pixel(*outside)
        difference = max(abs(a - b) for a, b in zip(corner, beside))
        print(f"shell: Dock {name} corner {corner}, wallpaper beside it {beside}")
        if difference > 28:
            failures.append(
                f"the Dock's {name} corner is {corner}, not the wallpaper beside it {beside}"
            )


def check_window_shadow(hwnd: int, name: str, screenshots: Path | None, failures: list[str]) -> None:
    """A Lulo app window has the Mac's soft shadow: just outside its bottom
    edge the screen is darker with the window there than without it."""
    rendering = nc_rendering(hwnd)
    left, top, right, bottom = frame_bounds(hwnd)
    width, height = screen_size()
    x = (left + right) // 2
    points = [(x, bottom + offset) for offset in (3, 6, 10) if bottom + offset < height]
    points += [(right + offset, (top + bottom) // 2) for offset in (3, 6, 10) if right + offset < width]
    if not points:
        print(f"shell: {name}'s shadow not checked (its edges are off screen)")
        return
    with_window = [luminance(pixel(*point)) for point in points]
    user32().ShowWindow(wintypes.HWND(hwnd), SW_MINIMIZE)
    time.sleep(1.2)
    without = [luminance(pixel(*point)) for point in points]
    user32().ShowWindow(wintypes.HWND(hwnd), SW_RESTORE)
    time.sleep(1.0)
    # Back in front, as before the check (minimising gave the foreground away).
    bring_to_front(hwnd)
    time.sleep(0.8)
    darker = [round(behind - shaded, 1) for shaded, behind in zip(with_window, without)]
    print(
        f"shell: {name}'s frame {left},{top},{right},{bottom}; DWM non-client rendering {rendering}; "
        f"just outside its edges the window darkens the screen by {darker} (luminance)"
    )
    save(screenshots, "window-shadow")
    if rendering is False:
        failures.append(f"{name}'s window has DWM non-client rendering off: no shadow")
    # Windows Server's DWM shadow is fainter than a PC's; any clear
    # darkening just outside the edge is the shadow.
    if max(darker) < 1.5:
        failures.append(f"{name}'s window casts no shadow (screen just outside it darkened by {darker})")


def check_spotlight_empty(spotlight: int, rect, shell_pid: int, failures: list[str]) -> None:
    """Spotlight opens as the search bar alone, centred, in the upper third."""
    scale = window_scale(spotlight)
    width, height = screen_size()
    expected = round((SPOTLIGHT_FIELD + 2 * SPOTLIGHT_MARGIN) * scale)
    panel_top = rect[1] + round(SPOTLIGHT_MARGIN * scale)
    centre = (rect[0] + rect[2]) // 2
    print(
        f"shell: Spotlight before typing: window {rect} ({rect[3] - rect[1]} px high, the bar alone is "
        f"{expected}), panel top {panel_top} of {height}, centre {centre} of {width}"
    )
    if pid_of(spotlight) != shell_pid:
        failures.append("Spotlight's window was not in front when it opened")
        return
    if rect[3] - rect[1] > expected + 2:
        failures.append(f"Spotlight shows more than the search bar before typing ({rect[3] - rect[1]} px high)")
    if not panel_top < height / 3:
        failures.append(f"Spotlight's bar is at y={panel_top}, not in the upper third")
    if abs(centre - width // 2) > 2:
        failures.append(f"Spotlight is not centred (centre {centre} of {width})")


def minimize_others(shell_pid: int) -> None:
    """Minimise every app window but Lulo's own, so the desktop is in view
    (the runner's console window sits over it otherwise)."""
    for hwnd in z_order():
        if (
            user32().IsWindowVisible(wintypes.HWND(hwnd))
            and user32().GetWindowTextLengthW(wintypes.HWND(hwnd)) > 0
            and pid_of(hwnd) != shell_pid
            and class_of(hwnd) not in ("Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd")
            and not (user32().GetWindowLongW(wintypes.HWND(hwnd), GWL_EXSTYLE) & 0x80)  # tool windows
        ):
            user32().ShowWindow(wintypes.HWND(hwnd), SW_MINIMIZE)
    time.sleep(1.0)


def check_desktop_icons(
    log: Log,
    desktop_hwnd: int,
    folder: Path,
    note: Path,
    screenshots: Path | None,
    opened: list[str],
    failures: list[str],
) -> None:
    """Open, rename, drag, the context menu, new files, and the desktop
    staying under app windows."""
    # A click on the wallpaper puts the desktop in front: the bar shows its
    # Files menus, and the desktop stays under every app window.
    minimize_others(pid_of(desktop_hwnd))
    notepad = subprocess.Popen(["notepad.exe"])
    found = wait_until(lambda: app_window("notepad.exe"))
    width, height = screen_size()
    if found:
        # Top left, clear of the icons (top right) and the empty spot below.
        user32().MoveWindow(wintypes.HWND(found[1]), 40, 60, 420, 300, True)
        time.sleep(0.8)
    mark = len(log.lines())
    # Clear of Notepad, Calculator, the icons (top right) and the Dock.
    click(150, height - 120)
    active = log.wait_for(r"^desktop active", 5.0, after=mark)
    files = log.wait_for(r"^bar title 1 Files at", 5.0, after=mark)
    order = z_order()
    notepad_hwnd = found[1] if found else 0
    below = notepad_hwnd in order and desktop_hwnd in order and order.index(notepad_hwnd) < order.index(desktop_hwnd)
    print(
        f"shell: a click on the wallpaper: desktop {'active' if active else 'not active'}, bar "
        f"{'shows Files' if files else 'did not switch'}; Notepad still above the desktop: {below}"
    )
    save(screenshots, "desktop-clicked")
    if active is None:
        failures.append("a click on Lulo's desktop did not make it active")
    if files is None:
        failures.append("the bar did not show the desktop's Files menus")
    if found and not below:
        failures.append("a click on Lulo's desktop brought it over an app window")
    notepad.kill()
    for pid in processes_named("notepad.exe"):
        terminate(pid)
    time.sleep(0.8)

    # A file that appears is shown without a restart.
    mark = len(log.lines())
    later = note.parent / "Lulo Check Later.txt"
    later.write_text("added while Lulo runs\n", encoding="utf-8")
    added = log.wait_for(r"^desktop icon Lulo Check Later\.txt at (\d+),(\d+)", 10.0, after=mark)
    print(f"shell: a new file on the Desktop: {'shown' if added else 'not shown'}")
    if added is None:
        failures.append("a file added to the Desktop did not appear on Lulo's desktop")

    # Dragging an icon leaves it where it was dropped.
    if added is not None:
        start = (int(added.group(1)), int(added.group(2)))
        mark = len(log.lines())
        drag(start, (start[0] - 160, start[1] + 40))
        moved = log.wait_for(r"^desktop icon Lulo Check Later\.txt at (\d+),(\d+)", 5.0, after=mark)
        print(f"shell: dragged the new file from {start}: {moved.group(0) if moved else 'it did not move'}")
        if moved is None or abs(int(moved.group(1)) - (start[0] - 160)) > 12:
            failures.append("a dragged desktop icon did not stay where it was dropped")

    # The context menu on an icon.
    icon = log.last(r"^desktop icon Lulo Check Note\.txt at (\d+),(\d+)")
    if icon is not None:
        mark = len(log.lines())
        right_click(int(icon.group(1)), int(icon.group(2)))
        menu = log.wait_for(r"^menu dock open", 5.0, after=mark)
        time.sleep(0.6)
        save(screenshots, "desktop-icon-menu")
        print(f"shell: right-click on a desktop icon: {'menu' if menu else 'no menu'}")
        if menu is None:
            failures.append("right-clicking a desktop icon opened no menu")
        tap(VK_ESCAPE)
        log.wait_for(r"^menu closed", 5.0, after=mark)
        time.sleep(0.5)

    # Rename: select, Return, type, Return.
    icon = log.last(r"^desktop icon Lulo Check Note\.txt at (\d+),(\d+)")
    renamed = note.parent / "renamed.txt"
    if icon is not None:
        click(int(icon.group(1)), int(icon.group(2)))
        time.sleep(0.4)
        mark = len(log.lines())
        tap(VK_RETURN)
        editing = log.wait_for(r"^desktop rename", 5.0, after=mark)
        time.sleep(0.4)
        type_text("renamed")
        save(screenshots, "desktop-rename")
        tap(VK_RETURN)
        done = wait_until(renamed.exists, 8.0)
        print(f"shell: rename on the desktop: {'renamed.txt' if done else 'not renamed'} (editing {bool(editing)})")
        if not done:
            failures.append("renaming a desktop icon (Return, type, Return) did not rename the file")

    # A double-click on a folder opens it in Lulo's Files.
    icon = log.last(r"^desktop icon Lulo Check Folder at (\d+),(\d+)")
    if icon is not None:
        before = set(processes_named("rmac-files.exe"))
        double_click(int(icon.group(1)), int(icon.group(2)))
        files_window = wait_until(
            lambda: any(windows_of(pid, titled=True) for pid in processes_named("rmac-files.exe") if pid not in before),
            15.0,
        )
        time.sleep(1.0)
        save(screenshots, "desktop-folder-opened")
        launched = log.last(r"^launched rmac-files\.exe")
        print(f"shell: double-click on a desktop folder: {'Files opened' if files_window else 'nothing opened'}")
        if not files_window or launched is None:
            failures.append("double-clicking a desktop folder did not open it in Lulo's Files")
        for pid in processes_named("rmac-files.exe"):
            if pid not in before:
                terminate(pid)
        time.sleep(0.8)
    save(screenshots, "desktop-after")


def check_foreground(log: Log, profile: Path, screenshots: Path | None, opened: list[str], failures: list[str]) -> None:
    """Text Editor opened from the Dock and from Spotlight comes to the
    front over a maximised File Explorer (WIN-OS-45)."""
    known = {hwnd for hwnd in z_order() if class_of(hwnd) == "CabinetWClass"}
    subprocess.Popen(["explorer.exe", str(profile)])
    explorer = wait_until(lambda: explorer_window(known), 15.0)
    if not explorer:
        print("shell: File Explorer opened no window; the foreground check is skipped")
        return
    user32().ShowWindow(wintypes.HWND(explorer), SW_MAXIMIZE)
    time.sleep(1.0)

    def launch_and_check(how: str, launch) -> None:
        for pid in processes_named("rmac-text-editor.exe"):
            terminate(pid)
        time.sleep(0.8)
        if not bring_to_front(explorer):
            print(f"shell: {how}: File Explorer could not be put in front first")
        mark = len(log.lines())
        launch(mark)
        editor = wait_until(lambda: app_window("rmac-text-editor.exe"), 20.0)
        time.sleep(2.0)
        front = user32().GetForegroundWindow()
        order = z_order()
        front_pid = pid_of(front)
        editor_pid = editor[0] if editor else 0
        above = bool(editor) and editor[1] in order and explorer in order and order.index(editor[1]) < order.index(explorer)
        traced = log.wait_for(r"^launched rmac-text-editor\.exe in front: (.+)$", 3.0, after=mark)
        save(screenshots, f"foreground-{how.replace(' ', '-')}")
        print(
            f"shell: Text Editor from {how} over a maximised File Explorer: in front "
            f"{front_pid == editor_pid and editor_pid != 0} ({class_of(front)}), above Explorer {above}, "
            f"shell says {traced.group(1) if traced else 'nothing'}"
        )
        if editor is None:
            failures.append(f"Text Editor did not open from {how}")
        elif front_pid != editor_pid or not above:
            failures.append(f"Text Editor opened from {how} stayed behind File Explorer")

    def from_dock(_mark: int) -> None:
        tile = log.last(r"^dock tile rmac-text-editor\.exe Text Editor at (\d+),(\d+)")
        if tile is not None:
            click(int(tile.group(1)), int(tile.group(2)))

    def from_spotlight(mark: int) -> None:
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        if log.wait_for(r"^spotlight shown", 10.0, after=mark) is None:
            return
        time.sleep(0.6)
        type_text("text editor")
        log.wait_for(r'^spotlight "text editor": [1-9]', 10.0, after=mark)
        time.sleep(0.4)
        tap(VK_RETURN)

    launch_and_check("the Dock", from_dock)
    launch_and_check("Spotlight", from_spotlight)
    for pid in processes_named("rmac-text-editor.exe"):
        terminate(pid)
    user32().PostMessageW(wintypes.HWND(explorer), 0x0010, 0, 0)  # WM_CLOSE
    time.sleep(1.0)


PLACED_APPS = (
    "rmac-system-settings",
    "rmac-files",
    "rmac-text-editor",
    "rmac-notes",
    "rmac-calculator",
    "rmac-preview",
    "rmac-clock",
    "rmac-weather",
    "rmac-terminal",
)


def check_app_placement(bin_dir: Path, environment: dict, screenshots: Path | None, failures: list[str]) -> None:
    """Each Lulo app's first window lies inside the work area: below the
    bar and above the Dock (System Settings once opened with its bottom
    under the Dock on a 1366 × 768 PC)."""
    area = work_area()
    for app in PLACED_APPS:
        exe = bin_dir / f"{app}.exe"
        if not exe.is_file():
            continue
        for pid in processes_named(f"{app}.exe"):
            terminate(pid)
        process = subprocess.Popen([str(exe)], env=environment)
        found = wait_until(lambda: app_window(f"{app}.exe"), 20.0)
        time.sleep(1.5)
        if found is None:
            failures.append(f"{app} opened no window for the placement check")
        else:
            left, top, right, bottom = frame_bounds(found[1])
            inside = left >= area[0] and top >= area[1] and right <= area[2] and bottom <= area[3]
            print(
                f"shell: {app} window {left},{top},{right},{bottom} in work area "
                f"{area[0]},{area[1]},{area[2]},{area[3]}: {'inside' if inside else 'OUTSIDE'}"
            )
            if not inside:
                save(screenshots, f"placement-{app}")
                failures.append(f"{app} opened at {left},{top},{right},{bottom}, outside the work area {area}")
        for pid in processes_named(f"{app}.exe"):
            terminate(pid)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
        time.sleep(0.5)


def check_memory_after_use(log: Log, shell_pid: int, measurements: dict, failures: list[str]) -> None:
    """lulo-shell's memory 30 s after one use of Spotlight and a menu, once
    both have let go of their windows (`LULO_PANEL_RELEASE_SECONDS`)."""
    mark = len(log.lines())
    key(VK_MENU)
    tap(VK_SPACE)
    key(VK_MENU, up=True)
    if log.wait_for(r"^spotlight shown", 10.0, after=mark):
        time.sleep(0.6)
        type_text("notes")
        time.sleep(0.8)
        tap(VK_ESCAPE)
    title = log.last(r"^bar title 0 \S+ at (-?\d+),(-?\d+),(\d+),(\d+)")
    if title is not None:
        time.sleep(0.5)
        click(int(title.group(1)) + int(title.group(3)) // 2, int(title.group(2)) + int(title.group(4)) // 2)
        log.wait_for(r"^menu 0 open", 5.0, after=mark)
        time.sleep(0.6)
        tap(VK_ESCAPE)
    released = log.wait_for(r"^spotlight released", 20.0, after=mark)
    menu_released = log.wait_for(r"^menu panel released", 20.0, after=mark)
    time.sleep(30.0)
    memory = memory_mb(shell_pid)
    for line in log.lines()[mark:]:
        if line.startswith("memory "):
            print(f"shell: lulo-shell {line}")
    print(
        f"shell: lulo-shell 30 s after Spotlight and a menu closed: {memory} "
        f"(Spotlight {'released' if released else 'kept'}, menu {'released' if menu_released else 'kept'})"
    )
    if memory is not None:
        measurements["lulo-shell"]["after_use_working_set_mb"] = memory["working_set_mb"]
        measurements["lulo-shell"]["after_use_private_mb"] = memory["private_mb"]
    if released is None or menu_released is None:
        failures.append("Spotlight or the menu panel kept its window after closing")


def check_exit_paths(
    bin_dir: Path, environment: dict, log: Log, screenshots: Path | None, baseline: dict
) -> list[str]:
    """Sign-out, a crash of the shell, a crash of the whole layer and an
    uninstall each give the desktop back exactly as it was."""
    failures: list[str] = []

    def start() -> tuple[subprocess.Popen, int] | None:
        for attempt in range(2):
            mark = len(log.lines())
            text_mark = len(log.text())
            session = start_session(bin_dir, environment, log.path)
            starting = log.wait_for(r"^starting pid (\d+)", READY_TIMEOUT_SECONDS, after=mark)
            ready = log.wait_for(r"^ready at", READY_TIMEOUT_SECONDS, after=mark)
            if starting is not None and ready is not None:
                break
            # Say why, then try once more: a layer killed a moment ago may
            # still be going away.
            print(
                f"shell: exit-path start {attempt + 1} did not come up (session exit "
                f"{session.poll()}): {log.text()[text_mark:][-1500:]!r}"
            )
            session.kill()
            kill_all()
            time.sleep(3.0)
        else:
            failures.append("the Lulo layer did not come up for the exit-path checks")
            return None
        # Lulo mode is in force: the taskbar and Explorer's icons hidden.
        engaged = wait_until(
            lambda: lulo_record("DesktopIconsHidden") is not None or explorer_icons_visible() is None, 10.0
        )
        if not engaged:
            failures.append("Lulo mode did not hide Explorer's desktop icons")
        return session, int(starting.group(1))

    def expect_restored(path: str, keys=None, timeout: float = 10.0) -> None:
        settled = wait_until(lambda: not state_differences(baseline, desktop_state(), keys), timeout)
        differences = state_differences(baseline, desktop_state(), keys)
        print(f"shell: exit path {path}: {'restored' if settled else differences}")
        save(screenshots, f"exit-{path.replace(' ', '-')}")
        if differences:
            failures.extend(f"after {path}: {difference}" for difference in differences)

    def kill_all() -> None:
        for name in ("lulo-session.exe", "lulo-shell.exe"):
            for pid in processes_named(name):
                terminate(pid)
        wait_until(lambda: not processes_named("lulo-shell.exe"), 10.0)

    # Sign-out: Windows sends the session-end messages; the taskbar and
    # Explorer's icons come back before the session ends.
    started = start()
    if started is not None:
        session, _ = started
        bar = shell_window("BarWindow")
        send_message(bar, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF)
        send_message(bar, WM_ENDSESSION, 1, ENDSESSION_LOGOFF)
        expect_restored("sign-out", keys=["taskbar", "explorer icons visible"])
        stop_session(bin_dir, environment, session, failures)
        expect_restored("sign-out then Turn Off")

    # A crash of the shell alone: lulo-session restores and starts it again.
    started = start()
    if started is not None:
        session, shell_pid = started
        mark = len(log.lines())
        terminate(shell_pid)
        again = log.wait_for(r"^starting pid (\d+)", READY_TIMEOUT_SECONDS, after=mark)
        noticed = re.search(r"Lulo stopped", log.text()[-4000:])
        print(
            f"shell: exit path shell crash: lulo-session {'noticed' if noticed else 'did not notice'}, "
            f"{'started it again' if again else 'did not start it again'}"
        )
        if again is None:
            failures.append("lulo-session did not start the shell again after it crashed")
        log.wait_for(r"^ready at", READY_TIMEOUT_SECONDS, after=mark)
        stop_session(bin_dir, environment, session, failures)
        expect_restored("shell crash then Turn Off")

    # A crash of the whole layer: the desktop stays as Lulo left it until
    # the next start, which gives it back first, or the escape hatch does.
    started = start()
    if started is not None:
        kill_all()
        left = desktop_state()
        print(f"shell: exit path layer crash: left behind {left}")
        if not left["lulo records"]:
            failures.append("a crashed layer left no records to restore from")
        restarted = start()
        if restarted is not None:
            session, _ = restarted
            stop_session(bin_dir, environment, session, failures)
            expect_restored("layer crash then the next start")
    started = start()
    if started is not None:
        kill_all()
        subprocess.run([str(bin_dir / "lulo-session.exe"), "--restore-windows-desktop"], env=environment, timeout=60)
        expect_restored("layer crash then --restore-windows-desktop")

    # Uninstall (the installer runs `lulo-session --uninstall`): Lulo turns
    # off and Use Files for Folders, turned on from the Lulo menu, is undone.
    started = start()
    if started is not None:
        session, _ = started
        title = log.last(r"^bar title 0 \S+ at (-?\d+),(-?\d+),(\d+),(\d+)")
        if title is not None:
            time.sleep(2.0)
            opened_menu = None
            for _ in range(3):
                mark = len(log.lines())
                click(int(title.group(1)) + int(title.group(3)) // 2, int(title.group(2)) + int(title.group(4)) // 2)
                opened_menu = log.wait_for(r"^menu 0 open", 5.0, after=mark)
                if opened_menu:
                    break
                tap(VK_ESCAPE)
                time.sleep(1.0)
            if opened_menu:
                time.sleep(0.8)
                save(screenshots, "lulo-menu-files-for-folders")
                # The Lulo menu's eleventh row (the Mac's menu metrics:
                # 290 pt below the bar's bottom at its centre).
                scale = window_scale(shell_window("BarWindow"))
                bar_bottom = int(title.group(2)) + int(title.group(4)) + round(2 * scale)
                click(int(title.group(1)) + round(60 * scale), bar_bottom + round(288 * scale))
            on = log.wait_for(r"^files for folders on", 5.0, after=mark)
            print(f"shell: Use Files for Folders: {'on' if on else 'not turned on'}, folder verb {folder_verb()!r}")
            if on is None or folder_verb() != "LuloFiles":
                failures.append("the Lulo menu's Use Files for Folders did not make Files open folders")
        result = subprocess.run(
            [str(bin_dir / "lulo-session.exe"), "--uninstall"], env=environment, timeout=60
        )
        try:
            session.wait(timeout=STEP_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            failures.append("lulo-session kept running after --uninstall")
        # The icon helper (also lulo-shell.exe) ends when the shell's pipe
        # to it closes, a moment after the shell itself.
        if not wait_until(lambda: not processes_named("lulo-shell.exe"), 5.0):
            failures.append("lulo-shell kept running after --uninstall")
        print(f"shell: exit path uninstall: exit {result.returncode}, folder verb now {folder_verb()!r}")
        if folder_verb() == "LuloFiles" or lulo_record("FilesForFolders") is not None:
            failures.append("--uninstall left folders opening in Lulo's Files")
        expect_restored("uninstall")
    kill_all()
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
