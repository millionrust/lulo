"""Persistent wlroots screencopy client for high-rate nested Sway sampling.

Uses the already guarded Wayland connection from ``wlinput.Wayland``. Only
Sway's private output is bound; no live-session socket can be passed here.
Final review PNGs are still made by grim.
"""

from __future__ import annotations

import hashlib
import mmap
import os
import struct
import time

from PIL import Image


class Screencopy:
    def __init__(self, wire):
        self.wire = wire
        for interface in ("wl_shm", "wl_output", "zwlr_screencopy_manager_v1"):
            if interface not in wire.globals:
                raise RuntimeError(f"nested Sway lacks {interface}")
        self.shm = wire._bind("wl_shm", 1)
        self.output = wire._bind("wl_output", 1)
        self.manager = wire._bind("zwlr_screencopy_manager_v1", 1)
        wire.roundtrip()

    def image(self, region=None) -> Image.Image:
        wire = self.wire
        frame = wire._new_id()
        state = {"ready": False, "failed": False, "map": None, "buffer": None,
                 "width": None, "height": None, "stride": None, "format": None}

        def on_frame(opcode: int, body: bytes) -> None:
            if opcode == 0:  # buffer: format, width, height, stride
                fmt, width, height, stride = struct.unpack_from("<IIII", body)
                if fmt not in (0, 1) or width < 1 or height < 1 or width > 8192 or height > 8192:
                    state["failed"] = True
                    return
                size = stride * height
                fd = os.memfd_create("rmac-parallel-screencopy", 0)
                os.ftruncate(fd, size)
                mapping = mmap.mmap(fd, size)
                pool = wire._new_id()
                buffer = wire._new_id()
                wire._send(self.shm, 0, struct.pack("<II", pool, size), fds=[fd])
                wire._send(pool, 0, struct.pack("<IiiiiI", buffer, 0, width, height, stride, fmt))
                wire._send(pool, 1, b"")
                os.close(fd)
                state.update(map=mapping, buffer=buffer, width=width, height=height,
                             stride=stride, format=fmt)
                wire._send(frame, 0, struct.pack("<I", buffer))
            elif opcode == 2:  # ready
                state["ready"] = True
            elif opcode == 3:  # failed
                state["failed"] = True

        wire.handlers[frame] = on_frame
        if region:
            x, y, width, height = region
            wire._send(self.manager, 1, struct.pack("<IIIiiii", frame, 0, self.output,
                                                    x, y, width, height))
        else:
            wire._send(self.manager, 0, struct.pack("<III", frame, 0, self.output))
        deadline = time.monotonic() + 2.0
        try:
            while not (state["ready"] or state["failed"]):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError("nested screencopy frame timed out")
                wire.sock.settimeout(remaining)
                block = wire.sock.recv(65536)
                if not block:
                    raise RuntimeError("nested Sway closed the screencopy socket")
                wire.buffer += block
                wire._dispatch()
            if state["failed"] or state["map"] is None:
                raise RuntimeError("nested Sway rejected screencopy frame")
            # ARGB/XRGB8888 are BGRA/BGRX bytes in little-endian memory.
            raw = "BGRA" if state["format"] == 0 else "BGRX"
            image = Image.frombytes("RGBA" if raw == "BGRA" else "RGB",
                                    (state["width"], state["height"]), state["map"],
                                    "raw", raw, state["stride"], 1)
            return image.convert("RGB")
        finally:
            wire.handlers.pop(frame, None)
            if state["buffer"] is not None:
                wire._send(state["buffer"], 0, b"")
            wire._send(frame, 1, b"")
            if state["map"] is not None:
                state["map"].close()

    def fingerprint(self, region=None) -> str:
        image = self.image(region)
        gray = image.convert("L").resize((128, 96), Image.Resampling.BILINEAR)
        return hashlib.blake2s(gray.tobytes(), digest_size=12).hexdigest()
