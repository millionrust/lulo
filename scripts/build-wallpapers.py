#!/usr/bin/env python3
"""Render the Lulo OS wallpaper collection.

Every image is original procedural artwork: layered fields computed with
numpy, composited in linear light, then dithered to 8 bits. Nothing is traced,
sampled or derived from any other operating system's wallpapers.

The lulo (naranjilla) gives the collection its palette: an orange skin around
translucent green flesh that falls into four segments from the centre. The
hero wallpaper abstracts that cross-section into four soft, curved, luminous
lobes; the quieter wallpapers reuse the same colours as fields and waves.

    python3 scripts/build-wallpapers.py                 # every size, into packaging/
    python3 scripts/build-wallpapers.py --preview out/  # 960x540 previews only
    python3 scripts/build-wallpapers.py --only lulo --size 1920x1080 --out /tmp/x

Output: packaging/rmac-session/wallpapers/<id>-<light|dark>-<W>x<H>.jpg plus a
320x200 <id>-<light|dark>-thumbnail.jpg for the System Settings gallery. The
build is deterministic: each wallpaper seeds its own random generator.

Palette (sRGB), documented in docs/wallpaper.md:

    Lulo Orange  #F08A24   peel, the warm accent
    Peel Deep    #C9541A   shaded peel, ember
    Peel Light   #FFC27A   lit peel in light variants
    Lulo Green   #8CC63F   translucent flesh, lit
    Flesh Deep   #2E6B3A   flesh in shadow
    Forest       #0E2A1F   darkest green, night grounds
    Mist         #F3F0EA   warm neutral light ground
    Stone        #CFC8BD   warm neutral mid
    Graphite     #1B1D20   neutral dark ground
    Ink          #08090B   near-black
"""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "packaging" / "rmac-session" / "wallpapers"

# Packaged sizes: 4K 16:9, a 16:10 laptop size, and 1080p so a 1080p panel
# never decodes a 4K image. The runtime picks the smallest one that covers
# the output (rmac-wallpaper-image).
SIZES = ((3840, 2160), (2560, 1600), (1920, 1080))
THUMBNAIL = (320, 200)
JPEG_QUALITY = 94
DARK_GRAIN = 0.013

# ---------------------------------------------------------------- colour math


def srgb_to_linear(c: np.ndarray) -> np.ndarray:
    c = np.asarray(c, dtype=np.float32)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c: np.ndarray) -> np.ndarray:
    c = np.clip(c, 0.0, None)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)


def hexc(value: str) -> np.ndarray:
    value = value.lstrip("#")
    rgb = np.array([int(value[i : i + 2], 16) / 255 for i in (0, 2, 4)], np.float32)
    return srgb_to_linear(rgb).astype(np.float32)


def smoothstep(e0: float, e1: float, x: np.ndarray) -> np.ndarray:
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def mix(a, b, t):
    t = np.asarray(t, dtype=np.float32)
    if t.ndim == 2:
        t = t[..., None]
    return a + (b - a) * t


def ramp(stops: list[tuple[float, str]], t: np.ndarray) -> np.ndarray:
    """Colour ramp over t in [0, 1] through a cubic Hermite spline in linear
    light. The slope is continuous across stops, so no stop reads as a band
    (a per-segment smoothstep flattens at every stop and shows Mach bands)."""
    t = np.clip(t, 0.0, 1.0).astype(np.float32)
    positions = [p for p, _ in stops]
    colours = [hexc(c) for _, c in stops]
    count = len(stops)
    tangents = []
    for i in range(count):
        lo, hi = max(0, i - 1), min(count - 1, i + 1)
        tangents.append((colours[hi] - colours[lo]) / (positions[hi] - positions[lo]))
    out = np.zeros(t.shape + (3,), np.float32)
    out[...] = colours[0]
    for i in range(count - 1):
        p0, p1 = positions[i], positions[i + 1]
        span = p1 - p0
        u = np.clip((t - p0) / span, 0.0, 1.0)[..., None]
        u2, u3 = u * u, u * u * u
        value = (
            (2 * u3 - 3 * u2 + 1) * colours[i]
            + (u3 - 2 * u2 + u) * span * tangents[i]
            + (-2 * u3 + 3 * u2) * colours[i + 1]
            + (u3 - u2) * span * tangents[i + 1]
        )
        out = np.where((t >= p0)[..., None], value, out)
    return np.clip(out, 0.0, None)


