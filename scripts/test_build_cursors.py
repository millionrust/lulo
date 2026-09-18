"""Guards the original rmac cursor theme output (FEEL_SPEC.md §D.2)."""

from __future__ import annotations

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "build_cursors", Path(__file__).with_name("build-cursors.py")
)
assert _SPEC is not None and _SPEC.loader is not None
build_cursors = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(build_cursors)

XCURSOR_MAGIC = 0x72756358
IMAGE_TYPE = 0xFFFD0002


def _header(data: bytes) -> tuple[int, int, int]:
    magic, header_size, _version, ntoc = struct.unpack("<IIII", data[:16])
    assert magic == XCURSOR_MAGIC, "not an Xcursor file"
    return header_size, ntoc, 16


def _first_image_header(data: bytes) -> tuple[int, int, int, int, int, int, int]:
    _magic, _size, _version, ntoc = struct.unpack("<IIII", data[:16])
    assert ntoc >= 1
    _type, _subtype, position = struct.unpack("<III", data[16:28])
    header_size, image_type, nominal, _ver, width, height, _xh, _yh, delay = (
        struct.unpack("<IIIIIIIII", data[position : position + 36])
    )
    return header_size, image_type, nominal, width, height, delay, ntoc


class CursorBuildTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._tmp = tempfile.TemporaryDirectory()
        cls.theme = Path(cls._tmp.name)
        build_cursors.build(cls.theme)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._tmp.cleanup()

    def test_static_cursor_is_a_valid_four_size_xcursor(self) -> None:
        header_size, ntoc, _ = _header((self.theme / "default").read_bytes())
        self.assertEqual(header_size, 16)
        self.assertEqual(ntoc, 4)
        image = _first_image_header((self.theme / "default").read_bytes())
        self.assertEqual(image[0], 36)  # image header size, no length prefix
        self.assertEqual(image[1], IMAGE_TYPE)
        self.assertEqual(image[3], image[4])  # square
        self.assertEqual(image[5], 0)  # not animated

    def test_busy_cursor_is_animated_twelve_frames_per_size(self) -> None:
        data = (self.theme / "wait").read_bytes()
        _header_size, ntoc, _ = _header(data)
        self.assertEqual(ntoc, 48)  # 4 sizes × 12 frames
        self.assertEqual(_first_image_header(data)[5], 60)  # 60 ms per frame

    def test_theme_inherits_adwaita_and_leaves_the_hand_to_it(self) -> None:
        self.assertIn("Inherits=Adwaita", (self.theme / "index.theme").read_text())
        self.assertIn("Inherits=Adwaita", (self.theme / "cursor.theme").read_text())
        # The pointing hand is inherited, not aliased to the arrow.
        self.assertFalse((self.theme / "hand2").exists())
        self.assertTrue((self.theme / "default").exists())


if __name__ == "__main__":
    unittest.main()
