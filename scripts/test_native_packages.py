"""Focused fixture tests for the H2 native Debian package contract."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


LINUX_SCRIPTS = Path(__file__).parent / "linux"
sys.path.insert(0, str(LINUX_SCRIPTS))

import apt_archive
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
package_verifier = load_script("verify_native_packages", "verify-native-packages.py")


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
    def test_checksum_manifest_rejects_duplicate_archive_records(self):
        line = "a" * 64 + "  rmac-apps_0.9.0~beta.1-38_amd64.deb\n"
        package_verifier._verify_checksum_manifest(line.encode("ascii"), [line])
        with self.assertRaisesRegex(
            package_verifier.VerificationError,
            "checksum manifest differs",
        ):
            package_verifier._verify_checksum_manifest(
                (line + line).encode("ascii"), [line]
            )

    def test_build_host_path_scan_rejects_home_locations_across_chunks(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "binary"
            binary.write_bytes(
                b"x" * (64 * 1024 - 9)
                + b"\0/home/alice/project/src/main.rs\0"
            )
            with self.assertRaisesRegex(
                package_verifier.VerificationError,
                "contains a build-host home path",
            ):
                package_verifier._scan_binary_for_build_host_home(
                    binary, package_verifier.MAX_PACKAGE_SET_SCAN_BYTES
                )

            binary.write_bytes(b"debug text mentions /home/ and /Users/ only")
            scanned = package_verifier._scan_binary_for_build_host_home(
                binary, package_verifier.MAX_PACKAGE_SET_SCAN_BYTES
            )
            self.assertEqual(scanned, binary.stat().st_size)

            # Literals are packed back to back, so a leaked checkout path can
            # directly follow other text (SR-15).
            binary.write_bytes(
                b"/usr/share/icons/hicolor/scalable/apps/home/runner/work/lulo/shell\0"
            )
            with self.assertRaisesRegex(
                package_verifier.VerificationError,
                "contains a build-host home path",
            ):
                package_verifier._scan_binary_for_build_host_home(
                    binary, package_verifier.MAX_PACKAGE_SET_SCAN_BYTES
                )

            binary.write_bytes(b"location: /Users/alice/checkout/rmac/src/lib.rs\0")
            with self.assertRaisesRegex(
                package_verifier.VerificationError,
                "contains a build-host home path",
            ):
                package_verifier._scan_binary_for_build_host_home(
                    binary, package_verifier.MAX_PACKAGE_SET_SCAN_BYTES
                )

    def test_build_host_path_scan_rejects_symlink_and_exhausted_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "binary"
            binary.write_bytes(b"ordinary executable bytes")
            link = Path(temporary) / "linked-binary"
            link.symlink_to(binary)
            with self.assertRaisesRegex(
                package_verifier.VerificationError, "not a regular file"
            ):
                package_verifier._scan_binary_for_build_host_home(link, 100)
            with self.assertRaisesRegex(
                package_verifier.VerificationError, "exceeds its size limit"
            ):
                package_verifier._scan_binary_for_build_host_home(
                    binary, binary.stat().st_size - 1
                )

    def test_native_build_remaps_checkout_and_cargo_home_paths(self):
        script = (LINUX_SCRIPTS / "build-native-inputs.sh").read_text(
            encoding="utf-8"
        )
        block = script.split('cargo_home="${CARGO_HOME:-$HOME/.cargo}"', 1)[1]
        block = 'cargo_home="${CARGO_HOME:-$HOME/.cargo}"' + block.split("\nfi\n", 1)[0] + "\nfi\n"
        environment = {**os.environ, "HOME": "/build/user", "CARGO_HOME": "/build/cargo"}
        environment.pop("CARGO_ENCODED_RUSTFLAGS", None)
        environment["RUSTFLAGS"] = "-Cdebuginfo=0"
        plain = subprocess.run(
            ["bash", "-c", f"repo_root=/build/repo\n{block}\nprintf '%s' \"$RUSTFLAGS\""],
            env=environment, capture_output=True, check=True,
        )
        self.assertEqual(
            plain.stdout.decode(),
            "-Cdebuginfo=0 --remap-path-prefix=/build/repo=/rmac "
            "--remap-path-prefix=/build/cargo=/cargo",
        )
        environment["CARGO_ENCODED_RUSTFLAGS"] = "-Cdebuginfo=0"
        encoded = subprocess.run(
            ["bash", "-c", f"repo_root=/build/repo\n{block}\nprintf '%s' \"$CARGO_ENCODED_RUSTFLAGS\""],
            env=environment, capture_output=True, check=True,
        )
        self.assertEqual(
            encoded.stdout,
            b"-Cdebuginfo=0\x1f--remap-path-prefix=/build/repo=/rmac"
            b"\x1f--remap-path-prefix=/build/cargo=/cargo",
        )

    def test_native_build_accepts_release_and_iterate_profiles_only(self):
        script = LINUX_SCRIPTS / "build-native-inputs.sh"
        with tempfile.TemporaryDirectory() as temporary:
            # An unknown profile is refused immediately, before any
            # environment check (Linux, cargo, dpkg...) or destructive
            # operation, on every host this test might run on.
            rejected = subprocess.run(
                [
                    "bash",
                    str(script),
                    "--output",
                    f"{temporary}/fresh",
                    "--profile",
                    "bogus",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(rejected.returncode, 1)
            self.assertIn("unsupported cargo profile: bogus", rejected.stderr)

            # A known profile clears that gate. Point --output at a directory
            # that already exists so the script's own "must not already
            # exist" refusal stops it deterministically, on any host,
            # strictly before the real `cargo build` invocation -- this test
            # must never trigger an actual compile.
            existing = Path(temporary) / "already-there"
            existing.mkdir()
            for profile in ("release", "iterate"):
                with self.subTest(profile=profile):
                    accepted = subprocess.run(
                        [
                            "bash",
                            str(script),
                            "--output",
                            str(existing),
                            "--profile",
                            profile,
                        ],
                        capture_output=True,
                        text=True,
                    )
                    self.assertNotEqual(accepted.returncode, 0)
                    self.assertNotIn("unsupported cargo profile", accepted.stderr)

            # The default (no --profile) behaves exactly like --profile release.
            defaulted = subprocess.run(
                ["bash", str(script), "--output", str(existing)],
                capture_output=True,
                text=True,
            )
            self.assertNotIn("unsupported cargo profile", defaulted.stderr)

    def test_native_build_strips_only_the_staged_copy_for_a_candidate_profile(self):
        # Extracted straight from the script: the staging loop that copies
        # each binary and, only for a non-release profile, strips the staged
        # copy (never the shared target directory's own binary -- see
        # test_native_build_remaps_checkout_and_cargo_home_paths for the same
        # text-extraction approach).
        script = (LINUX_SCRIPTS / "build-native-inputs.sh").read_text(
            encoding="utf-8"
        )
        anchor = 'install -m 0755 "$(binary_source "$name")" "$staging/$name"'
        loop_body = anchor + script.split(anchor, 1)[1].split("\ndone\n", 1)[0]
        snippet = f'for name in "${{binary_names[@]}}"; do\n  {loop_body}\ndone\n'

        with tempfile.TemporaryDirectory() as temporary:
            bin_dir = Path(temporary) / "bin"
            bin_dir.mkdir()
            source_dir = Path(temporary) / "source"
            source_dir.mkdir()
            staging = Path(temporary) / "staging"
            staging.mkdir()
            (source_dir / "rmac-files").write_bytes(b"not really an elf")

            strip_log = Path(temporary) / "strip.log"
            fake_strip = bin_dir / "strip"
            fake_strip.write_text(
                "#!/bin/sh\n"
                f'printf \'%s\\n\' "$*" >> "{strip_log}"\n'
            )
            fake_strip.chmod(0o755)

            for profile, should_strip in (("release", False), ("iterate", True)):
                with self.subTest(profile=profile):
                    strip_log.unlink(missing_ok=True)
                    script_text = (
                        f'binary_names=(rmac-files)\n'
                        f'profile={profile}\n'
                        f'staging="{staging}"\n'
                        f'binary_source() {{ printf \'%s\\n\' "{source_dir}/rmac-files"; }}\n'
                        f"{snippet}"
                    )
                    result = subprocess.run(
                        ["bash", "-c", script_text],
                        env={**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}"},
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertTrue((staging / "rmac-files").exists())
                    if should_strip:
                        self.assertEqual(
                            strip_log.read_text(),
                            f"--strip-all {staging}/rmac-files\n",
                        )
                    else:
                        self.assertFalse(strip_log.exists())

    def test_shipping_shell_hosts_are_explicit_and_runtime_backed(self):
        self.assertEqual(
            set(contract.SHIPPING_SHELL_SOURCES),
            {
                "rmac-wallpaper",
                "rmac-top-bar",
                "rmac-dock",
                "rmac-osd",
                "rmac-app-switcher",
                "rmac-screenshot",
                "rmac-mission-control",
            },
        )
        self.assertTrue(
            set(contract.SHIPPING_SHELL_SOURCES).issubset(contract.SESSION_BINARIES)
        )
        manifest = (Path(__file__).parents[1] / "shell" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        for _, runtime_crates in contract.SHIPPING_SHELL_SOURCES.values():
            for crate in runtime_crates:
                self.assertIn(f'{crate} = {{ version = "=0.9.0-beta.1",', manifest)

    def test_inventory_covers_apps_and_supervised_session_exactly(self):
        self.assertEqual(len(contract.APPLICATION_BINARIES), 15)
        self.assertEqual(len(contract.SESSION_BINARIES), 28)
        self.assertEqual(len(contract.ALL_BINARIES), 41)
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
        self.assertIn("wl-clipboard", apps.static_dependencies)
        session = contract.PACKAGE_SPECS[1]
        self.assertIn("rmac-apps (= {version})", session.static_dependencies)
        self.assertIn(contract._third_party_floor("niri"), session.static_dependencies)
        self.assertIn(
            contract._third_party_floor("xwayland-satellite"), session.static_dependencies
        )
        self.assertNotIn("niri", session.static_dependencies)
        self.assertIn("swaylock", session.static_dependencies)
        self.assertIn("wl-clipboard", session.static_dependencies)
        self.assertNotIn("libpam0g", session.static_dependencies)
        self.assertIn("swayidle", session.static_dependencies)
        self.assertNotIn("brightnessctl", session.static_dependencies)
        self.assertIn("pipewire-bin", session.static_dependencies)
        # rmac-update-check drives PackageKit through PackageKitGlib and
        # notifies over D-Bus; it no longer shells out to notify-send.
        self.assertIn("packagekit", session.static_dependencies)
        self.assertIn("gir1.2-packagekitglib-1.0", session.static_dependencies)
        self.assertIn("python3-gi", session.static_dependencies)
        self.assertNotIn("libnotify-bin", session.static_dependencies)
        self.assertNotIn("packagekit-tools", session.static_dependencies)
        self.assertIn("rmac-wallpaper", session.binaries)
        self.assertIn("rmac-top-bar", session.binaries)
        self.assertIn("rmac-dock", session.binaries)
        self.assertIn("rmac-osd", session.binaries)
        self.assertIn("rmac-app-switcher", session.binaries)
        self.assertIn("rmac-screenshot", session.binaries)
        self.assertIn("rmac-mission-control", session.binaries)
        self.assertIn("grim", session.static_dependencies)
        self.assertIn("wl-clipboard", session.static_dependencies)
        self.assertIn("rmac-clipboard-service", session.binaries)
        self.assertIn("rmac-lock-provider", session.binaries)
        self.assertIn("rmac-sound", session.binaries)
        self.assertIn("rmac-mac-keyboard", session.binaries)
        self.assertIn("rmac-setup-assistant", session.binaries)
        self.assertIn("rmac-calendar-agent", session.binaries)
        self.assertEqual(session.maintainer_scripts, ("postinst", "postrm"))
        scripts = contract.maintainer_scripts(Path(__file__).parents[1], session)
        self.assertIn(b"\"$helper\" regenerate", scripts["postinst"])
        self.assertIn(b"/etc/keyd/rmac.conf", scripts["postrm"])

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
        self.assertIn("Recommends: gdm3, keyd, pkexec, qt6-gtk-platformtheme\n", control)
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
        self.assertEqual(contract.native_version(builder.REPO_ROOT), "0.9.0~beta.1-38")
        self.assertEqual(contract.debian_upstream_version("0.9.0-beta.1"), "0.9.0~beta.1")
        self.assertEqual(contract.debian_upstream_version("0.9.0"), "0.9.0")

    def test_candidate_build_metadata_tags_the_version_without_disturbing_a_release(self):
        # A real Beta/stable release (no build_metadata) is byte-for-byte what
        # it always was.
        self.assertEqual(contract.native_version(builder.REPO_ROOT), "0.9.0~beta.1-38")
        # A candidate build's cargo profile name becomes "+<profile>" build
        # metadata ahead of the Debian revision, so the rmac-apps/rmac-session
        # version pair still matches each other and dpkg still orders the
        # version, while the filename and manifest are visibly a candidate.
        self.assertEqual(
            contract.native_version(builder.REPO_ROOT, build_metadata="iterate"),
            "0.9.0~beta.1+iterate-38",
        )
        for invalid in ("", "Iterate", "iterate-1", "iter ate", "iter+ate", "ÿterate"):
            with self.subTest(value=invalid):
                with self.assertRaisesRegex(contract.ContractError, "build metadata"):
                    contract.native_version(builder.REPO_ROOT, build_metadata=invalid)

    def test_control_bytes_accepts_candidate_build_metadata_versions(self):
        spec = contract.PACKAGE_SPECS[1]
        version = "0.9.0~beta.1+iterate-38"
        combined = contract.combined_dependencies(spec, version, ())
        self.assertIn("rmac-apps (= 0.9.0~beta.1+iterate-38)", combined)
        control = contract.control_bytes(
            spec,
            version=version,
            architecture="amd64",
            dependencies=combined,
        ).decode("utf-8")
        self.assertIn(f"Version: {version}\n", control)
        with self.assertRaisesRegex(contract.ContractError, "version is invalid"):
            contract.control_bytes(
                spec,
                version="0.9.0~beta.1+Iterate-38",
                architecture="amd64",
                dependencies=combined,
            )

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

    def test_build_cli_threads_build_metadata_into_the_package_build(self):
        captured = {}

        def fake_build(**kwargs):
            captured.update(kwargs)

        with mock.patch.object(builder, "build", side_effect=fake_build):
            with mock.patch(
                "sys.argv",
                [
                    "build-native-packages.py",
                    "--binary-dir",
                    "/tmp/binaries",
                    "--output",
                    "/tmp/output",
                    "--architecture",
                    "amd64",
                    "--source-date-epoch",
                    "0",
                    "--build-metadata",
                    "iterate",
                ],
            ):
                self.assertEqual(builder.main(), 0)
        self.assertEqual(captured["build_metadata"], "iterate")

        captured.clear()
        with mock.patch.object(builder, "build", side_effect=fake_build):
            with mock.patch(
                "sys.argv",
                [
                    "build-native-packages.py",
                    "--binary-dir",
                    "/tmp/binaries",
                    "--output",
                    "/tmp/output",
                    "--architecture",
                    "amd64",
                    "--source-date-epoch",
                    "0",
                ],
            ):
                self.assertEqual(builder.main(), 0)
        self.assertIsNone(captured["build_metadata"])

    def test_verify_cli_derives_the_candidate_version_from_build_metadata(self):
        captured = {}

        def fake_verify_directory(directory, **kwargs):
            captured.update(kwargs)

        with mock.patch.object(
            package_verifier, "verify_directory", side_effect=fake_verify_directory
        ):
            with mock.patch(
                "sys.argv",
                [
                    "verify-native-packages.py",
                    "--directory",
                    "/tmp/output",
                    "--architecture",
                    "amd64",
                    "--build-metadata",
                    "iterate",
                ],
            ):
                with mock.patch.object(
                    package_verifier, "_require_dpkg_deb", return_value="/usr/bin/dpkg-deb"
                ):
                    self.assertEqual(package_verifier.main(), 0)
        self.assertEqual(
            captured["expected_version"], "0.9.0~beta.1+iterate-38"
        )

        # An explicit --version still wins over --build-metadata.
        captured.clear()
        with mock.patch.object(
            package_verifier, "verify_directory", side_effect=fake_verify_directory
        ):
            with mock.patch(
                "sys.argv",
                [
                    "verify-native-packages.py",
                    "--directory",
                    "/tmp/output",
                    "--architecture",
                    "amd64",
                    "--build-metadata",
                    "iterate",
                    "--version",
                    "9.9.9-1",
                ],
            ):
                with mock.patch.object(
                    package_verifier, "_require_dpkg_deb", return_value="/usr/bin/dpkg-deb"
                ):
                    self.assertEqual(package_verifier.main(), 0)
        self.assertEqual(captured["expected_version"], "9.9.9-1")



NOTES = (
    "Lulo OS 0.9.1 makes updates feel like the Mac.\n"
    "\n"
    "# Software Update\n"
    "Update Lulo OS from System Settings, with release notes.\n"
    "\n"
    "# Fixes\n"
    "The Dock no longer flickers.\n"
)


class ReleaseNotesFieldTests(unittest.TestCase):
    """rmac-session's signed Lulo-Release-Notes control field."""

    def control(self, root: Path, spec_index: int = 1, version: str = "0.9.1-38") -> str:
        spec = contract.PACKAGE_SPECS[spec_index]
        return contract.control_bytes(
            spec,
            version=version,
            architecture="amd64",
            dependencies=contract.combined_dependencies(spec, version, ()),
            notes_root=root,
        ).decode("utf-8")

    def write_notes(self, root: Path, name: str, raw: bytes) -> Path:
        directory = root / contract.RELEASE_NOTES_DIRECTORY
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / name
        path.write_bytes(raw)
        return path

    def test_specs_order_is_what_these_tests_assume(self):
        self.assertEqual(contract.PACKAGE_SPECS[1].name, "rmac-session")
        self.assertEqual(contract.PACKAGE_SPECS[0].name, "rmac-apps")

    def test_field_is_absent_without_a_notes_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.assertNotIn(contract.RELEASE_NOTES_FIELD, self.control(Path(temporary)))

    def test_field_is_encoded_as_deb822_continuation_lines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_notes(root, "0.9.1.txt", NOTES.encode("utf-8"))
            control = self.control(root)
        self.assertIn(
            "Lulo-Release-Notes:\n"
            " Lulo OS 0.9.1 makes updates feel like the Mac.\n"
            " .\n"
            " # Software Update\n"
            " Update Lulo OS from System Settings, with release notes.\n"
            " .\n"
            " # Fixes\n"
            " The Dock no longer flickers.\n"
            "Description: ",
            control,
        )

    def test_pre_release_notes_use_the_debian_upstream_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_notes(root, "0.9.0~beta.1.txt", b"Beta notes.\n")
            control = self.control(root, version="0.9.0~beta.1-38")
        self.assertIn("Lulo-Release-Notes:\n Beta notes.\n", control)

    def test_only_rmac_session_carries_the_field(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_notes(root, "0.9.1.txt", NOTES.encode("utf-8"))
            self.assertNotIn(contract.RELEASE_NOTES_FIELD, self.control(root, spec_index=0))

    def test_field_round_trips_through_the_archive_parser(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_notes(root, "0.9.1.txt", NOTES.encode("utf-8"))
            control = self.control(root)
        [paragraph] = apt_archive.parse_deb822(control, "control")
        value = paragraph[contract.RELEASE_NOTES_FIELD]
        decoded = "\n".join("" if line == "." else line for line in value.split("\n"))
        self.assertEqual(decoded + "\n", NOTES)
        # stage-apt-snapshot.py re-renders the paragraph with format_paragraph;
        # the result parses back to the same field value.
        rendered = apt_archive.format_paragraph(list(paragraph.items()))
        [again] = apt_archive.parse_deb822(rendered, "Packages")
        self.assertEqual(again[contract.RELEASE_NOTES_FIELD], value)
        self.assertEqual(again["Package"], "rmac-session")

    def test_invalid_notes_are_refused(self):
        cases = {
            "control character": b"Bad\tnotes\n",
            "carriage return": b"Bad\r\nnotes\n",
            "not UTF-8": b"\xff\xfe\n",
            "leading blank": b"\nNotes\n",
            "trailing blank": b"Notes\n\n",
            "doubled blank": b"One\n\n\nTwo\n",
            "trailing whitespace": b"Notes \n",
            "period line": b"Notes\n.\n",
            "period start": b"Notes\n.hidden\n",
            "empty": b"",
            "too large": b"x" * (contract.MAX_RELEASE_NOTES_BYTES + 1),
        }
        for label, raw in cases.items():
            with self.subTest(case=label):
                with self.assertRaises(contract.ContractError):
                    contract.validate_release_notes(raw)

    def test_notes_must_be_a_regular_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = self.write_notes(root, "target.txt", b"Notes\n")
            (root / contract.RELEASE_NOTES_DIRECTORY / "0.9.1.txt").symlink_to(target)
            with self.assertRaisesRegex(contract.ContractError, "regular file"):
                self.control(root)

    def test_committed_notes_are_valid(self):
        directory = contract.REPO_ROOT / contract.RELEASE_NOTES_DIRECTORY
        for path in sorted(directory.glob("*.txt")) if directory.is_dir() else ():
            with self.subTest(path=path.name):
                contract.validate_release_notes(path.read_bytes())


if __name__ == "__main__":
    unittest.main()