# ------------------------------------------------------------------- canvas


@dataclass
class Canvas:
    width: int
    height: int
    seed: int

    def __post_init__(self) -> None:
        self.rng = np.random.default_rng(self.seed)
        # y spans [0, 1]; x spans [0, aspect]. Artwork is authored on a 16:9
        # frame and centred, so 16:10 crops show a little more height.
        self.aspect = self.width / self.height
        ys = (np.arange(self.height, dtype=np.float32) + 0.5) / self.height
        xs = (np.arange(self.width, dtype=np.float32) + 0.5) / self.height
        design = 16 / 9
        xs = xs - (self.aspect - design) / 2
        self.x, self.y = np.meshgrid(xs, ys)
        self.design_w = design

    def noise(self, cells: float, octaves: int = 1, seed_offset: int = 0) -> np.ndarray:
        """Smooth value noise in [-1, 1]; `cells` is the feature count across the height."""
        rng = np.random.default_rng(self.seed * 7919 + seed_offset)
        total = np.zeros((self.height, self.width), np.float32)
        amplitude, norm = 1.0, 0.0
        for octave in range(octaves):
            n = cells * (2**octave)
            gh = max(2, int(np.ceil(n)) + 3)
            gw = max(2, int(np.ceil(n * self.aspect)) + 3)
            grid = rng.standard_normal((gh, gw)).astype(np.float32)
            image = Image.fromarray(grid, mode="F").resize(
                (self.width, self.height), Image.BICUBIC
            )
            total += amplitude * np.asarray(image, np.float32)
            norm += amplitude
            amplitude *= 0.5
        total /= norm
        return np.clip(total / 1.6, -1.0, 1.0)


def blur(field: np.ndarray, factor: int) -> np.ndarray:
    """Cheap large-radius blur: shrink then grow with bicubic filtering."""
    h, w = field.shape
    small = Image.fromarray(field.astype(np.float32), mode="F").resize(
        (max(2, w // factor), max(2, h // factor)), Image.BOX
    )
    return np.asarray(small.resize((w, h), Image.BICUBIC), np.float32)


def finish(canvas: Canvas, linear: np.ndarray, grain: float) -> Image.Image:
    """Tone, add film grain and triangular dither, quantise to 8-bit sRGB."""
    srgb = linear_to_srgb(linear)
    rng = np.random.default_rng(canvas.seed + 1)
    h, w = canvas.height, canvas.width
    if grain > 0:
        # Luminance grain, strongest in the mid-tones, slightly soft so it
        # reads as film rather than sensor noise.
        g = rng.standard_normal((h, w)).astype(np.float32)
        g = 0.75 * g + 0.25 * blur(g, 2)
        luma = srgb.mean(axis=2)
        # Keep the grain strong in deep shadows: JPEG smooths faint noise
        # away there first, which would bring 8-bit steps back.
        weight = 0.85 + 0.15 * (1 - np.abs(luma * 2 - 1))
        srgb = srgb + (grain * weight * g)[..., None]
    # Triangular-PDF dither of +-1 LSB per channel removes 8-bit banding.
    tpdf = (rng.random((h, w, 3), dtype=np.float32) - rng.random((h, w, 3), dtype=np.float32))
    out = np.floor(np.clip(srgb, 0, 1) * 255 + 0.5 + tpdf)
    return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), mode="RGB")


# ---------------------------------------------------------------- artworks
#
# Each artwork is f(canvas, dark) -> linear RGB (H, W, 3). Positions use the
# canvas frame: y in [0, 1] top to bottom, x in [0, 16/9] (c.design_w).


def tonemap(x: np.ndarray, white: float = 1.0) -> np.ndarray:
    """Soft shoulder so additive light never clips harshly."""
    return x * (1 + x / (white * white)) / (1 + x)


def over(image: np.ndarray, colour: np.ndarray, alpha: np.ndarray) -> np.ndarray:
    a = np.clip(alpha, 0, 1)[..., None]
    return image * (1 - a) + colour * a


def glow(image: np.ndarray, colour: str, amount: np.ndarray) -> np.ndarray:
    return image + hexc(colour) * amount[..., None]


