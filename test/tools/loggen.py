#!/usr/bin/env python3
"""
Log generator for WCM benchmark tests.
Writes JSON lines to a file or TCP at a controlled rate.

Usage:
  python3 loggen.py --mode steady  [--rate 5000] [--duration 120] [--output /tmp/test-logs.log]
  python3 loggen.py --mode burst   [--base 2000] [--peak 20000] [--cycles 3]
  python3 loggen.py --mode ramp    [--start 1000] [--max 20000] [--step 2000] [--step-secs 30]
  python3 loggen.py --mode steady  --target tcp [--host 127.0.0.1] [--port 24224]
"""

import argparse
import json
import random
import socket
import string
import sys
import time


def rand_str(n=32):
    return ''.join(random.choices(string.ascii_lowercase + string.digits, k=n))


def make_record():
    return json.dumps({
        "level":    random.choice(["INFO", "WARN", "ERROR", "DEBUG"]),
        "msg":      random.choice(["request received", "timeout", "retry", "connection closed", "parsed ok"]),
        "ts":       int(time.time() * 1000),
        "host":     f"node-{random.randint(1, 16):02d}",
        "payload":  rand_str(64),
        "write_ts": time.time(),
    })


_BAD_TEMPLATES = [
    lambda: "{broken",                              # 截斷 JSON
    lambda: '{"level": "INFO", "msg":}',           # 語法錯誤
    lambda: rand_str(32),                           # 純文字，非 JSON
    lambda: '{"level": null, "ts": "not-a-number"',# 型態錯誤且截斷
    lambda: "",                                     # 空行
]


def make_bad_record():
    return random.choice(_BAD_TEMPLATES)()


class FileSink:
    def __init__(self, path):
        self.f = open(path, "a", buffering=1)

    def write(self, line):
        self.f.write(line + "\n")

    def close(self):
        self.f.close()


class TcpSink:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port))

    def write(self, line):
        self.sock.sendall((line + "\n").encode())

    def close(self):
        self.sock.close()


def emit_at_rate(sink, rate_per_sec, duration_secs, error_rate=0.0):
    interval = 1.0 / rate_per_sec
    total = rate_per_sec * duration_secs
    sent = bad = 0
    t_start = time.monotonic()
    while sent < total:
        if error_rate > 0.0 and random.random() < error_rate:
            sink.write(make_bad_record())
            bad += 1
        else:
            sink.write(make_record())
        sent += 1
        next_t = t_start + sent * interval
        sleep = next_t - time.monotonic()
        if sleep > 0:
            time.sleep(sleep)
    elapsed = time.monotonic() - t_start
    print(
        f"[loggen] sent {sent} lines in {elapsed:.1f}s "
        f"({sent/elapsed:.0f} lines/s, {bad} bad/{sent} total)",
        file=sys.stderr,
    )


def mode_steady(sink, args):
    print(f"[loggen] steady {args.rate} lines/s × {args.duration}s  error_rate={args.error_rate:.1%}", file=sys.stderr)
    emit_at_rate(sink, args.rate, args.duration, args.error_rate)


def mode_burst(sink, args):
    print(f"[loggen] burst base={args.base} peak={args.peak} cycles={args.cycles}  error_rate={args.error_rate:.1%}", file=sys.stderr)
    for i in range(args.cycles):
        print(f"[loggen] cycle {i+1}/{args.cycles}: base 20s", file=sys.stderr)
        emit_at_rate(sink, args.base, 20, args.error_rate)
        print(f"[loggen] cycle {i+1}/{args.cycles}: peak 20s", file=sys.stderr)
        emit_at_rate(sink, args.peak, 20, args.error_rate)
    print(f"[loggen] final base 20s", file=sys.stderr)
    emit_at_rate(sink, args.base, 20, args.error_rate)


def mode_ramp(sink, args):
    rate = args.start
    print(f"[loggen] ramp {args.start} → {args.max} step={args.step} every {args.step_secs}s  error_rate={args.error_rate:.1%}", file=sys.stderr)
    while rate <= args.max:
        print(f"[loggen] rate={rate} lines/s", file=sys.stderr)
        emit_at_rate(sink, rate, args.step_secs, args.error_rate)
        rate += args.step


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--mode", choices=["steady", "burst", "ramp"], default="steady")
    p.add_argument("--target", choices=["file", "tcp"], default="file")
    p.add_argument("--output", default="/tmp/test-logs.log")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=24224)
    # steady
    p.add_argument("--rate", type=int, default=5000)
    p.add_argument("--duration", type=int, default=120)
    # burst
    p.add_argument("--base", type=int, default=2000)
    p.add_argument("--peak", type=int, default=20000)
    p.add_argument("--cycles", type=int, default=3)
    # ramp
    p.add_argument("--start", type=int, default=1000)
    p.add_argument("--max", type=int, default=20000)
    p.add_argument("--step", type=int, default=2000)
    p.add_argument("--step-secs", type=int, default=30)
    # error injection
    p.add_argument("--error-rate", type=float, default=0.0,
                   metavar="RATE", help="fraction of lines that are malformed (0.0–1.0)")
    args = p.parse_args()

    if args.target == "tcp":
        sink = TcpSink(args.host, args.port)
    else:
        sink = FileSink(args.output)

    try:
        {"steady": mode_steady, "burst": mode_burst, "ramp": mode_ramp}[args.mode](sink, args)
    finally:
        sink.close()


if __name__ == "__main__":
    main()
