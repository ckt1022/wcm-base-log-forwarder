#!/usr/bin/env python3
"""subwindows.py <run_dir>... — 30 s 子窗口 p50/p99（同正式 analyze_*_windows.py 的窗切法），用來判斷延遲是否隨時間增長（不穩定）。"""
import sys
import numpy as np
for d in sys.argv[1:]:
    a = np.loadtxt(f"{d}/latency_worker_1.csv", delimiter=",", skiprows=1, usecols=(1, 2))
    rel = a[:, 0] - a[:, 0].min()
    out = []
    for s in range(0, 180, 30):
        v = a[(rel >= s) & (rel < s + 30), 1]
        if len(v):
            out.append(f"{s}-{s+30}s p50={np.percentile(v,50):.0f} p99={np.percentile(v,99):.0f}")
    print(d.split('/runs/')[-1], " | ".join(out))