def field(c: Canvas, ground: np.ndarray, lights, warp: float, seed: int, mode: str) -> np.ndarray:
    """Large, gently warped gaussian lights over a ground.
    lights: (x, y, rx, ry, colour, amount) with x as a fraction of the width."""
    wx = c.noise(0.7, 2, 300 + seed) * warp
    wy = c.noise(0.7, 2, 400 + seed) * warp
    image = ground
    for x, y, rx, ry, colour, amount in lights:
        dx = (c.x + wx - x * c.design_w) / rx
        dy = (c.y + wy - y) / ry
        g = np.exp(-0.5 * (dx * dx + dy * dy))
        image = glow(image, colour, g * amount) if mode == "add" else over(image, hexc(colour), g * amount)
    return image


def _segments(r, phi, wob, fan, width0, bend0, edge_width, reach):
    """Curved translucent sheets fanning from one heart, one per segment."""
    out = []
    for k, base in enumerate(fan):
        width = width0 + 0.02 * k
        bend = bend0 - 0.04 * k
        s = (phi + bend * r + 0.008 * wob - base) / width
        body = smoothstep(-1.6, 0.85, s) * (1 - smoothstep(0.88, 1.0, s))
        edge = np.exp(-((s - 0.9) ** 2) / (2 * edge_width**2))
        halo = np.exp(-((s - 0.84) ** 2) / (2 * 0.09**2)) * (s < 1.0)
        length = smoothstep(0.05, 0.45, r) * (
            1 - smoothstep(reach[0] + 0.12 * k, reach[1] + 0.1 * k, r)
        )
        out.append((body * length, edge * length, halo * length, s, length))
    return out


def lulo(c: Canvas, dark: bool) -> np.ndarray:
    """Hero: four curved sheets of translucent green light fan out from a glowing
    heart below the lower-right corner, each lit along its edge in peel orange,
    with a fainter set of sheets behind for depth."""
    dx, dy = c.x - c.design_w * 1.02, c.y - 1.10
    r = np.sqrt(dx * dx + dy * dy)
    phi = np.arctan2(-dy, -dx)  # 0 points left, pi/2 points up
    wob = c.noise(0.6, 1, 3)
    along = np.clip(r / 1.8, 0, 1)
    t = np.clip((c.x / c.design_w) * 0.45 + (1 - c.y) * 0.55, 0, 1)
    heart = np.exp(-(r**2) / (2 * 0.38**2))
    core = np.exp(-(r**2) / (2 * 0.18**2))
    front = _segments(r, phi, wob, (0.02, 0.36, 0.70, 1.04), 0.17, -0.20, 0.028, (1.15, 2.1))
    back = _segments(r, phi, wob, (0.19, 0.53, 0.87), 0.20, -0.12, 0.05, (1.5, 2.6))

    if dark:
        image = ramp([(0.0, "#0E2119"), (0.55, "#0A1712"), (1.0, "#060D0B")], t)
        for body, edge, _, _, _ in back:
            image = glow(image, "#2F7D4A", body * 0.10)
            image = glow(image, "#F08A24", edge * 0.10)
        light = np.zeros_like(image)
        cover = np.zeros(r.shape, np.float32)
        for body, edge, halo, _, _ in front:
            flesh = ramp(
                [(0.0, "#E4FF8C"), (0.3, "#86C846"), (0.7, "#2F7D4A"), (1.0, "#1A5238")],
                along * 0.9 + (1 - body) * 0.3,
            )
            rim = ramp([(0.0, "#FFD27A"), (1.0, "#F08A24")], along)
            light += flesh * (body * 0.55)[..., None] + rim * (edge * 0.9 + halo * 0.18)[..., None]
            cover = np.maximum(cover, body)
        image = image * (1 - 0.35 * cover[..., None]) + tonemap(light, 1.6)
        image = glow(image, "#FF962E", heart * 0.45)
        image = glow(image, "#FFF2C4", core * 0.6)
        return glow(image, "#F08A24", np.exp(-(r**2) / (2 * 0.95**2)) * 0.04)

    image = ramp([(0.0, "#EDE3D2"), (0.55, "#F5EEE3"), (1.0, "#FBF8F2")], t)
    for body, edge, _, _, _ in back:
        image = over(image, hexc("#D9ECB4"), body * 0.22)
        image = over(image, hexc("#F7C48A"), edge * 0.25)
    for body, edge, _, s, length in front:
        flesh = ramp(
            [(0.0, "#F2FFC4"), (0.35, "#C4E57E"), (0.75, "#98CC58"), (1.0, "#86BE52")],
            along * 0.9 + (1 - body) * 0.25,
        )
        shade = 1 - 0.08 * (1 - smoothstep(-1.2, 0.6, s))
        image = over(image, flesh * shade[..., None], body * 0.72)
        soft = np.exp(-((s - 0.88) ** 2) / (2 * 0.05**2)) * smoothstep(0.05, 0.45, r)
        image = over(image, hexc("#F7A850"), soft * 0.45)
        image = over(image, ramp([(0.0, "#FFE2A8"), (1.0, "#F59A3C")], along), edge * 0.55)
    image = over(image, hexc("#FFD7A0"), heart * 0.45)
    return over(image, hexc("#FFFBEE"), core * 0.55)


