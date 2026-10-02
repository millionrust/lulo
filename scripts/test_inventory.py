#!/usr/bin/env python3
"""Unit tests for scripts/inventory/{normalize,diff,mac_inventory,rust_menu_parser}.py.

Runs with plain unittest / pytest; no Mac, no Lulo binaries, no GUI. The
mac_inventory.py tests below never call osascript for real — they patch
`mac_inventory.run_osascript` (and, for the budget test, `time.monotonic`)
with canned output, and only exercise the parsing/assembly logic.
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent / "inventory"))

import diff as d  # noqa: E402
import lulo_inventory as li  # noqa: E402
import mac_inventory as mi  # noqa: E402
import normalize as norm  # noqa: E402
import rust_menu_parser as rmp  # noqa: E402


class NormalizeAppNameTests(unittest.TestCase):
    def test_known_apps_map_to_lulo_names(self):
        self.assertEqual(norm.normalize_app_name("Finder"), "Finder")
        self.assertEqual(norm.normalize_app_name("TextEdit"), "Text Editor")
        self.assertEqual(norm.normalize_app_name("Activity Monitor"), "System Monitor")

    def test_unknown_app_passes_through(self):
        self.assertEqual(norm.normalize_app_name("Freeform"), "Freeform")


class NormalizeLabelTests(unittest.TestCase):
    def test_trash_bin_alias_both_directions(self):
        self.assertEqual(norm.normalize_label("Trash"), norm.normalize_label("Bin"))
        self.assertEqual(
            norm.normalize_label("Move to Trash"), norm.normalize_label("Move to Bin")
        )

    def test_ellipsis_preserved_through_alias(self):
        a = norm.normalize_label("Empty Trash…")
        b = norm.normalize_label("Empty Bin…")
        self.assertEqual(a, b)
        self.assertTrue(a.endswith("…"))

    def test_unrelated_labels_differ(self):
        self.assertNotEqual(norm.normalize_label("Copy"), norm.normalize_label("Paste"))

    def test_whitespace_is_collapsed(self):
        self.assertEqual(norm.normalize_label("New   Folder"), norm.normalize_label("New Folder"))

    def test_nonbreaking_hyphen_matches_ascii_hyphen(self):
        self.assertEqual(norm.normalize_label("Wi‑Fi"), norm.normalize_label("Wi-Fi"))

    def test_none_passes_through(self):
        self.assertIsNone(norm.normalize_label(None))

    def test_finder_selection_labels_match_their_runtime_command(self):
        self.assertEqual(norm.normalize_label('Copy “report.txt” as Pathname'), "Copy as Pathname")
        self.assertEqual(norm.normalize_label('Quick Look “report.txt”'), "Quick Look")
        self.assertEqual(norm.normalize_label('Slideshow “report.txt”'), "Slideshow")
        self.assertEqual(norm.normalize_label('Compress “report.txt”'), "Compress")
        self.assertEqual(norm.normalize_label('Undo Move of “Untitled”'), "Undo")

    def test_finder_context_inventory_only_lists_single_selection_rows(self):
        menus = li.read_finder_context_menus()
        file_labels = {row.get("label") for row in menus["file"]}
        folder_labels = {row.get("label") for row in menus["folder"]}
        self.assertIn("Slideshow", file_labels)
        self.assertIn("Slideshow", folder_labels)
        self.assertIn("Open", file_labels)
        self.assertNotIn("Open", folder_labels)
        self.assertNotIn("New Folder with Selection", file_labels | folder_labels)
        self.assertNotIn("<handler.name.clone()>", file_labels | folder_labels)
        self.assertNotIn("Red", file_labels | folder_labels)


class NormalizeShortcutTests(unittest.TestCase):
    def test_hyphen_and_minus_sign_are_equal(self):
        self.assertEqual(norm.normalize_shortcut("⌘-"), norm.normalize_shortcut("⌘−"))

    def test_empty_shortcut_is_empty_string(self):
        self.assertEqual(norm.normalize_shortcut(""), "")
        self.assertEqual(norm.normalize_shortcut(None), "")

    def test_finder_private_use_up_arrow(self):
        self.assertEqual(norm.normalize_shortcut("⌘"), "⌘↑")


class SettingsCaptureTests(unittest.TestCase):
    def test_unnamed_ax_roles_and_window_title_are_not_controls(self):
        controls = [
            {"role": "AXHeading", "label": "heading"},
            {"role": "AXCheckBox", "label": "tickbox"},
            {"role": "AXStaticText", "label": "Finder Settings"},
            {"role": "AXCheckBox", "label": "Show all filename extensions"},
            {"role": "AXColorWell", "label": "rgb 0.9 0 0 1"},
            {"role": "AXTextField", "label": "text field"},
        ]
        self.assertEqual(
            d._labels_from_mac_controls(controls), {"Show all filename extensions"}
        )


class SystemSettingsSidebarTests(unittest.TestCase):
    def test_general_subpages_are_not_reported_as_sidebar_rows(self):
        labels = {row["label"] for row in li.read_settings_sidebar()}
        self.assertIn("General", labels)
        self.assertNotIn("Date & Time", labels)
        self.assertNotIn("Language & Region", labels)
        self.assertNotIn("Login Items", labels)
        self.assertNotIn("Sharing", labels)


class RustMenuParserTests(unittest.TestCase):
    def test_parses_a_simple_item(self):
        item = rmp.parse_macro_call('item!("New", "app::New", "⌘N")')
        self.assertEqual(item.label, "New")
        self.assertEqual(item.action, "app::New")
        self.assertEqual(item.shortcut, "⌘N")
        self.assertFalse(item.separator_before)
        self.assertEqual(item.children, [])

    def test_parses_item_with_separator(self):
        item = rmp.parse_macro_call('item!("Close", "app::Close", "⌘W", separator)')
        self.assertTrue(item.separator_before)

    def test_parses_a_submenu_with_children(self):
        item = rmp.parse_macro_call(
            'submenu!("Find", "app::FindMenu", '
            '[item!("Find…", "app::Find", "⌘F"), item!("Find Next", "app::FindNext", "⌘G")], '
            "separator)"
        )
        self.assertEqual(item.label, "Find")
        self.assertTrue(item.separator_before)
        self.assertEqual([c.label for c in item.children], ["Find…", "Find Next"])

    def test_parses_a_full_menu_spec_table(self):
        src = """
        const DEMO_MENUS: &[MenuSpec] = &[
            MenuSpec {
                label: "File",
                items: &[
                    item!("New", "app::New", "⌘N"),
                    item!("Close", "app::Close", "⌘W", separator),
                ],
            },
            MenuSpec {
                label: WINDOW_MENU,
                items: &[item!("Minimize", "app::Minimize", "⌘M")],
            },
        ];
        """
        table = rmp.extract_const_table(src, "DEMO_MENUS")
        menus = rmp.parse_menu_spec_table(table)
        self.assertEqual([m.label for m in menus], ["File", "Window"])
        self.assertEqual(len(menus[0].items), 2)
        self.assertTrue(menus[0].items[1].separator_before)

    def test_strips_line_comments_without_touching_strings(self):
        text = 'item!("A // not a comment", "a::A", "") // a real comment\n'
        stripped = rmp.strip_line_comments(text)
        self.assertIn('"A // not a comment"', stripped)
        self.assertNotIn("a real comment", stripped)

    def test_split_top_level_respects_nesting(self):
        parts = rmp.split_top_level('item!("A", "a", ""), item!("B", "b", "")', ",")
        # The whole thing has no top-level commas outside the two macro
        # calls' own parens, so split_top_level on the *outer* text (not
        # inside a single call) should still separate the two calls by
        # their outer commas correctly when used as parse_items_array does.
        items = rmp.parse_items_array('item!("A", "a", ""), item!("B", "b", "")')
        self.assertEqual([i.label for i in items], ["A", "B"])
        self.assertTrue(parts)  # sanity: produced something


class FlattenMenuItemsTests(unittest.TestCase):
    def test_flattens_nested_items_and_drops_separators(self):
        menus = [
            {
                "label": "File",
                "items": [
                    {"label": "New", "shortcut": "⌘N", "children": []},
                    {"label": None, "shortcut": "", "children": []},  # separator
                    {
                        "label": "Open Recent",
                        "shortcut": "",
                        "children": [
                            {"label": "Clear Menu", "shortcut": "", "children": []}
                        ],
                    },
                ],
            }
        ]
        flat = d.flatten_menu_items(menus)
        self.assertIn(("File", "New"), flat)
        self.assertIn(("File", "Open Recent"), flat)
        self.assertIn(("File", "Open Recent", "Clear Menu"), flat)
        # Exactly these three: the separator contributes no entry, and a
        # submenu row itself is a key as well as each of its children.
        self.assertEqual(len(flat), 3)

    def test_bold_menu_alias_maps_app_name_to_application(self):
        menus = [{"label": "TextEdit", "items": [{"label": "About", "shortcut": "", "children": []}]}]
        flat = d.flatten_menu_items(menus, bold_menu_alias="TextEdit")
        self.assertIn(("Application", "About"), flat)


class DiffMenuBarsTests(unittest.TestCase):
    def test_missing_item_with_shortcut_is_high_tier(self):
        mac = [{"label": "File", "items": [{"label": "Duplicate", "shortcut": "⌘D", "children": []}]}]
        lulo = [{"label": "File", "items": []}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].tier, d.TIER_MENU_WITH_SHORTCUT)
        self.assertEqual(gaps[0].label, "Duplicate")

    def test_missing_item_without_shortcut_is_lower_tier(self):
        mac = [{"label": "File", "items": [{"label": "Rename", "shortcut": "", "children": []}]}]
        lulo = [{"label": "File", "items": []}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(gaps[0].tier, d.TIER_MENU_NO_SHORTCUT)

    def test_matching_item_with_same_shortcut_is_not_a_gap(self):
        mac = [{"label": "File", "items": [{"label": "New", "shortcut": "⌘N", "children": []}]}]
        lulo = [{"label": "File", "items": [{"label": "New", "shortcut": "⌘N", "children": []}]}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(gaps, [])

    def test_shortcut_mismatch_is_reported(self):
        mac = [{"label": "File", "items": [{"label": "New", "shortcut": "⌘N", "children": []}]}]
        lulo = [{"label": "File", "items": [{"label": "New", "shortcut": "⇧⌘N", "children": []}]}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].tier, d.TIER_SHORTCUT_MISMATCH)

    def test_hyphen_vs_minus_shortcut_is_not_a_mismatch(self):
        mac = [{"label": "View", "items": [{"label": "Smaller", "shortcut": "⌘-", "children": []}]}]
        lulo = [{"label": "View", "items": [{"label": "Smaller", "shortcut": "⌘−", "children": []}]}]
        gaps = d.diff_menu_bars("App", mac, lulo, "App")
        self.assertEqual(gaps, [])

    def test_extra_lulo_only_item_is_lowest_tier(self):
        mac = [{"label": "File", "items": []}]
        lulo = [{"label": "File", "items": [{"label": "Experimental Thing", "shortcut": "", "children": []}]}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].tier, d.TIER_EXTRA_LULO_ONLY)

    def test_unread_dynamic_mac_children_are_not_lulo_extras(self):
        mac = [{"label": "File", "items": [{"label": "Open Recent", "shortcut": "",
                "children": [], "children_omitted": "dynamic/personal submenu, not read"}]}]
        lulo = [{"label": "File", "items": [{"label": "Open Recent", "shortcut": "",
                 "children": [{"label": "Clear Menu", "shortcut": "", "children": []}]}]}]
        self.assertEqual(d.diff_menu_bars("Text Editor", mac, lulo, "TextEdit"), [])

    def test_bold_app_menu_aliasing_lines_up_with_application(self):
        mac = [{"label": "Finder", "items": [{"label": "Settings…", "shortcut": "⌘,", "children": []}]}]
        lulo = [{"label": "Application", "items": [{"label": "Settings…", "shortcut": "⌘,", "children": []}]}]
        gaps = d.diff_menu_bars("Finder", mac, lulo, "Finder")
        self.assertEqual(gaps, [])


class DiffContextMenusTests(unittest.TestCase):
    def test_missing_row_reported_for_captured_category_only(self):
        mac = {
            "file": [{"label": "Open"}, {"label": "Get Info"}, {"label": None}],
            "background": [],  # Mac side failed to capture this one
        }
        lulo = {"file": [{"type": "item", "label": "Open"}], "background": []}
        gaps = d.diff_context_menus("Finder", mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].label, "Get Info")
        self.assertEqual(gaps[0].tier, d.TIER_CONTEXT_MENU)

    def test_extra_lulo_row_reported_as_lulo_only(self):
        mac = {"file": [{"label": "Open"}]}
        lulo = {"file": [{"type": "item", "label": "Open"}, {"type": "item", "label": "Quick Actions"}]}
        gaps = d.diff_context_menus("Finder", mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].label, "Quick Actions")
        self.assertEqual(gaps[0].tier, d.TIER_EXTRA_LULO_ONLY)


class ContaminatedBackgroundCaptureTests(unittest.TestCase):
    def test_selection_leak_is_detected(self):
        rows = [{"label": "Open"}, {"label": "Move to Bin"}, {"label": "Rename"}]
        self.assertTrue(d._is_contaminated_background_capture(rows))

    def test_genuine_background_menu_is_not_flagged(self):
        rows = [{"label": "New Folder"}, {"label": "Get Info"}, {"label": "Paste"}]
        self.assertFalse(d._is_contaminated_background_capture(rows))

    def test_contaminated_background_is_excluded_from_the_diff(self):
        mac = {
            "file": [{"label": "Open"}],
            "background": [{"label": "Open"}, {"label": "Move to Bin"}, {"label": "Rename"}],
        }
        lulo = {"file": [{"type": "item", "label": "Open"}], "background": []}
        gaps = d.diff_context_menus("Finder", mac, lulo)
        self.assertEqual(gaps, [])  # file matches; background was skipped, not diffed


class DiffSettingsAndToolbarTests(unittest.TestCase):
    def test_terminal_settings_inventory_includes_rendered_profile_rows(self):
        controls = li.read_settings_window("Terminal")["controls"]
        labels = {control["label"] for control in controls}
        self.assertIn("Clear Dark", labels)
        self.assertIn("Silver Aerogel", labels)
        self.assertIn("Blink cursor", labels)
        self.assertIn("▊ Block", labels)

    def test_text_editor_settings_inventory_includes_dynamic_values_and_units(self):
        controls = li.read_settings_window("Text Editor")["controls"]
        labels = {control["label"] for control in controls}
        self.assertTrue({"96", "30", "characters", "lines"} <= labels)

    def test_unnamed_ax_scaffolding_is_not_a_settings_control(self):
        mac = {"present": True, "controls": [
            {"role": "AXTable", "label": "table"},
            {"role": "AXButton", "label": "close button"},
            {"role": "AXCheckBox", "label": "Display ANSI colours"},
        ]}
        lulo = {"present": True, "controls": []}
        gaps = d.diff_settings("Terminal", mac, lulo)
        self.assertEqual([gap.label for gap in gaps], ["Display ANSI colours"])

    def test_terminal_captured_menu_is_compared(self):
        gaps, notes = d.diff_app("Terminal", "Terminal")
        self.assertTrue(any(gap.category == "menu" for gap in gaps))
        self.assertFalse(any("menu diff skipped" in note for note in notes))

    def test_settings_entirely_missing_in_lulo(self):
        mac = {"present": True, "controls": [{"label": "General"}]}
        lulo = {"present": False, "controls": []}
        gaps = d.diff_settings("Terminal", mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].tier, d.TIER_SETTINGS)

    def test_settings_present_both_sides_diffs_labels(self):
        mac = {"present": True, "controls": [{"label": "General"}, {"label": "Advanced"}]}
        lulo = {"present": True, "controls": [{"label": "General"}]}
        gaps = d.diff_settings("Finder", mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].label, "Advanced")

    def test_neither_present_is_not_a_gap(self):
        mac = {"present": False, "controls": []}
        lulo = {"present": False, "controls": []}
        self.assertEqual(d.diff_settings("Clock", mac, lulo), [])

    def test_toolbar_missing_entirely(self):
        mac = {"present": True, "items": [{"label": "Back"}]}
        lulo = {"present": False, "items": []}
        gaps = d.diff_toolbar("Finder", mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0].tier, d.TIER_TOOLBAR)

    def test_notes_toolbar_compares_named_commands(self):
        mac = {"present": True, "items": [
            {"role": "AXButton", "label": "New Note"},
            {"role": "AXMenuButton", "label": "Media"},
            {"role": "AXGroup", "label": "group"},
            {"role": "AXButton", "label": "button"},
        ]}
        lulo = {"present": True, "groups": ["New Note"]}
        self.assertEqual([gap.label for gap in d.diff_toolbar("Notes", mac, lulo)], ["Media"])

    def test_notes_toolbar_extractor_finds_wrapped_buttons_and_popups(self):
        toolbar = li.read_toolbar("Notes")
        self.assertTrue({"New Note", "View Options", "More"} <= set(toolbar["groups"]))


class DiffSidebarTests(unittest.TestCase):
    def test_missing_and_extra_panes(self):
        mac_sidebar = ["Wi-Fi", "Bluetooth", "Battery"]
        lulo_sidebar = [{"id": "wifi", "label": "Wi-Fi"}, {"id": "extra", "label": "Made Up Pane"}]
        gaps = d.diff_sidebar(mac_sidebar, lulo_sidebar)
        labels_missing = {g.label for g in gaps if g.tier == d.TIER_SIDEBAR}
        labels_extra = {g.label for g in gaps if g.tier == d.TIER_EXTRA_LULO_ONLY}
        self.assertEqual(labels_missing, {"Bluetooth", "Battery"})
        self.assertEqual(labels_extra, {"Made Up Pane"})


class AllowlistTests(unittest.TestCase):
    def test_exact_match_allowlisted(self):
        allowlist = [{"app": "*", "label": "Apple", "match": "exact", "reason": "OS chrome"}]
        self.assertEqual(d.is_allowlisted("Finder", "Apple", allowlist), "OS chrome")

    def test_prefix_match_allowlisted(self):
        allowlist = [{"app": "*", "label": "Share…", "match": "prefix", "reason": "share sheets"}]
        self.assertEqual(
            d.is_allowlisted("Finder", "Share… to Mail", allowlist), "share sheets"
        )

    def test_app_scoped_entry_does_not_leak_to_other_apps(self):
        allowlist = [{"app": "Finder", "label": "Thing", "match": "exact", "reason": "r"}]
        self.assertIsNone(d.is_allowlisted("Notes", "Thing", allowlist))

    def test_non_matching_label_is_not_allowlisted(self):
        allowlist = [{"app": "*", "label": "Apple", "match": "exact", "reason": "r"}]
        self.assertIsNone(d.is_allowlisted("Finder", "File", allowlist))


class AssignIdsTests(unittest.TestCase):
    def test_ids_are_stable_and_sequential_per_app_and_category(self):
        gaps = [
            d.Gap(app="Finder", category="menu", tier=0, label="A", path="p"),
            d.Gap(app="Finder", category="menu", tier=0, label="B", path="p"),
            d.Gap(app="Notes", category="menu", tier=0, label="C", path="p"),
        ]
        d.assign_ids(gaps)
        self.assertEqual(gaps[0].gap_id, "FIL-MENU-001")
        self.assertEqual(gaps[1].gap_id, "FIL-MENU-002")
        self.assertEqual(gaps[2].gap_id, "NOT-MENU-001")


class SynthesizedStandardMenusTests(unittest.TestCase):
    """The menu bar synthesizes the Application/Window/Help menus for every
    app (`app_menu`/`window_menu`/`help_menu` in
    shell/bins/rmac-menubar/src/{main,menu_model}.rs); `lulo_inventory.py`
    must mirror that exactly, or the diff reports standard items as missing
    even though they are on screen (see docs/inventory-gaps.md's history:
    Calculator, Clock and Weather each showed 76 such false gaps)."""

    def test_take_menu_removes_and_returns_items(self):
        menus = [rmp.Menu("File", [li._item("New", "x::New")]), rmp.Menu("Window", [])]
        items = li._take_menu(menus, "Window")
        self.assertEqual(items, [])
        self.assertEqual([m.label for m in menus], ["File"])

    def test_take_menu_absent_returns_empty_list_and_keeps_menus(self):
        menus = [rmp.Menu("File", [li._item("New", "x::New")])]
        self.assertEqual(li._take_menu(menus, "Window"), [])
        self.assertEqual(len(menus), 1)

    def test_app_menu_has_the_standard_rows_in_order(self):
        menu = li._synthesize_app_menu("Clock", [])
        labels = [item.label for item in menu.items]
        self.assertEqual(
            labels,
            ["About Clock", "Services", "Hide Clock", "Hide Others", "Show All", "Quit Clock"],
        )
        by_label = {item.label: item for item in menu.items}
        self.assertEqual(by_label["Hide Clock"].shortcut, "⌘H")
        self.assertEqual(by_label["Hide Others"].shortcut, "⌥⌘H")
        self.assertEqual(by_label["Quit Clock"].shortcut, "⌘Q")

    def test_app_menu_keeps_the_apps_own_items_after_about(self):
        settings = li._item("Settings…", "weather::ShowSettings", "⌘,")
        menu = li._synthesize_app_menu("Weather", [settings])
        labels = [item.label for item in menu.items]
        self.assertEqual(labels[0], "About Weather")
        self.assertEqual(labels[1], "Settings…")
        self.assertTrue(menu.items[1].separator_before)

    def test_quit_and_keep_windows_follows_quit(self):
        keep = li._item("Quit and Keep Windows", "preview::QuitAndKeepWindows", "⌥⌘Q")
        menu = li._synthesize_app_menu("Preview", [keep])
        self.assertEqual(
            [item.label for item in menu.items[-2:]],
            ["Quit Preview", "Quit and Keep Windows"],
        )
        self.assertEqual(menu.items[-1].shortcut, "⌥⌘Q")

    def test_finder_app_menu_has_no_quit_row(self):
        menu = li._synthesize_app_menu("Finder", [])
        labels = [item.label for item in menu.items]
        self.assertNotIn("Quit Finder", labels)

    def test_window_menu_has_minimise_all(self):
        menu = li._synthesize_window_menu([])
        by_label = {item.label: item for item in menu.items}
        self.assertEqual(by_label["Minimise All"].shortcut, "⌥⌘M")
        self.assertEqual(by_label["Minimise"].shortcut, "⌘M")
        self.assertIn("Bring All to Front", by_label)
        move_and_resize = by_label["Move & Resize"]
        self.assertEqual(
            [child.label for child in move_and_resize.children],
            [
                "Halves", "Left", "Right", "Top", "Bottom", "Quarters",
                "Top Left", "Top Right", "Bottom Left", "Bottom Right",
                "Return to Previous Size",
            ],
        )

    def test_window_menu_keeps_the_apps_own_window_items(self):
        tab = li._item("Show Next Tab", "finder::NextTab", "⌃⇥")
        menu = li._synthesize_window_menu([tab])
        labels = [item.label for item in menu.items]
        self.assertIn("Show Next Tab", labels)
        self.assertLess(labels.index("Show Next Tab"), labels.index("Bring All to Front"))

    def test_help_menu_has_app_help_with_its_shortcut(self):
        menu = li._synthesize_help_menu("Calculator", [])
        self.assertEqual(menu.items[0].label, "Calculator Help")
        self.assertEqual(menu.items[0].shortcut, "⌘?")

    def test_help_menu_keeps_the_apps_own_help_items(self):
        extra = li._item("File Format Help", "")
        menu = li._synthesize_help_menu("Preview", [extra])
        self.assertEqual([item.label for item in menu.items], ["Preview Help", "File Format Help"])

    def test_read_menu_bar_assembles_application_window_and_help(self):
        menus = li.read_menu_bar("Clock", "CLOCK_MENUS")
        labels = [menu["label"] for menu in menus]
        self.assertEqual(labels[0], "Application")
        self.assertEqual(labels[-2], "Window")
        self.assertEqual(labels[-1], "Help")
        application_items = {item["label"] for item in menus[0]["items"]}
        self.assertIn("About Clock", application_items)
        self.assertIn("Hide Clock", application_items)
        window_items = {item["label"] for item in menus[-2]["items"]}
        self.assertIn("Minimise All", window_items)
        help_items = {item["label"] for item in menus[-1]["items"]}
        self.assertIn("Clock Help", help_items)


class ParseMenuDumpTests(unittest.TestCase):
    """`mac_inventory.parse_menu_dump` turns `dumpMenuItems`'s tab-separated
    output (8 fields: depth, title, cmdChar, cmdVirtualKey, cmdModifiers,
    enabled, markChar, omitReason) into the nested tree `mac_inventory.py`
    writes to tests/inventory/mac/<app>.json."""

    def test_separator_has_none_label(self):
        tree = mi.parse_menu_dump("1\t\t\t\t\t1\t\t\n")
        self.assertEqual(len(tree), 1)
        self.assertIsNone(tree[0]["label"])

    def test_nested_children_recorded_when_not_omitted(self):
        raw = "1\tFile\t\t\t\t1\t\t\n" "2\tNew\tn\t\t\t1\t\t\n"
        tree = mi.parse_menu_dump(raw)
        self.assertEqual(tree[0]["label"], "File")
        self.assertNotIn("children_omitted", tree[0])
        self.assertEqual([c["label"] for c in tree[0]["children"]], ["New"])

    def test_shortcut_enabled_and_checked_parsed_from_all_8_fields(self):
        raw = "1\tCopy\tc\t\t8\t0\t✓\t\n"
        node = mi.parse_menu_dump(raw)[0]
        self.assertEqual(node["shortcut"], "c")  # mod 8 = no-command: no ⌘ prefix
        self.assertFalse(node["enabled"])
        self.assertTrue(node["checked"])

    def test_omit_reason_field_marks_children_omitted_and_stops_recursion(self):
        # dumpMenuItems decided this submenu was too wide to walk and so
        # emitted no deeper-depth lines for it; depth-1 is a false flag
        # here only to prove nothing deeper got attributed to it.
        raw = "1\tNew Window\t\t\t\t1\t\tdynamic (42 items), not read\n"
        tree = mi.parse_menu_dump(raw)
        self.assertEqual(tree[0]["children_omitted"], "dynamic (42 items), not read")
        self.assertEqual(tree[0]["children"], [])

    def test_named_dynamic_submenu_omitted_via_its_own_omit_reason(self):
        raw = "1\tServices\t\t\t\t1\t\tdynamic/personal submenu, not read\n"
        tree = mi.parse_menu_dump(raw)
        self.assertEqual(tree[0]["children_omitted"], "dynamic/personal submenu, not read")

    def test_legacy_7_field_raw_without_omit_reason_falls_back_to_named_list(self):
        # A raw dump shaped like the one dumpMenuItems produced before it
        # grew the omitReason column: Python-side DYNAMIC_PERSONAL_SUBMENUS
        # still catches the known named submenus as a defence-in-depth.
        raw = "1\tServices\t\t\t\t1\t\n"
        tree = mi.parse_menu_dump(raw)
        self.assertEqual(tree[0]["children_omitted"], "dynamic/personal submenu, not read")

    def test_unrelated_submenu_title_is_not_treated_as_omitted(self):
        raw = "1\tFile\t\t\t\t1\t\t\n" "2\tNew\tn\t\t\t1\t\t\n"
        tree = mi.parse_menu_dump(raw)
        self.assertNotIn("children_omitted", tree[0])


class DumpMenuBarAssemblyTests(unittest.TestCase):
    """`mac_inventory.dump_menu_bar` reads one top-level menu per osascript
    call (its own timeout) instead of one giant call for the whole bar, so
    Terminal's Shell menu or TextEdit's Format menu timing out does not
    lose the rest of the bar (the original 90s-all-or-nothing bug this
    fixes — see docs/inventory-gaps.md's history). These tests never touch
    osascript or a real app; `mi.run_osascript` is replaced with a fake
    that inspects the script text to decide which call it is answering."""

    @staticmethod
    def _menu_index(script: str) -> int:
        match = re.search(r"menu bar item (\d+) of menu bar 1", script)
        assert match, script
        return int(match.group(1))

    def test_reads_each_top_level_menu_in_its_own_osascript_call(self):
        calls: list[str] = []

        def fake_run_osascript(script, timeout=30):
            calls.append(script)
            if "on dumpMenuItems" not in script:
                return "File\nEdit\n"
            index = self._menu_index(script)
            return f"1\tItem{index}\t\t\t\t1\t\t\n"

        with mock.patch.object(mi, "run_osascript", side_effect=fake_run_osascript):
            menus, errors = mi.dump_menu_bar("TextEdit")

        self.assertEqual(errors, {})
        self.assertEqual([m["label"] for m in menus], ["File", "Edit"])
        self.assertEqual(menus[0]["items"][0]["label"], "Item1")
        self.assertEqual(menus[1]["items"][0]["label"], "Item2")
        # One call for the top-level titles, one more per top-level menu —
        # never one call that walks the whole bar.
        self.assertEqual(len(calls), 3)

    def test_one_failing_menu_does_not_lose_the_others(self):
        def fake_run_osascript(script, timeout=30):
            if "on dumpMenuItems" not in script:
                return "File\nWindow\nHelp\n"
            index = self._menu_index(script)
            if index == 2:  # Window: e.g. a dynamic/slow open-window list
                raise RuntimeError("osascript timed out after 30s")
            return f"1\tItem{index}\t\t\t\t1\t\t\n"

        with mock.patch.object(mi, "run_osascript", side_effect=fake_run_osascript):
            menus, errors = mi.dump_menu_bar("Terminal")

        self.assertEqual([m["label"] for m in menus], ["File", "Help"])
        self.assertEqual(errors, {"Window": "osascript timed out after 30s"})

    def test_overall_budget_exhausted_skips_remaining_menus_without_calling_them(self):
        calls: list[str] = []
        clock = {"t": 0.0}

        def fake_monotonic():
            return clock["t"]

        def fake_run_osascript(script, timeout=30):
            calls.append(script)
            if "on dumpMenuItems" not in script:
                return "File\nEdit\n"
            clock["t"] += 1000.0  # blow the overall budget after this menu
            return "1\tNew\t\t\t\t1\t\t\n"

        with mock.patch.object(mi, "run_osascript", side_effect=fake_run_osascript), mock.patch.object(
            time, "monotonic", side_effect=fake_monotonic
        ):
            menus, errors = mi.dump_menu_bar("Terminal", overall_budget=90.0)

        self.assertEqual([m["label"] for m in menus], ["File"])
        self.assertIn("overall menu-bar budget", errors.get("Edit", ""))
        # The budget-exhausted menu's own osascript call never happens:
        # just the titles call plus File's.
        self.assertEqual(len(calls), 2)

    def test_every_menu_failing_returns_no_menus_and_all_the_errors(self):
        def fake_run_osascript(script, timeout=30):
            if "on dumpMenuItems" not in script:
                return "File\n"
            raise RuntimeError("osascript timed out after 30s")

        with mock.patch.object(mi, "run_osascript", side_effect=fake_run_osascript):
            menus, errors = mi.dump_menu_bar("Terminal")

        self.assertEqual(menus, [])
        self.assertEqual(errors, {"File": "osascript timed out after 30s"})


class InventoryAppMenuBarMergeTests(unittest.TestCase):
    """`inventory_app` must keep a partially-successful menu-bar read
    (some top-level menus captured, some not) rather than discarding
    everything the way a single `menu_bar_error` used to, and must still
    fall back to `menu_bar_error` when nothing at all was captured."""

    def _run(self, dump_menu_bar_result):
        with mock.patch.object(mi, "is_running", return_value=True), mock.patch.object(
            mi, "dump_menu_bar", return_value=dump_menu_bar_result
        ), mock.patch.object(
            mi, "dump_toolbar", return_value={"present": False, "items": []}
        ), mock.patch.object(
            mi, "dump_settings_window", return_value={"present": False, "controls": []}
        ):
            return mi.inventory_app("Terminal", mi.APPS["Terminal"])

    def test_partial_menu_errors_are_kept_alongside_the_captured_menus(self):
        data = self._run(
            ([{"label": "File", "items": []}], {"Window": "osascript timed out after 30s"})
        )
        self.assertEqual(data["menu_bar"], [{"label": "File", "items": []}])
        self.assertEqual(
            data["menu_bar_partial_errors"], {"Window": "osascript timed out after 30s"}
        )
        self.assertNotIn("menu_bar_error", data)

    def test_every_menu_failing_falls_back_to_menu_bar_error(self):
        data = self._run(([], {"File": "osascript timed out after 30s"}))
        self.assertNotIn("menu_bar", data)
        self.assertNotIn("menu_bar_partial_errors", data)
        self.assertIn("osascript timed out after 30s", data["menu_bar_error"])

    def test_no_partial_errors_omits_the_partial_errors_key(self):
        data = self._run(([{"label": "File", "items": []}], {}))
        self.assertEqual(data["menu_bar"], [{"label": "File", "items": []}])
        self.assertNotIn("menu_bar_partial_errors", data)
        self.assertNotIn("menu_bar_error", data)


class DiffExcludesPartiallyFailedMenusTests(unittest.TestCase):
    """`diff.py` must not turn a top-level menu `mac_inventory.py` could
    not capture this run into a pile of false "Lulo-only" gaps for every
    item under it — it should exclude that menu from both sides and leave
    a coverage note instead (mirroring the existing contaminated-
    background-capture handling for Finder's context menus)."""

    def test_failed_top_level_menu_is_excluded_from_both_sides(self):
        mac = [{"label": "File", "items": [{"label": "New", "shortcut": "⌘N", "children": []}]}]
        lulo = [
            {"label": "File", "items": [{"label": "New", "shortcut": "⌘N", "children": []}]},
            {"label": "Window", "items": [{"label": "Minimise", "shortcut": "⌘M", "children": []}]},
        ]
        gaps = d.diff_menu_bars("Terminal", mac, lulo, "Terminal")
        # Without the exclusion, Lulo's whole Window menu would show up as
        # TIER_EXTRA_LULO_ONLY simply because the Mac side has no entry for
        # a menu it never got to capture this run.
        self.assertTrue(any(g.path.startswith("Window") for g in gaps))

        filtered_lulo = [m for m in lulo if m.get("label") != "Window"]
        gaps_excluded = d.diff_menu_bars("Terminal", mac, filtered_lulo, "Terminal")
        self.assertEqual(gaps_excluded, [])

    def test_diff_app_performs_the_exclusion_itself_from_menu_bar_partial_errors(self):
        mac_data = {
            "app": "Terminal",
            "menu_bar": [
                {
                    "label": "File",
                    "items": [{"label": "New Window", "shortcut": "⌘N", "children": []}],
                },
            ],
            "menu_bar_partial_errors": {"Window": "osascript timed out after 30s"},
            "toolbar": {"present": False, "items": []},
            "settings": {"present": False, "controls": []},
        }
        lulo_data = {
            "menu_bar": [
                {
                    "label": "File",
                    "items": [{"label": "New Window", "shortcut": "⌘N", "children": []}],
                },
                {
                    "label": "Window",
                    "items": [{"label": "Minimise", "shortcut": "⌘M", "children": []}],
                },
            ],
            "toolbar": {"present": False, "items": []},
            "settings": {"present": False, "controls": []},
        }
        with tempfile.TemporaryDirectory() as tmp:
            mac_dir = Path(tmp) / "mac"
            lulo_dir = Path(tmp) / "lulo"
            mac_dir.mkdir()
            lulo_dir.mkdir()
            (mac_dir / "Terminal.json").write_text(json.dumps(mac_data))
            (lulo_dir / "Terminal.json").write_text(json.dumps(lulo_data))
            with mock.patch.object(d, "MAC_DIR", mac_dir), mock.patch.object(d, "LULO_DIR", lulo_dir):
                gaps, notes = d.diff_app("Terminal", "Terminal")

        # Lulo's whole Window menu must not show up as TIER_EXTRA_LULO_ONLY
        # just because the Mac side could not capture it this run.
        self.assertFalse(any(g.path.startswith("Window") for g in gaps))
        self.assertTrue(
            any("Window" in note and "excluded from the menu diff" in note for note in notes)
        )


if __name__ == "__main__":
    unittest.main()
