#!/usr/bin/env python3
"""Assert Notes' lists and text fields over AT-SPI (ACC-02).

Run against one rmac-notes started with temporary XDG_* directories, so its
library is a fresh, empty one (see scripts/linux/run-content-accessibility.sh).
The script creates one note through the accessible New Note toolbar action,
then checks:

- the folder sidebar and the note list are `list box`es of named `list
  item`s with exactly one selected, through the Selection interface;
- New Note and New Folder expose names and Click actions; Checklist, Add
  Photo… and Move Note… have names in the newly created note state; the
  in-note Find controls expose Click actions and can be closed;
- Search, Title, Body and Tags are named `entry` nodes with a Text interface,
  and no unnamed entry is left over from the field's inner input;
- grabbing focus on Title focuses it (AccessKit's Focus action) and its
  caret is reported. A new note's fields are empty, so moving the caret is
  checked on Files' rename field instead.

Typing through AT-SPI (EditableText) is not checked: accesskit_unix 0.22.1
does not implement it. A screen-reader user types with the keyboard.
"""

from __future__ import annotations

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_NOTES_APP", "rmac-notes")
DUMP = os.environ.get("RMAC_A11Y_DUMP")


def app():
    return support.find_app(APP)


def list_box(label):
    root = app()
    if root is None:
        return None
    boxes = support.nodes_with(root, "list box", label)
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


def named_button(label):
    return support.wait_for(
        lambda: next(
            (
                node
                for node in support.descendants(app())
                if support.role(node) in {"push button", "button"}
                and support.name(node) == label
            ),
            None,
        ),
        f"the named {label} button",
    )


folders = support.wait_for(lambda: list_box("Folders"), "the Folders list box")
folder_items = [
    node for node in support.descendants(folders) if support.role(node) == "list item"
]
assert folder_items, "the Folders list box has no items"
assert all(support.name(node) for node in folder_items), "a folder item is unnamed"
selected_folders = [n for n in folder_items if "selected" in support.states(n)]
assert len(selected_folders) == 1, f"{len(selected_folders)} folders selected"
assert support.selected_count(folders) == 1, "Folders' Selection interface disagrees"
assert all("click" in support.actions(node) for node in folder_items), "a folder item has no click"

new_note = named_clickable_button("New Note")
assert support.click(new_note), "New Note accessibility action was not handled"


def editor_ready():
    root = app()
    names = {support.name(node) for node in support.nodes_with(root, "entry")}
    return {"Title", "Body", "Tags", "Search"} <= names


support.wait_for(editor_ready, "the Title, Body, Tags and Search fields of a new note")
# Let the new note's selection settle in the list.
time.sleep(1.0)
root = app()
if DUMP:
    support.dump(root, DUMP)

entries = support.nodes_with(root, "entry")
unnamed = [node for node in entries if not support.name(node)]
assert not unnamed, f"{len(unnamed)} unnamed entry nodes remain"
fields = {support.name(node): node for node in entries}
for label in ("Search", "Title", "Body", "Tags"):
    assert support.has_text(fields[label]), f"{label} has no Text interface"

notes = list_box("Notes")
assert notes is not None, "no Notes list box"
note_items = [node for node in support.descendants(notes) if support.role(node) == "list item"]
assert len(note_items) >= 1, "the new note is not in the Notes list"
assert support.selected_count(notes) == 1, "the new note is not the selected list item"



def fields_now(label):
    for node in support.nodes_with(app(), "entry", label):
        return node
    return None


fields["Title"].queryComponent().grabFocus()
support.wait_for(lambda: "focused" in support.states(fields_now("Title")), "Title to take focus")
title = fields_now("Title")
text, caret, _ = support.text_of(title)
assert 0 <= caret <= len(text), f"Title caret {caret} outside {len(text)} characters"
body = fields_now("Body")
body_text, body_caret, _ = support.text_of(body)
assert 0 <= body_caret <= len(body_text)

for label in ("Checklist", "Add Photo…", "Move Note…"):
    named_button(label)
named_clickable_button("New Folder")

support.activate_menu("org.rmac.Notes.Menu", "notes::FindInNote")
previous = named_clickable_button("Previous match")
following = named_clickable_button("Next match")
done = named_clickable_button("Done")
assert support.click(previous), "Previous match accessibility action was not handled"
assert support.click(following), "Next match accessibility action was not handled"
assert support.click(done), "Done accessibility action was not handled"
support.wait_for(
    lambda: not support.nodes_with(app(), "button", "Done"),
    "the in-note Find bar to close",
)

print(
    "AT-SPI Notes: "
    f"{len(folder_items)} folder items (1 selected), {len(note_items)} note items (1 selected), "
    "named toolbar controls, actionable New Note/New Folder/Find, "
    "Search/Title/Body/Tags entries with Text, "
    f"Title focus via grabFocus, caret {caret}"
)
