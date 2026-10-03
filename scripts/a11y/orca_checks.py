"""Pure checks for the automated Orca audit (scripts/a11y/orca_audit.py).

Every function here takes plain dictionaries — an AT-SPI snapshot of the
focused object and the utterances Orca produced — so the rules are unit
tested on any machine (scripts/test_orca_audit.py) without a screen reader.

A snapshot is::

    {"key": "<bus>:<path>", "app": "rmac-files", "role": "push button",
     "name": "Back", "description": "", "states": ["focusable", ...],
     "value": None, "text": None, "labelled_by": "", "content": ""}

`content` is the visible text under an item (what Orca falls back to when an
item has no name of its own).
"""

from __future__ import annotations

import re
from typing import Any, Optional

# Roles a keyboard user can land on and expect to hear something specific.
INTERACTIVE_ROLES = {
    "button", "push button", "push button menu", "toggle button", "check box", "radio button", "combo box",
    "slider", "spin button", "page tab", "link", "menu item", "check menu item",
    "radio menu item", "list item", "tree item", "table row", "table cell",
    "switch", "entry", "text", "password text", "terminal", "icon",
}
# Containers that should never be the focus target themselves: Orca reads
# them as "panel"/"unknown" with nothing useful to act on.
GENERIC_ROLES = {
    "panel", "filler", "unknown", "section", "invalid", "redundant object",
    "layered pane", "scroll pane", "grouping", "group", "generic",
}
TEXT_ROLES = {"entry", "text", "password text", "terminal", "editbar", "search field"}
# A toggle button reports "pressed", not checkable/checked (AT-SPI convention).
CHECKABLE_ROLES = {"check box", "switch", "check menu item", "radio button", "radio menu item"}
EXPANDABLE_ROLES = {"combo box"}
SELECTABLE_ROLES = {"list item", "tree item", "page tab", "table row"}
# Roles whose name may legitimately come from their visible content.
CONTENT_NAMED_ROLES = {"list item", "tree item", "table row", "table cell", "menu item", "page tab"}

SEVERITY = {
    "focus-lost": "high",
    "unnamed": "high",
    "generic-role": "medium",
    "missing-state": "medium",
    "silent-focus": "medium",
    "silent-change": "medium",
    "no-effect": "low",
    "speech-mismatch": "low",
    "tab-stuck": "high",
    "tab-trap": "high",
    "reverse-order": "medium",
    "unreachable": "medium",
    "pointer-only": "high",
    "escape-focus": "medium",
    "focus-nowhere": "medium",
    "tab-per-item": "medium",
}


def flag(kind: str, detail: str, **extra: Any) -> dict[str, Any]:
    return {"kind": kind, "severity": SEVERITY.get(kind, "low"), "detail": detail, **extra}


def describe(snapshot: Optional[dict[str, Any]]) -> str:
    """What a screen reader would say, roughly: name, role, states, value."""

    if not snapshot:
        return "(nothing focused)"
    parts = [snapshot.get("name") or snapshot.get("labelled_by") or snapshot.get("content") or "(unnamed)",
             snapshot.get("role") or "?"]
    states = set(snapshot.get("states") or [])
    if "checkable" in states or snapshot.get("role") in CHECKABLE_ROLES:
        parts.append("checked" if "checked" in states else "not checked")
    if "expandable" in states:
        parts.append("expanded" if "expanded" in states else "collapsed")
    if "selected" in states:
        parts.append("selected")
    if "sensitive" not in states and "enabled" not in states and states:
        parts.append("dimmed")
    if snapshot.get("value") is not None:
        parts.append(f"value {snapshot['value']:g}")
    elif snapshot.get("text"):
        parts.append(repr(snapshot["text"][:40]))
    return ", ".join(parts)


def accessible_label(snapshot: dict[str, Any]) -> str:
    label = (snapshot.get("name") or "").strip() or (snapshot.get("labelled_by") or "").strip()
    if not label and snapshot.get("role") in CONTENT_NAMED_ROLES:
        label = (snapshot.get("content") or "").strip()
    return label


