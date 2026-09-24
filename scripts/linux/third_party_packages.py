#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Pins, version rules, notices, and SBOMs for Lulo OS's niri builds.

Lulo OS builds niri and xwayland-satellite itself because neither is in the
Ubuntu 26.04 archive. The exact upstream sources are pinned in
packaging/third-party/upstreams.json (tag, commit, tarball SHA-256, and the
SHA-256 of the `cargo vendor` tarball once it has been recorded).
scripts/linux/build-niri-packages.sh drives the build; this module holds the
side-effect-free parts it and the tests share, plus a small CLI:

  shell-vars --name NAME         print the pin as shell assignments
  notices --vendor DIR --output FILE
                                 write the licence notices of every vendored crate
  sbom --name NAME --cargo-lock FILE --vendor-sha256 HEX --output FILE
       [--artifact PATH ...]     write a CycloneDX SBOM for one package
  check-vendor --name NAME --sha256 HEX
                                 compare a vendor tarball with its recorded pin

Packages keep the upstream names (niri, xwayland-satellite) with a Debian
version suffix of +luloN, never an epoch or a tilde; see docs/release-process.md
"Third-party packages" for why.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import sys


REPO_ROOT = Path(__file__).resolve().parents[2]
PINS_PATH = Path("packaging/third-party/upstreams.json")
PACKAGE_NAMES = ("niri", "xwayland-satellite")
MAX_NOTICE_FILE_BYTES = 256 * 1024
LICENSE_FILE = re.compile(r"(?i)^(licen[cs]e|copying|notice|unlicense|copyright)([-._].*)?$")


class ThirdPartyError(RuntimeError):
    """A deterministic third-party packaging contract failure."""


@dataclass(frozen=True)
class Pin:
    name: str
    upstream_version: str
    debian_revision: str
    tag: str
    commit: str
    tarball_url: str
    tarball_sha256: str
    tarball_top_directory: str
    vendor_sha256: str | None
    license: str
    homepage: str
    cargo_package: str

    @property
    def debian_version(self) -> str:
        # No hyphen: dpkg compares the upstream_version *component* of a
        # hyphenated version ("26.04" here) on its own before it ever looks
        # at a revision after a hyphen, and a bare "26.04" already sorts
        # below the danklinux PPA's "26.04ppaN" (letters sort below the "+"
        # that follows them). Folding "+luloN" into the upstream-version
        # component itself -- Debian's usual "+dfsg"/"+repack" pattern -- is
        # the only way to sort above the PPA. See docs/release-process.md
        # "Package names and versions".
        return f"{self.upstream_version}+{self.debian_revision}"

    @property
    def orig_tarball(self) -> str:
        # Named after the *full* debian_version, not the bare upstream tag:
        # with no hyphen in the version, "26.04+lulo1" is the whole
        # upstream-version component dpkg-source expects the orig tarball's
        # name to carry.
        return f"{self.name}_{self.debian_version}.orig.tar.gz"

    @property
    def vendor_tarball(self) -> str:
        return f"{self.name}_{self.debian_version}.orig-vendor.tar.xz"


_FIELD_PATTERNS = {
    "upstream_version": r"[0-9]+(?:\.[0-9]+)+",
    # "+luloN" sorts above a bare upstream version, above any official
    # Debian (-1) or Ubuntu (-0ubuntu1) revision, and above the danklinux
    # PPA's "<version>ppaN" (letters sort below "+"), while a future upstream
    # release ("26.05") still sorts above it, since the minor-version digits
    # differ before "+luloN" is ever compared. No tilde: GitHub renames "~"
    # in Release asset names. No epoch: unneeded here, and an epoch is
    # permanent and highly visible in `apt policy`/`dpkg -l` for something
    # that only exists to out-rank a PPA. See docs/release-process.md
    # "Package names and versions".
    "debian_revision": r"lulo[1-9][0-9]*",
    "tag": r"v[0-9]+(?:\.[0-9]+)+",
    "commit": r"[0-9a-f]{40}",
    "tarball_url": r"https://github\.com/[A-Za-z0-9-]+/[A-Za-z0-9._-]+/archive/refs/tags/v[0-9.]+\.tar\.gz",
    "tarball_sha256": r"[0-9a-f]{64}",
    "tarball_top_directory": r"[a-z0-9-]+-[0-9]+(?:\.[0-9]+)+",
    "license": r"GPL-3\.0-or-later|MPL-2\.0",
    "homepage": r"https://github\.com/[A-Za-z0-9-]+/[A-Za-z0-9._-]+",
    "cargo_package": r"[a-z0-9-]+",
}


