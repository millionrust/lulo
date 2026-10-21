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
5. a maximised window stops below the bar and above the Dock, and a
   third-party window opened at 0,0 and 4000 × 4000 is moved into the work
   area while a full-screen one is left alone;
6. Alt+Space opens Spotlight, typing finds Text Editor and Return opens it
   (and, reported but not required, "notepad" finds Windows' Notepad
   through the Apps folder);
6b. Alt+Tab (⌘Tab) is Lulo's switcher: a quick tap switches without the
   panel, holding Alt shows it and the release switches, Esc cancels; a
   notification on Lulo's pipe shows a banner, its poster hears it close,
   and the bar's clock opens Notification Centre listing it;
7. `lulo-session --stop` turns the layer off, and the work area, the
   taskbar's visibility and its auto-hide setting are as before.

and, for Lulo mode (ADR 0023 "Lulo mode", WIN-OS-41 on):

- Lulo's desktop window covers the screen just above Explorer's desktop
  and below app windows (also after a click on it), Explorer's desktop
  icons are hidden while Lulo runs, and the user's Desktop folder shows as
  Lulo icons that open (a folder in Files), rename, drag, show a context
  menu and follow new files;
- a Lulo app window has a shadow (pixels just outside its edge darken);
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
        # The shell, its apps and helpers share the log, so another
        # process's write can land in front of a trace line on the same
        # physical line; the trace line still starts at its prefix.
        return [
            line[line.index(prefix) + len(prefix) :]
            for line in self.text().splitlines()
            if prefix in line
        ]

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


#: Settling after lulo-shell's `settled` mark before the idle window: its
#: last frames park within a few vblanks.
SHELL_IDLE_SETTLE_SECONDS = 2.0
#: The idle window's length (launch_smoke.py's IDLE_WINDOW_SECONDS).
IDLE_WINDOW_SECONDS = 20.0


