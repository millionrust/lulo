#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Side-effect-free Debian archive helpers shared by the rmac APT tools.

`stage-apt-snapshot.py` and `apt-publication.py` both need to read binary
package control data, parse deb822 files (`.dsc`, `Packages`, `Sources`),
compare Debian versions, and unpack release archives without trusting their
member names. Everything here reads bytes and returns plain values; nothing
signs, publishes, or talks to the network.
"""

from __future__ import annotations

import gzip
import io
import lzma
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tarfile
from typing import Dict, List, Optional, Tuple

import third_party_packages


MAX_CONTROL_BYTES = 1024 * 1024
MAX_DEB822_BYTES = 16 * 1024 * 1024
FIELD_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]*")
PACKAGE_RE = re.compile(r"[a-z0-9][a-z0-9+.-]+")
VERSION_RE = re.compile(r"[0-9A-Za-z.+:~_-]{1,128}")
ARCHITECTURE_RE = re.compile(r"[a-z0-9-]+")


class ArchiveError(RuntimeError):
    """A bounded, privacy-safe archive parsing failure."""


# --- Debian versions ----------------------------------------------------------


def compare_versions(left: str, right: str) -> int:
    """Order two Debian versions exactly like `dpkg --compare-versions`."""
    for value in (left, right):
        if not VERSION_RE.fullmatch(value):
            raise ArchiveError("Debian version is not canonical")
    return third_party_packages.compare_versions(left, right)


def version_without_epoch(version: str) -> str:
    return version.split(":", 1)[1] if ":" in version else version


def upstream_version(version: str) -> str:
    value = version_without_epoch(version)
    return value.rsplit("-", 1)[0] if "-" in value else value


# --- deb822 ---------------------------------------------------------------------


def strip_clearsign(text: str) -> str:
    """Return the cleartext of an OpenPGP clearsigned document, else the text.

    Only used for `.dsc` files, whose own signature (if any) is not what rmac
    trusts: every source file is bound by hashes in the signed Release chain.
    """
    if not text.startswith("-----BEGIN PGP SIGNED MESSAGE-----"):
        return text
    lines = text.splitlines()
    try:
        start = lines.index("") + 1
        end = lines.index("-----BEGIN PGP SIGNATURE-----")
    except ValueError as error:
        raise ArchiveError("clearsigned document is malformed") from error
    body = [line[2:] if line.startswith("- ") else line for line in lines[start:end]]
    return "\n".join(body) + "\n"


def parse_deb822(text: str, label: str) -> List[Dict[str, str]]:
    """Parse deb822 paragraphs, keeping field order and continuation lines.

    A continuation value keeps its lines joined by "\\n" with the single
    leading space removed, which `format_paragraph` reverses exactly.
    """
    if "\r" in text or "\x00" in text:
        raise ArchiveError(f"{label} encoding is invalid")
    paragraphs: List[Dict[str, str]] = []
    current: Dict[str, str] = {}
    field: Optional[str] = None
    for line in text.splitlines():
        if not line.strip():
            if current:
                paragraphs.append(current)
                current = {}
                field = None
            continue
        if line.startswith("#"):
            continue
        if line[0] in " \t":
            if field is None:
                raise ArchiveError(f"{label} continuation has no field")
            current[field] = current[field] + "\n" + line[1:] if current[field] else line[1:]
            continue
        name, separator, value = line.partition(":")
        if not separator or not FIELD_RE.fullmatch(name) or name in current:
            raise ArchiveError(f"{label} field inventory is invalid")
        current[name] = value.strip()
        field = name
    if current:
        paragraphs.append(current)
    return paragraphs


def format_paragraph(fields: List[Tuple[str, str]]) -> str:
    lines: List[str] = []
    for name, value in fields:
        if not FIELD_RE.fullmatch(name):
            raise ArchiveError("deb822 field name is invalid")
        if "\n" in value:
            first, *rest = value.split("\n")
            lines.append(f"{name}: {first}" if first else f"{name}:")
            lines.extend(f" {line}" for line in rest)
        else:
            lines.append(f"{name}: {value}")
    return "\n".join(lines)


def read_deb822_file(path: Path, label: str) -> List[Dict[str, str]]:
    raw = read_regular(path, MAX_DEB822_BYTES, label)
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ArchiveError(f"{label} is not UTF-8") from error
    return parse_deb822(strip_clearsign(text), label)


def checksum_rows(value: str, algorithm: str) -> List[Tuple[str, int, str]]:
    """Parse a `Checksums-*` field: (digest, size, filename) rows."""
    width = {"Sha1": 40, "Sha256": 64, "Sha512": 128, "Files": 32}[algorithm]
    rows: List[Tuple[str, int, str]] = []
    for line in value.splitlines():
        parts = line.split()
        if not parts:
            continue
        if (
            len(parts) != 3
            or not re.fullmatch(rf"[0-9a-f]{{{width}}}", parts[0])
            or not parts[1].isdecimal()
            or "/" in parts[2]
            or parts[2] in {".", ".."}
        ):
            raise ArchiveError(f"{algorithm} checksum inventory is invalid")
        rows.append((parts[0], int(parts[1]), parts[2]))
    return rows


# --- files ----------------------------------------------------------------------


def read_regular(path: Path, maximum: int, label: str) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ArchiveError(f"{label} is unavailable") from error
    if not stat.S_ISREG(metadata.st_mode):
        raise ArchiveError(f"{label} must be a regular file")
    if metadata.st_size > maximum:
        raise ArchiveError(f"{label} exceeds its size limit")
    try:
        return path.read_bytes()
    except OSError as error:
        raise ArchiveError(f"{label} cannot be read") from error


def pool_directory(source: str) -> str:
    """Debian's pool layout: pool/main/<initial>/<source>."""
    if not PACKAGE_RE.fullmatch(source):
        raise ArchiveError("source package name is invalid")
    initial = source[:4] if source.startswith("lib") and len(source) > 3 else source[0]
    return f"pool/main/{initial}/{source}"


