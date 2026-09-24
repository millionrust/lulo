"""Tests for scripts/release/create-archive-key.sh.

The static checks run everywhere. The end-to-end key ceremony needs GnuPG
(gpg and gpgv) and is skipped when they are missing; it only ever uses
temporary GnuPG homes (HOME is redirected to a temporary directory too).
"""

import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = REPO_ROOT / "scripts/release/create-archive-key.sh"
INSTALL_SH = REPO_ROOT / "scripts/linux/install.sh"
ARCHIVE_KEY_JSON = REPO_ROOT / "packaging/apt/archive-key.json"
CONTRACT_MODULE = REPO_ROOT / "scripts/linux/keyring_package_contract.py"
TODO_LINE = 'RMAC_ARCHIVE_KEYRING_FINGERPRINT="TODO_REPLACE_WITH_THE_REAL_ARCHIVE_FINGERPRINT"'
PRIMARY_PASSPHRASE = "primary passphrase for tests only 1"
BACKUP_PASSPHRASE = "backup passphrase for tests only 22"
HAVE_GPG = bool(shutil.which("gpg") and shutil.which("gpgv") and shutil.which("gpgconf"))


def load_contract():
    spec = importlib.util.spec_from_file_location("keyring_package_contract", CONTRACT_MODULE)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def code_outside_heredocs(text):
    """Return the script's shell code lines, without heredoc bodies or comments."""
    lines = []
    terminator = None
    for line in text.splitlines():
        if terminator is not None:
            if line.strip() == terminator:
                terminator = None
            continue
        match = re.search(r"<<-?\s*'?\"?([A-Za-z_]+)'?\"?", line)
        if match:
            terminator = match.group(1)
        stripped = line.strip()
        if stripped.startswith("#"):
            continue
        lines.append(line)
    return lines


