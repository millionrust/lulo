"""Focused build-free tests for the destructive H5 lifecycle boundary."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).parent / "linux/run-package-lifecycle.py"
SPECIFICATION = importlib.util.spec_from_file_location("package_lifecycle", SCRIPT)
assert SPECIFICATION is not None and SPECIFICATION.loader is not None
lifecycle = importlib.util.module_from_spec(SPECIFICATION)
sys.modules[SPECIFICATION.name] = lifecycle
SPECIFICATION.loader.exec_module(lifecycle)


class PackageLifecycleTests(unittest.TestCase):
    def test_reviewed_contract_is_exact_and_keeps_the_disk_floor(self):
        contract = lifecycle.load_contract()
        self.assertEqual(contract, lifecycle.expected_contract())
        self.assertEqual(contract["minimum_free_gib"], 15)
        self.assertEqual(contract["packages"], ["rmac-apps", "rmac-session"])
        self.assertEqual(
            contract["steps"],
            [
                "install-baseline",
                "upgrade-candidate",
                "interrupt-rollback-after-unpack",
                "recover-rollback",
                "remove",
                "purge",
                "reinstall-candidate",
                "final-purge",
            ],
        )

    def test_contract_drift_and_non_disposable_hosts_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "lifecycle.json"
            changed = lifecycle.expected_contract()
            changed["minimum_free_gib"] = 1
            path.write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaisesRegex(lifecycle.LifecycleError, "reviewed policy"):
                lifecycle.load_contract(path)

        contract = lifecycle.expected_contract()
        with (
            mock.patch.object(lifecycle.os, "geteuid", return_value=0),
            mock.patch.object(
                lifecycle,
                "_regular_bytes",
                side_effect=lifecycle.LifecycleError("marker unavailable"),
            ),
            self.assertRaisesRegex(lifecycle.LifecycleError, "marker unavailable"),
        ):
            lifecycle._preflight(
                contract,
                baseline=Path("/baseline"),
                candidate=Path("/candidate"),
                evidence=Path("/evidence"),
                tools={},
            )

    def test_os_release_symlink_reads_the_usr_lib_file_ubuntu_ships(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "etc").mkdir()
            (root / "usr/lib").mkdir(parents=True)
            (root / "usr/lib/os-release").write_text(
                'ID=ubuntu\nVERSION_ID="26.04"\n', encoding="utf-8"
            )
            (root / "etc/os-release").symlink_to("../usr/lib/os-release")
            fields = lifecycle._load_os_release(root / "etc/os-release")
            self.assertEqual(fields["ID"], "ubuntu")
            self.assertEqual(fields["VERSION_ID"], "26.04")
            (root / "usr/lib/os-release").unlink()
            (root / "usr/lib/os-release").symlink_to("/elsewhere")
            with self.assertRaises(lifecycle.LifecycleError):
                lifecycle._load_os_release(root / "etc/os-release")

    def test_remove_keeps_config_files_but_purge_must_not(self):
        def query(status):
            return lifecycle.subprocess.CompletedProcess(
                ["dpkg-query"], 0, status.encode(), b""
            )
        tools = {"dpkg-query": "/usr/bin/dpkg-query"}
        with mock.patch.object(lifecycle, "_run", return_value=query("config-files")):
            lifecycle._require_removed(tools, keep_config=True)
            with self.assertRaises(lifecycle.LifecycleError):
                lifecycle._require_removed(tools)
        with mock.patch.object(lifecycle, "_run", return_value=query("installed")):
            with self.assertRaises(lifecycle.LifecycleError):
                lifecycle._require_removed(tools, keep_config=True)

    def test_package_set_rejects_wrong_order_and_unsafe_archive_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = {
                "architecture": "amd64",
                "version": "0.2.0-1",
                "packages": [
                    {"package": "rmac-session", "filename": "session.deb"},
                    {"package": "rmac-apps", "filename": "apps.deb"},
                ],
            }
            (root / "native-packages.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )
            with self.assertRaisesRegex(lifecycle.LifecycleError, "identity"):
                lifecycle._package_set(root)

            manifest["packages"].reverse()
            manifest["packages"][0]["filename"] = "../apps.deb"
            (root / "native-packages.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )
            _, _, loaded = lifecycle._package_set(root)
            with self.assertRaisesRegex(lifecycle.LifecycleError, "archive identity"):
                lifecycle._archives(root, loaded)

    def test_commands_are_argument_separated_and_output_bounded(self):
        completed = lifecycle.subprocess.CompletedProcess(
            [], 0, stdout=b"installed\t0.2.0-1", stderr=b""
        )
        with mock.patch.object(
            lifecycle.subprocess, "run", return_value=completed
        ) as run:
            result = lifecycle._run(
                ["dpkg-query", "-W", "rmac-session"],
                tools={"dpkg-query": "/usr/bin/dpkg-query"},
            )
        self.assertIs(result, completed)
        command = run.call_args.args[0]
        self.assertEqual(
            command, ["/usr/bin/dpkg-query", "-W", "rmac-session"]
        )
        self.assertIs(run.call_args.kwargs["stdin"], lifecycle.subprocess.DEVNULL)

        excessive = lifecycle.subprocess.CompletedProcess(
            [], 0, stdout=b"x" * (lifecycle.MAX_TOOL_OUTPUT_BYTES + 1), stderr=b""
        )
        with (
            mock.patch.object(lifecycle.subprocess, "run", return_value=excessive),
            self.assertRaisesRegex(lifecycle.LifecycleError, "excessive output"),
        ):
            lifecycle._run(["dpkg"], tools={"dpkg": "/usr/bin/dpkg"})

    def test_installed_application_payload_failure_fails_lifecycle(self):
        verifier = mock.Mock()
        verifier.verify_installed_host.side_effect = ValueError("payload mismatch")
        with mock.patch.object(
            lifecycle, "_load_script", return_value=verifier
        ) as load_script:
            with self.assertRaisesRegex(
                lifecycle.LifecycleError, "installed application payload check failed"
            ):
                lifecycle._verify_application_host()
        load_script.assert_called_once_with(
            "rmac_lifecycle_application", "verify-application-package.py"
        )
        verifier.verify_installed_host.assert_called_once_with(Path("/"))

    def test_installed_checkpoint_invokes_application_payload_verifier(self):
        native = mock.Mock()
        session = mock.Mock()
        with (
            mock.patch.object(
                lifecycle, "_load_script", side_effect=[native, session]
            ),
            mock.patch.object(lifecycle, "_require_versions"),
            mock.patch.object(
                lifecycle,
                "_verify_application_host",
                side_effect=lifecycle.LifecycleError("app payload changed"),
            ) as verify_app,
            self.assertRaisesRegex(lifecycle.LifecycleError, "app payload changed"),
        ):
            lifecycle._verify_installed(
                Path("/candidate"),
                ("0.9.0~beta.1-38", "amd64", {"packages": []}),
                tools={"dpkg-deb": "/usr/bin/dpkg-deb"},
            )
        native.verify_directory.assert_called_once()
        verify_app.assert_called_once_with()
        session.verify_installed_host.assert_not_called()


if __name__ == "__main__":
    unittest.main()
