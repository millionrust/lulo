"""Contract tests for the rmac Debian source package."""

from __future__ import annotations

import email.utils
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "linux"))

import native_package_contract as native  # noqa: E402
import rmac_source_package as source  # noqa: E402


DEBIAN = REPO_ROOT / "packaging" / "rmac-source" / "debian"
BUILD_SCRIPT = REPO_ROOT / "scripts" / "linux" / "build-rmac-source-package.sh"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"
COMMIT = "0123456789abcdef0123456789abcdef01234567"

# The shape `cargo vendor --locked --sync shell/Cargo.toml vendor` prints for
# this repository: crates.io plus git sources with and without a pinned rev.
CARGO_VENDOR_OUTPUT = """\
[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/longbridge/gpui-component.git?rev=0775df394083c1ed74f36f846b78868d1267398f"]
git = "https://github.com/longbridge/gpui-component.git"
rev = "0775df394083c1ed74f36f846b78868d1267398f"
replace-with = "vendored-sources"

[source."git+https://github.com/zed-industries/font-kit?rev=94b0f28166665e8fd2f53ff6d268a14955c82269"]
git = "https://github.com/zed-industries/font-kit"
rev = "94b0f28166665e8fd2f53ff6d268a14955c82269"
replace-with = "vendored-sources"

[source."git+https://github.com/zed-industries/zed.git"]
git = "https://github.com/zed-industries/zed.git"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
"""


def _paragraphs(text: str) -> list[dict[str, str]]:
    """A minimal deb822 reader: comments dropped, continuation lines joined."""
    paragraphs = []
    for block in re.split(r"\n\s*\n", text.strip()):
        fields: dict[str, str] = {}
        key = None
        for line in block.splitlines():
            if line.startswith("#"):
                continue
            if line[:1] in (" ", "\t") and key is not None:
                fields[key] += "\n" + line.strip()
                continue
            key, _, value = line.partition(":")
            fields[key] = value.strip()
        paragraphs.append(fields)
    return paragraphs


def _control() -> list[dict[str, str]]:
    return _paragraphs((DEBIAN / "control").read_text(encoding="utf-8"))


def _relation_names(value: str) -> list[str]:
    names = []
    for entry in value.replace("\n", " ").split(","):
        entry = entry.strip()
        if entry:
            names.append(re.sub(r"\s*\(.*\)$", "", entry).split(":")[0])
    return names


class NamingTests(unittest.TestCase):
    def test_pre_release_names_use_the_tilde_upstream_version(self):
        names = source.artifact_names("0.9.0~beta.1-38")
        self.assertEqual(names.upstream, "0.9.0~beta.1")
        self.assertEqual(names.top_directory, "rmac-0.9.0~beta.1")
        self.assertEqual(
            names.published,
            (
                "rmac_0.9.0~beta.1-38.dsc",
                "rmac_0.9.0~beta.1.orig.tar.xz",
                "rmac_0.9.0~beta.1.orig-vendor.tar.xz",
                "rmac_0.9.0~beta.1-38.debian.tar.xz",
                "rmac_0.9.0~beta.1-38_source.buildinfo",
                "rmac_0.9.0~beta.1-38_source.changes",
            ),
        )

    def test_final_release_names(self):
        names = source.artifact_names("1.0.0-38")
        self.assertEqual(names.orig, "rmac_1.0.0.orig.tar.xz")
        self.assertEqual(names.orig_vendor, "rmac_1.0.0.orig-vendor.tar.xz")
        self.assertEqual(names.dsc, "rmac_1.0.0-38.dsc")
        self.assertEqual(names.changes, "rmac_1.0.0-38_source.changes")

    def test_the_checkout_version_has_valid_names(self):
        version = native.native_version(REPO_ROOT)
        self.assertEqual(source.artifact_names(version).version, version)

    def test_invalid_versions_are_refused(self):
        for version in ("0.9.0", "0.9.0-beta.1-38", "1:0.9.0-38", "0.9.0-0", "0.9-1", ""):
            with self.subTest(version=version):
                with self.assertRaises(source.SourcePackageError):
                    source.artifact_names(version)

    def test_shell_assignments_are_quoted_and_complete(self):
        text = source.shell_assignments("0.9.0~beta.1-38")
        self.assertIn("SRC_DSC='rmac_0.9.0~beta.1-38.dsc'", text)
        self.assertIn("SRC_ORIG_VENDOR='rmac_0.9.0~beta.1.orig-vendor.tar.xz'", text)
        self.assertIn("SRC_TOP_DIRECTORY='rmac-0.9.0~beta.1'", text)
        for line in text.splitlines():
            self.assertRegex(line, r"^SRC_[A-Z_]+='[A-Za-z0-9._:/~+-]+'$")


