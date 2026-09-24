#!/usr/bin/env python3
"""Verify rmac Flatpak decisions, permissions, and offline Cargo sources."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import stat
import sys
import tomllib
from typing import NamedTuple
from urllib.parse import parse_qs, urlparse


class VerificationError(RuntimeError):
    pass


APP_IDS = {
    "org.rmac.TextEditor",
    "org.rmac.Notes",
    "org.rmac.Files",
    "org.rmac.Terminal",
    "org.rmac.SystemMonitor",
    "org.rmac.AppDrawer",
    "org.rmac.SystemSettings",
}
TEXT_EDITOR_ID = "org.rmac.TextEditor"
TEXT_EDITOR_PERMISSIONS = {"--socket=wayland", "--device=dri"}
FORBIDDEN_PERMISSION_PREFIXES = (
    "--filesystem=",
    "--persist=",
    "--share=network",
    "--socket=session-bus",
    "--socket=system-bus",
    "--socket=x11",
    "--socket=fallback-x11",
    "--socket=pulseaudio",
    "--device=all",
    "--talk-name=",
    "--own-name=",
    "--system-talk-name=",
    "--system-own-name=",
)
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_DRIVER_BYTES = 64 * 1024


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise VerificationError(f"could not read {path}: {error}") from error


def verify_decisions(document: dict) -> None:
    if document.get("schema_version") != 1:
        raise VerificationError("unsupported Flatpak decision schema")
    applications = document.get("applications")
    if not isinstance(applications, list):
        raise VerificationError("Flatpak decisions require an application list")
    by_id = {entry.get("id"): entry for entry in applications if isinstance(entry, dict)}
    if set(by_id) != APP_IDS or len(by_id) != len(applications):
        raise VerificationError("Flatpak decisions must cover each exact application once")

    eligible = [entry for entry in applications if entry.get("sandbox_eligible") is True]
    if [entry.get("id") for entry in eligible] != [TEXT_EDITOR_ID]:
        raise VerificationError("only the reviewed Text Editor may currently be sandboxed")
    text_editor = by_id[TEXT_EDITOR_ID]
    if text_editor.get("manifest") != "org.rmac.TextEditor.json":
        raise VerificationError("Text Editor must bind its exact manifest")
    if set(text_editor.get("permissions", [])) != TEXT_EDITOR_PERMISSIONS:
        raise VerificationError("Text Editor decision permissions changed")

    for entry in applications:
        authority = entry.get("authority")
        if not isinstance(authority, str) or not authority.strip():
            raise VerificationError(f"{entry.get('id')} has no authority rationale")
        if entry.get("sandbox_eligible") is not True:
            if entry.get("manifest") is not None or entry.get("permissions") != []:
                raise VerificationError(
                    f"{entry.get('id')} cannot carry a sandbox manifest or permissions"
                )


def verify_manifest(document: dict) -> None:
    expected = {
        "app-id": TEXT_EDITOR_ID,
        "runtime": "org.freedesktop.Platform",
        "runtime-version": "25.08",
        "sdk": "org.freedesktop.Sdk",
        "command": "rmac-text-editor",
    }
    for field, value in expected.items():
        if document.get(field) != value:
            raise VerificationError(f"unexpected Flatpak {field}")
    if document.get("sdk-extensions") != [
        "org.freedesktop.Sdk.Extension.rust-stable"
    ]:
        raise VerificationError("Flatpak must use only the matching Rust SDK extension")

    permissions = document.get("finish-args")
    if not isinstance(permissions, list) or set(permissions) != TEXT_EDITOR_PERMISSIONS:
        raise VerificationError("Flatpak final permissions differ from reviewed policy")
    for permission in permissions:
        if permission.startswith(FORBIDDEN_PERMISSION_PREFIXES):
            raise VerificationError(f"forbidden Flatpak permission: {permission}")

    build_options = document.get("build-options", {})
    if build_options.get("append-path") != "/usr/lib/sdk/rust-stable/bin":
        raise VerificationError("Rust SDK path is not exact")
    if build_options.get("env") != {
        "CARGO_HOME": "/run/build/rmac-text-editor/cargo"
    }:
        raise VerificationError("offline Cargo home is not exact")
    if "build-args" in build_options:
        raise VerificationError("Flatpak build must not request network or host access")

    modules = document.get("modules")
    if not isinstance(modules, list) or len(modules) != 1:
        raise VerificationError("Text Editor Flatpak must have one exact module")
    module = modules[0]
    if module.get("name") != "rmac-text-editor" or module.get("buildsystem") != "simple":
        raise VerificationError("unexpected Text Editor Flatpak module")
    commands = module.get("build-commands", [])
    required_commands = {
        "cargo --offline --locked build --release --package rmac-text-editor",
        "install -Dm755 target/release/rmac-text-editor /app/bin/rmac-text-editor",
        "install -Dm644 packaging/rmac-apps/applications/org.rmac.TextEditor.desktop /app/share/applications/org.rmac.TextEditor.desktop",
        "sed -i 's|Exec=/usr/bin/rmac-text-editor|Exec=rmac-text-editor|' /app/share/applications/org.rmac.TextEditor.desktop",
        "install -Dm644 packaging/rmac-apps/metainfo/org.rmac.TextEditor.metainfo.xml /app/share/metainfo/org.rmac.TextEditor.metainfo.xml",
        "install -Dm644 packaging/rmac-apps/icons/org.rmac.TextEditor.svg /app/share/icons/hicolor/scalable/apps/org.rmac.TextEditor.svg",
    }
    if set(commands) != required_commands or len(commands) != len(required_commands):
        raise VerificationError("Text Editor Flatpak install/build commands changed")

    sources = module.get("sources")
    if not isinstance(sources, list) or len(sources) != 2:
        raise VerificationError("Flatpak module sources are not exact")
    if sources[0] != "cargo-sources.json":
        raise VerificationError("offline Cargo sources must be included first")
    local = sources[1]
    if local.get("type") != "dir" or local.get("path") != "../..":
        raise VerificationError("Flatpak development source must be the repository")
    if set(local.get("skip", [])) != {
        ".git",
        ".flatpak-builder",
        "build-dir",
        "repo",
        "target",
    }:
        raise VerificationError("Flatpak source exclusions are incomplete")


VENDOR = "cargo/vendor"
GIT_CACHE = "flatpak-cargo/git"
GIT_COMMIT = re.compile(r"^[0-9a-f]{40}$")
GIT_COPY = re.compile(
    r'^cp -r --reflink=auto "(flatpak-cargo/git/[^"/]+)/([^"]+)" "cargo/vendor/([^"/]+)"$'
)


class GitPackage(NamedTuple):
    """A Cargo.lock package fetched from a Git repository at a fixed commit."""

    repository: str
    commit: str
    version: str
    rev: str | None


def _read_lock(lock_path: Path) -> dict:
    try:
        return tomllib.loads(lock_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise VerificationError(f"could not read Cargo.lock: {error}") from error


def registry_packages(lock_path: Path) -> dict[tuple[str, str], str]:
    packages = {}
    for package in _read_lock(lock_path).get("package", []):
        source = package.get("source", "")
        if not source.startswith("registry+"):
            continue
        key = (package["name"], package["version"])
        checksum = package.get("checksum")
        if key in packages or not isinstance(checksum, str) or not SHA256.fullmatch(checksum):
            raise VerificationError(f"invalid or duplicate locked registry package {key}")
        packages[key] = checksum
    return packages


def canonical_git_url(source: str) -> str:
    """Cargo's canonical repository URL, as flatpak-cargo-generator spells it."""
    parsed = urlparse(source.removeprefix("git+"))
    path = parsed.path.rstrip("/")
    if parsed.netloc == "github.com":
        path = path.lower()
    path = path.removesuffix(".git")
    if parsed.scheme != "https" or not parsed.netloc or path.count("/") != 2:
        raise VerificationError(f"unsupported locked Git source {source}")
    return f"https://{parsed.netloc}{path}"


