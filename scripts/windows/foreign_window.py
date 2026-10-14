"""A third-party Win32 app for shell_smoke.py's window placement check.

    python scripts/windows/foreign_window.py

Opens two plain Win32 windows the way careless apps do and pumps messages
until it is terminated:

- "Lulo Foreign Window": an overlapped window at 0,0, 4000 × 4000, which
  Lulo must shrink into the work area (macOS's visible frame);
- "Lulo Foreign Full Screen": a borderless popup over the whole primary
  monitor, which Lulo must leave alone, like a full-screen game.
"""

from __future__ import annotations

import ctypes
import sys
from ctypes import wintypes

WS_OVERLAPPEDWINDOW = 0x00CF0000
WS_POPUP = 0x80000000
WS_VISIBLE = 0x10000000
SW_SHOW = 5
SM_CXSCREEN = 0
SM_CYSCREEN = 1

LRESULT = ctypes.c_ssize_t
WNDPROC = ctypes.WINFUNCTYPE(LRESULT, wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)


class WNDCLASSW(ctypes.Structure):
    _fields_ = [
        ("style", wintypes.UINT),
        ("lpfnWndProc", WNDPROC),
        ("cbClsExtra", ctypes.c_int),
        ("cbWndExtra", ctypes.c_int),
        ("hInstance", wintypes.HINSTANCE),
        ("hIcon", wintypes.HICON),
        ("hCursor", wintypes.HANDLE),
        ("hbrBackground", wintypes.HBRUSH),
        ("lpszMenuName", wintypes.LPCWSTR),
        ("lpszClassName", wintypes.LPCWSTR),
    ]


def main() -> int:
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    user32.DefWindowProcW.restype = LRESULT
    user32.DefWindowProcW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.CreateWindowExW.restype = wintypes.HWND
    user32.CreateWindowExW.argtypes = [
        wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
        ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
        wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID,
    ]

    @WNDPROC
    def procedure(hwnd, message, wparam, lparam):
        return user32.DefWindowProcW(hwnd, message, wparam, lparam)

    instance = kernel32.GetModuleHandleW(None)
    window_class = WNDCLASSW()
    window_class.lpfnWndProc = procedure
    window_class.hInstance = instance
    window_class.hbrBackground = 6  # COLOR_WINDOW + 1
    window_class.lpszClassName = "LuloForeignWindow"
    if not user32.RegisterClassW(ctypes.byref(window_class)):
        print("RegisterClassW failed", file=sys.stderr)
        return 1
    width, height = user32.GetSystemMetrics(SM_CXSCREEN), user32.GetSystemMetrics(SM_CYSCREEN)
    big = user32.CreateWindowExW(0, "LuloForeignWindow", "Lulo Foreign Window",
                                 WS_OVERLAPPEDWINDOW | WS_VISIBLE, 0, 0, 4000, 4000,
                                 None, None, instance, None)
    full = user32.CreateWindowExW(0, "LuloForeignWindow", "Lulo Foreign Full Screen",
                                  WS_POPUP | WS_VISIBLE, 0, 0, width, height,
                                  None, None, instance, None)
    if not big or not full:
        print("CreateWindowExW failed", file=sys.stderr)
        return 1
    user32.ShowWindow(big, SW_SHOW)
    user32.ShowWindow(full, SW_SHOW)
    print(f"foreign windows {big:#x} {full:#x}", flush=True)
    message = wintypes.MSG()
    while user32.GetMessageW(ctypes.byref(message), None, 0, 0) > 0:
        user32.TranslateMessage(ctypes.byref(message))
        user32.DispatchMessageW(ctypes.byref(message))
    return 0


if __name__ == "__main__":
    sys.exit(main())
