#!/usr/bin/env python3
"""
Simple HTTP log receiver for testing the transport WASM component.
Listens on port 8080, accepts POST /ingest, prints per-batch summary with throughput.
Uses ThreadingHTTPServer to handle concurrent requests from multiple workers.
"""

import sys
import time
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn

PORT = 5000

# Thread-safe counters
_lock = threading.Lock()
_total_batches = 0
_total_bytes   = 0
_total_lines   = 0
_start_time    = None   # set on first request
_last_time     = None   # time of previous request (for per-batch tput)


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
        now = time.monotonic()

        global _total_batches, _total_bytes, _total_lines, _start_time, _last_time
        with _lock:
            if _start_time is None:
                _start_time = now

            # Instantaneous throughput: lines in this batch / time since last batch
            if _last_time is not None and now > _last_time:
                batch_tput = n_lines / (now - _last_time)
            else:
                batch_tput = 0.0
            _last_time = now

            _total_batches += 1
            _total_bytes   += len(body)
            _total_lines   += n_lines
            batch_num = _total_batches
            tb = _total_bytes
            tl = _total_lines

            # Cumulative average throughput since first request
            elapsed = now - _start_time
            avg_tput = tl / elapsed if elapsed > 0 else 0.0

        print(
            f"[server] #{batch_num:>5}  {len(body):>10}B  {n_lines:>6} lines"
            f"  {batch_tput:>8.0f}/s"
            f"  | total: {tb:>12}B  {tl:>8} lines  avg={avg_tput:>8.0f}/s",
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
    print(
        f"{'#':>7}  {'bytes':>10}   {'lines':>6}  {'batch/s':>8}"
        f"  | {'total bytes':>12}  {'total lines':>8}  {'avg/s':>12}",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        with _lock:
            elapsed = (time.monotonic() - _start_time) if _start_time else 0
            final_avg = _total_lines / elapsed if elapsed > 0 else 0
            print(
                f"\n[server] Stopped."
                f"  batches={_total_batches}"
                f"  bytes={_total_bytes}"
                f"  lines={_total_lines}"
                f"  avg={final_avg:.0f} lines/s"
            )


if __name__ == "__main__":
    main()
