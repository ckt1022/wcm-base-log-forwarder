#!/usr/bin/env python3
"""
Mock HTTP sink for WCM benchmark tests.
Accepts POST /ingest, counts received lines and computes throughput/latency.

Usage:
  python3 sink_server.py [--port 8080] [--report-interval 5] [--out metrics.csv]
"""

import argparse
import csv
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Lock


class Stats:
    def __init__(self):
        self.lock = Lock()
        self.total_lines = 0
        self.latency_sum = 0.0
        self.latency_count = 0
        self.window_lines = 0
        self.window_start = time.monotonic()

    def record(self, lines, latencies):
        with self.lock:
            self.total_lines += lines
            self.window_lines += lines
            for lat in latencies:
                self.latency_sum += lat
                self.latency_count += 1

    def flush_window(self):
        with self.lock:
            now = time.monotonic()
            elapsed = now - self.window_start
            tput = self.window_lines / elapsed if elapsed > 0 else 0
            avg_lat = (self.latency_sum / self.latency_count * 1000) if self.latency_count > 0 else 0
            result = {
                "ts": time.time(),
                "window_secs": round(elapsed, 2),
                "lines_in_window": self.window_lines,
                "throughput_lps": round(tput, 1),
                "avg_latency_ms": round(avg_lat, 3),
                "total_lines": self.total_lines,
            }
            self.window_lines = 0
            self.window_start = now
            self.latency_sum = 0.0
            self.latency_count = 0
            return result


STATS = Stats()
CSV_WRITER = None
CSV_FILE = None


class SinkHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode(errors="replace")
        recv_ts = time.time()

        lines = [l for l in body.splitlines() if l.strip()]
        latencies = []
        for line in lines:
            try:
                obj = json.loads(line)
                # Fluent Bit json_lines format: [timestamp, {record}]
                # Flat dict format: {"field": ...}
                record = obj[1] if isinstance(obj, list) and len(obj) == 2 else obj
                write_ts = record.get("write_ts") if isinstance(record, dict) else None
                if write_ts:
                    latencies.append(recv_ts - float(write_ts))
            except Exception:
                pass

        STATS.record(len(lines), latencies)

        self.send_response(200)
        self.end_headers()

    def log_message(self, *args):
        pass


def reporter(interval, writer):
    while True:
        time.sleep(interval)
        row = STATS.flush_window()
        print(
            f"[sink] tput={row['throughput_lps']} lps  "
            f"lat={row['avg_latency_ms']} ms  "
            f"total={row['total_lines']}",
            file=sys.stderr,
        )
        if writer:
            writer.writerow(row)
            CSV_FILE.flush()


def main():
    global CSV_WRITER, CSV_FILE

    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=8080)
    p.add_argument("--report-interval", type=int, default=5)
    p.add_argument("--out", default="sink_metrics.csv")
    args = p.parse_args()

    CSV_FILE = open(args.out, "w", newline="")
    fieldnames = ["ts", "window_secs", "lines_in_window", "throughput_lps", "avg_latency_ms", "total_lines"]
    CSV_WRITER = csv.DictWriter(CSV_FILE, fieldnames=fieldnames)
    CSV_WRITER.writeheader()

    import threading
    t = threading.Thread(target=reporter, args=(args.report_interval, CSV_WRITER), daemon=True)
    t.start()

    print(f"[sink] listening on port {args.port}, writing to {args.out}", file=sys.stderr)
    server = HTTPServer(("0.0.0.0", args.port), SinkHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        row = STATS.flush_window()
        print(f"[sink] final total={row['total_lines']} lines", file=sys.stderr)
    finally:
        CSV_FILE.close()


if __name__ == "__main__":
    main()