def load_pins(repo_root: Path = REPO_ROOT) -> dict[str, Pin]:
    try:
        document = json.loads((repo_root / PINS_PATH).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ThirdPartyError("third-party pin file is unreadable") from error
    if not isinstance(document, dict) or document.get("format") != 1:
        raise ThirdPartyError("third-party pin file format is not 1")
    packages = document.get("packages")
    if not isinstance(packages, dict) or tuple(sorted(packages)) != PACKAGE_NAMES:
        raise ThirdPartyError("third-party pin file must pin exactly niri and xwayland-satellite")
    pins = {}
    for name in PACKAGE_NAMES:
        entry = packages[name]
        expected = set(_FIELD_PATTERNS) | {"vendor_sha256"}
        if not isinstance(entry, dict) or set(entry) != expected:
            raise ThirdPartyError(f"{name}: pin fields are not exact")
        for field, pattern in _FIELD_PATTERNS.items():
            value = entry[field]
            if not isinstance(value, str) or not re.fullmatch(pattern, value):
                raise ThirdPartyError(f"{name}: {field} is invalid")
        vendor = entry["vendor_sha256"]
        if vendor is not None and (
            not isinstance(vendor, str) or not re.fullmatch(r"[0-9a-f]{64}", vendor)
        ):
            raise ThirdPartyError(f"{name}: vendor_sha256 is invalid")
        pin = Pin(name=name, **entry)
        if pin.tag != f"v{pin.upstream_version}":
            raise ThirdPartyError(f"{name}: tag does not match the upstream version")
        if pin.tarball_top_directory != f"{name}-{pin.upstream_version}":
            raise ThirdPartyError(f"{name}: tarball directory does not match")
        if not pin.tarball_url.startswith(pin.homepage + "/archive/refs/tags/"):
            raise ThirdPartyError(f"{name}: tarball URL is not the pinned homepage's tag")
        if not pin.tarball_url.endswith(f"/{pin.tag}.tar.gz"):
            raise ThirdPartyError(f"{name}: tarball URL does not name the pinned tag")
        pins[name] = pin
    return pins


# --- Debian version comparison (dpkg's algorithm, deb-version(7)) ---------


def _order(character: str) -> int:
    # An exhausted string reads as "", which orders like dpkg's NUL.
    if character == "" or character.isdigit():
        return 0
    if character == "~":
        return -1
    if character.isascii() and character.isalpha():
        return ord(character)
    return ord(character) + 256


def _is_digit(character: str) -> bool:
    return character != "" and character in "0123456789"


def _compare_fragment(left: str, right: str) -> int:
    """A direct port of dpkg's verrevcmp()."""

    def at(text: str, index: int) -> str:
        return text[index] if index < len(text) else ""

    i = j = 0
    while at(left, i) or at(right, j):
        first_difference = 0
        while (at(left, i) and not _is_digit(at(left, i))) or (
            at(right, j) and not _is_digit(at(right, j))
        ):
            a = _order(at(left, i))
            b = _order(at(right, j))
            if a != b:
                return a - b
            i += 1
            j += 1
        while at(left, i) == "0":
            i += 1
        while at(right, j) == "0":
            j += 1
        while _is_digit(at(left, i)) and _is_digit(at(right, j)):
            if not first_difference:
                first_difference = ord(left[i]) - ord(right[j])
            i += 1
            j += 1
        if _is_digit(at(left, i)):
            return 1
        if _is_digit(at(right, j)):
            return -1
        if first_difference:
            return first_difference
    return 0


def _split_version(version: str) -> tuple[int, str, str]:
    epoch = 0
    if ":" in version:
        raw_epoch, version = version.split(":", 1)
        epoch = int(raw_epoch)
    if "-" in version:
        upstream, revision = version.rsplit("-", 1)
    else:
        upstream, revision = version, ""
    return epoch, upstream, revision


def compare_versions(left: str, right: str) -> int:
    """Return <0, 0, or >0 exactly as `dpkg --compare-versions` orders them."""
    left_epoch, left_upstream, left_revision = _split_version(left)
    right_epoch, right_upstream, right_revision = _split_version(right)
    if left_epoch != right_epoch:
        return left_epoch - right_epoch
    result = _compare_fragment(left_upstream, right_upstream)
    if result:
        return result
    return _compare_fragment(left_revision, right_revision)


# --- Cargo.lock, notices, and SBOM ---------------------------------------


def parse_cargo_lock(text: str) -> list[dict[str, str]]:
    """Read the [[package]] entries of a Cargo.lock (v3/v4) without tomllib."""
    packages: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == "[[package]]":
            current = {}
            packages.append(current)
            continue
        if stripped.startswith("["):
            current = None
            continue
        if current is None:
            continue
        match = re.fullmatch(r'(name|version|source|checksum) = "([^"\\]*)"', stripped)
        if match:
            current[match.group(1)] = match.group(2)
    for package in packages:
        if "name" not in package or "version" not in package:
            raise ThirdPartyError("Cargo.lock has a package without a name or version")
    return sorted(packages, key=lambda item: (item["name"], item["version"]))


def _manifest_field(manifest: str, field: str) -> str | None:
    section = re.search(r"(?ms)^\[package\]\s*$(.*?)(?=^\[|\Z)", manifest)
    if section is None:
        return None
    match = re.search(rf'(?m)^{re.escape(field)}\s*=\s*"((?:[^"\\]|\\.)*)"\s*$', section.group(1))
    return match.group(1) if match else None


def dependency_notices(vendor_dir: Path, heading: str) -> str:
    """Collect every vendored crate's licence expression and licence texts."""
    if not vendor_dir.is_dir():
        raise ThirdPartyError("vendor directory is missing")
    parts = [
        heading,
        "",
        "This file lists every Rust crate vendored into the source package, with",
        "the licence it declares and the licence and notice files it ships. Some",
        "vendored crates only build for other platforms and are not linked in.",
        "",
    ]
    for crate_dir in sorted(p for p in vendor_dir.iterdir() if p.is_dir() and not p.name.startswith(".")):
        manifest_path = crate_dir / "Cargo.toml"
        if not manifest_path.is_file():
            continue
        manifest = manifest_path.read_text(encoding="utf-8", errors="replace")
        name = _manifest_field(manifest, "name") or crate_dir.name
        version = _manifest_field(manifest, "version") or "unknown"
        license_expression = _manifest_field(manifest, "license")
        license_file = _manifest_field(manifest, "license-file")
        parts.append(f"======== {name} {version} ========")
        parts.append(f"License: {license_expression or 'see licence file'}")
        texts = sorted(
            path for path in crate_dir.iterdir()
            if path.is_file() and LICENSE_FILE.match(path.name)
        )
        if license_file:
            declared = crate_dir / license_file
            if declared.is_file() and declared not in texts:
                texts.append(declared)
        if not texts and not license_expression:
            raise ThirdPartyError(f"vendored crate {name} {version} declares no licence")
        for text_path in texts:
            raw = text_path.read_bytes()[:MAX_NOTICE_FILE_BYTES]
            parts.append(f"-------- {text_path.relative_to(crate_dir)} --------")
            parts.append(raw.decode("utf-8", errors="replace").rstrip("\n"))
        parts.append("")
    return "\n".join(parts) + "\n"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cyclonedx_sbom(
    pin: Pin,
    lock_packages: list[dict[str, str]],
    vendor_sha256: str,
    artifacts: dict[str, str],
) -> dict:
    """A deterministic CycloneDX 1.5 document for one Lulo OS package."""
    main_ref = f"pkg:deb/lulo/{pin.name}@{pin.debian_version}"
    components = []
    for package in lock_packages:
        component = {
            "type": "library",
            "name": package["name"],
            "version": package["version"],
            "purl": f"pkg:cargo/{package['name']}@{package['version']}",
            "bom-ref": f"pkg:cargo/{package['name']}@{package['version']}",
        }
        if "checksum" in package:
            component["hashes"] = [{"alg": "SHA-256", "content": package["checksum"]}]
        source = package.get("source", "")
        if source.startswith("git+"):
            component["externalReferences"] = [{"type": "vcs", "url": source[4:]}]
        components.append(component)
    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": pin.name,
                "version": pin.debian_version,
                "bom-ref": main_ref,
                "purl": main_ref,
                "licenses": [{"license": {"id": pin.license}}],
                "externalReferences": [
                    {"type": "vcs", "url": f"{pin.homepage}.git", "comment": f"{pin.tag} {pin.commit}"},
                    {
                        "type": "distribution",
                        "url": pin.tarball_url,
                        "hashes": [{"alg": "SHA-256", "content": pin.tarball_sha256}],
                    },
                ],
                "properties": [
                    {"name": "lulo:upstream-commit", "value": pin.commit},
                    {"name": "lulo:vendor-tarball-sha256", "value": vendor_sha256},
                    {"name": "lulo:components-scope", "value": "every Cargo.lock entry; a superset of what is linked"},
                ]
                + [
                    {"name": f"lulo:artifact-sha256:{name}", "value": digest}
                    for name, digest in sorted(artifacts.items())
                ],
            }
        },
        "components": components,
        "dependencies": [
            {"ref": main_ref, "dependsOn": [component["bom-ref"] for component in components]}
        ],
    }


