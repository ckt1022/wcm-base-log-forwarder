#!/usr/bin/env python3
"""Calculate Fluent Bit WASM 30-second latency subwindows near saturation."""

import csv
from pathlib import Path

import numpy as np


HERE = Path(__file__).resolve().parent
RUN_ROOT = HERE / "runs/fbit-wasm"
OUT = HERE.parent / "results/fbit_wasm_fields1k_latency_cgroup_subwindows.csv"
RATES = (5000, 5500, 5600, 5700)
WINDOWS = ((30, 60), (60, 90), (90, 120), (120, 150))

rows = []
for rate in RATES:
    path = RUN_ROOT / f"lat_rate_{rate}_fields1k_latency_cgroup_sweep/latency_worker_1.csv"
    receive, latency = [], []
    with path.open(encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            receive.append(float(row["receive_unix"]))
            latency.append(float(row["latency_ms"]))
    receive = np.asarray(receive, dtype=np.float64)
    latency = np.asarray(latency, dtype=np.float64)
    relative = receive - receive.min()
    for start, end in WINDOWS:
        values = latency[(relative >= start) & (relative < end)]
        rows.append({
            "target_eps": rate,
            "window_start_s": start,
            "window_end_s": end,
            "samples": len(values),
            "p50_ms": float(np.percentile(values, 50)),
            "p99_ms": float(np.percentile(values, 99)),
            "p99_9_ms": float(np.percentile(values, 99.9)),
        })

with OUT.open("w", newline="", encoding="utf-8") as handle:
    writer = csv.DictWriter(handle, fieldnames=rows[0])
    writer.writeheader()
    writer.writerows(rows)
print(f"wrote {OUT} ({len(rows)} rows)")
