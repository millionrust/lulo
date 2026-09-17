#!/usr/bin/env python3
"""Compose an rmac / reference-Mac screenshot pair side by side with labels.

Usage:
  python3 scripts/compose-evidence.py rmac.png macos.png out.png \
      [--rmac-label "rmac @ <commit>"] [--macos-label "macOS 26"]

The two screenshots are aligned at the top under a label band. The rmac label
defaults to the current short commit; the macOS label defaults to "macOS".
Inputs and the output must be regular files inside the repository; inputs are
size-bounded so a stray capture cannot exhaust memory.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys

MAX_INPUT_BYTES = 64 * 1024 * 1024
LABEL_HEIGHT = 28
GAP = 12
PADDING = 8
BACKGROUND = (24, 24, 27, 255)
FOREGROUND = (245, 245, 247, 255)


class ComposeError(RuntimeError):
    """A bounded evidence-composition failure."""


def _load_image(path: Path):
    from PIL import Image

    if path.is_symlink() or not path.is_file():
        raise ComposeError(f"input is not a regular file: {path.name}")
    if path.stat().st_size > MAX_INPUT_BYTES:
        raise ComposeError(f"input exceeds the size budget: {path.name}")
    try:
        return Image.open(path).convert("RGBA")
    except OSError as error:
        raise ComposeError(f"input cannot be decoded: {path.name}") from error


def default_rmac_label(repo_root: Path) -> str:
    try:
        commit = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        commit = ""
    return f"rmac @ {commit}" if commit else "rmac"


def compose(
    rmac_path: Path,
    macos_path: Path,
    output_path: Path,
    rmac_label: str,
    macos_label: str,
) -> Path:
    from PIL import Image, ImageDraw

    rmac = _load_image(rmac_path)
    macos = _load_image(macos_path)
    height = max(rmac.height, macos.height)
    width = rmac.width + macos.width + GAP
    canvas = Image.new("RGBA", (width + 2 * PADDING, height + LABEL_HEIGHT + PADDING), BACKGROUND)
    canvas.paste(rmac, (PADDING, LABEL_HEIGHT))
    canvas.paste(macos, (PADDING + rmac.width + GAP, LABEL_HEIGHT))

    draw = ImageDraw.Draw(canvas)
    draw.text((PADDING, 7), rmac_label, fill=FOREGROUND)
    draw.text((PADDING + rmac.width + GAP, 7), macos_label, fill=FOREGROUND)

    if output_path.exists() and (output_path.is_symlink() or not output_path.is_file()):
        raise ComposeError(f"output is not a regular file: {output_path.name}")
    temporary = output_path.with_name(f".{output_path.name}.tmp")
    canvas.convert("RGB").save(temporary, format="PNG")
    temporary.replace(output_path)
    return output_path


def main(argv: list[str] | None = None) -> int:
    repo_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rmac")
    parser.add_argument("macos")
    parser.add_argument("output")
    parser.add_argument("--rmac-label", default=None)
    parser.add_argument("--macos-label", default="macOS")
    args = parser.parse_args(argv)

    rmac_label = args.rmac_label or default_rmac_label(repo_root)
    try:
        output = compose(
            Path(args.rmac), Path(args.macos), Path(args.output), rmac_label, args.macos_label
        )
    except ComposeError as error:
        raise SystemExit(f"compose-evidence: {error}") from error
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