class ChangelogTests(unittest.TestCase):
    EPOCH = 1790000000

    def _parse(self, text: str) -> dict[str, str]:
        lines = text.split("\n")
        header = re.fullmatch(
            r"(?P<source>[a-z0-9][a-z0-9+.-]+) \((?P<version>[^ ()]+)\) "
            r"(?P<distribution>[a-z0-9-]+); urgency=(?P<urgency>[a-z]+)",
            lines[0],
        )
        self.assertIsNotNone(header, lines[0])
        self.assertEqual(lines[1], "")
        changes = []
        index = 2
        while lines[index] != "":
            self.assertTrue(lines[index].startswith("  "), lines[index])
            changes.append(lines[index])
            index += 1
        trailer = re.fullmatch(r" -- (?P<maintainer>.+ <[^>]+>)  (?P<date>.+)", lines[index + 1])
        self.assertIsNotNone(trailer, lines[index + 1])
        self.assertEqual(lines[index + 2 :], [""])
        self.assertTrue(changes[0].startswith("  * "))
        for line in lines:
            self.assertLessEqual(len(line), 80)
        result = dict(header.groupdict())
        result.update(trailer.groupdict())
        result["changes"] = "\n".join(changes)
        return result

    def test_tagged_changelog_parses_by_debian_rules(self):
        text = source.render_changelog(
            version="0.9.0~beta.1-38", commit=COMMIT, epoch=self.EPOCH, tag="v0.9.0-beta.1"
        )
        parsed = self._parse(text)
        self.assertEqual(parsed["source"], "rmac")
        self.assertEqual(parsed["version"], "0.9.0~beta.1-38")
        self.assertEqual(parsed["distribution"], "resolute")
        self.assertEqual(parsed["urgency"], "medium")
        self.assertEqual(parsed["maintainer"], native.MAINTAINER)
        self.assertIn("v0.9.0-beta.1", parsed["changes"])
        self.assertIn(COMMIT, parsed["changes"])
        self.assertEqual(email.utils.mktime_tz(email.utils.parsedate_tz(parsed["date"])), self.EPOCH)
        self.assertRegex(
            parsed["date"], r"^(Mon|Tue|Wed|Thu|Fri|Sat|Sun), [0-3][0-9] [A-Z][a-z]{2} \d{4} \d\d:\d\d:\d\d \+0000$"
        )

    def test_untagged_changelog_names_the_commit(self):
        parsed = self._parse(
            source.render_changelog(version="1.0.0-38", commit=COMMIT, epoch=self.EPOCH)
        )
        self.assertEqual(parsed["version"], "1.0.0-38")
        self.assertIn(COMMIT, parsed["changes"])
        self.assertIn("no release tag", parsed["changes"])

    def test_known_date(self):
        self.assertEqual(source.rfc2822_date(0), "Thu, 01 Jan 1970 00:00:00 +0000")
        self.assertEqual(source.rfc2822_date(1790000000), "Mon, 21 Sep 2026 14:13:20 +0000")

    def test_invalid_identities_are_refused(self):
        with self.assertRaises(source.SourcePackageError):
            source.render_changelog(version="1.0.0-38", commit="abc", epoch=1)
        with self.assertRaises(source.SourcePackageError):
            source.render_changelog(version="1.0.0-38", commit=COMMIT, epoch=1, tag="v1 bad")
        with self.assertRaises(native.ContractError):
            source.render_changelog(version="1.0.0-38", commit=COMMIT, epoch=-1)


