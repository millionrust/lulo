"""Unit tests for the interaction-probe matrix (scripts/interaction,
tests/interaction). They run anywhere: no compositor, no AT-SPI and no Mac
are needed - only the pure comparison rules in probes.py and diff.py.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "interaction"))

import interaction_diff as diff  # noqa: E402
import probes as pr  # noqa: E402
import surfaces as sf  # noqa: E402
import lulo_probe as lp  # noqa: E402


class ProbeRegistryTests(unittest.TestCase):
    def test_every_probe_declares_at_least_one_fact(self):
        for probe_id, spec in pr.PROBES.items():
            self.assertTrue(spec["facts"], probe_id)

    def test_every_automated_fact_has_a_tolerance_rule(self):
        for probe_id in pr.AUTOMATED:
            for fact in pr.PROBES[probe_id]["facts"]:
                self.assertIn(fact, pr.FACT_RULE, f"{probe_id}.{fact}")

    def test_probes_for_filters_by_applies_to(self):
        self.assertIn("outside_click", pr.probes_for("menu"))
        self.assertNotIn("scroll", pr.probes_for("menu"))
        self.assertIn("scroll", pr.probes_for("list"))

    def test_probes_for_orders_automated_before_planned(self):
        ids = pr.probes_for("list")
        statuses = [pr.PROBES[pid]["status"] for pid in ids]
        self.assertEqual(statuses, sorted(statuses, key=lambda s: s != "automated"))


class FieldMatchTests(unittest.TestCase):
    def test_exact_rule_on_booleans(self):
        self.assertTrue(pr.field_matches("exact", True, True))
        self.assertFalse(pr.field_matches("exact", True, False))

    def test_ignore_rule_always_matches(self):
        self.assertTrue(pr.field_matches("ignore", True, False))

    def test_unknown_rule_raises(self):
        with self.assertRaises(ValueError):
            pr.field_matches("fuzzy", True, False)


class CompareProbeResultTests(unittest.TestCase):
    def test_matching_facts_produce_no_mismatch(self):
        result = pr.compare_probe_result("outside_click", {"closed": True}, {"closed": True})
        self.assertEqual(result["mismatches"], [])
        self.assertEqual(result["inconclusive"], [])

    def test_the_two_owner_reported_gaps_are_caught(self):
        # Gap 1: top-bar menus/Control Centre did not close on an outside click.
        outside = pr.compare_probe_result("outside_click", {"closed": True}, {"closed": False})
        self.assertEqual(len(outside["mismatches"]), 1)
        self.assertEqual(outside["mismatches"][0], {"fact": "closed", "mac": True, "lulo": False, "rule": "exact"})
        # Gap 2: the brightness/volume sliders did not visibly react to hover.
        hover = pr.compare_probe_result("hover", {"changed": True}, {"changed": False})
        self.assertEqual(len(hover["mismatches"]), 1)
        self.assertEqual(hover["mismatches"][0]["fact"], "changed")

    def test_a_none_value_on_either_side_is_inconclusive_not_a_mismatch(self):
        result = pr.compare_probe_result("outside_click", {"closed": None}, {"closed": False})
        self.assertEqual(result["mismatches"], [])
        self.assertEqual(result["inconclusive"], ["closed"])

    def test_missing_dict_on_either_side_is_inconclusive(self):
        result = pr.compare_probe_result("outside_click", None, {"closed": False})
        self.assertEqual(result["mismatches"], [])
        self.assertEqual(result["inconclusive"], ["closed"])


class SurfaceMatrixTests(unittest.TestCase):
    def test_every_surface_id_is_unique(self):
        ids = [item["id"] for item in sf.SURFACES]
        self.assertEqual(len(ids), len(set(ids)))

    def test_every_automated_surface_names_both_platforms(self):
        for item in sf.SURFACES:
            if item["status"] == "automated":
                self.assertIn("mac", item, item["id"])
                self.assertIn("lulo", item, item["id"])

    def test_surface_lookup_by_id(self):
        self.assertEqual(sf.surface("control-centre")["kind"], "popover")
        with self.assertRaises(KeyError):
            sf.surface("does-not-exist")

    def test_matrix_lists_every_declared_pair(self):
        pairs = sf.matrix()
        self.assertIn(("control-centre", "outside_click"), pairs)
        self.assertIn(("control-centre", "hover:display-brightness-slider"), pairs)
        self.assertIn(("lulo-menu", "switch_neighbor"), pairs)


class DiffReportTests(unittest.TestCase):
    """diff.py's pure functions, fed fixture recordings directly - never the
    real tests/interaction directory, so this suite never depends on a run
    having happened on either platform."""

    def _item(self, **overrides):
        item = {"id": "x", "title": "X", "kind": "menu", "status": "automated"}
        item.update(overrides)
        return item

    AUTOMATED_MENU_FACTS = {
        "outside_click": {"closed": True}, "escape": {"closed": True},
        "reopen_same_title": {"closed": True}, "switch_neighbor": {"switched": True},
        "hover": {"changed": True},
    }

    def test_matching_recordings_produce_no_gap(self):
        mac = {"probes": self.AUTOMATED_MENU_FACTS}
        lulo = {"probes": self.AUTOMATED_MENU_FACTS}
        gaps, unprobed = diff.compare_surface(self._item(), mac, lulo)
        self.assertEqual(gaps, [])
        # "menu" also declares the still-planned "arrow_keys" probe.
        self.assertEqual([u["probe"] for u in unprobed], ["arrow_keys"])

    def test_outside_click_mismatch_is_reported_as_a_gap(self):
        mac = {"probes": self.AUTOMATED_MENU_FACTS}
        lulo = {"probes": {**self.AUTOMATED_MENU_FACTS, "outside_click": {"closed": False}}}
        gaps, unprobed = diff.compare_surface(self._item(), mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0]["probe"], "outside_click")
        self.assertEqual([u["probe"] for u in unprobed], ["arrow_keys"])

    def test_missing_lulo_recording_is_unprobed_not_a_silent_pass(self):
        mac = {"probes": self.AUTOMATED_MENU_FACTS}
        gaps, unprobed = diff.compare_surface(self._item(), mac, None)
        self.assertEqual(gaps, [])
        self.assertEqual({u["probe"] for u in unprobed},
                          {"outside_click", "escape", "reopen_same_title", "switch_neighbor", "hover", "arrow_keys"})

    def test_surface_level_error_marks_every_probe_unprobed(self):
        mac = {"error": "the app did not come to the front"}
        lulo = {"probes": {"outside_click": {"closed": False}}}
        gaps, unprobed = diff.compare_surface(self._item(), mac, lulo)
        self.assertEqual(gaps, [])
        self.assertTrue(unprobed)

    def test_planned_probe_is_unprobed_even_with_both_recordings_present(self):
        # "menu" includes the planned "arrow_keys" probe.
        mac = {"probes": {}}
        lulo = {"probes": {}}
        gaps, unprobed = diff.compare_surface(self._item(), mac, lulo)
        self.assertIn("arrow_keys", {u["probe"] for u in unprobed})

    def test_hover_control_probes_are_included_per_control(self):
        item = self._item(kind="popover", hover_controls=[{"label": "a"}, {"label": "b"}])
        ids = diff.probe_ids_for(item)
        self.assertIn("hover:a", ids)
        self.assertIn("hover:b", ids)

    def test_assign_ids_continues_after_the_highest_existing_number(self):
        existing = ("| id | surface | probe | fact | Mac | Lulo |\n|---|---|---|---|---|---|\n"
                     "| INT-001 | a | p1 | closed | yes | no |\n"
                     "| INT-007 | b | p2 | closed | yes | no |\n")
        gaps = [{"surface": "c", "probe": "p3", "fact": "closed"}, {"surface": "d", "probe": "p4", "fact": "closed"}]
        self.assertEqual(diff.assign_ids(gaps, existing), ["INT-008", "INT-009"])

    def test_assign_ids_starts_at_one_with_no_existing_rows(self):
        gaps = [{"surface": "a", "probe": "p1", "fact": "closed"}]
        self.assertEqual(diff.assign_ids(gaps, ""), ["INT-001"])

    def test_assign_ids_is_stable_across_reruns_with_no_new_gaps(self):
        # A rerun that finds the exact same gaps must not inflate INT-NNN
        # forever: the same (surface, probe, fact) keeps its existing id.
        existing = ("| id | surface | probe | fact | Mac | Lulo |\n|---|---|---|---|---|---|\n"
                     "| INT-001 | a | p1 | closed | yes | no |\n"
                     "| INT-002 | b | p2 | closed | yes | no |\n")
        gaps = [{"surface": "a", "probe": "p1", "fact": "closed"}, {"surface": "b", "probe": "p2", "fact": "closed"}]
        self.assertEqual(diff.assign_ids(gaps, existing), ["INT-001", "INT-002"])

    def test_assign_ids_mixes_reused_and_new_ids(self):
        existing = ("| id | surface | probe | fact | Mac | Lulo |\n|---|---|---|---|---|---|\n"
                     "| INT-001 | a | p1 | closed | yes | no |\n")
        gaps = [{"surface": "a", "probe": "p1", "fact": "closed"}, {"surface": "b", "probe": "p2", "fact": "closed"}]
        self.assertEqual(diff.assign_ids(gaps, existing), ["INT-001", "INT-002"])

    def test_render_markdown_lists_gaps_and_unprobed_separately(self):
        gaps = [{"surface": "control-centre", "title": "Control Centre", "probe": "outside_click",
                 "fact": "closed", "mac": True, "lulo": False, "rule": "exact"}]
        unprobed = [{"surface": "dock", "probe": "hover", "reason": "planned, no driver yet"}]
        text = diff.render_markdown(gaps, unprobed)
        self.assertIn("control-centre", text)
        self.assertIn("outside_click", text)
        self.assertIn("Not yet probed", text)
        self.assertIn("dock", text)

    def test_render_markdown_with_no_gaps_still_mentions_the_matrix(self):
        text = diff.render_markdown([], [])
        self.assertIn("No gaps found", text)


class LuloOnlySurfaceTests(unittest.TestCase):
    """The newly-widened surfaces (dock, spotlight, Notification Centre,
    the Files context menu, the Text Editor alert, the Settings sidebar):
    a Lulo driver exists, but this agent never drives the owner's live Mac,
    so each one declares "lulo-only", not "automated"."""

    LULO_ONLY_IDS = {
        "dock", "spotlight", "clock-notification-centre",
        "files-window-context-menu", "text-editor-save-sheet", "settings-sidebar-list",
    }

    def test_every_expected_surface_is_lulo_only(self):
        by_id = {item["id"]: item for item in sf.SURFACES}
        for sid in self.LULO_ONLY_IDS:
            self.assertEqual(by_id[sid]["status"], "lulo-only", sid)

    def test_every_lulo_only_surface_still_names_both_platforms(self):
        # Ground truth for a future Mac driver stays declared even though
        # nobody has written or run that driver yet.
        for item in sf.SURFACES:
            if item["status"] == "lulo-only":
                self.assertIn("mac", item, item["id"])
                self.assertIn("lulo", item, item["id"])

    def test_spotlight_dispatches_the_real_launcher_shortcut_id(self):
        # Regression: this surface used to name a shortcut id ("spotlight")
        # that crates/rmac-shortcuts/src/model.rs does not know, which
        # rmac-shortcut-dispatch would reject outright.
        self.assertEqual(sf.surface("spotlight")["lulo"]["shortcut"], "launcher")

    def test_matrix_includes_the_widened_probes(self):
        pairs = set(sf.matrix())
        self.assertIn(("dock", "hover"), pairs)
        self.assertIn(("dock", "right_click"), pairs)
        self.assertIn(("spotlight", "outside_click"), pairs)
        self.assertIn(("spotlight", "escape"), pairs)
        self.assertIn(("clock-notification-centre", "outside_click"), pairs)
        self.assertIn(("files-window-context-menu", "outside_click"), pairs)
        self.assertIn(("text-editor-save-sheet", "tab_focus"), pairs)
        self.assertIn(("settings-sidebar-list", "scroll"), pairs)


class LuloOnlyDiffTests(unittest.TestCase):
    """A "lulo-only" surface must be compared exactly like an "automated"
    one once a Lulo recording exists - never lumped into the generic
    "surface has no driver yet" bucket "planned" surfaces get."""

    def _item(self, **overrides):
        item = {"id": "x", "title": "X", "kind": "dock", "status": "lulo-only"}
        item.update(overrides)
        return item

    def test_a_lulo_recording_with_no_mac_recording_is_pending_not_skipped(self):
        lulo = {"probes": {"hover": {"changed": True}, "right_click": {"opened": True}}}
        gaps, unprobed = diff.compare_surface(self._item(), None, lulo)
        self.assertEqual(gaps, [])
        reasons = {u["probe"]: u["reason"] for u in unprobed}
        self.assertEqual(reasons["hover"], "no recording")
        self.assertEqual(reasons["right_click"], "no recording")

    def test_once_both_sides_are_recorded_lulo_only_compares_like_automated(self):
        mac = {"probes": {"hover": {"changed": True}, "right_click": {"opened": True}}}
        lulo = {"probes": {"hover": {"changed": False}, "right_click": {"opened": True}}}
        gaps, unprobed = diff.compare_surface(self._item(), mac, lulo)
        self.assertEqual(len(gaps), 1)
        self.assertEqual(gaps[0]["probe"], "hover")
        # "dock" also declares the still-planned "press_hold" probe.
        self.assertEqual([u["probe"] for u in unprobed], ["press_hold"])

    def test_evaluate_does_not_fall_back_to_the_generic_planned_note(self):
        # A "planned" surface's probes are bucketed under its own note; a
        # "lulo-only" surface's probes go through compare_surface instead,
        # so a missing mac recording must read as "no recording", not the
        # surface-level note string.
        item = self._item(note="should never be shown for a lulo-only surface")
        originals = sf.SURFACES
        sf.SURFACES = [item]
        try:
            gaps, unprobed = diff.evaluate()
        finally:
            sf.SURFACES = originals
        self.assertEqual(gaps, [])
        self.assertTrue(unprobed)
        for row in unprobed:
            self.assertNotEqual(row["reason"], "should never be shown for a lulo-only surface")


class PopoverRegistryTests(unittest.TestCase):
    """run_popover_surface (scripts/interaction/lulo_probe.py) dispatches a
    shell-harness popover by surface id, to either a surface-specific
    runner (Control Centre) or the generic shortcut-driven one (Spotlight,
    Notification Centre). A surface declared in surfaces.py with no entry
    in either table would raise KeyError the first time anyone recorded
    it - this is checked here, with no AT-SPI bus or compositor needed."""

    def test_every_shell_popover_surface_is_registered(self):
        for item in sf.SURFACES:
            if item["kind"] != "popover" or item.get("lulo", {}).get("harness") != "shell":
                continue
            if item["id"] in lp.POPOVER_RUNNERS:
                continue
            self.assertIn(item["id"], lp.POPOVER_STARTERS, item["id"])

    def test_dispatcher_prefers_the_surface_specific_runner(self):
        calls = []
        lp.POPOVER_RUNNERS["x-test"] = lambda shell, item: calls.append(("specific", shell, item["id"]))
        try:
            lp.run_popover_surface("fake-shell", {"id": "x-test", "title": "X"})
        finally:
            del lp.POPOVER_RUNNERS["x-test"]
        self.assertEqual(calls, [("specific", "fake-shell", "x-test")])

    def test_dispatcher_falls_back_to_the_generic_runner(self):
        self.assertNotIn("spotlight", lp.POPOVER_RUNNERS)
        self.assertIs(lp.POPOVER_STARTERS["spotlight"], lp.ShellSession.start_launcher)


if __name__ == "__main__":
    unittest.main()
