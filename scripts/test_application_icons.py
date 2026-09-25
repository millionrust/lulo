"""Validate the complete original rmac icon inventory and its generator."""

from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import re
import stat
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).parents[1]
ICON_DIR = ROOT / "packaging/rmac-apps/icons"
DOCK_DIR = ROOT / "crates/rmac-dock/assets/icons"
IDENTITIES = (
    "org.rmac.Files",
    "org.rmac.Terminal",
    "org.rmac.Notes",
    "org.rmac.TextEditor",
    "org.rmac.SystemMonitor",
    "org.rmac.AppDrawer",
    "org.rmac.SystemSettings",
    "org.rmac.Calculator",
    "org.rmac.Preview",
    "org.rmac.ArchiveUtility",
    "org.rmac.Clock",
    "org.rmac.Weather",
    "org.rmac.Player",
)
DOCK_ICONS = (
    "application",
    "files",
    "downloads",
    "folder",
    "trash-empty",
    "trash-full",
    "more",
    "stack-item-document",
)
SVG_NAMESPACE = "http://www.w3.org/2000/svg"
ALLOWED_ELEMENTS = {
    "svg", "defs", "g", "rect", "path", "circle", "linearGradient",
    "radialGradient", "stop", "clipPath", "filter", "feDropShadow", "feColorMatrix",
}
LOCAL_REFERENCE = re.compile(r"^url\(#([A-Za-z][\w-]*)\)$")
MAX_ICON_BYTES = 16 * 1024


def load_generator():
    spec = importlib.util.spec_from_file_location("build_icons", ROOT / "scripts/build-icons.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def icon_files():
    return [ICON_DIR / f"{identity}.svg" for identity in IDENTITIES] + [
        DOCK_DIR / f"{name}.svg" for name in DOCK_ICONS
    ]


class ApplicationIconTests(unittest.TestCase):
    def test_inventory_is_exact_regular_bounded_and_unique(self):
        for directory, names in ((ICON_DIR, IDENTITIES), (DOCK_DIR, DOCK_ICONS)):
            expected = {f"{name}.svg" for name in names}
            actual = {path.name for path in directory.iterdir()}
            self.assertEqual(actual, expected)
        hashes = set()
        for path in icon_files():
            with self.subTest(path=path.name):
                metadata = path.lstat()
                self.assertTrue(stat.S_ISREG(metadata.st_mode))
                self.assertFalse(path.is_symlink())
                self.assertLessEqual(metadata.st_size, MAX_ICON_BYTES)
                contents = path.read_bytes()
                self.assertEqual(len(contents), metadata.st_size)
                self.assertIn(b"Original rmac artwork, MIT licensed.", contents)
                hashes.add(hashlib.sha256(contents).digest())
        # The Dock's Files artwork is the Files application icon itself.
        self.assertEqual(len(hashes), len(icon_files()) - 1)
        self.assertEqual(
            (ICON_DIR / "org.rmac.Files.svg").read_bytes(), (DOCK_DIR / "files.svg").read_bytes()
        )

    def test_icons_are_self_contained_safe_scalable_svg(self):
        for path in icon_files():
            with self.subTest(path=path.name):
                root = ET.fromstring(path.read_bytes())
                self.assertEqual(root.tag, f"{{{SVG_NAMESPACE}}}svg")
                self.assertEqual(
                    root.attrib, {"viewBox": "0 0 1024 1024", "width": "128", "height": "128"}
                )
                ids = {element.get("id") for element in root.iter() if element.get("id")}
                for element in root.iter():
                    local = element.tag.removeprefix(f"{{{SVG_NAMESPACE}}}")
                    self.assertIn(local, ALLOWED_ELEMENTS)
                    self.assertFalse(element.text and element.text.strip())
                    for name, value in element.attrib.items():
                        lowered = name.lower()
                        self.assertFalse(lowered.startswith("on"))
                        self.assertNotIn(lowered, {"href", "style"})
                        self.assertFalse(lowered.endswith("}href"))
                        self.assertNotIn("javascript:", value.lower())
                        if "url(" in value:
                            match = LOCAL_REFERENCE.match(value)
                            self.assertIsNotNone(match, value)
                            self.assertIn(match.group(1), ids)

    def test_plates_sit_on_the_measured_squircle_grid(self):
        generator = load_generator()
        self.assertEqual((generator.SIZE, generator.PLATE), (1024, 824))
        plate = generator.squircle_path()
        numbers = [float(n) for n in re.findall(r"-?\d+(?:\.\d+)?", plate)]
        xs, ys = numbers[0::2], numbers[1::2]
        self.assertAlmostEqual(min(xs), 100, delta=0.5)
        self.assertAlmostEqual(max(xs), 924, delta=0.5)
        self.assertAlmostEqual(min(ys), 100, delta=0.5)
        self.assertAlmostEqual(max(ys), 924, delta=0.5)
        for identity in IDENTITIES:
            with self.subTest(identity=identity):
                self.assertIn(plate, (ICON_DIR / f"{identity}.svg").read_text())

    def test_every_generated_icon_is_current(self):
        generator = load_generator()
        for path, text in generator.outputs().items():
            with self.subTest(path=str(path.relative_to(ROOT))):
                self.assertTrue(path.is_file())
                self.assertEqual(path.read_text(), text)


if __name__ == "__main__":
    unittest.main()
