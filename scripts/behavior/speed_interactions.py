"""Interaction speed scenarios for scripts/behavior/run_speed_sweep.py
(speed round 7): what people DO once an app is open, against the macOS
targets the owner set --

  - scrolling: >= 99 % of frames within the 16.7 ms (60 Hz) budget (Files
    list and icon view with 2,000 items, Notes list, Mail list with 10,000
    messages, a long Settings pane, Terminal scrollback);
  - Files: a 2,000-item folder's first rows < 100 ms after the open
    keystroke, 200 image thumbnails arriving progressively with no frame
    over the budget's stall threshold, Quick Look open < 100 ms;
  - typing, key press to the present that shows it: p95 < 16 ms (Text
    Editor plain and rich with a 1 MB document, Notes with a long note,
    Terminal, Spotlight), and Spotlight's results < 50 ms after the last
    key;
  - window actions: the Cmd-Tab switch, minimise, an edge-resize drag and
    Mission Control's open animation (frame budget).

All timings come from the traced process's own RMAC_FRAME_TRACE
(shell/compat/gpui_linux/src/linux/wayland/frame_trace.rs), one clock per
process:

  - frame cost: `frame_callback` (GPUI's frame starts: layout, prepaint,
    paint) to that frame's `present`; a frame that drew nothing
    (`draw_skip`, or a callback with no draw before the next callback) is
    not a frame. This is the CPU+GPU work per frame against the budget.
  - key latency: a key press's `input` row to the first `present` after it.
    The harness types one lowercase key every 60-80 ms and holds each for
    20 ms, so a press is an `input` row more than KEY_GAP_US after the
    previous one; its release (and any modifier row) follows within that
    gap (`key_press_latencies_ms`).
  - settled: the last `present` of a burst that started after an input,
    where a burst ends at the first quiet gap of `gap_ms` (so a caret that
    starts blinking later is not counted as content arriving).

Run with `run_speed_sweep.py --only <scenario>` (or `--interactions` for
all of them); scenario names are the keys of `SCENARIOS`.
"""

from __future__ import annotations

import math
import statistics
import struct
import subprocess
import time
import zlib
from pathlib import Path
from typing import Any, Optional

from run_frame_timing import read_trace

FRAME_BUDGET_MS = 1000.0 / 60.0
FRAME_SHARE_TARGET = 0.99
TYPING_P95_TARGET_MS = 16.0
SPOTLIGHT_RESULTS_TARGET_MS = 50.0
FOLDER_OPEN_TARGET_MS = 100.0
QUICK_LOOK_TARGET_MS = 100.0
# A frame this long is a visible hitch (three missed vblanks at 60 Hz).
STALL_MS = 50.0
SETTLE_S = 0.3
NOTES_COUNT = 500
LONG_NOTE_BYTES = 200_000
MAIL_MESSAGES = 10_000
DOCUMENT_BYTES = 1_000_000
# Settings panes long enough to scroll in a 1080p window with no hardware.
SETTINGS_SCROLL_PANES = ("accessibility", "privacy-security", "keyboard")

FILES_APP_ID = "org.rmac.Files"
NOTES_APP_ID = "org.rmac.Notes"
MAIL_APP_ID = "org.rmac.Mail"
SETTINGS_APP_ID = "org.rmac.SystemSettings"
TERMINAL_APP_ID = "org.rmac.Terminal"
TEXT_EDITOR_APP_ID = "org.rmac.TextEditor"

Events = list[tuple[str, int]]


# --------------------------------------------------------------------------
# Trace math -- pure, unit-tested in scripts/test_speed_interactions.py.
# --------------------------------------------------------------------------


def frame_costs_ms(events: Events) -> list[float]:
    """Each drawn frame's cost: its `frame_callback` to its `present`."""

    out: list[float] = []
    started: Optional[int] = None
    for event, micros in events:
        if event == "frame_callback":
            started = micros
        elif event == "present" and started is not None:
            out.append((micros - started) / 1000.0)
            started = None
        elif event == "draw_skip":
            started = None
    return out


def frame_split_ms(events: Events) -> dict[str, Any]:
    """Where drawn frames spend their time: GPUI's render, layout, prepaint
    and paint (`frame_callback` to `draw_start`) and the renderer's submit
    and present (`draw_start` to `present`), as p50/p95."""

    render: list[float] = []
    submit: list[float] = []
    started: Optional[int] = None
    drawing: Optional[int] = None
    for event, micros in events:
        if event == "frame_callback":
            started, drawing = micros, None
        elif event == "draw_start" and started is not None:
            drawing = micros
        elif event == "present" and started is not None and drawing is not None:
            render.append((drawing - started) / 1000.0)
            submit.append((micros - drawing) / 1000.0)
            started = drawing = None
        elif event == "draw_skip":
            started = drawing = None
    return {
        "render_p50_ms": percentile(render, 0.5),
        "render_p95_ms": percentile(render, 0.95),
        "submit_p50_ms": percentile(submit, 0.5),
        "submit_p95_ms": percentile(submit, 0.95),
    }


KEY_GAP_US = 45_000


def key_presses(events: Events) -> list[int]:
    """The `input` rows that start a stroke (see the module docstring).
    Builds that trace `draw_for_key` (written right after a key-down's
    `input` row) name the key-downs exactly; the 45 ms gap rule misread a
    key release that came late as the next press."""

    if any(event == "draw_for_key" for event, _ in events):
        exact: list[int] = []
        last_input: Optional[int] = None
        for event, micros in events:
            if event == "input":
                last_input = micros
            elif event == "draw_for_key" and last_input is not None:
                exact.append(last_input)
                last_input = None
        return exact
    presses: list[int] = []
    previous: Optional[int] = None
    for event, micros in events:
        if event != "input":
            continue
        if previous is None or micros - previous > KEY_GAP_US:
            presses.append(micros)
        previous = micros
    return presses


