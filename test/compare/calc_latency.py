#!/usr/bin/env python3
# calc_latency.py — 穩態窗口延遲統計
# 用法：calc_latency.py <latency_csv> [start_s] [end_s]
# CSV 欄位：ts,receive_unix,latency_ms（tools/test/server.go 寫出）
# 窗口以第一筆 record 的 receive_unix 為 0 點，只取 [start_s, end_s] 內的資料，
# 預設 30~230 秒，避開暖機與收尾 drain 的影響
import csv
import math
import sys


def percentile(sorted_vals, p):
    # 與 tools/test/server.go 的 percentile 對齊：線性插值
    n = len(sorted_vals)
    if n == 0:
        return 0.0
    k = (n - 1) * p / 100.0
    lo = math.floor(k)
    hi = min(lo + 1, n - 1)
    return sorted_vals[lo] + (k - lo) * (sorted_vals[hi] - sorted_vals[lo])


def main():
    if len(sys.argv) < 2:
        print("用法：calc_latency.py <latency_csv> [start_s] [end_s]", file=sys.stderr)
        return 2
    path = sys.argv[1]
    start = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0
    end = float(sys.argv[3]) if len(sys.argv) > 3 else 230.0

    rows = []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            rows.append((float(row["receive_unix"]), float(row["latency_ms"])))
    if not rows:
        print("[calc] CSV 無資料")
        return 1

    t0 = min(t for t, _ in rows)
    lats = sorted(lat for t, lat in rows if start <= t - t0 <= end)

    print(f"[calc] ── 穩態延遲統計（第 {start:g}~{end:g} 秒，0 點 = 第一筆 receive）──")
    print(f"[calc] 來源：{path}")
    print(f"[calc] 窗口內 {len(lats)} 筆 / 全部 {len(rows)} 筆")
    if not lats:
        print("[calc] 窗口內無資料")
        return 1

    print(f"[calc]   mean   = {sum(lats) / len(lats):10.3f} ms")
    print(f"[calc]   min    = {lats[0]:10.3f} ms")
    for p in (50, 90, 95, 99, 99.9):
        print(f"[calc]   p{p:<5g} = {percentile(lats, p):10.3f} ms")
    print(f"[calc]   max    = {lats[-1]:10.3f} ms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
