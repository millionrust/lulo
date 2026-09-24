#!/usr/bin/env python3
"""Assert the passive layer Dock's bounded semantic projection."""

import os
import time

import pyatspi


EXPECTED_DOCKS = int(os.environ.get("RMAC_EXPECTED_DOCKS", "2"))
if not 1 <= EXPECTED_DOCKS <= 16:
    raise ValueError("RMAC_EXPECTED_DOCKS must be between 1 and 16")


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
        if node.getRoleName() == "tool bar" and node.name == "Dock"
    ]
    if len(docks) == EXPECTED_DOCKS:
        break
    time.sleep(0.1)

if len(docks) != EXPECTED_DOCKS:
    raise AssertionError(
        f"expected {EXPECTED_DOCKS} accessible Docks, found {len(docks)}"
    )

for dock in docks:
    nodes = list(descendants(dock))
    buttons = [
        node for node in nodes if node.getRoleName() in {"button", "push button"}
    ]
    names = [button.name for button in buttons]
    trash_names = [name for name in names if name == "Trash" or name.startswith("Trash, ")]
    # Folder/file stacks (§ folder/file stacks left of the Trash) are named
    # "<name>, stack" (Downloads' being "Downloads, stack"), the same
    # convention Trash and other place buttons already use for their state.
    # No stack is expected in every Dock, so this only checks the ones that
    # do appear are well-formed and still counted as buttons.
    stack_names = [name for name in names if name.endswith(", stack")]
    if (
        len(buttons) < 2
        or len(trash_names) != 1
        or any(not name for name in names)
        or any(name == ", stack" for name in stack_names)
    ):
        roles = [(node.getRoleName(), node.name) for node in nodes]
        raise AssertionError(
            "a Dock must expose at least one named application and exactly one Trash; "
            f"found {roles!r}"
        )
    focusable = [
        node
        for node in nodes
        if node.getState().contains(pyatspi.STATE_FOCUSABLE)
    ]
    if focusable:
        raise AssertionError("the non-keyboard-interactive Dock added focus stops")

print(f"AT-SPI exposed {EXPECTED_DOCKS} passive Docks with named app and Trash actions")
