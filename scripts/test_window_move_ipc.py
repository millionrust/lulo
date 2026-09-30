"""The window runner reads niri's newline-delimited socket protocol."""

from __future__ import annotations

import socket
import sys
import tempfile
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "behavior"))
import run_window_move  # noqa: E402


class WindowMoveIpcTests(unittest.TestCase):
    def test_windows_query_retries_a_failed_reply(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "niri.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen(2)
                requests = []

                def serve():
                    for reply in (b'{"Err":"busy"}\n', b'{"Ok":{"Windows":[{"app_id":"test"}]}}\n'):
                        connection, _ = server.accept()
                        with connection:
                            requests.append(connection.recv(1024))
                            connection.sendall(reply)

                worker = threading.Thread(target=serve, daemon=True)
                worker.start()
                runner = object.__new__(run_window_move.Run)
                runner.env = {"NIRI_SOCKET": path}
                self.assertEqual(runner.niri("windows"), [{"app_id": "test"}])
                worker.join(timeout=2)
                self.assertFalse(worker.is_alive())
                self.assertEqual(requests, [b'"Windows"\n', b'"Windows"\n'])


if __name__ == "__main__":
    unittest.main()
