"""Pure-logic unit tests for scripts/linux/sweep-settings-errors.py.

These run with plain `python3 -m pytest scripts/test_sweep_settings_errors.py`
on macOS (no pyatspi, no live session required): they cover the text/role
classifier, the "only a list-item row is safe to click" rule, and the
PANE_ROUTES-stays-in-sync-with-navigation.rs check only. The live AT-SPI
sweep itself can only be exercised on the reference Linux laptop.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "linux" / "sweep-settings-errors.py"
NAVIGATION_RS = ROOT / "crates" / "system-settings" / "src" / "navigation.rs"

SPEC = importlib.util.spec_from_file_location("sweep_settings_errors", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
sweep = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = sweep
SPEC.loader.exec_module(sweep)


class ModuleImportTests(unittest.TestCase):
    def test_imports_without_pyatspi(self):
        # macOS has no pyatspi (and no atspi_assert_support that imports
        # it); the module must still load so its pure logic is testable.
        self.assertTrue(hasattr(sweep, "pyatspi"))
        self.assertTrue(hasattr(sweep, "support"))


class ClassifyTests(unittest.TestCase):
    def test_each_documented_pattern_is_recognized(self):
        for pattern in sweep.ERROR_PATTERNS:
            with self.subTest(pattern=pattern):
                text = f"Something {pattern} here"
                reasons = sweep.classify(text)
                self.assertIn(f"text:{pattern}", reasons)

    def test_matching_is_case_insensitive(self):
        self.assertIn("text:unavailable", sweep.classify("Bluetooth is UNAVAILABLE right now"))

    def test_ordinary_text_has_no_reasons(self):
        self.assertEqual(sweep.classify("Connect to Wi-Fi networks and manage known networks."), [])

    def test_empty_text_has_no_reasons(self):
        self.assertEqual(sweep.classify(""), [])
        self.assertEqual(sweep.classify(None), [])

    def test_a_node_can_match_on_text_and_role_at_once(self):
        reasons = sweep.classify("The device could not be reached.", role="alert")
        self.assertIn("text:could not", reasons)
        self.assertIn("role:alert", reasons)

    def test_role_alert_is_flagged_even_with_reassuring_text(self):
        # Toast/EmptyState-error both set Role::Alert regardless of wording
        # (crates/rmac-ui/src/feedback.rs); a screen reader announces the
        # node because of the role, so the sweep must record it too.
        reasons = sweep.classify("Everything is fine.", role="alert")
        self.assertEqual(reasons, ["role:alert"])

    def test_banner_and_notification_roles_are_flagged(self):
        self.assertIn("role:banner", sweep.classify("", role="banner"))
        self.assertIn("role:notification", sweep.classify("", role="notification"))

    def test_unrelated_role_with_plain_text_is_not_flagged(self):
        self.assertEqual(sweep.classify("Wi-Fi", role="list item"), [])

    def test_substring_match_does_not_require_word_boundaries(self):
        # Documented, deliberate behaviour: the task's own pattern list is a
        # plain substring scan, so a compound word like "unsupported" still
        # matches "unsupported" even though it also contains "supported".
        self.assertIn("text:unsupported", sweep.classify("This resolution is unsupported."))


class RowSafetyTests(unittest.TestCase):
    def test_list_item_with_a_name_is_safe(self):
        self.assertTrue(sweep.is_safe_subpage_row("list item", "Storage"))

    def test_sidebar_category_rows_are_excluded(self):
        for _pane_id, category_name in sweep.PANE_ROUTES:
            with self.subTest(category=category_name):
                self.assertFalse(sweep.is_safe_subpage_row("list item", category_name))

    def test_unnamed_rows_are_excluded(self):
        self.assertFalse(sweep.is_safe_subpage_row("list item", ""))

    def test_mutating_roles_are_never_safe_regardless_of_name(self):
        # The whole point of the rule: a toggle switch or a destructive
        # button never becomes a click target just because its name looks
        # like a plain row.
        for role in ("push button", "toggle button", "check box", "radio button", "combo box"):
            with self.subTest(role=role):
                self.assertFalse(sweep.is_safe_subpage_row(role, "Forget This Network…"))


class RaceDetectionTimingTests(unittest.TestCase):
    def test_immediate_sweep_runs_before_the_pane_has_had_time_to_settle(self):
        # sweep_immediate_and_settled's whole point (catching a
        # placeholder/error shown before a watcher's first tick lands) only
        # holds if the "immediate" sweep really does fire well before
        # wait_settle's own poll would have already stabilized.
        self.assertGreater(sweep.SETTLE_TIMEOUT_S, sweep.IMMEDIATE_SWEEP_DELAY_S)

    def test_immediate_sweep_delay_is_positive_but_short(self):
        self.assertGreater(sweep.IMMEDIATE_SWEEP_DELAY_S, 0.0)
        self.assertLess(sweep.IMMEDIATE_SWEEP_DELAY_S, 1.0)


class FakeNode:
    def __init__(self, role, name, actions=()):
        self.role = role
        self.name = name
        self.actions = actions


class FakeSupport:
    """A minimal stand-in for atspi_assert_support, so
    clickable_named_row/enter_named_row's own logic (not pyatspi) is
    testable on any platform."""

    def __init__(self, nodes):
        self.nodes = nodes
        self.clicked = []

    def nodes_with(self, _app, role, name):
        return [node for node in self.nodes if node.role == role and node.name == name]

    def actions(self, node):
        return node.actions

    def click(self, node):
        self.clicked.append(node)

    def wait_for(self, predicate, description, timeout):
        # No live session to poll here: either the predicate is already
        # true (the common case these tests cover) or it times out at once.
        value = predicate()
        if value:
            return value
        raise AssertionError(f"timed out waiting for {description}")


class EnterNamedRowTests(unittest.TestCase):
    def setUp(self):
        self.real_support = sweep.support

    def tearDown(self):
        sweep.support = self.real_support

    def test_clicks_the_row_with_a_click_action(self):
        clickable = FakeNode("list item", "Storage", actions=["click"])
        fake = FakeSupport([clickable])
        sweep.support = fake
        self.assertTrue(sweep.enter_named_row(object(), "Storage"))
        self.assertEqual(fake.clicked, [clickable])

    def test_ignores_a_same_named_row_with_no_click_action(self):
        # A toggle or disabled row that happens to share a name must never
        # be treated as a safe navigation target.
        inert = FakeNode("list item", "Storage", actions=[])
        fake = FakeSupport([inert])
        sweep.support = fake
        self.assertFalse(sweep.enter_named_row(object(), "Storage"))
        self.assertEqual(fake.clicked, [])

    def test_returns_false_honestly_when_the_row_never_appears(self):
        fake = FakeSupport([])
        sweep.support = fake
        self.assertFalse(sweep.enter_named_row(object(), "Nonexistent Mode"))


class NavigationRsSyncTests(unittest.TestCase):
    def test_pane_routes_matches_navigation_rs(self):
        source = NAVIGATION_RS.read_text(encoding="utf-8")
        rust_routes = sweep.navigation_rs_pane_routes(source)
        self.assertEqual(
            rust_routes,
            sweep.PANE_ROUTES,
            "scripts/linux/sweep-settings-errors.py's PANE_ROUTES has drifted "
            "from crates/system-settings/src/navigation.rs -- update the copy",
        )

    def test_parser_rejects_text_with_no_pane_routes(self):
        with self.assertRaises(sweep.SweepError):
            sweep.navigation_rs_pane_routes("pub(super) const SOMETHING_ELSE: [&str; 0] = [];")


if __name__ == "__main__":
    unittest.main()
