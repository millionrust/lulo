#!/usr/bin/env python3
"""Measure colours and geometry from reference captures — no dependencies.

Decodes PNG in pure Python (zlib only), so it runs on the space-constrained Mac
and on the reference PC without Pillow.

Usage:
  # colour at points
  python3 scripts/measure-reference.py sample shot.png 100,620 700,600

  # find horizontal edges along a row (panel bounds, sidebar width, gaps)
  python3 scripts/measure-reference.py edges-row shot.png 620 --from 0 --to 400

  # find vertical edges along a column (bar heights, row pitch, shelf bounds)
  python3 scripts/measure-reference.py edges-col shot.png 300 --from 0 --to 200

  # solve a material's tint and alpha from the same surface over two backgrounds
  python3 scripts/measure-reference.py solve over-black.png over-white.png 960,280
"""
from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path


def load(path: str):
    data = Path(path).read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", f"{path} is not a PNG"
    i, idat, w, h, depth, ctype = 8, b"", None, None, None, None
    while i < len(data):
        ln = struct.unpack(">I", data[i:i + 4])[0]
        typ = data[i + 4:i + 8]
        chunk = data[i + 8:i + 8 + ln]
        i += 12 + ln
        if typ == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", chunk[:10])
        elif typ == b"IDAT":
            idat += chunk
        elif typ == b"IEND":
            break
    assert depth == 8, "only 8-bit PNGs are supported"
    channels = {0: 1, 2: 3, 4: 2, 6: 4}[ctype]
    raw = zlib.decompress(idat)
    stride = w * channels
    out = bytearray(h * stride)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        filt = raw[p]; p += 1
        line = bytearray(raw[p:p + stride]); p += stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if filt == 1:
                line[x] = (line[x] + a) & 255
            elif filt == 2:
                line[x] = (line[x] + b) & 255
            elif filt == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif filt == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return {"w": w, "h": h, "ch": channels, "data": bytes(out)}


def px(img, x, y):
    o = (y * img["w"] + x) * img["ch"]
    return tuple(img["data"][o:o + 3])


def hexa(p):
    return "%02X%02X%02X" % p


def arg(name, default):
    if name in sys.argv:
        return int(sys.argv[sys.argv.index(name) + 1])
    return default


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    mode, path = sys.argv[1], sys.argv[2]

    if mode == "sample":
        img = load(path)
        for spec in sys.argv[3:]:
            x, y = (int(v) for v in spec.split(","))
            print(f"{spec:>12}  #{hexa(px(img, x, y))}")
        return 0

    if mode in ("edges-row", "edges-col"):
        img = load(path)
        fixed = int(sys.argv[3])
        start = arg("--from", 0)
        end = arg("--to", img["w"] if mode == "edges-row" else img["h"])
        threshold = arg("--threshold", 14)
        prev = None
        hits = []
        for v in range(start, end):
            p = px(img, v, fixed) if mode == "edges-row" else px(img, fixed, v)
            mean = sum(p) / 3
            if prev is not None and abs(mean - prev) > threshold:
                hits.append(v)
            prev = mean
        print(f"edges: {hits}")
        if len(hits) >= 2:
            print(f"span {hits[0]}..{hits[-1]} = {hits[-1] - hits[0]} px")
            gaps = [hits[i + 1] - hits[i] for i in range(len(hits) - 1)]
            print(f"gaps: {gaps[:20]}")
        return 0

    if mode == "solve":
        # material over black vs the same material over white
        black, white = load(sys.argv[2]), load(sys.argv[3])
        for spec in sys.argv[4:]:
            x, y = (int(v) for v in spec.split(","))
            cb, cw = px(black, x, y), px(white, x, y)
            alphas, tints = [], []
            for i in range(3):
                alpha = 1.0 - (cw[i] - cb[i]) / 255.0
                alphas.append(alpha)
                tints.append(cb[i] / alpha if alpha > 0.02 else 0.0)
            alpha = sum(alphas) / 3
            tint = tuple(min(255, int(round(t))) for t in tints)
            print(f"{spec:>12}  tint #{hexa(tint)}  alpha {alpha:.3f} "
                  f"(0x{int(alpha * 255):02X})")
        return 0

    print(__doc__)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