# --- binary packages ------------------------------------------------------------


def _parse_ar(value: bytes) -> List[Tuple[str, bytes]]:
    if not value.startswith(b"!<arch>\n"):
        raise ArchiveError("binary package is not an ar archive")
    offset = 8
    members: List[Tuple[str, bytes]] = []
    while offset < len(value):
        if offset + 60 > len(value):
            raise ArchiveError("binary package ar header is truncated")
        header = value[offset : offset + 60]
        offset += 60
        if header[58:60] != b"`\n":
            raise ArchiveError("binary package ar header is invalid")
        try:
            name = header[:16].decode("ascii").strip()
            if name.endswith("/"):
                name = name[:-1]
            size = int(header[48:58].decode("ascii").strip())
        except (UnicodeDecodeError, ValueError) as error:
            raise ArchiveError("binary package ar metadata is invalid") from error
        if not name or size < 0 or offset + size > len(value):
            raise ArchiveError("binary package ar inventory is invalid")
        members.append((name, value[offset : offset + size]))
        offset += size + size % 2
    return members


def _decompress(name: str, value: bytes) -> Optional[bytes]:
    try:
        if name.endswith(".xz"):
            return lzma.decompress(value, format=lzma.FORMAT_XZ, memlimit=64 * 1024 * 1024)
        if name.endswith(".gz"):
            return gzip.decompress(value)
        if name.endswith(".tar"):
            return value
    except (lzma.LZMAError, OSError, EOFError) as error:
        raise ArchiveError("binary package control archive is corrupt") from error
    return None


