"""The cross-platform shell comparison (scripts/compare_shell_scenes.py)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import compare_shell_scenes  # noqa: E402

try:
    from PIL import Image, ImageDraw
except ImportError:  # pragma: no cover - CI installs Pillow
    Image = None


def scene(path: Path, dock_x: int = 300, bar_colour=(30, 30, 30), text_shift: int = 0) -> Path:
    image = Image.new("RGB", (400, 300), (225, 235, 210))
    draw = ImageDraw.Draw(image)
    draw.rectangle((0, 0, 399, 29), fill=(235, 240, 230))
    draw.text((10 + text_shift, 8), "Files  File  Edit", fill=bar_colour)
    draw.rounded_rectangle((dock_x - 120, 230, dock_x - 20, 290), radius=12, fill=(245, 245, 245))
    image.save(path)
    return path


@unittest.skipIf(Image is None, "Pillow is not installed")
class CompareShellScenes(unittest.TestCase):
    def test_identical_scenes_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            first = scene(Path(directory) / "linux.png")
            second = scene(Path(directory) / "windows.png")
            _, _, _, scores = compare_shell_scenes.compare(first, second, 1.5, 32)
            self.assertEqual(compare_shell_scenes.failures(scores, 6.0, 0.03), [])
            self.assertEqual(scores["screen"]["mean"], 0.0)

    def test_a_moved_dock_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            first = scene(Path(directory) / "linux.png")
            second = scene(Path(directory) / "windows.png", dock_x=380)
            _, _, _, scores = compare_shell_scenes.compare(first, second, 1.5, 32)
            problems = compare_shell_scenes.failures(scores, 1.0, 0.01)
            self.assertTrue(any(problem.startswith("dock") for problem in problems), problems)

    def test_one_pixel_text_offset_is_tolerated(self):
        with tempfile.TemporaryDirectory() as directory:
            first = scene(Path(directory) / "linux.png")
            second = scene(Path(directory) / "windows.png", text_shift=1)
            _, _, _, scores = compare_shell_scenes.compare(first, second, 1.5, 32)
            self.assertEqual(compare_shell_scenes.failures(scores, 6.0, 0.03), [])

    def test_the_diff_sheet_shows_both_screens_and_the_differences(self):
        with tempfile.TemporaryDirectory() as directory:
            first = scene(Path(directory) / "linux.png")
            second = scene(Path(directory) / "windows.png", bar_colour=(200, 0, 0))
            linux, windows, difference, _ = compare_shell_scenes.compare(first, second, 1.5, 32)
            sheet = compare_shell_scenes.diff_image(linux, windows, difference)
            self.assertEqual(sheet.size, (1200, 300))


if __name__ == "__main__":
    unittest.main()