def focus_flags(snapshot: Optional[dict[str, Any]]) -> list[dict[str, Any]]:
    """Problems with one focused object, regardless of how focus got there."""

    if not snapshot:
        return [flag("focus-lost", "keyboard focus is on no accessible object")]
    out = []
    role = snapshot.get("role") or ""
    states = set(snapshot.get("states") or [])
    if role in {"frame", "window"}:
        out.append(flag("focus-nowhere", "keyboard focus is on the window itself, not on any control"))
    if role in GENERIC_ROLES and not accessible_label(snapshot):
        out.append(flag("generic-role", f"focus landed on an unnamed {role!r}"))
    elif role in GENERIC_ROLES:
        out.append(flag("generic-role", f"focus landed on {role!r} {snapshot.get('name')!r}, not a control"))
    if (role in INTERACTIVE_ROLES or role in TEXT_ROLES) and not accessible_label(snapshot):
        if role in TEXT_ROLES and (snapshot.get("description") or "").strip():
            pass  # a described field is announced; naming it is still better but not silent
        else:
            out.append(flag("unnamed", f"{role} has no accessible name"))
    if role in CHECKABLE_ROLES and "checkable" not in states and "checked" not in states:
        out.append(flag("missing-state", f"{role} reports no checkable/checked state"))
    if role in EXPANDABLE_ROLES and "expandable" not in states:
        out.append(flag("missing-state", f"{role} reports no expandable state"))
    if role in SELECTABLE_ROLES and "selectable" not in states:
        out.append(flag("missing-state", f"{role} reports no selectable state"))
    return out


def normalise(text: str) -> str:
    return re.sub(r"[^0-9a-z]+", " ", text.lower()).strip()


def spoken(speech: list[str]) -> str:
    return normalise(" ".join(speech))


def step_flags(before: Optional[dict[str, Any]], after: Optional[dict[str, Any]], speech: list[str],
               expect: Optional[str] = None) -> list[dict[str, Any]]:
    """Problems with one key press: what changed, and whether Orca said so.

    expect: "move" (focus should move), "toggle" (checked should flip),
    "value" (value/text should change), "expand" (a popup should open),
    or None (just observe).
    """

    out = []
    moved = (before or {}).get("key") != (after or {}).get("key")
    said = spoken(speech)
    if after and moved and not said:
        out.append(flag("silent-focus", f"focus moved to {describe(after)} but Orca said nothing"))
    if after and moved and said:
        label = normalise(accessible_label(after))
        if label and label not in said and not any(word in said for word in label.split() if len(word) > 3):
            out.append(flag("speech-mismatch", f"Orca said {' / '.join(speech)[:120]!r}, not the name {label!r}"))
    if not after or not before or moved:
        if expect == "move" and not moved:
            out.append(flag("no-effect", "the key did not move focus"))
        if expect in {"toggle", "value"} and moved:
            out.append(flag("no-effect", f"expected a {expect} change in place; focus moved instead"))
        return out
    before_states = set(before.get("states") or [])
    after_states = set(after.get("states") or [])
    changed_states = before_states ^ after_states
    value_changed = before.get("value") != after.get("value") or before.get("text") != after.get("text")
    if expect == "toggle":
        if not ({"checked", "pressed", "selected"} & changed_states):
            out.append(flag("no-effect", f"Space did not change {describe(after)}'s checked state"))
        elif not said:
            out.append(flag("silent-change", f"{describe(after)} changed state but Orca said nothing"))
    elif expect == "value":
        if not value_changed and not ({"selected"} & changed_states):
            out.append(flag("no-effect", f"the key did not change {describe(after)}'s value"))
        elif not said:
            out.append(flag("silent-change", f"{describe(after)} changed value but Orca said nothing"))
    elif expect == "move" and not said:
        # A list that keeps focus and moves its selection (an active
        # descendant) is fine as long as Orca reads the new item.
        out.append(flag("no-effect", "the key neither moved focus nor made Orca say anything"))
    elif expect is None and (value_changed or {"checked", "expanded", "selected"} & changed_states) and not said:
        out.append(flag("silent-change", f"{describe(after)} changed but Orca said nothing"))
    return out


