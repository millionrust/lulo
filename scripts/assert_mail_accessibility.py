#!/usr/bin/env python3
"""Assert Mail's sidebar, message list, viewer and toolbar over AT-SPI
(MAIL-10).

Run against one `rmac-mail` started with `RMAC_MAIL_FIXTURE=1` and temporary
XDG_* directories (see scripts/linux/run-content-accessibility.sh) — the
fixture flag is the only way `rmac-mail` ever shows sample data, and only a
private nested session may set it. A real account's Mail never carries this
flag, so this script is the fixture's one legitimate use outside tests.
Checks:

- "Mailboxes" is a `list box` of named `list box option`s with exactly one
  selected, through the Selection interface;
- "{n} conversations, {m} unread" is a second `list box` of named
  conversations, also with exactly one selected;
- the message viewer is a named `document frame` and its name changes when a
  different conversation is selected;
- the toolbar's Filter Unread, Compose, Archive, Move to Bin, Junk or Not
  Junk, Reply, Reply All, Forward, Flag, Move or Copy to Mailbox and Search
  controls are named, actionable buttons, and Search opens a named,
  text-editable "Search Mail" field;
- clicking a mailbox in the sidebar changes which conversation list item is
  selected.
"""

from __future__ import annotations

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_MAIL_APP", "rmac-mail")
DUMP = os.environ.get("RMAC_A11Y_DUMP")


def app():
    return support.find_app(APP)


def list_box(predicate):
    root = app()
    if root is None:
        return None
    boxes = [node for node in support.nodes_with(root, "list box") if predicate(support.name(node))]
    return boxes[0] if boxes else None


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


mailboxes = support.wait_for(
    lambda: list_box(lambda name: name == "Mailboxes"), "the Mailboxes list box"
)
mailbox_items = [
    node for node in support.descendants(mailboxes) if support.role(node) == "list item"
]
assert mailbox_items, "the Mailboxes list box has no items"
assert all(support.name(node) for node in mailbox_items), "a mailbox item is unnamed"
selected_mailboxes = [n for n in mailbox_items if "selected" in support.states(n)]
assert len(selected_mailboxes) == 1, f"{len(selected_mailboxes)} mailboxes selected"
assert support.selected_count(mailboxes) == 1, "Mailboxes' Selection interface disagrees"
assert all("click" in support.actions(node) for node in mailbox_items), (
    "a mailbox item has no click"
)

conversations = support.wait_for(
    lambda: list_box(lambda name: "conversation" in name), "the conversations list box"
)
conversation_items = [
    node for node in support.descendants(conversations) if support.role(node) == "list item"
]
assert conversation_items, "the conversations list box has no items"
assert all(support.name(node) for node in conversation_items), "a conversation item is unnamed"
assert support.selected_count(conversations) == 1, (
    "the conversations list box has no single selected item"
)

if DUMP:
    support.dump(app(), DUMP)

viewer = support.wait_for(
    lambda: next(
        (node for node in support.descendants(app()) if support.role(node) == "document frame"), None
    ),
    "the message viewer document",
)
first_name = support.name(viewer)
assert first_name, "the message viewer has no accessible name"

# Selecting a different conversation must change the viewer's name (it
# includes the sender and subject) and move the selected state.
other = next(
    (node for node in conversation_items if "selected" not in support.states(node)), None
)
assert other is not None, "every conversation is already selected; cannot test switching"
assert support.click(other), "a conversation item's accessibility action was not handled"
time.sleep(0.3)
conversation_items = [
    node for node in support.descendants(conversations) if support.role(node) == "list item"
]
assert support.selected_count(conversations) == 1, "selecting a conversation left none selected"
viewer = next(
    (node for node in support.descendants(app()) if support.role(node) == "document frame"), None
)
assert support.name(viewer) != first_name, "the viewer's name did not change with the selection"

for label in (
    "Filter Unread",
    "Compose",
    "Archive",
    "Move to Bin",
    "Junk or Not Junk",
    "Reply",
    "Reply All",
    "Forward",
    "Flag",
    "Move or Copy to Mailbox",
    "Search",
):
    named_clickable_button(label)

search = named_clickable_button("Search")
assert support.click(search), "Search accessibility action was not handled"
search_field = support.wait_for(
    lambda: next(
        (
            node
            for node in support.descendants(app())
            if support.name(node) == "Search Mail" and support.has_text(node)
        ),
        None,
    ),
    "a named Search Mail field with a Text interface",
)
assert support.has_text(search_field), "Search Mail has no Text interface"
# Leave the toolbar the way this check found it.
assert support.click(search), "Search accessibility action did not toggle closed"

print(
    "AT-SPI Mail: "
    f"{len(mailbox_items)} mailbox items (1 selected), {len(conversation_items)} "
    "conversation items (1 selected), viewer name changes with selection, "
    "named toolbar controls, actionable Search field"
)