def git_packages(lock_path: Path) -> dict[str, GitPackage]:
    packages = {}
    for package in _read_lock(lock_path).get("package", []):
        source = package.get("source", "")
        if not source.startswith("git+"):
            continue
        name = package["name"]
        parsed = urlparse(source)
        commit = parsed.fragment
        revs = parse_qs(parsed.query).get("rev")
        # The generator vendors Git crates by bare name, so two locked Git
        # packages with one name would overwrite each other.
        if name in packages or not GIT_COMMIT.fullmatch(commit) or (
            revs is not None and len(revs) != 1
        ):
            raise VerificationError(f"invalid or duplicate locked Git package {name}")
        packages[name] = GitPackage(
            canonical_git_url(source),
            commit,
            package["version"],
            revs[0] if revs else None,
        )
    return packages


def git_checkout(package: GitPackage) -> str:
    repository = package.repository.rsplit("/", 1)[1]
    return f"{GIT_CACHE}/{repository}-{package.commit[:7]}"


def verify_cargo_sources(
    sources: list,
    locked: dict[tuple[str, str], str],
    git_locked: dict[str, GitPackage] | None = None,
) -> None:
    git_locked = git_locked or {}
    repositories = {
        (package.repository, package.commit) for package in git_locked.values()
    }
    expected_count = len(locked) * 2 + len(repositories) + len(git_locked) * 3 + 1
    if not isinstance(sources, list) or len(sources) != expected_count:
        raise VerificationError("generated Cargo source cardinality does not match Cargo.lock")
    archives = {}
    checksum_files = {}
    manifests = {}
    copies = {}
    checkouts = set()
    config = []
    for source in sources:
        if not isinstance(source, dict):
            raise VerificationError("generated Cargo source must be an object")
        source_type = source.get("type")
        if source_type == "archive":
            dest = source.get("dest", "")
            prefix = f"{VENDOR}/"
            if not dest.startswith(prefix):
                raise VerificationError("Cargo archive escaped the vendor directory")
            key = dest[len(prefix) :]
            archives[key] = source
        elif source_type == "git":
            if set(source) != {"type", "url", "commit", "dest"}:
                raise VerificationError("generated Git source is not exact")
            checkouts.add((source["url"], source["commit"], source["dest"]))
        elif source_type == "shell":
            commands = source.get("commands")
            if set(source) != {"type", "commands"} or not (
                isinstance(commands, list) and len(commands) == 1
            ):
                raise VerificationError("generated Git crate copy is not exact")
            match = GIT_COPY.fullmatch(commands[0])
            if match is None:
                raise VerificationError("generated Git crate copy is not exact")
            checkout, path, name = match.groups()
            if (
                name in copies
                or path.startswith("/")
                or ".." in path.split("/")
            ):
                raise VerificationError("generated Git crate copy escaped its checkout")
            copies[name] = (checkout, path)
        elif (
            source_type == "inline"
            and source.get("dest-filename") == ".cargo-checksum.json"
        ):
            checksum_files[source.get("dest", "")] = source
        elif source_type == "inline" and source.get("dest-filename") == "Cargo.toml":
            manifests[source.get("dest", "")] = source
        elif source_type == "inline" and source.get("dest") == "cargo":
            config.append(source)
        else:
            raise VerificationError("unexpected generated Cargo source type")
    if len(config) != 1:
        raise VerificationError("generated Cargo sources need one offline config")
    if config[0].get("dest-filename") != "config":
        raise VerificationError("generated Cargo config filename changed")
    try:
        config_document = tomllib.loads(config[0].get("contents", ""))
    except tomllib.TOMLDecodeError as error:
        raise VerificationError("generated Cargo config is invalid") from error
    expected_config = {
        "vendored-sources": {"directory": VENDOR},
        "crates-io": {"replace-with": "vendored-sources"},
    }
    for package in git_locked.values():
        replacement = {"git": package.repository, "replace-with": "vendored-sources"}
        if package.rev is not None:
            replacement["rev"] = package.rev
        if expected_config.setdefault(package.repository, replacement) != replacement:
            raise VerificationError(f"locked Git source {package.repository} is ambiguous")
    if config_document != {"source": expected_config}:
        raise VerificationError("generated Cargo config does not force vendored sources")

    expected_names = {f"{name}-{version}" for name, version in locked}
    if set(archives) != expected_names:
        raise VerificationError("generated Cargo archive inventory differs from Cargo.lock")
    expected_checksums = {f"{VENDOR}/{name}" for name in expected_names}
    expected_checksums |= {f"{VENDOR}/{name}" for name in git_locked}
    if set(checksum_files) != expected_checksums or len(expected_checksums) != len(
        expected_names
    ) + len(git_locked):
        raise VerificationError("generated Cargo checksum inventory differs from Cargo.lock")
    for (name, version), checksum in locked.items():
        vendor_name = f"{name}-{version}"
        archive = archives[vendor_name]
        expected_url = (
            f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
        )
        if (
            archive.get("archive-type") != "tar-gzip"
            or archive.get("url") != expected_url
            or archive.get("sha256") != checksum
        ):
            raise VerificationError(f"generated source mismatch for {vendor_name}")
        _verify_checksum_file(checksum_files[f"{VENDOR}/{vendor_name}"], vendor_name, checksum)

    expected_checkouts = {
        (package.repository, package.commit, git_checkout(package))
        for package in git_locked.values()
    }
    if checkouts != expected_checkouts:
        raise VerificationError("generated Git checkouts differ from Cargo.lock")
    if set(copies) != set(git_locked) or set(manifests) != {
        f"{VENDOR}/{name}" for name in git_locked
    }:
        raise VerificationError("generated Git crate inventory differs from Cargo.lock")
    for name, package in git_locked.items():
        if copies[name][0] != git_checkout(package):
            raise VerificationError(f"generated Git crate {name} is copied from another checkout")
        try:
            manifest = tomllib.loads(manifests[f"{VENDOR}/{name}"].get("contents", ""))
        except tomllib.TOMLDecodeError as error:
            raise VerificationError(f"generated manifest is invalid for {name}") from error
        declared = manifest.get("package", {})
        if declared.get("name") != name or declared.get("version") != package.version:
            raise VerificationError(f"generated Git crate {name} is not the locked package")
        _verify_checksum_file(checksum_files[f"{VENDOR}/{name}"], name, None)


