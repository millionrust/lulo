"""Fixture tests for the rmac-update-check daily nudge script."""

from __future__ import annotations

import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "linux" / "rmac-update-check.sh"


def _write_stub(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


def _run(bin_dir: Path) -> subprocess.CompletedProcess[str]:
    # The stub directory is searched first so it can shadow pkcon /
    # notify-send, but the script also needs ordinary tools (grep, command,
    # printf) that the fixture does not stub out.
    environment = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(bin_dir)}
    return subprocess.run(
        ["/bin/sh", str(SCRIPT)],
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )


class UpdateCheckScriptTests(unittest.TestCase):
    def test_script_is_syntactically_valid_posix_shell(self):
        result = subprocess.run(
            ["/bin/sh", "-n", str(SCRIPT)],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_calls_main_once_on_the_last_line(self):
        text = SCRIPT.read_text(encoding="utf-8")
        lines = [line for line in text.splitlines() if line.strip()]
        self.assertEqual(lines[-1], 'main "$@"')
        self.assertEqual(text.count("\nmain "), 1)

    def test_missing_pkcon_is_a_silent_no_op(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            result = _run(bin_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "")

    def test_no_updates_never_calls_notify_send(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _write_stub(bin_dir / "pkcon", "exit 0")
            marker = bin_dir / "notified"
            _write_stub(bin_dir / "notify-send", f"echo called >>'{marker}'")
            result = _run(bin_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(marker.exists())

    def test_available_updates_trigger_one_notification(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _write_stub(
                bin_dir / "pkcon",
                'case "$1" in\n'
                "refresh) exit 0 ;;\n"
                "get-updates)\n"
                "  echo 'Security     rmac-session;1.2.0-1;amd64;rmac'\n"
                "  echo 'Bugfix       rmac-apps;1.2.0-1;amd64;rmac'\n"
                "  ;;\n"
                "esac\n",
            )
            marker = bin_dir / "notified.txt"
            _write_stub(bin_dir / "notify-send", f"printf '%s\\n' \"$@\" >>'{marker}'")
            result = _run(bin_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(marker.exists())
            body = marker.read_text(encoding="utf-8")
            self.assertIn("2 updates are available.", body)

    def test_missing_notify_send_is_still_a_clean_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _write_stub(
                bin_dir / "pkcon",
                'case "$1" in\n'
                "refresh) exit 0 ;;\n"
                "get-updates) echo 'Security     rmac-session;1.2.0-1;amd64;rmac' ;;\n"
                "esac\n",
            )
            result = _run(bin_dir)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_script_is_executable_and_a_posix_shell_script(self):
        mode = SCRIPT.stat().st_mode
        self.assertTrue(mode & stat.S_IXUSR)
        text = SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("#!/bin/sh\n"))
        self.assertNotIn("\r", text)


if __name__ == "__main__":
    unittest.main()
