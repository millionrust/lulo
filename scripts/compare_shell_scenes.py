#!/usr/bin/env python3
"""Hold Lulo OS's and Windows' shells to the same pixels.

    python3 scripts/compare_shell_scenes.py LINUX.png WINDOWS.png \\
        --diff DIFF.png --report REPORT.json [--max-mean 6] [--max-changed 0.03]

ADR 0023, "Phase 3 revised: shared shell views": Lulo on Windows runs Lulo
OS's own menu bar, Dock and desktop. CI draws the same fixed scene on both
(scripts/behavior/run_shell_scene.py and scripts/windows/shell_scene.py) and
this compares the two screens, so Windows can never drift from Lulo OS
again unnoticed.

The comparison is perceptual rather than exact: both screens are blurred a
little first, so text antialiasing (FreeType/swash on Lulo OS, DirectWrite
on Windows) and the compositors' blur implementations do not count, while a
moved, missing, recoloured or resized element does. It reports, for the
whole screen and for the menu bar, the Dock and the desktop between them:

- `mean`: the mean luminance difference (0 to 255);
- `changed`: the share of pixels differing by more than `--pixel-threshold`.

It writes a diff image (the Lulo OS screen in grey with the differences in
red) beside the two screens, and a JSON report, and exits 1 when the whole
screen or any region is over the limits.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

BAR_HEIGHT = 30
DOCK_HEIGHT = 110


def regions(width: int, height: int) -> dict[str, tuple[int, int, int, int]]:
    return {
        "screen": (0, 0, width, height),
        "menu bar": (0, 0, width, BAR_HEIGHT),
        "dock": (0, height - DOCK_HEIGHT, width, height),
        "desktop": (0, BAR_HEIGHT, width, height - DOCK_HEIGHT),
    }


def score(difference, box: tuple[int, int, int, int], pixel_threshold: int) -> dict[str, float]:
    """Mean and changed share of a luminance difference image inside `box`."""
    region = difference.crop(box)
    histogram = region.histogram()
    total = sum(histogram) or 1
    mean = sum(value * count for value, count in enumerate(histogram)) / total
    changed = sum(histogram[pixel_threshold + 1:]) / total
    return {"mean": round(mean, 3), "changed": round(changed, 5)}


def compare(linux_path: Path, windows_path: Path, blur: float, pixel_threshold: int):
    from PIL import Image, ImageChops, ImageFilter

    linux = Image.open(linux_path).convert("RGB")
    windows = Image.open(windows_path).convert("RGB")
    if windows.size != linux.size:
        windows = windows.resize(linux.size, Image.Resampling.LANCZOS)
    soft_linux = linux.filter(ImageFilter.GaussianBlur(blur))
    soft_windows = windows.filter(ImageFilter.GaussianBlur(blur))
    difference = ImageChops.difference(soft_linux, soft_windows).convert("L")
    width, height = linux.size
    scores = {name: score(difference, box, pixel_threshold) for name, box in regions(width, height).items()}
    return linux, windows, difference, scores


def diff_image(linux, windows, difference):
    """Lulo OS's screen, Windows' screen and the differences in red."""
    from PIL import Image, ImageOps

    grey = ImageOps.grayscale(linux).convert("RGB")
    red = Image.new("RGB", linux.size, (255, 0, 0))
    mask = difference.point(lambda value: min(255, value * 4))
    marked = Image.composite(red, grey, mask)
    width, height = linux.size
    sheet = Image.new("RGB", (width * 3, height), (255, 255, 255))
    sheet.paste(linux, (0, 0))
    sheet.paste(windows, (width, 0))
    sheet.paste(marked, (width * 2, 0))
    return sheet


def failures(scores: dict[str, dict[str, float]], max_mean: float, max_changed: float) -> list[str]:
    problems = []
    for name, values in scores.items():
        if values["mean"] > max_mean:
            problems.append(f"{name}: mean difference {values['mean']} over {max_mean}")
        if values["changed"] > max_changed:
            problems.append(f"{name}: {values['changed']:.2%} of pixels differ, over {max_changed:.2%}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("linux", type=Path)
    parser.add_argument("windows", type=Path)
    parser.add_argument("--diff", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--blur", type=float, default=1.5)
    parser.add_argument("--pixel-threshold", type=int, default=32)
    parser.add_argument("--max-mean", type=float, default=6.0)
    parser.add_argument("--max-changed", type=float, default=0.03)
    args = parser.parse_args()

    linux, windows, difference, scores = compare(args.linux, args.windows, args.blur, args.pixel_threshold)
    args.diff.parent.mkdir(parents=True, exist_ok=True)
    diff_image(linux, windows, difference).save(args.diff)
    problems = failures(scores, args.max_mean, args.max_changed)
    report = {
        "linux": str(args.linux),
        "windows": str(args.windows),
        "size": list(linux.size),
        "blur": args.blur,
        "pixel_threshold": args.pixel_threshold,
        "max_mean": args.max_mean,
        "max_changed": args.max_changed,
        "scores": scores,
        "failures": problems,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    for name, values in scores.items():
        print(f"{args.linux.stem}: {name}: mean {values['mean']}, changed {values['changed']:.2%}")
    for problem in problems:
        print(f"::error::shell scenes drifted apart ({args.linux.stem}): {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
