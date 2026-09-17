#!/usr/bin/env python3
"""Assert the current top bar's bounded semantic projection."""

import os
import time

import pyatspi


EXPECTED_BARS = int(os.environ.get("RMAC_EXPECTED_TOP_BARS", "2"))
if not 1 <= EXPECTED_BARS <= 16:
    raise ValueError("RMAC_EXPECTED_TOP_BARS must be between 1 and 16")


def descendants(node):
    yield node
    for index in range(node.childCount):
        yield from descendants(node.getChildAtIndex(index))


desktop = pyatspi.Registry.getDesktop(0)
bars = []
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    bars = [
        node
        for node in descendants(desktop)
        if node.getRoleName() == "tool bar" and node.name == "rmac top bar"
    ]
    if len(bars) == EXPECTED_BARS:
        break
    time.sleep(0.1)

if len(bars) != EXPECTED_BARS:
    raise AssertionError(
        f"expected {EXPECTED_BARS} accessible top bars, found {len(bars)}"
    )

for bar in bars:
    nodes = list(descendants(bar))
    buttons = [
        node for node in nodes if node.getRoleName() in {"button", "push button"}
    ]
    names = [button.name for button in buttons]
    required = {"Spotlight", "Control Center"}
    missing = required.difference(names)
    clocks = [name for name in names if name.startswith("Date and time:")]
    if missing or len(clocks) != 1 or any(not name for name in names):
        roles = [(node.getRoleName(), node.name) for node in nodes]
        raise AssertionError(
            "top bar must expose named Spotlight, Control Center, and one "
            f"date/time button; missing={sorted(missing)!r}, found={roles!r}"
        )
    focusable = [
        node
        for node in nodes
        if node.getState().contains(pyatspi.STATE_FOCUSABLE)
    ]
    if focusable:
        raise AssertionError("passive top bar must not add keyboard focus stops")

print(
    f"AT-SPI exposed {EXPECTED_BARS} passive top bars with current actionable semantics"
)
