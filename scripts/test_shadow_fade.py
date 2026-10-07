"""Unit tests for scripts/behavior/shadow_fade.py's panel-shadow check.

Builds small synthetic captures with PIL (no compositor, no laptop) so the
fade/hard-edge distinction itself is verified directly: a smooth, analytic
falloff passes, and the two ways a clipped shadow can fail -- a single hard
step, and a flat band that never reaches the background within the sampled
margin -- are each reproduced and caught.
"""

from __future__ import annotations

import importlib.util
import math
import sys
import unittest
from pathlib import Path

from PIL import Image

SCRIPT = Path(__file__).parent / "behavior" / "shadow_fade.py"
SPEC = importlib.util.spec_from_file_location("shadow_fade", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
shadow_fade = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = shadow_fade
SPEC.loader.exec_module(shadow_fade)


WIDTH, HEIGHT = 360, 360
BACKGROUND = 230
PANEL = 40
BOX = (100, 100, 120, 90)  # x, y, w, h: panel from (100,100) to (220,190)


def _fill(width: int, height: int, value) -> Image.Image:
    image = Image.new("RGB", (width, height))
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            level = value(x, y)
            pixels[x, y] = (level, level, level)
    return image


def _smooth_capture() -> Image.Image:
    """A panel whose shadow decays exponentially toward the background --
    the shape a real Gaussian-ish drop shadow actually falls off in."""

    x, y, w, h = BOX

    def level(px: int, py: int) -> int:
        if x <= px < x + w and y <= py < y + h:
            return PANEL
        # Distance outside the panel's nearest edge, 0 at the edge.
        dx = max(x - px, px - (x + w - 1), 0)
        dy = max(y - py, py - (y + h - 1), 0)
        distance = max(dx, dy)
        decay = (BACKGROUND - PANEL) * (1 - math.exp(-distance / 12.0))
        return min(BACKGROUND, round(PANEL + decay))

    return _fill(WIDTH, HEIGHT, level)


def _edge_then_smooth_capture() -> Image.Image:
    """A panel whose own opaque content gives way to its shadow over just a
    couple of pixels -- a real, legitimate step, since a blur's value right
    against a solid edge is already close to its darkest -- and only then
    decays smoothly toward the background. This is the shape an actual
    nested-session capture of the fixed confirmation dialog showed (a jump
    of ~40-45 in the first couple of device pixels, then a smooth climb
    over the next ~100 px): `check_panel_shadow`'s `skip` must not flag
    this as a hard edge."""

    x, y, w, h = BOX
    entry = PANEL + 42  # the shade the shadow starts at, right past the edge

    def level(px: int, py: int) -> int:
        if x <= px < x + w and y <= py < y + h:
            return PANEL
        dx = max(x - px, px - (x + w - 1), 0)
        dy = max(y - py, py - (y + h - 1), 0)
        distance = max(dx, dy)
        decay = (BACKGROUND - entry) * (1 - math.exp(-distance / 30.0))
        return min(BACKGROUND, round(entry + decay))

    return _fill(WIDTH, HEIGHT, level)


def _clipped_capture() -> Image.Image:
    """A panel whose shadow is a flat mid-tone band for a short run past
    its edge, then cuts straight to the background -- the surface-too-small
    artifact this check exists to catch (docs/parity.md SESSION-09)."""

    x, y, w, h = BOX
    band = 10
    mid = (PANEL + BACKGROUND) // 2

    def level(px: int, py: int) -> int:
        if x <= px < x + w and y <= py < y + h:
            return PANEL
        dx = max(x - px, px - (x + w - 1), 0)
        dy = max(y - py, py - (y + h - 1), 0)
        distance = max(dx, dy)
        if distance == 0:
            return PANEL
        if distance <= band:
            return mid
        return BACKGROUND

    return _fill(WIDTH, HEIGHT, level)


def _stuck_capture() -> Image.Image:
    """A panel whose shadow sits at a constant mid-tone -- no hard step
    anywhere in the sampled band, so the first check alone would not catch
    it -- because the surface clipped it there instead of letting it fall
    off: an opaque slab rather than a fading blur (docs/parity.md
    SESSION-07's "black shadow/slab" report). The true background only
    resumes well past the sampled band, like the real wallpaper resuming
    past a backdrop window's edge, so a background sample taken further out
    still reads the real background rather than the stuck shade."""

    x, y, w, h = BOX
    floor = PANEL + 70
    reach = 80  # comfortably past CHECK_BAND, short of CHECK_BAND + 40

    def level(px: int, py: int) -> int:
        if x <= px < x + w and y <= py < y + h:
            return PANEL
        dx = max(x - px, px - (x + w - 1), 0)
        dy = max(y - py, py - (y + h - 1), 0)
        distance = max(dx, dy)
        return floor if distance <= reach else BACKGROUND

    return _fill(WIDTH, HEIGHT, level)


class ShadowFadeTests(unittest.TestCase):
    def test_luminance_is_rec709(self):
        self.assertAlmostEqual(shadow_fade.luminance((255, 255, 255)), 255.0, places=3)
        self.assertAlmostEqual(shadow_fade.luminance((0, 0, 0)), 0.0, places=3)
        self.assertAlmostEqual(shadow_fade.luminance((0, 255, 0)), 255 * 0.7152, places=3)

    def test_a_sharp_edge_right_against_the_panel_then_smooth_decay_passes(self):
        image = _edge_then_smooth_capture()
        below, right = shadow_fade.check_panel_shadow(image, BOX, band=60, step=2)
        self.assertTrue(below.ok, below.detail)
        self.assertTrue(right.ok, right.detail)

    def test_smooth_analytic_falloff_passes(self):
        image = _smooth_capture()
        below, right = shadow_fade.check_panel_shadow(image, BOX, band=60, step=2)
        self.assertTrue(below.ok, below.detail)
        self.assertTrue(right.ok, right.detail)

    def test_flat_band_with_a_hard_cutoff_fails(self):
        image = _clipped_capture()
        below, right = shadow_fade.check_panel_shadow(image, BOX, band=60, step=2)
        self.assertFalse(below.ok)
        self.assertIn("hard edge", below.detail)
        self.assertFalse(right.ok)
        self.assertIn("hard edge", right.detail)

    def test_shadow_that_never_reaches_the_background_fails(self):
        image = _stuck_capture()
        below, right = shadow_fade.check_panel_shadow(image, BOX, band=60, step=2)
        self.assertFalse(below.ok)
        self.assertIn("not faded", below.detail)
        self.assertFalse(right.ok)
        self.assertIn("not faded", right.detail)

    def test_check_fade_requires_at_least_two_samples(self):
        report = shadow_fade.check_fade([200.0], background=230.0)
        self.assertFalse(report.ok)
        self.assertIn("fewer than 2 samples", report.detail)

    def test_sample_column_and_row_stop_at_image_edges(self):
        image = _smooth_capture()
        column = shadow_fade.sample_column(image, 90, HEIGHT - 3, count=10, step=1)
        self.assertEqual(len(column), 3)
        row = shadow_fade.sample_row(image, 90, WIDTH - 3, count=10, step=1)
        self.assertEqual(len(row), 3)


if __name__ == "__main__":
    unittest.main()
