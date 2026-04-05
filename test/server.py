#!/usr/bin/env python3
"""
Simple HTTP log receiver for testing the transport WASM component.
Listens on port 8080, accepts POST /ingest, prints received payload.
"""

import sys
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = 8080
RECEIVED_BATCHES = []


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
            # No Content-Length: read in chunks until done
            chunks = []
            while True:
                chunk = self.rfile.read(4096)
                if not chunk:
                    break
                chunks.append(chunk)
            body = b"".join(chunks)

        RECEIVED_BATCHES.append(body)

        print(f"\n[server] --- Received batch #{len(RECEIVED_BATCHES)} ---")
        print(f"[server] Content-Type: {self.headers.get('Content-Type', '(none)')}")
        print(f"[server] Bytes: {len(body)}")

        # Try to print each line
        for i, line in enumerate(body.splitlines(), 1):
            try:
                decoded = line.decode("utf-8")
                print(f"[server] Line {i}: {decoded}")
            except Exception:
                print(f"[server] Line {i}: <binary {len(line)} bytes>")

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, fmt, *args):
        # Suppress default access log (we print our own)
        pass


def main():
    server = HTTPServer(("127.0.0.1", PORT), LogReceiver)
    print(f"[server] Listening on http://127.0.0.1:{PORT}/ingest")
    print("[server] Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print(f"\n[server] Stopped. Total batches received: {len(RECEIVED_BATCHES)}")


if __name__ == "__main__":
    main()