def _control_from_dpkg_deb(path: Path) -> str:
    tool = shutil.which("dpkg-deb")
    if tool is None:
        raise ArchiveError("dpkg-deb is required for this control compression")
    try:
        result = subprocess.run(
            [tool, "--field", str(path)],
            check=False,
            capture_output=True,
            timeout=60,
            env={"LC_ALL": "C", "PATH": "/usr/bin:/bin"},
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ArchiveError("dpkg-deb could not read a binary package") from error
    if result.returncode != 0 or len(result.stdout) > MAX_CONTROL_BYTES:
        raise ArchiveError("dpkg-deb could not read a binary package")
    return result.stdout.decode("utf-8")


def deb_control(path: Path, maximum: int = 2 * 1024 * 1024 * 1024) -> Dict[str, str]:
    """Return the control paragraph of one binary package.

    xz, gzip, and uncompressed control members are read directly; anything
    else (zstd) is delegated to `dpkg-deb --field`, which prints the same
    deb822 control paragraph.
    """
    value = read_regular(path, maximum, "binary package")
    members = _parse_ar(value)
    if len(members) < 3 or members[0] != ("debian-binary", b"2.0\n"):
        raise ArchiveError("binary package member inventory is invalid")
    name, compressed = members[1]
    if not name.startswith("control.tar"):
        raise ArchiveError("binary package has no control archive")
    raw = _decompress(name, compressed)
    if raw is None:
        text = _control_from_dpkg_deb(path)
    else:
        if len(raw) > 64 * 1024 * 1024:
            raise ArchiveError("binary package control archive is oversized")
        try:
            with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
                member = next(
                    (
                        item
                        for item in archive.getmembers()
                        if item.name in {"./control", "control"} and item.isfile()
                    ),
                    None,
                )
                if member is None or member.size > MAX_CONTROL_BYTES:
                    raise ArchiveError("binary package control file is missing")
                source = archive.extractfile(member)
                if source is None:
                    raise ArchiveError("binary package control file is unreadable")
                text = source.read().decode("utf-8")
        except (tarfile.TarError, UnicodeDecodeError) as error:
            raise ArchiveError("binary package control archive is invalid") from error
    paragraphs = parse_deb822(text, "binary package control")
    if len(paragraphs) != 1:
        raise ArchiveError("binary package control must be one paragraph")
    control = paragraphs[0]
    for field in ("Package", "Version", "Architecture", "Maintainer", "Description"):
        if not control.get(field):
            raise ArchiveError(f"binary package control lacks {field}")
    if (
        not PACKAGE_RE.fullmatch(control["Package"])
        or not VERSION_RE.fullmatch(control["Version"])
        or not ARCHITECTURE_RE.fullmatch(control["Architecture"])
    ):
        raise ArchiveError("binary package identity is invalid")
    return control


def deb_filename(control: Dict[str, str]) -> str:
    return (
        f"{control['Package']}_{version_without_epoch(control['Version'])}"
        f"_{control['Architecture']}.deb"
    )


def binary_source_name(control: Dict[str, str]) -> str:
    """The source package a binary was built from (Source: may carry a version)."""
    value = control.get("Source", control["Package"]).split()[0]
    if not PACKAGE_RE.fullmatch(value):
        raise ArchiveError("binary package Source is invalid")
    return value


# --- archives -------------------------------------------------------------------


def safe_extract_tar(archive_path: Path, destination: Path, maximum_bytes: int) -> List[str]:
    """Extract only regular files and directories under `destination`.

    Links, devices, absolute names, `..`, duplicate names, and archives that
    expand past `maximum_bytes` are refused. Returns the extracted file names.
    """
    extracted: List[str] = []
    total = 0
    seen = set()
    try:
        with tarfile.open(archive_path, mode="r:*") as archive:
            for member in archive:
                name = member.name
                while name.startswith("./"):
                    name = name[2:]
                name = name.rstrip("/")
                if name in {"", "."}:
                    if not member.isdir():
                        raise ArchiveError("archive root is not a directory")
                    continue
                path = PurePosixPath(name)
                if (
                    path.is_absolute()
                    or any(part in {"", ".", ".."} for part in path.parts)
                    or "\\" in name
                    or "\x00" in name
                    or name in seen
                ):
                    raise ArchiveError("archive member name is unsafe")
                seen.add(name)
                target = destination / name
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                if not member.isfile():
                    raise ArchiveError("archive contains a link or special file")
                total += member.size
                if total > maximum_bytes:
                    raise ArchiveError("archive expands beyond its limit")
                target.parent.mkdir(parents=True, exist_ok=True)
                source = archive.extractfile(member)
                if source is None:
                    raise ArchiveError("archive member is unreadable")
                with source, target.open("xb") as output:
                    shutil.copyfileobj(source, output, 1024 * 1024)
                target.chmod(0o644)
                extracted.append(name)
    except (OSError, tarfile.TarError) as error:
        raise ArchiveError("archive cannot be extracted safely") from error
    return extracted
