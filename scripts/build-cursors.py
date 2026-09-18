#!/usr/bin/env python3
"""Generate the original rmac Xcursor theme (FEEL_SPEC.md §D.2).

No SVG rasterizer or xcursorgen is assumed, so the shapes are drawn
programmatically with Pillow and written in the Xcursor binary format. Every
shape is original rmac artwork; the theme inherits Adwaita only for the
artistic cursors that are not drawn yet, so a missing name never breaks the
pointer.

Usage:
    python3 scripts/build-cursors.py [output_dir]

The default output is `assets/cursors/rmac`.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw

SUPERSAMPLE = 4
SIZES = (24, 32, 48, 64)

# Xcursor constants.
IMAGE_TYPE = 0xFFFD0002
XCURSOR_MAGIC = 0x72756358
XCURSOR_HEADER = 16
XCURSOR_VERSION = 0x00010000
IMAGE_HEADER = 36

BLACK = (0, 0, 0, 255)
WHITE = (255, 255, 255, 255)
SHADOW = (0, 0, 0, 64)


def _canvas(size: int) -> Image.Image:
    return Image.new("RGBA", (size * SUPERSAMPLE, size * SUPERSAMPLE), (0, 0, 0, 0))


def _downscale(image: Image.Image, size: int) -> Image.Image:
    return image.resize((size, size), Image.LANCZOS)


def _arrow(draw: ImageDraw.ImageDraw, s: float) -> None:
    """The rmac arrow: black fill, white outline, soft contact shadow."""
    points = [
        (0.10, 0.02),
        (0.10, 0.78),
        (0.28, 0.62),
        (0.40, 0.95),
        (0.56, 0.88),
        (0.44, 0.56),
        (0.66, 0.54),
    ]
    poly = [(x * s, y * s) for x, y in points]
    draw.polygon([(x + 0.03 * s, y + 0.03 * s) for x, y in poly], fill=SHADOW)
    draw.polygon(poly, fill=BLACK)
    draw.line(poly + [poly[0]], fill=WHITE, width=max(1, int(0.05 * s)), joint="curve")


def _double_arrow(image: Image.Image, s: float, angle: float) -> None:
    """A two-headed arrow rotated to `angle` degrees for resize/move."""
    layer = Image.new("RGBA", (int(s), int(s)), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    head = 0.22 * s
    width = max(1, int(0.09 * s))
    d.line([(0.5 * s, 0.12 * s), (0.5 * s, 0.88 * s)], fill=BLACK, width=width)
    for y, sign in ((0.12, 1), (0.88, -1)):
        head_poly = [
            (0.5 * s, y * s),
            (0.5 * s - head * 0.6, (y + 0.16 * sign) * s),
            (0.5 * s + head * 0.6, (y + 0.16 * sign) * s),
        ]
        d.polygon(head_poly, fill=BLACK)
        d.line(head_poly + [head_poly[0]], fill=WHITE, width=max(1, int(0.04 * s)))
    layer = layer.rotate(angle, resample=Image.BICUBIC, center=(0.5 * s, 0.5 * s))
    image.alpha_composite(layer)


def _ring(draw: ImageDraw.ImageDraw, s: float, gap: bool) -> None:
    width = max(1, int(0.10 * s))
    pad = 0.28 * s
    draw.ellipse([pad, pad, s - pad, s - pad], outline=BLACK, width=width)
    if gap:
        draw.rectangle([0.5 * s, pad, s - pad, s - 0.5 * s], fill=(0, 0, 0, 0))


def shapes(name: str, image: Image.Image, size: int) -> tuple[int, int]:
    """Draw `name` and return (xhot, yhot) in pixels."""
    s = size * SUPERSAMPLE
    draw = ImageDraw.Draw(image)
    if name in {"default", "copy", "alias", "context-menu", "help", "zoom-in", "zoom-out"}:
        _arrow(draw, s)
        hot = (int(0.12 * size), int(0.04 * size))
        if name == "copy":
            draw.ellipse([0.68 * s, 0.06 * s, 0.90 * s, 0.28 * s], fill=BLACK, outline=WHITE)
            draw.line([(0.79 * s, 0.11 * s), (0.79 * s, 0.23 * s)], fill=WHITE, width=max(1, int(0.06 * s)))
            draw.line([(0.73 * s, 0.17 * s), (0.85 * s, 0.17 * s)], fill=WHITE, width=max(1, int(0.06 * s)))
        return hot
    if name == "text":
        w = max(1, int(0.10 * s))
        draw.line([(0.5 * s, 0.16 * s), (0.5 * s, 0.84 * s)], fill=BLACK, width=w)
        for y in (0.16, 0.84):
            draw.line([(0.34 * s, y * s), (0.66 * s, y * s)], fill=BLACK, width=w)
        return (int(0.5 * size), int(0.5 * size))
    if name == "vertical-text":
        w = max(1, int(0.10 * s))
        draw.line([(0.16 * s, 0.5 * s), (0.84 * s, 0.5 * s)], fill=BLACK, width=w)
        for x in (0.16, 0.84):
            draw.line([(x * s, 0.34 * s), (x * s, 0.66 * s)], fill=BLACK, width=w)
        return (int(0.5 * size), int(0.5 * size))
    if name in {"crosshair", "all-scroll"}:
        w = max(1, int(0.09 * s))
        draw.line([(0.5 * s, 0.12 * s), (0.5 * s, 0.88 * s)], fill=BLACK, width=w)
        draw.line([(0.12 * s, 0.5 * s), (0.88 * s, 0.5 * s)], fill=BLACK, width=w)
        if name == "all-scroll":
            for cx, cy, dx, dy in ((0.5, 0.12, 0, -1), (0.5, 0.88, 0, 1), (0.12, 0.5, -1, 0), (0.88, 0.5, 1, 0)):
                draw.polygon(
                    [
                        (cx * s, cy * s),
                        ((cx + 0.05 * dx - 0.05 * dy) * s, (cy + 0.10 * dy + 0.05 * dx) * s),
                        ((cx + 0.05 * dx + 0.05 * dy) * s, (cy + 0.10 * dy - 0.05 * dx) * s),
                    ],
                    fill=BLACK,
                )
        return (int(0.5 * size), int(0.5 * size))
    if name in {"ns-resize", "row-resize"}:
        _double_arrow(image, s, 0.0)
        return (int(0.5 * size), int(0.5 * size))
    if name in {"ew-resize", "col-resize"}:
        _double_arrow(image, s, 90.0)
        return (int(0.5 * size), int(0.5 * size))
    if name == "nesw-resize":
        _double_arrow(image, s, 45.0)
        return (int(0.5 * size), int(0.5 * size))
    if name == "nwse-resize":
        _double_arrow(image, s, 135.0)
        return (int(0.5 * size), int(0.5 * size))
    if name == "move":
        _double_arrow(image, s, 0.0)
        _double_arrow(image, s, 90.0)
        return (int(0.5 * size), int(0.5 * size))
    if name == "not-allowed":
        _ring(draw, s, gap=False)
        draw.line([(0.24 * s, 0.76 * s), (0.76 * s, 0.24 * s)], fill=BLACK, width=max(1, int(0.10 * s)))
        return (int(0.5 * size), int(0.5 * size))
    if name == "wait":
        _ring(draw, s, gap=True)
        return (int(0.5 * size), int(0.5 * size))
    if name == "progress":
        _arrow(draw, s)
        _ring(draw, s, gap=True)
        return (int(0.12 * size), int(0.04 * size))
    # Anything else is not drawn yet; inherit Adwaita so the pointer still works.
    raise KeyError(name)


# Shapes drawn by this generator. The rest inherit Adwaita (index.theme).
DRAWN = (
    "default",
    "text",
    "vertical-text",
    "crosshair",
    "move",
    "all-scroll",
    "ns-resize",
    "ew-resize",
    "nesw-resize",
    "nwse-resize",
    "row-resize",
    "col-resize",
    "not-allowed",
    "wait",
    "progress",
    "copy",
    "context-menu",
    "help",
)

ALIASES = {
    "left_ptr": "default",
    "arrow": "default",
    "top_left_arrow": "default",
    "xterm": "text",
    "ibeam": "text",
    "hand2": "default",
    "sb_h_double_arrow": "ew-resize",
    "sb_v_double_arrow": "ns-resize",
    "size_hor": "ew-resize",
    "size_ver": "ns-resize",
    "size_fdiag": "nwse-resize",
    "size_bdiag": "nesw-resize",
    "fleur": "move",
    "watch": "wait",
    "crossed_circle": "not-allowed",
    "left_ptr_watch": "progress",
}


def _xcursor(images: list[tuple[int, Image.Image, int, int]], path: Path) -> None:
    """Write one Xcursor file with one image per nominal size."""
    toc = []
    body = bytearray()
    position = XCURSOR_HEADER + 12 * len(images)
    for nominal, image, xhot, yhot in images:
        pixels = image.tobytes("raw", "BGRA")  # Xcursor wants ARGB little-endian.
        header = struct.pack(
            "<IIIIIIIII",
            IMAGE_HEADER,
            IMAGE_TYPE,
            nominal,
            1,
            image.width,
            image.height,
            xhot,
            yhot,
            0,
        )
        # Premultiply on the fly (Pillow gives straight alpha).
        out = bytearray()
        for i in range(0, len(pixels), 4):
            b, g, r, a = pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]
            out += bytes(((a * b) // 255, (a * g) // 255, (a * r) // 255, a))
        chunk = struct.pack("<I", len(header) + len(out)) + header + bytes(out)
        toc.append((IMAGE_TYPE, nominal, position))
        body += chunk
        position += len(chunk)
    data = bytearray()
    data += struct.pack("<IIII", XCURSOR_MAGIC, XCURSOR_HEADER, XCURSOR_VERSION, len(toc))
    for entry_type, subtype, pos in toc:
        data += struct.pack("<III", entry_type, subtype, pos)
    data += body
    path.write_bytes(data)


def build(output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for name in DRAWN:
        rendered = []
        for size in SIZES:
            image = _canvas(size)
            xhot, yhot = shapes(name, image, size)
            rendered.append((size, _downscale(image, size), xhot, yhot))
        _xcursor(rendered, output / name)
    for alias, target in ALIASES.items():
        (output / alias).write_bytes((output / target).read_bytes())
    (output / "cursor.theme").write_text(
        "[Icon Theme]\nName=rmac\nComment=Original rmac pointer theme\n",
        encoding="utf-8",
    )
    (output / "index.theme").write_text(
        "[Icon Theme]\n"
        "Name=rmac\n"
        "Comment=Original rmac pointer theme\n"
        "Inherits=Adwaita\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    destination = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("assets/cursors/rmac")
    build(destination)
    print(f"wrote {destination}")
