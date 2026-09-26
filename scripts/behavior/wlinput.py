#!/usr/bin/env python3
"""A tiny wtype-like Wayland input injector, in pure Python.

It speaks the Wayland wire protocol directly and binds
``zwp_virtual_keyboard_manager_v1`` and ``zwlr_virtual_pointer_manager_v1``,
which wlroots compositors (the headless Sway the behaviour suite nests)
offer. Nothing is compiled and nothing beyond the standard library and
libxkbcommon (through ctypes, for the keymap text) is needed.

It refuses to connect to the owner's live session: see ``assert_nested``.
The behaviour runner imports it; ``python3 wlinput.py key cmd-shift-n`` and
``python3 wlinput.py type hello`` also work by hand inside a nested
compositor.
"""

from __future__ import annotations

import array
import ctypes
import ctypes.util
import os
import socket
import struct
import sys
import time
from typing import Callable, Optional

LIVE_WAYLAND_DISPLAYS = {"wayland-1"}
LIVE_RUNTIME_DIRS = {"/run/user/1000"}


class InjectorError(RuntimeError):
    pass


def assert_nested(environ: dict[str, str]) -> None:
    """Refuse to inject into the owner's live session.

    Input may only go to a compositor this suite started: its runtime dir
    is a fresh temporary directory, never /run/user/<uid>, and its socket is
    never the live session's wayland-1.
    """

    display = environ.get("WAYLAND_DISPLAY", "")
    runtime = os.path.realpath(environ.get("XDG_RUNTIME_DIR", "")) if environ.get("XDG_RUNTIME_DIR") else ""
    if not display or not runtime:
        raise InjectorError("WAYLAND_DISPLAY and XDG_RUNTIME_DIR must name the nested compositor")
    if display in LIVE_WAYLAND_DISPLAYS or os.path.basename(display) in LIVE_WAYLAND_DISPLAYS:
        raise InjectorError(f"refusing to inject into {display}: that is the live session")
    if runtime in LIVE_RUNTIME_DIRS or runtime.startswith("/run/user/"):
        raise InjectorError(f"refusing to inject with XDG_RUNTIME_DIR={runtime}: that is the live session")
    if environ.get("RMAC_BEHAVIOR_NESTED") != "1":
        raise InjectorError("RMAC_BEHAVIOR_NESTED=1 is set only by run_lulo.py's nested compositor")


# --------------------------------------------------------------------------
# Key names: GPUI-style chords ("cmd-shift-n"), Mac glyphs (⇧⌘N) also parse.
# ⌘ is Super on Lulo (docs/decisions/0017-mac-keyboard.md), ⌥ Alt, ⌃ Control.
# --------------------------------------------------------------------------

KEY_LEFTSHIFT = 42
KEY_LEFTCTRL = 29
KEY_LEFTALT = 56
KEY_LEFTMETA = 125

MODIFIER_KEYS = {
    "shift": (KEY_LEFTSHIFT, 1 << 0),
    "ctrl": (KEY_LEFTCTRL, 1 << 2),
    "alt": (KEY_LEFTALT, 1 << 3),
    "cmd": (KEY_LEFTMETA, 1 << 6),
}
MODIFIER_ALIASES = {
    "shift": "shift", "⇧": "shift",
    "ctrl": "ctrl", "control": "ctrl", "⌃": "ctrl",
    "alt": "alt", "option": "alt", "opt": "alt", "⌥": "alt",
    "cmd": "cmd", "command": "cmd", "super": "cmd", "⌘": "cmd",
}

_LETTERS = "qwertyuiop"
_ROW2 = "asdfghjkl"
_ROW3 = "zxcvbnm"
KEYCODES: dict[str, int] = {}
for _i, _c in enumerate(_LETTERS):
    KEYCODES[_c] = 16 + _i
for _i, _c in enumerate(_ROW2):
    KEYCODES[_c] = 30 + _i
for _i, _c in enumerate(_ROW3):
    KEYCODES[_c] = 44 + _i
for _i, _c in enumerate("1234567890"):
    KEYCODES[_c] = 2 + _i
