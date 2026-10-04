"""A tiny read-only stand-in for cupsd's local socket, for the private
nested behaviour session (scripts/behavior/fake_hardware.py).

System Settings ▸ Printers & Scanners reads printers and jobs as IPP over
HTTP on cupsd's domain socket (crates/rmac-printers-linux/src/cups.rs) and
honours CUPS_SERVER when it names a socket. This server answers the three
operations it sends -- CUPS-Get-Printers, CUPS-Get-Default and Get-Jobs --
with one idle default printer ("Lulo Laser") and one waiting job, so the
pane shows real data in nested runs. It never changes anything; changes go
through the cups-pk-helper mock (tests/dbusmock/cups_pk_helper.py).
"""

from __future__ import annotations

import socketserver
import struct
import threading
from pathlib import Path

CUPS_GET_PRINTERS = 0x4002
CUPS_GET_DEFAULT = 0x4001
GET_JOBS = 0x000A


def _attr(tag: int, name: str, value: bytes) -> bytes:
    encoded = name.encode()
    return bytes([tag]) + struct.pack(">H", len(encoded)) + encoded + struct.pack(">H", len(value)) + value


def _text(tag: int, name: str, value: str) -> bytes:
    return _attr(tag, name, value.encode())


def _int(tag: int, name: str, value: int) -> bytes:
    return _attr(tag, name, struct.pack(">i", value))


def _response(request_id: int, groups: list[tuple[int, bytes]]) -> bytes:
    body = bytes([2, 0]) + struct.pack(">HI", 0, request_id)
    body += bytes([0x01]) + _text(0x47, "attributes-charset", "utf-8") + _text(0x48, "attributes-natural-language", "en")
    for tag, attributes in groups:
        body += bytes([tag]) + attributes
    return body + bytes([0x03])


PRINTER = (
    _text(0x42, "printer-name", "Lulo_Laser")
    + _text(0x41, "printer-info", "Lulo Laser")
    + _text(0x41, "printer-location", "Study")
    + _text(0x41, "printer-make-and-model", "Lulo Laser 400")
    + _int(0x23, "printer-state", 3)
    + _text(0x44, "printer-state-reasons", "none")
    + _attr(0x22, "printer-is-accepting-jobs", b"\x01")
    + _attr(0x22, "printer-is-shared", b"\x00")
    + _int(0x23, "printer-type", 0x0004)
    + _text(0x45, "device-uri", "ipp://lulo-laser.local/ipp/print")
)
JOB = (
    _int(0x21, "job-id", 7)
    + _text(0x42, "job-name", "Lulo Test Page")
    + _text(0x42, "job-originating-user-name", "lulo")
    + _int(0x23, "job-state", 3)
    + _int(0x21, "job-k-octets", 12)
)


class _Handler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.rfile.read1(4096)
            if not chunk:
                return
            head += chunk
        header, _, body = head.partition(b"\r\n\r\n")
        length = 0
        for line in header.split(b"\r\n"):
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1].strip())
        while len(body) < length:
            chunk = self.rfile.read1(4096)
            if not chunk:
                return
            body += chunk
        if len(body) < 8:
            return
        operation, request_id = struct.unpack(">HI", body[2:8])
        if operation == CUPS_GET_PRINTERS:
            reply = _response(request_id, [(0x04, PRINTER)])
        elif operation == CUPS_GET_DEFAULT:
            reply = _response(request_id, [(0x04, _text(0x42, "printer-name", "Lulo_Laser"))])
        elif operation == GET_JOBS:
            reply = _response(request_id, [(0x02, JOB)])
        else:
            reply = bytes([2, 0]) + struct.pack(">HI", 0x0501, request_id) + bytes([0x03])
        self.wfile.write(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: "
            + str(len(reply)).encode()
            + b"\r\nConnection: close\r\n\r\n"
            + reply
        )


class FakeCupsd:
    def __init__(self, socket_path: Path) -> None:
        self.socket_path = socket_path
        if socket_path.exists():
            socket_path.unlink()
        self._server = socketserver.ThreadingUnixStreamServer(str(socket_path), _Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        try:
            self.socket_path.unlink()
        except OSError:
            pass
