"""Guards the user-facing wording gate (FEEL_SPEC.md §D.10)."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_wording", Path(__file__).with_name("check-wording.py")
)
assert _SPEC is not None and _SPEC.loader is not None
check_wording = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(check_wording)


class WordingGateTests(unittest.TestCase):
    def test_flags_forbidden_user_text(self) -> None:
        for literal in (
            "Failed to open the file.",
            "Error: bad",
            "Warning: disk",
            "saved in /home/jacob",
            "the dbus name is invalid",
        ):
            with self.subTest(literal=literal):
                self.assertIsNotNone(
                    check_wording.scan_literal(Path("crates/x/src/lib.rs"), 1, literal)
                )

    def test_ignores_protocol_constants_and_clean_copy(self) -> None:
        for literal in (
            "/org/freedesktop/DBus",  # D-Bus object path
            "org.freedesktop.DBus.Properties",
            "Error::Io",  # Rust debug shape
            "Move to Trash",
            "The file could not be saved.",
        ):
            with self.subTest(literal=literal):
                self.assertIsNone(
                    check_wording.scan_literal(Path("crates/x/src/lib.rs"), 1, literal)
                )

    def test_internal_marker_exempts_only_its_own_line(self) -> None:
        relative = Path("crates/x/src/lib.rs")
        self.assertEqual(
            check_wording.scan_rust_line(
                relative, 1, '    "dbus-run-session", // wording: internal'
            ),
            [],
        )
        self.assertEqual(
            len(check_wording.scan_rust_line(relative, 1, '    "dbus-run-session",')),
            1,
        )

    def test_desktop_key_matcher_covers_visible_fields(self) -> None:
        for line in ("Name=Files", "Comment=Open a file", "GenericName=Editor"):
            self.assertIsNotNone(check_wording.DESKTOP_KEY.match(line))
        for line in ("Exec=rmac-files", "Type=Application", "Icon=files"):
            self.assertIsNone(check_wording.DESKTOP_KEY.match(line))


if __name__ == "__main__":
    unittest.main()