def grove(c: Canvas, dark: bool) -> np.ndarray:
    """Three long, overlapping green sheets with soft shading and a faint lit lip."""
    xn = c.x / c.design_w
    t = np.clip(c.y * 0.8 + xn * 0.2, 0, 1)
    if dark:
        image = ramp([(0.0, "#0A1813"), (1.0, "#07110D")], t)
        sheets = [
            (0.34, 0.10, 1.6, 0.4, -0.10, "#1C5A3E", "#0B1F16", "#B8E26A", 0.08),
            (0.56, 0.12, 1.3, 2.4, 0.08, "#2A7A4C", "#0D2419", "#C8EC8A", 0.10),
            (0.78, 0.09, 1.8, 4.2, -0.06, "#3E9150", "#0E2A1F", "#DDF5A8", 0.12),
        ]
    else:
        image = ramp([(0.0, "#F4F7EE"), (1.0, "#EEF3E5")], t)
        sheets = [
            (0.34, 0.10, 1.6, 0.4, -0.10, "#DDEBCB", "#CFE2B8", "#FFFFFF", 0.06),
            (0.56, 0.12, 1.3, 2.4, 0.08, "#C4DEA4", "#B2D38E", "#FAFFF2", 0.07),
            (0.78, 0.09, 1.8, 4.2, -0.06, "#A6CC7C", "#93BE68", "#F5FFE6", 0.08),
        ]
    for base, amp, wavelength, phase, tilt, lit, deep, lip, lip_amount in sheets:
        top = base + tilt * (xn - 0.5) + amp * np.sin(2 * np.pi * xn / wavelength + phase)
        d = c.y - top
        body = smoothstep(-0.003, 0.012, d)
        fill = ramp([(0.0, lit), (1.0, deep)], np.clip(d / 0.35, 0, 1) ** 0.7)
        shadow = np.exp(-np.maximum(-d, 0) / 0.05) * (d < 0)
        image = image * (1 - (0.35 if dark else 0.10) * shadow)[..., None]
        image = over(image, fill, body)
        image = glow(image, lip, np.exp(-((d - 0.004) ** 2) / (2 * 0.006**2)) * lip_amount)
    return image


def ember(c: Canvas, dark: bool) -> np.ndarray:
    """A warm glow rising from below the lower right, ringed like cut fruit."""
    cx, cy = c.design_w * 0.80, 1.18
    rr = np.sqrt(((c.x - cx) / 1.25) ** 2 + (c.y - cy) ** 2) + 0.02 * c.noise(0.6, 2, 17)
    # Ring spacing grows outward: crowded near the core, open at the edges.
    rings = (0.5 + 0.5 * np.cos(2 * np.pi * np.sqrt(rr / 0.018))) ** 3
    light = np.exp(-(rr**2) / (2 * 0.55**2))
    t = np.clip(c.y * 0.7 + (c.x / c.design_w) * 0.3, 0, 1)
    fall = np.clip(rr / 1.1, 0, 1)
    if dark:
        image = ramp([(0.0, "#120907"), (0.6, "#170B07"), (1.0, "#0E0605")], t)
        body = ramp([(0.0, "#FFB85C"), (0.25, "#F08A24"), (0.6, "#A8401A"), (1.0, "#2A0F07")], fall)
        image = image + body * (light * 0.85)[..., None]
        return glow(image, "#FFB45A", rings * light * 0.10)
    image = ramp([(0.0, "#FFF8F0"), (0.6, "#FDF1E4"), (1.0, "#FBEAD8")], t)
    body = ramp([(0.0, "#FFC98E"), (0.3, "#F7A260"), (0.7, "#F5C39C"), (1.0, "#FBEAD8")], fall)
    image = over(image, body, light)
    return over(image, hexc("#FFF3E2"), rings * light * 0.30)


