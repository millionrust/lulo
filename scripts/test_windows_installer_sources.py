#!/usr/bin/env python3
"""The Windows installer's WiX sources must be well-formed XML.

The installer only builds in windows-preview.yml, so a malformed .wxs (for
example `--` inside an XML comment, which WiX rejects with WIX0104) would
otherwise pass every regular CI job and only fail when a preview is built.
"""

from __future__ import annotations

import unittest
import xml.etree.ElementTree as ElementTree
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = ROOT / "packaging" / "windows"


class WixSourcesAreWellFormed(unittest.TestCase):
    def test_every_wxs_parses(self) -> None:
        sources = sorted(WINDOWS.glob("*.wxs"))
        self.assertTrue(sources, "expected at least one .wxs under packaging/windows")
        for source in sources:
            with self.subTest(source=source.name):
                ElementTree.parse(source)


if __name__ == "__main__":
    unittest.main()
