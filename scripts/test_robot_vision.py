"""The live robot's screenshot heuristics (scripts/robot/vision.py and
shell_tour.find_clusters) on synthetic images."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "robot"))

try:
    from PIL import Image, ImageDraw
except ImportError:  # pragma: no cover - CI installs Pillow
    Image = None

if Image is not None:
    import shell_tour  # noqa: E402
    import vision  # noqa: E402


@unittest.skipIf(Image is None, "Pillow is not installed")
class VisionTests(unittest.TestCase):
    def window(self, body=(30, 30, 30), text=None):
        image = Image.new("RGB", (400, 300), (40, 120, 60))
        draw = ImageDraw.Draw(image)
        draw.rectangle((50, 40, 349, 259), fill=body)
        draw.rectangle((50, 40, 349, 80), fill=(60, 60, 60))
        for x in (62, 80, 98):
            draw.ellipse((x - 6, 54, x + 6, 66), fill=(250, 90, 80))
        if text:
            draw.text((60, 92), text, fill=text[1] if isinstance(text, tuple) else (230, 230, 230))
        return image

    def test_a_flat_body_is_blank_and_one_line_of_text_is_not(self):
        box = (50, 40, 350, 260)
        self.assertEqual([p["kind"] for p in vision.blank_window(self.window(), box)], ["blank-window"])
        image = self.window()
        ImageDraw.Draw(image).text((60, 92), "jacob@Jake:~$", fill=(230, 230, 230))
        self.assertEqual(vision.blank_window(image, box), [])

    def test_a_black_body_is_reported_as_black(self):
        kinds = [p["kind"] for p in vision.blank_window(self.window(body=(0, 0, 0)), (50, 40, 350, 260))]
        self.assertEqual(kinds, ["black-window"])

    def test_light_text_on_light_background_is_low_contrast(self):
        image = Image.new("RGB", (200, 80), (235, 235, 235))
        ImageDraw.Draw(image).text((10, 30), "Hard to read text here", fill=(215, 215, 215))
        self.assertEqual([p["kind"] for p in vision.low_contrast(image, (0, 0, 200, 80))], ["low-contrast"])
        dark = Image.new("RGB", (200, 80), (235, 235, 235))
        ImageDraw.Draw(dark).text((10, 30), "Easy to read text here", fill=(20, 20, 20))
        self.assertEqual(vision.low_contrast(dark, (0, 0, 200, 80)), [])

    def test_text_cut_by_the_right_edge_is_flagged(self):
        image = Image.new("RGB", (200, 120), (250, 250, 250))
        draw = ImageDraw.Draw(image)
        for y in range(25, 100, 6):  # glyph stems running into the edge
            draw.rectangle((150, y, 199, y + 2), fill=(10, 10, 10))
        self.assertIn("edge-clipping", [p["kind"] for p in vision.edge_ink(image, (0, 0, 200, 120))])
        clean = Image.new("RGB", (200, 120), (250, 250, 250))
        self.assertEqual(vision.edge_ink(clean, (0, 0, 200, 120)), [])

    def test_residue_ignores_the_clock_but_not_a_leftover_panel(self):
        before = Image.new("RGB", (300, 200), (20, 80, 40))
        after = before.copy()
        ImageDraw.Draw(after).rectangle((250, 0, 299, 10), fill=(255, 255, 255))  # clock tick
        self.assertEqual(vision.residue(before, after, [(240, 0, 300, 12)]), [])
        ImageDraw.Draw(after).rectangle((20, 40, 120, 120), fill=(200, 200, 200))  # ghost menu
        self.assertEqual([p["kind"] for p in vision.residue(before, after, [(240, 0, 300, 12)])], ["residue"])

    def test_clusters_find_status_icons_in_a_bar(self):
        image = Image.new("RGB", (400, 30), (30, 30, 30))
        draw = ImageDraw.Draw(image)
        for left in (100, 160, 220):
            draw.rectangle((left, 8, left + 16, 22), fill=(240, 240, 240))
        spans = shell_tour.find_clusters(image, (0, 0, 400, 30))
        self.assertEqual(len(spans), 3)
        self.assertTrue(all(right - left >= 16 for left, right in spans))


if __name__ == "__main__":
    unittest.main()
