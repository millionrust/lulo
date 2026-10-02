"""The interaction-probe matrix: declarative probes and the pure rules used
to compare a Mac recording against a Lulo recording (scripts/interaction/diff.py).

This is a sibling to scripts/inventory (which diffs *item lists*: menu
entries, shortcuts) and scripts/behavior (which diffs one app's reaction to
a fixed script of keys and clicks). This suite instead answers a narrower,
previously invisible question for shell chrome and transient UI: when you
click elsewhere, press Escape, or just rest the pointer on a control, does
Lulo *behave* the way the real control does? The owner found two such gaps
by hand (top-bar menus/Control Centre not closing on an outside click;
Control Centre's sliders not growing on hover) that no menu-item inventory
could ever see, because both are reactions, not items.

Every probe result is a small dict of booleans/numbers, exactly like
scripts/behavior/scenario.py's facts: never a screenshot, never an absolute
path. `status` says whether a probe has a real driver on both platforms yet
("automated") or is declared for the matrix but not wired up ("planned");
diff.py only compares "automated" probes.
"""

from __future__ import annotations

from typing import Any, Optional

# --------------------------------------------------------------------------
# Probe registry
# --------------------------------------------------------------------------

PROBES: dict[str, dict[str, Any]] = {
    "outside_click": {
        "title": "Clicking elsewhere closes it",
        "applies_to": {"menu", "popover", "context-menu"},
        "facts": ["closed"],
        "status": "automated",
    },
    "escape": {
        "title": "Escape dismisses it",
        "applies_to": {"menu", "popover", "context-menu", "dialog"},
        "facts": ["closed"],
        "status": "automated",
    },
    "reopen_same_title": {
        "title": "Clicking the same menu title again closes it",
        "applies_to": {"menu"},
        "facts": ["closed"],
        "status": "automated",
    },
    "switch_neighbor": {
        "title": "Clicking a neighbouring menu title switches to it",
        "applies_to": {"menu"},
        "facts": ["switched"],
        "status": "automated",
    },
    "hover": {
        "title": "Hovering a control visibly reacts",
        "applies_to": {"popover", "menu", "dock", "list"},
        "facts": ["changed"],
        "status": "automated",
    },
    "right_click": {
        "title": "Right-click opens a context menu",
        "applies_to": {"list", "dock"},
        "facts": ["opened"],
        "status": "automated",
    },
    "double_click": {
        "title": "Double-click activates (opens/zooms)",
        "applies_to": {"list", "window-title-bar"},
        "facts": ["activated"],
        "status": "planned",
    },
    "tab_focus": {
        "title": "Tab/Shift-Tab moves focus, with a visible focus ring",
        "applies_to": {"dialog", "list"},
        "facts": ["moved"],
        "status": "automated",
    },
    "arrow_keys": {
        "title": "Arrow keys move the selection",
        "applies_to": {"menu", "list"},
        "facts": ["moved"],
        "status": "planned",
    },
    "type_to_select": {
        "title": "Typing jumps to a matching row",
        "applies_to": {"list"},
        "facts": ["jumped"],
        "status": "planned",
    },
    "scroll": {
        "title": "Scrolling moves the content (and rubber-bands at the end)",
        "applies_to": {"list"},
        "facts": ["scrolled"],
        "status": "automated",
    },
    "press_hold": {
        "title": "Press-and-hold has a distinct reaction from a click",
        "applies_to": {"dock", "popover"},
        "facts": ["reacted"],
        "status": "planned",
    },
}

AUTOMATED = {pid for pid, spec in PROBES.items() if spec["status"] == "automated"}

# Fields that are booleans; "exact" tolerance just means "both sides agree,
# once neither is None (not measured)".
FACT_RULE = {
    "closed": "exact",
    "switched": "exact",
    "changed": "exact",
    "opened": "exact",
    "activated": "exact",
    "moved": "exact",
    "jumped": "exact",
    "scrolled": "exact",
    "reacted": "exact",
}


def probes_for(kind: str) -> list[str]:
    """Every probe id declared against surface `kind`, automated first."""

    ids = [pid for pid, spec in PROBES.items() if kind in spec["applies_to"]]
    return sorted(ids, key=lambda pid: (PROBES[pid]["status"] != "automated", pid))


def field_matches(rule: str, expected: Any, actual: Any) -> bool:
    if rule == "ignore":
        return True
    if rule == "exact":
        return expected == actual
    raise ValueError(f"unknown interaction-probe tolerance rule {rule!r}")


def compare_probe_result(probe_id: str, mac: Optional[dict[str, Any]], lulo: Optional[dict[str, Any]]) -> dict[str, Any]:
    """One probe's comparison: per-fact mismatches, plus which facts were not
    measured on one or both sides ("inconclusive" rather than a gap - a
    missing measurement must never silently read as a pass or a mismatch)."""

    spec = PROBES[probe_id]
    mismatches: list[dict[str, Any]] = []
    inconclusive: list[str] = []
    for fact in spec["facts"]:
        mac_value = (mac or {}).get(fact)
        lulo_value = (lulo or {}).get(fact)
        if mac is None or lulo is None or mac_value is None or lulo_value is None:
            inconclusive.append(fact)
            continue
        rule = FACT_RULE.get(fact, "exact")
        if not field_matches(rule, mac_value, lulo_value):
            mismatches.append({"fact": fact, "mac": mac_value, "lulo": lulo_value, "rule": rule})
    return {"probe": probe_id, "mismatches": mismatches, "inconclusive": inconclusive}


def describe(value: Any) -> str:
    if value is None:
        return "not measured"
    if isinstance(value, bool):
        return "yes" if value else "no"
    return str(value)