def _verify_checksum_file(source: dict, vendor_name: str, checksum: str | None) -> None:
    try:
        checksum_document = json.loads(source.get("contents", ""))
    except json.JSONDecodeError as error:
        raise VerificationError(
            f"generated checksum is invalid for {vendor_name}"
        ) from error
    if checksum_document != {"package": checksum, "files": {}}:
        raise VerificationError(f"generated checksum mismatch for {vendor_name}")


def verify_offline_driver_text(text: str) -> None:
    online = re.search(
        r"(?ms)^prepare_online\(\) \{\n(.*?)^\}\n\nbuild_offline\(\) \{",
        text,
    )
    offline = re.search(
        r"(?ms)^build_offline\(\) \{\n(.*?)^\}\n\ncase \"\$mode\" in$",
        text,
    )
    if online is None or offline is None:
        raise VerificationError("Flatpak candidate phases are not explicit")
    online_body = online.group(1)
    offline_body = offline.group(1)
    if not all(
        token in online_body
        for token in (
            "--download-only",
            "--install-deps-from=flathub",
            "--user",
            "sources_cached\\ttrue",
        )
    ):
        raise VerificationError("Flatpak online preparation contract changed")
    if not all(
        token in offline_body
        for token in (
            "--disable-download",
            "--sandbox",
            "--repo=\"$staged_repo\"",
            "flatpak build-bundle",
            "online preparation does not match this candidate",
            "source_downloads\\tdisabled",
        )
    ):
        raise VerificationError("Flatpak offline build contract changed")
    if any(
        token in offline_body
        for token in (
            "--download-only",
            "--install-deps-from",
            "curl ",
            "wget ",
            "http://",
            "https://",
        )
    ):
        raise VerificationError("Flatpak offline phase can access a downloader")
    if (
        "flatpak run" in text
        or "flatpak install" in offline_body
        or re.search(r"(?m)(?:^|\s)--install(?:\s|=|$)", offline_body)
    ):
        raise VerificationError("Flatpak candidate builder must not launch or install")


