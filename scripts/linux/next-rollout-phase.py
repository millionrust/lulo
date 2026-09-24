#!/usr/bin/env python3
"""Decide rollout.yml's next Phased-Update-Percentage step, if any.

Normal rollout only ever moves forward through 10 -> 25 -> 50 -> 100, with at
least 24 hours of observation between steps (docs/update-trust.md "Staged
rollout and rollback"). A manual run can additionally request 0, which halts
a rollout immediately regardless of timing; a scheduled tick never resumes a
halted rollout on its own, since that would turn an operator's deliberate
stop into something a cron job undoes.

This script makes no network call and touches no repository: it is pure
decision logic over values the caller already read from the live
repository's InRelease Date and Packages index, kept separate so it can be
tested without a live Pages site.
"""

from __future__ import annotations

import argparse
import sys


NORMAL_PERCENTAGES = (10, 25, 50, 100)
ALL_PERCENTAGES = (0,) + NORMAL_PERCENTAGES
MINIMUM_OBSERVATION_SECONDS = 24 * 3600


class RolloutError(RuntimeError):
    """An invalid rollout decision input."""


def next_phase(
    *,
    current_phase: int,
    current_date_seconds: int,
    now_seconds: int,
    requested_phase: int | None,
) -> int | None:
    if current_phase not in ALL_PERCENTAGES:
        raise RolloutError("current phase is not a recognized percentage")
    if now_seconds < current_date_seconds:
        raise RolloutError("now is before the live repository's Date")

    if requested_phase is not None:
        if requested_phase not in ALL_PERCENTAGES:
            raise RolloutError("requested phase is not a recognized percentage")
        if requested_phase == 0:
            # An explicit halt is always allowed, immediately.
            return 0
        if requested_phase < current_phase:
            raise RolloutError(
                "a manual step cannot move the rollout backward except to 0"
            )
        return requested_phase

    # Unattended (scheduled) tick: never resume a halt, and never advance
    # past 100.
    if current_phase in (0, 100):
        return None
    elapsed = now_seconds - current_date_seconds
    if elapsed < MINIMUM_OBSERVATION_SECONDS:
        return None
    position = NORMAL_PERCENTAGES.index(current_phase)
    return NORMAL_PERCENTAGES[position + 1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--current-phase", required=True, type=int)
    parser.add_argument("--current-date-seconds", required=True, type=int)
    parser.add_argument("--now-seconds", required=True, type=int)
    parser.add_argument("--requested-phase", type=int, default=None)
    arguments = parser.parse_args()
    try:
        result = next_phase(
            current_phase=arguments.current_phase,
            current_date_seconds=arguments.current_date_seconds,
            now_seconds=arguments.now_seconds,
            requested_phase=arguments.requested_phase,
        )
    except RolloutError as error:
        parser.exit(2, f"next-rollout-phase: {error}\n")
    if result is None:
        print("no-step")
        return 0
    print(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
