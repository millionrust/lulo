#!/usr/bin/env python3
"""Generate rmac's layered app icons (FEEL_SPEC.md §D.6, macOS 27 icon grid).

Every icon is built from the same recipe, which is what makes a Dock look like
one system instead of a pile of logos:

  1. a squircle plate (true superellipse, not a rounded rect)
  2. a diagonal gradient fill
  3. an inner top highlight (the "glass" edge)
  4. an inner bottom shade
  5. the glyph, in white or a tinted colour
  6. a soft contact shadow, baked for the light appearance

Four appearances are emitted per icon, as macOS 27 requires:
  default · dark · clear · tinted

Output: assets/icons/<name>-<appearance>.svg  (1024 × 1024, scalable)

Usage:
    python3 scripts/build-icons.py [output_dir]
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

SIZE = 1024
PLATE = 824            # macOS icon grid: the plate is 824 of 1024
N = 5.0                # superellipse exponent — 5 reads like Apple's squircle


def squircle_path(cx: float, cy: float, half: float, n: float = N,
                  steps: int = 200) -> str:
    """|x/a|^n + |y/a|^n = 1, sampled and emitted as a closed SVG path."""
    pts = []
    for i in range(steps):
        t = 2.0 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        x = math.copysign(abs(ct) ** (2.0 / n), ct) * half
        y = math.copysign(abs(st) ** (2.0 / n), st) * half
        pts.append((cx + x, cy + y))
    head = f"M {pts[0][0]:.2f} {pts[0][1]:.2f}"
    body = " ".join(f"L {x:.2f} {y:.2f}" for x, y in pts[1:])
    return f"{head} {body} Z"


# name -> (top colour, bottom colour, glyph colour, glyph svg)
# Glyphs are drawn inside a 1024 box, centred, roughly 420–480 across.
GLYPHS: dict[str, tuple[str, str, str, str]] = {
    "files": ("#5AC8FA", "#0A84FF", "#FFFFFF", """
        <path d="M300 372h150l46 56h228c26 0 46 21 46 46v210c0 26-20 46-46 46H300
                 c-26 0-46-20-46-46V418c0-26 20-46 46-46z" fill="#FFFFFF" opacity=".92"/>
        <path d="M300 470h424c26 0 46 21 46 46v168c0 26-20 46-46 46H300c-26 0-46-20-46-46V516
                 c0-25 20-46 46-46z" fill="#FFFFFF"/>
        <rect x="316" y="546" width="180" height="26" rx="13" fill="#0A84FF" opacity=".55"/>
        <rect x="316" y="604" width="120" height="26" rx="13" fill="#0A84FF" opacity=".35"/>
    """),
    "apps": ("#A78BFA", "#6C4BE8", "#FFFFFF", """
        <g fill="#FFFFFF">
          <rect x="300" y="300" width="128" height="128" rx="34"/>
          <rect x="448" y="300" width="128" height="128" rx="34"/>
          <rect x="596" y="300" width="128" height="128" rx="34"/>
          <rect x="300" y="448" width="128" height="128" rx="34"/>
          <rect x="448" y="448" width="128" height="128" rx="34" opacity=".85"/>
          <rect x="596" y="448" width="128" height="128" rx="34"/>
          <rect x="300" y="596" width="128" height="128" rx="34"/>
          <rect x="448" y="596" width="128" height="128" rx="34"/>
          <rect x="596" y="596" width="128" height="128" rx="34" opacity=".85"/>
        </g>
    """),
    "notes": ("#FFE082", "#F5B301", "#8A5B00", """
        <rect x="286" y="262" width="452" height="500" rx="40" fill="#FFFFFF"/>
        <rect x="286" y="262" width="452" height="96" rx="40" fill="#FFD233"/>
        <rect x="286" y="318" width="452" height="40" fill="#FFD233"/>
        <g fill="#C9A227">
          <rect x="342" y="426" width="340" height="22" rx="11"/>
          <rect x="342" y="494" width="340" height="22" rx="11"/>
          <rect x="342" y="562" width="248" height="22" rx="11"/>
          <rect x="342" y="630" width="180" height="22" rx="11"/>
        </g>
    """),
    "text-editor": ("#FFFFFF", "#D8D8DE", "#3A3A3C", """
        <rect x="300" y="238" width="424" height="548" rx="40" fill="#FFFFFF"/>
        <g fill="#8E8E93">
          <rect x="358" y="330" width="308" height="20" rx="10"/>
          <rect x="358" y="392" width="308" height="20" rx="10"/>
          <rect x="358" y="454" width="240" height="20" rx="10"/>
        </g>
        <path d="M596 700l150-150 58 58-150 150-74 16z" fill="#1372F9"/>
        <path d="M596 700l16-74 58 58z" fill="#0A5BD0"/>
    """),
    "terminal": ("#3A3A3F", "#141417", "#30D158", """
        <rect x="262" y="286" width="500" height="452" rx="44" fill="#0B0B0D"/>
        <rect x="262" y="286" width="500" height="86" rx="44" fill="#2C2C31"/>
        <rect x="262" y="330" width="500" height="42" fill="#2C2C31"/>
        <circle cx="318" cy="329" r="14" fill="#FF5F57"/>
        <circle cx="362" cy="329" r="14" fill="#FEBC2E"/>
        <circle cx="406" cy="329" r="14" fill="#28C840"/>
        <path d="M330 456l86 74-86 74" stroke="#30D158" stroke-width="30"
              fill="none" stroke-linecap="round" stroke-linejoin="round"/>
        <rect x="452" y="586" width="170" height="28" rx="14" fill="#30D158"/>
    """),
    "system-settings": ("#C7C7CC", "#8E8E93", "#FFFFFF", """
        <path d="M512 330c14 0 26 10 28 24l7 47a190 190 0 0148 28l44-19c13-6 28-1 35 11l22 38
                 c7 12 4 27-7 36l-37 30a190 190 0 010 55l37 30c11 9 14 24 7 36l-22 38
                 c-7 12-22 17-35 11l-44-19a190 190 0 01-48 28l-7 47c-2 14-14 24-28 24h-44
                 c-14 0-26-10-28-24l-7-47a190 190 0 01-48-28l-44 19c-13 6-28 1-35-11l-22-38
                 c-7-12-4-27 7-36l37-30a190 190 0 010-55l-37-30c-11-9-14-24-7-36l22-38
                 c7-12 22-17 35-11l44 19a190 190 0 0148-28l7-47c2-14 14-24 28-24z"
              fill="#FFFFFF"/>
        <circle cx="490" cy="512" r="92" fill="#8E8E93"/>
    """),
    "system-monitor": ("#5AC8FA", "#0A84FF", "#FFFFFF", """
        <rect x="262" y="286" width="500" height="452" rx="44" fill="#FFFFFF" opacity=".95"/>
        <path d="M312 606l84-96 78 62 92-140 64 86 82-52" stroke="#0A84FF" stroke-width="34"
              fill="none" stroke-linecap="round" stroke-linejoin="round"/>
        <g fill="#0A84FF" opacity=".35">
          <rect x="330" y="642" width="52" height="56" rx="14"/>
          <rect x="414" y="614" width="52" height="84" rx="14"/>
          <rect x="498" y="586" width="52" height="112" rx="14"/>
          <rect x="582" y="628" width="52" height="70" rx="14"/>
        </g>
    """),
    "calculator": ("#5A5C63", "#26272B", "#FF9500", """
        <rect x="302" y="232" width="420" height="560" rx="72" fill="#141417"/>
        <rect x="346" y="276" width="332" height="104" rx="30" fill="#2C2D32"/>
        <rect x="552" y="318" width="92" height="22" rx="11" fill="#F2F2F4"/>
        <g fill="#A5A7AD">
          <circle cx="386" cy="450" r="34"/><circle cx="470" cy="450" r="34"/>
          <circle cx="554" cy="450" r="34"/>
        </g>
        <g fill="#4A4C53">
          <circle cx="386" cy="534" r="34"/><circle cx="470" cy="534" r="34"/>
          <circle cx="554" cy="534" r="34"/>
          <circle cx="386" cy="618" r="34"/><circle cx="470" cy="618" r="34"/>
          <circle cx="554" cy="618" r="34"/>
          <circle cx="386" cy="702" r="34"/><circle cx="470" cy="702" r="34"/>
          <circle cx="554" cy="702" r="34"/>
        </g>
        <g fill="#FF9500">
          <circle cx="638" cy="450" r="34"/><circle cx="638" cy="534" r="34"/>
          <circle cx="638" cy="618" r="34"/><circle cx="638" cy="702" r="34"/>
        </g>
    """),
    "trash": ("#D8D8DE", "#A5A5AC", "#FFFFFF", """
        <path d="M368 372h288l-28 348c-2 26-24 46-50 46H446c-26 0-48-20-50-46z"
              fill="#FFFFFF" opacity=".92"/>
        <rect x="336" y="322" width="352" height="54" rx="27" fill="#FFFFFF"/>
        <rect x="452" y="282" width="120" height="44" rx="22" fill="#FFFFFF"/>
        <g stroke="#A5A5AC" stroke-width="18" stroke-linecap="round">
          <path d="M452 452v250"/><path d="M512 452v250"/><path d="M572 452v250"/>
        </g>
    """),
}

APPEARANCES = ("default", "dark", "clear", "tinted")


def build(name: str, appearance: str) -> str:
    top, bottom, _glyph_colour, glyph = GLYPHS[name]
    cx = cy = SIZE / 2
    half = PLATE / 2
    plate = squircle_path(cx, cy, half)
    inner = squircle_path(cx, cy, half - 10)

    if appearance == "dark":
        top, bottom = shade(top, 0.72), shade(bottom, 0.62)
    elif appearance == "clear":
        top, bottom = "#FFFFFF", "#E6E6EA"
    elif appearance == "tinted":
        top, bottom = "#9A9AA2", "#6E6E76"

    plate_opacity = ".28" if appearance == "clear" else "1"
    glyph_wrap_open = '<g opacity=".92">' if appearance in ("clear", "tinted") else "<g>"

    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}"
     width="{SIZE}" height="{SIZE}">
  <defs>
    <linearGradient id="plate" x1="0" y1="0" x2="0.35" y2="1">
      <stop offset="0" stop-color="{top}"/>
      <stop offset="1" stop-color="{bottom}"/>
    </linearGradient>
    <linearGradient id="gloss" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity=".30"/>
      <stop offset=".45" stop-color="#FFFFFF" stop-opacity=".04"/>
      <stop offset="1" stop-color="#000000" stop-opacity=".10"/>
    </linearGradient>
    <filter id="contact" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity=".22"/>
    </filter>
    <clipPath id="clip"><path d="{plate}"/></clipPath>
  </defs>

  <!-- 1 plate + 6 contact shadow -->
  <path d="{plate}" fill="url(#plate)" opacity="{plate_opacity}" filter="url(#contact)"/>
  <!-- 3 + 4 glass edge -->
  <g clip-path="url(#clip)">
    <path d="{plate}" fill="url(#gloss)"/>
    <path d="{inner}" fill="none" stroke="#FFFFFF" stroke-opacity=".28" stroke-width="3"/>
  </g>
  <!-- 5 glyph -->
  {glyph_wrap_open}{glyph}</g>
</svg>
"""


def shade(hex_colour: str, factor: float) -> str:
    h = hex_colour.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    return "#%02X%02X%02X" % tuple(max(0, min(255, int(c * factor))) for c in (r, g, b))


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("assets/icons")
    out.mkdir(parents=True, exist_ok=True)
    count = 0
    for name in GLYPHS:
        for appearance in APPEARANCES:
            suffix = "" if appearance == "default" else f"-{appearance}"
            path = out / f"{name}{suffix}.svg"
            path.write_text(build(name, appearance))
            count += 1
    print(f"wrote {count} icons to {out}/")
    print("preview them all: open design-lab/icons.html")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
