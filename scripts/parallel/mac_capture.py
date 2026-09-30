"""Fast Quartz screenshots for response measurements on macOS."""

from __future__ import annotations

import ctypes
import ctypes.util
import hashlib
from pathlib import Path

from PIL import Image


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


class CGSize(ctypes.Structure):
    _fields_ = [("width", ctypes.c_double), ("height", ctypes.c_double)]


class CGRect(ctypes.Structure):
    _fields_ = [("origin", CGPoint), ("size", CGSize)]


api = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))
api.CGWindowListCreateImage.argtypes = [CGRect, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
api.CGWindowListCreateImage.restype = ctypes.c_void_p
for function in ("CGImageGetWidth", "CGImageGetHeight", "CGImageGetBytesPerRow"):
    getattr(api, function).argtypes = [ctypes.c_void_p]
    getattr(api, function).restype = ctypes.c_size_t
api.CGImageGetDataProvider.argtypes = [ctypes.c_void_p]
api.CGImageGetDataProvider.restype = ctypes.c_void_p
api.CGDataProviderCopyData.argtypes = [ctypes.c_void_p]
api.CGDataProviderCopyData.restype = ctypes.c_void_p
api.CFDataGetBytePtr.argtypes = [ctypes.c_void_p]
api.CFDataGetBytePtr.restype = ctypes.c_void_p
api.CFDataGetLength.argtypes = [ctypes.c_void_p]
api.CFDataGetLength.restype = ctypes.c_long
api.CFRelease.argtypes = [ctypes.c_void_p]
api.CGMainDisplayID.restype = ctypes.c_uint32
api.CGDisplayBounds.argtypes = [ctypes.c_uint32]
api.CGDisplayBounds.restype = CGRect


def grab(region: tuple[int, int, int, int] | None = None) -> Image.Image:
    if region is None:
        rect = api.CGDisplayBounds(api.CGMainDisplayID())
    else:
        rect = CGRect(CGPoint(region[0], region[1]), CGSize(region[2], region[3]))
    image = api.CGWindowListCreateImage(rect, 1, 0, 0)
    if not image:
        raise RuntimeError("Quartz could not capture the target region")
    try:
        width, height = api.CGImageGetWidth(image), api.CGImageGetHeight(image)
        stride = api.CGImageGetBytesPerRow(image)
        data = api.CGDataProviderCopyData(api.CGImageGetDataProvider(image))
        try:
            pixels = ctypes.string_at(api.CFDataGetBytePtr(data), api.CFDataGetLength(data))
            converted = Image.frombytes("RGBA", (width, height), pixels, "raw", "BGRA", stride, 1)
            return converted.convert("RGB")
        finally:
            api.CFRelease(data)
    finally:
        api.CFRelease(image)


def capture(path: Path, region: tuple[int, int, int, int] | None = None) -> None:
    grab(region).save(path, compress_level=0)


def fingerprint(region: tuple[int, int, int, int] | None = None) -> str:
    image = grab(region).convert("L").resize((128, 96), Image.Resampling.BILINEAR)
    return hashlib.blake2s(image.tobytes(), digest_size=12).hexdigest()
