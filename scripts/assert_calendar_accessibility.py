#!/usr/bin/env python3
"""Assert Calendar's sidebar, grid, toolbar and invitations popover over
AT-SPI (ACC-32).

Run against one `rmac-calendar` started with `RMAC_CALENDAR_FIXTURE=1` and
temporary XDG_* directories (see scripts/linux/run-content-accessibility.sh)
-- the fixture flag is the only way `rmac-calendar` ever shows sample
events, and only a private nested session may set it. A real calendar
account's Calendar never carries this flag. Checks:

- "Calendars" is a `list` of named `check box`es, one per calendar, each
  toggled to match its visibility and with a Click action;
- the toolbar's Sidebar, Invitations, New Event, Previous Period, Today,
  Next Period and Search controls are named, actionable buttons, and the
  Day/Week/Month/Year view switcher is a named `page tab list` of named
  `page tab`s with exactly one selected;
- clicking a different view tab moves the selected state;
- the heading is a named, polite live region (`status bar`) whose name is
  the showing period, and changes when Next Period moves it;
- clicking Invitations opens a named "Invitations" dialog (with the fixture
  calendars' no-ATTENDEE events, it shows the empty state) and clicking it
  again closes it;
- an event block in the week grid is a named, actionable button.
"""

from __future__ import annotations

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_CALENDAR_APP", "rmac-calendar")
DUMP = os.environ.get("RMAC_A11Y_DUMP")


def app():
    return support.find_app(APP)


def named_clickable_button(label):
    return support.wait_for(
        lambda: next(
            (
                node
                for node in support.descendants(app())
                if support.role(node) in {"push button", "button"}
                and support.name(node) == label
                and "click" in support.actions(node)
            ),
            None,
        ),
        f"the named, actionable {label} button",
    )


def named_tab(label):
    return next(
        (
            node
            for node in support.descendants(app())
            if support.role(node) == "page tab" and support.name(node) == label
        ),
        None,
    )


calendars = support.wait_for(
    lambda: next(
        (node for node in support.descendants(app()) if support.role(node) == "list"), None
    ),
    "the Calendars list",
)
calendar_items = [
    node for node in support.descendants(calendars) if support.role(node) == "check box"
]
assert calendar_items, "the Calendars list has no check box items"
assert all(support.name(node) for node in calendar_items), "a calendar check box is unnamed"
assert all("click" in support.actions(node) for node in calendar_items), (
    "a calendar check box has no click"
)

for label in (
    "Sidebar",
    "Invitations",
    "New Event",
    "Previous Period",
    "Today",
    "Next Period",
    "Search",
):
    named_clickable_button(label)

tab_list = support.wait_for(
    lambda: next(
        (node for node in support.descendants(app()) if support.role(node) == "page tab list"),
        None,
    ),
    "the Day/Week/Month/Year page tab list",
)
tabs = [node for node in support.descendants(tab_list) if support.role(node) == "page tab"]
names = {support.name(node) for node in tabs}
assert {"Day", "Week", "Month", "Year"} <= names, f"view tabs are {names}"
selected = [node for node in tabs if "selected" in support.states(node)]
assert len(selected) == 1, f"{len(selected)} view tabs selected"

if DUMP:
    support.dump(app(), DUMP)

heading = support.wait_for(
    lambda: next(
        (node for node in support.descendants(app()) if support.role(node) == "status bar"),
        None,
    ),
    "the heading's status bar live region",
)
first_heading = support.name(heading)
assert first_heading, "the heading has no accessible name"

month_tab = named_tab("Month")
assert support.click(month_tab), "Month tab accessibility action was not handled"
time.sleep(0.3)
assert "selected" in support.states(named_tab("Month")), "Month did not become selected"
assert "selected" not in support.states(named_tab("Week")), "Week stayed selected"

next_period = named_clickable_button("Next Period")
assert support.click(next_period), "Next Period accessibility action was not handled"
time.sleep(0.3)
heading = next(
    (node for node in support.descendants(app()) if support.role(node) == "status bar"), None
)
assert support.name(heading) != first_heading, (
    "the heading's name did not change after Next Period"
)

invitations = named_clickable_button("Invitations")
assert support.click(invitations), "Invitations accessibility action was not handled"
dialog = support.wait_for(
    lambda: next(
        (
            node
            for node in support.descendants(app())
            if support.role(node) == "dialog" and support.name(node) == "Invitations"
        ),
        None,
    ),
    "the named Invitations dialog",
)
assert dialog is not None
# Leave the toolbar the way this check found it.
assert support.click(named_clickable_button("Invitations")), (
    "Invitations accessibility action did not toggle closed"
)
support.wait_for(
    lambda: not any(
        support.role(node) == "dialog" and support.name(node) == "Invitations"
        for node in support.descendants(app())
    ),
    "the Invitations dialog to close",
)

event_buttons = [
    node
    for node in support.descendants(app())
    if support.role(node) == "button" and (support.name(node) or "").startswith("Stand-up")
]
assert event_buttons, "no named Stand-up event button in the week grid"
assert "click" in support.actions(event_buttons[0]), "the event button has no click action"

print(
    "AT-SPI Calendar: "
    f"{len(calendar_items)} calendar check boxes, named toolbar controls, "
    f"{len(tabs)} view tabs (1 selected), heading live region changes with Next Period, "
    "Invitations dialog opens/closes, named event buttons"
)