def cycle_flags(stops: list[dict[str, Any]], outcome: str,
                start: Optional[dict[str, Any]] = None) -> list[dict[str, Any]]:
    """Problems with a Tab cycle. outcome is "cycle" (returned to the first
    stop), "stuck" (Tab stopped moving), "subcycle" (looped among later stops
    without returning), or "limit" (gave up)."""

    out = []
    if outcome == "stuck":
        last = stops[-1] if stops else None
        if not last or last.get("role") not in TEXT_ROLES or "multi line" not in (last.get("states") or []):
            out.append(flag("tab-stuck", f"Tab stopped moving at {describe(last)}"))
    elif outcome == "subcycle":
        out.append(flag("tab-trap", "Tab loops among " + ", ".join(describe(s) for s in stops[:6])
                        + f" and never returns to {describe(start)}"))
    elif outcome == "single":
        out.append(flag("tab-trap", f"Tab never leaves {describe(stops[0] if stops else None)}"))
    return out


ITEM_ROLES = {"list item", "tree item", "table row", "table cell"}


def item_stop_flags(stops: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Three or more items of one list in a row in the Tab order: each item
    is its own Tab stop, where a list should be one stop with arrow keys
    inside it."""

    out = []
    run: list[dict[str, Any]] = []
    for stop in stops + [{}]:
        if run and stop.get("role") in ITEM_ROLES and stop.get("parent") == run[0].get("parent"):
            run.append(stop)
            continue
        if len(run) >= 3:
            names = [accessible_label(item) or "(unnamed)" for item in run]
            out.append(flag("tab-per-item", f"{len(run)} items of one list are separate Tab stops "
                            f"({', '.join(names[:5])}{' ...' if len(names) > 5 else ''})"))
        run = [stop] if stop.get("role") in ITEM_ROLES else []
    return out


def reverse_flags(stops: list[dict[str, Any]], reverse: list[Optional[dict[str, Any]]]) -> list[dict[str, Any]]:
    """After a full forward cycle back to stops[0], Shift-Tab must visit
    stops[-1], stops[-2], ... in that order."""

    out = []
    expected = list(reversed(stops))[: len(reverse)]
    for index, (want, got) in enumerate(zip(expected, reverse)):
        if (got or {}).get("key") != want.get("key"):
            out.append(flag("reverse-order",
                            f"Shift-Tab #{index + 1} went to {describe(got)}, expected {describe(want)}"))
            break
    return out


ARROW_CONTAINERS = {"list", "list box", "tree", "table", "tree table", "menu", "menu bar", "radio group",
                    "page tab list", "tool bar"}
REACHABLE_ROLES = {"button", "push button", "push button menu", "toggle button", "check box", "radio button", "combo box", "slider",
                   "spin button", "link", "switch", "entry", "text", "password text", "page tab"}


def reachability_flags(controls: list[dict[str, Any]], visited: set[str]) -> list[dict[str, Any]]:
    """controls: every showing, enabled control in the window, each with an
    `ancestors` list of roles. Those Tab never reached are reported: as
    pointer-only when they are not even focusable, otherwise as outside
    the Tab order. Items inside lists, menus, tab lists, toolbars and radio
    groups are reached with the arrow keys, so they are skipped."""

    pointer: list[str] = []
    skipped: list[str] = []
    for control in controls:
        if control.get("key") in visited or control.get("role") not in REACHABLE_ROLES:
            continue
        if set(control.get("ancestors") or []) & ARROW_CONTAINERS:
            continue
        states = set(control.get("states") or [])
        if "sensitive" not in states and "enabled" not in states:
            continue
        label = f"{control['role']} {accessible_label(control) or '(unnamed)'!r}"
        (skipped if "focusable" in states else pointer).append(label)
    out = []
    if pointer:
        out.append(flag("pointer-only", f"{len(pointer)} control(s) cannot take keyboard focus: "
                        + ", ".join(pointer[:12]) + (" ..." if len(pointer) > 12 else ""), controls=pointer))
    if skipped:
        out.append(flag("unreachable", f"{len(skipped)} focusable control(s) are not in the Tab order: "
                        + ", ".join(skipped[:12]) + (" ..." if len(skipped) > 12 else ""), controls=skipped))
    return out


def summarise(flags: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for item in flags:
        counts[item["kind"]] = counts.get(item["kind"], 0) + 1
    return dict(sorted(counts.items()))
