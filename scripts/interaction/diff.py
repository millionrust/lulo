#!/usr/bin/env python3
"""Diff the Mac and Lulo interaction-probe recordings and write
docs/interaction-gaps.md.

    python3 scripts/interaction/diff.py
    python3 scripts/interaction/diff.py --markdown    # print the table too

Reads tests/interaction/mac/<surface>.json and
tests/interaction/lulo/<surface>.json (written by mac_probe.py and
lulo_probe.py), compares them probe by probe with the rules in probes.py,
and writes one row per (surface, probe) where Lulo's reaction differs from
the Mac's beyond tolerance. A probe or surface with no recording on one
side is reported separately, under "Not yet probed", and is never silently
treated as a pass.

Each gap gets a stable id (INT-NNN) so a fix can cite it and mark the row
`Fixed <sha>`, the same convention as docs/inventory-gaps.md
(scripts/inventory/diff.py) and docs/parity.md.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

import probes as pr  # noqa: E402
import surfaces as sf  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
MAC_DIR = REPO_ROOT / "tests" / "interaction" / "mac"
LULO_DIR = REPO_ROOT / "tests" / "interaction" / "lulo"
OUT_PATH = REPO_ROOT / "docs" / "interaction-gaps.md"


def load(root: Path, surface_id: str) -> Optional[dict[str, Any]]:
    path = root / f"{surface_id}.json"
    if not path.exists():
        return None
    return json.loads(path.read_text())


def probe_ids_for(item: dict[str, Any]) -> list[str]:
    ids = list(pr.probes_for(item["kind"]))
    for control in item.get("hover_controls", []):
        ids.append(f"hover:{control['label']}")
    return ids


def compare_surface(item: dict[str, Any], mac_doc: Optional[dict[str, Any]], lulo_doc: Optional[dict[str, Any]]
                     ) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """(gaps, unprobed) for one surface: gaps are mismatches beyond
    tolerance; unprobed are probes with no usable recording on one or both
    sides (missing file, the surface failed to open, or a fact wasn't
    measured)."""

    gaps: list[dict[str, Any]] = []
    unprobed: list[dict[str, Any]] = []
    mac_error = (mac_doc or {}).get("error")
    lulo_error = (lulo_doc or {}).get("error")
    for probe_id in probe_ids_for(item):
        base_probe = probe_id.split(":", 1)[0]
        spec = pr.PROBES.get(base_probe)
        if spec is None or spec["status"] != "automated":
            unprobed.append({"surface": item["id"], "probe": probe_id, "reason": "planned, no driver yet"})
            continue
        if mac_doc is None or lulo_doc is None:
            unprobed.append({"surface": item["id"], "probe": probe_id,
                              "reason": "no recording" if mac_doc is None else "no Lulo recording"})
            continue
        if mac_error or lulo_error:
            unprobed.append({"surface": item["id"], "probe": probe_id,
                              "reason": f"surface failed to open ({mac_error or lulo_error})"})
            continue
        mac_result = (mac_doc.get("probes") or {}).get(probe_id)
        lulo_result = (lulo_doc.get("probes") or {}).get(probe_id)
        comparison = pr.compare_probe_result(base_probe, mac_result, lulo_result)
        if comparison["inconclusive"]:
            unprobed.append({"surface": item["id"], "probe": probe_id,
                              "reason": f"not measured: {', '.join(comparison['inconclusive'])}"})
        for mismatch in comparison["mismatches"]:
            gaps.append({"surface": item["id"], "title": item["title"], "probe": probe_id, **mismatch})
    return gaps, unprobed


def evaluate() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    all_gaps: list[dict[str, Any]] = []
    all_unprobed: list[dict[str, Any]] = []
    for item in sf.SURFACES:
        if item["status"] != "automated":
            for probe_id in probe_ids_for(item):
                all_unprobed.append({"surface": item["id"], "probe": probe_id,
                                      "reason": item.get("note", "surface has no driver yet")})
            continue
        mac_doc = load(MAC_DIR, item["id"])
        lulo_doc = load(LULO_DIR, item["id"])
        gaps, unprobed = compare_surface(item, mac_doc, lulo_doc)
        all_gaps.extend(gaps)
        all_unprobed.extend(unprobed)
    return all_gaps, all_unprobed


def assign_ids(gaps: list[dict[str, Any]], existing_text: str) -> list[str]:
    """A stable id per (surface, probe, fact): a gap already cited in the
    current doc keeps its id across reruns (so a rerun that finds nothing
    new never inflates INT-NNN forever), and only a genuinely new gap gets
    the next free number."""

    import re

    row = re.compile(r"\|\s*(INT-\d+)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|")
    existing: dict[tuple[str, str, str], str] = {}
    numbers = []
    for match in row.finditer(existing_text):
        gap_id, surface, probe, fact = match.groups()
        existing[(surface, probe, fact)] = gap_id
        numbers.append(int(gap_id.split("-")[1]))
    next_number = (max(numbers) + 1) if numbers else 1
    ids = []
    for gap in gaps:
        key = (gap["surface"], gap["probe"], gap["fact"])
        if key in existing:
            ids.append(existing[key])
            continue
        ids.append(f"INT-{str(next_number).zfill(3)}")
        next_number += 1
    return ids


def render_markdown(gaps: list[dict[str, Any]], unprobed: list[dict[str, Any]]) -> str:
    lines = [
        "# Interaction-probe gaps",
        "",
        "Generated by `scripts/interaction/diff.py` from "
        "`tests/interaction/mac/*.json` and `tests/interaction/lulo/*.json`. "
        "Each row is one surface×probe where Lulo's reaction to input differs "
        "from the real Mac's beyond tolerance - a *behaviour* gap (closing, "
        "hover, focus, switching), not a missing menu item "
        "(see docs/inventory-gaps.md for those). Mark a fixed row `Fixed <sha>`.",
        "",
    ]
    existing_text = OUT_PATH.read_text() if OUT_PATH.exists() else ""
    ids = assign_ids(gaps, existing_text)
    if gaps:
        lines += ["| id | surface | probe | fact | Mac | Lulo |", "|---|---|---|---|---|---|"]
        for gap_id, gap in zip(ids, gaps):
            lines.append(
                f"| {gap_id} | {gap['surface']} | {gap['probe']} | {gap['fact']} "
                f"| {pr.describe(gap['mac'])} | {pr.describe(gap['lulo'])} |"
            )
    else:
        lines.append("No gaps found among the probes that have run on both platforms.")
    lines += ["", "## Not yet probed", "",
              "Either the surface/probe has no driver yet (`status: \"planned\"` in "
              "surfaces.py/probes.py), or a side's recording is missing, or a fact "
              "could not be measured. None of these count as a pass.", ""]
    if unprobed:
        lines += ["| surface | probe | why |", "|---|---|---|"]
        for item in unprobed:
            lines.append(f"| {item['surface']} | {item['probe']} | {item['reason']} |")
    else:
        lines.append("Every declared surface×probe has a recording on both platforms.")
    lines.append("")
    return "\n".join(lines)


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--markdown", action="store_true", help="also print the table to stdout")
    parser.add_argument("--dry-run", action="store_true", help="print the report instead of writing it")
    args = parser.parse_args(argv)
    gaps, unprobed = evaluate()
    report = render_markdown(gaps, unprobed)
    if args.dry_run:
        print(report)
    else:
        OUT_PATH.write_text(report)
        print(f"wrote {OUT_PATH} ({len(gaps)} gaps, {len(unprobed)} not yet probed)")
    if args.markdown:
        print(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
