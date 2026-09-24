"""Keep rmac's windows visible to AT-SPI clients as soon as AT-SPI is enabled.

accesskit_unix before 0.22 only registered a window with the AT-SPI registry
while `org.a11y.Status.ScreenReaderEnabled` was true. On a Lulo session the
bus reports `IsEnabled` (toolkit-accessibility) without a screen reader
running, so no rmac surface -- top bar, Dock, Launcher or any app -- appeared
under the registry root, and Orca, Accerciser and pyatspi could not see Lulo.
accesskit_unix 0.22 watches `IsEnabled`, the property GTK and Qt honour.

Both workspaces build the vendored gpui_linux, so both lockfiles must resolve
the fixed release.
"""

from __future__ import annotations

from pathlib import Path
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
LOCKFILES = [ROOT / "Cargo.lock", ROOT / "shell/Cargo.lock"]
GPUI_LINUX_MANIFEST = ROOT / "shell/compat/gpui_linux/Cargo.toml"
FIRST_IS_ENABLED_RELEASE = (0, 22)


def version_tuple(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in version.split("-", 1)[0].split("."))


def locked_versions(lockfile: Path, name: str) -> list[str]:
    document = tomllib.loads(lockfile.read_text(encoding="utf-8"))
    return [
        package["version"]
        for package in document.get("package", [])
        if package.get("name") == name
    ]


class AccessKitActivationTests(unittest.TestCase):
    def test_lockfiles_resolve_accesskit_unix_that_watches_is_enabled(self):
        for lockfile in LOCKFILES:
            with self.subTest(lockfile=lockfile.relative_to(ROOT)):
                versions = locked_versions(lockfile, "accesskit_unix")
                self.assertEqual(len(versions), 1, versions)
                self.assertGreaterEqual(
                    version_tuple(versions[0])[:2], FIRST_IS_ENABLED_RELEASE
                )

    def test_gpui_linux_requires_accesskit_unix_that_watches_is_enabled(self):
        manifest = tomllib.loads(GPUI_LINUX_MANIFEST.read_text(encoding="utf-8"))
        dependencies = manifest["target"][
            'cfg(any(target_os = "linux", target_os = "freebsd"))'
        ]["dependencies"]
        requirement = dependencies["accesskit_unix"]
        self.assertIsInstance(requirement, str)
        self.assertGreaterEqual(
            version_tuple(requirement.lstrip("^="))[:2], FIRST_IS_ENABLED_RELEASE
        )


if __name__ == "__main__":
    unittest.main()
