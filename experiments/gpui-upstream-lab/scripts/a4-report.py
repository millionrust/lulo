#!/usr/bin/env python3
"""Create or verify the bounded real-hardware GPUI A4 result report."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import stat
import sys
from typing import Sequence


REPORT_VERSION = "1"
PINNED_UPSTREAM_REVISION = "76c93968da5b8b8809bdd72e4ad9e7d0e946bad0"
MAX_REPORT_BYTES = 16 * 1024
VALID_RESULTS = frozenset(("pending", "pass", "fail"))
REQUIRED_RESULTS = (
    "environment.reviewed",
    "automation.wayland-clippy",
    "automation.nested-smoke-live-revision",
    "gnome.orca-nodes",
    "gnome.orca-focus-order",
    "gnome.orca-actions-state",
    "gnome.scale-100",
    "gnome.scale-125",
    "gnome.scale-150",
    "gnome.scale-200",
    "gnome.mixed-scale-move",
    "niri.layer-placement",
    "niri.layer-exclusive-zone",
    "niri.layer-keyboard-noninterference",
    "niri.topbar-per-output",
    "niri.topbar-orca-semantics",
    "niri.output-hotplug",
    "niri.fractional-scale",
    "niri.fullscreen-overview",
    "niri.idle-redraw",
    "niri.interaction-30m",
    "niri.soak-4h",
    "privacy.reviewed-evidence",
)


class ReportError(ValueError):
    """A privacy-safe structural report error."""


@dataclass(frozen=True)
class ReportSummary:
    pending: tuple[str, ...]
    failed: tuple[str, ...]

    @property
    def recording_complete(self) -> bool:
        return not self.pending

    @property
    def all_passed(self) -> bool:
        return not self.pending and not self.failed


def report_template() -> str:
    lines = [
        f"rmac-upstream-a4-report={REPORT_VERSION}",
        f"revision={PINNED_UPSTREAM_REVISION}",
    ]
    lines.extend(f"result.{result}=pending" for result in REQUIRED_RESULTS)
    report = "\n".join(lines) + "\n"
    if len(report.encode("ascii")) > MAX_REPORT_BYTES:
        raise AssertionError("static A4 report exceeds its resource contract")
    return report


def parse_report(text: str) -> tuple[str, ...]:
    try:
        encoded = text.encode("ascii")
    except UnicodeEncodeError as error:
        raise ReportError("report must contain only the documented ASCII fields") from error
    if len(encoded) > MAX_REPORT_BYTES:
        raise ReportError("report exceeds the 16 KiB limit")
    if not text.endswith("\n"):
        raise ReportError("report must end with one newline")

    lines = text.splitlines()
    expected_keys = (
        "rmac-upstream-a4-report",
        "revision",
        *(f"result.{result}" for result in REQUIRED_RESULTS),
    )
    if len(lines) != len(expected_keys):
        raise ReportError("report has missing or extra fields")

    values: list[str] = []
    for line, expected_key in zip(lines, expected_keys):
        key, separator, value = line.partition("=")
        if not separator or key != expected_key:
            raise ReportError(f"expected field {expected_key}")
        if not value:
            raise ReportError(f"field {expected_key} is empty")
        values.append(value)

    if values[0] != REPORT_VERSION:
        raise ReportError("unsupported report version")
    if values[1] != PINNED_UPSTREAM_REVISION:
        raise ReportError("report revision does not match the pinned experiment")
    for result_id, status in zip(REQUIRED_RESULTS, values[2:]):
        if status not in VALID_RESULTS:
            raise ReportError(f"result {result_id} must be pending, pass, or fail")
    return tuple(values[2:])


def summarize(statuses: Sequence[str]) -> ReportSummary:
    if len(statuses) != len(REQUIRED_RESULTS):
        raise ReportError("result cardinality does not match the A4 contract")
    pending = tuple(
        result_id
        for result_id, status in zip(REQUIRED_RESULTS, statuses)
        if status == "pending"
    )
    failed = tuple(
        result_id
        for result_id, status in zip(REQUIRED_RESULTS, statuses)
        if status == "fail"
    )
    return ReportSummary(pending=pending, failed=failed)


def verification_exit_code(summary: ReportSummary) -> int:
    if summary.pending:
        return 4
    if summary.failed:
        return 5
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("create", help="print a new all-pending report")
    verify = subparsers.add_parser("verify", help="verify a completed report")
    verify.add_argument("report", type=Path)
    return parser.parse_args()


def read_report(path: Path) -> str:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as error:
        raise ReportError("report could not be opened without following links") from error
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise ReportError("report must be a regular non-symlink file")
        if metadata.st_size > MAX_REPORT_BYTES:
            raise ReportError("report exceeds the 16 KiB limit")
        with os.fdopen(descriptor, "rb", closefd=True) as report:
            descriptor = -1
            payload = report.read(MAX_REPORT_BYTES + 1)
        if len(payload) > MAX_REPORT_BYTES:
            raise ReportError("report exceeds the 16 KiB limit")
        return payload.decode("ascii")
    except UnicodeDecodeError as error:
        raise ReportError("report must contain only the documented ASCII fields") from error
    except OSError as error:
        raise ReportError("report could not be read") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def main() -> int:
    args = parse_args()
    if args.command == "create":
        sys.stdout.write(report_template())
        return 0

    try:
        statuses = parse_report(read_report(args.report))
        summary = summarize(statuses)
    except ReportError as error:
        print(f"A4 report invalid: {error}", file=sys.stderr)
        return 3

    exit_code = verification_exit_code(summary)
    if exit_code == 4:
        print(
            "A4 report incomplete; pending: " + ", ".join(summary.pending),
            file=sys.stderr,
        )
        return exit_code
    if exit_code == 5:
        print(
            "A4 report records failed gates: " + ", ".join(summary.failed),
            file=sys.stderr,
        )
        return exit_code

    print("a4_report=pass")
    print(f"revision={PINNED_UPSTREAM_REVISION}")
    print(f"result_count={len(REQUIRED_RESULTS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
