#!/usr/bin/env python3
"""
Simple HTTP log receiver for testing the transport WASM component.
Listens on port 8080, accepts POST /ingest, prints per-batch summary only.
Uses ThreadingHTTPServer to handle concurrent requests from multiple workers.
"""

import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn

PORT = 8080

# Thread-safe counters
_lock = threading.Lock()
_total_batches = 0
_total_bytes = 0
_total_lines = 0


class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    """Handles each request in a separate thread."""
    daemon_threads = True


class LogReceiver(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/ingest":
            self.send_response(404)
            self.end_headers()
            return

        length_header = self.headers.get("Content-Length")
        if length_header is not None:
            body = self.rfile.read(int(length_header))
        else:
            chunks = []
            while True:
                chunk = self.rfile.read(4096)
                if not chunk:
                    break
                chunks.append(chunk)
            body = b"".join(chunks)

        n_lines = body.count(b"\n")
        #print(body)
        global _total_batches, _total_bytes, _total_lines
        with _lock:
            _total_batches += 1
            _total_bytes += len(body)
            _total_lines += n_lines
            batch_num = _total_batches
            tb = _total_bytes
            tl = _total_lines

        print(
            f"[server] batch #{batch_num:>5}  {len(body):>8} B  {n_lines:>6} lines"
            f"  | cumulative: {tb:>10} B  {tl:>8} lines",
            flush=True,
        )

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, fmt, *args):
        pass  # suppress default access log


def main():
    server = ThreadingHTTPServer(("127.0.0.1", PORT), LogReceiver)
    print(f"[server] Listening on http://127.0.0.1:{PORT}/ingest (multi-threaded)")
    print("[server] Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        with _lock:
            print(
                f"\n[server] Stopped."
                f"  Total batches={_total_batches}"
                f"  bytes={_total_bytes}"
                f"  lines={_total_lines}"
            )


if __name__ == "__main__":
    main()