def dusk(c: Canvas, dark: bool) -> np.ndarray:
    """A calm sky: deep teal overhead, warming to a low orange glow."""
    xn = c.x / c.design_w
    t = np.clip(c.y + 0.03 * c.noise(0.6, 2, 7) + 0.04 * np.sin(xn * 2.4 + 0.5), 0, 1)
    if dark:
        sky = ramp(
            [(0.0, "#061016"), (0.40, "#0B1D24"), (0.70, "#1D2527"), (0.88, "#4A2C1E"), (1.0, "#7A3A18")],
            t,
        )
        return field(c, sky, [
            (0.62, 1.10, 0.60, 0.20, "#F08A24", 0.45),
            (0.62, 1.08, 0.22, 0.08, "#FFC27A", 0.30),
            (0.15, 1.05, 0.40, 0.18, "#8A3A18", 0.25),
        ], 0.08, 3, "add")
    sky = ramp(
        [(0.0, "#D8E8E4"), (0.40, "#E6EFEA"), (0.70, "#F4EFE4"), (0.88, "#FAE0C4"), (1.0, "#F8C79A")],
        t,
    )
    return field(c, sky, [
        (0.62, 1.10, 0.60, 0.22, "#FFB870", 0.45),
        (0.62, 1.06, 0.22, 0.08, "#FFF1DC", 0.55),
    ], 0.08, 3, "over")


def mist(c: Canvas, dark: bool) -> np.ndarray:
    """Neutral: an almost flat warm field with soft, out-of-focus light."""
    t = np.clip(c.y * 0.6 + (c.x / c.design_w) * 0.4, 0, 1)
    if dark:
        ground = ramp([(0.0, "#232528"), (0.5, "#1B1D20"), (1.0, "#131416")], t)
        return field(c, ground, [
            (0.25, 0.25, 0.45, 0.35, "#3A3530", 0.40),
            (0.80, 0.75, 0.50, 0.35, "#1F2A26", 0.45),
            (0.55, 0.05, 0.40, 0.20, "#34322E", 0.25),
        ], 0.12, 4, "add")
    ground = ramp([(0.0, "#F7F5F1"), (0.5, "#EFECE6"), (1.0, "#E6E1D9")], t)
    return field(c, ground, [
        (0.25, 0.25, 0.45, 0.35, "#FBF6EC", 0.70),
        (0.80, 0.75, 0.50, 0.35, "#E5E7E0", 0.60),
        (0.55, 0.05, 0.40, 0.20, "#FFFFFF", 0.40),
    ], 0.12, 4, "over")


