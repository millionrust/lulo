#!/usr/bin/env python3
"""Post one mouse click at a screen point through Quartz (macOS only).

    python3 mac_click.py right X Y
    python3 mac_click.py left X Y --count 2          # double-click
    python3 mac_click.py left X Y --shift             # shift-click
    python3 mac_click.py left X Y --cmd                # command-click

record_mac.py uses it for "context" steps, because Finder's AXShowMenu
action does not open a menu that the AX API can then read, and for
"select" steps that need a real click (shift-click, command-click,
double-click) rather than a Finder "select" Apple Event, which only sets
selection state and never opens a rename edit or a folder. The pointer is
put back where it was afterwards.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import sys
import time


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


# CGEventFlags bits (ApplicationServices/CGEventTypes.h).
FLAG_SHIFT = 1 << 17
FLAG_COMMAND = 1 << 20
# CGEventField.kCGMouseEventClickState: 1 for a single click, 2 for the
# second click of a double-click (must be set on both the down and up of
# that click so the receiving app recognises it as one).
FIELD_CLICK_STATE = 1


def click(button: str, x: float, y: float, count: int = 1, shift: bool = False, cmd: bool = False) -> None:
    quartz = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))
    quartz.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    quartz.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32]
    quartz.CGEventCreate.restype = ctypes.c_void_p
    quartz.CGEventCreate.argtypes = [ctypes.c_void_p]
    quartz.CGEventGetLocation.restype = CGPoint
    quartz.CGEventGetLocation.argtypes = [ctypes.c_void_p]
    quartz.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    quartz.CFRelease.argtypes = [ctypes.c_void_p]
    quartz.CGWarpMouseCursorPosition.argtypes = [CGPoint]
    quartz.CGEventSetFlags.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
    quartz.CGEventSetIntegerValueField.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int64]

    here = quartz.CGEventCreate(None)
    previous = quartz.CGEventGetLocation(here)
    quartz.CFRelease(here)
    down, up, number = {"left": (1, 2, 0), "right": (3, 4, 1)}[button]
    point = CGPoint(x, y)
    flags = (FLAG_SHIFT if shift else 0) | (FLAG_COMMAND if cmd else 0)
    for click_number in range(1, count + 1):
        for kind in (down, up):
            event = quartz.CGEventCreateMouseEvent(None, kind, point, number)
            if flags:
                quartz.CGEventSetFlags(event, flags)
            if count > 1:
                quartz.CGEventSetIntegerValueField(event, FIELD_CLICK_STATE, click_number)
            quartz.CGEventPost(0, event)
            quartz.CFRelease(event)
            time.sleep(0.05)
        if click_number < count:
            time.sleep(0.08)  # well inside the system double-click interval
    time.sleep(0.3)
    quartz.CGWarpMouseCursorPosition(previous)


if __name__ == "__main__":
    args = sys.argv[1:]
    flags_only = {a for a in args if a.startswith("--")}
    positional = [a for a in args if not a.startswith("--")]
    count = 1
    if "--count" in args:
        count = int(args[args.index("--count") + 1])
    click(
        positional[0], float(positional[1]), float(positional[2]),
        count=count, shift="--shift" in flags_only, cmd="--cmd" in flags_only,
    )
