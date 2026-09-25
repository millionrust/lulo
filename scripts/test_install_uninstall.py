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

    def test_install_pin_matches_the_repository_archive_key_pin(self):
        # One reviewed pin: before the archive key exists install.sh carries
        # a marked placeholder; afterwards create-archive-key.sh writes the
        # same primary fingerprint into both files.
        import json
        import re as regex

        text = INSTALL.read_text(encoding="utf-8")
        lines = regex.findall(r'(?m)^RMAC_ARCHIVE_KEYRING_FINGERPRINT="([^"]*)"$', text)
        self.assertEqual(len(lines), 1)
        pin = json.loads(
            (Path(__file__).parent.parent / "packaging/apt/archive-key.json").read_text(encoding="utf-8")
        )["primary_fingerprints"]
        if pin:
            self.assertEqual(lines[0], pin[0])
            self.assertTrue(
                (Path(__file__).parent.parent / "packaging/apt/archive-keyring.asc").is_file()
            )
        else:
            self.assertTrue(lines[0].startswith("TODO_"))

    def test_repository_install_pins_niri_and_xwayland_satellite_too(self):
        text = INSTALL.read_text(encoding="utf-8")
        preferences = (Path(__file__).parent.parent / "packaging/apt/rmac.pref").read_text(encoding="utf-8")
        self.assertIn(preferences, text)

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
            self.assertIn("not published yet", result.stderr)
            self.assertIn("--from-release", result.stderr)

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

    def test_allow_unattested_applies_only_to_a_release_download(self):
        for arguments in (["--allow-unattested"], ["--from-dir", "/tmp", "--allow-unattested"]):
            result = subprocess.run(
                ["/bin/sh", str(INSTALL), *arguments],
                check=False,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--allow-unattested applies only to --from-release", result.stderr)

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


class InstallReleaseAttestationTests(unittest.TestCase):
    """SR-17: a release's provenance must verify and name this repository's
    release workflow as its signer. Without gh the install refuses unless
    --allow-unattested was passed."""

    def _download(self, gh_exit: int | None, allow_unattested: bool = False):
        import hashlib

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = root / "release"
            release.mkdir()
            names = (
                "rmac-apps_0.9.0.beta.1-1_amd64.deb",
                "rmac-session_0.9.0.beta.1-1_amd64.deb",
            )
            lines = []
            for name in names:
                (release / name).write_bytes(name.encode())
                lines.append(f"{hashlib.sha256(name.encode()).hexdigest()}  {name}")
            (release / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            curl = bin_dir / "curl"
            curl.write_text(
                "#!/bin/sh\n"
                'while [ $# -gt 0 ]; do case "$1" in -o) out="$2"; shift 2;; *) url="$1"; shift;; esac; done\n'
                f'cp "{release}/$(basename "$url")" "$out"\n',
                encoding="utf-8",
            )
            curl.chmod(0o755)
            log = root / "gh.log"
            if gh_exit is not None:
                gh = bin_dir / "gh"
                gh.write_text(
                    f'#!/bin/sh\necho "$@" >> "{log}"\nexit {gh_exit}\n', encoding="utf-8"
                )
                gh.chmod(0o755)
            curl_log = root / "curl.log"
            curl.write_text(
                curl.read_text(encoding="utf-8")
                + f'echo "$url" >> "{curl_log}"\n',
                encoding="utf-8",
            )
            path = f"{bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin"
            flag = "true" if allow_unattested else "false"
            result = _run_install_function(
                f'PATH="{path}"\narchitecture=amd64\nallow_unattested={flag}\n'
                'download_release_packages v0.9.0-beta.1\necho downloaded'
            )
            calls = log.read_text(encoding="utf-8") if log.exists() else ""
            self.downloads = (
                curl_log.read_text(encoding="utf-8") if curl_log.exists() else ""
            )
            return result, calls

    def test_a_failed_attestation_stops_the_install(self):
        result, calls = self._download(gh_exit=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("attestation did not verify", result.stderr)
        self.assertNotIn("downloaded", result.stdout)
        self.assertIn("attestation verify", calls)

    def test_attestation_must_come_from_the_release_workflow(self):
        result, calls = self._download(gh_exit=0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("downloaded", result.stdout)
        self.assertEqual(calls.count("attestation verify"), 2)
        self.assertIn("--signer-workflow millionrust/lulo/.github/workflows/release.yml", calls)

    def _skip_if_gh_on_system_path(self):
        if any(Path(d, "gh").exists() for d in ("/usr/bin", "/bin", "/usr/sbin", "/sbin")):
            self.skipTest("this host has gh on the system path")

    def test_without_gh_the_install_refuses_before_downloading(self):
        self._skip_if_gh_on_system_path()
        result, calls = self._download(gh_exit=None)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("downloaded", result.stdout)
        self.assertIn("'gh' is not installed", result.stderr)
        self.assertIn("--allow-unattested", result.stderr)
        self.assertEqual(self.downloads, "")

    def test_allow_unattested_installs_without_gh_and_says_what_was_not_checked(self):
        self._skip_if_gh_on_system_path()
        result, calls = self._download(gh_exit=None, allow_unattested=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("downloaded", result.stdout)
        self.assertIn("attestation was not checked", result.stderr)
        self.assertIn("rests on HTTPS", result.stderr)

    def test_allow_unattested_never_skips_a_failing_attestation(self):
        result, calls = self._download(gh_exit=1, allow_unattested=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("attestation did not verify", result.stderr)
        self.assertNotIn("downloaded", result.stdout)


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

    def test_finds_a_beta_package_under_the_name_github_stores(self):
        # release.yml renames "~" to "." before checksumming and upload.
        listing = (
            "aa  rmac-apps_0.9.0.beta.1-38_amd64.deb\n"
            "bb  rmac-session_0.9.0.beta.1-38_amd64.deb\n"
        )
        result = self._asset(listing, "rmac-session")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "rmac-session_0.9.0.beta.1-38_amd64.deb")

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

class LocalPackageSelectionTests(unittest.TestCase):
    """Drive install_local_packages() with stubbed dpkg/apt tools."""

    FILES = (
        "rmac-apps_0.9.0.beta.1-38_amd64.deb",
        "rmac-session_0.9.0.beta.1-38_amd64.deb",
        "rmac-apps_0.9.0.beta.1-38_arm64.deb",
        "niri_26.04+lulo1-1_amd64.deb",
        "xwayland-satellite_0.8.2+lulo1-1_amd64.deb",
        "niri_26.04+lulo1-1.dsc",
        "niri_26.04+lulo1.orig.tar.gz",
    )

    def _run(self, files, installed=None, corrupt=None):
        installed = installed or {}
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        packages = root / "package set"
        packages.mkdir()
        for name in files:
            (packages / name).write_text(name, encoding="utf-8")
        result = subprocess.run(
            ["sha256sum", *files], cwd=packages, capture_output=True, text=True, check=True
        )
        (packages / "SHA256SUMS").write_text(result.stdout, encoding="utf-8")
        if corrupt:
            (packages / corrupt).write_text("tampered", encoding="utf-8")
        log = root / "apt.log"
        _stub_sudo(bin_dir)
        _write_stub(bin_dir / "apt-get", f'printf "%s\\n" "$@" > "{log}"')
        status = "\n".join(
            f'  {name}) version="{version}" ;;' for name, version in installed.items()
        )
        _write_stub(
            bin_dir / "dpkg-query",
            'eval "package=\\${$#}"\n'
            'case "$*" in *Status-Abbrev*) mode=status ;; *) mode=version ;; esac\n'
            'case "$package" in\n' + status + "\n  *) exit 1 ;;\nesac\n"
            '[ "$mode" = status ] && echo "ii " || echo "$version"',
        )
        # The package files hold their own names; the version is the middle
        # field, exactly what `dpkg-deb -f FILE Version` would report.
        _write_stub(bin_dir / "dpkg-deb", 'basename "$2" | cut -d_ -f2')
        linux = Path(__file__).parent / "linux"
        _write_stub(
            bin_dir / "dpkg",
            f'exec python3 -c "import sys; sys.path.insert(0, \'{linux}\'); '
            "from third_party_packages import compare_versions as c; "
            "a, op, b = sys.argv[2:5]; r = c(a, b); "
            "sys.exit(0 if {'gt': r > 0, 'lt': r < 0, 'eq': r == 0}[op] else 1)\" \"$@\"",
        )
        driver = root / "driver.sh"
        body = INSTALL.read_text(encoding="utf-8").rsplit('main "$@"', 1)[0]
        driver.write_text(
            body + 'architecture=amd64\ninstall_local_packages "$1"\n', encoding="utf-8"
        )
        completed = subprocess.run(
            ["/bin/sh", str(driver), str(packages)],
            env={"PATH": f"{bin_dir}:/usr/bin:/bin:/sbin", "HOME": str(root)},
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
        installed_args = log.read_text(encoding="utf-8").splitlines() if log.exists() else []
        return completed, [Path(arg).name for arg in installed_args[3:]], installed_args

    def test_installs_rmac_and_the_lulo_niri_builds_for_this_architecture(self):
        completed, names, args = self._run(self.FILES)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(args[:3], ["install", "--yes", "--"])
        self.assertEqual(
            sorted(names),
            sorted([
                "rmac-apps_0.9.0.beta.1-38_amd64.deb",
                "rmac-session_0.9.0.beta.1-38_amd64.deb",
                "niri_26.04+lulo1-1_amd64.deb",
                "xwayland-satellite_0.8.2+lulo1-1_amd64.deb",
            ]),
        )

    def test_installs_over_an_older_ppa_niri_without_allow_downgrades(self):
        # "26.04+lulo1-1" sorts above the danklinux PPA's "26.04ppaN" for
        # any N, so a machine that already has the PPA build gets ours as an
        # ordinary upgrade -- no --allow-downgrades needed.
        completed, names, args = self._run(
            self.FILES, installed={"niri": "26.04ppa3", "xwayland-satellite": "0.8.1-1"}
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("niri_26.04+lulo1-1_amd64.deb", names)
        self.assertIn("xwayland-satellite_0.8.2+lulo1-1_amd64.deb", names)
        self.assertNotIn("--allow-downgrades", " ".join(args))

    def test_keeps_a_newer_niri_instead_of_downgrading(self):
        completed, names, _ = self._run(
            self.FILES, installed={"niri": "26.05-1", "xwayland-satellite": "0.8.1-1"}
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertNotIn("niri_26.04+lulo1-1_amd64.deb", names)
        self.assertIn("xwayland-satellite_0.8.2+lulo1-1_amd64.deb", names)
        self.assertIn("keeping the installed niri 26.05-1", completed.stderr)
        self.assertIn("--allow-downgrades ./niri_26.04+lulo1-1_amd64.deb", completed.stderr)

    def test_a_package_set_without_niri_still_installs_rmac(self):
        completed, names, _ = self._run(self.FILES[:3])
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(len(names), 2)
        self.assertIn("has no niri for amd64", completed.stderr)

    def test_a_tampered_niri_package_stops_the_install(self):
        completed, names, _ = self._run(self.FILES, corrupt="niri_26.04+lulo1-1_amd64.deb")
        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(names, [])
        self.assertIn("did not match SHA256SUMS", completed.stderr)


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
