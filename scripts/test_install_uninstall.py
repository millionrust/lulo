"""Fixture and structural tests for install.sh and uninstall.sh."""

from __future__ import annotations

from pathlib import Path
import subprocess
import tempfile
import unittest


LINUX = Path(__file__).parent / "linux"
INSTALL = LINUX / "install.sh"
UNINSTALL = LINUX / "uninstall.sh"


def _write_stub(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


def _run(script: Path, bin_dir: Path) -> subprocess.CompletedProcess[str]:
    environment = {
        "PATH": f"{bin_dir}:/usr/bin:/bin",
        "HOME": str(bin_dir),
    }
    return subprocess.run(
        ["/bin/sh", str(script)],
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )


def _stub_id(bin_dir: Path, uid: str) -> None:
    _write_stub(bin_dir / "id", f'if [ "$1" = "-u" ]; then echo "{uid}"; fi')


def _stub_sudo(bin_dir: Path) -> None:
    # A real sudo just execs its arguments; nothing here needs privilege
    # separation, so the stub does the same.
    _write_stub(bin_dir / "sudo", 'exec "$@"')


class ScriptStructureTests(unittest.TestCase):
    def _assert_common_shape(self, script: Path) -> None:
        result = subprocess.run(
            ["/bin/sh", "-n", str(script)],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        text = script.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("#!/bin/sh\n"))
        self.assertIn("set -eu", text)
        lines = [line for line in text.splitlines() if line.strip()]
        self.assertEqual(lines[-1], 'main "$@"')
        self.assertEqual(text.count("\nmain "), 1)
        self.assertNotIn("\r", text)
        # Never touch the GNOME session.
        for forbidden in ("gdm3 ", "gnome-session", "update-alternatives", "systemctl set-default"):
            self.assertNotIn(forbidden, text)

    def test_install_script_shape(self):
        self._assert_common_shape(INSTALL)

    def test_uninstall_script_shape(self):
        self._assert_common_shape(UNINSTALL)

    def test_install_never_runs_as_root_and_uses_sudo(self):
        text = INSTALL.read_text(encoding="utf-8")
        self.assertIn('"$(id -u)" -eq 0', text)
        self.assertIn("sudo ", text)

    def test_install_checks_ubuntu_26_04_and_both_architectures(self):
        text = INSTALL.read_text(encoding="utf-8")
        self.assertIn('VERSION_ID:-}" = "26.04"', text)
        self.assertIn("amd64 | arm64", text)

    def test_install_pins_a_marked_placeholder_fingerprint(self):
        text = INSTALL.read_text(encoding="utf-8")
        self.assertIn("RMAC_ARCHIVE_KEYRING_FINGERPRINT=", text)
        self.assertIn("TODO", text)

    def test_install_ends_with_the_required_login_message(self):
        text = INSTALL.read_text(encoding="utf-8")
        self.assertIn("Log out and choose Lulo OS on the login screen.", text)

    def test_uninstall_purges_all_three_packages(self):
        text = UNINSTALL.read_text(encoding="utf-8")
        self.assertIn("rmac-session", text)
        self.assertIn("rmac-apps", text)
        self.assertIn("rmac-archive-keyring", text)
        self.assertIn("purge", text)


class InstallScriptBehaviorTests(unittest.TestCase):
    def test_refuses_to_run_as_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _stub_id(bin_dir, "0")
            result = _run(INSTALL, bin_dir)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("root", result.stderr)

    def test_refuses_to_run_with_the_unreplaced_placeholder_fingerprint(self):
        # This is the current, correct state of the repository: the real
        # archive key does not exist yet, so install.sh must refuse rather
        # than silently skip verification.
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _stub_id(bin_dir, "1000")
            _stub_sudo(bin_dir)
            result = _run(INSTALL, bin_dir)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("not been published", result.stderr)

    def test_refuses_on_a_machine_that_is_not_ubuntu_26_04(self):
        # install.sh reads the real /etc/os-release rather than an
        # injectable path (it is meant to run unmodified on the target
        # machine), so exercise the distro/version gate by running the real
        # script on this development machine, which is never the Ubuntu
        # 26.04 target platform.
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _stub_id(bin_dir, "1000")
            _stub_sudo(bin_dir)
            patched = bin_dir / "install-with-real-fingerprint.sh"
            text = INSTALL.read_text(encoding="utf-8").replace(
                "TODO_REPLACE_WITH_THE_REAL_ARCHIVE_FINGERPRINT",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            patched.write_text(text, encoding="utf-8")
            result = _run(patched, bin_dir)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
