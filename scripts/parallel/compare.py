#!/usr/bin/env python3
"""Build review images, HTML index and JSON summary outside the repository."""

from __future__ import annotations

import argparse
from html import escape
import json
from pathlib import Path

from PIL import Image, ImageDraw

import journey


def result(root: Path, name: str) -> dict:
    path = root / name / "result.json"
    return json.loads(path.read_text()) if path.exists() else {"status": "not_run", "steps": []}


def scaled(path: Path, points=None) -> Image.Image:
    with Image.open(path) as image:
        rgb = image.convert("RGB")
    if points:
        width, height = map(int, points)
        if width > 0 and height > 0 and (rgb.width, rgb.height) != (width, height):
            rgb = rgb.resize((width, height), Image.Resampling.LANCZOS)
    return rgb


def paired(mac: Path | None, lulo: Path | None, points, destination: Path) -> None:
    left = scaled(mac, points) if mac and mac.exists() else None
    right = scaled(lulo) if lulo and lulo.exists() else None
    if left and right and right.height != left.height:
        width = max(1, round(right.width * left.height / right.height))
        right = right.resize((width, left.height), Image.Resampling.LANCZOS)
    height = max(left.height if left else 0, right.height if right else 0, 180)
    lw = left.width if left else 460
    rw = right.width if right else 460
    canvas = Image.new("RGB", (lw + rw + 12, height + 32), "#e5e5e5")
    draw = ImageDraw.Draw(canvas)
    draw.text((8, 8), "Mac", fill="black")
    draw.text((lw + 20, 8), "Lulo", fill="black")
    if left:
        canvas.paste(left, (0, 32))
    if right:
        canvas.paste(right, (lw + 12, 32))
    destination.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(destination)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mac", type=Path, required=True)
    parser.add_argument("--lulo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if journey.ROOT in args.output.resolve().parents:
        parser.error("comparison output must be outside the repository")
    args.output.mkdir(parents=True, exist_ok=True)
    summary = {"journeys": [], "counts": {"journeys": 0, "paired_shots": 0, "mac_shots": 0, "lulo_shots": 0}}
    parts = ["<!doctype html><meta charset='utf-8'><title>Parallel journeys</title>",
             "<style>body{font:15px system-ui;margin:2rem;background:#f5f5f5}section{margin:2rem 0;background:white;padding:1rem}"
             "img{max-width:100%;height:auto}table{border-collapse:collapse}td,th{padding:.35rem .8rem;border:1px solid #ccc}</style>",
             "<h1>Mac and Lulo parallel journeys</h1>"]
    for path in journey.paths([]):
        name = path.stem
        definition = journey.load(path)
        mac, lulo = result(args.mac, name), result(args.lulo, name)
        msteps = {row["name"]: row for row in mac["steps"]}
        lsteps = {row["name"]: row for row in lulo["steps"]}
        rows = []
        parts.append(f"<section><h2>{escape(definition['title'])}</h2><p>{name}: Mac {escape(mac['status'])}; Lulo {escape(lulo['status'])}</p>")
        if mac.get("reason"):
            parts.append(f"<p>Mac skip: {escape(mac['reason'])}</p>")
        if mac.get("error") or lulo.get("error"):
            parts.append(f"<p>Errors: Mac {escape(mac.get('error', 'none'))}; Lulo {escape(lulo.get('error', 'none'))}</p>")
        for issue in lulo.get("issues", []):
            parts.append(f"<p>Lulo step {issue['index']}: {escape(issue['error'])}</p>")
        for step in definition["steps"]:
            if "shot" not in step:
                continue
            shot = step["shot"]
            m, l = msteps.get(shot), lsteps.get(shot)
            mp = args.mac / name / m["image"] if m and m.get("image") else None
            lp = args.lulo / name / l["image"] if l and l.get("image") else None
            image = f"images/{name}-{shot}.png"
            if (mp and mp.exists()) or (lp and lp.exists()):
                paired(mp, lp, m.get("point_size") if m else None, args.output / image)
                parts.append(f"<h3>{escape(shot)}</h3><img loading='lazy' src='{escape(image)}'>")
            else:
                image = None
            if m and m.get("error"):
                parts.append(f"<p>Mac: {escape(m['error'])}</p>")
            if l and l.get("error"):
                parts.append(f"<p>Lulo: {escape(l['error'])}</p>")
            parts.append("<table><tr><th></th><th>Mac</th><th>Lulo</th></tr>"
                         f"<tr><th>First change</th><td>{m.get('first_change_ms') if m else '—'} ms</td>"
                         f"<td>{l.get('first_change_ms') if l else '—'} ms</td></tr>"
                         f"<tr><th>Settled</th><td>{m.get('settled_ms') if m else '—'} ms</td>"
                         f"<td>{l.get('settled_ms') if l else '—'} ms</td></tr></table>")
            rows.append({"name": shot, "image": image, "mac": m, "lulo": l})
            summary["counts"]["mac_shots"] += bool(m)
            summary["counts"]["lulo_shots"] += bool(l)
            summary["counts"]["paired_shots"] += bool(m and l)
        parts.append("</section>")
        summary["journeys"].append({"name": name, "mac_status": mac["status"],
                                    "lulo_status": lulo["status"], "steps": rows})
        summary["counts"]["journeys"] += 1
    html = args.output / "index.html"
    html.write_text("\n".join(parts) + "\n")
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"{html}\n{args.output / 'summary.json'}\n{summary['counts']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
