"""Unit tests for the automated Orca audit's pure checks (scripts/a11y)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "a11y"))

import orca_checks as checks  # noqa: E402


def node(role: str, name: str = "", states=(), key: str = "k", **extra):
    return {"key": key, "role": role, "name": name, "states": list(states), "value": None, "text": None,
            "labelled_by": "", "content": "", "description": "", **extra}


def kinds(flags):
    return sorted(item["kind"] for item in flags)


class FocusFlagTests(unittest.TestCase):
    def test_nothing_focused_is_focus_lost(self):
        self.assertEqual(kinds(checks.focus_flags(None)), ["focus-lost"])

    def test_unnamed_button(self):
        self.assertEqual(kinds(checks.focus_flags(node("button", states=["focusable"]))), ["unnamed"])

    def test_named_button_is_clean(self):
        self.assertEqual(checks.focus_flags(node("button", "Back", ["focusable"])), [])

    def test_labelled_by_counts_as_a_name(self):
        self.assertEqual(checks.focus_flags(node("entry", labelled_by="Search")), [])

    def test_list_item_named_by_its_content(self):
        item = node("list item", states=["selectable"], content="Budget.txt")
        self.assertEqual(checks.focus_flags(item), [])

    def test_generic_container_focus(self):
        self.assertEqual(kinds(checks.focus_flags(node("panel"))), ["generic-role"])

    def test_checkbox_without_checkable_state(self):
        flags = checks.focus_flags(node("check box", "Wrap", ["focusable"]))
        self.assertEqual(kinds(flags), ["missing-state"])
        self.assertEqual(checks.focus_flags(node("check box", "Wrap", ["checkable"])), [])

    def test_combo_box_needs_expandable(self):
        self.assertEqual(kinds(checks.focus_flags(node("combo box", "Sort"))), ["missing-state"])

    def test_list_item_needs_selectable(self):
        self.assertEqual(kinds(checks.focus_flags(node("list item", "A"))), ["missing-state"])


class StepFlagTests(unittest.TestCase):
    def test_silent_focus_move(self):
        flags = checks.step_flags(node("button", "A", key="a"), node("button", "B", key="b"), [])
        self.assertEqual(kinds(flags), ["silent-focus"])

    def test_spoken_focus_move(self):
        flags = checks.step_flags(node("button", "A", key="a"), node("button", "Back", key="b"),
                                  ["Back push button"])
        self.assertEqual(flags, [])

    def test_speech_without_the_name(self):
        flags = checks.step_flags(node("button", "A", key="a"), node("button", "Share", key="b"),
                                  ["button"])
        self.assertEqual(kinds(flags), ["speech-mismatch"])

    def test_toggle_without_effect(self):
        box = node("check box", "Wrap", ["checkable"])
        self.assertEqual(kinds(checks.step_flags(box, box, [], "toggle")), ["no-effect"])

    def test_silent_toggle(self):
        before = node("check box", "Wrap", ["checkable"])
        after = node("check box", "Wrap", ["checkable", "checked"])
        self.assertEqual(kinds(checks.step_flags(before, after, [], "toggle")), ["silent-change"])
        self.assertEqual(checks.step_flags(before, after, ["checked"], "toggle"), [])

    def test_silent_value_change(self):
        before = node("slider", "Volume", value=10.0)
        after = node("slider", "Volume", value=15.0)
        self.assertEqual(kinds(checks.step_flags(before, after, [], "value")), ["silent-change"])

    def test_tab_that_does_not_move(self):
        here = node("button", "A")
        self.assertEqual(kinds(checks.step_flags(here, here, [], "move")), ["no-effect"])


class CycleTests(unittest.TestCase):
    def test_stuck_in_multiline_text_is_expected(self):
        editor = node("text", "Document", ["multi line", "editable"])
        self.assertEqual(checks.cycle_flags([editor], "stuck"), [])

    def test_stuck_on_a_button_is_flagged(self):
        self.assertEqual(kinds(checks.cycle_flags([node("button", "A")], "stuck")), ["tab-stuck"])

    def test_subcycle_is_a_trap(self):
        self.assertEqual(kinds(checks.cycle_flags([node("button", "A")], "subcycle")), ["tab-trap"])

    def test_reverse_order(self):
        stops = [node("button", n, key=n) for n in "abc"]
        good = [stops[2], stops[1]]
        self.assertEqual(checks.reverse_flags(stops, good), [])
        bad = [stops[1]]
        self.assertEqual(kinds(checks.reverse_flags(stops, bad)), ["reverse-order"])

    def test_reachability(self):
        controls = [
            node("button", "Visited", ["focusable", "sensitive"], key="v", ancestors=["frame"]),
            node("button", "Pointer", ["sensitive"], key="p", ancestors=["frame"]),
            node("button", "Skipped", ["focusable", "sensitive"], key="s", ancestors=["frame"]),
            node("button", "Row action", ["sensitive"], key="r", ancestors=["frame", "list"]),
            node("button", "Disabled", [], key="d", ancestors=["frame"]),
        ]
        flags = checks.reachability_flags(controls, {"v"})
        self.assertEqual(kinds(flags), ["pointer-only", "unreachable"])


class ItemStopTests(unittest.TestCase):
    def test_each_item_a_tab_stop(self):
        items = [node("list item", n, key=n, parent="sidebar") for n in ("Wi-Fi", "Bluetooth", "Network")]
        self.assertEqual(kinds(checks.item_stop_flags(items)), ["tab-per-item"])

    def test_two_items_or_different_lists_are_fine(self):
        items = [node("list item", "A", key="a", parent="one"), node("list item", "B", key="b", parent="one"),
                 node("list item", "C", key="c", parent="two")]
        self.assertEqual(checks.item_stop_flags(items), [])

    def test_window_focus_is_nowhere(self):
        self.assertEqual(kinds(checks.focus_flags(node("frame", "Notes"))), ["focus-nowhere"])

    def test_selection_move_with_speech_is_fine(self):
        listbox = node("list box", "Files")
        self.assertEqual(checks.step_flags(listbox, listbox, ["Budget.txt"], "move"), [])
        self.assertEqual(kinds(checks.step_flags(listbox, listbox, [], "move")), ["no-effect"])


class DescribeTests(unittest.TestCase):
    def test_describe_reads_states(self):
        text = checks.describe(node("check box", "Wrap", ["checkable", "checked", "sensitive"]))
        self.assertEqual(text, "Wrap, check box, checked")
        self.assertEqual(checks.describe(None), "(nothing focused)")


if __name__ == "__main__":
    unittest.main()
