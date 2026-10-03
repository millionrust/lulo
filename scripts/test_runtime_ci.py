"""The CI merger must detect omissions as well as explicit runner failures."""

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_lulo  # noqa: E402
import runtime_ci  # noqa: E402


class RuntimeCITest(unittest.TestCase):
    def test_prepare_restores_shell_entry_points(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for alias in runtime_ci.SHELL_BINARIES.values():
                (root / alias).write_bytes(b"binary")
            runtime_ci.prepare(root)
            for binary, alias in runtime_ci.SHELL_BINARIES.items():
                self.assertEqual((root / binary).resolve(), (root / alias).resolve())

    def test_shards_partition_recorded_scenarios(self):
        self.assertEqual(runtime_ci.JOURNEYS,
                         {path.stem for path in (Path(__file__).resolve().parents[1]
                                                  / "tests/parallel").glob("*.json")})
        paths = run_lulo.runnable_scenarios([])
        shards = [paths[index::runtime_ci.SHARDS] for index in range(runtime_ci.SHARDS)]
        self.assertEqual(len(paths), sum(map(len, shards)))
        self.assertEqual(len(paths), len({path for shard in shards for path in shard}))
        self.assertRaises(ValueError, run_lulo.parse_shard, "8/8")
        self.assertRaises(ValueError, run_lulo.parse_shard, "wrong")

    def test_summary_requires_every_scenario_and_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in runtime_ci.CHECKS | {f"scenario-{index}" for index in range(runtime_ci.SHARDS)}:
                target = root / name
                target.mkdir()
                (target / "status.json").write_text('{"exit_code":0}')
                if name.startswith("scenario-"):
                    index = int(name.removeprefix("scenario-"))
                    paths = run_lulo.runnable_scenarios([])[index::runtime_ci.SHARDS]
                    results = [{"scenario": runtime_ci.scenario.scenario_id(path), "status": "pass"}
                               for path in paths]
                    (target / "behavior-results.json").write_text(json.dumps({"results": results}))
            summary = root / "summary.md"
            self.assertEqual(runtime_ci.summarize(root, summary), 0)
            count = len(run_lulo.runnable_scenarios([]))
            self.assertIn(f"{count}/{count}", summary.read_text())
            (root / "scenario-0" / "behavior-results.json").unlink()
            self.assertEqual(runtime_ci.summarize(root, summary), 1)
            self.assertIn("missing behavior-results.json", summary.read_text())


if __name__ == "__main__":
    unittest.main()
