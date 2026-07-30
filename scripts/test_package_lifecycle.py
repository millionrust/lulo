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


if __name__ == "__main__":
    unittest.main()
