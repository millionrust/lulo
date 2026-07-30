"""Focused fixtures for the I7 user-documentation set."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).parent / "verify-documentation.py"
SPEC = importlib.util.spec_from_file_location("verify_documentation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
verify = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verify
SPEC.loader.exec_module(verify)


class DocumentationTests(unittest.TestCase):
    def test_committed_set_covers_all_fifteen_goal_topics_and_links(self):
        manifest = verify.load_manifest()
        self.assertEqual(len(manifest["topics"]), 15)
        self.assertEqual(len(manifest["documents"]), 12)
        verify.verify_documentation(manifest)

    def test_rejects_manifest_without_safe_mode_topic(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "docs.json"
            document = json.loads(verify.MANIFEST_PATH.read_text(encoding="utf-8"))
            del document["topics"]["safe-mode"]
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(verify.DocumentationError, "differs"):
                verify.load_manifest(path)

    def test_rejects_local_link_outside_repository(self):
        with self.assertRaisesRegex(verify.DocumentationError, "escapes"):
            verify._validate_local_link(
                verify.REPO_ROOT / "docs/user-guide.md",
                "../../../outside-rmac",
            )


if __name__ == "__main__":
    unittest.main()
