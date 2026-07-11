#!/usr/bin/env python3
import time

import pyatspi


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
    if len(bars) == 2:
        break
    time.sleep(0.1)

if len(bars) != 2:
    raise AssertionError(f"expected two accessible top bars, found {len(bars)}")

for bar in bars:
    nodes = list(descendants(bar))
    # AccessKit maps its semantic Time role to AT-SPI's Static role because
    # AT-SPI has no dedicated time role. The accessible name carries the value.
    clocks = [
        node
        for node in nodes
        if node.getRoleName() == "static" and node.name
    ]
    if len(clocks) != 1 or not clocks[0].name:
        roles = [(node.getRoleName(), node.name) for node in nodes]
        raise AssertionError(
            f"top bar must expose exactly one named static clock node; found {roles!r}"
        )
    focusable = [
        node
        for node in nodes
        if node.getState().contains(pyatspi.STATE_FOCUSABLE)
    ]
    if focusable:
        raise AssertionError("static top bar must not add keyboard focus stops")
