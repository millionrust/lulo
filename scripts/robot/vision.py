"""Screenshot checks for the live robot, using Pillow only (the reference
laptop has no numpy or OCR).

Every check takes a PIL image (or two) plus a box in *image* pixels and
returns a list of plain-dict problems, so the heuristics are unit-tested in
``scripts/test_robot_vision.py`` on synthetic images.

They are heuristics: each problem says what was measured, and the report
marks visual findings for review rather than treating them as proven bugs.
"""

from __future__ import annotations

from typing import Optional

try:
    from PIL import Image, ImageChops, ImageStat
except ImportError:  # pragma: no cover - the Mac's python may lack Pillow
    Image = ImageChops = ImageStat = None  # type: ignore[assignment]

Box = tuple[int, int, int, int]  # left, top, right, bottom (exclusive)


def _luma(image):
    return image.convert("L")


def clamp_box(box: Box, size: tuple[int, int]) -> Optional[Box]:
    left, top, right, bottom = box
    left, top = max(0, left), max(0, top)
    right, bottom = min(size[0], right), min(size[1], bottom)
    if right - left < 4 or bottom - top < 4:
        return None
    return (left, top, right, bottom)


def blank_window(image, box: Box, title_bar: int = 44, margin: int = 6) -> list[dict]:
    """A window body drawn as one flat colour (blank) or nearly black.

    The body is everything under the title bar (`title_bar` image pixels,
    35 pt at 1.25x: the traffic lights' row), inset by `margin` for the rounded corners.
    It must be the *whole* body: a document with two lines at the top is
    not blank, even though most of its body is background.
    """

    box = clamp_box(box, image.size)
    if box is None:
        return []
    crop = _luma(image.crop(box))
    w, h = crop.size
    if w <= 2 * margin or h <= title_bar + margin:
        return []
    inner = crop.crop((margin, title_bar, w - margin, h - margin))
    stat = ImageStat.Stat(inner)
    mean, stddev = stat.mean[0], stat.stddev[0]
    problems = []
    if mean < 6 and stddev < 3:
        problems.append({"kind": "black-window", "detail": f"window body mean luma {mean:.1f}, stddev {stddev:.1f}"})
    elif stddev < 1.0:
        problems.append({"kind": "blank-window", "detail": f"window body is one flat colour (luma {mean:.1f}, stddev {stddev:.2f})"})
    return problems


def _histogram_mode(hist: list[int]) -> int:
    return max(range(len(hist)), key=lambda value: hist[value])


