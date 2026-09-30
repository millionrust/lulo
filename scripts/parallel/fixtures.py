"""Small disposable Preview fixtures created inside each journey sandbox."""

from __future__ import annotations

from pathlib import Path
import zlib
import struct


def prepare(sandbox: Path, setup: dict) -> None:
    for name, content in setup.get("files", {}).items():
        path = sandbox / name
        if name.endswith("/"):
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content or "")
    for name in setup.get("fixtures", []):
        if name == "sample.pdf":
            objects = [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
                b"<< /Length 54 >>\nstream\nBT /F1 22 Tf 40 220 Td (Parallel Preview Sample) Tj ET\nendstream",
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            ]
            data = bytearray(b"%PDF-1.4\n")
            offsets = [0]
            for index, obj in enumerate(objects, 1):
                offsets.append(len(data))
                data += f"{index} 0 obj\n".encode() + obj + b"\nendobj\n"
            xref = len(data)
            data += f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode()
            for offset in offsets[1:]:
                data += f"{offset:010d} 00000 n \n".encode()
            data += f"trailer << /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
            (sandbox / name).write_bytes(data)
        elif name == "sample.png":
            def chunk(kind: bytes, body: bytes) -> bytes:
                return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
            rows = b"".join(b"\0" + b"".join(bytes((x * 2, y * 2, 100)) for x in range(100)) for y in range(100))
            png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 100, 100, 8, 2, 0, 0, 0))
            png += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
            (sandbox / name).write_bytes(png)
        else:
            raise ValueError(f"unknown fixture {name}")
