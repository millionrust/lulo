#!/usr/bin/env python3
"""Compare the Mac robot's states with Lulo's and rank the differences.

    python3 scripts/robot/mac/diff.py [--lulo DIR] [--top 30] [--no-images]

Reads .claude/mac-captures/<app>/states.jsonl (mac_walk.py) and
<lulo>/<app>/states.jsonl (lulo_walk.py; default .claude/mac-captures/lulo),
drops what tests/robot/mac/allowlist.json and tests/inventory/allowlist.json
allow, and writes .claude/mac-captures/diff/report.json and report.md plus a
side-by-side JPEG per state with both screenshots (all local, never
committed). Prints the top differences and the totals by category.

Categories: missing-state, extra-state, missing-item, extra-item,
label, disabled, shortcut, missing-control, extra-control, window-size,
geometry, visual, crash.
"""

from __future__ import annotations

import argparse
import difflib
import fnmatch
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import common as cm  # noqa: E402

MACAX = cm.CAPTURES / "bin" / "macax"
WEIGHT = {"window": 10, "item": 7, "settings": 8, "pane": 8, "tab": 7, "toolbar": 5, "context": 6, "menu": 6,
          "shell": 6}
APP_WEIGHT = {"Finder": 1.3, "System Settings": 1.3, "TextEdit": 1.1, "Notes": 1.1, "Mail": 1.0, "Terminal": 1.0,
              "Preview": 1.0, "Calendar": 1.0, "Shell": 1.3}
GEOMETRY_TOLERANCE = 40  # points, control position relative to the window
SIZE_TOLERANCE = 0.15
DIFF_ROLES = {"button", "menubutton", "popup", "checkbox", "radio", "textfield", "slider", "stepper", "link",
              "disclosure", "colorwell"}


def load_allowlist() -> list[dict[str, Any]]:
    rows = []
    path = cm.SUMMARIES / "allowlist.json"
    if path.exists():
        rows += json.loads(path.read_text())["entries"]
    inventory = cm.REPO / "tests/inventory/allowlist.json"
    if inventory.exists():
        reverse = {v["lulo"]: k for k, v in cm.LULO_APPS.items()}
        for entry in json.loads(inventory.read_text())["entries"]:
            app = entry["app"] if entry["app"] == "*" else reverse.get(entry["app"], entry["app"])
            rows.append({"app": app, "path": "*", "label": entry["label"], "match": entry.get("match", "exact"),
                         "reason": entry["reason"]})
    return rows


class Allow:
    def __init__(self) -> None:
        self.rows = load_allowlist()
        self.used: Counter = Counter()

    def __call__(self, app: str, path: str, label: Optional[str] = None) -> Optional[str]:
        for row in self.rows:
            if row["app"] not in ("*", app):
                continue
            if not fnmatch.fnmatchcase(path, row.get("path", "*")):
                continue
            want = row.get("label")
            if want is None:
                if label is None:
                    self.used[(row["app"], row.get("path"), None)] += 1
                    return row["reason"]
                continue
            if label is None:
                continue
            if (row.get("match") == "prefix" and label.startswith(want)) or label == want:
                self.used[(row["app"], row.get("path"), want)] += 1
                return row["reason"]
        return None


def impact(app: str, path: str, base: float) -> float:
    head = path.split("/")[0]
    return round(base * WEIGHT.get(head, 5) / 5 * APP_WEIGHT.get(app, 1.0), 2)


CHROME_LABELS = {"close window", "close", "minimize", "minimise", "enter full screen", "zoom", "exit full screen"}
WINDOW_LIST_END = {"Bring All to Front", "Arrange in Front"}


