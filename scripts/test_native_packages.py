"""Focused fixture tests for the H2 native Debian package contract."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest import mock


LINUX_SCRIPTS = Path(__file__).parent / "linux"
sys.path.insert(0, str(LINUX_SCRIPTS))

import native_package_contract as contract


def load_script(name: str, filename: str):
    path = LINUX_SCRIPTS / filename
    specification = importlib.util.spec_from_file_location(name, path)
    assert specification is not None and specification.loader is not None
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


builder = load_script("build_native_packages", "build-native-packages.py")


def write_elf(path: Path, machine: int, *, executable_type: int = 3) -> None:
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HH", header, 16, executable_type, machine)
    path.write_bytes(bytes(header))
    path.chmod(0o755)


def populate_binary_directory(directory: Path, machine: int) -> None:
    for name in contract.ALL_BINARIES:
        write_elf(directory / name, machine)


class NativePackageContractTests(unittest.TestCase):
    def test_inventory_covers_apps_and_supervised_session_exactly(self):
        self.assertEqual(len(contract.APPLICATION_BINARIES), 7)
        self.assertEqual(len(contract.SESSION_BINARIES), 18)
        self.assertEqual(len(contract.ALL_BINARIES), 23)
        self.assertEqual(
            set(contract.ALL_BINARIES),
            set(contract.APPLICATION_BINARIES) | set(contract.SESSION_BINARIES),
        )
        self.assertEqual([spec.name for spec in contract.PACKAGE_SPECS], [
            "rmac-apps",
            "rmac-session",
        ])
        apps = contract.PACKAGE_SPECS[0]
        self.assertIn("packagekit", apps.static_dependencies)
        self.assertNotIn("packagekit-tools", apps.static_dependencies)
        session = contract.PACKAGE_SPECS[1]
        self.assertIn("rmac-apps (= {version})", session.static_dependencies)
        self.assertIn("niri", session.static_dependencies)
        self.assertIn("swaylock", session.static_dependencies)
        self.assertIn("libpam0g", session.static_dependencies)
        self.assertIn("swayidle", session.static_dependencies)
        self.assertNotIn("brightnessctl", session.static_dependencies)
        self.assertIn("pipewire-bin", session.static_dependencies)
        self.assertIn("rmac-wallpaper", session.binaries)
        self.assertIn("rmac-top-bar", session.binaries)
        self.assertIn("rmac-dock", session.binaries)
        self.assertIn("rmac-osd", session.binaries)

    def test_accepts_exact_amd64_and_arm64_elf_inventories(self):
        for architecture, machine in contract.ARCHITECTURES.items():
            with self.subTest(architecture=architecture):
                with tempfile.TemporaryDirectory() as temporary:
                    directory = Path(temporary)
                    populate_binary_directory(directory, machine)
                    records = contract.validate_binary_directory(
                        directory, architecture
                    )
                    self.assertEqual(tuple(records), contract.ALL_BINARIES)
                    self.assertTrue(
                        all(record.architecture == architecture for record in records.values())
                    )
                    self.assertTrue(
                        all(len(record.sha256) == 64 for record in records.values())
                    )

    def test_rejects_wrong_architecture_non_elf_links_and_inventory_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            populate_binary_directory(directory, contract.ARCHITECTURES["amd64"])
            write_elf(
                directory / contract.ALL_BINARIES[0],
                contract.ARCHITECTURES["arm64"],
            )
            with self.assertRaisesRegex(
                contract.ContractError, "architecture does not match"
            ):
                contract.validate_binary_directory(directory, "amd64")

            write_elf(
                directory / contract.ALL_BINARIES[0],
                contract.ARCHITECTURES["amd64"],
            )
            target = directory / contract.ALL_BINARIES[1]
            target.unlink()
            target.symlink_to(directory / contract.ALL_BINARIES[0])
            with self.assertRaisesRegex(contract.ContractError, "not a regular file"):
                contract.validate_binary_directory(directory, "amd64")

            target.unlink()
            write_elf(target, contract.ARCHITECTURES["amd64"])
            (directory / "unreviewed-binary").write_bytes(b"extra")
            with self.assertRaisesRegex(
                contract.ContractError, "inventory is not exact"
            ):
                contract.validate_binary_directory(directory, "amd64")

    def test_dependency_and_control_metadata_are_canonical(self):
        spec = contract.PACKAGE_SPECS[1]
        shared = contract.dependency_entries(
            "libgcc-s1 (>= 3.0), libc6 (>= 2.38), libc6 (>= 2.38)"
        )
        self.assertEqual(shared, ("libc6 (>= 2.38)", "libgcc-s1 (>= 3.0)"))
        combined = contract.combined_dependencies(spec, "0.1.0-1", shared)
        self.assertEqual(combined, tuple(sorted(set(combined))))
        self.assertIn("rmac-apps (= 0.1.0-1)", combined)
        control = contract.control_bytes(
            spec,
            version="0.1.0-1",
            architecture="arm64",
            dependencies=combined,
        ).decode("utf-8")
        self.assertIn("Package: rmac-session\n", control)
        self.assertIn("Architecture: arm64\n", control)
        self.assertIn("Recommends: gdm3\n", control)
        self.assertTrue(control.endswith("\n"))
        with self.assertRaisesRegex(contract.ContractError, "unsupported relation"):
            contract.dependency_entries("libc6; touch /tmp/not-allowed")

    def test_source_epoch_and_workspace_version_are_strict(self):
        self.assertEqual(contract.source_date_epoch("0"), 0)
        self.assertEqual(contract.source_date_epoch(1_700_000_000), 1_700_000_000)
        for invalid in (None, True, "-1", "01", "1.0", " 1"):
            with self.subTest(value=invalid):
                with self.assertRaises(contract.ContractError):
                    contract.source_date_epoch(invalid)
        self.assertEqual(contract.native_version(builder.REPO_ROOT), "0.1.0-21")

    def test_shlibdeps_is_argument_separated_and_uses_clean_native_context(self):
        with tempfile.TemporaryDirectory() as temporary:
            working = Path(temporary) / "analysis"
            binary = Path(temporary) / "rmac-files"
            write_elf(binary, contract.ARCHITECTURES["amd64"])
            captured = {}

            def fake_run(command, **kwargs):
                captured["command"] = command
                captured.update(kwargs)
                return builder.subprocess.CompletedProcess(
                    command,
                    0,
                    stdout=b"shlibs:Depends=libc6 (>= 2.38), libgcc-s1 (>= 3.0)\n",
                    stderr=b"",
                )

            with mock.patch.object(builder, "_run", side_effect=fake_run):
                dependencies = builder.derive_shared_library_dependencies(
                    dpkg_shlibdeps="/usr/bin/dpkg-shlibdeps",
                    spec=contract.PACKAGE_SPECS[0],
                    binaries=(binary,),
                    architecture="amd64",
                    epoch=1_700_000_000,
                    working=working,
                    base_environment={
                        "PATH": "/usr/bin",
                        "LD_LIBRARY_PATH": "/private/untrusted",
                    },
                )
            self.assertEqual(
                dependencies, ("libc6 (>= 2.38)", "libgcc-s1 (>= 3.0)")
            )
            self.assertEqual(
                captured["command"],
                [
                    "/usr/bin/dpkg-shlibdeps",
                    "-O",
                    "--package=rmac-apps",
                    f"-e{binary}",
                ],
            )
            self.assertNotIn("LD_LIBRARY_PATH", captured["environment"])
            self.assertEqual(captured["environment"]["DEB_HOST_ARCH"], "amd64")
            self.assertEqual(
                captured["environment"]["SOURCE_DATE_EPOCH"], "1700000000"
            )
            self.assertTrue((working / "debian/control").is_file())

    def test_shlibdeps_rejects_empty_or_ambiguous_output(self):
        for output in (
            b"shlibs:Depends=\n",
            b"warning\nshlibs:Depends=libc6 (>= 2.38)\n",
        ):
            with self.subTest(output=output):
                with tempfile.TemporaryDirectory() as temporary:
                    with mock.patch.object(
                        builder,
                        "_run",
                        return_value=builder.subprocess.CompletedProcess(
                            [], 0, stdout=output, stderr=b""
                        ),
                    ):
                        with self.assertRaises(builder.PackageBuildError):
                            builder.derive_shared_library_dependencies(
                                dpkg_shlibdeps="/usr/bin/dpkg-shlibdeps",
                                spec=contract.PACKAGE_SPECS[0],
                                binaries=(Path(temporary) / "binary",),
                                architecture="amd64",
                                epoch=0,
                                working=Path(temporary) / "analysis",
                                base_environment={},
                            )


if __name__ == "__main__":
    unittest.main()
