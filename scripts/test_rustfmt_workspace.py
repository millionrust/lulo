#!/usr/bin/env python3
"""The formatting gate includes every root workspace package, not path dependencies."""

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).parent / "check-rustfmt.py"
SPEC = importlib.util.spec_from_file_location("check_rustfmt", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
check_rustfmt = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_rustfmt)


class RustfmtWorkspaceTests(unittest.TestCase):
    def test_local_path_dependency_is_not_a_workspace_format_target(self):
        metadata = {
            "workspace_members": ["root#text-editor", "root#finder"],
            "packages": [
                {"id": "root#text-editor", "name": "rmac-text-editor"},
                {"id": "root#finder", "name": "rmac-finder"},
                {"id": "path#gpui-component", "name": "gpui-component"},
            ],
        }
        self.assertEqual(
            check_rustfmt.workspace_packages(metadata),
            ["rmac-finder", "rmac-text-editor"],
        )


if __name__ == "__main__":
    unittest.main()
