#!/usr/bin/env python3
"""Assert Clock's tabs and alarm repeat controls over live AT-SPI.

Run against an isolated rmac-clock instance from run-content-accessibility.sh.
Clicking tabs uses AT-SPI Action.doAction, without injecting pointer input.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = "rmac-clock"
LABELS = ("World Clock", "Alarms", "Stopwatch", "Timers")
REPEAT_DAYS = (
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"
)


def tabs():
    root = support.find_app(APP)
    if root is None:
        return None
    groups = support.nodes_with(root, "page tab list", "Clock tabs")
    if len(groups) != 1:
        return None
    pages = support.nodes_with(groups[0], "page tab")
    if [support.name(page) for page in pages] != list(LABELS):
        return None
    return root, groups[0], pages


root, _, pages = support.wait_for(tabs, "Clock's four named toolbar tabs")
if os.environ.get("RMAC_A11Y_DUMP"):
    support.dump(root, os.environ["RMAC_A11Y_DUMP"])

assert all("click" in support.actions(page) for page in pages), "a Clock tab is not actionable"


def selected_name():
    current = tabs()
    if current is None:
        return None
    selected = [support.name(page) for page in current[2] if "selected" in support.states(page)]
    return selected[0] if len(selected) == 1 else None


support.wait_for(lambda: selected_name() == "World Clock", "World Clock to be selected")
for label in ("Alarms", "Stopwatch", "Timers", "World Clock"):
    current = tabs()
    assert current is not None, "Clock's tab list disappeared"
    page = next(page for page in current[2] if support.name(page) == label)
    assert support.click(page), f"AT-SPI click on {label} returned false"
    support.wait_for(lambda: selected_name() == label, f"{label} to become the selected tab")

alarms = next(page for page in tabs()[2] if support.name(page) == "Alarms")
assert support.click(alarms), "AT-SPI click on Alarms returned false"
support.wait_for(lambda: selected_name() == "Alarms", "Alarms to become selected")


def add_alarm_button():
    app = support.find_app(APP)
    buttons = support.nodes_with(app, "push button", "Add an alarm") if app else []
    return buttons[0] if len(buttons) == 1 else None


add = support.wait_for(add_alarm_button, "named Add an alarm button")
assert "click" in support.actions(add), "Add an alarm button is not actionable"
assert support.click(add), "AT-SPI click on Add an alarm returned false"


def repeat_days():
    app = support.find_app(APP)
    if app is None:
        return None
    days = [
        node
        for node in support.descendants(app)
        if support.role(node) == "check box"
        and support.name(node) in REPEAT_DAYS
    ]
    return days if tuple(support.name(day) for day in days) == REPEAT_DAYS else None


days = support.wait_for(repeat_days, "seven Sunday-first alarm repeat checkboxes")
assert all("click" in support.actions(day) for day in days), "a repeat day is not actionable"
assert "checked" not in support.states(days[0]), "Sunday starts checked on a new alarm"
assert support.click(days[0]), "AT-SPI click on Sunday returned false"
support.wait_for(
    lambda: "checked" in support.states(repeat_days()[0]) if repeat_days() else False,
    "Sunday to become checked",
)

print("AT-SPI Clock: tabs and Sunday-first alarm repeat checkboxes have actions and state")
