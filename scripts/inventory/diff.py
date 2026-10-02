#!/usr/bin/env python3
"""Diff the Mac and Lulo feature inventories and write docs/inventory-gaps.md.

Reads tests/inventory/mac/<MacApp>.json and tests/inventory/lulo/<LuloApp>.json
(written by mac_inventory.py and lulo_inventory.py), normalises names with
normalize.py, drops anything in tests/inventory/allowlist.json, and sorts
the rest by likely user impact: menu items with a keyboard shortcut and
context-menu items first, then shortcut mismatches, then everything else.
Each gap gets a stable id (e.g. `FIL-MENU-003`) so an agent fixing it can
reference and later mark the row `Fixed <sha>`.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import normalize as norm  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
MAC_DIR = REPO_ROOT / "tests/inventory/mac"
LULO_DIR = REPO_ROOT / "tests/inventory/lulo"
ALLOWLIST_PATH = REPO_ROOT / "tests/inventory/allowlist.json"
OUT_PATH = REPO_ROOT / "docs/inventory-gaps.md"

# Lulo app display name -> a short, stable code used in gap ids.
APP_CODES = {
    "Finder": "FIL",
    "Text Editor": "TXT",
    "Preview": "PRV",
    "Notes": "NOT",
    "Terminal": "TRM",
    "Calculator": "CLC",
    "System Monitor": "MON",
    "System Settings": "SET",
    "Clock": "CLK",
    "Weather": "WTH",
}

# Impact tiers: lower sorts first (shown first in the report).
TIER_MENU_WITH_SHORTCUT = 0
TIER_CONTEXT_MENU = 1
TIER_MENU_NO_SHORTCUT = 2
TIER_SHORTCUT_MISMATCH = 3
TIER_SETTINGS = 4
TIER_SIDEBAR = 5
TIER_TOOLBAR = 6
TIER_EXTRA_LULO_ONLY = 7

TIER_NAMES = {
    TIER_MENU_WITH_SHORTCUT: "missing menu item (has a shortcut)",
    TIER_CONTEXT_MENU: "missing context-menu item",
    TIER_MENU_NO_SHORTCUT: "missing menu item",
    TIER_SHORTCUT_MISMATCH: "wrong/missing shortcut",
    TIER_SETTINGS: "missing settings control",
    TIER_SIDEBAR: "missing sidebar pane",
    TIER_TOOLBAR: "missing toolbar item",
    TIER_EXTRA_LULO_ONLY: "Lulo-only (not on the Mac)",
}


@dataclass
class Gap:
    app: str
    category: str  # "menu" | "context" | "settings" | "sidebar" | "toolbar"
    tier: int
    label: str
    path: str
    mac_shortcut: str = ""
    lulo_shortcut: str = ""
    note: str = ""
    gap_id: str = field(default="", compare=False)


# ---------------------------------------------------------------------------
# Allowlist
# ---------------------------------------------------------------------------


def load_allowlist(path: Path = ALLOWLIST_PATH) -> list[dict]:
    if not path.exists():
        return []
    data = json.loads(path.read_text())
    return data.get("entries", [])


def is_allowlisted(app: str, label: str, allowlist: list[dict]) -> str | None:
    """Return the reason this label is allowlisted for `app`, or None."""
    norm_label = norm.normalize_label(label) or ""
    for entry in allowlist:
        scope = entry.get("app", "*")
        if scope != "*" and scope != app:
            continue
        candidate = norm.normalize_label(entry.get("label", "")) or ""
        match = entry.get("match", "exact")
        if match == "exact" and norm_label == candidate:
            return entry.get("reason", "allowlisted")
        if match == "prefix" and norm_label.startswith(candidate):
            return entry.get("reason", "allowlisted")
    return None


# ---------------------------------------------------------------------------
# Menu bar
# ---------------------------------------------------------------------------


def flatten_menu_items(
    menus: list[dict], *, bold_menu_alias: str | None = None
) -> dict[tuple[str, ...], dict]:
    """Flatten a `menu_bar`-shaped list (as both sides emit it) into
    {path-of-normalised-labels: {label, shortcut}}. Separators are
    dropped. `bold_menu_alias`, when given, is a menu label (the Mac's
    bold app-name menu) to treat as "Application" so it lines up with
    Lulo's menu of that name."""
    out: dict[tuple[str, ...], dict] = {}

    def menu_label(label: str) -> str:
        if bold_menu_alias is not None and label == bold_menu_alias:
            return "Application"
        return label

    def walk(items: list[dict], path: tuple[str, ...]) -> None:
        for item in items:
            label = item.get("label")
            if label is None:
                continue  # separator
            norm_label = norm.normalize_label(label) or ""
            item_path = path + (norm_label,)
            out[item_path] = {
                "label": label,
                "shortcut": item.get("shortcut", ""),
            }
            children = item.get("children") or []
            if children:
                walk(children, item_path)

    for menu in menus:
        top_label = menu_label(menu.get("label", ""))
        norm_top = norm.normalize_label(top_label) or ""
        walk(menu.get("items", []), (norm_top,))
    return out


