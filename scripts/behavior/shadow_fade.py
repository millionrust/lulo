"""Luminance-fade check for a popup panel's drop shadow.

Proves a panel's shadow fades smoothly into its surrounding background
instead of being clipped to a flat, hard-edged band by its own surface —
the class of bug in docs/parity.md SESSION-09 (the Restart/Shut Down
confirmation) and APPS-05 (App Drawer): a surface sized exactly to its
visible panel, with no margin reserved for the shadow's blur and offset,
cuts the blur off flat instead of letting it fall away.

Pure pixel-sampling math only — no compositor, no AT-SPI, no subprocess —
so it is exercised directly by scripts/test_shadow_fade.py without a nested
session, and reused by scripts/behavior/run_power_dialogs.py (and any other
behaviour runner that captures a popup with grim) against a real capture.
"""

from __future__ import annotations

from dataclasses import dataclass


def luminance(pixel: tuple[int, int, int]) -> float:
    """Rec. 709 relative luminance of an (r, g, b) pixel, 0-255."""

    r, g, b = pixel[:3]
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def sample_column(image, x: int, y_start: int, count: int, step: int = 1) -> list[float]:
    """`count` luminance samples starting at `(x, y_start)`, walking down by
    `step` device pixels each time. Stops early at the image's bottom edge."""

    width, height = image.size
    pixels = image.load()
    x = min(max(x, 0), width - 1)
    values = []
    for i in range(count):
        y = y_start + i * step
        if y < 0 or y >= height:
            break
        values.append(luminance(pixels[x, y]))
    return values


def sample_row(image, y: int, x_start: int, count: int, step: int = 1) -> list[float]:
    """`count` luminance samples starting at `(x_start, y)`, walking right by
    `step` device pixels each time. Stops early at the image's right edge."""

    width, height = image.size
    pixels = image.load()
    y = min(max(y, 0), height - 1)
    values = []
    for i in range(count):
        x = x_start + i * step
        if x < 0 or x >= width:
            break
        values.append(luminance(pixels[x, y]))
    return values


@dataclass
class FadeReport:
    ok: bool
    detail: str
    samples: list[float]


def check_fade(
    samples: list[float],
    background: float,
    *,
    max_step: float = 18.0,
    background_tolerance: float = 8.0,
    label: str = "band",
) -> FadeReport:
    """`samples` runs away from the panel's edge, each one device pixel
    further out than the last. A soft shadow starts darker (near the panel)
    and falls smoothly toward `background` (sampled well clear of any
    shadow, e.g. 40 px past the sampled band). Fails on either symptom of a
    clipped shadow:

    - a hard edge: some adjacent pair of samples jumps by more than
      `max_step` — a blur's falloff is smooth, a surface-edge clip is not;
    - a stuck band: the last sample is still more than
      `background_tolerance` away from `background` — the shadow never
      finished fading within the margin the surface reserved for it, so
      something past the sampled band is still visibly darker than the
      wallpaper (whether or not any single step was sharp enough to trip
      the first check).
    """

    if len(samples) < 2:
        return FadeReport(False, f"{label}: fewer than 2 samples ({len(samples)})", samples)
    steps = [abs(b - a) for a, b in zip(samples, samples[1:])]
    worst_jump = max(steps)
    if worst_jump > max_step:
        index = steps.index(worst_jump)
        return FadeReport(
            False,
            f"{label}: hard edge -- luminance jumped {worst_jump:.1f} between "
            f"samples {index} and {index + 1} (max allowed {max_step})",
            samples,
        )
    tail_gap = abs(samples[-1] - background)
    if tail_gap > background_tolerance:
        return FadeReport(
            False,
            f"{label}: shadow has not faded to the background within the "
            f"sampled band -- last sample {samples[-1]:.1f} vs background "
            f"{background:.1f} (tolerance {background_tolerance})",
            samples,
        )
    return FadeReport(
        True,
        f"{label}: faded smoothly ({len(samples)} samples, worst step "
        f"{worst_jump:.1f}, tail gap {tail_gap:.1f})",
        samples,
    )


def check_panel_shadow(
    image,
    box: tuple[int, int, int, int],
    *,
    band: int = 80,
    step: int = 1,
    skip: int = 4,
    max_step: float = 18.0,
    background_tolerance: float = 8.0,
) -> tuple[FadeReport, FadeReport]:
    """Checks the shadow below and to the right of `box` (a panel's
    `(x, y, w, h)` in the image's own device pixels, e.g. from an AT-SPI
    `getExtents` call). Returns `(below, right)` reports.

    `max_step` is calibrated per device pixel: sampling at a coarser `step`
    scales it up to match, so a slow, genuinely smooth falloff sampled every
    few pixels is not mistaken for a hard edge.

    The first `skip` device pixels right against the panel's own edge are
    sampled (so they still count toward reaching the far-field background)
    but excluded from the hard-edge check: a panel's opaque content gives
    way to its shadow over just a pixel or two there by construction (the
    blur's value near a solid edge is close to its darkest, near-constant
    before it starts falling away), not because anything clipped it. The
    class of bug this guards against -- a surface with no margin for the
    blur -- shows up further out, as a band that stays flat instead of
    continuing to fade, or a late, sudden cutoff; this skip exists so that
    legitimate edge is not mistaken for it."""

    x, y, w, h = box
    width, height = image.size
    pixels = image.load()
    count = max(band // step, 2)
    scaled_max_step = max_step * step
    # `skip` is a device-pixel count, like `band`; convert to the matching
    # number of samples at this `step` so it means the same physical margin
    # regardless of sampling density.
    skip_samples = -(-skip // step)  # ceiling division

    cx = min(max(x + w // 2, 0), width - 1)
    below_background_y = min(y + h + band + 40, height - 1)
    below = sample_column(image, cx, y + h, count, step)
    below_report = check_fade(
        below[skip_samples:],
        luminance(pixels[cx, below_background_y]),
        max_step=scaled_max_step,
        background_tolerance=background_tolerance,
        label="below the panel",
    )

    cy = min(max(y + h // 2, 0), height - 1)
    right_background_x = min(x + w + band + 40, width - 1)
    right = sample_row(image, cy, x + w, count, step)
    right_report = check_fade(
        right[skip_samples:],
        luminance(pixels[right_background_x, cy]),
        max_step=scaled_max_step,
        background_tolerance=background_tolerance,
        label="right of the panel",
    )

    return below_report, right_report
