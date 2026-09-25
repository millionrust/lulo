"""Shared helpers for the scripts/assert_*_accessibility.py checks.

These run on a Linux session with the AT-SPI bus enabled, against one app
instance started with temporary XDG_* directories. They only read the tree
and use AT-SPI's own actions (Action.doAction, Component.grabFocus,
Text.setCaretOffset); they never inject keyboard or pointer input.
"""

from __future__ import annotations

import json
import os
import subprocess
import time

import pyatspi

TIMEOUT_SECONDS = float(os.environ.get("RMAC_A11Y_TIMEOUT", "20"))


def descendants(node, depth=0, limit=40):
    yield node
    if depth >= limit:
        return
    try:
        count = node.childCount
    except (LookupError, RuntimeError):
        return
    for index in range(count):
        try:
            child = node.getChildAtIndex(index)
        except (LookupError, RuntimeError):
            continue
        if child is not None:
            yield from descendants(child, depth + 1, limit)


def find_app(name):
    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        try:
            app = desktop.getChildAtIndex(index)
            if app is not None and app.name == name:
                return app
        except (LookupError, RuntimeError):
            continue
    return None


def wait_for(predicate, description, timeout=TIMEOUT_SECONDS):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            value = predicate()
        except (LookupError, RuntimeError) as error:
            value = None
            last_error = error
        if value:
            return value
        time.sleep(0.2)
    suffix = f" (last error: {last_error})" if last_error else ""
    raise AssertionError(f"timed out waiting for {description}{suffix}")


def role(node):
    try:
        return node.getRoleName()
    except (LookupError, RuntimeError):
        return ""


def name(node):
    try:
        return node.name or ""
    except (LookupError, RuntimeError):
        return ""


def description(node):
    try:
        return node.description or ""
    except (LookupError, RuntimeError):
        return ""


def states(node):
    try:
        state_set = node.getState()
    except (LookupError, RuntimeError):
        return set()
    return {
        pyatspi.stateToString(index)
        for index in range(pyatspi.STATE_LAST_DEFINED)
        if state_set.contains(index)
    }


def nodes_with(root, wanted_role, wanted_name=None):
    return [
        node
        for node in descendants(root)
        if role(node) == wanted_role and (wanted_name is None or name(node) == wanted_name)
    ]


def text_of(node):
    """(text, caret, selections) from the node's AT-SPI Text interface."""
    text = node.queryText()
    count = text.characterCount
    selections = [text.getSelection(index) for index in range(text.getNSelections())]
    return text.getText(0, count), text.caretOffset, selections


def has_text(node):
    try:
        node.queryText()
        return True
    except NotImplementedError:
        return False


def line_at(node, offset):
    # AccessKit implements GetStringAtOffset, not the older GetTextAtOffset.
    text = node.queryText()
    line, start, end = text.getStringAtOffset(offset, pyatspi.TEXT_GRANULARITY_LINE)
    return line, start, end


def actions(node):
    try:
        action = node.queryAction()
    except NotImplementedError:
        return []
    return [action.getName(index) for index in range(action.nActions)]


def click(node):
    action = node.queryAction()
    for index in range(action.nActions):
        if action.getName(index) == "click":
            return action.doAction(index)
    raise AssertionError(f"{role(node)} {name(node)!r} has no click action")


def selected_count(node):
    try:
        return node.querySelection().nSelectedChildren
    except NotImplementedError:
        return None


def activate_menu(bus_name, action):
    """Run an app menu command through the app's own menu endpoint, the way
    the top bar does, instead of a keyboard shortcut."""
    subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            bus_name,
            "--object-path",
            "/org/rmac/AppMenu1",
            "--method",
            "org.rmac.AppMenu1.Activate",
            action,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        timeout=5,
    )


def dump(root, path):
    """Write the app's tree (roles, names, descriptions, states, actions,
    text and caret) as JSON lines for the evidence log."""
    with open(path, "w", encoding="utf-8") as output:
        stack = [(root, 0)]
        while stack:
            node, depth = stack.pop()
            entry = {
                "depth": depth,
                "role": role(node),
                "name": name(node),
                "description": description(node),
                "states": sorted(states(node) & {"focused", "selected", "focusable", "selectable", "editable", "expanded"}),
                "actions": actions(node),
            }
            if has_text(node):
                try:
                    text, caret, selections = text_of(node)
                    entry["text"] = text[:400]
                    entry["characters"] = len(text)
                    entry["caret"] = caret
                    entry["selections"] = selections
                except (LookupError, RuntimeError):
                    pass
            output.write(json.dumps(entry, ensure_ascii=False) + "\n")
            try:
                children = [node.getChildAtIndex(i) for i in range(node.childCount)]
            except (LookupError, RuntimeError):
                children = []
            for child in reversed(children):
                if child is not None and depth < 40:
                    stack.append((child, depth + 1))
