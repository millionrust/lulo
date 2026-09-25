#!/usr/bin/env python3
"""Assert Files' rows, sidebar and rename field over AT-SPI (ACC-03).

Run against one rmac-files started with temporary XDG_* directories on a
seeded folder, RMAC_FILES_FOLDER, holding `alpha.txt`, `beta folder/` and
`gamma.pdf` (see scripts/linux/run-content-accessibility.sh). Checks:

- the folder listing is a `list box` of `list item`s named after each item,
  described by its kind, with position, a click action and selection;
- the sidebar is a `list box` of named, clickable places;
- AT-SPI's click on `alpha.txt` selects exactly it;
- File > Rename (through Files' own menu endpoint, as the top bar calls it)
  opens a focused `Name` entry whose Text is the name, reporting the
  field's selection (the name up to its extension, since b8428ab5);
- Text.setCaretOffset moves the rename field's caret, and focusing the row
  again ends the rename without changing the file.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import atspi_assert_support as support  # noqa: E402

APP = os.environ.get("RMAC_FILES_APP", "rmac-files")
FOLDER = os.environ["RMAC_FILES_FOLDER"]
DUMP = os.environ.get("RMAC_A11Y_DUMP")
EXPECTED = {
    "alpha.txt": "Plain Text Document",
    "beta folder": "Folder",
    "gamma.pdf": "PDF document",
}


def app():
    return support.find_app(APP)


def listing():
    root = app()
    if root is None:
        return None
    label = os.path.basename(FOLDER.rstrip("/"))
    for box in support.nodes_with(root, "list box", label):
        items = [n for n in support.descendants(box) if support.role(n) == "list item"]
        if {support.name(n) for n in items} >= set(EXPECTED):
            return box, items
    return None


box, items = support.wait_for(listing, "the seeded folder's list box and items")
if DUMP:
    support.dump(app(), DUMP)

by_name = {support.name(item): item for item in items}
for item_name, kind in EXPECTED.items():
    item = by_name[item_name]
    assert support.description(item) == kind, (
        f"{item_name!r} described as {support.description(item)!r}, expected {kind!r}"
    )
    assert "click" in support.actions(item), f"{item_name!r} has no click action"
    assert "selectable" in support.states(item), f"{item_name!r} is not selectable"

sidebar = support.nodes_with(app(), "list box", "Sidebar")
assert sidebar, "no Sidebar list box"
places = [n for n in support.descendants(sidebar[0]) if support.role(n) == "list item"]
assert places and all(support.name(p) for p in places), "sidebar places missing or unnamed"
assert all("click" in support.actions(p) for p in places), "a sidebar place has no click"

support.click(by_name["alpha.txt"])


def alpha_selected():
    found = listing()
    if not found:
        return False
    current_box, current = found
    selected = [support.name(n) for n in current if "selected" in support.states(n)]
    return selected == ["alpha.txt"] and support.selected_count(current_box) == 1


support.wait_for(alpha_selected, "alpha.txt to be the only selected item")

support.activate_menu("org.rmac.Files.Menu", "finder::RenameItem")


def rename_field():
    for node in support.nodes_with(app(), "entry", "Name"):
        return node
    return None


field = support.wait_for(rename_field, "the rename field's Name entry")
if DUMP:
    support.dump(app(), DUMP + ".rename")
support.wait_for(lambda: "focused" in support.states(rename_field()), "the Name entry to be focused")
text, caret, selections = support.text_of(rename_field())
assert text == "alpha.txt", f"the Name entry reads {text!r}"
# Like Finder, Rename selects a file's name up to its extension (b8428ab5);
# builds before that open with nothing selected. Either way AT-SPI must
# report the field's real selection and caret.
assert selections in ([], [(0, len("alpha"))]), f"rename selection is {selections}"
assert caret == (selections[0][1] if selections else 0), f"rename caret is {caret}"
rename_selection = selections
rename_field().queryText().setCaretOffset(3)


def caret_at_three():
    for node in support.nodes_with(app(), "entry", "Name"):
        _, caret, selections = support.text_of(node)
        return caret == 3 and not selections
    return False


support.wait_for(caret_at_three, "setCaretOffset(3) to move the rename caret")

# Focus the row again: the rename field loses focus and closes unchanged.
found = listing()
assert found, "the listing went away during rename"
row = {support.name(n): n for n in found[1]}["alpha.txt"]
row.queryComponent().grabFocus()
support.wait_for(lambda: not support.nodes_with(app(), "entry", "Name"), "the rename to end")
assert os.path.exists(os.path.join(FOLDER, "alpha.txt")), "alpha.txt was renamed"

print(
    "AT-SPI Files: "
    f"{len(items)} items with kind descriptions and click/selection, {len(places)} sidebar places, "
    f"click selects, Rename opens a focused Name entry (selection {rename_selection}), "
    "setCaretOffset moves the caret, refocusing the row ends the rename"
)
