#!/usr/bin/env python3
"""Fault isolation 實驗的穩定負載產生器（含 marker 注入）。

以固定 EPS 對 TCP 送出 1 KiB JSON 記錄；在 --fault-at 秒注入一筆（或數筆）
msg 含 FAULT_* marker 的記錄，使正常批次在故障前後各跑一段時間。

每筆記錄含遞增的 "seq" 欄位，sink 以此估算遺失。輸出 summary JSON。

用法：fault_gen.py --addr 127.0.0.1:5140 --eps 5000 --secs 40 --fault-at 15 \
          --marker FAULT_LOOP [--marker-count 1] --summary out.json
執行環境以 -I 啟動（避免載入故障注入目錄下的同名模組）。
"""
import argparse
import json
import socket
import time


def build_record(seq: int, level: str, msg: str) -> bytes:
    prefix = (
        f'{{"ts":"2026-10-13T00:00:00Z","level":"{level}","seq":{seq},"msg":"'
    )
    suffix = '","att":{"service":"fault-bench","code":"200"}}'
    pad = 1024 - len(prefix) - len(msg) - len(suffix) - 1  # -1 for newline
    if pad < 0:
        raise ValueError("record too long")
    line = prefix + msg + ("x" * pad) + suffix + "\n"
    assert len(line) == 1024, len(line)
    return line.encode()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--addr", default="127.0.0.1:5140")
    ap.add_argument("--eps", type=int, default=5000)
    ap.add_argument("--secs", type=int, default=40)
    ap.add_argument("--fault-at", type=int, default=15)
    ap.add_argument("--marker", default="")  # 空 = baseline，不注入故障
    ap.add_argument("--marker-count", type=int, default=1)
    ap.add_argument("--summary", default="generator_summary.json")
    args = ap.parse_args()

    host, port = args.addr.split(":")
    sock = socket.create_connection((host, int(port)))
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    total = args.eps * args.secs
    start = time.monotonic()
    sent = 0
    fault_seq = -1
    buf = bytearray()
    # 每 20ms 一個 batch（與其他實驗的 batch-lines/flush 節奏相近）
    per_tick = max(1, args.eps // 50)
    tick = 0
    while sent < total:
        target_time = start + (sent / args.eps)
        now = time.monotonic()
        if target_time > now:
            time.sleep(target_time - now)
        n = min(per_tick, total - sent)
        elapsed = time.monotonic() - start
        for _ in range(n):
            level = "warn" if sent % 2 == 0 else "info"
            # 在 fault-at 時刻注入 marker 記錄
            if args.marker and elapsed >= args.fault_at and fault_seq < 0:
                for k in range(args.marker_count):
                    buf += build_record(sent, "warn", f"{args.marker} injected marker {k}")
                    if k == 0:
                        fault_seq = sent
                    sent += 1
                    if sent >= total:
                        break
                continue
            buf += build_record(sent, level, f"normal message {sent}")
            sent += 1
        sock.sendall(buf)
        buf.clear()
        tick += 1

    elapsed = time.monotonic() - start
    sock.shutdown(socket.SHUT_WR)
    try:
        sock.recv(4096)
    except OSError:
        pass
    sock.close()
    summary = {
        "sent": sent,
        "eps_target": args.eps,
        "eps_actual": sent / elapsed if elapsed else 0,
        "elapsed_sec": elapsed,
        "marker": args.marker,
        "marker_count": args.marker_count if args.marker else 0,
        "fault_at_sec": args.fault_at,
        "fault_seq": fault_seq,
    }
    with open(args.summary, "w") as f:
        json.dump(summary, f, indent=2)
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
