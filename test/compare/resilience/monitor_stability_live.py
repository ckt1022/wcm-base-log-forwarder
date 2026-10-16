#!/usr/bin/env python3
"""Print live generator, validation-sink, and DUT cgroup metrics."""

from __future__ import annotations

import argparse
import csv
import json
import signal
import time
import urllib.request
from datetime import timedelta
from pathlib import Path


running = True


def stop(_signum, _frame):
    global running
    running = False


def last_csv_row(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    with path.open(newline="") as handle:
        rows = csv.DictReader(handle)
        last = {}
        for row in rows:
            last = row
        return last


def sink_status(url: str) -> dict[str, object]:
    try:
        with urllib.request.urlopen(url, timeout=1.0) as response:
            return json.load(response)
    except Exception:
        return {}


def number(row: dict[str, object], key: str) -> float:
    try:
        return float(row.get(key, 0) or 0)
    except (TypeError, ValueError):
        return 0.0


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--status-url", default="http://127.0.0.1:8080/status")
    parser.add_argument("--interval", type=float, default=5.0)
    parser.add_argument("--wire-bytes", type=int, default=1025)
    parser.add_argument("--log-file", type=Path)
    args = parser.parse_args()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    started = time.monotonic()
    log_handle = args.log_file.open("a", buffering=1) if args.log_file else None

    try:
        while running:
            gen = last_csv_row(args.run_dir / "generator_per_sec.csv")
            stats = last_csv_row(args.run_dir / "stats.csv")
            sink = sink_status(args.status_url)

            input_eps = number(gen, "send_eps")
            input_mbs = input_eps * args.wire_bytes / 1_000_000
            pending = int(number(gen, "pending"))
            phase = str(gen.get("phase", "waiting"))
            output_eps = number(sink, "last_1s_eps")
            output_rolling = number(sink, "rolling_10s_eps")
            p50 = number(sink, "last_1s_p50_ms")
            p99 = number(sink, "last_1s_p99_ms")
            cpu = number(stats, "cpu_pct")
            memory = number(stats, "rss_mb")
            received = int(number(sink, "received_total"))

            elapsed = str(timedelta(seconds=int(time.monotonic() - started)))
            line = (
                f"[live {elapsed}] phase={phase} "
                f"input={input_eps:.0f} EPS/{input_mbs:.3f} MB/s pending={pending} | "
                f"server={output_eps:.0f} EPS (10s={output_rolling:.1f}) received={received} | "
                f"CPU={cpu:.1f}% MEM={memory:.2f} MiB | "
                f"p50={p50:.3f} ms p99={p99:.3f} ms"
            )
            print(line, flush=True)
            if log_handle:
                print(line, file=log_handle, flush=True)

            deadline = time.monotonic() + args.interval
            while running and time.monotonic() < deadline:
                time.sleep(min(0.2, deadline - time.monotonic()))
    finally:
        if log_handle:
            log_handle.close()


if __name__ == "__main__":
    main()
