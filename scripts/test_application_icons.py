"""Validate the complete original rmac application-icon inventory."""

from __future__ import annotations

import hashlib
from pathlib import Path
import stat
import unittest
import xml.etree.ElementTree as ET


ICON_DIR = Path(__file__).parents[1] / "packaging/rmac-apps/icons"
IDENTITIES = (
    "org.rmac.Finder",
    "org.rmac.Terminal",
    "org.rmac.Notes",
    "org.rmac.TextEditor",
    "org.rmac.ActivityMonitor",
    "org.rmac.AppDrawer",
    "org.rmac.SystemSettings",
)
SVG_NAMESPACE = "http://www.w3.org/2000/svg"
ALLOWED_ELEMENTS = {"svg", "g", "rect", "path", "circle"}
MAX_ICON_BYTES = 16 * 1024


class ApplicationIconTests(unittest.TestCase):
    def test_inventory_is_exact_regular_bounded_and_unique(self):
        expected = {f"{identity}.svg" for identity in IDENTITIES}
        actual = {path.name for path in ICON_DIR.iterdir()}
        self.assertEqual(actual, expected)
        hashes = set()
        for filename in sorted(expected):
            with self.subTest(filename=filename):
                path = ICON_DIR / filename
                metadata = path.lstat()
                self.assertTrue(stat.S_ISREG(metadata.st_mode))
                self.assertFalse(path.is_symlink())
                self.assertLessEqual(metadata.st_size, MAX_ICON_BYTES)
                contents = path.read_bytes()
                self.assertEqual(len(contents), metadata.st_size)
                self.assertIn(b"Original rmac artwork, MIT licensed.", contents)
                hashes.add(hashlib.sha256(contents).digest())
        self.assertEqual(len(hashes), len(expected))

    def test_icons_are_self_contained_safe_scalable_svg(self):
        for identity in IDENTITIES:
            with self.subTest(identity=identity):
                root = ET.fromstring((ICON_DIR / f"{identity}.svg").read_bytes())
                self.assertEqual(root.tag, f"{{{SVG_NAMESPACE}}}svg")
                self.assertEqual(root.attrib, {"viewBox": "0 0 128 128"})
                for element in root.iter():
                    local = element.tag.removeprefix(f"{{{SVG_NAMESPACE}}}")
                    self.assertIn(local, ALLOWED_ELEMENTS)
                    self.assertFalse(element.text and element.text.strip())
                    for name, value in element.attrib.items():
                        lowered = name.lower()
                        self.assertFalse(lowered.startswith("on"))
                        self.assertNotIn(lowered, {"href", "style", "filter"})
                        self.assertNotIn("url(", value.lower())
                        self.assertNotIn("javascript:", value.lower())


if __name__ == "__main__":
    unittest.main()