def strip_window_list(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The Window menu ends with the app's open windows: runtime content."""

    last = max((i for i, row in enumerate(items) if row.get("label") in WINDOW_LIST_END), default=None)
    return items if last is None else items[: last + 1]


def labels_of(rows: list[dict[str, Any]], app: str, roles: Optional[set[str]] = None) -> dict[str, dict[str, Any]]:
    out = {}
    for row in rows:
        if row.get("label") in (None, "", "-") or row.get("dynamic"):
            continue
        if cm.norm_label(row["label"], app) in CHROME_LABELS:
            continue
        if roles is not None and row.get("role") not in roles:
            continue
        out.setdefault(cm.norm_label(row["label"], app), row)
    return out


def norm_key(key: Optional[str]) -> str:
    key = (key or "").replace(" ", "").replace("Ctrl+", "⌃").replace("Super+", "⌘").replace("🌐", "").upper()
    return "".join(sorted(c for c in key if c in "⌃⌥⇧⌘")) + "".join(c for c in key if c not in "⌃⌥⇧⌘")


def common_labels(states: dict[tuple[str, str], dict[str, Any]], app: str) -> dict[str, set[str]]:
    """Per state family (pane, settings, tab, item…), the labels present in
    most of its states: the window's own chrome (a settings sidebar, the
    account card, a toolbar), compared once in window/main, not per pane."""

    families: dict[str, list[set[str]]] = {}
    for (_a, path), state in states.items():
        if state.get("controls") is None:
            continue
        families.setdefault(path.split("/")[0], []).append(
            {cm.norm_label(c.get("label"), app) for c in state["controls"] if c.get("label")})
    out = {}
    for family, sets in families.items():
        if len(sets) < 3:
            out[family] = set()
            continue
        counts = Counter(label for labels in sets for label in labels)
        out[family] = {label for label, n in counts.items() if n >= 0.6 * len(sets)}
    return out


def compare(mac: dict[str, Any], lulo: dict[str, Any], app: str, allow: Allow, out: list[dict[str, Any]],
            chrome: Optional[set[str]] = None) -> None:
    path = mac["path"]
    chrome = chrome or set()

    def add(category: str, detail: str, base: float, labels: Optional[list[str]] = None) -> None:
        out.append({"app": app, "path": path, "category": category, "detail": detail,
                    "impact": impact(app, path, base), "labels": labels or []})

    if lulo.get("kind") == "crash":
        add("crash", lulo.get("note", "app exited"), 10)
        return
    if mac.get("items") is not None and lulo.get("items") is not None:
        mac_items, lulo_items = mac["items"], lulo["items"]
        if path == "menu/Window":
            mac_items, lulo_items = strip_window_list(mac_items), strip_window_list(lulo_items)
        m = labels_of(mac_items, app)
        l_ = labels_of(lulo_items, app)
        missing = [k for k in m if k not in l_ and not allow(app, path, m[k]["label"])]
        extra = [k for k in l_ if k not in m]
        relabel = []
        for key in list(missing):
            close = difflib.get_close_matches(key, extra, n=1, cutoff=0.82)
            if close:
                relabel.append(f"{m[key]['label']!r} → {l_[close[0]]['label']!r}")
                missing.remove(key)
                extra.remove(close[0])
        if missing:
            add("missing-item", ", ".join(m[k]["label"] for k in missing), 1.5 + 0.5 * min(len(missing), 10),
                [m[k]["label"] for k in missing])
        if extra:
            add("extra-item", ", ".join(l_[k]["label"] for k in extra), 0.5 + 0.2 * min(len(extra), 10),
                [l_[k]["label"] for k in extra])
        if relabel:
            add("label", "; ".join(relabel), 1.0 + 0.3 * len(relabel))
        disabled = [m[k]["label"] for k in m if k in l_ and m[k].get("enabled") and not l_[k].get("enabled")]
        if disabled:
            add("disabled", ", ".join(disabled), 1.0 + 0.4 * min(len(disabled), 10), disabled)
        keys = [f"{m[k]['label']}: {m[k].get('key')} vs {l_[k].get('key') or '—'}" for k in m
                if k in l_ and m[k].get("key") and norm_key(m[k]["key"]) != norm_key(l_[k].get("key"))]
        if keys:
            add("shortcut", "; ".join(keys), 0.8 + 0.3 * min(len(keys), 10))
    if mac.get("controls") is not None and lulo.get("controls") is not None:
        # Static text is compared only where it is the UI's own wording
        # (settings and panes), not content (forecasts, versions, files).
        roles = DIFF_ROLES | ({"text"} if path.startswith(("settings", "pane")) else set())
        m = {k: v for k, v in labels_of(mac["controls"], app, roles).items() if k not in chrome}
        l_ = labels_of(lulo["controls"], app)
        missing = [k for k in m if k not in l_ and not allow(app, path, m[k]["label"])]
        extra = [k for k in labels_of(lulo["controls"], app, roles) if k not in labels_of(mac["controls"], app)
                 and k not in chrome]
        relabel = []
        for key in list(missing):
            close = difflib.get_close_matches(key, extra, n=1, cutoff=0.85)
            if close:
                relabel.append(f"{m[key]['label']!r} → {l_[close[0]]['label']!r}")
                missing.remove(key)
                extra.remove(close[0])
        if missing:
            add("missing-control", ", ".join(m[k]["label"] for k in missing[:25]), 1.0 + 0.4 * min(len(missing), 12),
                [m[k]["label"] for k in missing])
        if extra:
            add("extra-control", ", ".join(l_[k]["label"] for k in extra[:25]), 0.3 + 0.1 * min(len(extra), 10))
        if relabel:
            add("label", "; ".join(relabel), 0.8 + 0.3 * len(relabel))
        disabled = [m[k]["label"] for k in m if k in l_ and m[k].get("enabled") and not l_[k].get("enabled")
                    and m[k]["role"] in DIFF_ROLES and l_[k]["role"] == m[k]["role"]]
        if disabled:
            add("disabled", ", ".join(disabled[:20]), 1.0 + 0.3 * min(len(disabled), 10), disabled)
        moved = []
        for key, row in m.items():
            other = l_.get(key)
            if (not other or not row.get("f") or not other.get("f") or row["role"] not in DIFF_ROLES
                    or other["role"] != row["role"]):
                continue
            dx, dy = other["f"][0] - row["f"][0], other["f"][1] - row["f"][1]
            if max(abs(dx), abs(dy)) > GEOMETRY_TOLERANCE:
                moved.append(f"{row['label']} ({dx:+d},{dy:+d})")
        if moved:
            add("geometry", "; ".join(moved[:12]), 0.5 + 0.15 * min(len(moved), 10))
    mw, lw = mac.get("window"), lulo.get("window")
    if mw and lw and mac.get("kind") not in {"menu", "context"} and min(mw) > 0:
        rw, rh = lw[0] / mw[0], lw[1] / mw[1]
        if abs(rw - 1) > SIZE_TOLERANCE or abs(rh - 1) > SIZE_TOLERANCE:
            add("window-size", f"Mac {mw[0]}×{mw[1]}, Lulo {lw[0]}×{lw[1]}", 1.0 + min(2.0, abs(rw - 1) + abs(rh - 1)))


def visual(mac: dict[str, Any], lulo: dict[str, Any], lulo_root: Path, app: str, out_dir: Path) -> Optional[float]:
    if not mac.get("shot") or not lulo.get("shot"):
        return None
    a, b = cm.CAPTURES / mac["shot"], lulo_root / lulo["shot"]
    if not a.exists() or not b.exists():
        return None
    result = subprocess.run([str(MACAX), "score", str(a), str(b)], capture_output=True, text=True)
    try:
        score = json.loads(result.stdout)["score"]
    except (json.JSONDecodeError, KeyError):
        return None
    side = out_dir / app / (Path(mac["shot"]).stem + ".jpg")
    side.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(MACAX), "sbs", str(a), str(b), str(side), f"Mac · {mac['path']}", "Lulo"],
                   capture_output=True)
    return score


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--lulo", default=str(cm.CAPTURES / "lulo"))
    parser.add_argument("--top", type=int, default=30)
    parser.add_argument("--no-images", action="store_true")
    args = parser.parse_args()
    lulo_root = Path(args.lulo)
    out_dir = cm.CAPTURES / "diff"
    out_dir.mkdir(parents=True, exist_ok=True)
    allow = Allow()
    diffs: list[dict[str, Any]] = []
    allowed: list[dict[str, Any]] = []
    counts = {"mac_states": 0, "lulo_states": 0, "matched": 0, "visual": []}
    apps = sorted(p.name for p in cm.CAPTURES.iterdir() if (p / "states.jsonl").exists())
    for app in apps:
        mac_states = cm.load_states(cm.CAPTURES / app / "states.jsonl")
        lulo_file = lulo_root / app / "states.jsonl"
        lulo_states = cm.load_states(lulo_file)
        counts["mac_states"] += len(mac_states)
        counts["lulo_states"] += len(lulo_states)
        walked = lulo_file.exists()
        mac_chrome = common_labels(mac_states, app)
        lulo_chrome = common_labels(lulo_states, app)
        for (a, path), mac in mac_states.items():
            if "#" in path:
                continue
            reason = allow(app, path)
            if reason:
                allowed.append({"app": app, "path": path, "reason": reason})
                continue
            lulo = lulo_states.get((a, path))
            if lulo is None:
                if not walked:
                    diffs.append({"app": app, "path": path, "category": "not-walked",
                                  "detail": "no Lulo walk for this app/surface", "impact": 0, "labels": []})
                else:
                    kind = mac.get("kind")
                    what = {"other-app": "opens another app on the Mac"}.get(kind, f"Mac shows a {kind}")
                    diffs.append({"app": app, "path": path, "category": "missing-state",
                                  "detail": f"{what}; Lulo has no such state", "impact": impact(app, path, 3.0),
                                  "labels": []})
                continue
            counts["matched"] += 1
            if lulo.get("kind") == "in-window" and mac.get("kind") not in {"in-window", "other-app"}:
                diffs.append({"app": app, "path": path, "category": "missing-state",
                              "detail": f"Mac opens a {mac.get('kind')}; on Lulo the item opened nothing",
                              "impact": impact(app, path, 3.0), "labels": []})
            family = path.split("/")[0]
            compare(mac, lulo, app, allow, diffs,
                    mac_chrome.get(family, set()) | lulo_chrome.get(family, set()) if family != "window" else set())
            if not args.no_images:
                score = visual(mac, lulo, lulo_root, app, out_dir)
                if score is not None:
                    counts["visual"].append(score)
                    if score < 0.55:
                        diffs.append({"app": app, "path": path, "category": "visual",
                                      "detail": f"perceptual score {score:.2f}", "impact": impact(app, path, (0.55 - score) * 4),
                                      "labels": [], "score": score})
        for (a, path), lulo in lulo_states.items():
            if (a, path) not in mac_states and path.startswith("menu/") and "#" not in path:
                diffs.append({"app": app, "path": path, "category": "extra-state",
                              "detail": "Lulo menu the Mac does not have", "impact": impact(app, path, 0.8),
                              "labels": []})
    diffs.sort(key=lambda d: -d["impact"])
    totals = Counter(d["category"] for d in diffs)
    visual_scores = counts.pop("visual")
    report = {"counts": counts, "totals": dict(totals), "allowed_states": allowed,
              "allowlist_hits": [{"app": k[0], "path": k[1], "label": k[2], "hits": v} for k, v in allow.used.items()],
              "mean_visual": round(sum(visual_scores) / len(visual_scores), 3) if visual_scores else None,
              "differences": diffs}
    (out_dir / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=1))
    lines = ["# Mac vs Lulo robot report", "", f"Mac states {counts['mac_states']}, Lulo states {counts['lulo_states']}, "
             f"matched {counts['matched']}, mean visual score {report['mean_visual']}", "", "| # | app | state | category | what differs |",
             "|---|---|---|---|---|"]
    for index, d in enumerate([d for d in diffs if d["category"] != "not-walked"], 1):
        lines.append(f"| {index} | {d['app']} | {d['path']} | {d['category']} | {d['detail'][:300]} |")
    (out_dir / "report.md").write_text("\n".join(lines) + "\n")
    print(lines[2])
    for d in [d for d in diffs if d["category"] != "not-walked"][: args.top]:
        print(f"{d['impact']:5.1f}  {d['app']:<17} {d['path']:<45} {d['category']:<15} {d['detail'][:140]}")
    print("totals:", dict(totals))
    print(f"allowlisted states: {len(allowed)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