KEYCODES.update({
    "escape": 1, "esc": 1, "-": 12, "minus": 12, "=": 13, "equal": 13,
    "backspace": 14, "delete": 14, "tab": 15, "[": 26, "]": 27,
    "return": 28, "enter": 28, ";": 39, "'": 40, "`": 41, "\\": 43,
    ",": 51, ".": 52, "/": 53, "space": 57, " ": 57,
    "f1": 59, "f2": 60, "f3": 61, "f4": 62, "f5": 63,
    "home": 102, "up": 103, "pageup": 104, "left": 105, "right": 106,
    "end": 107, "down": 108, "pagedown": 109, "forwarddelete": 111,
    "kpenter": 96,
})
# Characters that need Shift on a US layout.
SHIFTED = {
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
    "*": "8", "(": "9", ")": "0", "_": "-", "+": "=", "{": "[", "}": "]",
    ":": ";", '"': "'", "~": "`", "|": "\\", "<": ",", ">": ".", "?": "/",
}
GLYPH_KEYS = {"⌫": "backspace", "⌦": "forwarddelete", "↩": "return", "⎋": "escape",
              "←": "left", "→": "right", "↑": "up", "↓": "down", "⇥": "tab"}


def parse_chord(chord: str) -> tuple[list[str], int]:
    """Parse "cmd-shift-n" or "⇧⌘N" into (modifiers, evdev keycode)."""

    chord = chord.strip()
    mods: list[str] = []
    if any(glyph in chord for glyph in "⇧⌘⌥⌃"):
        rest = chord
        while rest and rest[0] in "⇧⌘⌥⌃":
            mods.append(MODIFIER_ALIASES[rest[0]])
            rest = rest[1:]
        key = GLYPH_KEYS.get(rest, rest).lower()
    else:
        parts = chord.split("-")
        # "cmd--" is Command-minus.
        if chord.endswith("--"):
            parts = chord[:-2].split("-") + ["-"]
        key = parts[-1].lower()
        for part in parts[:-1]:
            alias = MODIFIER_ALIASES.get(part.lower())
            if alias is None:
                raise InjectorError(f"unknown modifier {part!r} in {chord!r}")
            mods.append(alias)
        key = GLYPH_KEYS.get(key, key)
    if key in SHIFTED:
        mods.append("shift")
        key = SHIFTED[key]
    if key not in KEYCODES:
        raise InjectorError(f"unknown key {key!r} in {chord!r}")
    ordered = [m for m in ("ctrl", "alt", "shift", "cmd") if m in mods]
    return ordered, KEYCODES[key]


def text_to_strokes(text: str) -> list[tuple[list[str], int]]:
    strokes = []
    for char in text:
        if char == "\n":
            strokes.append(([], KEYCODES["return"]))
        elif char.isascii() and char.isalpha():
            strokes.append((["shift"] if char.isupper() else [], KEYCODES[char.lower()]))
        elif char in SHIFTED:
            strokes.append((["shift"], KEYCODES[SHIFTED[char]]))
        elif char in KEYCODES:
            strokes.append(([], KEYCODES[char]))
        else:
            raise InjectorError(f"cannot type {char!r} with the US keymap")
    return strokes


# --------------------------------------------------------------------------
# Wayland wire protocol
# --------------------------------------------------------------------------


def _pad(data: bytes) -> bytes:
    return data + b"\0" * ((4 - len(data) % 4) % 4)


def _string(value: str) -> bytes:
    raw = value.encode() + b"\0"
    return struct.pack("<I", len(raw)) + _pad(raw)


def _fixed(value: float) -> int:
    return int(round(value * 256)) & 0xFFFFFFFF


def keymap_text() -> bytes:
    """The default (us) XKB keymap, as text, from libxkbcommon."""

    path = ctypes.util.find_library("xkbcommon") or "libxkbcommon.so.0"
    xkb = ctypes.CDLL(path)
    xkb.xkb_context_new.restype = ctypes.c_void_p
    xkb.xkb_keymap_new_from_names.restype = ctypes.c_void_p
    xkb.xkb_keymap_new_from_names.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int]
    xkb.xkb_keymap_get_as_string.restype = ctypes.c_void_p
    xkb.xkb_keymap_get_as_string.argtypes = [ctypes.c_void_p, ctypes.c_int]
    libc = ctypes.CDLL(None)
    libc.free.argtypes = [ctypes.c_void_p]
    context = xkb.xkb_context_new(0)
    if not context:
        raise InjectorError("xkb_context_new failed")
    keymap = xkb.xkb_keymap_new_from_names(context, None, 0)
    if not keymap:
        raise InjectorError("xkb_keymap_new_from_names failed")
    pointer = xkb.xkb_keymap_get_as_string(keymap, 1)
    text = ctypes.string_at(pointer)
    libc.free(pointer)
    return text + b"\0"


