#!/usr/bin/env python3
"""Unit tests for scripts/inventory/{normalize,diff,rust_menu_parser}.py.

Runs with plain unittest / pytest; no Mac, no Lulo binaries, no GUI.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "inventory"))

import diff as d  # noqa: E402
import lulo_inventory as li  # noqa: E402
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

    def test_none_passes_through(self):
        self.assertIsNone(norm.normalize_label(None))


class NormalizeShortcutTests(unittest.TestCase):
    def test_hyphen_and_minus_sign_are_equal(self):
        self.assertEqual(norm.normalize_shortcut("⌘-"), norm.normalize_shortcut("⌘−"))

    def test_empty_shortcut_is_empty_string(self):
        self.assertEqual(norm.normalize_shortcut(""), "")
        self.assertEqual(norm.normalize_shortcut(None), "")


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

    def test_unnamed_ax_scaffolding_is_not_a_settings_control(self):
        mac = {"present": True, "controls": [
            {"role": "AXTable", "label": "table"},
            {"role": "AXButton", "label": "close button"},
            {"role": "AXCheckBox", "label": "Display ANSI colours"},
        ]}
        lulo = {"present": True, "controls": []}
        gaps = d.diff_settings("Terminal", mac, lulo)
        self.assertEqual([gap.label for gap in gaps], ["Display ANSI colours"])

    def test_terminal_menu_timeout_does_not_make_all_lulo_rows_extra(self):
        gaps, notes = d.diff_app("Terminal", "Terminal")
        self.assertFalse(any(gap.category == "menu" for gap in gaps))
        self.assertTrue(any("menu diff skipped" in note for note in notes))

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
            ["Left", "Right", "Top", "Bottom", "Return to Previous Size"],
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


if __name__ == "__main__":
    unittest.main()