def settled_after(events: Events, start: int, gap_ms: float = 250.0) -> Optional[int]:
    """The last `present` of the burst that follows `start`."""

    last: Optional[int] = None
    for event, micros in events:
        if event != "present" or micros < start:
            continue
        if last is not None and micros - last > gap_ms * 1000:
            break
        last = micros
    return last


def key_press_latencies_ms(events: Events) -> list[float]:
    """Press-to-present for every stroke. A press with no later present
    contributes nothing."""

    presents = [micros for event, micros in events if event == "present"]
    out: list[float] = []
    cursor = 0
    for press in key_presses(events):
        while cursor < len(presents) and presents[cursor] < press:
            cursor += 1
        if cursor < len(presents):
            out.append((presents[cursor] - press) / 1000.0)
    return out


def key_press_app_latencies_ms(events: Events) -> list[float]:
    """Press-to-present minus any wait for the compositor's next frame
    callback: from the first `frame_callback` at or after the press to the
    present that follows. A window whose frame loop is parked draws at
    once; one that just presented waits for the compositor's next frame,
    about 33 ms in the nested test session (SPEED-10)."""

    out: list[float] = []
    for press in key_presses(events):
        callback = next((micros for event, micros in events
                         if event == "frame_callback" and micros >= press), None)
        if callback is None:
            continue
        present = next((micros for event, micros in events
                        if event == "present" and micros >= callback), None)
        if present is not None:
            out.append((present - callback) / 1000.0)
    return out


def percentile(values: list[float], pct: float) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(1, math.ceil(pct * len(ordered))) - 1]


def share_within(values: list[float], budget_ms: float) -> Optional[float]:
    if not values:
        return None
    return sum(1 for value in values if value <= budget_ms) / len(values)


def frames_summary(events: Events) -> dict[str, Any]:
    return frames_summary_from_costs(frame_costs_ms(events))


def frames_summary_from_costs(costs: list[float]) -> dict[str, Any]:
    share = share_within(costs, FRAME_BUDGET_MS)
    return {
        "frames": len(costs),
        "within_16_7ms_share": share,
        "frame_p50_ms": percentile(costs, 0.5),
        "frame_p95_ms": percentile(costs, 0.95),
        "worst_frame_ms": max(costs) if costs else None,
        "stalls": sum(1 for cost in costs if cost > STALL_MS),
        "pass": share is not None and share >= FRAME_SHARE_TARGET,
    }


def typing_summary(events: Events, target_ms: float = TYPING_P95_TARGET_MS) -> dict[str, Any]:
    latencies = key_press_latencies_ms(events)
    p95 = percentile(latencies, 0.95)
    return {
        "keys": len(latencies),
        "echo_p50_ms": percentile(latencies, 0.5),
        "echo_p95_ms": p95,
        "echo_max_ms": max(latencies) if latencies else None,
        "echo_after_frame_callback_p95_ms": percentile(key_press_app_latencies_ms(events), 0.95),
        # SPEED-13: the first keys after a document opens.
        "first_keys_ms": [round(latency, 3) for latency in latencies[:3]],
        "pass": p95 is not None and p95 <= target_ms,
        **frames_summary(events),
        # frames_summary's own pass is the frame budget; typing is judged
        # on the echo.
        "frames_pass": share_within(frame_costs_ms(events), FRAME_BUDGET_MS) is not None
        and share_within(frame_costs_ms(events), FRAME_BUDGET_MS) >= FRAME_SHARE_TARGET,
    }


def last_present_after(events: Events, micros: int) -> Optional[int]:
    presents = [at for event, at in events if event == "present" and at >= micros]
    return presents[-1] if presents else None


def first_present_after(events: Events, micros: int) -> Optional[int]:
    return next((at for event, at in events if event == "present" and at >= micros), None)


def last_input(events: Events) -> Optional[int]:
    inputs = [at for event, at in events if event == "input"]
    return inputs[-1] if inputs else None


# --------------------------------------------------------------------------
# Fixtures
# --------------------------------------------------------------------------


def write_png(path: Path, width: int, height: int, rgb: tuple[int, int, int]) -> None:
    """A small solid PNG with a diagonal band, written without PIL (the
    laptop's system Python may not have it)."""

    rows = []
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            band = (x + y) // 24 % 2
            row += bytes(rgb) if band else bytes(255 - c for c in rgb)
        rows.append(bytes(row))
    raw = zlib.compress(b"".join(rows), 6)

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", raw)
                     + chunk(b"IEND", b""))


def make_plain_text(size: int) -> str:
    paragraph = ("The quick brown fox jumps over the lazy dog, and the speed sweep counts how "
                 "long each keystroke takes to reach the screen in a large document. ")
    lines = []
    total = 0
    index = 0
    while total < size:
        index += 1
        line = f"{index}. {paragraph}{paragraph}\n"
        lines.append(line)
        total += len(line)
    return "".join(lines)


def make_rtf(size: int) -> str:
    """A plain RTF document of about `size` bytes: paragraphs with bold
    and italic runs, the shape of a long rich-text file."""

    head = "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}}\\f0\\fs24\n"
    body = []
    total = len(head)
    index = 0
    while total < size:
        index += 1
        para = (f"{index}. The {{\\b quick brown fox}} jumps over the {{\\i lazy dog}} while the speed "
                "sweep measures typing in a long rich document.\\par\n")
        body.append(para)
        total += len(para)
    return head + "".join(body) + "}\n"