def diff_menu_bars(
    app: str, mac_menus: list[dict], lulo_menus: list[dict], mac_app_display_name: str
) -> list[Gap]:
    mac_flat = flatten_menu_items(mac_menus, bold_menu_alias=mac_app_display_name)
    lulo_flat = flatten_menu_items(lulo_menus)

    gaps: list[Gap] = []
    for path, mac_item in mac_flat.items():
        path_str = " ▸ ".join(path)
        if path not in lulo_flat:
            has_shortcut = bool(mac_item["shortcut"])
            tier = TIER_MENU_WITH_SHORTCUT if has_shortcut else TIER_MENU_NO_SHORTCUT
            gaps.append(
                Gap(
                    app=app,
                    category="menu",
                    tier=tier,
                    label=mac_item["label"],
                    path=path_str,
                    mac_shortcut=mac_item["shortcut"],
                    note="missing from Lulo's menu bar",
                )
            )
            continue
        lulo_item = lulo_flat[path]
        mac_sc = norm.normalize_shortcut(mac_item["shortcut"])
        lulo_sc = norm.normalize_shortcut(lulo_item["shortcut"])
        if mac_sc != lulo_sc:
            gaps.append(
                Gap(
                    app=app,
                    category="menu",
                    tier=TIER_SHORTCUT_MISMATCH,
                    label=mac_item["label"],
                    path=path_str,
                    mac_shortcut=mac_item["shortcut"],
                    lulo_shortcut=lulo_item["shortcut"],
                    note="shortcut differs",
                )
            )
    for path, lulo_item in lulo_flat.items():
        if path not in mac_flat:
            gaps.append(
                Gap(
                    app=app,
                    category="menu",
                    tier=TIER_EXTRA_LULO_ONLY,
                    label=lulo_item["label"],
                    path=" ▸ ".join(path),
                    lulo_shortcut=lulo_item["shortcut"],
                    note="present in Lulo but not found on the Mac",
                )
            )
    return gaps


# ---------------------------------------------------------------------------
# Context menus (Finder only, today)
# ---------------------------------------------------------------------------


# Rows whose children are populated from installed apps/extensions (a
# machine-specific "Open With" app list, a third-party Finder Sync
# extension's own entries) or from Automator workflows the owner happens
# to have — never comparable across machines, and not something Lulo
# would port item-for-item. The row itself is still compared (Lulo having
# no "Open With" at all would still be a real gap); only its contents are
# skipped.
_MACHINE_SPECIFIC_SUBMENUS = {"Open With", "Always Open With", "Quick Actions"}