def low_contrast(image, box: Box, min_ratio: float = 2.2, min_ink: float = 0.002) -> list[dict]:
    """Text-ish ink too close to its background (light text on light).

    Takes the commonest luma as the background and the pixels that differ
    from it by more than 12 levels as ink. If there is real ink but even its
    most extreme 1% stays under a WCAG-style contrast ratio of `min_ratio`
    against the background, the panel's content is hard to read.
    """

    box = clamp_box(box, image.size)
    if box is None:
        return []
    crop = _luma(image.crop(box))
    hist = crop.histogram()
    total = sum(hist)
    background = _histogram_mode(hist)
    ink = [(value, count) for value, count in enumerate(hist) if abs(value - background) > 12 and count]
    ink_total = sum(count for _value, count in ink)
    if total == 0 or ink_total / total < min_ink:
        return []
    # The most extreme 1% of ink pixels.
    ink.sort(key=lambda pair: abs(pair[0] - background), reverse=True)
    budget = max(1, ink_total // 100)
    extreme = ink[0][0]
    seen = 0
    for value, count in ink:
        seen += count
        extreme = value
        if seen >= budget:
            break
    ratio = contrast_ratio(background, extreme)
    if ratio < min_ratio:
        return [{"kind": "low-contrast", "detail": f"strongest ink luma {extreme} on background {background}: contrast {ratio:.2f} < {min_ratio}"}]
    return []


def contrast_ratio(a: int, b: int) -> float:
    def lum(value: int) -> float:
        c = value / 255.0
        return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4

    high, low = sorted((lum(a), lum(b)), reverse=True)
    return (high + 0.05) / (low + 0.05)


def edge_ink(image, box: Box, band: int = 3, inset: int = 4, min_runs: int = 6) -> list[dict]:
    """Text cut by a panel's right or bottom edge.

    Looks at a thin band just inside the edge: text clipped there leaves
    many short dark/light runs (glyph stems) along the band, where a clean
    edge shows padding (one flat run) or a straight border.
    """

    box = clamp_box(box, image.size)
    if box is None:
        return []
    crop = _luma(image.crop(box))
    w, h = crop.size
    problems = []
    for edge in ("right", "bottom"):
        if edge == "right":
            strip = crop.crop((w - inset - band, h // 6, w - inset, h - h // 6))
            line = [strip.getpixel((band // 2, y)) for y in range(strip.size[1])]
        else:
            strip = crop.crop((w // 8, h - inset - band, w - w // 8, h - inset))
            line = [strip.getpixel((x, band // 2)) for x in range(strip.size[0])]
        runs = _contrast_runs(line)
        if runs >= min_runs:
            problems.append({"kind": "edge-clipping", "detail": f"{runs} glyph-like runs in the {band}px band {inset}px inside the {edge} edge"})
    return problems


def _contrast_runs(line: list[int], threshold: int = 60, max_run: int = 6) -> int:
    """Count short excursions from the line's commonest value (glyph stems)."""

    if not line:
        return 0
    counts: dict[int, int] = {}
    for value in line:
        counts[value // 8] = counts.get(value // 8, 0) + 1
    base = max(counts, key=lambda key: counts[key]) * 8 + 4
    runs = 0
    length = 0
    for value in line + [base]:
        if abs(value - base) > threshold:
            length += 1
        else:
            if 0 < length <= max_run:
                runs += 1
            length = 0
    return runs


def changed_fraction(before, after, box: Optional[Box] = None, threshold: int = 24) -> float:
    """The fraction of pixels (in `box`) that changed by more than
    `threshold` luma levels between two screenshots."""

    if before.size != after.size:
        return 1.0
    if box is not None:
        clamped = clamp_box(box, before.size)
        if clamped is None:
            return 0.0
        before, after = before.crop(clamped), after.crop(clamped)
    diff = ImageChops.difference(_luma(before), _luma(after))
    hist = diff.histogram()
    total = sum(hist)
    return sum(hist[threshold + 1:]) / total if total else 0.0


def changed_box(before, after, threshold: int = 24) -> Optional[Box]:
    """The bounding box of pixels that changed by more than `threshold`."""

    if before.size != after.size:
        return (0, 0, *after.size)
    diff = ImageChops.difference(_luma(before), _luma(after)).point(lambda v: 255 if v > threshold else 0)
    return diff.getbbox()


def residue(before, after_close, ignore: list[Box], min_fraction: float = 0.002) -> list[dict]:
    """Pixels a dismissed panel left behind: the screen after closing a
    menu/popup should match the screen before opening it, apart from
    regions known to change on their own (`ignore`: the clock, live
    graphs)."""

    if before.size != after_close.size:
        return []
    a, b = _luma(before), _luma(after_close)
    for box in ignore:
        clamped = clamp_box(box, a.size)
        if clamped:
            a.paste(0, clamped)
            b.paste(0, clamped)
    fraction = changed_fraction(a, b)
    if fraction >= min_fraction:
        bbox = changed_box(a, b)
        return [{"kind": "residue", "detail": f"{fraction * 100:.2f}% of the screen still differs after closing (box {bbox})"}]
    return []


def analyse_panel(image, box: Box) -> list[dict]:
    """Every per-panel check on one box."""

    return blank_window(image, box) + low_contrast(image, box) + edge_ink(image, box)