def make_big_folder(root: Path, count: int = 2000) -> Path:
    folder = root / "Big"
    folder.mkdir(parents=True, exist_ok=True)
    kinds = ("txt", "md", "pdf", "json", "rs", "png")
    for index in range(count):
        kind = kinds[index % len(kinds)]
        name = folder / f"Item {index:04d}.{kind}"
        if kind == "png":
            name.write_bytes(b"")  # an empty "image": a placeholder icon, no decode
        else:
            name.write_text(f"{index}\n")
    return folder


def make_picture_folder(root: Path, count: int = 200) -> Path:
    folder = root / "Pictures200"
    folder.mkdir(parents=True, exist_ok=True)
    for index in range(count):
        rgb = ((index * 37) % 256, (index * 91) % 256, (index * 53) % 256)
        write_png(folder / f"Photo {index:03d}.png", 640, 480, rgb)
    return folder


# --------------------------------------------------------------------------
# Scenarios: a mixin for run_speed_sweep.Run (spawn/traced/stop/input/
# niri_msg/window_by_app_id/wait_for come from there).
# --------------------------------------------------------------------------


class InteractionScenarios:
    # -- helpers ---------------------------------------------------------

    def _mark(self, trace: Path) -> int:
        return len(read_trace(trace))

    def _since(self, trace: Path, mark: int) -> Events:
        return read_trace(trace)[mark:]

    def _settle(self, trace: Path, timeout: float = 4.0, quiet: float = SETTLE_S) -> None:
        """Wait until the trace records no new `present` for `quiet` s."""

        deadline = time.monotonic() + timeout
        last = -1
        changed = time.monotonic()
        while time.monotonic() < deadline:
            count = sum(1 for event, _ in read_trace(trace) if event == "present")
            now = time.monotonic()
            if count != last:
                last, changed = count, now
            elif now - changed >= quiet:
                return
            time.sleep(0.03)

    def _launch(self, argv: list[str], label: str, app_id: str,
                extra: Optional[dict[str, str]] = None, timeout: float = 30.0):
        process, trace = self.traced(argv, label, extra)
        window = self.wait_for(lambda: self.window_by_app_id(app_id), timeout=timeout)
        if window is None:
            self.stop(process)
            raise RuntimeError(f"{app_id} did not map a window")
        self._settle(trace, 8.0)
        return process, trace, window

    def _window_centre(self, window: dict[str, Any], fx: float = 0.6, fy: float = 0.55) -> tuple[float, float]:
        layout = window.get("layout") or {}
        pos = layout.get("tile_pos_in_workspace_view") or [0, 0]
        size = layout.get("window_size") or layout.get("tile_size") or [800, 600]
        return float(pos[0]) + float(size[0]) * fx, float(pos[1]) + float(size[1]) * fy

    def _scroll(self, trace: Path, window: dict[str, Any], steps: int = 40, amount: float = 5.0,
                fx: float = 0.6, fy: float = 0.55, interval: float = 0.016) -> dict[str, Any]:
        costs = self._scroll_costs(trace, window, steps, amount, fx, fy, interval)
        return {**frames_summary_from_costs(costs), **frame_split_ms(self._last_scroll_events)}

    def _scroll_costs(self, trace: Path, window: dict[str, Any], steps: int = 40, amount: float = 5.0,
                      fx: float = 0.6, fy: float = 0.55, interval: float = 0.016) -> list[float]:
        """Wheel-scroll down `steps` notches then back up, one notch per
        `interval` (a fast flick's event rate), and score every frame."""

        x, y = self._window_centre(window, fx, fy)
        self.input.move(x, y, self.out_w, self.out_h)
        time.sleep(0.3)
        self._settle(trace, 2.0)
        mark = self._mark(trace)
        for direction in (-amount, amount):
            for _ in range(steps):
                self.input.scroll(vertical=direction)
                time.sleep(interval)
        time.sleep(0.5)
        self._settle(trace, 3.0)
        self._last_scroll_events = self._since(trace, mark)
        return frame_costs_ms(self._last_scroll_events)

    def _type(self, trace: Path, text: str, delay: float = 0.06) -> dict[str, Any]:
        self._settle(trace, 2.0)
        mark = self._mark(trace)
        self.input.type_text(text, delay=delay)
        time.sleep(0.4)
        self._settle(trace, 3.0)
        return typing_summary(self._since(trace, mark))

    @property
    def out_w(self) -> int:
        return int(self.args.output.split("x")[0])

    @property
    def out_h(self) -> int:
        return int(self.args.output.split("x")[1])

    def _fixture_root(self) -> Path:
        root = Path(self.env["HOME"]) / "SpeedFixtures"
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _files_bin(self) -> Path:
        binary = self.bin("rmac-files")
        if binary is None:
            raise RuntimeError("rmac-files not found under --bin-dir")
        return binary

    # -- Files -------------------------------------------------------------

    def scenario_files_list_scroll(self) -> dict[str, Any]:
        big = make_big_folder(self._fixture_root())
        process, trace, window = self._launch([str(self._files_bin()), "--path", str(big)],
                                              "files-list", FILES_APP_ID)
        try:
            self.input.key("cmd-2")  # as List
            self._settle(trace, 3.0)
            return self._scroll(trace, self.window_by_app_id(FILES_APP_ID) or window)
        finally:
            self.stop(process)

    def scenario_files_icon_scroll(self) -> dict[str, Any]:
        big = make_big_folder(self._fixture_root())
        process, trace, window = self._launch([str(self._files_bin()), "--path", str(big)],
                                              "files-icons", FILES_APP_ID)
        try:
            self.input.key("cmd-1")  # as Icons
            self._settle(trace, 3.0)
            return self._scroll(trace, self.window_by_app_id(FILES_APP_ID) or window)
        finally:
            self.stop(process)

    def scenario_files_open_folder(self) -> dict[str, Any]:
        """Open the 2,000-item folder from its parent (select it by typing
        it with a click, then Cmd-Down) and time the key to the first frame that
        shows its rows (the window title switches to the folder) and to
        the last frame before the window goes quiet."""

        root = self._fixture_root()
        make_big_folder(root)
        process, trace, window = self._launch([str(self._files_bin()), "--path", str(root)],
                                              "files-open", FILES_APP_ID)
        try:
            self.input.key("cmd-2")
            self._settle(trace, 3.0)
            runs: list[dict[str, Any]] = []
            for _ in range(self.args.repeat):
                # Click "Big", the first row (toolbar 52 + header 28 + 5 +
                # half a row), then Cmd-Down opens it.
                current = self.window_by_app_id(FILES_APP_ID) or window
                x, _ = self._window_centre(current, 0.6, 0.0)
                pos = (current.get("layout") or {}).get("tile_pos_in_workspace_view") or [0, 0]
                self.input.click(x, float(pos[1]) + 97.0, self.out_w, self.out_h)
                self._settle(trace, 2.0)
                mark = self._mark(trace)
                self.input.key("cmd-down")
                titled = self.wait_for(
                    lambda: (self.window_by_app_id(FILES_APP_ID) or {}).get("title", "").startswith("Big"),
                    timeout=5.0)
                self._settle(trace, 4.0)
                events = self._since(trace, mark)
                inputs = [at for event, at in events if event == "input"]
                if not titled or not inputs:
                    if runs:
                        break
                    raise RuntimeError(f"Cmd-Down did not open Big (title {window.get('title')!r})")
                first = first_present_after(events, inputs[0])
                last = settled_after(events, inputs[0])
                runs.append({
                    "first_frame_ms": None if first is None else (first - inputs[0]) / 1000.0,
                    "settled_ms": None if last is None else (last - inputs[0]) / 1000.0,
                    "worst_frame_ms": max(frame_costs_ms(events), default=None),
                })
                self.input.key("cmd-up")
                self.wait_for(lambda: not (self.window_by_app_id(FILES_APP_ID) or {}).get("title", "").startswith("Big"),
                              timeout=5.0)
                self._settle(trace, 3.0)
            first_ms = statistics.median([r["first_frame_ms"] for r in runs if r["first_frame_ms"] is not None])
            settled_ms = statistics.median([r["settled_ms"] for r in runs if r["settled_ms"] is not None])
            return {"runs": runs, "first_frame_ms": first_ms, "settled_ms": settled_ms,
                    "pass": settled_ms <= FOLDER_OPEN_TARGET_MS}
        finally:
            self.stop(process)

    def scenario_files_thumbnails(self) -> dict[str, Any]:
        """Switch a folder of 200 PNGs to icon view: thumbnails decode off
        the UI thread and land progressively. Scored on the frame budget
        and stalls while they arrive, plus the time until the last one."""

        pictures = make_picture_folder(self._fixture_root())
        process, trace, window = self._launch([str(self._files_bin()), "--path", str(pictures)],
                                              "files-thumbs", FILES_APP_ID)
        try:
            self.input.key("cmd-2")
            self._settle(trace, 3.0)
            mark = self._mark(trace)
            self.input.key("cmd-1")
            time.sleep(0.5)
            self._settle(trace, 15.0, quiet=1.5)
            events = self._since(trace, mark)
            summary = frames_summary(events)
            inputs = [at for event, at in events if event == "input"]
            last = settled_after(events, inputs[0], gap_ms=1000.0) if inputs else None
            summary["settled_ms"] = None if last is None else (last - inputs[0]) / 1000.0
            # Then scroll through them all while they keep loading.
            summary["scroll"] = self._scroll(trace, self.window_by_app_id(FILES_APP_ID) or window, steps=30)
            summary["pass"] = summary["stalls"] == 0 and summary["pass"]
            return summary
        finally:
            self.stop(process)

    def scenario_quick_look(self) -> dict[str, Any]:
        big = make_big_folder(self._fixture_root())
        process, trace, _window = self._launch([str(self._files_bin()), "--path", str(big)],
                                               "files-quicklook", FILES_APP_ID)
        try:
            self.input.key("cmd-2")
            self._settle(trace, 3.0)
            self.input.key("down")
            self._settle(trace, 2.0)
            opens: list[float] = []
            for _ in range(self.args.repeat):
                before = {w.get("id") for w in self.niri_msg("windows") or []}
                mark = self._mark(trace)
                self.input.key("space")
                appeared = self.wait_for(lambda: [w for w in self.niri_msg("windows") or []
                                                  if w.get("id") not in before], timeout=5.0)
                self._settle(trace, 3.0)
                events = self._since(trace, mark)
                inputs = [at for event, at in events if event == "input"]
                first = first_present_after(events, inputs[0]) if inputs else None
                if not appeared or first is None:
                    raise RuntimeError("Space did not open Quick Look")
                opens.append((first - inputs[0]) / 1000.0)
                self.input.key("space")
                self.wait_for(lambda: {w.get("id") for w in self.niri_msg("windows") or []} <= before,
                              timeout=5.0)
                self._settle(trace, 2.0)
            open_ms = statistics.median(opens)
            return {"opens": opens, "open_ms": open_ms, "pass": open_ms <= QUICK_LOOK_TARGET_MS}
        finally:
            self.stop(process)

    # -- Notes, Mail, Settings, Text Editor ---------------------------------

    def _notes(self, label: str):
        binary = self.bin("rmac-notes")
        seed = self.bin("seed_speed_fixture")
        if binary is None or seed is None:
            raise RuntimeError("rmac-notes / seed_speed_fixture (rmac-notes-storage example) not found")
        data = Path(self.env.get("XDG_DATA_HOME") or Path(self.env["HOME"]) / ".local/share")
        root = data / "rmac" / "notes"
        if not (root / "library.bin").exists():
            root.mkdir(parents=True, exist_ok=True)
            seeded = subprocess.run([str(seed), str(root), str(NOTES_COUNT), str(LONG_NOTE_BYTES)],
                                    env=self.env, capture_output=True, text=True, timeout=60)
            if seeded.returncode != 0:
                raise RuntimeError(f"seeding Notes failed: {seeded.stderr[-300:]}")
        return self._launch([str(binary)], label, NOTES_APP_ID)

    def scenario_notes_list_scroll(self) -> dict[str, Any]:
        process, trace, window = self._notes("notes-list")
        try:
            # The note list is the middle column.
            return self._scroll(trace, window, fx=0.32)
        finally:
            self.stop(process)

    def scenario_notes_typing(self) -> dict[str, Any]:
        process, trace, window = self._notes("notes-typing")
        try:
            # The long note opens selected; click into its text.
            x, y = self._window_centre(window, 0.75, 0.4)
            self.input.click(x, y, self.out_w, self.out_h)
            time.sleep(0.5)
            return self._type(trace, "the quick brown fox jumps over the lazy dog")
        finally:
            self.stop(process)

    def scenario_mail_list_scroll(self) -> dict[str, Any]:
        binary = self.bin("rmac-mail")
        if binary is None:
            raise RuntimeError("rmac-mail not found under --bin-dir")
        process, trace, window = self._launch(
            [str(binary)], "mail-list", MAIL_APP_ID,
            extra={"RMAC_MAIL_FIXTURE": "1", "RMAC_MAIL_FIXTURE_MESSAGES": str(MAIL_MESSAGES)})
        try:
            return self._scroll(trace, window, fx=0.35)
        finally:
            self.stop(process)

    def scenario_settings_pane_scroll(self) -> dict[str, Any]:
        binary = self.bin("rmac-system-settings")
        if binary is None:
            raise RuntimeError("rmac-system-settings not found under --bin-dir")
        costs: dict[str, list[float]] = {}
        for pane in SETTINGS_SCROLL_PANES:
            process, trace, window = self._launch([str(binary), "--pane", pane], f"settings-{pane}",
                                                  SETTINGS_APP_ID)
            try:
                costs[pane] = self._scroll_costs(trace, window, steps=25, fx=0.7)
            finally:
                self.stop(process)
                time.sleep(0.5)
        summary = frames_summary_from_costs([cost for pane in costs.values() for cost in pane])
        summary["panes"] = {pane: frames_summary_from_costs(values) for pane, values in costs.items()}
        return summary

    def _text_editor(self, label: str, document: Path):
        binary = self.bin("rmac-text-editor")
        if binary is None:
            raise RuntimeError("rmac-text-editor not found under --bin-dir")
        return self._launch([str(binary), str(document)], label, TEXT_EDITOR_APP_ID, timeout=40.0)

    def scenario_text_editor_plain_typing(self) -> dict[str, Any]:
        document = self._fixture_root() / "Speed.txt"
        document.write_text(make_plain_text(DOCUMENT_BYTES))
        process, trace, _window = self._text_editor("te-plain", document)
        try:
            return self._type(trace, "the quick brown fox jumps over the lazy dog")
        finally:
            self.stop(process)

    def scenario_text_editor_rich_typing(self) -> dict[str, Any]:
        document = self._fixture_root() / "Speed.rtf"
        document.write_text(make_rtf(DOCUMENT_BYTES))
        process, trace, _window = self._text_editor("te-rich", document)
        try:
            return self._type(trace, "the quick brown fox jumps over the lazy dog")
        finally:
            self.stop(process)

    # -- Terminal ----------------------------------------------------------

    def _terminal(self, label: str, command: str):
        binary = self.bin("rmac-terminal")
        if binary is None:
            raise RuntimeError("rmac-terminal not found under --bin-dir")
        return self._launch([str(binary), "-e", "bash", "--norc", "-c", command], label, TERMINAL_APP_ID,
                            extra={"PS1": "$ "})

    def scenario_terminal_scrollback(self) -> dict[str, Any]:
        process, trace, window = self._terminal(
            "terminal-scroll", "seq -f 'line %g of the scrollback test' 1 20000; exec bash --norc")
        try:
            time.sleep(1.0)
            self._settle(trace, 5.0)
            # Up through the scrollback first (content moves), then back.
            x, y = self._window_centre(window)
            self.input.move(x, y, self.out_w, self.out_h)
            time.sleep(0.3)
            self._settle(trace, 2.0)
            mark = self._mark(trace)
            for direction in (5.0, -5.0):
                for _ in range(40):
                    self.input.scroll(vertical=direction)
                    time.sleep(0.016)
            time.sleep(0.5)
            self._settle(trace, 3.0)
            return frames_summary(self._since(trace, mark))
        finally:
            self.stop(process)

    def scenario_terminal_typing(self) -> dict[str, Any]:
        process, trace, _window = self._terminal("terminal-typing", "exec bash --norc")
        try:
            time.sleep(1.0)
            return self._type(trace, "echo the quick brown fox jumps over the lazy dog")
        finally:
            self.stop(process)

    # -- Spotlight ---------------------------------------------------------

    def scenario_spotlight_typing(self) -> dict[str, Any]:
        binary = self.bin("rmac-launcher")
        dispatcher = self.bin("rmac-shortcut-dispatch")
        if binary is None or dispatcher is None:
            raise RuntimeError("rmac-launcher / rmac-shortcut-dispatch not found under --bin-dir")
        process, trace = self.traced([str(binary)], "spotlight-typing")
        try:
            endpoint = self.runtime / "rmac" / "shortcut-launcher.sock"
            if not self.wait_for(endpoint.exists, timeout=20.0):
                raise RuntimeError("Spotlight did not register its shortcut endpoint")
            time.sleep(1.0)
            runs = []
            for query in ("calculator", "terminal", "settings")[: max(1, self.args.repeat)]:
                subprocess.run([str(dispatcher), "launcher"], env=self.env, capture_output=True, timeout=10)
                time.sleep(0.6)
                self._settle(trace, 3.0)
                mark = self._mark(trace)
                self.input.type_text(query, delay=0.08)
                time.sleep(0.5)
                self._settle(trace, 3.0)
                events = self._since(trace, mark)
                summary = typing_summary(events)
                presses = key_presses(events)
                press = presses[-1] if presses else None
                last = settled_after(events, press) if press is not None else None
                summary["results_ms"] = None if last is None else (last - press) / 1000.0
                runs.append(summary)
                self.input.key("escape")
                time.sleep(0.4)
                self._settle(trace, 2.0)
            latencies = [r["echo_p95_ms"] for r in runs if r["echo_p95_ms"] is not None]
            results = [r["results_ms"] for r in runs if r["results_ms"] is not None]
            echo = max(latencies) if latencies else None
            results_ms = statistics.median(results) if results else None
            return {
                "runs": runs,
                "echo_p95_ms": echo,
                "results_ms": results_ms,
                "pass": echo is not None and echo <= TYPING_P95_TARGET_MS
                and results_ms is not None and results_ms <= SPOTLIGHT_RESULTS_TARGET_MS,
            }
        finally:
            self.stop(process)

    # -- window actions ----------------------------------------------------

    def _presents(self, trace: Path) -> int:
        return sum(1 for event, _ in read_trace(trace) if event == "present")

    def _wait_present(self, trace: Path, baseline: int, start: float, timeout: float = 3.0) -> Optional[float]:
        """Harness-clock milliseconds from `start` until `trace` records a
        present beyond `baseline`."""

        deadline = start + timeout
        while time.monotonic() < deadline:
            if self._presents(trace) > baseline:
                return (time.monotonic() - start) * 1000.0
            time.sleep(0.003)
        return None

    def _focused_app_id(self) -> Optional[str]:
        focused = self.niri_msg("focused-window")
        return focused.get("app_id") if isinstance(focused, dict) else None

    def _switcher_session(self):
        """The App Switcher service plus Calculator and Clock to switch
        between: (service, service trace, [(process, trace, app_id)])."""

        switcher = self.bin("rmac-app-switcher", "app-switcher")
        if switcher is None:
            raise RuntimeError("rmac-app-switcher not found under --bin-dir")
        service, service_trace = self.traced([str(switcher), "--service"], "switch-service")
        apps = []
        try:
            if not self.wait_for((self.runtime / "rmac" / "app-switcher.sock").exists, timeout=20.0):
                raise RuntimeError("rmac-app-switcher --service did not bind its socket")
            for name, app_id in (("rmac-calculator", "org.rmac.Calculator"), ("rmac-clock", "org.rmac.Clock")):
                binary = self.bin(name)
                if binary is None:
                    raise RuntimeError(f"{name} not found under --bin-dir")
                process, trace, _window = self._launch([str(binary)], f"switch-{name}", app_id)
                apps.append((process, trace, app_id))
        except BaseException:
            for process, _trace, _app in apps:
                self.stop(process)
            self.stop(service)
            raise
        return service, service_trace, apps

    def _switch_target(self, apps):
        before = self._focused_app_id()
        return next((a for a in apps if a[2] != before), None)

    def scenario_cmd_tab(self) -> dict[str, Any]:
        """A quick Cmd-Tab tap (Mod is Alt in a nested niri) between two
        apps: from the stroke to the newly focused app's first frame drawn
        active. The switcher's own first frame is reported too."""

        service, service_trace, apps = self._switcher_session()
        try:
            switches: list[Optional[float]] = []
            switcher_frames: list[Optional[float]] = []
            for _ in range(max(3, self.args.repeat)):
                target = self._switch_target(apps)
                if target is None:
                    break
                baseline = self._presents(target[1])
                service_baseline = self._presents(service_trace)
                start = time.monotonic()
                self.input.key("alt-tab")
                switcher_frames.append(self._wait_present(service_trace, service_baseline, start))
                presented = self._wait_present(target[1], baseline, start)
                focused = self.wait_for(lambda: self._focused_app_id() == target[2], timeout=3.0)
                switches.append(presented if focused else None)
                time.sleep(0.4)
                self._settle(target[1], 2.0)
            done = [s for s in switches if s is not None]
            frames = [f for f in switcher_frames if f is not None]
            switch_ms = statistics.median(done) if done else None
            return {"switches": switches, "switch_ms": switch_ms,
                    "switcher_first_frame_ms": statistics.median(frames) if frames else None,
                    "pass": switch_ms is not None and switch_ms <= 100.0,
                    **({} if done else {"error": "Alt-Tab never switched apps"})}
        finally:
            for process, _trace, _app in apps:
                self.stop(process)
            self.stop(service)

    def _wait_reveal(self, trace: Path, mark: int, start: float, timeout: float = 3.0) -> Optional[float]:
        """Harness-clock ms from `start` until the switcher presents its
        full-size panel: the first `present` after a `resize` row (the
        surface opens at 1 x 1 and grows when it reveals)."""

        deadline = start + timeout
        while time.monotonic() < deadline:
            resized = False
            for event, _micros in read_trace(trace)[mark:]:
                if event == "resize":
                    resized = True
                elif event == "present" and resized:
                    return (time.monotonic() - start) * 1000.0
            time.sleep(0.003)
        return None

    def scenario_cmd_tab_hold(self) -> dict[str, Any]:
        """Cmd-Tab with Cmd held, as when browsing the switcher: from the
        Tab stroke to the panel's first full-size frame (target < 50 ms),
        then from releasing Cmd to the chosen app's next frame."""

        service, service_trace, apps = self._switcher_session()
        try:
            reveals: list[Optional[float]] = []
            commits: list[Optional[float]] = []
            for _ in range(max(3, self.args.repeat)):
                target = self._switch_target(apps)
                if target is None:
                    break
                mark = self._mark(service_trace)
                start = time.monotonic()
                # niri's Mod is Alt in the nested session, but the switcher
                # reads ⌘ (Super) as the held modifier, as in the real
                # session: hold Super as well once the bind has fired.
                self.input.hold("alt-tab", extra=("cmd",))
                reveals.append(self._wait_reveal(service_trace, mark, start))
                time.sleep(0.3)
                baseline = self._presents(target[1])
                start = time.monotonic()
                self.input.release("alt-tab", extra=("cmd",))
                presented = self._wait_present(target[1], baseline, start)
                focused = self.wait_for(lambda: self._focused_app_id() == target[2], timeout=3.0)
                commits.append(presented if focused else None)
                time.sleep(0.4)
                self._settle(target[1], 2.0)
            shown = [r for r in reveals if r is not None]
            done = [c for c in commits if c is not None]
            reveal_ms = statistics.median(shown) if shown else None
            return {"reveals": reveals, "commits": commits, "reveal_ms": reveal_ms,
                    "release_to_switch_ms": statistics.median(done) if done else None,
                    "pass": reveal_ms is not None and reveal_ms <= 50.0 and bool(done)}
        finally:
            for process, _trace, _app in apps:
                self.stop(process)
            self.stop(service)

    def scenario_minimise_restore(self) -> dict[str, Any]:
        """Mod-M (`mission-control minimize`) parks the focused window; the
        Dock's restore moves it back and focuses it (the same two niri
        actions). Minimise is timed to niri reporting the move, restore to
        the app's next present."""

        binary = self.bin("rmac-calculator")
        mission_control = self.bin("rmac-mission-control", "mission-control")
        if binary is None or mission_control is None:
            raise RuntimeError("rmac-calculator / rmac-mission-control not found under --bin-dir")
        # `mission-control minimize` (Mod-M's spawn) asks the resident service.
        service = self.spawn([str(mission_control), "--service"], "minimise-mc-service")
        time.sleep(1.0)
        process, trace, window = self._launch([str(binary)], "minimise", "org.rmac.Calculator")
        try:
            workspace = window.get("workspace_id")
            index = next((int(w.get("idx", 1)) for w in self.niri_msg("workspaces") or []
                          if w.get("id") == workspace), 1)
            minimise: list[Optional[float]] = []
            restore: list[Optional[float]] = []
            for _ in range(self.args.repeat):
                start = time.monotonic()
                self.input.key("alt-m")
                moved = self.wait_for(
                    lambda: (self.window_by_app_id("org.rmac.Calculator") or {}).get("workspace_id") != workspace,
                    timeout=3.0, step=0.003)
                minimise.append((time.monotonic() - start) * 1000.0 if moved else None)
                time.sleep(0.5)
                baseline = self._presents(trace)
                start = time.monotonic()
                subprocess.run([self.args.niri, "msg", "action", "move-window-to-workspace", "--window-id",
                                str(window["id"]), "--focus", "false", str(index)],
                               env=self.env, capture_output=True, timeout=5)
                subprocess.run([self.args.niri, "msg", "action", "focus-window", "--id", str(window["id"])],
                               env=self.env, capture_output=True, timeout=5)
                restore.append(self._wait_present(trace, baseline, start))
                time.sleep(0.5)
                self._settle(trace, 2.0)
            done_min = [m for m in minimise if m is not None]
            done_res = [r for r in restore if r is not None]
            restore_ms = statistics.median(done_res) if done_res else None
            return {
                "minimise": minimise, "restore": restore,
                "minimise_ms": statistics.median(done_min) if done_min else None,
                "restore_ms": restore_ms,
                "pass": bool(done_min) and restore_ms is not None and restore_ms <= 100.0,
            }
        finally:
            self.stop(process)
            self.stop(service)

    def scenario_resize_drag(self) -> dict[str, Any]:
        """A left-edge resize drag of the Settings window: every configure
        re-lays the window out; scored on the frame budget. (Few frames: the
        press lands on the edge only part of the time; see
        `resize-drag-long` for niri's own interactive resize.)"""

        binary = self.bin("rmac-system-settings")
        if binary is None:
            raise RuntimeError("rmac-system-settings not found under --bin-dir")
        process, trace, window = self._launch([str(binary)], "resize", SETTINGS_APP_ID)
        try:
            layout = window.get("layout") or {}
            pos = layout.get("tile_pos_in_workspace_view") or [0, 0]
            size = layout.get("window_size") or layout.get("tile_size") or [800, 600]
            x, y = float(pos[0]), float(pos[1]) + float(size[1]) / 2
            mark = self._mark(trace)
            for offset in (-160, 160):
                self.input.drag((x, y), (x + offset, y), self.out_w, self.out_h, steps=30, step_delay=0.016)
                time.sleep(0.3)
                layout = (self.window_by_app_id(SETTINGS_APP_ID) or window).get("layout") or {}
                x = float((layout.get("tile_pos_in_workspace_view") or [x, 0])[0])
            self._settle(trace, 3.0)
            events = self._since(trace, mark)
            summary = {**frames_summary(events), **frame_split_ms(events)}
            final = (self.window_by_app_id(SETTINGS_APP_ID) or {}).get("layout") or {}
            summary["final_size"] = final.get("window_size")
            return summary
        finally:
            self.stop(process)

    def scenario_resize_drag_long(self) -> dict[str, Any]:
        """niri's interactive resize (Mod + right-drag; Mod is Alt in the
        nested session) of Settings: three seconds each way, wider then
        narrower, so every step is a real configure. Scored on the frame
        budget, with the configures and resizes counted (SPEED-12)."""

        binary = self.bin("rmac-system-settings")
        if binary is None:
            raise RuntimeError("rmac-system-settings not found under --bin-dir")
        process, trace, window = self._launch([str(binary)], "resize-long", SETTINGS_APP_ID)
        try:
            layout = window.get("layout") or {}
            pos = layout.get("tile_pos_in_workspace_view") or [0, 0]
            size = layout.get("window_size") or layout.get("tile_size") or [800, 600]
            # The right half, mid-height: niri resizes the right edge.
            x = float(pos[0]) + float(size[0]) * 0.8
            y = float(pos[1]) + float(size[1]) * 0.5
            start_size = list(size)
            mark = self._mark(trace)
            for offset in (240.0, -240.0):
                self.input.drag((x, y), (x + offset, y), self.out_w, self.out_h, button="right",
                                steps=180, step_delay=0.016, modifiers=["alt"])
                time.sleep(0.3)
            self._settle(trace, 3.0)
            events = self._since(trace, mark)
            summary = {**frames_summary(events), **frame_split_ms(events)}
            summary["configures"] = sum(1 for event, _ in events if event == "configure")
            summary["resizes"] = sum(1 for event, _ in events if event == "resize")
            summary["start_size"] = start_size
            final = (self.window_by_app_id(SETTINGS_APP_ID) or {}).get("layout") or {}
            summary["final_size"] = final.get("window_size")
            return summary
        finally:
            self.stop(process)

    def scenario_mission_control_animation(self) -> dict[str, Any]:
        mission_control = self.bin("rmac-mission-control", "mission-control")
        if mission_control is None:
            raise RuntimeError("rmac-mission-control not found under --bin-dir")
        helpers = []
        process, trace = self.traced([str(mission_control), "--service"], "mc-animation")
        try:
            for name, app_id in (("rmac-calculator", "org.rmac.Calculator"), ("rmac-clock", "org.rmac.Clock")):
                binary = self.bin(name)
                if binary is not None:
                    helpers.append(self.spawn([str(binary)], f"mc-helper-{name}"))
                    self.wait_for(lambda app_id=app_id: self.window_by_app_id(app_id), timeout=20.0)
            time.sleep(1.0)
            all_events: Events = []
            for _ in range(self.args.repeat):
                mark = self._mark(trace)
                self.input.key("ctrl-up")
                time.sleep(0.2)
                self._settle(trace, 3.0)
                self.input.key("escape")
                time.sleep(0.2)
                self._settle(trace, 3.0)
                all_events.extend(self._since(trace, mark))
            return frames_summary(all_events)
        finally:
            self.stop(process)
            for helper in helpers:
                self.stop(helper)


