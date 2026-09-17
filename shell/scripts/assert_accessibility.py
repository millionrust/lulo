#!/usr/bin/python3
"""Assert the GPUI lab's semantic tree and AT-SPI actions."""

import sys
import time

import pyatspi


TIMEOUT_SECONDS = 20


def descendants(node):
    yield node
    try:
        for index in range(node.childCount):
            yield from descendants(node.getChildAtIndex(index))
    except (LookupError, RuntimeError):
        return


def snapshot():
    desktop = pyatspi.Registry.getDesktop(0)
    return [node for app in desktop for node in descendants(app)]


def wait_for_name(name):
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        for node in snapshot():
            try:
                if node.name == name:
                    return node
            except (LookupError, RuntimeError):
                continue
        time.sleep(0.25)
    raise AssertionError(f"AT-SPI node did not appear: {name!r}")


def assert_role(node, expected):
    actual = node.getRoleName()
    assert actual == expected, f"{node.name!r}: expected role {expected!r}, got {actual!r}"


def wait_until(predicate, description):
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.25)
    raise AssertionError(f"timed out waiting for {description}")


def state_names(node):
    states = node.getState()
    return {
        pyatspi.stateToString(index)
        for index in range(pyatspi.STATE_LAST_DEFINED)
        if states.contains(index)
    }


def click(node):
    actions = node.queryAction()
    names = [actions.getName(index) for index in range(actions.nActions)]
    assert "click" in names, f"{node.name!r}: missing click action; found {names!r}"
    assert actions.doAction(names.index("click")), f"{node.name!r}: click action failed"


def main():
    heading = wait_for_name("Accessibility gate")
    counter = wait_for_name("Counter: 0")
    switch = wait_for_name("Enable experimental feature")

    assert_role(heading, "heading")
    assert_role(counter, "spin button")
    assert_role(switch, "toggle button")

    value = counter.queryValue()
    assert value.currentValue == 0.0, f"unexpected initial counter: {value.currentValue}"
    click(counter)
    wait_until(
        lambda: counter.queryValue().currentValue == 1.0,
        "counter value to become 1 after its AT-SPI click action",
    )

    assert "pressed" not in state_names(switch), "switch unexpectedly starts pressed"
    click(switch)
    wait_until(
        lambda: "pressed" in state_names(switch),
        "switch to expose its pressed state after its AT-SPI click action",
    )

    print("AT-SPI roles, names, numeric value, actions, and toggled state passed")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - print diagnostics for CI
        print(f"accessibility assertion failed: {error}", file=sys.stderr)
        for node in snapshot():
            try:
                name = node.name
                if name:
                    print(f"  {node.getRoleName()}: {name}", file=sys.stderr)
            except (LookupError, RuntimeError):
                pass
        raise
