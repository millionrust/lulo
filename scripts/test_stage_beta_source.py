"""Focused local fixture checks for scripts/stage-beta-source.py."""

from __future__ import annotations

import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest import mock


SCRIPT = Path(__file__).parent / "stage-beta-source.py"
SPEC = importlib.util.spec_from_file_location("stage_beta_source", SCRIPT)
assert SPEC and SPEC.loader
stage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage)


class StageBetaSourceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Fixture")
        (self.root / ".gitignore").write_text("target/\nignored-cache/\n", encoding="utf-8")
        (self.root / "tracked.txt").write_text("committed\n", encoding="utf-8")
        (self.root / "deleted.txt").write_text("remove me\n", encoding="utf-8")
        (self.root / "credentials.json").write_text('{"token":"old-secret"}\n', encoding="utf-8")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture base")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(self.root), *args], check=True, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        ).stdout

    def test_snapshot_has_current_sources_and_provenance_but_excludes_outputs_and_secrets(self) -> None:
        (self.root / "tracked.txt").write_text("staged edit\n", encoding="utf-8")
        self.git("add", "tracked.txt")
        (self.root / "tracked.txt").write_text("worktree edit\n", encoding="utf-8")
        (self.root / "credentials.json").write_text('{"token":"new-secret"}\n', encoding="utf-8")
        (self.root / "new source.py").write_text("print('source')\n", encoding="utf-8")
        (self.root / ".env.local").write_text("TOKEN=private\n", encoding="utf-8")
        (self.root / "id_ed25519").write_text("private key\n", encoding="utf-8")
        (self.root / "absolute-link").symlink_to(self.root / "tracked.txt")
        (self.root / "target").mkdir()
        (self.root / "target" / "binary").write_text("build output", encoding="utf-8")
        (self.root / "ignored-cache").mkdir()
        (self.root / "ignored-cache" / "data").write_text("ignored", encoding="utf-8")
        (self.root / "deleted.txt").unlink()

        destination = Path(self.temp.name) / "payload"
        manifest = stage.build_snapshot(self.root, destination)
        names = {item["path"] for item in manifest["files"]}

        self.assertEqual(names, {".gitignore", "tracked.txt", "new source.py"})
        self.assertEqual((destination / "source" / "tracked.txt").read_text(encoding="utf-8"), "worktree edit\n")
        self.assertFalse((destination / "history.bundle").exists())
        patch = (destination / "worktree.patch").read_bytes()
        self.assertIn(b"tracked.txt", patch)
        self.assertIn(b"deleted.txt", patch)
        self.assertNotIn(b"new-secret", patch)
        self.assertNotIn(b"old-secret", patch)
        self.assertEqual((destination / "HEAD").read_text(encoding="utf-8").strip(), manifest["head"])
        loaded = json.loads((destination / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(loaded["fingerprint_sha256"], manifest["fingerprint_sha256"])
        stage.verify_snapshot(destination)
        subprocess.run(
            ["python3", str(SCRIPT), "--verify", str(destination)],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        artifact_link = destination / "source" / "target"
        artifact_link.symlink_to(Path.home() / "rmac/target")
        stage.verify_snapshot(destination)
        artifact_link.unlink()
        shell_target = destination / "source" / "shell" / "target"
        shell_target.parent.mkdir(exist_ok=True)
        shell_target.symlink_to(Path.home() / "rmac-wt/shell/target")
        stage.verify_snapshot(destination)
        shell_target.unlink()
        shell_target.symlink_to(Path.home() / "unapproved-target")
        with self.assertRaisesRegex(ValueError, "unlisted source file"):
            stage.verify_snapshot(destination)
        shell_target.unlink()
        extra_config = destination / "source" / ".cargo" / "config.toml"
        extra_config.parent.mkdir()
        extra_config.write_text("[build]\nrustflags = ['--cfg', 'unverified']\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "unlisted source file"):
            stage.verify_snapshot(destination)
        extra_config.unlink()
        (destination / "source" / "tracked.txt").write_text("changed in transit\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "source mismatch"):
            stage.verify_snapshot(destination)
        self.assertEqual(stage.validate_host("jacob@192.168.18.52"), None)
        with self.assertRaises(ValueError):
            stage.validate_host("host; touch /tmp/unwanted")
        with self.assertRaises(ValueError):
            stage.validate_host("-oProxyCommand=touch /tmp/unwanted")
        self.assertTrue(stage.is_excluded("nested/TARGET/build-output"))

    def test_staging_retries_connect_and_reuses_one_control_socket(self) -> None:
        original_run = subprocess.run
        calls: list[list[str]] = []
        first_connect = True

        def fake_run(argv, *args, **kwargs):
            nonlocal first_connect
            if argv[0] == "git":
                return original_run(argv, *args, **kwargs)
            calls.append(list(argv))
            if argv[0] == "ssh" and argv[-1].startswith("umask ") and first_connect:
                first_connect = False
                return subprocess.CompletedProcess(argv, 255)
            return subprocess.CompletedProcess(argv, 0)

        previous_cwd = Path.cwd()
        try:
            os.chdir(self.root)
            with mock.patch.object(stage.subprocess, "run", side_effect=fake_run), \
                 mock.patch.object(sys, "argv", ["stage-beta-source.py", "testhost"]), \
                 redirect_stdout(io.StringIO()):
                self.assertEqual(stage.main(), 0)
        finally:
            os.chdir(previous_cwd)

        ssh_calls = [call for call in calls if call[0] == "ssh"]
        self.assertEqual(sum(call[-1].startswith("umask ") for call in ssh_calls), 2)
        self.assertTrue(any("-O" in call and call[-1] == "testhost" for call in ssh_calls))
        paths = {
            option.split("=", 1)[1]
            for call in ssh_calls
            for option in call
            if option.startswith("ControlPath=")
        }
        self.assertEqual(len(paths), 1)
        [rsync_call] = [call for call in calls if call[0] == "rsync"]
        self.assertIn("ControlPath=" + next(iter(paths)), rsync_call[rsync_call.index("-e") + 1])


if __name__ == "__main__":
    unittest.main()
