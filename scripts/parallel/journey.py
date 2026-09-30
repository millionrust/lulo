"""Shared validation and capture timing for parallel first-hour journeys."""

from __future__ import annotations

import hashlib
import json
import time
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
JOURNEYS = ROOT / "tests" / "parallel"
ACTIONS = {"launch", "click", "menu", "key", "type", "wait", "drag_window", "shot"}
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