def nocturne(c: Canvas, dark: bool) -> np.ndarray:
    """Very dark and minimal: one faint arc of green-to-orange light low in the frame."""
    xn = c.x / c.design_w
    d = np.sqrt((c.x - c.design_w * 0.5) ** 2 + (c.y - 2.95) ** 2) - 2.20
    side = np.exp(-((xn - 0.5) ** 2) / (2 * 0.28**2))
    inside = smoothstep(0.0, -0.01, d)
    depth = np.clip(-d / 0.3, 0, 1)
    if dark:
        image = ramp([(0.0, "#07080A"), (0.7, "#090B0C"), (1.0, "#0B0D0D")], np.clip(c.y, 0, 1))
        hue = ramp([(0.0, "#2E6B3A"), (0.5, "#8CC63F"), (1.0, "#F08A24")], np.clip(xn * 1.1 - 0.05, 0, 1))
        atmosphere = np.exp(-np.maximum(d, 0) / 0.05) * (d > -0.002)
        rim = np.exp(-(d**2) / (2 * 0.0035**2))
        image = image + hue * (atmosphere * 0.10 * side + rim * 0.35 * side)[..., None]
        image = over(image, ramp([(0.0, "#0A0C0C"), (1.0, "#040505")], depth), inside)
        inner = np.exp(-(np.minimum(d, 0) ** 2) / (2 * 0.02**2)) * inside
        return image + hue * (inner * 0.04 * side)[..., None]
    image = ramp([(0.0, "#E9EAEB"), (0.7, "#E2E3E3"), (1.0, "#DCDDDC")], np.clip(c.y, 0, 1))
    hue = ramp([(0.0, "#9CCB63"), (0.5, "#CDE89A"), (1.0, "#F7B25E")], np.clip(xn * 1.1 - 0.05, 0, 1))
    atmosphere = np.exp(-np.maximum(d, 0) / 0.06) * (d > -0.002)
    image = over(image, hue, atmosphere * 0.6 * side)
    image = over(image, hexc("#FFFFFF"), np.exp(-(d**2) / (2 * 0.003**2)) * 0.6 * side)
    return over(image, ramp([(0.0, "#D3D5D4"), (1.0, "#C6C8C7")], depth), inside)


# id -> (title, renderer, grain, seed). Order is the System Settings order and
# must match rmac_wallpaper::BuiltInId::ALL.
ARTWORKS: dict[str, tuple[str, Callable[[Canvas, bool], np.ndarray], float, int]] = {
    "lulo": ("Lulo", lulo, 0.010, 1),
    "lulo-grove": ("Grove", grove, 0.008, 2),
    "lulo-ember": ("Ember", ember, 0.009, 3),
    "lulo-dusk": ("Dusk", dusk, 0.008, 4),
    "lulo-mist": ("Mist", mist, 0.007, 5),
    "lulo-nocturne": ("Nocturne", nocturne, 0.006, 6),
}


# ------------------------------------------------------------------- driver


def render(identifier: str, dark: bool, size: tuple[int, int]) -> Image.Image:
    _, renderer, grain, seed = ARTWORKS[identifier]
    canvas = Canvas(size[0], size[1], seed * 2 + (1 if dark else 0))
    linear = renderer(canvas, dark).astype(np.float32)
    # Dark grounds need more grain: at JPEG quality below ~95 the encoder
    # smooths a faint dither away and 8-bit steps reappear in the shadows.
    return finish(canvas, linear, max(grain, DARK_GRAIN) if dark else grain)


def parse_size(value: str) -> tuple[int, int]:
    width, height = value.lower().split("x")
    return int(width), int(height)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--only", action="append", choices=sorted(ARTWORKS))
    parser.add_argument("--size", action="append", type=parse_size)
    parser.add_argument("--preview", type=Path, help="write 960x540 PNG previews here")
    args = parser.parse_args(argv)
    identifiers = args.only or list(ARTWORKS)

    if args.preview:
        args.preview.mkdir(parents=True, exist_ok=True)
        for identifier in identifiers:
            for dark in (False, True):
                image = render(identifier, dark, (960, 540))
                name = f"{identifier}-{'dark' if dark else 'light'}.png"
                image.save(args.preview / name)
                print(args.preview / name)
        return 0

    args.out.mkdir(parents=True, exist_ok=True)
    sizes = args.size or list(SIZES)
    for identifier in identifiers:
        for dark in (False, True):
            appearance = "dark" if dark else "light"
            largest = None
            for size in sizes:
                image = render(identifier, dark, size)
                path = args.out / f"{identifier}-{appearance}-{size[0]}x{size[1]}.jpg"
                image.save(path, quality=JPEG_QUALITY, optimize=True, progressive=True, subsampling=0)
                print(f"{path.relative_to(ROOT) if path.is_relative_to(ROOT) else path}  {path.stat().st_size / 1e6:.2f} MB")
                if largest is None or size[0] * size[1] > largest.width * largest.height:
                    largest = image
            if not args.size:
                thumb = render(identifier, dark, (THUMBNAIL[0] * 2, THUMBNAIL[1] * 2))
                thumb = thumb.resize(THUMBNAIL, Image.LANCZOS)
                path = args.out / f"{identifier}-{appearance}-thumbnail.jpg"
                thumb.save(path, quality=88, optimize=True)
                print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
