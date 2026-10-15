"""Scroll smoothness on Windows: touchpad-like wheel deltas, then notches.

Usage: python scripts/windows/scroll_smoke.py <bin-dir> [--before] [--json <file>]

Opens Text Editor on a long document with `gpui_windows`' scroll trace on
(`RMAC_GPUI_SCROLL_TRACE=1`, docs/decisions/0025-vendor-gpui-windows.md,
"Smooth scrolling"), puts the pointer over the text and injects:

1. a precision-touchpad-like stream: a 15-unit wheel delta (an eighth of a
   notch) every 8 ms for 0.6 s, the way a touchpad Direct Manipulation does
   not own reports at about 125 Hz; then nothing;
2. three mouse notches, 0.4 s apart.

From the trace it reads every frame's time and vertical scroll and checks,
for the touchpad stream:

- the scroll moves on every frame from its first moving frame to its last
  (no frame without movement, every one drawn);
- no gap between those frames above `GAP_FACTOR` x the vsync interval
  (reported but not counted without a hardware GPU, as on CI runners,
  where WARP's own frame cost decides);
- it keeps moving (momentum) for at least `MIN_MOMENTUM_FRAMES` frames
  after the last delta;

and prints the frame-interval and per-frame-step numbers for both parts.
Each notch must move on several frames (eased, not one jump).

`--before` runs the same input with `RMAC_GPUI_SMOOTH_SCROLL=0`
(upstream's immediate line deltas) and only prints its numbers, for a
before/after comparison. Windows only: it calls user32 through ctypes.
`launch_smoke.py` runs the check (not `--before`) when Text Editor is in
its app list.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes
from pathlib import Path

GAP_FACTOR = 1.5
MIN_MOMENTUM_FRAMES = 10
TOUCHPAD_DELTA = -15  # scroll down
TOUCHPAD_INTERVAL = 0.008
TOUCHPAD_SECONDS = 0.6
NOTCHES = 3
NOTCH_SPACING = 0.4
MOUSEEVENTF_WHEEL = 0x0800
WINDOW_TIMEOUT_SECONDS = 60.0

LINE = re.compile(r"^gpui_windows scroll: (.*) at ([0-9.]+) ms$")


def user32():
    return ctypes.windll.user32


def visible_window(pid: int) -> int | None:
    found: list[int] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def collect(hwnd, _lparam):
        owner = wintypes.DWORD()
        user32().GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32().IsWindowVisible(hwnd):
            if user32().GetWindowTextLengthW(hwnd) > 0:
                found.append(hwnd)
        return True

    user32().EnumWindows(collect, 0)
    return found[0] if found else None


def wheel(delta: int) -> None:
    user32().mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta & 0xFFFFFFFF, 0)


def software_display() -> bool:
    """Whether no display adapter is a real GPU (a CI runner's VM): Direct3D
    then renders on WARP, whose frames in a debug build can take longer
    than a vblank whatever the backend does."""
    try:
        names = subprocess.run(
            ["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name"],
            capture_output=True,
            text=True,
            timeout=60,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    adapters = [name.strip() for name in names.splitlines() if name.strip()]
    return all("Basic" in name or "Hyper-V" in name for name in adapters)


def write_document(path: Path) -> None:
    lines = [f"Line {n}: the quick brown fox jumps over the lazy dog." for n in range(4000)]
    path.write_text("\n".join(lines), encoding="utf-8")


def parse(log: str) -> tuple[float | None, list[float], list[tuple[float, bool, float]]]:
    """(vsync interval ms, input times, frames as (time, drew, dy))."""
    interval = None
    inputs: list[float] = []
    frames: list[tuple[float, bool, float]] = []
    for line in log.splitlines():
        match = LINE.match(line.strip())
        if not match:
            continue
        body, at = match.group(1), float(match.group(2))
        fields = dict(part.split("=", 1) for part in body.split() if "=" in part)
        if body.startswith("vsync interval"):
            interval = float(body.split()[2])
        elif body.startswith("input"):
            inputs.append((at, float(fields.get("px", "0"))))
        elif body.startswith("frame"):
            frames.append((at, fields.get("drew") == "1", float(fields.get("dy", "0"))))
    return interval, inputs, frames


def frame_steps(inputs, frames, smoothed: bool) -> list[tuple[float, bool, float]]:
    """Each frame's scroll: the trace's own `dy` when the backend hands
    scroll out per frame, else the input that arrived since the frame
    before (upstream applies each delta the moment it arrives)."""
    if smoothed:
        return frames
    steps = []
    previous = float("-inf")
    for at, drew, _ in frames:
        moved = sum(px for time_, px in inputs if previous < time_ <= at)
        steps.append((at, drew, moved))
        previous = at
    return steps


def analyse(name: str, steps, start: float, end: float, interval: float) -> dict:
    """Numbers for the frames from the first that moves after `start` to the
    last that moves before `end`."""
    window = [step for step in steps if start <= step[0] <= end]
    moving = [index for index, step in enumerate(window) if step[2] != 0.0]
    if not moving:
        return {"name": name, "frames": 0}
    run = window[moving[0] : moving[-1] + 1]
    times = [step[0] for step in run]
    gaps = [b - a for a, b in zip(times, times[1:])]
    distances = [abs(step[2]) for step in run]
    still = sum(1 for step in run if step[2] == 0.0)
    undrawn = sum(1 for step in run if not step[1])
    result = {
        "name": name,
        "frames": len(run),
        "still_frames": still,
        "undrawn_frames": undrawn,
        "max_gap_ms": round(max(gaps), 2) if gaps else 0.0,
        "mean_gap_ms": round(statistics.mean(gaps), 2) if gaps else 0.0,
        "gaps_over_budget": sum(1 for gap in gaps if gap > GAP_FACTOR * interval),
        "step_mean_px": round(statistics.mean(distances), 2),
        "step_cv": round(statistics.pstdev(distances) / statistics.mean(distances), 3)
        if statistics.mean(distances)
        else 0.0,
        "last_moving_ms": round(times[-1], 1),
        "run": [(round(step[0] - times[0], 1), int(step[1]), round(step[2], 2)) for step in run],
    }
    return result


def run(bin_dir: Path, before: bool) -> tuple[list[str], dict]:
    """Return (failures, numbers)."""
    binary = bin_dir / "rmac-text-editor.exe"
    with tempfile.TemporaryDirectory(prefix="scroll-smoke-", ignore_cleanup_errors=True) as folder:
        profile = Path(folder)
        for sub in ("Roaming", "Local"):
            (profile / sub).mkdir()
        document = profile / "long.txt"
        write_document(document)
        environment = dict(os.environ)
        environment["APPDATA"] = str(profile / "Roaming")
        environment["LOCALAPPDATA"] = str(profile / "Local")
        environment["RMAC_GPUI_SCROLL_TRACE"] = "1"
        if before:
            environment["RMAC_GPUI_SMOOTH_SCROLL"] = "0"
        log_path = profile / "output.log"
        with log_path.open("wb") as output:
            process = subprocess.Popen(
                [str(binary), str(document)],
                env=environment,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
        try:
            hwnd = None
            deadline = time.monotonic() + WINDOW_TIMEOUT_SECONDS
            while hwnd is None and time.monotonic() < deadline and process.poll() is None:
                hwnd = visible_window(process.pid)
                time.sleep(0.05)
            if hwnd is None:
                return ["Text Editor opened no window"], {}
            time.sleep(3.0)
            user32().ShowWindow(hwnd, 9)  # SW_RESTORE
            user32().SetForegroundWindow(hwnd)
            rect = wintypes.RECT()
            user32().GetClientRect(hwnd, ctypes.byref(rect))
            origin = wintypes.POINT(0, 0)
            user32().ClientToScreen(hwnd, ctypes.byref(origin))
            user32().SetCursorPos(
                origin.x + (rect.right - rect.left) // 2, origin.y + (rect.bottom - rect.top) // 2
            )
            time.sleep(1.0)

            # 1. The touchpad-like stream, on a steady 8 ms schedule.
            started = time.perf_counter()
            sent = 0
            while True:
                due = started + sent * TOUCHPAD_INTERVAL
                if due - started > TOUCHPAD_SECONDS:
                    break
                while time.perf_counter() < due:
                    pass
                wheel(TOUCHPAD_DELTA)
                sent += 1
            time.sleep(2.5)
            # 2. Mouse notches.
            for _ in range(NOTCHES):
                wheel(-120)
                time.sleep(NOTCH_SPACING)
            time.sleep(1.0)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=30)
        log = log_path.read_text(encoding="utf-8", errors="replace")

    label = "before" if before else "after"
    interval, inputs, frames = parse(log)
    failures: list[str] = []
    if interval is None:
        interval = 16.667
    if len(inputs) < sent // 2:
        return [f"only {len(inputs)} of {sent} wheel deltas reached the window"], {}
    touchpad_inputs = [at for at, px in inputs if abs(px) < 30]
    notch_inputs = [at for at, px in inputs if abs(px) >= 30]
    if not touchpad_inputs or len(notch_inputs) < NOTCHES:
        return [f"wheel deltas missing: {len(touchpad_inputs)} small, {len(notch_inputs)} notches"], {}
    steps = frame_steps(inputs, frames, smoothed=not before)
    first, last = touchpad_inputs[0], touchpad_inputs[-1]
    touchpad = analyse("touchpad", steps, first, notch_inputs[0] - 1.0, interval)
    momentum = sum(1 for step in steps if last + 50.0 < step[0] < notch_inputs[0] and step[2] != 0.0)
    touchpad["momentum_frames"] = momentum
    touchpad["input_ms"] = round(last - first, 1)
    # Per-frame steps while the stream is steady: from 100 ms in to its end.
    steady = [abs(step[2]) for step in steps if first + 100.0 <= step[0] <= last]
    if len(steady) > 2 and statistics.mean(steady):
        touchpad["steady_step_cv"] = round(statistics.pstdev(steady) / statistics.mean(steady), 3)
        touchpad["steady_step_range_px"] = [round(min(steady), 2), round(max(steady), 2)]
    run_detail = touchpad.pop("run", [])
    notches = []
    for index, at in enumerate(notch_inputs[:NOTCHES]):
        end = notch_inputs[index + 1] - 1.0 if index + 1 < len(notch_inputs) else at + 400.0
        notches.append(analyse(f"notch {index + 1}", steps, at, end, interval))
    numbers = {"mode": label, "vsync_ms": interval, "touchpad": touchpad, "notches": notches}
    print(f"scroll ({label}): vsync {interval:.2f} ms, {len(inputs)} wheel deltas, {len(frames)} frames traced")
    print(f"scroll ({label}): touchpad {json.dumps(touchpad)}")
    for notch in notches:
        notch.pop("run", None)
        print(f"scroll ({label}): {json.dumps(notch)}")
    if not before and (touchpad.get("still_frames") or touchpad.get("gaps_over_budget")):
        print(f"scroll ({label}): touchpad frames (ms, drew, dy): {run_detail}")

    if not before:
        if touchpad.get("frames", 0) < 10:
            failures.append(f"the touchpad stream moved on only {touchpad.get('frames', 0)} frames")
        else:
            # On WARP a frame can land between two late deltas and move
            # nothing; one such frame is WARP's timing, more is a stall.
            still_allowed = 1 if software_display() else 0
            if touchpad["still_frames"] > still_allowed:
                failures.append(f"{touchpad['still_frames']} frames without movement mid-scroll")
            if touchpad["undrawn_frames"]:
                failures.append(f"{touchpad['undrawn_frames']} scrolled frames drew nothing")
            if touchpad["gaps_over_budget"] and software_display():
                print(
                    f"scroll ({label}): {touchpad['gaps_over_budget']} frame gaps over "
                    f"{GAP_FACTOR} x {interval:.2f} ms with no hardware GPU "
                    "(WARP's frame cost; reported, not counted)"
                )
            elif touchpad["gaps_over_budget"]:
                failures.append(
                    f"{touchpad['gaps_over_budget']} frame gaps over {GAP_FACTOR} x "
                    f"{interval:.2f} ms (max {touchpad['max_gap_ms']} ms)"
                )
        if momentum < MIN_MOMENTUM_FRAMES:
            failures.append(f"momentum moved only {momentum} frames after the last delta")
        for notch in notches:
            if notch.get("frames", 0) < 4:
                failures.append(f"{notch['name']} moved on {notch.get('frames', 0)} frames, not eased")
    if failures and software_display():
        # Without a hardware GPU, Text Editor's debug-build frames on WARP
        # stall the main thread long enough that wheel input arrives in
        # bursts or not at all: on 2026-10-14/15 the upstream `--before`
        # run lost it the same way (5-11 moving frames of 37). The check
        # gates on real GPUs; here it reports.
        for failure in failures:
            print(f"scroll ({label}): {failure} (no hardware GPU: reported, not counted)")
        failures = []
    return failures, numbers


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8", errors="replace")
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bin_dir", type=Path)
    parser.add_argument("--before", action="store_true")
    parser.add_argument("--json", type=Path)
    arguments = parser.parse_args()
    if sys.platform != "win32":
        print("scroll_smoke.py runs only on Windows", file=sys.stderr)
        return 2
    user32().SetProcessDPIAware()
    failures, numbers = run(arguments.bin_dir, arguments.before)
    if arguments.json is not None:
        arguments.json.write_text(json.dumps(numbers, indent=2), encoding="utf-8")
    for failure in failures:
        print(f"scroll: FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