def flatten_context_rows(rows: list[dict]) -> list[str]:
    """Both sides represent a context menu as a flat, ordered list of rows
    (`{"type": ..., "label": ...}` for Lulo, a `dumpMenuItems`-shaped tree
    for the Mac). This returns just the ordered, normalised labels,
    dropping separators, so the two shapes compare the same way."""
    out = []
    for row in rows:
        label = row.get("label")
        if label is None:
            continue
        norm_label = norm.normalize_label(label)
        if norm_label:
            out.append(norm_label)
        if norm_label in _MACHINE_SPECIFIC_SUBMENUS:
            continue
        children = row.get("children")
        if children:
            out.extend(flatten_context_rows(children))
    return out


# A background (no-selection) capture that still contains these labels is
# almost certainly the per-item menu leaking through (see
# mac_inventory.py's dump_finder_context_menus): AXShowMenu on macOS 26.2
# does not reliably follow a cleared AXSelectedRows. Newer captures already
# self-filter this at the source; this guard also protects against an
# older tests/inventory/mac/Finder.json committed before that fix existed.
_SELECTION_LEAK_TELLTALES = {"Rename", "Move to Bin", "Duplicate"}


def _is_contaminated_background_capture(rows: list[dict]) -> bool:
    top_labels = {r.get("label") for r in rows if r.get("label")}
    return bool(_SELECTION_LEAK_TELLTALES & top_labels)


def diff_context_menus(
    app: str, mac_menus: dict[str, list[dict]], lulo_menus: dict[str, list[dict]]
) -> list[Gap]:
    gaps: list[Gap] = []
    # Only compare the categories the Mac side actually captured a result
    # for, and that are meant to be compared (keys starting with "_" in
    # the Lulo file are helper submenus already inlined elsewhere).
    categories = [c for c in ("file", "folder", "background") if mac_menus.get(c)]
    if "background" in categories and _is_contaminated_background_capture(
        mac_menus.get("background", [])
    ):
        categories.remove("background")
    for category in categories:
        mac_rows = set(flatten_context_rows(mac_menus.get(category, [])))
        lulo_rows = set(flatten_context_rows(lulo_menus.get(category, [])))
        for label in sorted(mac_rows - lulo_rows):
            gaps.append(
                Gap(
                    app=app,
                    category="context",
                    tier=TIER_CONTEXT_MENU,
                    label=label,
                    path=f"context menu ▸ {category}",
                    note="missing from Lulo's context menu",
                )
            )
        for label in sorted(lulo_rows - mac_rows):
            gaps.append(
                Gap(
                    app=app,
                    category="context",
                    tier=TIER_EXTRA_LULO_ONLY,
                    label=label,
                    path=f"context menu ▸ {category}",
                    note="present in Lulo but not found on the Mac",
                )
            )
    return gaps


# ---------------------------------------------------------------------------
# Settings window / toolbar (coarse: label-set comparison)
# ---------------------------------------------------------------------------


def _labels_from_mac_controls(controls: list[dict]) -> set[str]:
    out = set()
    for c in controls:
        # These are AX tree scaffolding or window chrome, not settings that
        # someone can configure. Their fallback labels are generated from
        # the role when the node has no accessible name.
        if c.get("role") in {
            "AXGroup", "AXTabGroup", "AXRadioGroup", "AXScrollArea",
            "AXScrollBar", "AXTable", "AXRow", "AXColumn", "AXToolbar",
            "AXValueIndicator",
        }:
            continue
        if c.get("label") in {
            "close button", "minimise button", "zoom button",
            "increment arrow button", "decrement arrow button",
            "increment page button", "decrement page button",
            "action", "text", "Finder Settings",
        }:
            continue
        if (c.get("role"), c.get("label")) in {
            ("AXHeading", "heading"),
            ("AXCheckBox", "tickbox"),
        }:
            continue
        label = c.get("label")
        norm_label = norm.normalize_label(label) if label else None
        if norm_label:
            out.add(norm_label)
    return out


