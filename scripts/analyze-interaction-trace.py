#!/usr/bin/env python3
"""Summarize a normalized, platform-native 60-second interaction trace.

This tool analyzes exported observations only. It does not inject input or
establish that a trace came from native instrumentation; review provenance
before using its output as release evidence.

Input format (JSON):
  {"format":1,"refresh_hz":60,"start_ns":0,"end_ns":60000000000,
   "interactions":[{"id":"1","input_ns":10,"visible_ns":100}],
   "frames":[{"at_ns":100,"duration_ns":1000000,"missed":false}]}
Each interaction pairs a native input event with the first visible response
presentation. Frame durations and missed flags must come from the same native
trace window and output.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import stat
import sys


MAX_BYTES = 16 * 1024 * 1024
MIN_INTERACTIONS = 20
WINDOW_NS = 60_000_000_000


class TraceError(ValueError):
    pass


def _integer(value: object, field: str, *, positive: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < (1 if positive else 0):
        raise TraceError(f"{field} must be a {'positive' if positive else 'non-negative'} integer")
    return value


def nearest_rank(values: list[float], percentile: float) -> float:
    if not values:
        raise TraceError("trace contains no samples")
    ordered = sorted(values)
    return ordered[math.ceil(percentile * len(ordered)) - 1]


def analyze(document: object) -> dict[str, object]:
    if not isinstance(document, dict) or set(document) != {
        "end_ns", "format", "frames", "interactions", "refresh_hz", "start_ns"
    }:
        raise TraceError("trace fields are not exact")
    if document["format"] != 1:
        raise TraceError("unsupported trace format")
    refresh_hz = document["refresh_hz"]
    if isinstance(refresh_hz, bool) or refresh_hz not in (60, 120):
        raise TraceError("refresh_hz must be 60 or 120")
    start = _integer(document["start_ns"], "start_ns")
    end = _integer(document["end_ns"], "end_ns")
    if end - start < WINDOW_NS:
        raise TraceError("trace window must cover at least 60 seconds")

    interactions = document["interactions"]
    frames = document["frames"]
    if not isinstance(interactions, list) or len(interactions) < MIN_INTERACTIONS:
        raise TraceError(f"at least {MIN_INTERACTIONS} interactions are required")
    # The input is an output-frame trace, not an app redraw trace. A sparse
    # export must not claim a good missed-frame percentage by omitting slots.
    expected_frames = ((end - start) * refresh_hz + 999_999_999) // 1_000_000_000
    required_frames = (expected_frames * 99 + 99) // 100
    if not isinstance(frames, list) or len(frames) < required_frames:
        raise TraceError(f"at least {required_frames} output-frame samples are required")

    seen_ids: set[str] = set()
    latencies_ms: list[float] = []
    for sample in interactions:
        if not isinstance(sample, dict) or set(sample) != {"id", "input_ns", "visible_ns"}:
            raise TraceError("interaction sample fields are not exact")
        sample_id = sample["id"]
        if not isinstance(sample_id, str) or not sample_id or len(sample_id) > 80 or sample_id in seen_ids:
            raise TraceError("interaction IDs must be unique non-empty strings of at most 80 characters")
        seen_ids.add(sample_id)
        input_ns = _integer(sample["input_ns"], "input_ns")
        visible_ns = _integer(sample["visible_ns"], "visible_ns")
        if not start <= input_ns <= visible_ns <= end:
            raise TraceError("interaction timestamps fall outside the trace window or are reversed")
        latencies_ms.append((visible_ns - input_ns) / 1_000_000)

    frame_ms: list[float] = []
    missed = 0
    previous_at: int | None = None
    for sample in frames:
        if not isinstance(sample, dict) or set(sample) != {"at_ns", "duration_ns", "missed"}:
            raise TraceError("frame sample fields are not exact")
        at_ns = _integer(sample["at_ns"], "frame at_ns")
        if not start <= at_ns <= end:
            raise TraceError("frame timestamp falls outside the trace window")
        if previous_at is not None and at_ns <= previous_at:
            raise TraceError("frame timestamps must increase")
        previous_at = at_ns
        duration_ns = _integer(sample["duration_ns"], "duration_ns", positive=True)
        if not isinstance(sample["missed"], bool):
            raise TraceError("frame missed must be a boolean")
        frame_ms.append(duration_ns / 1_000_000)
        missed += sample["missed"]
    frame_period_ns = 1_000_000_000 / refresh_hz
    if frames[0]["at_ns"] > start + 2 * frame_period_ns or frames[-1]["at_ns"] < end - 2 * frame_period_ns:
        raise TraceError("frame samples do not span the trace window")

    input_p95 = nearest_rank(latencies_ms, 0.95)
    frame_p95 = nearest_rank(frame_ms, 0.95)
    # An omitted output slot is at least as concerning as a marked miss.
    # Include the shortfall rather than letting a sparse export improve the
    # reported percentage.
    unreported_frames = max(expected_frames - len(frames), 0)
    missed_percent = (missed + unreported_frames) * 100 / max(expected_frames, len(frames))
    frame_budget_ms = 16 if refresh_hz == 60 else 8
    result = {
        "format": 1,
        "measurement_status": "unverified_trace_summary",
        "native_trace_review_required": True,
        "refresh_hz": refresh_hz,
        "sample_counts": {
            "expected_frames": expected_frames,
            "frames": len(frames),
            "interactions": len(interactions),
            "unreported_frames": unreported_frames,
        },
        "metrics": {
            "input_to_visible_p95_ms": round(input_p95, 3),
            "frame_p95_ms": round(frame_p95, 3),
            "missed_frames_percent": round(missed_percent, 3),
        },
        "limits": {
            "input_to_visible_p95_ms": 50,
            "frame_p95_ms": frame_budget_ms,
            "missed_frames_percent": 1,
        },
        "within_limits": (
            input_p95 <= 50 and frame_p95 <= frame_budget_ms and missed_percent <= 1
        ),
    }
    return result


def read_trace(path: Path) -> object:
    try:
        with os.fdopen(os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)), "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_BYTES:
                raise TraceError("trace must be a regular file no larger than 16 MiB")
            raw = stream.read(MAX_BYTES + 1)
            after = os.fstat(stream.fileno())
        if len(raw) > MAX_BYTES:
            raise TraceError("trace must be a regular file no larger than 16 MiB")
        if (len(raw), before.st_size, before.st_mtime_ns) != (after.st_size, after.st_size, after.st_mtime_ns):
            raise TraceError("trace changed while being read")
        return json.loads(raw)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise TraceError("trace is unreadable or invalid JSON") from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path, help="normalized native trace JSON")
    args = parser.parse_args()
    try:
        result = analyze(read_trace(args.trace))
    except TraceError as error:
        print(f"interaction trace: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["within_limits"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
