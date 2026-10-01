#!/usr/bin/env python3
"""Build a feature inventory of Lulo's own apps, read from source.

Primary source of truth: the static menu tables in
`crates/rmac-app-menu/src/lib.rs` (`FILES_MENUS`, `TEXT_EDITOR_MENUS`, ...),
which is exactly what each app publishes on the session bus as its menu
bar (see `rmac_app_menu::definition`). We parse those tables with
`rust_menu_parser` and apply the same two small transforms the app side
applies at runtime (`definition_for_vocabulary` in that file):

  1. Certain Finder actions get a localized label ("Move to Bin" instead of
     the table's literal "Move to Trash") from `rmac_locale::FileVocabulary`.
     We hardcode the Linux/English result since that is what ships.
  2. Every app that registers `rmac_app_menu::ABOUT_ACTION` gets an "About
     <App>" row inserted at the top of its "Application" menu (or a new
     one-item "Application" menu, if it declares none). All ten target
     apps are rmac-ui apps and register it.

Secondary sources, read best-effort and marked `"extraction": "heuristic"`
where the structure is inferred from string literals rather than a typed
table: Settings windows, toolbars. Context menus are parsed with a
bespoke (non-heuristic) reader for Finder, the only app with one today.

System Settings' sidebar comes from the exact `PANE_ROUTES` table in
`crates/system-settings/src/navigation.rs`.

Output: one JSON file per app under tests/inventory/lulo/<App Name>.json.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rust_menu_parser as rmp  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
APP_MENU_SRC = REPO_ROOT / "crates/rmac-app-menu/src/lib.rs"
NAV_SRC = REPO_ROOT / "crates/system-settings/src/navigation.rs"
OUT_DIR = REPO_ROOT / "tests/inventory/lulo"

# app display name -> (menu table const name, finder-style label overrides)
APP_TABLES = {
    "Finder": "FILES_MENUS",
    "Text Editor": "TEXT_EDITOR_MENUS",
    "Preview": "PREVIEW_MENUS",
    "Notes": "NOTES_MENUS",
    "Terminal": "TERMINAL_MENUS",
    "Calculator": "CALCULATOR_MENUS",
    "System Monitor": "MONITOR_MENUS",
    "System Settings": "SETTINGS_MENUS",
    "Clock": "CLOCK_MENUS",
    "Weather": "WEATHER_MENUS",
}

# Mirrors the `match spec.action` block in `resolve_items`
# (crates/rmac-app-menu/src/lib.rs) for the Linux-English `FileVocabulary`,
# where the trash is called "Bin".
LABEL_OVERRIDES = {
    "finder::MoveToTrash": "Move to Bin",
    "finder::GoTrash": "Bin",
    "finder::EmptyTrash": "Empty Bin…",
}

ABOUT_ACTION = "rmac::ShowAboutPanel"
APPLICATION_MENU = "Application"


def apply_label_overrides(item: rmp.MenuItem) -> None:
    if item.action in LABEL_OVERRIDES:
        item.label = LABEL_OVERRIDES[item.action]
    for child in item.children:
        apply_label_overrides(child)


def inject_about(menus: list[rmp.Menu], app_display_name: str) -> list[rmp.Menu]:
    about = rmp.MenuItem(
        label="About", action=ABOUT_ACTION, shortcut="", separator_before=False
    )
    for menu in menus:
        if menu.label == APPLICATION_MENU:
            menu.items.insert(0, about)
            return menus
    menus.insert(0, rmp.Menu(APPLICATION_MENU, [about]))
    return menus


def read_menu_bar(app_display_name: str, table_name: str) -> list[dict]:
    source = APP_MENU_SRC.read_text()
    table_src = rmp.extract_const_table(source, table_name)
    menus = rmp.parse_menu_spec_table(table_src)
    for menu in menus:
        for item in menu.items:
            apply_label_overrides(item)
    menus = inject_about(menus, app_display_name)
    return [m.to_dict() for m in menus]


# ---------------------------------------------------------------------------
# System Settings sidebar (exact) + best-effort per-pane row labels.
# ---------------------------------------------------------------------------


def read_settings_sidebar() -> list[dict]:
    source = NAV_SRC.read_text()
    m = re.search(
        r"PANE_ROUTES:\s*\[\(&str,\s*&str\);\s*\d+\]\s*=\s*\[(.*?)\];",
        source,
        re.S,
    )
    if not m:
        return []
    body = rmp.strip_line_comments(m.group(1))
    rows = []
    for entry in rmp.split_top_level(body, ","):
        entry = entry.strip()
        if not entry.startswith("("):
            continue
        inner = entry[1:-1] if entry.endswith(")") else entry[1:]
        parts = rmp.split_top_level(inner, ",")
        if len(parts) != 2:
            continue
        slug = parts[0].strip().strip('"')
        name = parts[1].strip().strip('"')
        rows.append({"id": slug, "label": name})
    return rows


# ---------------------------------------------------------------------------
# Settings window (heuristic: string literals in the app's settings* files).
# ---------------------------------------------------------------------------

SETTINGS_FILES = {
    "Finder": [
        "crates/finder/src/view/settings_window.rs",
        "crates/finder/src/view/settings.rs",
    ],
    "Terminal": [
        "crates/terminal/src/settings_window.rs",
        "crates/terminal/src/settings.rs",
    ],
}


def read_settings_window(app_display_name: str) -> dict:
    files = SETTINGS_FILES.get(app_display_name, [])
    existing = [REPO_ROOT / f for f in files if (REPO_ROOT / f).exists()]
    if not existing:
        return {"present": False, "extraction": "none", "controls": []}
    labels: list[dict] = []
    seen = set()
    for path in existing:
        text = rmp.strip_line_comments(path.read_text())
        for m in re.finditer(r'Self::\w+\s*=>\s*"([^"]+)"', text):
            key = ("tab", m.group(1))
            if key not in seen:
                seen.add(key)
                labels.append({"kind": "tab", "label": m.group(1)})
        for m in re.finditer(r'section_label\("([^"]+)"\)', text):
            key = ("section", m.group(1))
            if key not in seen:
                seen.add(key)
                labels.append({"kind": "section", "label": m.group(1)})
        for m in re.finditer(r'"settings-[\w-]+"\s*,\s*\n?\s*"([^"]+)"', text):
            key = ("control", m.group(1))
            if key not in seen:
                seen.add(key)
                labels.append({"kind": "control", "label": m.group(1)})
        for m in re.finditer(r'\.label\("([^"]+)"\)', text):
            key = ("control", m.group(1))
            if key not in seen:
                seen.add(key)
                labels.append({"kind": "control", "label": m.group(1)})
    return {"present": True, "extraction": "heuristic", "controls": labels}


# ---------------------------------------------------------------------------
# Toolbar (heuristic: .aria_label("...") literals in the app's toolbar file).
# ---------------------------------------------------------------------------

TOOLBAR_FILES = {
    "Finder": ["crates/finder/src/view/chrome_presentation/toolbar.rs"],
    "Notes": ["crates/notes/src/toolbar.rs"],
}


def read_toolbar(app_display_name: str) -> dict:
    files = TOOLBAR_FILES.get(app_display_name, [])
    existing = [REPO_ROOT / f for f in files if (REPO_ROOT / f).exists()]
    if not existing:
        return {"present": False, "extraction": "none", "groups": []}
    groups: list[str] = []
    for path in existing:
        text = rmp.strip_line_comments(path.read_text())
        for m in re.finditer(r'\.aria_label\("([^"]+)"\)', text):
            if m.group(1) not in groups:
                groups.append(m.group(1))
    return {"present": True, "extraction": "heuristic", "groups": groups}


# ---------------------------------------------------------------------------
# Finder context menus: a bespoke (non-heuristic) reader for
# crates/finder/src/view/chrome_presentation/menus_tabs.rs, the only app
# with a context-menu builder today.
# ---------------------------------------------------------------------------

_ROW_CALL = re.compile(
    r"\.(item|command_item|danger_command_item|checked_item|checked_item_with_swatch|separator|submenu|header)\s*\("
)


def _extract_balanced_call(text: str, start: int) -> tuple[str, int]:
    """`text[start]` is the `(` right after a method name; return (inner,
    index just past the matching `)`)."""
    close = rmp._find_matching_paren(text, start)
    return text[start + 1 : close], close + 1


def _rows_from_block(text: str) -> list[dict]:
    """Scan `text` for `.item(...)`-style chain calls in order and return a
    flat list of {"type", "label"} rows. Submenus are listed by label only
    (their own contents are resolved by the caller when known)."""
    rows: list[dict] = []
    for m in _ROW_CALL.finditer(text):
        kind = m.group(1)
        if kind == "separator":
            rows.append({"type": "separator"})
            continue
        if kind == "header":
            continue
        inner, _ = _extract_balanced_call(text, m.end() - 1)
        args = rmp.split_top_level(inner, ",")
        if not args:
            continue
        first = args[0].strip()
        if first.startswith('"'):
            label = rmp._strip_literal(first)
        elif first == "move_to_bin":
            label = "Move to Bin"
        else:
            # A non-literal expression (a local variable or a computed
            # label): we cannot resolve it statically, so mark it instead
            # of printing the Rust identifier bare (which could otherwise
            # be mistaken for an actual on-screen string).
            label = f"<{first}>"
        rows.append({"type": kind, "label": label})
    return rows


def _function_body(text: str, signature_pattern: str) -> str | None:
    m = re.search(signature_pattern, text)
    if not m:
        return None
    brace = text.index("{", m.end())
    close = rmp._brace_match(text, brace)
    return text[brace + 1 : close]


def _branch_block(body: str, if_pattern: str, *, require_else: bool = False) -> str | None:
    """Return the `{ ... }` immediately following the LAST match of
    `if_pattern` in `body` that is followed by an `else` block, if
    `require_else`; otherwise the last match regardless."""
    best = None
    for m in re.finditer(if_pattern, body):
        brace = body.index("{", m.end())
        close = rmp._brace_match(body, brace)
        if require_else:
            rest = body[close + 1 :]
            if not re.match(r"\s*else\s*\{", rest):
                continue
        best = (brace, close)
    if best is None:
        return None
    brace, close = best
    return body[brace + 1 : close]


def _else_block(body: str, if_pattern: str) -> str | None:
    for m in re.finditer(if_pattern, body):
        brace = body.index("{", m.end())
        close = rmp._brace_match(body, brace)
        rest = body[close + 1 :]
        em = re.match(r"\s*else\s*\{", rest)
        if not em:
            continue
        ebrace = rest.index("{", em.start())
        eclose = rmp._brace_match(rest, ebrace)
        return rest[ebrace + 1 : eclose]
    return None


def read_finder_context_menus() -> dict:
    path = REPO_ROOT / "crates/finder/src/view/chrome_presentation/menus_tabs.rs"
    if not path.exists():
        return {}
    text = rmp.strip_line_comments(path.read_text())

    body = _function_body(text, r"fn build_context_menu\s*\(")
    if body is None:
        return {}

    menus: dict[str, list[dict]] = {}

    has_sel_true = _branch_block(body, r"if\s+has_selection\s*\{", require_else=True)
    has_sel_false = _else_block(body, r"if\s+has_selection\s*\{")
    # A file and a folder both land in the has_selection branch today; Lulo
    # does not special-case folders.
    if has_sel_true:
        rows = _rows_from_block(has_sel_true)
        menus["file"] = rows
        menus["folder"] = rows
    if has_sel_false:
        menus["background"] = _rows_from_block(has_sel_false)

    trash_true = _branch_block(body, r"if\s+trash_view\s*\{")
    if trash_true:
        inner_sel = _branch_block(trash_true, r"if\s+has_selection\s*\{")
        if inner_sel:
            menus["trash_item"] = _rows_from_block(inner_sel)

    apps_true = _branch_block(body, r"if\s+applications_view\s*\{")
    if apps_true:
        menus["applications_background"] = _rows_from_block(apps_true)

    sort_body = _function_body(text, r"fn build_sort_menu\s*\(")
    if sort_body:
        menus["_sort_by_submenu"] = _rows_from_block(sort_body)

    view_body = _function_body(text, r"fn build_view_submenu\s*\(")
    if view_body:
        menus["_view_submenu"] = _rows_from_block(view_body)

    return menus


def build_app(app_display_name: str) -> dict:
    table_name = APP_TABLES[app_display_name]
    data = {
        "app": app_display_name,
        "source": "rust-static-analysis",
        "menu_bar": read_menu_bar(app_display_name, table_name),
        "settings": read_settings_window(app_display_name),
        "toolbar": read_toolbar(app_display_name),
        "context_menus": {},
    }
    if app_display_name == "Finder":
        data["context_menus"] = read_finder_context_menus()
        data["settings_sidebar"] = None  # Finder's own Settings window, not System Settings'
    if app_display_name == "System Settings":
        data["sidebar"] = read_settings_sidebar()
    return data


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    written = []
    for app_display_name in APP_TABLES:
        data = build_app(app_display_name)
        out_path = OUT_DIR / f"{app_display_name}.json"
        out_path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
        written.append(out_path)
    for path in written:
        print(f"wrote {path.relative_to(REPO_ROOT)}")


if __name__ == "__main__":
    main()