def _labels_from_lulo_controls(controls: list[dict]) -> set[str]:
    out = set()
    for c in controls:
        label = c.get("label")
        norm_label = norm.normalize_label(label) if label else None
        if norm_label:
            out.add(norm_label)
    return out


def diff_settings(app: str, mac_settings: dict, lulo_settings: dict) -> list[Gap]:
    mac_present = bool(mac_settings.get("present"))
    lulo_present = bool(lulo_settings.get("present"))
    if mac_present and not lulo_present:
        return [
            Gap(
                app=app,
                category="settings",
                tier=TIER_SETTINGS,
                label="Settings window",
                path="settings",
                note="the Mac has a Settings window for this app; Lulo has none yet",
            )
        ]
    if not mac_present or not lulo_present:
        return []
    mac_labels = _labels_from_mac_controls(mac_settings.get("controls", []))
    lulo_labels = _labels_from_lulo_controls(lulo_settings.get("controls", []))
    gaps = []
    for label in sorted(mac_labels - lulo_labels):
        gaps.append(
            Gap(
                app=app,
                category="settings",
                tier=TIER_SETTINGS,
                label=label,
                path="settings",
                note="seen on the Mac's Settings window, not found in Lulo's",
            )
        )
    return gaps


def diff_toolbar(app: str, mac_toolbar: dict, lulo_toolbar: dict) -> list[Gap]:
    mac_present = bool(mac_toolbar.get("present"))
    lulo_present = bool(lulo_toolbar.get("present"))
    if mac_present and not lulo_present:
        return [
            Gap(
                app=app,
                category="toolbar",
                tier=TIER_TOOLBAR,
                label="Toolbar",
                path="toolbar",
                note="the Mac has a toolbar for this app; Lulo has none implemented yet",
            )
        ]
    if app != "Notes" or not mac_present:
        return []
    # Notes' Mac capture names each real button. The Lulo extractor reads
    # .aria_label, accessible_icon_button and PopUpButton::new labels.
    # Groups, the search field and its anonymous clear button are layout,
    # not separate commands.
    mac_items = {
        norm.normalize_label(item.get("label")): item.get("label")
        for item in mac_toolbar.get("items", [])
        if item.get("role") in {"AXButton", "AXMenuButton"}
        and item.get("label") not in {None, "button"}
    }
    lulo_labels = {norm.normalize_label(label) for label in lulo_toolbar.get("groups", [])}
    return [
        Gap(
            app=app,
            category="toolbar",
            tier=TIER_TOOLBAR,
            label=label,
            path="toolbar",
            note="named Mac toolbar command absent from Lulo's toolbar source",
        )
        for normalized, label in sorted(mac_items.items())
        if normalized not in lulo_labels
    ]


# ---------------------------------------------------------------------------
# System Settings sidebar
# ---------------------------------------------------------------------------


def diff_sidebar(mac_sidebar: list[str], lulo_sidebar: list[dict]) -> list[Gap]:
    mac_labels = {norm.normalize_label(x) for x in mac_sidebar if x}
    lulo_labels = {norm.normalize_label(row.get("label")) for row in lulo_sidebar}
    gaps = []
    for label in sorted(mac_labels - lulo_labels):
        gaps.append(
            Gap(
                app="System Settings",
                category="sidebar",
                tier=TIER_SIDEBAR,
                label=label,
                path="sidebar",
                note="missing from Lulo's System Settings sidebar",
            )
        )
    for label in sorted(lulo_labels - mac_labels):
        gaps.append(
            Gap(
                app="System Settings",
                category="sidebar",
                tier=TIER_EXTRA_LULO_ONLY,
                label=label,
                path="sidebar",
                note="present in Lulo's sidebar but not found on the Mac",
            )
        )
    return gaps


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def load_json(path: Path) -> dict | None:
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as error:
        print(f"could not parse {path}: {error}", file=sys.stderr)
        return None