class StaticTests(unittest.TestCase):
    def setUp(self):
        self.text = SCRIPT.read_text(encoding="utf-8")

    def test_executable_and_bash_syntax(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK), "script must be executable")
        self.assertTrue(self.text.startswith("#!/usr/bin/env bash\n"))
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)
        self.assertIn("set -euo pipefail", self.text)

    def test_never_uses_the_everyday_gnupg_home(self):
        self.assertNotIn("~/.gnupg", self.text)
        self.assertNotIn("$HOME/.gnupg", self.text)
        self.assertNotIn("${HOME}/.gnupg", self.text)
        self.assertIn("mktemp -d", self.text)
        # Every gpg invocation outside heredocs names its home explicitly.
        for line in code_outside_heredocs(self.text):
            if re.search(r"(^\s*|[|;(]\s*)gpg\s+-", line):
                self.assertIn("--homedir", line, line)

    def test_no_network_or_publishing(self):
        code = "\n".join(code_outside_heredocs(self.text))
        for word in ("curl", "wget", "git push", "--send-key", "--recv-key", "--keyserver"):
            self.assertNotIn(word, self.text, word)
        # gh appears only in the printed next steps, never as a command.
        self.assertIsNone(re.search(r"(^|[\s|;&(`])gh\s", code), "gh must not be called")

    def test_passphrases_never_on_a_command_line(self):
        self.assertIsNone(re.search(r"--passphrase(?!-fd|-repeat)\b", self.text))
        self.assertNotIn("export primary_passphrase", self.text)
        self.assertNotIn("export backup_passphrase", self.text)
        self.assertIn("--passphrase-fd 0", self.text)

    def test_help_documents_test_only_mode(self):
        result = subprocess.run(
            ["bash", str(SCRIPT), "--help"], capture_output=True, text=True, check=True
        )
        self.assertIn("--non-interactive", result.stdout)
        self.assertIn("Test-suite only", result.stdout)
        self.assertNotIn("set -euo", result.stdout)

    def test_missing_gpg_is_reported(self):
        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary)
            for tool in ("dirname", "uname", "tail", "sed", "mktemp", "rm", "chmod"):
                found = shutil.which(tool)
                if found:
                    (bin_dir / tool).symlink_to(found)
            result = subprocess.run(
                [shutil.which("bash") or "/bin/bash", str(SCRIPT), "--no-write-repo"],
                capture_output=True,
                text=True,
                env={"PATH": str(bin_dir), "HOME": temporary},
                stdin=subprocess.DEVNULL,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("'gpg' was not found", result.stderr)


def remove_home(home):
    subprocess.run(["gpgconf", "--homedir", str(home), "--kill", "all"], capture_output=True)
    shutil.rmtree(home, ignore_errors=True)


def colon_records(listing, kind):
    return [line.split(":") for line in listing.splitlines() if line.startswith(kind + ":")]


@unittest.skipUnless(HAVE_GPG, "gpg, gpgv and gpgconf are required")
class CeremonyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="lulo-key-test.")
        cls.root = Path(cls.temporary.name)
        cls.home = cls.root / "home"
        cls.home.mkdir()
        cls.repo = cls.root / "repo"
        (cls.repo / "packaging/apt").mkdir(parents=True)
        (cls.repo / "scripts/linux").mkdir(parents=True)
        shutil.copy(ARCHIVE_KEY_JSON, cls.repo / "packaging/apt/archive-key.json")
        cls.install_stub = cls.repo / "scripts/linux/install.sh"
        cls.install_stub.write_text(
            "#!/bin/sh\n# stub\n" + TODO_LINE + "\necho keep-this-line\n", encoding="utf-8"
        )
        cls.install_stub.chmod(0o755)
        cls.primary_file = cls.root / "primary.pass"
        cls.backup_file = cls.root / "backup.pass"
        cls.primary_file.write_text(PRIMARY_PASSPHRASE + "\n", encoding="utf-8")
        cls.backup_file.write_text(BACKUP_PASSPHRASE + "\n", encoding="utf-8")
        cls.out = cls.root / "out"
        cls.result = cls.run_script(cls.out)
        if cls.result.returncode != 0:
            cls.temporary.cleanup()
            raise AssertionError(
                "create-archive-key.sh failed:\n" + cls.result.stdout + cls.result.stderr
            )
        cls.fingerprint = (cls.out / "primary-fingerprint.txt").read_text().strip()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    @classmethod
    def run_script(cls, out, *extra):
        env = dict(os.environ)
        env["HOME"] = str(cls.home)
        env.pop("GNUPGHOME", None)
        return subprocess.run(
            [
                "bash",
                str(SCRIPT),
                "--non-interactive",
                "--email",
                "archive@example.org",
                "--primary-passphrase-file",
                str(cls.primary_file),
                "--backup-passphrase-file",
                str(cls.backup_file),
                "--subkey-lifetime",
                "1y",
                "--output-dir",
                str(out),
                "--write-repo",
                str(cls.repo),
                *extra,
            ],
            capture_output=True,
            text=True,
            env=env,
            stdin=subprocess.DEVNULL,
            timeout=600,
        )

    def gpg_home(self):
        home = Path(tempfile.mkdtemp(prefix="g.", dir="/tmp"))
        home.chmod(0o700)
        self.addCleanup(remove_home, home)
        return home

    def gpg(self, home, *args, input=None, check=True):
        return subprocess.run(
            ["gpg", "--homedir", str(home), "--batch", "--no-tty", "--with-colons", *args],
            input=input,
            capture_output=True,
            check=check,
        )

    def test_everyday_gnupg_home_untouched(self):
        self.assertFalse((self.home / ".gnupg").exists())

    def test_output_files_and_modes(self):
        self.assertEqual(stat.S_IMODE(self.out.stat().st_mode), 0o700)
        modes = {path.name: stat.S_IMODE(path.stat().st_mode) for path in self.out.iterdir()}
        self.assertEqual(
            modes,
            {
                "archive-keyring.asc": 0o644,
                "rmac-archive-keyring.gpg": 0o644,
                "primary-fingerprint.txt": 0o644,
                "key-info.txt": 0o644,
                "RMAC_APT_SIGNING_SUBKEY.asc": 0o600,
                "primary-key-backup.tar.gpg": 0o600,
            },
        )
        self.assertRegex(self.fingerprint, r"^[0-9A-F]{40}$")
        self.assertIn("gh secret set RMAC_APT_SIGNING_SUBKEY --env apt-signing", self.result.stdout)
        self.assertIn("gh secret set RMAC_APT_SIGNING_SUBKEY --env apt-refresh", self.result.stdout)
        self.assertIn(
            "gh variable set RMAC_ARCHIVE_SIGNING_FINGERPRINT --body " + self.fingerprint,
            self.result.stdout,
        )

    def test_primary_is_certify_only_with_one_expiring_signing_subkey(self):
        home = self.gpg_home()
        self.gpg(home, "--import", str(self.out / "archive-keyring.asc"))
        listing = self.gpg(home, "--list-keys").stdout.decode()
        pubs = colon_records(listing, "pub")
        self.assertEqual(len(pubs), 1)
        self.assertEqual(re.sub("[^a-z]", "", pubs[0][11]), "c")
        self.assertEqual(pubs[0][6], "")
        self.assertEqual(pubs[0][3], "22")
        subs = colon_records(listing, "sub")
        self.assertEqual(len(subs), 1)
        self.assertEqual(subs[0][11], "s")
        self.assertTrue(subs[0][6].isdigit())
        expiry = int(subs[0][6])
        now = time.time()
        self.assertGreater(expiry, now + 360 * 86400)
        self.assertLess(expiry, now + 370 * 86400)
        self.assertEqual(colon_records(listing, "sec"), [])

    def test_subkey_export_has_no_primary_secret_and_signs(self):
        home = self.gpg_home()
        self.gpg(home, "--import", str(self.out / "RMAC_APT_SIGNING_SUBKEY.asc"))
        listing = self.gpg(home, "--with-keygrip", "--list-secret-keys").stdout.decode()
        secs = colon_records(listing, "sec")
        ssbs = colon_records(listing, "ssb")
        self.assertEqual(len(secs), 1)
        self.assertEqual(secs[0][14], "#", "primary must be a stub")
        self.assertEqual(len(ssbs), 1)
        self.assertEqual(ssbs[0][14], "+", "subkey secret must be present")
        self.assertEqual(len(list((home / "private-keys-v1.d").iterdir())), 1)

        message = home / "Release"
        message.write_text("Origin: test\n")
        self.gpg(
            home,
            "--pinentry-mode",
            "error",
            "--local-user",
            self.fingerprint,
            "--clearsign",
            "--output",
            str(home / "InRelease"),
            str(message),
        )
        verify_home = self.gpg_home()
        result = subprocess.run(
            [
                "gpgv",
                "--homedir",
                str(verify_home),
                "--status-fd",
                "1",
                "--keyring",
                str(self.out / "rmac-archive-keyring.gpg"),
                str(home / "InRelease"),
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        validsig = [
            line.split() for line in result.stdout.splitlines() if " VALIDSIG " in line
        ]
        self.assertEqual(len(validsig), 1)
        self.assertEqual(validsig[0][-1], self.fingerprint)
        self.assertNotEqual(validsig[0][2], self.fingerprint, "must be signed by the subkey")

    def test_public_keyring_satisfies_the_keyring_package_contract(self):
        contract = load_contract()
        for path in (
            self.out / "archive-keyring.asc",
            self.out / "rmac-archive-keyring.gpg",
            self.repo / "packaging/apt/archive-keyring.asc",
        ):
            identity = contract.canonicalize_keyring(
                path,
                (self.fingerprint,),
                epoch=int(time.time()),
                maximum_bytes=4 * 1024 * 1024,
            )
            self.assertEqual(identity.fingerprints, (self.fingerprint,))
            self.assertTrue(identity.bytes)
        with self.assertRaises(contract.KeyringPackageError):
            contract.canonicalize_keyring(
                self.out / "RMAC_APT_SIGNING_SUBKEY.asc",
                (self.fingerprint,),
                epoch=int(time.time()),
                maximum_bytes=4 * 1024 * 1024,
            )

    def run_install_sh_check(self, pinned, keyring):
        source = INSTALL_SH.read_text(encoding="utf-8")
        match = re.search(r"^verify_keyring_listing\(\) \{\n.*?^\}\n", source, re.S | re.M)
        self.assertIsNotNone(match, "install.sh verify_keyring_listing not found")
        home = self.gpg_home()
        listing = self.gpg(home, "--show-keys", str(keyring)).stdout.decode()
        program = (
            'fail() { echo "$1" >&2; exit 1; }\n'
            + match.group(0)
            + 'RMAC_ARCHIVE_KEYRING_FINGERPRINT="$1"\nverify_keyring_listing "$2"\n'
        )
        return subprocess.run(
            ["sh", "-c", program, "sh", pinned, listing], capture_output=True, text=True
        )

    def test_install_sh_pin_accepts_the_keyring(self):
        keyring = self.out / "rmac-archive-keyring.gpg"
        accepted = self.run_install_sh_check(self.fingerprint, keyring)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        rejected = self.run_install_sh_check("0" * 40, keyring)
        self.assertNotEqual(rejected.returncode, 0)

    def test_backup_decrypts_with_backup_passphrase_only(self):
        home = self.gpg_home()
        restored = home / "backup.tar"
        base = ["--pinentry-mode", "loopback", "--passphrase-fd", "0", "--no-symkey-cache"]
        wrong = self.gpg(
            home,
            *base,
            "--decrypt",
            "--output",
            str(home / "wrong.tar"),
            str(self.out / "primary-key-backup.tar.gpg"),
            input=(PRIMARY_PASSPHRASE + "\n").encode(),
            check=False,
        )
        self.assertNotEqual(wrong.returncode, 0)
        self.gpg(
            home,
            *base,
            "--decrypt",
            "--output",
            str(restored),
            str(self.out / "primary-key-backup.tar.gpg"),
            input=(BACKUP_PASSPHRASE + "\n").encode(),
        )
        names = subprocess.run(
            ["tar", "-tf", str(restored)], capture_output=True, text=True, check=True
        ).stdout.split()
        prefix = "lulo-archive-key-backup/"
        for member in (
            "primary-secret-key.asc",
            "revocation-certificate.rev",
            "archive-keyring.asc",
            "README.txt",
        ):
            self.assertIn(prefix + member, names)
        extract = home / "x"
        extract.mkdir()
        subprocess.run(["tar", "-C", str(extract), "-xf", str(restored)], check=True)
        self.gpg(home, "--import", str(extract / prefix / "primary-secret-key.asc"))
        listing = self.gpg(home, "--list-secret-keys").stdout.decode()
        secs = colon_records(listing, "sec")
        self.assertEqual(len(secs), 1)
        self.assertEqual(secs[0][14], "+", "backup must hold the primary secret")
        readme = (extract / prefix / "README.txt").read_text()
        self.assertIn(self.fingerprint, readme)
        self.assertIn("--quick-set-expire", readme)

    def test_repository_files_updated(self):
        data = json.loads((self.repo / "packaging/apt/archive-key.json").read_text())
        original = json.loads(ARCHIVE_KEY_JSON.read_text())
        self.assertEqual(data["primary_fingerprints"], [self.fingerprint])
        self.assertEqual(data["format"], original["format"])
        self.assertEqual(data["public_keyring"], original["public_keyring"])
        self.assertEqual(
            (self.repo / "packaging/apt/archive-keyring.asc").read_bytes(),
            (self.out / "archive-keyring.asc").read_bytes(),
        )
        script = self.install_stub.read_text()
        self.assertNotIn("TODO_REPLACE", script)
        self.assertIn('\nRMAC_ARCHIVE_KEYRING_FINGERPRINT="%s"\n' % self.fingerprint, script)
        self.assertIn("echo keep-this-line\n", script)
        self.assertEqual(stat.S_IMODE(self.install_stub.stat().st_mode), 0o755)

    def test_rerun_without_replace_refuses(self):
        out = self.root / "second"
        result = self.run_script(out)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ROTATION", result.stderr)
        self.assertIn("docs/update-trust.md", result.stderr)
        self.assertFalse(out.exists())
        data = json.loads((self.repo / "packaging/apt/archive-key.json").read_text())
        self.assertEqual(data["primary_fingerprints"], [self.fingerprint])


if __name__ == "__main__":
    unittest.main()
