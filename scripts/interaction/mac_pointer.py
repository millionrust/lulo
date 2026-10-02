#!/usr/bin/env python3
"""Move the real mouse pointer without clicking (macOS only), for hover probes.

    python3 mac_pointer.py X Y

mac_click.py posts a mouse-down+up pair and warps the cursor back to where
it was afterwards; a hover probe instead needs the pointer to genuinely
rest at a point so AppKit's NSTrackingArea hover state activates, and to
stay there until the caller has compared a screenshot. This posts one real
kCGEventMouseMoved (not a warp, which delivers no event an app can react
to) and leaves the pointer there.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import sys


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


KCGEVENTMOUSEMOVED = 5


def move(x: float, y: float) -> None:
    quartz = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))
    quartz.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    quartz.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32]
    quartz.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    quartz.CFRelease.argtypes = [ctypes.c_void_p]
    event = quartz.CGEventCreateMouseEvent(None, KCGEVENTMOUSEMOVED, CGPoint(x, y), 0)
    if not event:
        raise RuntimeError("CGEventCreateMouseEvent failed")
    try:
        quartz.CGEventPost(0, event)
    finally:
        quartz.CFRelease(event)


if __name__ == "__main__":
    move(float(sys.argv[1]), float(sys.argv[2]))
