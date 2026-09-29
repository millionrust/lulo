"""The private power runners must never reach the host systemctl."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


def load_runner(name: str):
    path = Path(__file__).parent / "behavior" / name
    spec = importlib.util.spec_from_file_location(name.removesuffix(".py"), path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class PrivatePowerGuardTests(unittest.TestCase):
    def test_each_runner_requires_its_own_fake_systemctl_first_on_path(self):
        for name in ("run_power_dialogs.py", "run_shutdown.py"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                module = load_runner(name)
                work = Path(directory)
                fakebin = work / "fakebin"
                fakebin.mkdir()
                fake = fakebin / "systemctl"
                fake.write_text(
                    f'#!/bin/sh\necho "$@" >> "{work / "systemctl-calls.log"}"\nexit 0\n',
                    encoding="utf-8",
                )
                fake.chmod(0o755)
                other = work / "other"
                other.mkdir()
                other_systemctl = other / "systemctl"
                other_systemctl.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                other_systemctl.chmod(0o755)
                runtime = work / "runtime"
                runtime.mkdir()
                common = {"XDG_RUNTIME_DIR": str(runtime), "WAYLAND_DISPLAY": ""}
                with mock.patch.dict(os.environ, {**common, "PATH": str(other)}):
                    with self.assertRaisesRegex(SystemExit, "fake systemctl is not first"):
                        module.Run(SimpleNamespace(), work)
                with mock.patch.dict(os.environ, {**common, "PATH": f"{fakebin}:{other}"}):
                    runner = module.Run(SimpleNamespace(), work)
                self.assertEqual(runner.systemctl_calls(), [])

    def test_each_runner_rejects_fake_systemctl_symlink_to_another_executable(self):
        for name in ("run_power_dialogs.py", "run_shutdown.py"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                module = load_runner(name)
                work = Path(directory)
                fakebin = work / "fakebin"
                fakebin.mkdir()
                other = work / "other"
                other.mkdir()
                target = other / "systemctl"
                target.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                target.chmod(0o755)
                (fakebin / "systemctl").symlink_to(target)
                runtime = work / "runtime"
                runtime.mkdir()
                common = {"XDG_RUNTIME_DIR": str(runtime), "WAYLAND_DISPLAY": ""}
                with mock.patch.dict(os.environ, {**common, "PATH": f"{fakebin}:{other}"}):
                    with self.assertRaisesRegex(SystemExit, "fake systemctl is not first"):
                        module.Run(SimpleNamespace(), work)

    def test_each_runner_rejects_an_unrelated_regular_executable_in_fakebin(self):
        for name in ("run_power_dialogs.py", "run_shutdown.py"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                module = load_runner(name)
                work = Path(directory)
                fakebin = work / "fakebin"
                fakebin.mkdir()
                fake = fakebin / "systemctl"
                fake.write_text("#!/bin/sh\nexec /usr/bin/systemctl \"$@\"\n", encoding="utf-8")
                fake.chmod(0o755)
                runtime = work / "runtime"
                runtime.mkdir()
                common = {"XDG_RUNTIME_DIR": str(runtime), "WAYLAND_DISPLAY": ""}
                with mock.patch.dict(os.environ, {**common, "PATH": str(fakebin)}):
                    with self.assertRaisesRegex(SystemExit, "fake systemctl is not first"):
                        module.Run(SimpleNamespace(), work)


if __name__ == "__main__":
    unittest.main()