def clear_of_minute(now: float, settle: float, window: float, margin: float = 0.5) -> float:
    """How long to settle from `now` (seconds since the epoch) so that a
    `window`-second measurement after it holds no minute boundary, where
    the bar's clock redraws: at least `settle`, or until just after the
    next minute."""
    start = (now + settle) % 60.0
    if start < margin or start + window + margin > 60.0:
        return settle + (60.0 - start + margin) % 60.0
    return settle


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
    # CI runners have no battery: a stand-in charging one for the battery
    # check (crates/rmac-power/src/win.rs).
    environment["LULO_FAKE_BATTERY"] = FAKE_BATTERY
    for folder in ("Roaming", "Local"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    # Lulo OS's default Dock (Files, Notes, Text Editor, Terminal, System
    # Settings) and Calculator, which the Dock check opens.
    settings = profile / "Roaming" / "Lulo" / "Config" / "rmac"
    settings.mkdir(parents=True, exist_ok=True)
    pins = ",".join(
        f'"org.rmac.{app}"' for app in ("Files", "Notes", "TextEditor", "Terminal", "SystemSettings", "Calculator")
    )
    (settings / "shell.json").write_text(
        f'{{"version": 3, "settings": {{"pinned_apps": [{pins}]}}}}\n', encoding="utf-8"
    )
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
        # The shared views' layer surfaces report the strips their exclusive
        # zones hold (ADR 0023, "Phase 3 revised: shared shell views").
        bar_strip = log.wait_for(r"^strip rmac-top-bar-\d+ at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
        dock_strip = log.wait_for(r"^strip rmac-dock-\d+ at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
        time.sleep(SETTLE_SECONDS)
        save(screenshots, "desktop")

        # 1. The work area.
        area = work_area()
        visible, state = taskbar_state()
        print(f"shell: while Lulo runs: work area {area}, taskbar visible {visible}, state {state}")
        bar_bottom = None
        if bar_strip is None or dock_strip is None:
            failures.append("the shell did not report the strips the bar and the Dock hold")
        else:
            bar_bottom = int(bar_strip.group(4))
            dock_top = int(dock_strip.group(2))
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

        # 1c. The bar is clear over the wallpaper, as on Lulo OS; the Dock's
        # shelf material draws the wallpaper blurred as niri blurs it (DWM's
        # blur painted a box on Windows Server and real PCs: WIN-OS-49).
        for surface, pattern, kind in (
            ("bar", r"^backdrop rmac-top-bar-\d+: (.+)$", "none"),
            ("Dock shelf", r"^backdrop rmac-dock-material-\d+: (.+)$", "wallpaper blur"),
        ):
            traced = log.wait_for(pattern, 10.0)
            print(f"shell: {surface} backdrop: {traced.group(1) if traced else 'not reported'}")
            if traced is None:
                failures.append(f"the {surface} reported no backdrop")
            elif traced.group(1) != kind:
                failures.append(f"the {surface} backdrop is {traced.group(1)}, not {kind}")
        check_dock_corners(log, failures)

        # 1d. The Spotlight hotkey (free here) and the Dock's tiles.
        hotkey = log.wait_for(r"^spotlight hotkey (.+) \((first choice|fallback)\)", 5.0)
        print(f"shell: Spotlight hotkey: {hotkey.group(0) if hotkey else 'not reported'}")
        if hotkey is None or hotkey.group(1) != "Alt+Space":
            failures.append("Alt+Space was free but is not Spotlight's hotkey")
        if log.wait_for(r"^notice shown", 0.5) is not None:
            failures.append("a hotkey notice showed although Alt+Space was free")
        bin_tile = log.wait_for(r"^dock tile trash at (\d+),(\d+) (full|empty)", 10.0)
        files_tile = log.last(r"^dock tile org\.rmac\.Files(?:\.desktop)? Files at (\d+),(\d+)")
        print(
            f"shell: Dock: Recycle Bin tile {bin_tile.group(0) if bin_tile else 'missing'}; "
            f"Files tile {files_tile.group(0) if files_tile else 'missing'}"
        )
        if bin_tile is None:
            failures.append("the Dock has no Recycle Bin tile")
        if files_tile is None:
            failures.append("the Dock has no Files tile")

        # 2. Idle, before any input, from the moment lulo-shell says its
        # start-up work is done (`settled`: Windows' Apps folder read into
        # desktop entries by its helper process, slow on the runner; the
        # icon helpers ended; the start-up memory trims), not after a fixed
        # delay. The bar's clock redraws once a minute, at the minute, so
        # the window is placed to hold no minute boundary.
        apps = log.wait_for(r"^windows apps: \d+ entries", 90.0)
        print(f"shell: {apps.group(0) if apps else 'windows apps: not read within 90 s'}")
        settled_at = time.monotonic()
        settled = log.wait_for(r"^settled$", 90.0)
        if settled is None:
            failures.append("lulo-shell never traced `settled`: some start-up work did not end")
        else:
            print(f"shell: settled {time.monotonic() - settled_at:.1f} s after the Apps folder was read")
        idle_settle = clear_of_minute(time.time(), SHELL_IDLE_SETTLE_SECONDS, IDLE_WINDOW_SECONDS)
        phases = startup_phases(log.text())
        for name, pid in (("lulo-shell", shell_pid), ("lulo-session", session.pid)):
            measurement: dict = {"launch_ms": round(launch_ms), "startup": phases if name == "lulo-shell" else []}
            if name == "lulo-shell":
                measurement["ready_ms"] = int(ready.group(1))
            measurements[name] = measurement
        # The session is measured over the same window as the shell.
        from launch_smoke import busy_threads, thread_cpu_times_100ns

        session_before = process_cpu_time_100ns(session.pid)
        session_threads = thread_cpu_times_100ns(session.pid)
        idle_shell = measure_idle_cpu(shell_pid, settle=idle_settle, log=log_path)
        session_after = process_cpu_time_100ns(session.pid)
        idle_session = None
        if session_before is not None and session_after is not None:
            ticks = max(session_after - session_before, 0) / tick_100ns
            threads_after = thread_cpu_times_100ns(session.pid)
            threads = busy_threads(session_threads, threads_after)
            ended = [tid for tid in session_threads if tid not in threads_after]
            unattributed = ticks - sum(thread_ticks for _, thread_ticks in threads)
            if unattributed >= 0.5:
                threads.append((f"{len(ended)} thread(s) that ended in the window", unattributed))
            idle_session = (ticks, 0.0, "", threads)
            if ticks >= 1.0:
                # lulo-session only waits on lulo-shell. What it was charged
                # comes from Windows' own thread-pool threads retiring about
                # a minute after it started (CI run 38538069296: 2.00
                # ticks, no live thread busy), so read it once more, as
                # launch_smoke.py does for an app; a real cost shows twice.
                print(f"shell: lulo-session: idle CPU = {ticks:.2f} ticks; measuring again")
                for thread, thread_ticks in threads:
                    print(f"shell: lulo-session:   first reading: thread {thread} used {thread_ticks:.2f} ticks")
                measurements["lulo-session"]["idle_first_ticks"] = round(ticks, 3)
                idle_session = measure_idle_cpu(session.pid, settle=2.0)
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
                # Every wake-up in order, and what the shell itself traced
                # meanwhile (window events and whose they were, helpers), to
                # name what woke an idle shell.
                for line in trace.splitlines()[:300]:
                    if line.startswith(wake_prefix):
                        print(f"shell: lulo-shell idle: {line[len(wake_prefix):]}")
                    elif not line.startswith("gpui_windows cpu: "):
                        print(f"shell: lulo-shell idle trace: {line}")
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

        note_memory(shell_pid, "before the Dock opens Calculator")
        # 3. The Dock opens Calculator.
        tile = log.last(r"^dock tile org\.rmac\.Calculator(?:\.desktop)? Calculator at (\d+),(\d+)")
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
                if log.wait_for(r"^dock tile org\.rmac\.Calculator(?:\.desktop)? Calculator at \d+,\d+ running", after=before) is None:
                    failures.append("Calculator's Dock tile shows no running dot")
                time.sleep(SETTLE_SECONDS)
                save(screenshots, "dock-opened-calculator")

                # 4. Calculator's own menus in the bar, and About from them.
                menus = log.wait_for(r"^menus from org\.rmac\.Calculator", after=before)
                title = log.wait_for(r"^bar title 1 Calculator at (-?\d+),(-?\d+)$", after=before)
                if menus is None or title is None:
                    failures.append("the bar does not show Calculator's menus")
                else:
                    pid, _ = calculator
                    windows_before = len(windows_of(pid, titled=True))
                    click(int(title.group(1)), int(title.group(2)))
                    if log.wait_for(r"^menu 1 row ", after=before) is None:
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

        note_memory(shell_pid, "after Calculator and its menu")
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
                tile = log.wait_for(r"^dock tile org\.rmac\.Files(?:\.desktop)? Files at \d+,\d+ running", after=before)
                stray = log.wait_for(r"^dock tile \S*files-dev", 0.5, after=before)
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

        note_memory(shell_pid, "after Files as files-dev.exe")
        # 4c. The Recycle Bin tile follows the bin and has its menu.
        if bin_tile is not None:
            before = len(log.lines())
            initial = bin_tile
            junk = profile / "lulo-recycle-check.txt"
            junk.write_text("Lulo's Recycle Bin check\n", encoding="utf-8")
            recycled = recycle(junk)
            full = log.wait_for(r"^dock tile trash at \d+,\d+ full", 10.0, after=before)
            if initial is not None and initial.group(3) == "full":
                full = initial
            print(
                f"shell: the Dock's bin started {initial.group(3) if initial else 'unreported'}; "
                f"recycled a file: {recycled}; the bin {'shows full' if full else 'did not turn full'}"
            )
            if not recycled or full is None:
                failures.append("the Dock's Recycle Bin did not show full after a file was recycled")
            right = log.last(r"^dock tile trash at (\d+),(\d+)")
            if right is not None:
                user32().SetCursorPos(int(right.group(1)), int(right.group(2)))
                time.sleep(0.15)
                user32().mouse_event(0x0008, 0, 0, 0, 0)
                time.sleep(0.05)
                user32().mouse_event(0x0010, 0, 0, 0, 0)
                menu = log.wait_for(r"^layer rmac-dock-menu-keyboard at", 5.0, after=before)
                time.sleep(0.8)
                save(screenshots, "dock-recycle-bin-menu")
                if menu is None:
                    failures.append("right-clicking the Recycle Bin tile opened no menu")
                tap(VK_ESCAPE)
                log.wait_for(r"^layer rmac-dock-menu-keyboard closed", 5.0, after=before)

        note_memory(shell_pid, "after the Recycle Bin")
        # 5a. The desktop's icons: open, rename, drag, menu, new files, and
        # the desktop staying under app windows.
        if desktop_hwnd:
            check_desktop_icons(log, desktop_hwnd, check_folder, check_note, screenshots, opened, failures)
            check_desktop_menu(log, desk, screenshots, failures)

        note_memory(shell_pid, "after the desktop icons")
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

        note_memory(shell_pid, "after maximised Notepad")
        # 6. Spotlight.
        before = len(log.lines())
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        shown = log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", after=before, timeout=10.0)
        if shown is None:
            failures.append("Alt+Space did not open Spotlight")
        else:
            time.sleep(0.8)
            spotlight = user32().GetForegroundWindow()
            surface = tuple(int(shown.group(index)) for index in range(1, 5))
            save(screenshots, "spotlight-empty")
            check_spotlight_empty(spotlight, surface, shell_pid, log, before, failures)
            type_text("text editor")
            results = log.wait_for(r'^spotlight "text editor": ([1-9]\d*) results', after=before, timeout=10.0)
            time.sleep(0.8)
            grown = log.wait_for(r"^spotlight expanded", 2.0, after=before)
            print(f"shell: Spotlight with results: {'expanded' if grown else 'still compact'}")
            if grown is None:
                failures.append("Spotlight did not grow for its results")
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
        if log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", after=before, timeout=10.0):
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
            elif log.wait_for(r"^layer rmac-launcher closed", 0.5, after=before) is None:
                # Nothing to open (the runner's Apps folder may have no
                # Notepad): close Spotlight as the user would. As on the
                # Mac, the first Escape clears the query and the second
                # closes Spotlight.
                tap(VK_ESCAPE)
                if log.wait_for(r"^layer rmac-launcher closed", 1.0, after=before) is None:
                    tap(VK_ESCAPE)
        if log.wait_for(r"^layer rmac-launcher closed", 5.0, after=before) is None:
            failures.append("Spotlight did not close")
        catalog = log.last(r"^catalog: (\d+) apps")
        print(f"shell: Spotlight's catalogue: {catalog.group(1) if catalog else '?'} apps")

        note_memory(shell_pid, "after Spotlight opened Text Editor and Notepad")
        # 6b. A closed Spotlight lets go of its window and file list.
        released = log.wait_for(r"^layer rmac-launcher closed", 15.0, after=before)
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
        # 6e1. The Dock's recent apps are the last three used that are not
        # kept; the bar's battery and its menu (macOS 26's bolt, "Charging").
        check_dock_recents(log, profile, failures)
        check_battery(log, screenshots, failures)

        # 6e2. Control Centre inside the screen; the bar over a full-screen
        # window; the Window menu's Full-Screen Tile and the app menu's
        # Hide Others and Show All (Windows robot).
        check_control_centre(log, shell_pid, screenshots, failures)
        check_full_screen_bar(log, bin_dir, environment, screenshots, failures)
        check_window_menu(log, bin_dir, environment, screenshots, failures)

        # 6f. ⌘Tab (Alt+Tab) and Notification Centre, the shared views
        # lulo-shell hosts; then their memory once they have closed.
        check_switcher(log, bin_dir, environment, screenshots, opened, failures)
        check_notification_centre(log, screenshots, failures)
        # 6g. Drag-and-drop keeping in the Dock (macOS 26): a running app
        # dragged across the separator, a reorder, a drag off to remove,
        # and an app dragged in from Apps.
        check_dock_drag(log, bin_dir, environment, screenshots, failures)
        check_apps_open_speed(log, bin_dir, environment, failures)
        time.sleep(5.0)
        memory = memory_mb(shell_pid)
        print(f"shell: lulo-shell memory after the switcher and Notification Centre closed: {memory}")
        if memory is not None:
            measurements["lulo-shell"]["after_switcher_nc_private_mb"] = memory["private_mb"]

        # 6g. A third-party window that opens at 0,0 and 4000 × 4000 is put
        # inside the work area (macOS's visible frame); a full-screen one is
        # left alone. Last, after the memory readings: a full-screen window
        # makes the bar and the Dock step aside.
        check_foreign_windows(screenshots, failures)

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
            if not line.startswith(wake_prefix) and not line.startswith("lulo-shell: window ")
        ]
        for line in trace[:600]:
            print(f"shell log: {line}")
    return failures


def note_memory(pid: int, step: str) -> None:
    """lulo-shell's memory at a step of the check, to find what a use
    keeps (WIN-OS-64); reported only."""
    memory = memory_mb(pid)
    if memory is not None:
        print(f"shell: memory {step}: private {memory['private_mb']} MB, working set {memory['working_set_mb']} MB")


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


def check_dock_corners(log: "Log", failures: list[str]) -> None:
    """Outside its rounded shelf the Dock's material must be clear: the
    pixel in each corner of the shelf's rectangle matches the wallpaper just
    beside it (WIN-OS-49: a dark box showed there on a real PC)."""
    placed = log.last(r"^layer rmac-dock-material-\d+ at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
    if placed is None:
        failures.append("the Dock's shelf material was not placed")
        return
    time.sleep(1.0)
    left, top, right, bottom = (int(placed.group(index)) for index in range(1, 5))
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
            failures.append(f"the Dock's {name} corner is {corner}, not the wallpaper beside it {beside}")


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
    # Lulo app windows draw their own frame and shadow (ADR 0023 "Window
    # chrome: Lulo's own corners and shadow"), with DWM's non-client
    # rendering off; the darkening below is what counts.
    # Windows Server's DWM shadow is fainter than a PC's; any clear
    # darkening just outside the edge is the shadow.
    if max(darker) < 1.5:
        failures.append(f"{name}'s window casts no shadow (screen just outside it darkened by {darker})")


def check_spotlight_empty(spotlight: int, rect, shell_pid: int, log: Log, after: int, failures: list[str]) -> None:
    """Spotlight opens as the search bar alone, centred, in the upper third.
    Its surface keeps the expanded size, as on Lulo OS; only the bar takes
    input (and shows) until results come."""
    scale = window_scale(spotlight)
    width, height = screen_size()
    panel_top = rect[1] + round(SPOTLIGHT_MARGIN * scale)
    centre = (rect[0] + rect[2]) // 2
    compact = log.wait_for(r"^spotlight (compact|expanded)", 2.0, after=after)
    print(
        f"shell: Spotlight before typing: surface {rect}, {compact.group(1) if compact else 'size not reported'}, "
        f"panel top {panel_top} of {height}, centre {centre} of {width}"
    )
    if pid_of(spotlight) != shell_pid:
        failures.append("Spotlight's window was not in front when it opened")
        return
    if compact is None or compact.group(1) != "compact":
        failures.append("Spotlight shows more than the search bar before typing")
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
    files = log.wait_for(r"^bar title 1 Files at", 5.0, after=mark) or log.last(r"^bar title 1 Files at")
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
        menu = log.wait_for(r"^desktop menu open", 5.0, after=mark)
        time.sleep(0.6)
        save(screenshots, "desktop-icon-menu")
        print(f"shell: right-click on a desktop icon: {'menu' if menu else 'no menu'}")
        if menu is None:
            failures.append("right-clicking a desktop icon opened no menu")
        tap(VK_ESCAPE)
        log.wait_for(r"^desktop menu closed", 5.0, after=mark)
        time.sleep(0.5)

    # Rename: select, Return, type, Return.
    icon = log.last(r"^desktop icon Lulo Check Note\.txt at (\d+),(\d+)")
    renamed = note.parent / "renamed.txt"
    if icon is not None:
        click(int(icon.group(1)), int(icon.group(2)))
        time.sleep(0.4)
        mark = len(log.lines())
        tap(VK_RETURN)
        editing = log.wait_for(r"^desktop rename editing", 5.0, after=mark)
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
        print(f"shell: double-click on a desktop folder: {'Files opened' if files_window else 'nothing opened'}")
        if not files_window:
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
        tile = log.last(r"^dock tile org\.rmac\.TextEditor(?:\.desktop)? Text Editor at (\d+),(\d+)")
        if tile is not None:
            click(int(tile.group(1)), int(tile.group(2)))

    def from_spotlight(mark: int) -> None:
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        if log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 10.0, after=mark) is None:
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


#: The rows of Lulo OS's desktop menu (Finder's on the Mac) that Windows
#: must show too (Windows robot).
DESKTOP_MENU_ROWS = (
    "New Folder",
    "Get Info",
    "Change Wallpaper…",
    "Use Stacks",
    "Sort By",
    "Clean Up",
    "Show View Options",
)
VK_UP = 0x26


def check_desktop_menu(log: Log, desk: Path, screenshots: Path | None, failures: list[str]) -> None:
    """A right-click on the wallpaper opens Lulo OS's desktop menu, and its
    rows work: New Folder makes a folder (and starts renaming it), Show View
    Options opens the panel."""
    width, height = screen_size()
    spot = (150, height - 120)  # Clear of the icons (top right) and the Dock.
    mark = len(log.lines())
    right_click(*spot)
    menu = log.wait_for(r"^desktop menu open: (.+)$", 5.0, after=mark)
    time.sleep(0.6)
    save(screenshots, "desktop-menu")
    rows = menu.group(1).split(" | ") if menu else []
    print(f"shell: right-click on the wallpaper: {rows if menu else 'no menu'}")
    if menu is None:
        failures.append("right-clicking the wallpaper opened no desktop menu")
        return
    missing = [row for row in DESKTOP_MENU_ROWS if row not in rows]
    if missing:
        failures.append(f"the desktop menu lacks {missing} (it has {rows})")
    # New Folder: the first row.
    before = {path.name for path in desk.iterdir()} if desk.exists() else set()
    tap(VK_DOWN)
    time.sleep(0.2)
    tap(VK_RETURN)
    created = wait_until(
        lambda: [path for path in desk.iterdir() if path.is_dir() and path.name not in before], 8.0
    )
    editing = log.wait_for(r"^desktop rename editing", 5.0, after=mark)
    time.sleep(0.5)
    save(screenshots, "desktop-new-folder")
    print(f"shell: desktop menu New Folder: {[path.name for path in created or []]}, renaming {bool(editing)}")
    if not created:
        failures.append("the desktop menu's New Folder made no folder on the Desktop")
    tap(VK_ESCAPE)
    log.wait_for(r"^desktop rename idle", 5.0, after=mark)
    time.sleep(0.5)
    for path in created or []:
        try:
            path.rmdir()
        except OSError as error:
            print(f"shell: could not remove {path}: {error}")
    # Show View Options: the last row.
    mark = len(log.lines())
    right_click(*spot)
    if log.wait_for(r"^desktop menu open", 5.0, after=mark) is None:
        failures.append("the desktop menu did not open a second time")
        return
    time.sleep(0.4)
    tap(VK_UP)
    time.sleep(0.2)
    tap(VK_RETURN)
    panel = log.wait_for(r"^desktop panel view options", 5.0, after=mark)
    time.sleep(0.6)
    save(screenshots, "desktop-view-options")
    print(f"shell: desktop menu Show View Options: {'panel open' if panel else 'nothing'}")
    if panel is None:
        failures.append("the desktop menu's Show View Options opened no panel")
    tap(VK_ESCAPE)
    if panel is not None and log.wait_for(r"^desktop panel none", 5.0, after=mark) is None:
        failures.append("Esc did not close the desktop's View Options")
    time.sleep(0.5)


def shell_window_at(shell_pid: int, left: int, top: int) -> int | None:
    """The shell's window whose top left corner is at `left`, `top`."""
    for hwnd in windows_of(shell_pid):
        rect = wintypes.RECT()
        user32().GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(rect))
        if rect.left == left and rect.top == top:
            return hwnd
    return None


def check_control_centre(log: Log, shell_pid: int, screenshots: Path | None, failures: list[str]) -> None:
    """Control Centre opens under its status item and, once grown to its
    content, still ends its 8 pt margin short of the screen's right edge
    (it sat 8 px past the edge: Windows robot)."""
    item = log.last(r"^bar control centre at (-?\d+),(-?\d+)")
    if item is None:
        failures.append("the bar did not say where its Control Centre item is")
        return
    mark = len(log.lines())
    click(int(item.group(1)), int(item.group(2)))
    placed = log.wait_for(r"^layer rmac-quick-settings at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 10.0, after=mark)
    if placed is None:
        failures.append("clicking the bar's Control Centre item opened no Control Centre")
        return
    time.sleep(1.5)
    save(screenshots, "control-centre")
    left, top = int(placed.group(1)), int(placed.group(2))
    hwnd = shell_window_at(shell_pid, left, top)
    width, _ = screen_size()
    if hwnd is None:
        failures.append("found no Control Centre window where it was placed")
    else:
        rect = wintypes.RECT()
        user32().GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(rect))
        margin = round(8 * window_scale(hwnd))
        print(
            f"shell: Control Centre window {rect.left},{rect.top},{rect.right},{rect.bottom}, "
            f"screen width {width}"
        )
        if rect.right > width - margin + 1:
            failures.append(
                f"Control Centre ends at x={rect.right}, past its margin from the screen's edge ({width})"
            )
    tap(VK_ESCAPE)
    if log.wait_for(r"^layer rmac-quick-settings closed", 5.0, after=mark) is None:
        click(int(item.group(1)), int(item.group(2)))
    time.sleep(0.8)


VK_F11 = 0x7A


def start_text_editor(bin_dir: Path, environment: dict) -> tuple[subprocess.Popen, int] | None:
    """Text Editor in front, its content clicked so it has the keyboard."""
    exe = bin_dir / "rmac-text-editor.exe"
    if not exe.is_file():
        return None
    for pid in processes_named("rmac-text-editor.exe"):
        terminate(pid)
    process = subprocess.Popen([str(exe)], env=environment)
    found = wait_until(lambda: app_window("rmac-text-editor.exe"), 20.0)
    if found is None:
        process.kill()
        return None
    time.sleep(1.5)
    hwnd = found[1]
    bring_to_front(hwnd)
    left, top, right, bottom = frame_bounds(hwnd)
    click((left + right) // 2, (top + bottom) // 2)
    time.sleep(0.5)
    return process, hwnd


def check_full_screen_bar(
    log: Log, bin_dir: Path, environment: dict, screenshots: Path | None, failures: list[str]
) -> None:
    """Over a full-screen window the bar hides, the pointer at the top edge
    reveals it and moving away hides it again, as on the Mac; when the
    window leaves full screen the bar is back (it stayed hidden, its buttons
    at y=-14, after Window-menu actions: Windows robot)."""
    started = start_text_editor(bin_dir, environment)
    if started is None:
        failures.append("Text Editor did not open for the full-screen check")
        return
    process, hwnd = started
    width, height = screen_size()
    try:
        mark = len(log.lines())
        tap(VK_F11)
        full = wait_until(lambda: frame_bounds(hwnd) == (0, 0, width, height), 5.0)
        hidden = log.wait_for(r"^top bar hidden over full screen", 5.0, after=mark)
        time.sleep(1.0)
        save(screenshots, "full-screen")
        print(f"shell: Text Editor full screen: {bool(full)}; bar {'hidden' if hidden else 'still shown'}")
        if full is None:
            failures.append("F11 did not put Text Editor in full screen")
            return
        if hidden is None:
            failures.append("the bar did not hide over a full-screen window")
        mark = len(log.lines())
        user32().SetCursorPos(width // 2, 0)
        revealed = log.wait_for(r"^top bar shown over full screen", 5.0, after=mark)
        time.sleep(0.6)
        save(screenshots, "full-screen-top-edge")
        # Revealed over a full-screen window, the bar is solid, as on the
        # Mac: its empty middle shows the bar's own fill (light F6F6F6 or
        # dark 1E1E20, `tokens::top_bar_full_screen_fill`), not the window.
        if revealed is not None:
            fill = pixel(width // 2, 10)
            solid = any(
                all(abs(channel - expected) <= 3 for channel, expected in zip(fill, colour))
                for colour in ((0xF6, 0xF6, 0xF6), (0x1E, 0x1E, 0x20))
            )
            print(f"shell: revealed bar over full screen at the middle: {fill}; solid: {solid}")
            if not solid:
                failures.append(f"the bar revealed over a full-screen window is not solid ({fill})")
        user32().SetCursorPos(width // 2, height // 2)
        hidden_again = log.wait_for(r"^top bar hidden over full screen", 5.0, after=mark)
        print(
            f"shell: pointer at the top edge: bar {'revealed' if revealed else 'not revealed'}; "
            f"away again: {'hidden' if hidden_again else 'still shown'}"
        )
        if revealed is None:
            failures.append("the pointer at the top edge did not reveal the bar over a full-screen window")
        elif hidden_again is None:
            failures.append("the bar did not hide again when the pointer left it")
        mark = len(log.lines())
        bring_to_front(hwnd)
        tap(VK_F11)
        back = log.wait_for(r"^top bar shown$", 5.0, after=mark)
        time.sleep(1.0)
        save(screenshots, "full-screen-left")
        print(f"shell: Text Editor left full screen: bar {'back' if back else 'NOT back'}")
        if back is None:
            failures.append("the bar did not come back when the window left full screen")
    finally:
        process.kill()
        time.sleep(1.0)


def bar_menu_row(log: Log, title: str, row: str, mark: int):
    """Open the bar's `title` menu; the screen point of its `row`."""
    titles = log.last(rf"^bar title (\d+) {re.escape(title)} at (-?\d+),(-?\d+)$")
    if titles is None:
        return None
    index = titles.group(1)
    click(int(titles.group(2)), int(titles.group(3)))
    return log.wait_for(rf"^menu {index} row {re.escape(row)} at (-?\d+),(-?\d+)$", 5.0, after=mark)


def check_window_menu(
    log: Log, bin_dir: Path, environment: dict, screenshots: Path | None, failures: list[str]
) -> None:
    """Window ▸ Full-Screen Tile ▸ Left of Screen puts the window on the
    left half (Windows' Snap Assist then offers the others); the app menu's
    Hide Others minimises the other apps' windows and Show All brings them
    back (both did nothing on Windows: Windows robot)."""
    notepad = subprocess.Popen(["notepad.exe"])
    other = wait_until(lambda: app_window("notepad.exe"), 15.0)
    started = start_text_editor(bin_dir, environment)
    if started is None or other is None:
        failures.append("Text Editor or Notepad did not open for the Window menu check")
        notepad.kill()
        return
    process, hwnd = started
    area = work_area()
    try:
        # Full-Screen Tile ▸ Left of Screen: hover the row, then its
        # submenu's first item from the keyboard.
        mark = len(log.lines())
        row = bar_menu_row(log, "Window", "Full-Screen Tile", mark)
        if row is None:
            failures.append("the bar's Window menu has no Full-Screen Tile row for Text Editor")
            tap(VK_ESCAPE)
        else:
            user32().SetCursorPos(int(row.group(1)), int(row.group(2)))
            time.sleep(1.0)
            tap(VK_DOWN)
            time.sleep(0.2)
            tap(VK_RETURN)
            middle = (area[0] + area[2]) // 2

            def on_left_half():
                # The left half, full height; no narrower than the app's
                # own minimum width (Text Editor's is wider than half of the
                # runner's 1024 px screen).
                left, top, right, bottom = frame_bounds(hwnd)
                return (
                    abs(left - area[0]) <= 2
                    and abs(top - area[1]) <= 2
                    and abs(bottom - area[3]) <= 2
                    and right - left <= max(middle - area[0], 620) + 12
                )

            tiled = wait_until(on_left_half, 5.0)
            time.sleep(1.0)
            save(screenshots, "full-screen-tile")
            print(f"shell: Full-Screen Tile ▸ Left of Screen: Text Editor at {frame_bounds(hwnd)}, work area {area}")
            if tiled is None:
                failures.append(f"Full-Screen Tile ▸ Left of Screen left Text Editor at {frame_bounds(hwnd)}")
            # Snap Assist's picker, if it showed.
            tap(VK_ESCAPE)
            time.sleep(0.5)
        # Hide Others, then Show All, from Text Editor's app menu.
        bring_to_front(hwnd)
        time.sleep(0.5)
        mark = len(log.lines())
        app_title = log.last(r"^bar title 1 (.+) at (-?\d+),(-?\d+)$")
        name = app_title.group(1) if app_title else "Text Editor"
        row = bar_menu_row(log, name, "Hide Others", mark)
        if row is None:
            failures.append("the app menu has no Hide Others row")
            tap(VK_ESCAPE)
            return
        click(int(row.group(1)), int(row.group(2)))
        hidden = wait_until(lambda: user32().IsIconic(wintypes.HWND(other[1])), 5.0)
        kept = not user32().IsIconic(wintypes.HWND(hwnd))
        time.sleep(0.8)
        save(screenshots, "hide-others")
        print(f"shell: Hide Others: Notepad {'minimised' if hidden else 'still shown'}, Text Editor kept {kept}")
        if not hidden:
            failures.append("Hide Others did not hide Notepad")
        if not kept:
            failures.append("Hide Others hid the app it was chosen in")
        bring_to_front(hwnd)
        time.sleep(0.5)
        mark = len(log.lines())
        row = bar_menu_row(log, name, "Show All", mark)
        if row is None:
            failures.append("the app menu has no Show All row")
            tap(VK_ESCAPE)
            return
        click(int(row.group(1)), int(row.group(2)))
        shown = wait_until(lambda: not user32().IsIconic(wintypes.HWND(other[1])), 5.0)
        time.sleep(0.8)
        save(screenshots, "show-all")
        print(f"shell: Show All: Notepad {'back' if shown else 'still hidden'}")
        if not shown:
            failures.append("Show All did not bring back the window Hide Others hid")
    finally:
        process.kill()
        notepad.kill()
        for pid in processes_named("notepad.exe"):
            terminate(pid)
        time.sleep(1.0)


def check_foreign_windows(screenshots: Path | None, failures: list[str]) -> None:
    """scripts/windows/foreign_window.py's two windows: the huge one ends
    inside the work area, the full-screen one still covers the screen."""
    import sys

    helper = Path(__file__).resolve().parent / "foreign_window.py"
    process = subprocess.Popen([sys.executable, str(helper)])
    try:
        def found():
            titled = {}
            for hwnd in windows_of(process.pid, titled=True):
                buffer = ctypes.create_unicode_buffer(128)
                user32().GetWindowTextW(hwnd, buffer, 128)
                titled[buffer.value] = hwnd
            if "Lulo Foreign Window" in titled and "Lulo Foreign Full Screen" in titled:
                return titled
            return None

        titled = wait_until(found, 20.0)
        if titled is None:
            failures.append("foreign_window.py opened no windows")
            return
        area = work_area()
        big = titled["Lulo Foreign Window"]

        def placed():
            left, top, right, bottom = frame_bounds(big)
            inside = left >= area[0] and top >= area[1] and right <= area[2] and bottom <= area[3]
            return (left, top, right, bottom) if inside else None

        inside = wait_until(placed, 10.0)
        bounds = frame_bounds(big)
        print(f"shell: third-party 4000 × 4000 window at 0,0 now {bounds}, work area {area}")
        save(screenshots, "foreign-window")
        if inside is None:
            failures.append(f"a third-party window opened at 0,0 4000 × 4000 stayed at {bounds}, "
                            f"outside the work area {area}")
        width, height = screen_size()
        full = frame_bounds(titled["Lulo Foreign Full Screen"])
        print(f"shell: third-party full-screen window {full}, screen {width} × {height}")
        if full[0] > 0 or full[1] > 0 or full[2] < width or full[3] < height:
            failures.append(f"a full-screen third-party window was moved to {full}")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
        time.sleep(0.5)


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


#: The stand-in battery: 85%, charging, 1:05 until full.
FAKE_BATTERY = "85,charging,3900"

#: The Lulo apps check_app_placement opens that the smoke Dock does not
#: keep, in the order it opens them: the Dock's recent apps afterwards.
RECENT_APPS = ("org.rmac.Preview.desktop", "org.rmac.Clock.desktop", "org.rmac.Weather.desktop")


def check_dock_recents(log: Log, profile: Path, failures: list[str]) -> None:
    """After check_app_placement opened and quit every Lulo app, the Dock's
    recent apps (as on the Mac and Lulo OS) are the last three it opened
    that the Dock does not keep, oldest first, kept in the Dock's state
    file and shown as tiles after the kept apps. Windows' own apps listed
    by an AppUserModelID (Edge's `MSEdge`) get an entry by their program,
    so they join the recent apps too."""
    state = profile / "Local" / "Lulo" / "State" / "rmac" / "dock-recents"

    def recents() -> list[str]:
        try:
            return [line.strip() for line in state.read_text(encoding="utf-8").splitlines() if line.strip()]
        except OSError:
            return []

    # Apps still open from earlier checks stay too (running apps are never
    # dropped), before these.
    saved = wait_until(lambda: recents()[-len(RECENT_APPS):] == list(RECENT_APPS), 10.0)
    print(f"shell: the Dock's recent apps after opening the Lulo apps: {recents()}")
    if saved is None:
        failures.append(f"the Dock's recent apps are {recents()}, not the last three used {list(RECENT_APPS)}")
    _, running = dock_groups(log)
    shown = [app_id for app_id, _, _ in running if app_id != "trash"]
    wanted = [dock_app_id(app_id) for app_id in RECENT_APPS]
    print(f"shell: Dock tiles after the separator: {shown}")
    if [app_id for app_id in shown if app_id in wanted] != wanted:
        failures.append(f"the Dock shows {shown} after its kept apps, not the recent apps {wanted}")
    applications = profile / "Roaming" / "Lulo" / "Data" / "applications"
    edge = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft" / "Edge" / "Application" / "msedge.exe"
    if edge.is_file():
        entry = wait_until(lambda: (applications / "msedge.exe.desktop").is_file(), 30.0)
        print(f"shell: Microsoft Edge (listed as MSEdge) has an entry by its program: {entry is not None}")
        if entry is None:
            failures.append("Microsoft Edge has no msedge.exe entry, so it can never be a recent app")


def check_battery(log: Log, screenshots: Path | None, failures: list[str]) -> None:
    """The bar's battery while it charges, as macOS 26 draws it: the bolt,
    "Battery 85%, Charging" to a screen reader, and its menu: the
    percentage, Power Source: Power Adapter and "Charging: 1:05 until
    full" (the stand-in FAKE_BATTERY)."""
    item = log.wait_for(r"^battery item Battery 85%, Charging, mark Some\(Charging\)", 10.0)
    print(f"shell: the bar's battery: {item.group(0) if item else log.last(r'^battery item .*')}")
    if item is None:
        failures.append("the bar's battery does not show the charging bolt for a charging battery")
        return
    spot = log.last(r"^battery status at (-?\d+),(-?\d+)")
    if spot is None:
        failures.append("the bar did not say where its battery is")
        return
    before = len(log.lines())
    click(int(spot.group(1)), int(spot.group(2)))
    menu = log.wait_for(r"^status menu Battery: (.+)$", 5.0, after=before)
    save(screenshots, "battery-menu")
    text = menu.group(1) if menu else ""
    print(f"shell: the Battery menu: {text}")
    for row in ("Battery 85%", "Power Source: Power Adapter", "Charging: 1:05 until full", "Battery Settings…"):
        if row not in text.split(" | "):
            failures.append(f"the Battery menu has no \"{row}\" row: {text!r}")
    tap(VK_ESCAPE)
    time.sleep(0.5)


VK_TAB = 0x09


def switcher_sizes(log: Log, after: int) -> list[tuple[int, int]]:
    """Every size the switcher's surface was placed at since line `after`."""
    sizes = []
    for line in log.lines()[after:]:
        match = re.search(r"^layer rmac-app-switcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)$", line)
        if match:
            left, top, right, bottom = (int(value) for value in match.groups())
            sizes.append((right - left, bottom - top))
    return sizes


def foreground_is(pid: int, timeout: float = 5.0) -> bool:
    return wait_until(lambda: foreground_pid_and_class()[0] == pid, timeout) is not None


def check_switcher(
    log: Log, bin_dir: Path, environment: dict, screenshots: Path | None, opened: list[str], failures: list[str]
) -> None:
    """⌘Tab on Windows is Alt+Tab, taken by Lulo's hook while the layer
    runs (ADR 0023, "Notification Centre and the app switcher on Windows"):
    a quick tap switches to the previous app without showing the panel,
    holding Alt shows Lulo's panel and its release switches, and Esc puts
    back the app that was in front."""
    started = {}
    for app in ("rmac-calculator", "rmac-notes"):
        exe = bin_dir / f"{app}.exe"
        for pid in processes_named(f"{app}.exe"):
            terminate(pid)
        subprocess.Popen([str(exe)], env=environment)
        found = wait_until(lambda app=app: app_window(f"{app}.exe"), 20.0)
        if found is None:
            failures.append(f"{app} opened no window for the switcher check")
            return
        opened.append(f"{app}.exe")
        started[app] = found
        time.sleep(1.5)
    calculator, notes = started["rmac-calculator"][0], started["rmac-notes"][0]
    first_line = len(log.lines())
    failures_before = len(failures)
    bring_to_front(started["rmac-notes"][1])
    if not foreground_is(notes):
        print(f"shell: switcher: Notes is not in front ({foreground_pid_and_class()}); continuing")
    time.sleep(1.0)

    # A quick tap: the previous app comes forward, no panel shows.
    before = len(log.lines())
    key(VK_MENU)
    tap(VK_TAB)
    key(VK_MENU, up=True)
    switched = foreground_is(calculator)
    time.sleep(0.5)
    flashed = [size for size in switcher_sizes(log, before) if size[0] > 50]
    print(
        f"shell: switcher quick Alt+Tab: Calculator {'in front' if switched else 'NOT in front'}, "
        f"panel {'shown ' + str(flashed) if flashed else 'not shown'}"
    )
    if not switched:
        failures.append("a quick Alt+Tab did not bring the previous app (Calculator) to the front")
    if flashed:
        failures.append(f"a quick Alt+Tab flashed the switcher panel ({flashed})")

    # Held: the panel shows; Alt's release switches back to Notes.
    before = len(log.lines())
    key(VK_MENU)
    tap(VK_TAB)
    shown = wait_until(lambda: [size for size in switcher_sizes(log, before) if size[0] > 50], 3.0)
    time.sleep(0.4)
    save(screenshots, "switcher")
    key(VK_MENU, up=True)
    back = foreground_is(notes)
    print(
        f"shell: switcher held Alt+Tab: panel {shown[-1] if shown else 'not shown'}, "
        f"Notes {'in front' if back else 'NOT in front'} after Alt was released"
    )
    if not shown:
        failures.append("holding Alt after Alt+Tab did not show the switcher panel")
    if not back:
        failures.append("releasing Alt did not switch to the selected app (Notes)")
    if log.wait_for(r"^layer rmac-app-switcher closed", 5.0, after=before) is None:
        failures.append("the switcher panel did not close after Alt was released")

    # Esc while held cancels: Notes stays in front.
    time.sleep(0.5)
    before = len(log.lines())
    key(VK_MENU)
    tap(VK_TAB)
    wait_until(lambda: [size for size in switcher_sizes(log, before) if size[0] > 50], 3.0)
    tap(VK_ESCAPE)
    key(VK_MENU, up=True)
    time.sleep(1.0)
    kept = foreground_pid_and_class()[0] == notes
    print(f"shell: switcher Esc: Notes {'still in front' if kept else 'NOT in front'}")
    if not kept:
        failures.append("Esc in the switcher did not keep the app that was in front (Notes)")
    if len(failures) > failures_before:
        # What the switcher saw and did, to tell a stale window list from a
        # refused activation.
        print(f"shell: switcher: Calculator pid {calculator} window {started['rmac-calculator'][1]}, "
              f"Notes pid {notes} window {started['rmac-notes'][1]}, "
              f"front now {foreground_pid_and_class()} window {user32().GetForegroundWindow()}")
        for line in log.lines()[first_line:]:
            if "switcher" in line or line.startswith("window list read") or "app-switcher" in line:
                print(f"shell: switcher trace: {line}")
        for line in log.text().splitlines():
            if "app switcher could not" in line:
                print(f"shell: switcher error: {line}")
    for app in ("rmac-calculator", "rmac-notes"):
        for pid in processes_named(f"{app}.exe"):
            terminate(pid)
        if f"{app}.exe" in opened:
            opened.remove(f"{app}.exe")
    time.sleep(1.0)


def post_notification(summary: str, body: str, timeout: float = 15.0):
    """Post on Lulo's notification pipe as a Lulo app does
    (`rmac_notifications::pipe`). Returns (id, a function that waits for
    the next answer line), or None."""
    import threading

    user = "".join(c for c in os.environ.get("USERNAME", "") if c.isalnum() or c in "-_.")
    path = rf"\\.\pipe\lulo-notifications-{user}"
    try:
        pipe = open(path, "r+b", buffering=0)
    except OSError as error:
        print(f"shell: notification pipe {path}: {error}")
        return None
    pipe.write(f"post\t{summary}\t{body}\t\t1\t\n".encode())
    lines: list[str] = []

    def read():
        buffer = b""
        while True:
            try:
                byte = pipe.read(1)
            except OSError:
                return
            if not byte:
                return
            if byte == b"\n":
                lines.append(buffer.decode(errors="replace"))
                buffer = b""
            else:
                buffer += byte

    threading.Thread(target=read, daemon=True).start()
    posted = wait_until(lambda: lines, timeout)
    if not posted or not lines[0].startswith("posted\t"):
        return None

    def next_answer(index: int, wait: float):
        return wait_until(lambda: lines[index] if len(lines) > index else None, wait)

    return int(lines[0].split("\t")[1]), next_answer


def check_notification_centre(log: Log, screenshots: Path | None, failures: list[str]) -> None:
    """A notification on Lulo's pipe shows as a Lulo banner, is kept in
    Notification Centre's history, and the menu bar's clock opens
    Notification Centre with it (ADR 0023)."""
    before = len(log.lines())
    posted = post_notification("Lulo check", "A banner from the Windows smoke test")
    if posted is None:
        failures.append("Lulo's notification pipe did not take a notification")
        return
    notification_id, next_answer = posted
    banner = log.wait_for(r"^layer rmac-notification-banners at", 10.0, after=before)
    time.sleep(1.0)
    save(screenshots, "notification-banner")
    print(f"shell: notification {notification_id} posted; banner {'shown' if banner else 'NOT shown'}")
    if banner is None:
        failures.append("a posted notification showed no Lulo banner")
    # The banner times out (as on the Mac, about 5 s) and the app hears it.
    closed = next_answer(1, 20.0)
    print(f"shell: the poster heard: {closed!r}")
    if closed is None or not closed.startswith("closed\t"):
        failures.append(f"the notification's poster did not hear it close (got {closed!r})")
    # The clock, the bar's rightmost item, opens Notification Centre.
    bar = log.last(r"^strip rmac-top-bar-\d+ at (-?\d+),(-?\d+),(-?\d+),(-?\d+)")
    if bar is None:
        failures.append("no menu bar strip to find the clock in")
        return
    left, top, right, bottom = (int(value) for value in bar.groups())
    before = len(log.lines())
    click(right - 40, (top + bottom) // 2)
    panel = log.wait_for(r"^layer rmac-notification-center at", 10.0, after=before)
    records = log.wait_for(r"^notification centre: (\d+) records", 10.0, after=before)
    time.sleep(1.0)
    save(screenshots, "notification-centre")
    print(
        f"shell: the clock opened Notification Centre: {'yes' if panel else 'NO'}, "
        f"{records.group(1) if records else '?'} records"
    )
    if panel is None:
        failures.append("clicking the menu bar's clock did not open Notification Centre")
    elif records is None or int(records.group(1)) < 1:
        failures.append("Notification Centre does not list the notification posted on the pipe")
    tap(VK_ESCAPE)
    if panel is not None and log.wait_for(r"^layer rmac-notification-center closed", 5.0, after=before) is None:
        failures.append("Esc did not close Notification Centre")


#: Apps must show its tiles this soon after the request once the shell has
#: read the catalogue (the first open above warmed it). The time includes
#: starting `lulo-shell.exe --app-drawer` and the 5 ms poll below.
APPS_OPEN_BUDGET_SECONDS = 0.30


def check_apps_open_speed(log: Log, bin_dir: Path, environment: dict, failures: list[str]) -> None:
    """Apps opens fast: the time from asking the running shell for Apps to
    the window's `apps ready` trace, three opens after the first."""
    pattern = re.compile(r"^apps ready \d+ apps(?:, (\d+) placeholders)?")
    times = []
    placeholders = []
    # A request toggles: one that finds Apps still open closes it, so an open
    # that never reports is retried after closing whatever is there.
    for _ in range(8):
        if len(times) == 4:
            break
        before = len(log.lines())
        started = time.monotonic()
        subprocess.run([str(bin_dir / "lulo-shell.exe"), "--app-drawer"], env=environment, timeout=30)
        elapsed = None
        while time.monotonic() - started < 3.0:
            ready = next((m for m in map(pattern.search, log.lines()[before:]) if m), None)
            if ready:
                elapsed = time.monotonic() - started
                placeholders.append(int(ready.group(1) or 0))
                break
            time.sleep(0.005)
        tap(VK_ESCAPE)
        time.sleep(0.8)
        if elapsed is not None:
            times.append(elapsed)
    if len(times) < 4:
        failures.append("apps speed: Apps did not report its tiles")
        return
    print(f"shell: apps tiles with a placeholder icon: {placeholders}")
    if any(placeholders):
        failures.append(f"apps look: {max(placeholders)} tiles show a placeholder or generic icon, not 0")
    print("shell: apps open to tiles (ms): " + ", ".join(f"{t * 1000:.0f}" for t in times))
    warm = sorted(times[1:])[len(times[1:]) // 2]
    if warm > APPS_OPEN_BUDGET_SECONDS:
        failures.append(
            f"apps speed: Apps took {warm * 1000:.0f} ms to show its tiles, over the "
            f"{APPS_OPEN_BUDGET_SECONDS * 1000:.0f} ms budget"
        )


DOCK_TILE = re.compile(r"^dock tile (\S+) .* at (-?\d+),(-?\d+) (\w+)$")


def dock_app_id(app_id: str) -> str:
    return app_id.removesuffix(".desktop").lower()


def dock_groups(log: Log) -> tuple[list[tuple[str, int, int]], list[tuple[str, int, int]]]:
    """The Dock's kept apps and its running apps, from its latest tile
    trace (`dock tile <id> <name> at x,y <state>` per tile, then Trash).
    The separator between them shows as a wider step between tiles."""
    blocks: list[list[tuple[str, int, int]]] = []
    current: list[tuple[str, int, int]] = []
    for line in log.lines():
        if line.startswith("dock tile trash at"):
            blocks.append(current)
            current = []
            continue
        match = DOCK_TILE.match(line)
        if match:
            current.append((dock_app_id(match.group(1)), int(match.group(2)), int(match.group(3))))
    tiles = blocks[-1] if blocks else []
    if len(tiles) < 2:
        return tiles, []
    steps = [tiles[index + 1][1] - tiles[index][1] for index in range(len(tiles) - 1)]
    pitch = min(steps)
    for index, step in enumerate(steps):
        if step > pitch * 1.3:
            return tiles[: index + 1], tiles[index + 1 :]
    return tiles, []


def slow_drag(start: tuple[int, int], end: tuple[int, int], hold: float = 0.4) -> None:
    """A press, a smooth move and a held release, as a hand drags a Dock
    icon (SendInput through SetCursorPos and mouse_event)."""
    user32().SetCursorPos(*start)
    time.sleep(0.2)
    user32().mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.1)
    steps = 30
    for step in range(1, steps + 1):
        x = start[0] + (end[0] - start[0]) * step // steps
        y = start[1] + (end[1] - start[1]) * step // steps
        user32().SetCursorPos(x, y)
        time.sleep(0.025)
    time.sleep(hold)
    user32().mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.3)


def kept_index(log: Log, app_id: str) -> int | None:
    kept, _ = dock_groups(log)
    return next((index for index, (tile, _, _) in enumerate(kept) if tile == app_id), None)


def between(tiles: list[tuple[str, int, int]], left: int) -> tuple[int, int]:
    return (tiles[left][1] + tiles[left + 1][1]) // 2, tiles[left][2]


def check_dock_drag(
    log: Log, bin_dir: Path, environment: dict, screenshots: Path | None, failures: list[str]
) -> None:
    clock_id = "org.rmac.clock"
    clock = bin_dir / "rmac-clock.exe"
    if not clock.exists():
        print("shell: dock drag: rmac-clock.exe is not in this build; skipped")
        return
    process = subprocess.Popen([str(clock)], env=environment)
    try:
        running = wait_until(
            lambda: any(tile == clock_id for tile, _, _ in dock_groups(log)[1]), 30.0
        )
        if not running:
            failures.append("dock drag: Clock never showed in the Dock's running apps")
            return
        time.sleep(1.0)
        # Drag the running Clock left across the separator, between the
        # first two kept apps: it is kept second.
        kept, others = dock_groups(log)
        tile = next(tile for tile in others if tile[0] == clock_id)
        slow_drag((tile[1], tile[2]), between(kept, 0))
        if wait_until(lambda: kept_index(log, clock_id) == 1, 10.0) is None:
            failures.append(
                f"dock drag: a running Clock dragged between the first two kept apps was not kept "
                f"second: {[tile for tile, _, _ in dock_groups(log)[0]]}"
            )
            return
        print("shell: dock drag: running Clock dragged across the separator is kept second")
        save(screenshots, "dock-drag-kept")
        # The Dock shows a drop's order at once, and takes no new drag until
        # the saved settings confirm it.
        time.sleep(2.5)
        # Reorder: Files, Clock, Apps, Notes… → between Apps and Notes.
        kept, _ = dock_groups(log)
        tile = kept[1]
        slow_drag((tile[1], tile[2]), between(kept, 2))
        if wait_until(lambda: kept_index(log, clock_id) == 2, 10.0) is None:
            failures.append(
                f"dock drag: dragging the kept Clock one place right did not move it: "
                f"{[tile for tile, _, _ in dock_groups(log)[0]]}"
            )
        else:
            print("shell: dock drag: the kept Clock reorders by dragging")
        # Drag it off the Dock and hold: released there it is no longer
        # kept, and as it runs it stays among the running apps.
        time.sleep(2.5)
        kept, _ = dock_groups(log)
        tile = next((tile for tile in kept if tile[0] == clock_id), None)
        if tile is not None:
            slow_drag((tile[1], tile[2]), (tile[1], tile[2] - 260), hold=1.0)
        removed = wait_until(
            lambda: kept_index(log, clock_id) is None
            and any(tile == clock_id for tile, _, _ in dock_groups(log)[1]),
            10.0,
        )
        if removed is None:
            failures.append("dock drag: Clock dragged off the Dock stayed kept or left the Dock")
        else:
            print("shell: dock drag: Clock dragged off the Dock is removed and stays running")
    finally:
        process.kill()
        process.wait(timeout=10)
    # An app dragged out of Apps between the first two kept apps is kept
    # second.
    kept, _ = dock_groups(log)
    if len(kept) < 2:
        failures.append("dock drag: the Dock has fewer than two kept apps")
        return
    before = len(log.lines())
    # Open Apps as its desktop entry does: a request to the running shell.
    subprocess.run([str(bin_dir / "lulo-shell.exe"), "--app-drawer"], env=environment, timeout=30)
    if log.wait_for(r"^apps first tile ", 15.0, after=before) is None:
        failures.append("dock drag: Apps did not open")
        return
    # Drag the first app in Apps (alphabetical) between the first two kept
    # apps: it is kept second.
    first = log.last(r"^apps first tile (\S+) at (-?\d+),(-?\d+)")
    if first is None:
        failures.append("dock drag: Apps showed no first tile")
        tap(VK_ESCAPE)
        return
    app_id = dock_app_id(first.group(1))
    was_kept = kept_index(log, app_id)
    time.sleep(0.8)
    slow_drag((int(first.group(2)), int(first.group(3))), between(kept, 0), hold=0.6)
    kept_second = wait_until(lambda: kept_index(log, app_id) == 1, 10.0)
    save(screenshots, "dock-drag-from-apps")
    tap(VK_ESCAPE)
    time.sleep(0.5)
    if kept_second is None:
        failures.append(
            f"dock drag: {app_id} dragged out of Apps between the first two kept apps was not "
            f"kept second: {[tile for tile, _, _ in dock_groups(log)[0]]}"
        )
        return
    print(f"shell: dock drag: {app_id} dragged out of Apps is kept where it was dropped")
    if was_kept is not None:
        return
    # Leave the Dock as it was: drag it off again.
    time.sleep(2.5)
    kept, _ = dock_groups(log)
    tile = kept[1]
    slow_drag((tile[1], tile[2]), (tile[1], tile[2] - 260), hold=1.0)
    if wait_until(lambda: kept_index(log, app_id) is None, 10.0) is None:
        failures.append(f"dock drag: {app_id} dragged off the Dock stayed kept")


def check_memory_after_use(log: Log, shell_pid: int, measurements: dict, failures: list[str]) -> None:
    """lulo-shell's memory 30 s after one use of Spotlight and a menu, once
    both have let go of their windows (`LULO_PANEL_RELEASE_SECONDS`)."""
    mark = len(log.lines())
    key(VK_MENU)
    tap(VK_SPACE)
    key(VK_MENU, up=True)
    if log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 10.0, after=mark):
        time.sleep(0.6)
        type_text("notes")
        time.sleep(0.8)
        # The first Escape clears the query, the second closes Spotlight.
        tap(VK_ESCAPE)
        if log.wait_for(r"^layer rmac-launcher closed", 1.0, after=mark) is None:
            tap(VK_ESCAPE)
    title = log.last(r"^bar title 0 \S+ at (-?\d+),(-?\d+)$")
    if title is not None:
        time.sleep(0.5)
        click(int(title.group(1)), int(title.group(2)))
        log.wait_for(r"^menu 0 row ", 5.0, after=mark)
        time.sleep(0.6)
        tap(VK_ESCAPE)
    released = log.wait_for(r"^layer rmac-launcher closed", 20.0, after=mark)
    menu_released = log.wait_for(r"^layer rmac-menu-material-\S+ closed", 20.0, after=mark)
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
        title = log.last(r"^bar title 0 \S+ at (-?\d+),(-?\d+)$")
        if title is not None:
            time.sleep(2.0)
            row = None
            for _ in range(3):
                mark = len(log.lines())
                click(int(title.group(1)), int(title.group(2)))
                row = log.wait_for(r"^menu 0 row Use Files for Folders at (\d+),(\d+)", 5.0, after=mark)
                if row:
                    break
                tap(VK_ESCAPE)
                time.sleep(1.0)
            if row:
                time.sleep(0.8)
                save(screenshots, "lulo-menu-files-for-folders")
                click(int(row.group(1)), int(row.group(2)))
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
            shown = log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 10.0, after=mark)
            time.sleep(0.8)
            front_pid, front_class = foreground_pid_and_class()
            save(screenshots, "spotlight-fallback")
            print(f"shell: {label} -> Spotlight {'shown' if shown else 'not shown'}; in front: pid {front_pid} {front_class}")
            if shown is None:
                failures.append(f"{label} did not open Spotlight")
            elif front_pid != shell_pid:
                failures.append(f"Spotlight opened on {label} but {front_class} (pid {front_pid}) has the keyboard")
            tap(VK_ESCAPE)
            log.wait_for(r"^layer rmac-launcher closed", 5.0, after=mark)
            time.sleep(0.5)

        # The menu bar's magnifier always opens Spotlight.
        icon = log.last(r"^bar spotlight at (-?\d+),(-?\d+)$")
        if icon is None:
            failures.append("the bar did not report its Spotlight icon")
        else:
            mark = len(log.lines())
            click(int(icon.group(1)), int(icon.group(2)))
            shown = log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 10.0, after=mark)
            time.sleep(0.5)
            save(screenshots, "spotlight-icon")
            print(f"shell: the bar's Spotlight icon {'opened' if shown else 'did not open'} Spotlight")
            if shown is None:
                failures.append("clicking the bar's Spotlight icon did not open Spotlight")
            tap(VK_ESCAPE)
            log.wait_for(r"^layer rmac-launcher closed", 5.0, after=mark)

        # The other app still has Alt+Space: Lulo never took it.
        presses = holder.presses()
        mark = len(log.lines())
        key(VK_MENU)
        tap(VK_SPACE)
        key(VK_MENU, up=True)
        got = wait_until(lambda: holder.presses() > presses, 5.0)
        stolen = log.wait_for(r"^layer rmac-launcher at (-?\d+),(-?\d+),(-?\d+),(-?\d+)", 1.0, after=mark)
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