def verify_offline_driver(path: Path) -> None:
    try:
        metadata = path.lstat()
        raw = path.read_bytes()
    except OSError as error:
        raise VerificationError("Flatpak candidate builder is unavailable") from error
    if (
        path.is_symlink()
        or not stat.S_ISREG(metadata.st_mode)
        or not metadata.st_mode & 0o111
        or not 0 < metadata.st_size <= MAX_DRIVER_BYTES
        or len(raw) != metadata.st_size
    ):
        raise VerificationError("Flatpak candidate builder is invalid")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise VerificationError("Flatpak candidate builder is not UTF-8") from error
    verify_offline_driver_text(text)


def verify_repository(root: Path) -> None:
    package = root / "packaging/flatpak"
    decisions = read_json(package / "decisions.json")
    manifest = read_json(package / "org.rmac.TextEditor.json")
    sources = read_json(package / "cargo-sources.json")
    verify_decisions(decisions)
    verify_manifest(manifest)
    verify_cargo_sources(
        sources,
        registry_packages(root / "Cargo.lock"),
        git_packages(root / "Cargo.lock"),
    )
    verify_offline_driver(root / "scripts/linux/build-flatpak-candidate.sh")
    for path in [
        root / "packaging/rmac-apps/applications/org.rmac.TextEditor.desktop",
        root / "packaging/rmac-apps/metainfo/org.rmac.TextEditor.metainfo.xml",
        root / "packaging/rmac-apps/icons/org.rmac.TextEditor.svg",
    ]:
        if not path.is_file() or path.is_symlink():
            raise VerificationError(f"required Flatpak metadata is unavailable: {path}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repository root",
    )
    arguments = parser.parse_args()
    try:
        verify_repository(arguments.root.resolve())
    except VerificationError as error:
        print(f"Flatpak package verification failed: {error}", file=sys.stderr)
        return 1
    print("Flatpak package verification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