class Wayland:
    """A minimal client: one registry, a seat, a virtual keyboard and a
    virtual pointer."""

    def __init__(self, environ: Optional[dict[str, str]] = None) -> None:
        environ = dict(os.environ if environ is None else environ)
        assert_nested(environ)
        display = environ["WAYLAND_DISPLAY"]
        path = display if display.startswith("/") else os.path.join(environ["XDG_RUNTIME_DIR"], display)
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)
        self.next_id = 2
        self.buffer = b""
        self.globals: dict[str, tuple[int, int]] = {}
        self.handlers: dict[int, Callable[[int, bytes], None]] = {}
        self.registry = self._new_id()
        self._send(1, 1, struct.pack("<I", self.registry))
        self.handlers[self.registry] = self._on_registry
        self.roundtrip()
        for interface in ("wl_seat", "zwp_virtual_keyboard_manager_v1", "zwlr_virtual_pointer_manager_v1"):
            if interface not in self.globals:
                raise InjectorError(f"the nested compositor does not offer {interface}")
        self.seat = self._bind("wl_seat", 1)
        keyboard_manager = self._bind("zwp_virtual_keyboard_manager_v1", 1)
        pointer_manager = self._bind("zwlr_virtual_pointer_manager_v1", 1)
        self.keyboard = self._new_id()
        self._send(keyboard_manager, 0, struct.pack("<II", self.seat, self.keyboard))
        self.pointer = self._new_id()
        self._send(pointer_manager, 0, struct.pack("<II", self.seat, self.pointer))
        text = keymap_text()
        fd = os.memfd_create("rmac-behavior-keymap", 0)
        os.write(fd, text)
        os.lseek(fd, 0, os.SEEK_SET)
        self._send(self.keyboard, 0, struct.pack("<II", 1, len(text)), fds=[fd])
        os.close(fd)
        self.started = time.monotonic()
        self.roundtrip()

    # -- plumbing ----------------------------------------------------------

    def _new_id(self) -> int:
        value = self.next_id
        self.next_id += 1
        return value

    def _send(self, object_id: int, opcode: int, payload: bytes, fds: Optional[list[int]] = None) -> None:
        header = struct.pack("<II", object_id, ((8 + len(payload)) << 16) | opcode)
        message = header + payload
        if fds:
            self.sock.sendmsg([message], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))])
        else:
            self.sock.sendall(message)

    def _bind(self, interface: str, version: int) -> int:
        name, advertised = self.globals[interface]
        new = self._new_id()
        payload = struct.pack("<I", name) + _string(interface) + struct.pack("<II", min(version, advertised), new)
        self._send(self.registry, 0, payload)
        return new

    def _on_registry(self, opcode: int, body: bytes) -> None:
        if opcode != 0:
            return
        name, length = struct.unpack_from("<II", body, 0)
        interface = body[8:8 + length - 1].decode()
        (version,) = struct.unpack_from("<I", body, 8 + len(_pad(body[8:8 + length])))
        self.globals.setdefault(interface, (name, version))

    def _dispatch(self) -> None:
        while len(self.buffer) >= 8:
            object_id, word = struct.unpack_from("<II", self.buffer, 0)
            size, opcode = word >> 16, word & 0xFFFF
            if len(self.buffer) < size:
                return
            body = self.buffer[8:size]
            self.buffer = self.buffer[size:]
            if object_id == 1 and opcode == 0:
                _obj, code = struct.unpack_from("<II", body, 0)
                (length,) = struct.unpack_from("<I", body, 8)
                message = body[12:12 + length - 1].decode(errors="replace")
                raise InjectorError(f"Wayland protocol error {code}: {message}")
            handler = self.handlers.get(object_id)
            if handler:
                handler(opcode, body)

    def roundtrip(self) -> None:
        callback = self._new_id()
        done = []
        self.handlers[callback] = lambda opcode, body: done.append(True)
        self._send(1, 0, struct.pack("<I", callback))
        self.sock.settimeout(5)
        while not done:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise InjectorError("the nested compositor closed the connection")
            self.buffer += chunk
            self._dispatch()
        del self.handlers[callback]

    def _time(self) -> int:
        return int((time.monotonic() - self.started) * 1000) & 0xFFFFFFFF

    # -- keyboard ----------------------------------------------------------

    def _modifiers(self, mask: int) -> None:
        self._send(self.keyboard, 2, struct.pack("<IIII", mask, 0, 0, 0))

    def _key(self, keycode: int, pressed: bool) -> None:
        self._send(self.keyboard, 1, struct.pack("<III", self._time(), keycode, 1 if pressed else 0))

    def stroke(self, mods: list[str], keycode: int, hold: float = 0.02) -> None:
        mask = 0
        for mod in mods:
            code, bit = MODIFIER_KEYS[mod]
            self._key(code, True)
            mask |= bit
            self._modifiers(mask)
        self._key(keycode, True)
        self.roundtrip()
        time.sleep(hold)
        self._key(keycode, False)
        for mod in reversed(mods):
            code, bit = MODIFIER_KEYS[mod]
            self._key(code, False)
            mask &= ~bit
            self._modifiers(mask)
        self.roundtrip()

    def key(self, chord: str) -> None:
        mods, code = parse_chord(chord)
        self.stroke(mods, code)

    def type_text(self, text: str, delay: float = 0.03) -> None:
        for mods, code in text_to_strokes(text):
            self.stroke(mods, code)
            time.sleep(delay)

    # -- pointer -----------------------------------------------------------

    def move(self, x: float, y: float, width: int, height: int) -> None:
        self._send(self.pointer, 1, struct.pack("<IIIII", self._time(), _fixed(x), _fixed(y), width, height))
        self._send(self.pointer, 4, b"")
        self.roundtrip()

    def button(self, pressed: bool, button: str = "left") -> None:
        code = {"left": 0x110, "right": 0x111, "middle": 0x112}[button]
        self._send(self.pointer, 2, struct.pack("<III", self._time(), code, int(pressed)))
        self._send(self.pointer, 4, b"")
        self.roundtrip()

    def drag(self, start: tuple[float, float], end: tuple[float, float], width: int, height: int,
             button: str = "left", steps: int = 8) -> None:
        self.move(*start, width, height)
        time.sleep(0.05)
        self.button(True, button)
        for step in range(1, steps + 1):
            fraction = step / steps
            self.move(start[0] + (end[0] - start[0]) * fraction,
                      start[1] + (end[1] - start[1]) * fraction, width, height)
            time.sleep(0.04)
        self.button(False, button)

    def click(self, x: float, y: float, width: int, height: int, button: str = "left", count: int = 1) -> None:
        code = {"left": 0x110, "right": 0x111, "middle": 0x112}[button]
        self.move(x, y, width, height)
        time.sleep(0.05)
        for _ in range(count):
            self._send(self.pointer, 2, struct.pack("<III", self._time(), code, 1))
            self._send(self.pointer, 4, b"")
            self.roundtrip()
            time.sleep(0.03)
            self._send(self.pointer, 2, struct.pack("<III", self._time(), code, 0))
            self._send(self.pointer, 4, b"")
            self.roundtrip()
            time.sleep(0.06)

    def close(self) -> None:
        try:
            self._send(self.keyboard, 3, b"")
            self._send(self.pointer, 8, b"")
            self.roundtrip()
        finally:
            self.sock.close()


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[0] not in {"key", "type"}:
        print("usage: wlinput.py key CHORD… | type TEXT", file=sys.stderr)
        return 2
    client = Wayland()
    try:
        if argv[0] == "key":
            for chord in argv[1:]:
                client.key(chord)
        else:
            client.type_text(" ".join(argv[1:]))
    finally:
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
