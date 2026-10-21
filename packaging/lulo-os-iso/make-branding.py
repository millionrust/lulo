#!/usr/bin/env python3
"""Render the Lulo OS image branding from the original Lulo artwork (ADR 0031).

Every input is original Lulo OS artwork in this repository (docs/brand.md):
the Segment mark and lockup in assets/brand, the app tile in assets/icons and
the Lulo wallpapers. Needs rsvg-convert (librsvg2-bin) and Pillow.

    python3 make-branding.py --repo REPO --out DIR
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path

from PIL import Image

# The greeter's night blue (packaging/rmac-session/greeter), used behind the
# splash so the boot, the login screen and the desktop share one colour.
NIGHT = (16, 22, 47)


def render_svg(svg: Path, png: Path, width: int) -> Image.Image:
    png.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["rsvg-convert", "--width", str(width), "--keep-aspect-ratio", "-o", str(png), str(svg)],
        check=True,
    )
    return Image.open(png).convert("RGBA")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    repo: Path = args.repo
    out: Path = args.out
    brand = repo / "assets" / "brand"
    here = repo / "packaging" / "lulo-os-iso"
    wallpaper = repo / "packaging" / "rmac-session" / "wallpapers" / "lulo-dark-1920x1080.jpg"
    out.mkdir(parents=True, exist_ok=True)

    # Plymouth: the light-on-dark lockup, centred by lulo.script.
    render_svg(brand / "lulo-os-lockup-dark.svg", out / "plymouth" / "lockup.png", 320)
    render_svg(brand / "lulo-mark.svg", out / "plymouth" / "dot.png", 12)

    # GRUB: the dark wallpaper, dimmed so the menu reads, plus the lockup.
    theme = out / "grub-theme"
    theme.mkdir(parents=True, exist_ok=True)
    background = Image.open(wallpaper).convert("RGB")
    veil = Image.new("RGB", background.size, NIGHT)
    Image.blend(background, veil, 0.45).save(theme / "background.png", optimize=True)
    render_svg(brand / "lulo-os-lockup-dark.svg", theme / "lockup.png", 300)
    shutil.copy(here / "grub" / "theme.txt", theme / "theme.txt")

    # Installer: the Dock/app icon and Calamares' images.
    shutil.copy(repo / "assets" / "icons" / "lulo.svg", out / "lulo-installer.svg")
    calamares = out / "calamares"
    render_svg(brand / "lulo-mark.svg", calamares / "logo.png", 128)
    render_svg(brand / "lulo-os-lockup.svg", calamares / "welcome.png", 420)
    slide = Image.open(wallpaper).convert("RGB").resize((800, 450), Image.LANCZOS)
    slide = Image.blend(slide, Image.new("RGB", slide.size, NIGHT), 0.25).convert("RGBA")
    lockup = render_svg(brand / "lulo-os-lockup-dark.svg", out / "tmp-lockup.png", 360)
    slide.alpha_composite(lockup, ((slide.width - lockup.width) // 2, (slide.height - lockup.height) // 2))
    slide.convert("RGB").save(calamares / "slide-1.png", optimize=True)
    (out / "tmp-lockup.png").unlink()


if __name__ == "__main__":
    main()