class VendorConfigTests(unittest.TestCase):
    def test_accepts_cargo_vendor_output(self):
        sources = source.parse_vendor_config(CARGO_VENDOR_OUTPUT)
        self.assertEqual(
            [item.url for item in sources],
            [
                "https://github.com/longbridge/gpui-component.git",
                "https://github.com/zed-industries/font-kit",
                "https://github.com/zed-industries/zed.git",
            ],
        )
        self.assertEqual(sources[0].reference, ("rev", "0775df394083c1ed74f36f846b78868d1267398f"))
        self.assertIsNone(sources[2].reference)

    def test_accepts_branch_and_tag_references(self):
        text = CARGO_VENDOR_OUTPUT.replace(
            "[source.vendored-sources]",
            '[source."git+https://example.org/a.git?branch=main"]\n'
            'git = "https://example.org/a.git"\nbranch = "main"\n'
            'replace-with = "vendored-sources"\n\n'
            '[source."git+https://example.org/b.git?tag=v1.2"]\n'
            'git = "https://example.org/b.git"\ntag = "v1.2"\n'
            'replace-with = "vendored-sources"\n\n'
            "[source.vendored-sources]",
        )
        self.assertEqual(len(source.parse_vendor_config(text)), 5)

    def test_every_git_source_in_the_lockfiles_has_a_matching_shape(self):
        # Each git source Cargo cares about must be expressible by the
        # validator: `git+URL[?rev=...]` without the #precise fragment.
        for lock in source.CARGO_LOCKS:
            text = (REPO_ROOT / lock).read_text(encoding="utf-8")
            for match in re.finditer(r'^source = "(git\+[^"#]+)#[0-9a-f]{40}"$', text, re.M):
                name = match.group(1)
                url, _, query = name[len("git+"):].partition("?")
                lines = [f'[source."{name}"]', f'git = "{url}"']
                if query:
                    key, _, value = query.partition("=")
                    lines.append(f'{key} = "{value}"')
                lines.append('replace-with = "vendored-sources"')
                text_config = (
                    '[source.crates-io]\nreplace-with = "vendored-sources"\n\n'
                    + "\n".join(lines)
                    + '\n\n[source.vendored-sources]\ndirectory = "vendor"\n'
                )
                with self.subTest(source=name):
                    self.assertEqual(len(source.parse_vendor_config(text_config)), 1)

    def test_rejects_extra_keys_paths_and_sources(self):
        bad = {
            "absolute directory": CARGO_VENDOR_OUTPUT.replace(
                'directory = "vendor"', 'directory = "/tmp/vendor"'
            ),
            "other directory": CARGO_VENDOR_OUTPUT.replace(
                'directory = "vendor"', 'directory = "../vendor"'
            ),
            "extra key": CARGO_VENDOR_OUTPUT.replace(
                'git = "https://github.com/zed-industries/zed.git"\n',
                'git = "https://github.com/zed-industries/zed.git"\nlocal-registry = "x"\n',
            ),
            "extra vendored key": CARGO_VENDOR_OUTPUT + 'registry = "x"\n',
            "other registry": '[source.my-registry]\nreplace-with = "vendored-sources"\n\n'
            + CARGO_VENDOR_OUTPUT,
            "http git": CARGO_VENDOR_OUTPUT.replace(
                "https://github.com/zed-industries/zed.git", "http://github.com/zed-industries/zed.git"
            ),
            "mismatched name": CARGO_VENDOR_OUTPUT.replace(
                '[source."git+https://github.com/zed-industries/zed.git"]',
                '[source."git+https://github.com/zed-industries/zed.git?rev=abc"]',
            ),
            "not replaced": CARGO_VENDOR_OUTPUT.replace(
                'git = "https://github.com/zed-industries/zed.git"\nreplace-with = "vendored-sources"',
                'git = "https://github.com/zed-industries/zed.git"\nreplace-with = "elsewhere"',
            ),
            "duplicate section": CARGO_VENDOR_OUTPUT + '\n[source.crates-io]\nreplace-with = "vendored-sources"\n',
            "comment": "# hello\n" + CARGO_VENDOR_OUTPUT,
            "net section": CARGO_VENDOR_OUTPUT + "\n[net]\noffline = true\n",
            "missing crates-io": CARGO_VENDOR_OUTPUT.replace(
                '[source.crates-io]\nreplace-with = "vendored-sources"\n', ""
            ),
            "two references": CARGO_VENDOR_OUTPUT.replace(
                'rev = "94b0f28166665e8fd2f53ff6d268a14955c82269"\n',
                'rev = "94b0f28166665e8fd2f53ff6d268a14955c82269"\nbranch = "main"\n',
            ),
            "carriage return": CARGO_VENDOR_OUTPUT.replace("\n", "\r\n"),
            "escaped quote": CARGO_VENDOR_OUTPUT.replace('directory = "vendor"', 'directory = "ven\\"dor"'),
        }
        for label, text in bad.items():
            with self.subTest(label):
                with self.assertRaises(source.SourcePackageError):
                    source.parse_vendor_config(text)

    def test_offline_config_is_absolute_and_offline(self):
        text = source.render_offline_cargo_config(CARGO_VENDOR_OUTPUT, "/build/rmac-1.0.0/vendor")
        self.assertIn('[source.vendored-sources]\ndirectory = "/build/rmac-1.0.0/vendor"\n', text)
        self.assertIn("[net]\noffline = true\n", text)
        self.assertNotIn('directory = "vendor"', text)
        self.assertEqual(text.count('replace-with = "vendored-sources"'), 4)
        self.assertIn('rev = "0775df394083c1ed74f36f846b78868d1267398f"', text)
        for bad in ("vendor", "/with\"quote", "/new\nline"):
            with self.subTest(bad=bad):
                with self.assertRaises(source.SourcePackageError):
                    source.render_offline_cargo_config(CARGO_VENDOR_OUTPUT, bad)


