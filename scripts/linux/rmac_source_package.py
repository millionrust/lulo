#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Names, changelog, vendor config, and manifests for the rmac source package.

scripts/linux/build-rmac-source-package.sh builds a Debian 3.0 (quilt) source
package named `rmac` from one exact commit: the `git archive` export as
rmac_<upstream>.orig.tar.xz, one `cargo vendor --locked` run over both Cargo
workspaces as rmac_<upstream>.orig-vendor.tar.xz, and packaging/rmac-source/
debian plus a few generated files as rmac_<version>.debian.tar.xz. This module
holds the side-effect-free parts that script, debian/rules, and the tests
share, plus a small CLI:

  version --repo-root DIR        print native_version() of a source tree
  shell-vars --version V         print the artifact names as shell assignments
  changelog --version V --commit SHA --epoch N [--tag T] --output FILE
                                 write debian/changelog
  check-vendor-config --input FILE
                                 validate the config `cargo vendor` printed
  cargo-config --input FILE --vendor-dir DIR --output FILE
                                 write the offline CARGO_HOME/config.toml
  notices --vendor DIR --version V --commit SHA --output FILE
                                 write debian/dependency-licenses.txt
  manifest --directory DIR --version V --commit SHA --epoch N [--tag T]
           --source-tree DIR     write DIR/rmac-source.json
  check-source-dir --directory DIR
                                 verify a source package directory against its
                                 rmac-source.json; print its identity as shell
                                 assignments
  rebuild-manifest --directory DIR --source-dir DIR --architecture ARCH
                                 write DIR/rebuild.json for rebuilt packages
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import sys
import time


sys.path.insert(0, str(Path(__file__).resolve().parent))

from native_package_contract import (  # noqa: E402
    ARCHITECTURES,
    MAINTAINER,
    PACKAGE_SPECS,
    ContractError,
    native_version,
    package_filename,
    source_date_epoch,
)


SOURCE_PACKAGE = "rmac"
DISTRIBUTION = "resolute"
FORMAT_VERSION = 1
MANIFEST_NAME = "rmac-source.json"
REBUILD_MANIFEST_NAME = "rebuild.json"
CHECKSUM_NAME = "SHA256SUMS"
VENDOR_DIRECTORY = "vendor"
VENDORED_SOURCES = "vendored-sources"
CARGO_LOCKS = ("Cargo.lock", "shell/Cargo.lock")

_VERSION = re.compile(
    r"(?P<upstream>[0-9]+(?:\.[0-9]+){2}(?:~[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)"
    r"-(?P<revision>[1-9][0-9]*)"
)
_COMMIT = re.compile(r"[0-9a-f]{40}")
_TAG = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,127}")
_GIT_URL = re.compile(r"https://[A-Za-z0-9._~%/:@+-]+")
_GIT_REFERENCE_VALUE = re.compile(r"[A-Za-z0-9._/+-]{1,255}")
_GIT_REFERENCE_KEYS = ("branch", "tag", "rev")

