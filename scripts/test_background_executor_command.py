"""Portable fixture checks for the background-executor command guard."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

CHECK = Path(__file__).with_name("check-background-executor-command.sh")


class BackgroundExecutorCommandTests(unittest.TestCase):
    def check_snippet(self, source: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.rs"
            path.write_text(source)
            return subprocess.run(["bash", str(CHECK), str(path)], capture_output=True, text=True)

    def test_bare_command_is_flagged(self):
        result = self.check_snippet("""
            cx.background_executor()
                .spawn(async move {
                    std::process::Command::new("gio").status();
                });
        """)
        self.assertEqual(result.returncode, 1)
        self.assertIn("fixture.rs:4: Command spawned", result.stdout)

    def test_blocking_pool_is_accepted(self):
        result = self.check_snippet("""
            cx.background_executor()
                .spawn(async move {
                    let status = blocking::unblock(move || {
                        std::process::Command::new("gio").status()
                    }).await;
                });
        """)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_comment_does_not_count_as_unblock(self):
        result = self.check_snippet("""
            cx.background_executor()
                .spawn(async move {
                    // blocking::unblock should wrap this call.
                    std::process::Command::new("gio").status();
                });
        """)
        self.assertEqual(result.returncode, 1)

    def test_explicit_allow_is_accepted(self):
        result = self.check_snippet("""
            cx.background_executor()
                .spawn(async move {
                    std::process::Command::new("gio").status(); // background-executor-allow: fixture
                });
        """)
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
