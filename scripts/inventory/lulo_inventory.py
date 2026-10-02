#!/usr/bin/env python3
"""Build a feature inventory of Lulo's own apps, read from source.

Primary source of truth: the static menu tables in
`crates/rmac-app-menu/src/lib.rs` (`FILES_MENUS`, `TEXT_EDITOR_MENUS`, ...),
which is what each app publishes on the session bus as its own menus (see
`rmac_app_menu::definition`). We parse those tables with `rust_menu_parser`
and apply the same transform the app side applies at runtime
(`definition_for_vocabulary` in that file): certain Finder actions get a
localized label ("Move to Bin" instead of the table's literal "Move to
Trash") from `rmac_locale::FileVocabulary`; we hardcode the Linux/English
result since that is what ships.

That published menu is not what actually shows in the menu bar, though:
the menu bar (`shell/bins/rmac-menubar/src/main.rs`'s `bar_menus`) pulls
the app's own "Application"/"Window"/"Help" rows out of what it exported
and folds them into three standard menus synthesized for *every* app,
first-party or not (`app_menu`/`window_menu`/`help_menu` in that file and
in `menu_model.rs`): About <App>, Services, Hide/Hide Others/Show
All/Quit <App> (Files/Finder is never quit); Minimise, Minimise All,
Zoom, Fill, Centre, Move & Resize, the app's own Window items, Bring All
to Front; and a Help menu with "<App> Help" plus the app's own Help
items. `_synthesize_app_menu`/`_synthesize_window_menu`/
`_synthesize_help_menu` below mirror that assembly exactly, because a
Lulo-only static reading of the per-app table alone makes every one of
these rows look missing even though they are on screen (menu items with
a keyboard shortcut are the diff's highest-impact tier, so this used to
be the single biggest source of noise in docs/inventory-gaps.md: 76 false
gaps each for Calculator, Clock and Weather).

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
    "finder::EmptyTrashImmediately": "Empty Bin",
}

ABOUT_ACTION = "rmac::ShowAboutPanel"
APPLICATION_MENU = "Application"
WINDOW_MENU = "Window"
HELP_MENU = "Help"

# The one app (Files, aka Finder) that can never be quit (§2.9 of the
# design notes); every other app gets "Quit <App>" in its synthesized
# Application menu.
NEVER_QUIT = {"Finder"}


def _item(
    label: str, action: str, shortcut: str = "", separator_before: bool = False
) -> rmp.MenuItem:
    return rmp.MenuItem(
        label=label, action=action, shortcut=shortcut, separator_before=separator_before
    )


def apply_label_overrides(item: rmp.MenuItem) -> None:
    if item.action in LABEL_OVERRIDES:
        item.label = LABEL_OVERRIDES[item.action]
    for child in item.children:
        apply_label_overrides(child)


def _take_menu(menus: list[rmp.Menu], label: str) -> list[rmp.MenuItem]:
    """Removes the exported menu named `label` from `menus` (if present) and
    returns its items, mirroring `rmac_app_menu::take_application_items` /
    `take_window_items` / `take_help_items` (crates/rmac-app-menu/src/lib.rs),
    which the menu bar calls before it assembles the standard menus below."""
    for index, menu in enumerate(menus):
        if menu.label == label:
            return menus.pop(index).items
    return []


def _synthesize_app_menu(
    app_display_name: str, exported_items: list[rmp.MenuItem]
) -> rmp.Menu:
    """The bold-name Application menu every app gets, exported or not
    (`app_menu` in shell/bins/rmac-menubar/src/main.rs, §3.3): About, the
    app's own items (Settings…, Files' Empty Trash…), Services, Hide/Hide
    Others/Show All, then Quit (Files/Finder is never quit)."""
    menu_name = "Activity Monitor" if app_display_name == "System Monitor" else app_display_name
    items = [_item(f"About {menu_name}", ABOUT_ACTION)]
    keep_windows = next(
        (item for item in exported_items if item.label == "Quit and Keep Windows"),
        None,
    )
    if keep_windows is not None:
        exported_items.remove(keep_windows)
    if exported_items:
        exported_items[0].separator_before = True
        items.extend(exported_items)
    items.extend(
        [
            _item("Services", "app::services", "", separator_before=True),
            _item(f"Hide {menu_name}", "app::hide", "⌘H", separator_before=True),
            _item("Hide Others", "app::hide-others", "⌥⌘H"),
            _item("Show All", "app::show-all"),
        ]
    )
    if app_display_name not in NEVER_QUIT:
        items.append(
            _item(f"Quit {menu_name}", "app::quit", "⌘Q", separator_before=True)
        )
        if keep_windows is not None:
            items.append(keep_windows)
        elif app_display_name == "System Settings":
            items.append(_item("Quit and Keep Windows", "app::quit-keep-windows", "⌥⌘Q"))
    return rmp.Menu(APPLICATION_MENU, items)


def _synthesize_window_menu(exported_items: list[rmp.MenuItem]) -> rmp.Menu:
    """The standard Window menu every app gets (`window_menu` in
    shell/bins/rmac-menubar/src/menu_model.rs): Minimise, Minimise All,
    Zoom, Fill, Centre, Move & Resize, the app's own Window items (Files'
    tabs), then Bring All to Front. The live window list is left out: it is
    a runtime fact (open windows), not something static source analysis can
    read."""
    move_and_resize = rmp.MenuItem(
        label="Move & Resize",
        action="window::move-and-resize",
        shortcut="",
        separator_before=True,
        children=[
            _item("Halves", "window::heading-halves"),
            _item("Left", "window::tile-left", "⌃⌘←"),
            _item("Right", "window::tile-right", "⌃⌘→"),
            _item("Top", "window::tile-top", "⌃⌘↑"),
            _item("Bottom", "window::tile-bottom", "⌃⌘↓"),
            _item("Quarters", "window::heading-quarters", separator_before=True),
            _item("Top Left", "window::tile-top-left"),
            _item("Top Right", "window::tile-top-right"),
            _item("Bottom Left", "window::tile-bottom-left"),
            _item("Bottom Right", "window::tile-bottom-right"),
            _item(
                "Return to Previous Size",
                "window::restore-size",
                "⌃⇧⌘R",
                separator_before=True,
            ),
        ],
    )
    items = [
        _item("Minimise", "window::minimise", "⌘M"),
        _item("Minimise All", "app::minimise-all", "⌥⌘M"),
        _item("Zoom", "window::zoom"),
        _item("Fill", "window::fill", "⌃⇧⌘F"),
        _item("Centre", "window::centre", "⌃⌘C"),
        move_and_resize,
    ]
    if exported_items:
        exported_items[0].separator_before = True
        items.extend(exported_items)
    items.append(
        _item("Bring All to Front", "window::bring-all-to-front", "", separator_before=True)
    )
    return rmp.Menu(WINDOW_MENU, items)


def _synthesize_help_menu(
    app_display_name: str, exported_items: list[rmp.MenuItem]
) -> rmp.Menu:
    """The standard Help menu every app gets (`help_menu` in
    shell/bins/rmac-menubar/src/menu_model.rs): a Spotlight-style search
    field (left out here: it is an AXTextField on the Mac, not a menu item,
    and Lulo's never registers it as one either), then "<App> Help" (no key
    equivalent for Preview, Terminal and Text Editor, as on the Mac), then the
    app's own Help items."""
    menu_name = "Activity Monitor" if app_display_name == "System Monitor" else app_display_name
    shortcut = "" if app_display_name in {"Preview", "Terminal", "Text Editor"} else "⌘?"
    items = [_item(f"{menu_name} Help", "help::app-help", shortcut)]
    items.extend(exported_items)
    return rmp.Menu(HELP_MENU, items)