_WEEKDAYS = ("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
_MONTHS = (
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
)


class SourcePackageError(RuntimeError):
    """A deterministic rmac source package contract failure."""


@dataclass(frozen=True)
class SourceArtifacts:
    version: str
    upstream: str
    top_directory: str
    dsc: str
    orig: str
    orig_vendor: str
    debian: str
    buildinfo: str
    changes: str

    @property
    def published(self) -> tuple[str, ...]:
        """Every file a source build publishes, in a fixed order."""
        return (
            self.dsc,
            self.orig,
            self.orig_vendor,
            self.debian,
            self.buildinfo,
            self.changes,
        )


def split_version(version: str) -> tuple[str, str]:
    match = _VERSION.fullmatch(version) if isinstance(version, str) else None
    if match is None:
        raise SourcePackageError("source package version is invalid")
    return match.group("upstream"), match.group("revision")


def artifact_names(version: str) -> SourceArtifacts:
    upstream, _ = split_version(version)
    stem = f"{SOURCE_PACKAGE}_{version}"
    return SourceArtifacts(
        version=version,
        upstream=upstream,
        top_directory=f"{SOURCE_PACKAGE}-{upstream}",
        dsc=f"{stem}.dsc",
        orig=f"{SOURCE_PACKAGE}_{upstream}.orig.tar.xz",
        orig_vendor=f"{SOURCE_PACKAGE}_{upstream}.orig-{VENDOR_DIRECTORY}.tar.xz",
        debian=f"{stem}.debian.tar.xz",
        buildinfo=f"{stem}_source.buildinfo",
        changes=f"{stem}_source.changes",
    )


def _commit(value: str) -> str:
    if not isinstance(value, str) or not _COMMIT.fullmatch(value):
        raise SourcePackageError("commit must be a full lowercase SHA-1")
    return value


def _tag(value: str | None) -> str | None:
    if value is None or value == "":
        return None
    if not _TAG.fullmatch(value):
        raise SourcePackageError("tag is invalid")
    return value


def rfc2822_date(epoch: int) -> str:
    """The changelog trailer date, independent of the locale and time zone."""
    moment = time.gmtime(source_date_epoch(epoch))
    return (
        f"{_WEEKDAYS[moment.tm_wday]}, {moment.tm_mday:02d} "
        f"{_MONTHS[moment.tm_mon - 1]} {moment.tm_year:04d} "
        f"{moment.tm_hour:02d}:{moment.tm_min:02d}:{moment.tm_sec:02d} +0000"
    )


def render_changelog(
    *,
    version: str,
    commit: str,
    epoch: int,
    tag: str | None = None,
    maintainer: str = MAINTAINER,
) -> str:
    split_version(version)
    commit = _commit(commit)
    tag = _tag(tag)
    if tag is None:
        entry = [
            f"  * Build of commit {commit}",
            "    (no release tag).",
        ]
    else:
        entry = [
            f"  * Release {tag}, built from commit",
            f"    {commit}.",
        ]
    return "\n".join(
        [
            f"{SOURCE_PACKAGE} ({version}) {DISTRIBUTION}; urgency=medium",
            "",
            *entry,
            "",
            f" -- {maintainer}  {rfc2822_date(epoch)}",
            "",
        ]
    )


# --- `cargo vendor` source replacement -------------------------------------

_HEADER = re.compile(r'\[source\.(?:(?P<plain>[a-z0-9-]+)|"(?P<quoted>[^"\\]+)")\]')
_ENTRY = re.compile(r'(?P<key>[a-z-]+) = "(?P<value>[^"\\\x00-\x1f]*)"')


@dataclass(frozen=True)
class GitSource:
    name: str
    url: str
    reference: tuple[str, str] | None


def parse_vendor_config(text: str) -> tuple[GitSource, ...]:
    """Validate the source replacement `cargo vendor` printed.

    Accepts exactly cargo's shape for crates.io plus git dependencies:
    `[source.crates-io]` and every `[source."git+https://..."]` replaced with
    `vendored-sources`, and one `[source.vendored-sources]` whose directory is
    the relative `vendor`. Anything else (another registry, a path, extra
    keys, duplicates) is refused. Returns the git sources in file order.
    """
    if not isinstance(text, str) or "\r" in text or "\0" in text:
        raise SourcePackageError("vendor config is not plain text")
    sections: dict[str, dict[str, str]] = {}
    order: list[str] = []
    current: dict[str, str] | None = None
    for number, line in enumerate(text.split("\n"), start=1):
        if line.strip() == "":
            continue
        header = _HEADER.fullmatch(line)
        if header is not None:
            name = header.group("plain") or header.group("quoted")
            if name in sections:
                raise SourcePackageError(f"vendor config repeats source {name}")
            current = {}
            sections[name] = current
            order.append(name)
            continue
        entry = _ENTRY.fullmatch(line)
        if entry is None or current is None:
            raise SourcePackageError(f"vendor config line {number} is not a source entry")
        if entry.group("key") in current:
            raise SourcePackageError(f"vendor config line {number} repeats a key")
        current[entry.group("key")] = entry.group("value")

    if sections.get("crates-io") != {"replace-with": VENDORED_SOURCES}:
        raise SourcePackageError("vendor config must replace crates-io with the vendored sources")
    if sections.get(VENDORED_SOURCES) != {"directory": VENDOR_DIRECTORY}:
        raise SourcePackageError('vendor config must name the one directory "vendor"')

    git_sources = []
    for name in order:
        if name in ("crates-io", VENDORED_SOURCES):
            continue
        entries = dict(sections[name])
        if not name.startswith("git+"):
            raise SourcePackageError(f"vendor config has an unexpected source {name}")
        if entries.pop("replace-with", None) != VENDORED_SOURCES:
            raise SourcePackageError(f"{name} is not replaced with the vendored sources")
        url = entries.pop("git", None)
        if url is None or not _GIT_URL.fullmatch(url):
            raise SourcePackageError(f"{name} does not name an https git URL")
        references = [(key, entries.pop(key)) for key in _GIT_REFERENCE_KEYS if key in entries]
        if entries:
            raise SourcePackageError(f"{name} has unexpected keys: {', '.join(sorted(entries))}")
        if len(references) > 1:
            raise SourcePackageError(f"{name} names more than one git reference")
        reference = references[0] if references else None
        if reference is not None and not _GIT_REFERENCE_VALUE.fullmatch(reference[1]):
            raise SourcePackageError(f"{name} has an invalid git reference")
        expected = f"git+{url}" + (f"?{reference[0]}={reference[1]}" if reference else "")
        if name != expected:
            raise SourcePackageError(f"{name} does not match its git URL and reference")
        git_sources.append(GitSource(name=name, url=url, reference=reference))
    return tuple(git_sources)


def _toml_path(path: str) -> str:
    if (
        not isinstance(path, str)
        or not path.startswith("/")
        or re.search(r'["\\\x00-\x1f\x7f]', path)
    ):
        raise SourcePackageError("vendor directory must be a plain absolute path")
    return path


def render_offline_cargo_config(vendor_config: str, vendor_directory: str) -> str:
    """The CARGO_HOME config.toml debian/rules builds with: vendored, offline."""
    git_sources = parse_vendor_config(vendor_config)
    directory = _toml_path(vendor_directory)
    lines = [
        "# Generated by debian/rules from vendor/.lulo-cargo-config.toml.",
        "# Every dependency comes from the vendor tarball; the network is off.",
        "",
        "[source.crates-io]",
        f'replace-with = "{VENDORED_SOURCES}"',
        "",
    ]
    for source in git_sources:
        lines.append(f'[source."{source.name}"]')
        lines.append(f'git = "{source.url}"')
        if source.reference is not None:
            lines.append(f'{source.reference[0]} = "{source.reference[1]}"')
        lines.append(f'replace-with = "{VENDORED_SOURCES}"')
        lines.append("")
    lines.extend(
        [
            f"[source.{VENDORED_SOURCES}]",
            f'directory = "{directory}"',
            "",
            "[net]",
            "offline = true",
            "",
        ]
    )
    return "\n".join(lines)


# --- manifests -------------------------------------------------------------


def _digests(path: Path) -> dict[str, object]:
    sha256 = hashlib.sha256()
    sha512 = hashlib.sha512()
    size = 0
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                sha256.update(chunk)
                sha512.update(chunk)
                size += len(chunk)
    except OSError as error:
        raise SourcePackageError(f"{path.name} cannot be read") from error
    return {"sha256": sha256.hexdigest(), "sha512": sha512.hexdigest(), "size": size}


def _regular(path: Path) -> Path:
    if path.is_symlink() or not path.is_file():
        raise SourcePackageError(f"{path.name} is missing or not a regular file")
    return path


def source_manifest(
    *,
    directory: Path,
    version: str,
    commit: str,
    epoch: int,
    tag: str | None,
    source_tree: Path,
) -> dict:
    names = artifact_names(version)
    artifacts = []
    for name in sorted(names.published):
        record = {"name": name}
        record.update(_digests(_regular(directory / name)))
        artifacts.append(record)
    vendor = next(item for item in artifacts if item["name"] == names.orig_vendor)
    locks = {}
    for lock in CARGO_LOCKS:
        locks[lock] = _digests(_regular(source_tree / lock))["sha256"]
    return {
        "artifacts": artifacts,
        "cargo_lock_sha256": locks,
        "commit": _commit(commit),
        "format": FORMAT_VERSION,
        "source": SOURCE_PACKAGE,
        "source_date_epoch": source_date_epoch(epoch),
        "tag": _tag(tag),
        "vendor_sha256": vendor["sha256"],
        "version": version,
    }


def _dump(document: dict) -> str:
    return json.dumps(document, indent=2, sort_keys=True) + "\n"


def check_source_directory(directory: Path) -> dict:
    """Verify a published source directory: exact inventory, sizes, hashes."""
    if not directory.is_absolute() or directory.is_symlink() or not directory.is_dir():
        raise SourcePackageError("source directory must be an absolute ordinary directory")
    try:
        document = json.loads(_regular(directory / MANIFEST_NAME).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise SourcePackageError(f"{MANIFEST_NAME} is unreadable") from error
    if not isinstance(document, dict) or document.get("format") != FORMAT_VERSION:
        raise SourcePackageError(f"{MANIFEST_NAME} format is not {FORMAT_VERSION}")
    if document.get("source") != SOURCE_PACKAGE:
        raise SourcePackageError(f"{MANIFEST_NAME} is not for source package {SOURCE_PACKAGE}")
    names = artifact_names(document.get("version"))
    _commit(document.get("commit"))
    _tag(document.get("tag"))
    source_date_epoch(document.get("source_date_epoch"))
    records = document.get("artifacts")
    if not isinstance(records, list) or sorted(
        item.get("name") for item in records if isinstance(item, dict)
    ) != sorted(names.published) or len(records) != len(names.published):
        raise SourcePackageError(f"{MANIFEST_NAME} artifact inventory is not exact")
    actual = {path.name for path in directory.iterdir()}
    if actual != set(names.published) | {MANIFEST_NAME, CHECKSUM_NAME}:
        raise SourcePackageError("source directory inventory is not exact")
    for record in records:
        measured = _digests(_regular(directory / record["name"]))
        for key in ("sha256", "sha512", "size"):
            if record.get(key) != measured[key]:
                raise SourcePackageError(f"{record['name']} does not match {MANIFEST_NAME}")
        if record["name"] == names.orig_vendor and document.get("vendor_sha256") != measured["sha256"]:
            raise SourcePackageError(f"{MANIFEST_NAME} vendor_sha256 is inconsistent")
    return document


def rebuild_manifest(*, directory: Path, source_directory: Path, architecture: str) -> dict:
    if architecture not in ARCHITECTURES:
        raise SourcePackageError("unsupported Debian architecture")
    source = check_source_directory(source_directory)
    version = source["version"]
    names = artifact_names(version)
    packages = []
    for spec in PACKAGE_SPECS:
        filename = package_filename(spec, version, architecture)
        record = {"filename": filename, "package": spec.name}
        record.update(_digests(_regular(directory / filename)))
        packages.append(record)
    dsc = next(item for item in source["artifacts"] if item["name"] == names.dsc)
    manifest = _digests(_regular(source_directory / MANIFEST_NAME))
    stem = f"{SOURCE_PACKAGE}_{version}_{architecture}"
    for name in (f"{stem}.buildinfo", f"{stem}.changes"):
        _regular(directory / name)
    return {
        "architecture": architecture,
        "buildinfo": f"{stem}.buildinfo",
        "changes": f"{stem}.changes",
        "commit": source["commit"],
        "format": FORMAT_VERSION,
        "packages": packages,
        "source": SOURCE_PACKAGE,
        "source_dsc_sha256": dsc["sha256"],
        "source_manifest_sha256": manifest["sha256"],
        "version": version,
    }


def _shell(values: dict[str, str]) -> str:
    lines = []
    for key, value in values.items():
        if not re.fullmatch(r"[A-Za-z0-9._:/~+-]*", value):
            raise SourcePackageError(f"{key} is not shell-safe")
        lines.append(f"{key}='{value}'")
    return "\n".join(lines) + "\n"


def shell_assignments(version: str) -> str:
    names = artifact_names(version)
    return _shell(
        {
            "SRC_VERSION": names.version,
            "SRC_UPSTREAM": names.upstream,
            "SRC_TOP_DIRECTORY": names.top_directory,
            "SRC_DSC": names.dsc,
            "SRC_ORIG": names.orig,
            "SRC_ORIG_VENDOR": names.orig_vendor,
            "SRC_DEBIAN": names.debian,
            "SRC_BUILDINFO": names.buildinfo,
            "SRC_CHANGES": names.changes,
        }
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    version = commands.add_parser("version")
    version.add_argument("--repo-root", required=True, type=Path)
    shell = commands.add_parser("shell-vars")
    shell.add_argument("--version", required=True)
    changelog = commands.add_parser("changelog")
    changelog.add_argument("--version", required=True)
    changelog.add_argument("--commit", required=True)
    changelog.add_argument("--epoch", required=True)
    changelog.add_argument("--tag")
    changelog.add_argument("--output", required=True, type=Path)
    check = commands.add_parser("check-vendor-config")
    check.add_argument("--input", required=True, type=Path)
    cargo = commands.add_parser("cargo-config")
    cargo.add_argument("--input", required=True, type=Path)
    cargo.add_argument("--vendor-dir", required=True)
    cargo.add_argument("--output", required=True, type=Path)
    notices = commands.add_parser("notices")
    notices.add_argument("--vendor", required=True, type=Path)
    notices.add_argument("--version", required=True)
    notices.add_argument("--commit", required=True)
    notices.add_argument("--output", required=True, type=Path)
    manifest = commands.add_parser("manifest")
    manifest.add_argument("--directory", required=True, type=Path)
    manifest.add_argument("--version", required=True)
    manifest.add_argument("--commit", required=True)
    manifest.add_argument("--epoch", required=True)
    manifest.add_argument("--tag")
    manifest.add_argument("--source-tree", required=True, type=Path)
    source_dir = commands.add_parser("check-source-dir")
    source_dir.add_argument("--directory", required=True, type=Path)
    rebuild = commands.add_parser("rebuild-manifest")
    rebuild.add_argument("--directory", required=True, type=Path)
    rebuild.add_argument("--source-dir", required=True, type=Path)
    rebuild.add_argument("--architecture", required=True)
    arguments = parser.parse_args(argv)

    try:
        if arguments.command == "version":
            print(native_version(arguments.repo_root))
        elif arguments.command == "shell-vars":
            sys.stdout.write(shell_assignments(arguments.version))
        elif arguments.command == "changelog":
            arguments.output.write_text(
                render_changelog(
                    version=arguments.version,
                    commit=arguments.commit,
                    epoch=source_date_epoch(arguments.epoch),
                    tag=arguments.tag,
                ),
                encoding="utf-8",
            )
        elif arguments.command == "check-vendor-config":
            sources = parse_vendor_config(arguments.input.read_text(encoding="utf-8"))
            print(f"vendor config replaces crates-io and {len(sources)} git sources")
        elif arguments.command == "cargo-config":
            arguments.output.write_text(
                render_offline_cargo_config(
                    arguments.input.read_text(encoding="utf-8"), arguments.vendor_dir
                ),
                encoding="utf-8",
            )
        elif arguments.command == "notices":
            # Imported here so the helpers above stay usable without it.
            from third_party_packages import ThirdPartyError, dependency_notices

            heading = (
                f"rmac {arguments.version} (commit {_commit(arguments.commit)}): "
                "licences of the vendored Rust crates"
            )
            try:
                text = dependency_notices(arguments.vendor, heading)
            except ThirdPartyError as error:
                raise SourcePackageError(str(error)) from error
            arguments.output.write_text(text, encoding="utf-8")
        elif arguments.command == "manifest":
            document = source_manifest(
                directory=arguments.directory,
                version=arguments.version,
                commit=arguments.commit,
                epoch=source_date_epoch(arguments.epoch),
                tag=arguments.tag,
                source_tree=arguments.source_tree,
            )
            (arguments.directory / MANIFEST_NAME).write_text(_dump(document), encoding="utf-8")
        elif arguments.command == "check-source-dir":
            document = check_source_directory(arguments.directory)
            sys.stdout.write(shell_assignments(document["version"]))
            sys.stdout.write(_shell({"SRC_COMMIT": document["commit"]}))
        elif arguments.command == "rebuild-manifest":
            document = rebuild_manifest(
                directory=arguments.directory,
                source_directory=arguments.source_dir,
                architecture=arguments.architecture,
            )
            (arguments.directory / REBUILD_MANIFEST_NAME).write_text(
                _dump(document), encoding="utf-8"
            )
    except (SourcePackageError, ContractError, OSError) as error:
        print(f"rmac_source_package: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
