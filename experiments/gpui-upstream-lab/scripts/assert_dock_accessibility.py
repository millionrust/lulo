#!/usr/bin/env python3
"""Assert the passive layer Dock's bounded semantic projection."""

import time

import pyatspi


def descendants(node):
    yield node
    try:
        for index in range(node.childCount):
            yield from descendants(node.getChildAtIndex(index))
    except (LookupError, RuntimeError):
        return


desktop = pyatspi.Registry.getDesktop(0)
docks = []
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    docks = [
        node
        for node in descendants(desktop)
        if node.getRoleName() == "tool bar" and node.name == "rmac Dock"
    ]
    if len(docks) == 2:
        break
    time.sleep(0.1)

if len(docks) != 2:
    raise AssertionError(f"expected two accessible Docks, found {len(docks)}")

for dock in docks:
    nodes = list(descendants(dock))
    buttons = [node for node in nodes if node.getRoleName() == "push button"]
    if len(buttons) != 6 or any(not button.name for button in buttons):
        roles = [(node.getRoleName(), node.name) for node in nodes]
        raise AssertionError(
            "a fresh-profile Dock must expose six named application buttons; "
            f"found {roles!r}"
        )
    focusable = [
        node
        for node in nodes
        if node.getState().contains(pyatspi.STATE_FOCUSABLE)
    ]
    if focusable:
        raise AssertionError("the non-keyboard-interactive Dock added focus stops")