def read_menu_bar(app_display_name: str, table_name: str) -> list[dict]:
    source = APP_MENU_SRC.read_text()
    table_src = rmp.extract_const_table(source, table_name)
    menus = rmp.parse_menu_spec_table(table_src)
    for menu in menus:
        for item in menu.items:
            apply_label_overrides(item)
    # Mirrors `bar_menus` in shell/bins/rmac-menubar/src/main.rs: the menu
    # bar pulls the app's own Application/Window/Help rows out of its
    # exported menus and folds them into the three standard menus every
    # app gets, rather than showing them as the app published them.
    application_items = _take_menu(menus, APPLICATION_MENU)
    window_items = _take_menu(menus, WINDOW_MENU)
    help_items = _take_menu(menus, HELP_MENU)
    menus.insert(0, _synthesize_app_menu(app_display_name, application_items))
    menus.append(_synthesize_window_menu(window_items))
    menus.append(_synthesize_help_menu(app_display_name, help_items))
    return [m.to_dict() for m in menus]


# ---------------------------------------------------------------------------
# System Settings sidebar (exact) + best-effort per-pane row labels.
# ---------------------------------------------------------------------------


def read_settings_sidebar() -> list[dict]:
    source = NAV_SRC.read_text()
    # PANE_ROUTES also contains destinations nested under General. The
    # renderer omits those from the sidebar via category_parent().
    parent_rule = re.search(
        r"fn category_parent\(.*?matches!\(\s*name,\s*(.*?)\)\s*\.then_some",
        source,
        re.S,
    )
    nested = set(re.findall(r'"([^"]+)"', parent_rule.group(1))) if parent_rule else set()
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
        if name in nested:
            continue
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
        "crates/terminal/src/profiles.rs",
    ],
    "Text Editor": [
        "crates/text-editor/src/settings_window.rs",
        "crates/text-editor/src/settings.rs",
    ],
    "Weather": [
        "crates/weather/src/settings_window.rs",
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
        if app_display_name == "Terminal" and path.name == "profiles.rs":
            # These labels are rendered through PROFILES.iter(), so the
            # generic literal scanner below cannot see them in the view.
            for name in re.findall(r'\bname:\s*"([^"]+)"', text):
                key = ("profile", name)
                if key not in seen:
                    seen.add(key)
                    labels.append({"kind": "profile", "label": name})
            continue
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
        if app_display_name == "Text Editor" and path.name == "settings_window.rs":
            # number_row renders its unit through `.child(unit)`, so the
            # literal scanner above cannot see the visible AX text.
            for unit in re.findall(
                r'Self::number_row\(\s*"[^"]+",\s*"[^"]+",\s*[^,]+,\s*"([^"]+)"',
                text,
            ):
                key = ("control", unit)
                if key not in seen:
                    seen.add(key)
                    labels.append({"kind": "control", "label": unit})
        if app_display_name == "Text Editor" and path.name == "settings.rs":
            # The default window dimensions appear as text input values.
            for field in ("width_chars", "height_lines"):
                match = re.search(rf'\b{field}:\s*(\d+)\b', text)
                if match is not None:
                    value = match.group(1)
                    key = ("control", value)
                    if key not in seen:
                        seen.add(key)
                        labels.append({"kind": "control", "label": value})
        for m in re.finditer(r'\.label\("([^"]+)"\)', text):
            key = ("control", m.group(1))
            if key not in seen:
                seen.add(key)
                labels.append({"kind": "control", "label": m.group(1)})
        if app_display_name == "Text Editor" and path.name == "settings_window.rs":
            for m in re.finditer(r'\.(?:aria_label|child)\("([^"]+)"\)', text):
                key = ("control", m.group(1))
                if key not in seen:
                    seen.add(key)
                    labels.append({"kind": "control", "label": m.group(1)})
        if app_display_name == "Terminal" and path.name == "settings_window.rs":
            for label in ("Blink cursor", "Use Option as Meta Key"):
                if f'.child("{label}")' in text:
                    labels.append({"kind": "control", "label": label})
    return {"present": True, "extraction": "heuristic", "controls": labels}


# ---------------------------------------------------------------------------
# Toolbar (heuristic: accessible names in the app's toolbar source).
# ---------------------------------------------------------------------------

TOOLBAR_FILES = {
    "Finder": ["crates/finder/src/view/chrome_presentation/toolbar.rs"],
    "Notes": ["crates/notes/src/toolbar.rs"],
    "System Settings": ["crates/system-settings/src/controller/chrome.rs"],
    "System Monitor": ["crates/activity-monitor/src/view/render/chrome.rs"],
    "Calculator": ["crates/calculator/src/view.rs"],
    "Clock": ["crates/clock/src/view.rs"],
    "Weather": ["crates/weather/src/view.rs"],
}


def read_toolbar(app_display_name: str) -> dict:
    files = TOOLBAR_FILES.get(app_display_name, [])
    existing = [REPO_ROOT / f for f in files if (REPO_ROOT / f).exists()]
    if not existing:
        return {"present": False, "extraction": "none", "groups": []}
    groups: list[str] = []
    for path in existing:
        text = rmp.strip_line_comments(path.read_text())
        patterns = [r'\.aria_label\("([^"]+)"\)']
        if app_display_name == "Notes":
            # Notes names glyph buttons through its accessibility wrapper
            # and menus through PopUpButton::new, not .aria_label calls.
            patterns.extend([
                r'accessible_icon_button\(\s*"[^"]+"\s*,\s*"([^"]+)"',
                r'PopUpButton::new\(\s*"[^"]+"\s*,\s*"([^"]+)"',
            ])
        if app_display_name == "Calculator":
            # Only the two calls inside render_toolbar are toolbar controls;
            # the other aria labels in this file name keys and the display.
            patterns = [r'button\(\s*"[^"]+"\s*,\s*"([^"]+)"']
        if app_display_name == "Clock":
            # The tab names are passed through `label`, not a string literal
            # at the aria_label call. Limit extraction to the TABS table.
            patterns = [r'\.aria_label\("(Clock tabs)"\)']
            tabs = re.search(r'const TABS:.*?=\s*\[(.*?)\];', text, re.S)
            if tabs:
                for label in re.findall(r'\(Tab::\w+,\s*"([^"]+)"\)', tabs.group(1)):
                    if label not in groups:
                        groups.append(label)
        for pattern in patterns:
            for m in re.finditer(pattern, text):
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
        elif first == "copy_pathname_label":
            label = "Copy as Pathname"
        elif first == "label":
            # This dynamic variable is the label returned by
            # compress_menu_label for the selected item(s).
            label = "Compress"
        elif first == "slideshow_label":
            label = "Slideshow"
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
        # The source contains mutually exclusive branches and builders for
        # nested submenus. Record the single-selection, top-level result in
        # each category; nested app handlers and tag swatches are not rows of
        # the selected item's menu itself.
        nested = {
            "<handler.name.clone()>", "Other…", "Red", "Orange", "Yellow",
            "Green", "Blue", "Purple", "Gray", "New Folder with Selection",
        }
        menus["file"] = [
            row for row in rows
            if row.get("label") not in nested | {"Open in New Tab", "Open in New Window"}
        ]
        menus["folder"] = [
            row for row in rows
            if row.get("label") not in nested | {"Open", "Open With", "Always Open With"}
        ]
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
