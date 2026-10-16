#!/usr/bin/env python3
"""correct_latency.py <run_dir> — 時鐘跳躍校正後的延遲與資源統計（30–150 s 窗）。

延遲語意與 capacity/calc_latency_stream.py 完全相同（窗口以第一筆 receive 為 0 點、
取 [30,150] 秒、百分位線性插值），只是：
  * gen 的 ts = start_wall + 排程偏移（Go time.Add，不受之後的 wall 跳躍影響），
    sink 的 receive = time.Now() wall（受跳躍影響）。
    校正延遲 = 量測延遲 − [D(receive) − D(gen_start)]，D = wall − monotonic（clock_drift.csv）。
  * 窗口 0 點與 30/150 s 邊界也在 monotonic 時間軸上計算。
資源：stats.csv 的 elapsed_s 是 wall 差（跳躍時該秒 CPU% 被低估）；改以
  CPU mean = Δcpu_time_s / Δmonotonic（窗內第一與最後樣本），Memory mean = 窗內 rss 平均。
輸出：latency_stats_30_150.txt（同 calc_latency_stream 格式）、resource_stats_30_150.txt、
      latency_stats_30_150_raw.txt（未校正，供對照）、subwindows_clockfix.txt
"""
import csv
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from clockfix import Clock  # noqa: E402


def pct(v, p):
    n = len(v)
    if n == 0:
        return 0.0
    k = (n - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, n - 1)
    return float(v[lo] + (k - lo) * (v[hi] - v[lo]))


def stats_block(lats, n_total, title, src):
    lats = np.sort(lats)
    out = [f"[calc] ── 穩態延遲統計(第 30~150 秒,0 點 = 第一筆 receive){title}──",
           f"[calc] 來源:{src}",
           f"[calc] 窗口內 {len(lats)} 筆 / 全部 {n_total} 筆"]
    if len(lats):
        out.append(f"[calc]   mean   = {lats.mean():10.3f} ms")
        out.append(f"[calc]   min    = {lats[0]:10.3f} ms")
        for name, p in (("p50", 50), ("p90", 90), ("p95", 95), ("p99", 99), ("p99.9", 99.9)):
            out.append(f"[calc]   {name:<6} = {pct(lats, p):10.3f} ms")
        out.append(f"[calc]   max    = {lats[-1]:10.3f} ms")
    return "\n".join(out) + "\n"


def main(d):
    ck = Clock(d)
    gen_start, gen_end = map(float, open(f"{d}/gen_epoch.txt").read().split())
    d0 = ck.D(gen_start)
    src = f"{d}/latency_worker_1.csv"
    x = np.loadtxt(src, delimiter=",", skiprows=1, usecols=(1, 2), ndmin=2)
    recv, lat = x[:, 0], x[:, 1]
    walls = np.asarray(ck.walls)
    drifts = np.asarray(ck.drifts)
    idx = np.clip(np.searchsorted(walls, recv, side="right") - 1, 0, len(walls) - 1)
    Drecv = drifts[idx]
    lat_c = lat - (Drecv - d0) * 1000.0
    mono = recv - Drecv
    rel_c = mono - mono.min()
    rel_r = recv - recv.min()
    m_c = (rel_c >= 30) & (rel_c <= 150)
    m_r = (rel_r >= 30) & (rel_r <= 150)
    open(f"{d}/latency_stats_30_150.txt", "w").write(
        stats_block(lat_c[m_c], len(lat), " [clock-step corrected]", src))
    open(f"{d}/latency_stats_30_150_raw.txt", "w").write(
        stats_block(lat[m_r], len(lat), " [RAW, uncorrected wall clock]", src))
    sub = []
    for s in range(0, 180, 30):
        v = lat_c[(rel_c >= s) & (rel_c < s + 30)]
        if len(v):
            sub.append(f"{s}-{s + 30}s p50={np.percentile(v, 50):.1f} p99={np.percentile(v, 99):.1f}")
    steps = [f"{w - gen_start:+.1f}s:{st:+.3f}" for w, st in ck.steps() if gen_start - 5 <= w <= gen_end + 30]
    open(f"{d}/subwindows_clockfix.txt", "w").write(" | ".join(sub) + "\nclock steps (rel. gen start): " + " ".join(steps) + "\n")
    # 資源
    sp = f"{d}/stats.csv"
    if os.path.exists(sp):
        rows = list(csv.DictReader(open(sp)))
        smp = []
        for r in rows:
            w = gen_start + float(r["elapsed_s"])
            smp.append((ck.mono(w) - ck.mono(gen_start), float(r["cpu_time_s"]), float(r["rss_mb"]), float(r["cpu_pct"])))
        win = [s for s in smp if 30 <= s[0] <= 150]
        if len(win) >= 2:
            cpu = (win[-1][1] - win[0][1]) / (win[-1][0] - win[0][0]) * 100
            mem = sum(s[2] for s in win) / len(win)
            open(f"{d}/resource_stats_30_150.txt", "w").write(
                f"[stats] window=30-150s(mono) samples={len(win)} cpu_mean={cpu:.3f}% cpu_max={max(s[3] for s in win):.3f}% "
                f"mem_mean={mem:.3f}MiB mem_max={max(s[2] for s in win):.3f}MiB [cpu_mean=Δcpu_time/Δmono]\n")


if __name__ == "__main__":
    for d in sys.argv[1:]:
        main(d.rstrip("/"))
