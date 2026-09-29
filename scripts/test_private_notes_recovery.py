"""Safe mock checks for the private Notes recovery runner's safety boundary."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock


SCRIPT = Path(__file__).parent / "behavior" / "run_private_notes_recovery.py"
SPEC = importlib.util.spec_from_file_location("private_notes_recovery", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


class PrivateNotesRecoverySafetyTests(unittest.TestCase):
    def test_private_environment_confines_home_and_data_to_work_directory(self):
        from tempfile import TemporaryDirectory

        with TemporaryDirectory() as raw:
            work = Path(raw)
            env = runner.private_environment(work)
        self.assertTrue(Path(env["HOME"]).is_relative_to(work))
        self.assertTrue(Path(env["XDG_DATA_HOME"]).is_relative_to(work))
        self.assertEqual(env["RMAC_BEHAVIOR_NESTED"], "1")

    def test_hard_kill_uses_only_the_owned_popen(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.pid = 43121
        runner.kill_owned_process(process)
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=8)

    def test_hard_kill_does_nothing_after_child_exits(self):
        process = mock.Mock()
        process.poll.return_value = 0
        runner.kill_owned_process(process)
        process.kill.assert_not_called()
        process.wait.assert_not_called()

    def test_process_group_cleanup_sends_term_and_waits(self):
        process = mock.Mock()
        process.pid = 43121
        process.poll.side_effect = [None, 0]
        with mock.patch.object(
            runner.os, "killpg", side_effect=[None, ProcessLookupError, ProcessLookupError]
        ) as killpg:
            runner.stop_owned_process_group(process)
        self.assertEqual(killpg.call_args_list, [
            mock.call(43121, runner.signal.SIGTERM),
            mock.call(43121, 0),
            mock.call(43121, 0),
        ])
        process.wait.assert_called_once_with(timeout=5)

    def test_process_group_cleanup_escalates_and_reaps_after_timeout(self):
        process = mock.Mock()
        process.pid = 43121
        process.poll.return_value = None
        process.wait.side_effect = [runner.subprocess.TimeoutExpired("session", 0), 0]
        with mock.patch.object(
            runner.os, "killpg",
            side_effect=[None, None, None],
        ) as killpg:
            runner.stop_owned_process_group(process, grace=0)
        self.assertEqual(killpg.call_args_list, [
            mock.call(43121, runner.signal.SIGTERM),
            mock.call(43121, 0),
            mock.call(43121, runner.signal.SIGKILL),
        ])
        self.assertEqual(process.wait.call_args_list, [
            mock.call(timeout=0), mock.call(timeout=8),
        ])

    def test_process_group_cleanup_stops_descendants_after_leader_exits(self):
        process = mock.Mock()
        process.pid = 43121
        process.poll.return_value = 0
        with mock.patch.object(
            runner.os, "killpg",
            side_effect=[None, ProcessLookupError, ProcessLookupError]
        ) as killpg:
            runner.stop_owned_process_group(process)
        self.assertEqual(killpg.call_args_list, [
            mock.call(43121, runner.signal.SIGTERM),
            mock.call(43121, 0),
            mock.call(43121, 0),
        ])
        process.wait.assert_not_called()

    def test_body_locator_uses_live_verified_entry_name_and_text_interface(self):
        class Node:
            name = "Body"

            def getRoleName(self):
                return "entry"

            def queryText(self):
                return "text-interface"

        node = Node()

        class App:
            childCount = 1

            def getChildAtIndex(self, index):
                if index != 0:
                    raise IndexError(index)
                return node

        app = App()
        self.assertIs(runner.body_node(app), node)

    def test_body_locator_rejects_wrong_role(self):
        class Node:
            name = "Body"

            def getRoleName(self):
                return "paragraph"

            def queryText(self):
                return "text-interface"

        class App:
            childCount = 1

            def getChildAtIndex(self, index):
                if index != 0:
                    raise IndexError(index)
                return Node()

        self.assertIsNone(runner.body_node(App()))

    def test_body_locator_requires_text_interface(self):
        class Node:
            name = "Body"

            def getRoleName(self):
                return "entry"

            def queryText(self):
                raise RuntimeError("no Text interface")

        class App:
            childCount = 1

            def getChildAtIndex(self, index):
                if index != 0:
                    raise IndexError(index)
                return Node()

        self.assertIsNone(runner.body_node(App()))

    def test_body_text_reads_accessible_editor(self):
        text = mock.Mock()
        text.getText.return_value = "draft MARKER"
        node = mock.Mock()
        node.name = "Body"
        node.getRoleName.return_value = "entry"
        node.queryText.return_value = text
        app = mock.Mock()
        app.childCount = 1
        app.getChildAtIndex.return_value = node

        self.assertEqual(runner.body_text(app), "draft MARKER")
        self.assertTrue(runner.recovered_marker_visible(app, "MARKER"))
        text.getText.assert_called_with(0, -1)

    def test_crash_wait_requires_typed_marker_in_private_draft_bytes(self):
        from tempfile import TemporaryDirectory

        with TemporaryDirectory() as raw:
            data_root = Path(raw)
            drafts = data_root / "drafts"
            drafts.mkdir()
            draft = drafts / "note-test.draft"
            draft.write_bytes(b"earlier draft without the latest edit")
            self.assertFalse(runner.persisted_marker_visible(data_root, "LATEST_MARKER"))
            draft.write_bytes(b"synthetic draft LATEST_MARKER")
            self.assertTrue(runner.persisted_marker_visible(data_root, "LATEST_MARKER"))


if __name__ == "__main__":
    unittest.main()
