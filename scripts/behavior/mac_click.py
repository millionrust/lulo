#!/usr/bin/env python3
"""Post one mouse click at a screen point through Quartz (macOS only).

    python3 mac_click.py right X Y

record_mac.py uses it for "context" steps, because Finder's AXShowMenu
action does not open a menu that the AX API can then read. The pointer is
put back where it was afterwards.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import sys
import time


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


def click(button: str, x: float, y: float) -> None:
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

    here = quartz.CGEventCreate(None)
    previous = quartz.CGEventGetLocation(here)
    quartz.CFRelease(here)
    down, up, number = {"left": (1, 2, 0), "right": (3, 4, 1)}[button]
    point = CGPoint(x, y)
    for kind in (down, up):
        event = quartz.CGEventCreateMouseEvent(None, kind, point, number)
        quartz.CGEventPost(0, event)
        quartz.CFRelease(event)
        time.sleep(0.05)
    time.sleep(0.3)
    quartz.CGWarpMouseCursorPosition(previous)


if __name__ == "__main__":
    click(sys.argv[1], float(sys.argv[2]), float(sys.argv[3]))