class ManifestTests(unittest.TestCase):
    VERSION = "0.9.0~beta.1-38"

    def _source_dir(self, root: Path) -> Path:
        directory = root / "out"
        directory.mkdir()
        for name in source.artifact_names(self.VERSION).published:
            (directory / name).write_bytes(name.encode("ascii") * 3)
        tree = root / "tree"
        (tree / "shell").mkdir(parents=True)
        (tree / "Cargo.lock").write_text("root lock\n", encoding="ascii")
        (tree / "shell" / "Cargo.lock").write_text("shell lock\n", encoding="ascii")
        self.assertEqual(
            source.main(
                [
                    "manifest", "--directory", str(directory), "--version", self.VERSION,
                    "--commit", COMMIT, "--epoch", "1790000000", "--tag", "v0.9.0-beta.1",
                    "--source-tree", str(tree),
                ]
            ),
            0,
        )
        (directory / "SHA256SUMS").write_text("", encoding="ascii")
        return directory

    def test_manifest_binds_every_artifact_and_lockfile(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._source_dir(Path(temporary))
            document = json.loads((directory / "rmac-source.json").read_text(encoding="utf-8"))
            names = source.artifact_names(self.VERSION)
            self.assertEqual(document["format"], 1)
            self.assertEqual(document["source"], "rmac")
            self.assertEqual(document["version"], self.VERSION)
            self.assertEqual(document["commit"], COMMIT)
            self.assertEqual(document["tag"], "v0.9.0-beta.1")
            self.assertEqual(document["source_date_epoch"], 1790000000)
            self.assertEqual([item["name"] for item in document["artifacts"]], sorted(names.published))
            for item in document["artifacts"]:
                self.assertEqual(set(item), {"name", "sha256", "sha512", "size"})
                self.assertRegex(item["sha256"], r"^[0-9a-f]{64}$")
                self.assertRegex(item["sha512"], r"^[0-9a-f]{128}$")
            vendor = next(i for i in document["artifacts"] if i["name"] == names.orig_vendor)
            self.assertEqual(document["vendor_sha256"], vendor["sha256"])
            self.assertEqual(set(document["cargo_lock_sha256"]), {"Cargo.lock", "shell/Cargo.lock"})
            self.assertEqual(source.check_source_directory(directory)["commit"], COMMIT)

    def test_untagged_manifest_records_null(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = self._source_dir(root)
            (directory / "rmac-source.json").unlink()
            source.main(
                [
                    "manifest", "--directory", str(directory), "--version", self.VERSION,
                    "--commit", COMMIT, "--epoch", "5", "--source-tree", str(root / "tree"),
                ]
            )
            document = json.loads((directory / "rmac-source.json").read_text(encoding="utf-8"))
            self.assertIsNone(document["tag"])

    def test_check_refuses_altered_extra_or_missing_files(self):
        names = source.artifact_names(self.VERSION)
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._source_dir(Path(temporary))
            (directory / names.debian).write_bytes(b"altered")
            with self.assertRaises(source.SourcePackageError):
                source.check_source_directory(directory)
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._source_dir(Path(temporary))
            (directory / "extra.txt").write_bytes(b"x")
            with self.assertRaises(source.SourcePackageError):
                source.check_source_directory(directory)
        with tempfile.TemporaryDirectory() as temporary:
            directory = self._source_dir(Path(temporary))
            (directory / names.changes).unlink()
            with self.assertRaises(source.SourcePackageError):
                source.check_source_directory(directory)

    def test_rebuild_manifest_hashes_both_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source_dir = self._source_dir(root)
            rebuilt = root / "rebuilt"
            rebuilt.mkdir()
            for spec in native.PACKAGE_SPECS:
                (rebuilt / native.package_filename(spec, self.VERSION, "amd64")).write_bytes(
                    spec.name.encode("ascii")
                )
            for suffix in ("buildinfo", "changes"):
                (rebuilt / f"rmac_{self.VERSION}_amd64.{suffix}").write_bytes(b"x")
            document = source.rebuild_manifest(
                directory=rebuilt, source_directory=source_dir, architecture="amd64"
            )
            self.assertEqual(
                [item["package"] for item in document["packages"]], ["rmac-apps", "rmac-session"]
            )
            self.assertEqual(document["commit"], COMMIT)
            self.assertEqual(document["version"], self.VERSION)
            self.assertEqual(document["changes"], f"rmac_{self.VERSION}_amd64.changes")


class DebianDirectoryTests(unittest.TestCase):
    def test_source_format_is_quilt(self):
        self.assertEqual((DEBIAN / "source" / "format").read_text(encoding="ascii"), "3.0 (quilt)\n")

    def test_only_the_hand_written_files_are_tracked_here(self):
        # changelog, dependency-licenses.txt, and rmac-revision are generated
        # per commit by build-rmac-source-package.sh.
        files = sorted(
            str(path.relative_to(DEBIAN)) for path in DEBIAN.rglob("*") if path.is_file()
        )
        self.assertEqual(files, ["control", "copyright", "rules", "source/format"])

    def test_control_declares_exactly_the_two_binary_packages(self):
        paragraphs = _control()
        header, packages = paragraphs[0], paragraphs[1:]
        self.assertEqual(header["Source"], "rmac")
        self.assertEqual(header["Section"], "x11")
        self.assertEqual(header["Priority"], "optional")
        self.assertEqual(header["Maintainer"], native.MAINTAINER)
        self.assertEqual(header["Rules-Requires-Root"], "no")
        self.assertEqual(header["Homepage"], "https://github.com/millionrust/lulo")
        self.assertEqual(
            [paragraph["Package"] for paragraph in packages],
            [spec.name for spec in native.PACKAGE_SPECS],
        )
        for paragraph, spec in zip(packages, native.PACKAGE_SPECS):
            with self.subTest(package=spec.name):
                self.assertEqual(paragraph["Architecture"].split(), sorted(native.ARCHITECTURES))
                summary, _, body = paragraph["Description"].partition("\n")
                self.assertEqual(summary, spec.summary)
                self.assertEqual(" ".join(body.split()), spec.description)
                self.assertEqual(paragraph.get("Section", header["Section"]), spec.section)

    def test_build_depends_needs_no_rust_from_the_archive(self):
        names = _relation_names(_control()[0]["Build-Depends"])
        self.assertEqual(len(names), len(set(names)))
        for name in ("clang", "dpkg-dev", "pkg-config", "python3", "xz-utils"):
            self.assertIn(name, names)
        for name in ("cargo", "rustc", "debhelper", "debhelper-compat", "fakeroot"):
            self.assertNotIn(name, names)

    def test_build_depends_are_installed_by_the_release_build(self):
        import yaml

        workflow = yaml.safe_load(RELEASE_WORKFLOW.read_text(encoding="utf-8"))
        steps = workflow["jobs"]["build-amd64"]["steps"]
        install = next(
            step["run"] for step in steps if "apt-get install" in str(step.get("run", ""))
        )
        command = install[install.index("apt-get install"):].replace("\\\n", " ")
        command = command.split("\n", 1)[0]
        installed = {word for word in command.split() if not word.startswith("-")} - {"apt-get", "install"}
        missing = sorted(set(_relation_names(_control()[0]["Build-Depends"])) - installed)
        self.assertEqual(missing, [], "release.yml build-amd64 must install every Build-Depends")

    def test_copyright_is_dep5_and_carries_the_mit_licence(self):
        text = (DEBIAN / "copyright").read_text(encoding="utf-8")
        paragraphs = _paragraphs(text)
        self.assertEqual(
            paragraphs[0]["Format"],
            "https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/",
        )
        files = {p["Files"]: p for p in paragraphs if "Files" in p}
        self.assertEqual(set(files), {"*", "vendor/*", "debian/*"})
        self.assertEqual(files["*"]["License"], "MIT")
        self.assertTrue(files["vendor/*"]["License"].startswith("various"))
        self.assertIn("debian/dependency-licenses.txt", files["vendor/*"]["License"])
        licence = next(p for p in paragraphs if p.get("License", "").startswith("MIT\n"))
        body = licence["License"].split("\n", 1)[1].replace("\n.\n", "\n")
        repository = (REPO_ROOT / "LICENSE").read_text(encoding="utf-8")
        permission = repository[repository.index("Permission is hereby granted"):]
        self.assertEqual(" ".join(body.split()), " ".join(permission.split()))

    def test_rules_build_offline_with_remapped_paths(self):
        rules = (DEBIAN / "rules").read_text(encoding="utf-8")
        self.assertTrue(rules.startswith("#!/usr/bin/make -f\n"))
        self.assertTrue((DEBIAN / "rules").stat().st_mode & 0o111, "debian/rules must be executable")
        self.assertIn(".RECIPEPREFIX = >", rules)
        self.assertIn("--remap-path-prefix=$(CURDIR)=", rules)
        self.assertIn("--remap-path-prefix=$(LULO_CARGO_HOME)=", rules)
        self.assertRegex(rules, r"(?m)^export CARGO_NET_OFFLINE := true$")
        self.assertRegex(rules, r"(?m)^export CARGO_HOME := \$\(LULO_CARGO_HOME\)$")
        self.assertIn("cargo-config", rules)
        self.assertIn("vendor/.lulo-cargo-config.toml", rules)
        self.assertIn("scripts/linux/build-native-inputs.sh", rules)
        self.assertIn("scripts/linux/build-native-packages.py", rules)
        self.assertIn("dpkg-distaddfile", rules)
        self.assertNotIn("check-native-reproducibility", rules)
        code = "\n".join(line for line in rules.splitlines() if not line.startswith("#"))
        for forbidden in ("cargo vendor", "curl", "wget", "git ", "http://", "https://", "rustup "):
            self.assertNotIn(forbidden, code)
        for target in ("build", "build-arch", "build-indep", "binary", "binary-arch", "binary-indep", "clean"):
            self.assertRegex(rules, rf"(?m)^{re.escape(target)}:")
        for line in rules.splitlines():
            self.assertFalse(line.startswith("\t"), "recipes must use the > prefix")
        clean = rules[rules.index("\nclean:"):]
        for path in ("target", "shell/target", "$(LULO_CARGO_HOME)", "$(NATIVE_INPUTS)", "$(NATIVE_PACKAGES)", "debian/files", "debian/stamp-build"):
            self.assertIn(path, clean)

    def test_rules_register_each_package_with_its_section(self):
        rules = (DEBIAN / "rules").read_text(encoding="utf-8")
        self.assertIn("dpkg-distaddfile $(APPS_DEB) utils optional", rules)
        self.assertIn("dpkg-distaddfile $(SESSION_DEB) x11 optional", rules)
        sections = {spec.name: spec.section for spec in native.PACKAGE_SPECS}
        self.assertEqual(sections, {"rmac-apps": "utils", "rmac-session": "x11"})


class BuildScriptTests(unittest.TestCase):
    def test_build_script_is_valid_bash(self):
        result = subprocess.run(
            ["bash", "-n", str(BUILD_SCRIPT)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_build_script_follows_the_disk_and_network_rules(self):
        script = BUILD_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("set -euo pipefail", script)
        self.assertIn("minimum_free_gib=25", script)
        self.assertIn('EUID} -ne 0', script)
        self.assertIn("cargo vendor --locked --sync shell/Cargo.toml vendor", script)
        self.assertIn("xz -T1 -6", script)
        self.assertIn('--mtime="@$epoch"', script)
        self.assertIn("--sort=name", script)
        self.assertIn("--owner=0 --group=0 --numeric-owner --mode=go-w", script)
        self.assertIn("dpkg-buildpackage -S -sa -us -uc -d", script)
        self.assertIn("dpkg-buildpackage -b -us -uc\n", script)
        self.assertIn("CARGO_NET_OFFLINE=true", script)
        self.assertIn("verify-native-packages.py", script)
        self.assertNotIn("check-native-reproducibility", script)
        self.assertNotIn("CARGO_TARGET_DIR", script)
        code = "\n".join(line for line in script.splitlines() if not line.lstrip().startswith("#"))
        self.assertEqual(len(re.findall(r"cargo vendor -", code)), 1)
        self.assertEqual(code.count("curl"), 0)

    def test_build_script_rejects_bad_arguments(self):
        for arguments in (
            [],
            ["publish"],
            ["source", "--output", "/nonexistent/out"],
            ["source", "--revision", COMMIT],
            ["rebuild", "--revision", COMMIT, "--output", "/x"],
            ["source", "--revision", COMMIT, "--output", "/x", "--jobs", "2"],
            ["rebuild", "--source-dir", "/x", "--output", "/y", "--jobs", "0"],
        ):
            with self.subTest(arguments=arguments):
                result = subprocess.run(
                    ["bash", str(BUILD_SCRIPT), *arguments],
                    capture_output=True, text=True, check=False,
                )
                self.assertIn(result.returncode, (1, 2), result.stderr)
                self.assertIn("build-rmac-source-package", result.stderr)


@unittest.skipUnless(
    shutil.which("dpkg-buildpackage") and shutil.which("dpkg-source") and shutil.which("git"),
    "Debian packaging tools are not installed",
)
class EndToEndSourceTests(unittest.TestCase):
    """Make a real source package from a tiny synthetic tree with our debian/."""

    def test_source_package_builds_and_extracts(self):
        version = "0.9.0~beta.1-38"
        names = source.artifact_names(version)
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            tree = parent / names.top_directory
            (tree / "shell").mkdir(parents=True)
            (tree / "Cargo.lock").write_text("# root\n", encoding="ascii")
            (tree / "shell" / "Cargo.lock").write_text("# shell\n", encoding="ascii")
            (tree / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.95.0"\n', encoding="ascii")
            subprocess.run(
                ["tar", "-cJf", str(parent / names.orig), "-C", str(parent), names.top_directory],
                check=True,
            )
            (tree / "vendor" / "demo").mkdir(parents=True)
            (tree / "vendor" / "demo" / "Cargo.toml").write_text(
                '[package]\nname = "demo"\nversion = "1.0.0"\nlicense = "MIT"\n', encoding="ascii"
            )
            (tree / "vendor" / ".lulo-cargo-config.toml").write_text(CARGO_VENDOR_OUTPUT, encoding="ascii")
            subprocess.run(
                ["tar", "-cJf", str(parent / names.orig_vendor), "-C", str(tree), "vendor"], check=True
            )
            shutil.copytree(DEBIAN, tree / "debian")
            (tree / "debian" / "changelog").write_text(
                source.render_changelog(version=version, commit=COMMIT, epoch=1790000000),
                encoding="utf-8",
            )
            (tree / "debian" / "rmac-revision").write_text(COMMIT + "\n", encoding="ascii")
            subprocess.run(["dpkg-buildpackage", "-S", "-sa", "-us", "-uc", "-d"], cwd=tree, check=True)
            for name in names.published:
                self.assertTrue((parent / name).is_file(), name)
            dsc = (parent / names.dsc).read_text(encoding="utf-8")
            self.assertIn("Binary: rmac-apps, rmac-session", dsc)
            self.assertIn(names.orig_vendor, dsc)
            extracted = parent / "extracted"
            subprocess.run(["dpkg-source", "-x", str(parent / names.dsc), str(extracted)], check=True)
            self.assertTrue((extracted / "vendor" / ".lulo-cargo-config.toml").is_file())
            self.assertTrue((extracted / "debian" / "rmac-revision").is_file())


if __name__ == "__main__":
    unittest.main()
