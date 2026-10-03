"""Shared validation and capture timing for parallel first-hour journeys."""

from __future__ import annotations

import hashlib
import json
import time
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[2]
JOURNEYS = ROOT / "tests" / "parallel"
ACTIONS = {"launch", "click", "menu", "key", "type", "wait", "drag_window", "shot"}
# Assertions a shot can carry. A journey FAILS when one does not hold.
#   expect_change    (default true after an action) the shot must differ from
#                    the previous shot by more than a caret blink.
#   expect_window    app id of the window that must hold focus.
#   expect_mapped    app ids that must have a mapped window.
#   expect_unmapped  app ids that must have no mapped window.
#   expect_files / expect_no_files   sandbox-relative globs.
#   expect_text      {"label": accessible name, "equals"|"contains": text}.
#   expect_accessible  accessible names that must be on screen (any app).
SHOT_CHECKS_TYPES = {
    "expect_change": bool, "expect_window": str, "expect_mapped": list, "expect_unmapped": list,
    "expect_files": list, "expect_no_files": list, "expect_text": dict, "expect_accessible": list,
}
SHOT_CHECKS = set(SHOT_CHECKS_TYPES)
APPS = {"files", "text-editor", "settings", "calculator", "preview", "notes", "terminal"}


def load(path: Path) -> dict:
    data = json.loads(path.read_text())
    if not isinstance(data, dict) or not isinstance(data.get("title"), str):
        raise ValueError(f"{path}: title is required")
    if data.get("mac") not in (None, "unsafe"):
        raise ValueError(f"{path}: mac must be 'unsafe' when present")
    if not isinstance(data.get("steps"), list):
        raise ValueError(f"{path}: steps must be a list")
    names = set()
    awaiting_shot = False
    for number, step in enumerate(data["steps"], 1):
        if not isinstance(step, dict) or len(ACTIONS.intersection(step)) != 1:
            raise ValueError(f"{path}:{number}: exactly one action is required")
        action = next(iter(ACTIONS.intersection(step)))
        if action == "shot":
            name = step["shot"]
            if not isinstance(name, str) or not name.replace("-", "").replace("_", "").isalnum() or name in names:
                raise ValueError(f"{path}:{number}: shot name must be unique and filename-safe")
            names.add(name)
            awaiting_shot = False
        elif action != "wait":
            if awaiting_shot:
                raise ValueError(f"{path}:{number}: action before shot for previous action")
            awaiting_shot = True
        if action == "launch" and step[action] not in APPS:
            raise ValueError(f"{path}:{number}: unknown app")
        if action == "menu" and (not isinstance(step[action], list) or
                                 len(step[action]) < 2 or
                                 not all(isinstance(label, str) and label for label in step[action])):
            raise ValueError(f"{path}:{number}: menu must be a path of accessible names")
        for key in SHOT_CHECKS.intersection(step):
            if action != "shot":
                raise ValueError(f"{path}:{number}: {key} belongs on a shot")
            if not isinstance(step[key], SHOT_CHECKS_TYPES[key]):
                raise ValueError(f"{path}:{number}: {key} has the wrong type")
    if awaiting_shot or not names:
        raise ValueError(f"{path}: every meaningful action needs a following shot")
    for name in data.get("setup", {}).get("files", {}):
        if Path(name).is_absolute() or ".." in Path(name).parts:
            raise ValueError(f"{path}: setup path escapes sandbox: {name}")
    return data


def paths(names: list[str]) -> list[Path]:
    return [JOURNEYS / f"{name}.json" for name in names] if names else sorted(JOURNEYS.glob("*.json"))


def fingerprint(path: Path) -> str:
    # Downsampled luma suppresses cursor blink and minor antialiasing noise.
    with Image.open(path) as source:
        image = source.convert("L").resize((128, 96), Image.Resampling.BILINEAR)
        return hashlib.blake2s(image.tobytes(), digest_size=12).hexdigest()


def changed_pixels(before: Image.Image, after: Image.Image, threshold: int = 40) -> int:
    """Pixels whose brightness moved by more than `threshold` (0-255)."""
    if before.size != after.size:
        return before.width * before.height
    difference = ImageChops.difference(before.convert("RGB"), after.convert("RGB")).convert("L")
    return sum(difference.point(lambda value: 255 if value > threshold else 0).histogram()[255:])


# A blinking caret is about 2 × 18 px; a ticked checkbox or a new row is far
# more. Below this many changed pixels two shots count as the same screen.
SAME_SCREEN_PIXELS = 120


def measure(capture, act, scratch: Path, timeout: float = 5.0, probe=None) -> dict:
    """Sample screenshots after input; all work runs off the app/UI thread.

    `capture` writes a PNG. Capturing starts before the input, so even a slow
    input helper is measured from dispatch. We request 45 Hz; the actual rate
    is reported because screencapture/grim subprocesses can be slower.
    """
    baseline = scratch / "baseline.png"
    sample = scratch / "sample.png"
    if probe:
        previous = probe()
    else:
        capture(baseline)
        previous = fingerprint(baseline)
    start = time.monotonic()
    act()
    sample_start = time.monotonic()
    first = None
    last_change = None
    samples = 0
    end = start + timeout
    while time.monotonic() < end:
        if probe:
            value = probe()
        else:
            capture(sample)
            value = fingerprint(sample)
        samples += 1
        now = time.monotonic()
        if value != previous:
            if first is None:
                first = now
            last_change = now
            previous = value
        if first is not None and last_change is not None and now - last_change >= 0.3:
            break
        time.sleep(max(0.0, 1 / 45 - (time.monotonic() - now)))
    finished = time.monotonic()
    return {
        "first_change_ms": round((first - start) * 1000) if first else None,
        "settled_ms": round((last_change + 0.3 - start) * 1000) if last_change and finished < end else None,
        "samples": samples,
        "actual_hz": round(samples / max(finished - sample_start, 0.001), 1),
        "timed_out": finished >= end,
    }
