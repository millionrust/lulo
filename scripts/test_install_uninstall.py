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

    def test_from_release_requires_a_tag_argument(self):
        # Argument validation happens before check_not_root/check_platform,
        # so this is exercised without any stubbing on any host.
        result = subprocess.run(
            ["/bin/sh", str(INSTALL), "--from-release"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--from-release requires a release tag", result.stderr)

    def test_from_dir_requires_a_directory_argument(self):
        result = subprocess.run(
            ["/bin/sh", str(INSTALL), "--from-dir"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--from-dir requires a directory path", result.stderr)

    def test_rejects_an_unrecognized_argument(self):
        result = subprocess.run(
            ["/bin/sh", str(INSTALL), "--bogus"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unrecognized argument", result.stderr)

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


def _run_install_function(snippet: str) -> subprocess.CompletedProcess[str]:
    """Run install.sh's function definitions (without main) plus `snippet`."""
    text = INSTALL.read_text(encoding="utf-8")
    body, _, _ = text.rpartition('\nmain "$@"')
    return subprocess.run(
        ["/bin/sh", "-c", f"{body}\n{snippet}\n"],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )


PINNED = "0123456789ABCDEF0123456789ABCDEF01234567"
OTHER = "FEDCBA9876543210FEDCBA9876543210FEDCBA98"


def _listing(*records: str) -> str:
    return "\n".join(records)


class InstallKeyringVerificationTests(unittest.TestCase):
    def _verify(self, listing: str) -> subprocess.CompletedProcess[str]:
        return _run_install_function(
            f"RMAC_ARCHIVE_KEYRING_FINGERPRINT={PINNED}\n"
            f"verify_keyring_listing '{listing}' && echo verified"
        )

    def test_accepts_the_pinned_primary_key_with_a_subkey(self):
        result = self._verify(
            _listing(
                "pub:-:255:22:0123456789ABCDEF:1700000000:::-:::scESC::::::23::0:",
                f"fpr:::::::::{PINNED}:",
                "uid:-::::1700000000::HASH::rmac archive::::::::::0:",
                "sub:-:255:18:AAAAAAAAAAAAAAAA:1700000000::::::e::::::23:",
                f"fpr:::::::::{OTHER}:",
            )
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified", result.stdout)

    def test_rejects_a_second_primary_key_beside_the_pinned_one(self):
        # Previously any keyring that merely *contained* the pinned
        # fingerprint passed, so an extra attacker key would have been
        # trusted by Signed-By too.
        result = self._verify(
            _listing(
                "pub:-:255:22:0123456789ABCDEF:1700000000:::-:::scESC::::::23::0:",
                f"fpr:::::::::{PINNED}:",
                "pub:-:255:22:FEDCBA9876543210:1700000000:::-:::scESC::::::23::0:",
                f"fpr:::::::::{OTHER}:",
            )
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one primary key", result.stderr)

    def test_rejects_the_pinned_fingerprint_only_as_a_subkey(self):
        result = self._verify(
            _listing(
                "pub:-:255:22:FEDCBA9876543210:1700000000:::-:::scESC::::::23::0:",
                f"fpr:::::::::{OTHER}:",
                "sub:-:255:18:0123456789ABCDEF:1700000000::::::e::::::23:",
                f"fpr:::::::::{PINNED}:",
            )
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("pinned rmac archive fingerprint", result.stderr)

    def test_rejects_an_empty_keyring(self):
        result = self._verify("")
        self.assertNotEqual(result.returncode, 0)

    def test_installs_only_the_verified_keyring_file_never_the_unsigned_deb(self):
        text = INSTALL.read_text(encoding="utf-8")
        self.assertNotIn("sudo dpkg -i", text)
        self.assertIn('install -o root -g root -m 0644 "$verified_keyring_file"', text)
        self.assertIn("apt-get install --yes rmac-archive-keyring rmac-session", text)


class InstallReleaseAssetNameTests(unittest.TestCase):
    def _asset(self, listing: str, package: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temporary:
            sums = Path(temporary) / "SHA256SUMS"
            sums.write_text(listing, encoding="utf-8")
            return _run_install_function(
                f"architecture=amd64\nrelease_asset_name '{sums}' {package}"
            )

    def test_finds_a_beta_package_with_a_debian_tilde_version(self):
        listing = (
            "aa  rmac-apps_0.9.0~beta.1-1_amd64.deb\n"
            "bb  rmac-apps_0.9.0~beta.1-1_arm64.deb\n"
            "cc  rmac-session_0.9.0~beta.1-1_amd64.deb\n"
        )
        result = self._asset(listing, "rmac-apps")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "rmac-apps_0.9.0~beta.1-1_amd64.deb")

    def test_finds_a_final_release_package(self):
        result = self._asset("aa  rmac-session_1.2.3-1_amd64.deb\n", "rmac-session")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "rmac-session_1.2.3-1_amd64.deb")

    def test_rejects_path_like_or_ambiguous_names(self):
        for listing in (
            "aa  ../rmac-apps_1.2.3-1_amd64.deb\n",
            "aa  rmac-apps_1.2.3-1_amd64.deb/x\n",
            "aa  rmac-apps_1.2.3-1_amd64.deb\nbb  rmac-apps_1.2.4-1_amd64.deb\n",
        ):
            with self.subTest(listing=listing):
                result = self._asset(listing, "rmac-apps")
                self.assertNotEqual(result.returncode, 0)


class UninstallScriptBehaviorTests(unittest.TestCase):
    def test_runs_cleanly_when_no_rmac_packages_are_known_to_dpkg(self):
        # On a machine where dpkg/systemctl are unavailable (this development
        # machine) or simply report nothing installed, uninstall.sh must
        # still finish cleanly rather than let a single unknown package name
        # abort the whole apt-get purge invocation.
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _stub_id(bin_dir, "1000")
            _stub_sudo(bin_dir)
            result = _run(UNINSTALL, bin_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("rmac has been removed", result.stdout)

    def test_refuses_to_run_as_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            _stub_id(bin_dir, "0")
            result = _run(UNINSTALL, bin_dir)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("root", result.stderr)


if __name__ == "__main__":
    unittest.main()
