"""Small contract tests for the parallel journey format and comparison."""

import json
from pathlib import Path

from PIL import Image
import pytest

import compare
import journey


def test_all_first_hour_journeys_have_a_shot_after_each_action():
    paths = journey.paths([])
    assert len(paths) == 10
    for path in paths:
        assert journey.load(path)["steps"]


def test_rejects_input_without_its_shot(tmp_path: Path):
    path = tmp_path / "broken.json"
    path.write_text(json.dumps({"title": "broken", "steps": [
        {"launch": "files"}, {"key": "cmd-n"}, {"shot": "late"}]}))
    with pytest.raises(ValueError, match="before shot"):
        journey.load(path)


def test_rejects_sandbox_escape(tmp_path: Path):
    path = tmp_path / "broken.json"
    path.write_text(json.dumps({"title": "broken", "setup": {"files": {"../owner.txt": "bad"}},
                                "steps": [{"launch": "files"}, {"shot": "open"}]}))
    with pytest.raises(ValueError, match="escapes sandbox"):
        journey.load(path)


def test_paired_image_keeps_both_sides_at_one_logical_height(tmp_path: Path):
    mac, lulo, out = (tmp_path / name for name in ("mac.png", "lulo.png", "pair.png"))
    Image.new("RGB", (400, 300), "red").save(mac)
    Image.new("RGB", (100, 75), "blue").save(lulo)
    compare.paired(mac, lulo, [200, 150], out)
    with Image.open(out) as image:
        assert image.size == (412, 212)
        assert image.getpixel((100, 50)) == (255, 0, 0)
        assert image.getpixel((312, 50)) == (0, 0, 255)
