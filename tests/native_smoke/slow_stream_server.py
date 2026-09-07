#!/usr/bin/env python3
"""Tiny chunked-streaming HTTP server for the native fetch-stream smoke (tests/native_smoke/fetch_stream_app.tish).

GET /slow?secs=N   streams one 101-byte chunk every 0.5s for N seconds, then ends the body cleanly.
GET /stall         sends headers + one chunk, then never sends another byte (idle-stall probe).

Usage: slow_stream_server.py PORT
"""
import http.server
import socketserver
import sys
import time


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # keep CI logs quiet
        pass

    def _chunk(self, data: bytes) -> None:
        self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
        self.wfile.flush()

    def do_GET(self):  # noqa: N802 — http.server naming
        try:
            if self.path.startswith("/slow"):
                secs = float(self.path.split("secs=")[1]) if "secs=" in self.path else 5.0
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                t0 = time.time()
                while time.time() - t0 < secs:
                    self._chunk(b"x" * 100 + b"\n")
                    time.sleep(0.5)
                self.wfile.write(b"0\r\n\r\n")
                self.wfile.flush()
            elif self.path.startswith("/stall"):
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                self._chunk(b"hello\n")
                time.sleep(600)
            else:
                body = b'{"ok":true}'
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # the client hung up (expected on the timeout cases)


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18888
    Server(("127.0.0.1", port), Handler).serve_forever()
