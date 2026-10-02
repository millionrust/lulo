"""Declarative interaction-probe surfaces (docs/interaction-gaps.md's matrix).

A surface is one piece of transient UI chrome whose *reaction* to input -
not its item list (scripts/inventory) and not a fixed app script
(scripts/behavior) - is compared between the real Mac and Lulo: does it
close when you click elsewhere, does Escape dismiss it, does a control
visibly react to hover. Ground truth for each Mac surface below (process
names, AX titles/descriptions, positions) was read live from the reference
Mac's accessibility tree (read-only, then one open/close per surface, all
under /tmp/mac-gui.lock) on 2026-10-02; see mac_probe.py.

`status`:
  - "automated": both mac_probe.py and lulo_probe.py can open it and run
    its probes end to end.
  - "planned": declared for the matrix (coverage the owner asked for) but
    no driver yet. These show up in docs/interaction-gaps.md as "not yet
    probed", never as a false pass or a fabricated gap.
"""

from __future__ import annotations

from typing import Any

import probes as pr

SURFACES: list[dict[str, Any]] = [
    # -- Shell chrome: nested niri + the shipped shell.kdl (shell_nested.py) --
    {
        "id": "control-centre",
        "title": "Control Centre / Quick Settings",
        "kind": "popover",
        "status": "automated",
        "mac": {"process": "ControlCenter", "open": "menu-extra", "extra_description": "Control Centre"},
        "lulo": {"harness": "shell", "open": "shortcut", "shortcut": "quick-settings"},
        # Hover targets: (probe label, Mac AXDescription on the AXSlider, Lulo AT-SPI accessible name).
        "hover_controls": [
            {"label": "display-brightness-slider", "mac_description": "display brightness", "lulo_name": "Display"},
            {"label": "sound-volume-slider", "mac_description": "sound volume", "lulo_name": "Sound"},
        ],
    },
    {
        "id": "lulo-menu",
        "title": "Apple/Lulo menu (leftmost item in the top bar)",
        "kind": "menu",
        "status": "automated",
        "mac": {"process": "Finder", "open": "menu-bar-title", "title": "Apple", "neighbor_title": "Finder"},
        # Ground truth read live (lulo_probe.py --explore, 2026-10-02): the
        # leftmost top-bar item's accessible name is literally "menu", and
        # every other one is "<App> menu" (e.g. "Files menu"), not the bare
        # app name.
        "lulo": {"harness": "shell", "open": "topbar-menu", "label": "menu", "neighbor_label": "Files menu"},
    },
    {
        "id": "files-app-menu",
        "title": "Files' “File” app menu in the top bar",
        "kind": "menu",
        "status": "automated",
        "mac": {"process": "Finder", "open": "menu-bar-title", "title": "File", "neighbor_title": "Edit"},
        "lulo": {"harness": "shell", "open": "topbar-menu", "label": "File menu", "neighbor_label": "Edit menu"},
    },
    {
        "id": "clock-notification-centre",
        "title": "Clock menu extra / Notification Centre",
        "kind": "popover",
        "status": "planned",
        "mac": {"process": "ControlCenter", "open": "menu-extra", "extra_description": "Clock"},
        "lulo": {"harness": "shell", "open": "topbar-click", "label": "Clock"},
        "note": "Lulo interaction driver not wired up yet; the separate nested AT-SPI assertion covers its accessible tree.",
    },
    {
        "id": "status-menu-sound",
        "title": "Sound status menu",
        "kind": "menu",
        "status": "planned",
        "mac": {"process": "ControlCenter", "open": "menu-extra", "extra_description": "Sound"},
        "lulo": {"harness": "shell", "open": "topbar-menu", "label": "Sound"},
        "note": "Real Mac has no separate Sound menu-bar extra by default (it lives in Control Centre); needs"
                " the owner's menu-bar extras enabled before recording, so left planned.",
    },
    {
        "id": "spotlight",
        "title": "Spotlight / Lulo launcher",
        "kind": "popover",
        "status": "planned",
        "mac": {"process": "Spotlight", "open": "menu-extra", "extra_description": "Spotlight"},
        "lulo": {"harness": "shell", "open": "shortcut", "shortcut": "spotlight"},
        "note": "Driver not wired up yet; run_cold_surfaces.py already covers Spotlight's latency and typing,"
                " this suite still needs to add outside-click/Escape here.",
    },
    {
        "id": "dock",
        "title": "Dock + Dock context menu",
        "kind": "dock",
        "status": "planned",
        "mac": {"process": "Dock", "open": "already-visible"},
        "lulo": {"harness": "shell", "open": "already-visible"},
        "note": "Driver not wired up yet.",
    },
    # -- App dialogs/menus: one app in headless Sway, no shell (lulo_probe.py's Nested) --
    {
        "id": "files-window-context-menu",
        "title": "Files window background context menu",
        "kind": "context-menu",
        "status": "planned",
        "mac": {"app": "files", "open": "context-background"},
        "lulo": {"harness": "app", "app": "files", "open": "context-background"},
        "note": "Driver not wired up yet; scripts/behavior's existing context()/context_background() already"
                " prove the item list on both sides, this suite still needs an outside-click/Escape driver here.",
    },
    {
        "id": "text-editor-save-sheet",
        "title": "Text Editor's unsaved-document alert",
        "kind": "dialog",
        "status": "planned",
        "mac": {"app": "text-editor", "open": "close-unsaved"},
        "lulo": {"harness": "app", "app": "text-editor", "open": "close-unsaved"},
        "note": "Driver not wired up yet; tests/behavior/text-editor/close-unsaved-save.json already proves the"
                " dialog's content, this suite still needs to add Escape/outside-click here.",
    },
    {
        "id": "settings-sidebar-list",
        "title": "Settings sidebar list",
        "kind": "list",
        "status": "planned",
        "mac": {"app": "settings", "open": "already-visible"},
        "lulo": {"harness": "app", "app": "settings", "open": "already-visible"},
        "note": "Driver not wired up yet.",
    },
]


def surface(sid: str) -> dict[str, Any]:
    for item in SURFACES:
        if item["id"] == sid:
            return item
    raise KeyError(sid)


def matrix() -> list[tuple[str, str]]:
    """Every (surface id, probe id) pair the matrix declares, regardless of
    whether either side has a driver yet - the full coverage promise."""

    pairs = []
    for item in SURFACES:
        for probe_id in pr.probes_for(item["kind"]):
            pairs.append((item["id"], probe_id))
        for control in item.get("hover_controls", []):
            pairs.append((item["id"], f"hover:{control['label']}"))
    return pairs
