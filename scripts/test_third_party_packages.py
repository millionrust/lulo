"""Contract tests for Lulo OS's own niri and xwayland-satellite packages."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "linux"))

import native_package_contract as native  # noqa: E402
import third_party_packages as third_party  # noqa: E402


PACKAGING = REPO_ROOT / "packaging" / "third-party"
BUILD_SCRIPT = REPO_ROOT / "scripts" / "linux" / "build-niri-packages.sh"


def _control(name: str) -> str:
    return (PACKAGING / name / "debian" / "control").read_text(encoding="utf-8")


def _field(control: str, field: str) -> str:
    match = re.search(rf"(?ms)^{field}:(.*?)(?=^\S|\Z)", control)
    assert match, field
    return " ".join(match.group(1).split())


class PinTests(unittest.TestCase):
    def test_pins_name_the_tested_upstream_releases_exactly(self):
        pins = third_party.load_pins(REPO_ROOT)
        niri = pins["niri"]
        self.assertEqual(niri.tag, "v26.04")
        self.assertEqual(niri.commit, "8ed0da44d974c32c6877d2f4630c314da0717ecb")
        self.assertEqual(
            niri.tarball_sha256,
            "134c602d8e0d53413a52d6cd58f9ce7e79a07d03288ee0a51ba1abd5db1b1ad9",
        )
        self.assertEqual(niri.license, "GPL-3.0-or-later")
        satellite = pins["xwayland-satellite"]
        self.assertEqual(satellite.tag, "v0.8.2")
        self.assertEqual(satellite.commit, "8d135d3b2854b30fd01ea6cd6c27e523dd50a839")
        self.assertEqual(
            satellite.tarball_sha256,
            "cb50bb6948582d5ec3aa511d2d66ad622989bb14bef94e3bb81bae8b64c120b1",
        )
        self.assertEqual(satellite.license, "MPL-2.0")

    def test_debian_packaging_matches_the_pins(self):
        for name, pin in third_party.load_pins(REPO_ROOT).items():
            debian = PACKAGING / name / "debian"
            changelog = (debian / "changelog").read_text(encoding="utf-8")
            self.assertTrue(changelog.startswith(f"{name} ({pin.debian_version}) resolute;"))
            self.assertIn(pin.commit, changelog)
            copyright_text = (debian / "copyright").read_text(encoding="utf-8")
            self.assertIn(pin.commit, copyright_text)
            self.assertIn(pin.tarball_url, copyright_text)
            self.assertIn(f"License: {pin.license}\n", copyright_text)
            self.assertEqual(
                (debian / "source" / "format").read_text(encoding="utf-8"), "3.0 (quilt)\n"
            )
            control = _control(name)
            self.assertIn(f"Source: {name}\n", control)
            self.assertIn(f"Package: {name}\n", control)
            self.assertIn("Rules-Requires-Root: no\n", control)
            self.assertIn(f"Maintainer: {native.MAINTAINER}\n", control)
            self.assertIn("${shlibs:Depends}", _field(control, "Depends"))
            rules = debian / "rules"
            self.assertTrue(rules.stat().st_mode & 0o111, f"{rules} is not executable")
            rules_text = rules.read_text(encoding="utf-8")
            self.assertIn("cargo build --release --frozen", rules_text)
            self.assertIn("dpkg-shlibdeps", rules_text)
            self.assertIn("--remap-path-prefix=", rules_text)
            self.assertNotIn("debhelper", control)

    def test_niri_keeps_the_files_the_rmac_session_relies_on(self):
        rules = (PACKAGING / "niri" / "debian" / "rules").read_text(encoding="utf-8")
        self.assertIn("/usr/bin/niri-session", rules)
        self.assertIn("/usr/lib/systemd/user/niri.service", rules)
        self.assertIn("/usr/lib/systemd/user/niri-shutdown.target", rules)
        self.assertIn('"$(DESTDIR)/usr/bin/niri"', rules)
        self.assertIn("NIRI_BUILD_COMMIT", rules)
        session = (REPO_ROOT / "packaging" / "rmac-session" / "rmac-wayland-session").read_text(
            encoding="utf-8"
        )
        self.assertIn("/usr/bin/niri-session", session)
        self.assertIn("niri.service", session)

    def test_niri_depends_on_the_satellite_and_dlopened_libraries(self):
        depends = _field(_control("niri"), "Depends")
        self.assertIn("xwayland-satellite (>= 0.8.2)", depends)
        self.assertIn("libwayland-server0", depends)
        self.assertIn("libegl1", depends)
        self.assertIn("xwayland", _field(_control("xwayland-satellite"), "Depends"))

    def test_satellite_copyright_covers_the_embedded_font(self):
        text = (PACKAGING / "xwayland-satellite" / "debian" / "copyright").read_text(
            encoding="utf-8"
        )
        self.assertIn("Files: OpenSans-Regular.ttf", text)
        self.assertIn("License: OFL-1.1\n", text)
        self.assertIn("SIL OPEN FONT LICENSE Version 1.1", text)

    def test_pin_validation_rejects_a_tilde_revision_and_a_moved_tag(self):
        document = json.loads((PACKAGING / "upstreams.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "packaging" / "third-party").mkdir(parents=True)
            for field, value in (("debian_revision", "0~lulo1"), ("tag", "v26.08")):
                broken = json.loads(json.dumps(document))
                broken["packages"]["niri"][field] = value
                (root / third_party.PINS_PATH).write_text(json.dumps(broken), encoding="utf-8")
                with self.assertRaises(third_party.ThirdPartyError):
                    third_party.load_pins(root)


class VersionPolicyTests(unittest.TestCase):
    def assertOrdered(self, lower: str, higher: str) -> None:
        self.assertLess(third_party.compare_versions(lower, higher), 0, (lower, higher))
        self.assertGreater(third_party.compare_versions(higher, lower), 0, (higher, lower))

    def test_comparison_matches_dpkg_on_known_cases(self):
        self.assertOrdered("1.0~rc1", "1.0")
        self.assertOrdered("1.0", "1.0a")
        self.assertOrdered("1.9", "1.10")
        self.assertOrdered("1.0-0lulo2", "1.0-0lulo10")
        self.assertOrdered("9:1.0", "10:0.1")
        self.assertEqual(third_party.compare_versions("1.0-1", "1.0-1"), 0)

    def test_lulo_builds_satisfy_rmac_but_yield_to_ppa_and_official_packages(self):
        for pin in third_party.load_pins(REPO_ROOT).values():
            ours = pin.debian_version
            upstream = pin.upstream_version
            # rmac-session's "(>= upstream)" accepts our build...
            self.assertOrdered(upstream, ours)
            # ...but a PPA install already on the machine stays (no forced
            # downgrade), and a future official Debian/Ubuntu package wins.
            self.assertOrdered(ours, f"{upstream}ppa1")
            self.assertOrdered(ours, f"{upstream}-1")
            self.assertOrdered(ours, f"{upstream}-0ubuntu1")
            self.assertOrdered(ours, f"{upstream}-0.1")
            self.assertNotIn("~", ours)
            self.assertNotIn(":", ours)
        self.assertOrdered("26.04-0lulo1", "26.04ppa3")
        self.assertOrdered("0.8.2-0lulo1", "0.8.2ppa1")

    def test_rmac_session_floors_are_the_pinned_upstream_versions(self):
        session = next(spec for spec in native.PACKAGE_SPECS if spec.name == "rmac-session")
        for pin in third_party.load_pins(REPO_ROOT).values():
            relation = f"{pin.name} (>= {pin.upstream_version})"
            self.assertIn(relation, session.static_dependencies)
            # Every build that satisfies the floor: ours and the PPA's.
            for candidate in (pin.debian_version, f"{pin.upstream_version}ppa3"):
                self.assertGreaterEqual(
                    third_party.compare_versions(candidate, pin.upstream_version), 0
                )
        control = native.control_bytes(
            session,
            version="0.9.0~beta.1-38",
            architecture="amd64",
            dependencies=native.resolved_static_dependencies(session, "0.9.0~beta.1-38"),
        ).decode("utf-8")
        self.assertIn("niri (>= 26.04)", control)
        self.assertIn("xwayland-satellite (>= 0.8.2)", control)


class NoticeAndSbomTests(unittest.TestCase):
    LOCK = (
        "version = 4\n\n"
        '[[package]]\nname = "zeta"\nversion = "1.0.0"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        'checksum = "' + "a" * 64 + '"\n'
        'dependencies = [\n "alpha",\n]\n\n'
        '[[package]]\nname = "alpha"\nversion = "0.1.0"\n'
        'source = "git+https://github.com/Smithay/smithay.git?rev=ff5f#ff5f"\n\n'
        '[[package]]\nname = "niri"\nversion = "26.4.0"\n'
    )

    def test_cargo_lock_parser_reads_every_package(self):
        packages = third_party.parse_cargo_lock(self.LOCK)
        self.assertEqual([p["name"] for p in packages], ["alpha", "niri", "zeta"])
        self.assertEqual(packages[2]["checksum"], "a" * 64)

    def test_sbom_is_deterministic_and_binds_the_pins(self):
        pin = third_party.load_pins(REPO_ROOT)["niri"]
        packages = third_party.parse_cargo_lock(self.LOCK)
        first = third_party.cyclonedx_sbom(pin, packages, "b" * 64, {"niri.deb": "c" * 64})
        second = third_party.cyclonedx_sbom(pin, packages, "b" * 64, {"niri.deb": "c" * 64})
        self.assertEqual(json.dumps(first, sort_keys=True), json.dumps(second, sort_keys=True))
        component = first["metadata"]["component"]
        self.assertEqual(component["version"], pin.debian_version)
        self.assertEqual(component["licenses"], [{"license": {"id": "GPL-3.0-or-later"}}])
        properties = {item["name"]: item["value"] for item in component["properties"]}
        self.assertEqual(properties["lulo:upstream-commit"], pin.commit)
        self.assertEqual(properties["lulo:vendor-tarball-sha256"], "b" * 64)
        self.assertEqual(properties["lulo:artifact-sha256:niri.deb"], "c" * 64)
        names = [c["name"] for c in first["components"]]
        self.assertEqual(names, ["alpha", "niri", "zeta"])
        alpha = first["components"][0]
        self.assertEqual(alpha["externalReferences"][0]["type"], "vcs")

    def test_notices_include_every_crate_licence(self):
        with tempfile.TemporaryDirectory() as temporary:
            vendor = Path(temporary) / "vendor"
            crate = vendor / "alpha"
            crate.mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "alpha"\nversion = "0.1.0"\nlicense = "MIT OR Apache-2.0"\n',
                encoding="utf-8",
            )
            (crate / "LICENSE-MIT").write_text("MIT text\n", encoding="utf-8")
            (vendor / ".lulo-cargo-config.toml").write_text("", encoding="utf-8")
            text = third_party.dependency_notices(vendor, "heading")
            self.assertIn("======== alpha 0.1.0 ========", text)
            self.assertIn("License: MIT OR Apache-2.0", text)
            self.assertIn("MIT text", text)
            bare = vendor / "beta"
            bare.mkdir()
            (bare / "Cargo.toml").write_text(
                '[package]\nname = "beta"\nversion = "1.0.0"\n', encoding="utf-8"
            )
            with self.assertRaises(third_party.ThirdPartyError):
                third_party.dependency_notices(vendor, "heading")

    def test_shell_assignments_are_quoted_and_complete(self):
        pin = third_party.load_pins(REPO_ROOT)["xwayland-satellite"]
        text = third_party.shell_assignments(pin)
        self.assertIn("PIN_DEBIAN_VERSION='0.8.2-0lulo1'\n", text)
        self.assertIn("PIN_VENDOR_TARBALL='xwayland-satellite_0.8.2.orig-vendor.tar.xz'\n", text)
        for line in text.splitlines():
            self.assertRegex(line, r"^PIN_[A-Z0-9_]+='[^']*'$")


class BuildScriptTests(unittest.TestCase):
    def test_build_script_is_valid_bash_and_honours_the_disk_rules(self):
        result = subprocess.run(
            ["bash", "-n", str(BUILD_SCRIPT)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        text = BUILD_SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(BUILD_SCRIPT.stat().st_mode & 0o111)
        self.assertIn("minimum_free_gib=25", text)
        self.assertIn('df -Pk "$work_dir"', text)
        self.assertIn('"${HOME:-}/rmac-niri-build"', text)
        self.assertIn('LULO_CARGO_TARGET_DIR="$work_dir/target"', text)
        self.assertIn("pax_headers.get(\"comment\"", text)
        self.assertIn("cargo vendor --locked", text)
        self.assertIn("check-vendor", text)
        self.assertIn('[[ ${EUID} -ne 0 ]]', text)
        for forbidden in ("docker", "sudo ", "--all-features", "--all-targets"):
            self.assertNotIn(forbidden, text)
        # The one reused target directory reaches debian/rules only through
        # LULO_CARGO_TARGET_DIR; the script never invents another one.
        self.assertIsNone(re.search(r"(?<!LULO_)CARGO_TARGET_DIR=", text))

    def test_build_script_rejects_unknown_packages(self):
        result = subprocess.run(
            ["bash", str(BUILD_SCRIPT), "--package", "sway"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