def shell_assignments(pin: Pin) -> str:
    values = {
        "PIN_NAME": pin.name,
        "PIN_UPSTREAM_VERSION": pin.upstream_version,
        "PIN_DEBIAN_VERSION": pin.debian_version,
        "PIN_TAG": pin.tag,
        "PIN_COMMIT": pin.commit,
        "PIN_TARBALL_URL": pin.tarball_url,
        "PIN_TARBALL_SHA256": pin.tarball_sha256,
        "PIN_TOP_DIRECTORY": pin.tarball_top_directory,
        "PIN_VENDOR_SHA256": pin.vendor_sha256 or "",
        "PIN_CARGO_PACKAGE": pin.cargo_package,
        "PIN_ORIG_TARBALL": pin.orig_tarball,
        "PIN_VENDOR_TARBALL": pin.vendor_tarball,
    }
    lines = []
    for key, value in values.items():
        if not re.fullmatch(r"[A-Za-z0-9._:/~+-]*", value):
            raise ThirdPartyError(f"{key} is not shell-safe")
        lines.append(f"{key}='{value}'")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    shell = commands.add_parser("shell-vars")
    shell.add_argument("--name", required=True, choices=PACKAGE_NAMES)
    notices = commands.add_parser("notices")
    notices.add_argument("--name", required=True, choices=PACKAGE_NAMES)
    notices.add_argument("--vendor", required=True, type=Path)
    notices.add_argument("--output", required=True, type=Path)
    sbom = commands.add_parser("sbom")
    sbom.add_argument("--name", required=True, choices=PACKAGE_NAMES)
    sbom.add_argument("--cargo-lock", required=True, type=Path)
    sbom.add_argument("--vendor-sha256", required=True)
    sbom.add_argument("--output", required=True, type=Path)
    sbom.add_argument("--artifact", action="append", default=[], type=Path)
    check = commands.add_parser("check-vendor")
    check.add_argument("--name", required=True, choices=PACKAGE_NAMES)
    check.add_argument("--sha256", required=True)
    arguments = parser.parse_args(argv)

    try:
        pins = load_pins()
        pin = pins[arguments.name]
        if arguments.command == "shell-vars":
            sys.stdout.write(shell_assignments(pin))
        elif arguments.command == "notices":
            heading = (
                f"Lulo OS build of {pin.name} {pin.debian_version} "
                f"(upstream {pin.tag}, commit {pin.commit})"
            )
            arguments.output.write_text(
                dependency_notices(arguments.vendor, heading), encoding="utf-8"
            )
        elif arguments.command == "sbom":
            if not re.fullmatch(r"[0-9a-f]{64}", arguments.vendor_sha256):
                raise ThirdPartyError("vendor SHA-256 is invalid")
            lock = parse_cargo_lock(arguments.cargo_lock.read_text(encoding="utf-8"))
            artifacts = {path.name: sha256_file(path) for path in arguments.artifact}
            document = cyclonedx_sbom(pin, lock, arguments.vendor_sha256, artifacts)
            arguments.output.write_text(
                json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
        elif arguments.command == "check-vendor":
            if pin.vendor_sha256 is None:
                print(
                    f"{pin.name}: vendor tarball SHA-256 is not pinned yet; record "
                    f'"vendor_sha256": "{arguments.sha256}" in {PINS_PATH} after review',
                    file=sys.stderr,
                )
            elif pin.vendor_sha256 != arguments.sha256:
                raise ThirdPartyError(
                    f"{pin.name}: vendor tarball is {arguments.sha256}, "
                    f"but {PINS_PATH} pins {pin.vendor_sha256}"
                )
    except (ThirdPartyError, OSError) as error:
        print(f"third_party_packages: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