def diff_app(mac_app_name: str, lulo_app_name: str) -> tuple[list[Gap], list[str]]:
    """Returns (gaps, notes) for one app. `notes` records missing input
    files instead of silently producing zero gaps for them."""
    notes: list[str] = []
    mac_path = MAC_DIR / f"{mac_app_name}.json"
    lulo_path = LULO_DIR / f"{lulo_app_name}.json"
    mac_data = load_json(mac_path)
    lulo_data = load_json(lulo_path)
    if mac_data is None:
        notes.append(f"no Mac inventory for {mac_app_name} ({mac_path} not found)")
        return [], notes
    if lulo_data is None:
        notes.append(f"no Lulo inventory for {lulo_app_name} ({lulo_path} not found)")
        return [], notes
    if "error" in mac_data and not mac_data.get("menu_bar"):
        notes.append(f"Mac inventory for {mac_app_name} failed: {mac_data['error']}")
        return [], notes

    gaps: list[Gap] = []
    if "menu_bar" in mac_data:
        lulo_menu_bar = lulo_data.get("menu_bar", [])
        # `mac_inventory.py`'s `dump_menu_bar` reads one top-level menu per
        # osascript call so one slow/stuck menu (a dynamic window list, a
        # profile list) does not lose the whole bar; a menu listed here
        # simply was not captured this run. Diffing it anyway would report
        # every one of Lulo's items under that menu as "extra" (not every
        # Mac item as "missing", since the Mac side just has no entry for
        # that top-level menu at all) — exclude it from both sides instead
        # and say so, rather than record a false gap.
        partial_errors = mac_data.get("menu_bar_partial_errors", {})
        if partial_errors:
            failed_labels = set(partial_errors)
            lulo_menu_bar = [m for m in lulo_menu_bar if m.get("label") not in failed_labels]
            for label, error in partial_errors.items():
                notes.append(
                    f"{mac_app_name}'s {label!r} menu was not captured: {error}; "
                    "excluded from the menu diff"
                )
        gaps += diff_menu_bars(
            lulo_app_name, mac_data["menu_bar"], lulo_menu_bar, mac_app_name
        )
    else:
        notes.append(
            f"Mac menu bar for {mac_app_name} was not captured: "
            f"{mac_data.get('menu_bar_error', 'no menu data')}; menu diff skipped"
        )
    if "context_menus" in mac_data and "context_menus" in lulo_data:
        mac_context = mac_data.get("context_menus", {})
        if _is_contaminated_background_capture(mac_context.get("background", [])):
            notes.append(
                f"{mac_app_name}'s captured background context menu looks like a "
                "per-item menu (AXShowMenu quirk); skipped rather than diffed — "
                "re-run mac_inventory.py to try again"
            )
        gaps += diff_context_menus(lulo_app_name, mac_context, lulo_data.get("context_menus", {}))
    gaps += diff_settings(lulo_app_name, mac_data.get("settings", {}), lulo_data.get("settings", {}))
    gaps += diff_toolbar(lulo_app_name, mac_data.get("toolbar", {}), lulo_data.get("toolbar", {}))
    if lulo_app_name == "System Settings":
        mac_sidebar = mac_data.get("settings_app", {}).get("sidebar", [])
        lulo_sidebar = lulo_data.get("sidebar", [])
        if mac_sidebar:
            gaps += diff_sidebar(mac_sidebar, lulo_sidebar)
        else:
            notes.append("Mac System Settings sidebar was not captured; sidebar diff skipped")
    return gaps, notes


def assign_ids(gaps: list[Gap]) -> None:
    counters: dict[str, int] = {}
    for gap in gaps:
        code = APP_CODES.get(gap.app, gap.app[:3].upper())
        key = f"{code}-{gap.category.upper()}"
        counters[key] = counters.get(key, 0) + 1
        gap.gap_id = f"{key}-{counters[key]:03d}"


