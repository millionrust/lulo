#!/usr/bin/env python3
"""Compare Lulo's observed behaviour against the recorded Mac behaviour.

    python3 scripts/behavior/compare.py RESULTS.json
    python3 scripts/behavior/compare.py RESULTS.json --emit-parity-rows

RESULTS.json is what run_lulo.py --output wrote. The comparison is redone
here from the raw Lulo observations and the current scenario files, so a
tolerance change in a scenario needs no re-run. Per-field rules live in
scenario.py (DEFAULT_RULES) and each scenario's "tolerance" object:
exact, set, text (quotes, ellipses and spacing normalized), role-class,
subset, present, count, ignore.

--emit-parity-rows prints proposed docs/parity.md rows for failing
scenarios that no row cites yet (a row cites one as `behavior:<area>/<name>`).
It never edits docs/parity.md.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

import scenario as sc  # noqa: E402

PARITY = sc.REPO / "docs" / "parity.md"


def evaluate(results: dict[str, Any], root: Path = sc.SCENARIO_ROOT) -> list[dict[str, Any]]:
    out = []
    for entry in results.get("results", []):
        sid = entry["scenario"]
        path = root / f"{sid}.json"
        if not path.exists():
            continue
        scenario = sc.load(path)
        expected_path = sc.expectation_path(path)
        if not expected_path.exists():
            continue
        expected = json.loads(expected_path.read_text())
        actual = entry.get("lulo", {})
        if "unsupported" in actual:
            out.append({"scenario": sid, "title": scenario["title"], "status": "unsupported",
                        "reason": actual["unsupported"], "mismatches": [], "spec": scenario})
            continue
        mismatches = sc.compare(scenario, expected, actual)
        out.append({"scenario": sid, "title": scenario["title"], "status": "pass" if not mismatches else "fail",
                    "mismatches": mismatches, "spec": scenario})
    return out


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("results", help="run_lulo.py --output file")
    parser.add_argument("--emit-parity-rows", action="store_true")
    parser.add_argument("--markdown", action="store_true", help="print a pass/fail table in Markdown")
    args = parser.parse_args(argv)
    results = json.loads(Path(args.results).read_text())
    evaluated = evaluate(results)
    if args.markdown:
        print("| Scenario | Result | First difference |")
        print("|---|---|---|")
        for item in evaluated:
            first = ""
            if item["mismatches"]:
                m = item["mismatches"][0]
                first = (f"{m['observation']}.{m['fact']}.{m['field']}: Mac {sc.describe(m['expected'])}, "
                         f"Lulo {sc.describe(m['actual'])}").replace("|", "/")
            elif item["status"] == "unsupported":
                first = item["reason"]
            print(f"| `{item['scenario']}` | {item['status']} | {first} |")
    else:
        for item in evaluated:
            if item["status"] == "unsupported":
                print(f"SKIP  {item['scenario']}  {item['reason']}")
            else:
                print("\n".join(sc.report_lines(item["scenario"], item["spec"], item["mismatches"])))
    passed = sum(item["status"] == "pass" for item in evaluated)
    print(f"\n{passed}/{len(evaluated)} scenarios match the Mac")
    if args.emit_parity_rows:
        failures = [(i["scenario"], i["spec"], i["mismatches"]) for i in evaluated if i["status"] == "fail"]
        rows = sc.parity_rows(PARITY.read_text() if PARITY.exists() else "", failures)
        for section, lines in rows.items():
            print(f"\n{section}\n")
            print("\n".join(lines))
    return 0 if passed == len(evaluated) else 1


if __name__ == "__main__":
    sys.exit(main())
