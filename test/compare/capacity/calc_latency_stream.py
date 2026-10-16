#!/usr/bin/env python3
# calc_latency_stream.py — calc_latency.py 的高流量版(千萬行等級 CSV 不爆記憶體)
# 語意與 test/compare/calc_latency.py 完全一致:
#   窗口以第一筆 receive_unix 為 0 點,取 [start_s, end_s],百分位線性插值
# 差異:串流讀檔 + numpy float64 陣列(8B/筆),不建 Python tuple
# 用法: calc_latency_stream.py <latency_csv> [start_s] [end_s]
import sys

import numpy as np


def percentile(sorted_vals, p):
    n = len(sorted_vals)
    if n == 0:
        return 0.0
    k = (n - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, n - 1)
    return float(sorted_vals[lo] + (k - lo) * (sorted_vals[hi] - sorted_vals[lo]))


def main():
    if len(sys.argv) < 2:
        print("用法: calc_latency_stream.py <latency_csv> [start_s] [end_s]", file=sys.stderr)
        return 2
    path = sys.argv[1]
    start = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0
    end = float(sys.argv[3]) if len(sys.argv) > 3 else 230.0

    # 第一遍(廉價):只掃第一筆的 receive_unix 當 0 點
    # CSV 欄位: ts,receive_unix,latency_ms(ts 可為空)
    t0 = None
    recv = []
    lat = []
    CHUNK = 1 << 20
    n_total = 0
    with open(path) as f:
        header = f.readline().strip().split(",")
        i_recv = header.index("receive_unix")
        i_lat = header.index("latency_ms")
        buf_r = np.empty(CHUNK, dtype=np.float64)
        buf_l = np.empty(CHUNK, dtype=np.float64)
        n = 0
        for line in f:
            parts = line.rstrip("\n").split(",")
            r = float(parts[i_recv])
            n_total += 1
            if t0 is None or r < t0:
                t0 = r
            buf_r[n] = r
            buf_l[n] = float(parts[i_lat])
            n += 1
            if n == CHUNK:
                recv.append(buf_r.copy())
                lat.append(buf_l.copy())
                n = 0
        if n:
            recv.append(buf_r[:n].copy())
            lat.append(buf_l[:n].copy())

    if not recv:
        print("[calc] CSV 無資料")
        return 1

    r_all = np.concatenate(recv)
    del recv
    l_all = np.concatenate(lat)
    del lat

    t0 = float(r_all.min())
    rel = r_all - t0
    mask = (rel >= start) & (rel <= end)
    del rel, r_all
    lats = l_all[mask]
    del l_all, mask
    lats.sort()

    print(f"[calc] ── 穩態延遲統計(第 {start:g}~{end:g} 秒,0 點 = 第一筆 receive)──")
    print(f"[calc] 來源:{path}")
    print(f"[calc] 窗口內 {len(lats)} 筆 / 全部 {n_total} 筆")
    if len(lats) == 0:
        print("[calc] 窗口內無資料")
        return 1

    print(f"[calc]   mean   = {float(lats.mean()):10.3f} ms")
    print(f"[calc]   min    = {float(lats[0]):10.3f} ms")
    for p in (50, 90, 95, 99, 99.9):
        print(f"[calc]   p{p:<5g} = {percentile(lats, p):10.3f} ms")
    print(f"[calc]   max    = {float(lats[-1]):10.3f} ms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