# Scenario name (for --only) -> InteractionScenarios method.
SCENARIOS: dict[str, str] = {
    "files-list-scroll": "scenario_files_list_scroll",
    "files-icon-scroll": "scenario_files_icon_scroll",
    "files-open-folder": "scenario_files_open_folder",
    "files-thumbnails": "scenario_files_thumbnails",
    "quick-look": "scenario_quick_look",
    "terminal-scrollback": "scenario_terminal_scrollback",
    "terminal-typing": "scenario_terminal_typing",
    "spotlight-typing": "scenario_spotlight_typing",
    "notes-list-scroll": "scenario_notes_list_scroll",
    "notes-typing": "scenario_notes_typing",
    "mail-list-scroll": "scenario_mail_list_scroll",
    "settings-pane-scroll": "scenario_settings_pane_scroll",
    "text-editor-plain-typing": "scenario_text_editor_plain_typing",
    "text-editor-rich-typing": "scenario_text_editor_rich_typing",
    "cmd-tab": "scenario_cmd_tab",
    "cmd-tab-hold": "scenario_cmd_tab_hold",
    "minimise-restore": "scenario_minimise_restore",
    "resize-drag": "scenario_resize_drag",
    "resize-drag-long": "scenario_resize_drag_long",
    "mission-control-animation": "scenario_mission_control_animation",
}
