#!/usr/bin/env python3
"""Verify the I2 visual inventory and optional reviewed PNG references."""

from __future__ import annotations

import argparse
import binascii
import hashlib
import json
from pathlib import Path
import re
import stat
import struct
import zlib


MANIFEST_PATH = Path(__file__).with_name("visual-suite.json")
MAX_JSON_BYTES = 1024 * 1024
MAX_PNG_BYTES = 32 * 1024 * 1024
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
FORBIDDEN_CHUNKS = {b"eXIf", b"iTXt", b"tEXt", b"tIME", b"zTXt"}


class VisualError(RuntimeError):
    """A bounded, privacy-safe visual-reference failure."""


def _regular_bytes(path: Path, maximum: int) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VisualError(f"required visual file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VisualError(f"required visual path is not regular: {path.name}")
    if metadata.st_size > maximum:
        raise VisualError(f"required visual file is too large: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise VisualError(f"required visual file cannot be read: {path.name}") from error
    if len(raw) != metadata.st_size:
        raise VisualError(f"required visual file changed while reading: {path.name}")
    return raw


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, object]:
    raw = _regular_bytes(path, MAX_JSON_BYTES)
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VisualError("visual manifest is invalid JSON") from error
    if not isinstance(document, dict) or set(document) != {
        "capture",
        "format",
        "scales",
        "screens",
        "themes",
    }:
        raise VisualError("visual manifest fields are not exact")
    if document["format"] != 1 or type(document["format"]) is not int:
        raise VisualError("visual manifest format is invalid")
    if document["capture"] != {
        "clock_utc": "2026-01-15T10:09:00Z",
        "font_antialiasing": "grayscale",
        "fonts": ["Inter", "JetBrains Mono"],
        "locale": "en_US.UTF-8",
        "seed": 20260730,
        "timezone": "UTC",
    }:
        raise VisualError("visual capture authority differs")
    if document["scales"] != [100, 125, 150, 200] or document["themes"] != [
        "dark",
        "dark-contrast",
        "light",
        "light-contrast",
    ]:
        raise VisualError("visual theme or scale inventory differs")
    screens = document["screens"]
    if not isinstance(screens, list) or len(screens) != 27:
        raise VisualError("critical-screen inventory is invalid")
    ids = []
    groups = set()
    for screen in screens:
        if not isinstance(screen, dict) or set(screen) != {
            "group",
            "id",
            "viewport",
        }:
            raise VisualError("critical-screen fields are invalid")
        if (
            screen["group"] not in {"app", "settings", "shell"}
            or not isinstance(screen["id"], str)
            or not re.fullmatch(r"[a-z]+(?:-[a-z]+)*", screen["id"])
            or not isinstance(screen["viewport"], list)
            or len(screen["viewport"]) != 2
            or any(type(value) is not int or value < 640 or value > 1600 for value in screen["viewport"])
        ):
            raise VisualError("critical-screen contract is invalid")
        ids.append(screen["id"])
        groups.add(screen["group"])
    if ids != sorted(set(ids)) or groups != {"app", "settings", "shell"}:
        raise VisualError("critical-screen identity or group coverage is invalid")
    return document


def expected_images(manifest: dict[str, object]) -> dict[str, tuple[int, int]]:
    images = {}
    for screen in manifest["screens"]:
        width, height = screen["viewport"]
        for theme in manifest["themes"]:
            for scale in manifest["scales"]:
                name = f"{screen['id']}--{theme}--{scale}.png"
                images[name] = (width * scale // 100, height * scale // 100)
    return images


def png_dimensions(raw: bytes) -> tuple[int, int]:
    if not raw.startswith(PNG_SIGNATURE):
        raise VisualError("visual reference is not PNG")
    offset = len(PNG_SIGNATURE)
    dimensions = None
    color_type = None
    compressed = []
    saw_end = False
    chunk_index = 0
    while offset < len(raw):
        if offset + 12 > len(raw):
            raise VisualError("visual PNG chunk is truncated")
        length = struct.unpack_from(">I", raw, offset)[0]
        chunk_type = raw[offset + 4 : offset + 8]
        end = offset + 12 + length
        if end > len(raw):
            raise VisualError("visual PNG chunk length is invalid")
        data = raw[offset + 8 : offset + 8 + length]
        expected_crc = struct.unpack_from(">I", raw, offset + 8 + length)[0]
        if binascii.crc32(chunk_type + data) & 0xFFFFFFFF != expected_crc:
            raise VisualError("visual PNG chunk checksum differs")
        if chunk_type in FORBIDDEN_CHUNKS:
            raise VisualError("visual PNG contains private or nondeterministic metadata")
        if chunk_index == 0 and chunk_type != b"IHDR":
            raise VisualError("visual PNG header is not first")
        if chunk_type == b"IHDR":
            if dimensions is not None or length != 13:
                raise VisualError("visual PNG header is invalid")
            width, height, depth, color, compression, filtering, interlace = struct.unpack(
                ">IIBBBBB", data
            )
            if (
                width == 0
                or height == 0
                or depth != 8
                or color not in {2, 6}
                or compression != 0
                or filtering != 0
                or interlace != 0
            ):
                raise VisualError("visual PNG encoding is not canonical")
            dimensions = (width, height)
            color_type = color
        elif chunk_type == b"IDAT":
            compressed.append(data)
        elif chunk_type == b"IEND":
            if length != 0 or end != len(raw):
                raise VisualError("visual PNG terminator is invalid")
            saw_end = True
        offset = end
        chunk_index += 1
    if dimensions is None or not compressed or not saw_end:
        raise VisualError("visual PNG structure is incomplete")
    width, height = dimensions
    row_bytes = width * (3 if color_type == 2 else 4)
    expected_size = height * (row_bytes + 1)
    if expected_size > 64 * 1024 * 1024:
        raise VisualError("visual PNG decoded size is excessive")
    try:
        pixels = zlib.decompress(b"".join(compressed))
    except zlib.error as error:
        raise VisualError("visual PNG pixel stream is invalid") from error
    if len(pixels) != expected_size or any(
        pixels[row * (row_bytes + 1)] > 4 for row in range(height)
    ):
        raise VisualError("visual PNG scanlines are invalid")
    return dimensions


def verify_references(
    manifest: dict[str, object],
    directory: Path,
    *,
    revision: str,
) -> None:
    if (
        not directory.is_absolute()
        or directory.is_symlink()
        or not directory.is_dir()
    ):
        raise VisualError("visual reference directory must be absolute and ordinary")
    expected = expected_images(manifest)
    inventory = set(expected) | {"visual-review.json"}
    try:
        actual = {path.name for path in directory.iterdir()}
    except OSError as error:
        raise VisualError("visual reference inventory cannot be inspected") from error
    if actual != inventory:
        raise VisualError("visual reference inventory is not exact")
    review_raw = _regular_bytes(directory / "visual-review.json", MAX_JSON_BYTES)
    try:
        review = json.loads(review_raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VisualError("visual review is invalid JSON") from error
    if not isinstance(review, dict) or set(review) != {
        "font_sha256",
        "format",
        "manifest_sha256",
        "results",
        "revision",
    }:
        raise VisualError("visual review fields are not exact")
    manifest_hash = hashlib.sha256(
        _regular_bytes(MANIFEST_PATH, MAX_JSON_BYTES)
    ).hexdigest()
    if (
        review["format"] != 1
        or type(review["format"]) is not int
        or review["revision"] != revision
        or review["manifest_sha256"] != manifest_hash
        or not isinstance(review["font_sha256"], dict)
        or set(review["font_sha256"]) != {"Inter", "JetBrains Mono"}
        or any(
            not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value)
            for value in review["font_sha256"].values()
        )
    ):
        raise VisualError("visual review identity is invalid")
    expected_results = []
    for name, dimensions in sorted(expected.items()):
        raw = _regular_bytes(directory / name, MAX_PNG_BYTES)
        if png_dimensions(raw) != dimensions:
            raise VisualError(f"visual reference dimensions differ: {name}")
        expected_results.append(
            {
                "file": name,
                "sha256": hashlib.sha256(raw).hexdigest(),
                "status": "pass",
            }
        )
    if review["results"] != expected_results:
        raise VisualError("visual review does not prove every exact reference")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--references", type=Path)
    parser.add_argument("--revision")
    arguments = parser.parse_args()
    try:
        manifest = load_manifest()
        if arguments.references is not None:
            if not re.fullmatch(r"[0-9a-f]{40}", arguments.revision or ""):
                raise VisualError(
                    "an exact 40-hex --revision is required with references"
                )
            verify_references(
                manifest,
                arguments.references,
                revision=arguments.revision,
            )
    except VisualError as error:
        parser.exit(4, f"verify-visual-suite: {error}\n")
    print(f"rmac visual suite verified ({len(expected_images(manifest))} references)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