def render_markdown(gaps: list[Gap], notes: list[str], allowlist_count: int) -> str:
    lines = [
        "# Lulo vs. macOS feature inventory gaps",
        "",
        "Generated by `scripts/inventory/diff.py` from",
        "`tests/inventory/mac/*.json` (read from the real Mac's Accessibility",
        "tree by `scripts/inventory/mac_inventory.py`) and",
        "`tests/inventory/lulo/*.json` (read from Lulo's own source by",
        "`scripts/inventory/lulo_inventory.py`). Do not hand-edit the gap rows;",
        "re-run the pipeline and edit `tests/inventory/allowlist.json` instead",
        "if a row should never appear.",
        "",
        "Sorted by likely user impact: menu items with a keyboard shortcut and",
        "context-menu items first, then shortcut mismatches, then settings and",
        "sidebar gaps, then Lulo-only extras last (informational, not a gap to",
        "fix).",
        "",
        "To re-run: `python3 scripts/inventory/lulo_inventory.py && "
        "python3 scripts/inventory/mac_inventory.py && "
        "python3 scripts/inventory/diff.py` (the Mac step needs to run on the "
        "reference Mac, unlocked; see the note at the bottom if it has not run yet).",
        "",
        f"_{len(gaps)} gaps across {len({g.app for g in gaps})} apps; "
        f"{allowlist_count} Mac-only items were allowlisted (see "
        "`tests/inventory/allowlist.json`)._",
        "",
    ]
    if notes:
        lines.append("## Coverage notes")
        lines.append("")
        for note in notes:
            lines.append(f"- {note}")
        lines.append("")
        incomplete = sorted(
            {app for app in norm.APP_NAMES if any(app in note for note in notes)}
        )
        if incomplete:
            lines.append(
                "To finish this run once the Mac is free: "
                f"`python3 scripts/inventory/mac_inventory.py {' '.join(repr(a) if ' ' in a else a for a in incomplete)} "
                "&& python3 scripts/inventory/diff.py`."
            )
            lines.append("")

    by_app: dict[str, list[Gap]] = {}
    for gap in gaps:
        by_app.setdefault(gap.app, []).append(gap)

    for app in sorted(by_app):
        app_gaps = sorted(by_app[app], key=lambda g: (g.tier, g.category, g.path, g.label))
        lines.append(f"## {app}")
        lines.append("")
        lines.append("| id | impact | item | where | Mac shortcut | Lulo shortcut | note |")
        lines.append("|---|---|---|---|---|---|---|")
        for gap in app_gaps:
            lines.append(
                f"| {gap.gap_id} | {TIER_NAMES[gap.tier]} | {gap.label} | {gap.path} | "
                f"{gap.mac_shortcut} | {gap.lulo_shortcut} | {gap.note} |"
            )
        lines.append("")

    if not gaps:
        lines.append("No gaps recorded (either everything matched, or no inventory data was "
                      "available yet — see Coverage notes above).")
        lines.append("")

    return "\n".join(lines)


def main() -> int:
    allowlist = load_allowlist()
    all_gaps: list[Gap] = []
    all_notes: list[str] = []
    allowlisted_count = 0

    for mac_app_name, lulo_app_name in norm.APP_NAMES.items():
        gaps, notes = diff_app(mac_app_name, lulo_app_name)
        all_notes.extend(notes)
        kept = []
        for gap in gaps:
            reason = is_allowlisted(gap.app, gap.label, allowlist)
            if reason is not None:
                allowlisted_count += 1
                continue
            kept.append(gap)
        all_gaps.extend(kept)

    assign_ids(all_gaps)
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(render_markdown(all_gaps, all_notes, allowlisted_count))
    print(f"wrote {OUT_PATH.relative_to(REPO_ROOT)}: {len(all_gaps)} gaps, "
          f"{allowlisted_count} allowlisted, {len(all_notes)} coverage notes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
