#!/usr/bin/env python3
"""Summarize the Vector 1024 B multifield cgroup latency sweep."""

import csv
import re
from pathlib import Path


HERE = Path(__file__).resolve().parent
RUN_ROOT = HERE / "runs/vector"
RESULTS = HERE.parent / "results"
OUT_CSV = RESULTS / "vector_fields1k_latency_cgroup_sweep.csv"
OUT_MD = RESULTS / "vector_fields1k_latency_cgroup_sweep.md"
RATES = (2500, 5000, 7500, 10000, 12500, 15000, 15250, 15500)


def latency_value(text, label):
    match = re.search(rf"\b{re.escape(label)}\s*=\s*([0-9.]+)\s*ms", text)
    if not match:
        raise ValueError(f"missing {label}")
    return float(match.group(1))


rows = []
for rate in RATES:
    variant = "fields1k_latency_cgroup_warmcheck" if rate == 2500 else "fields1k_latency_cgroup_sweep"
    run_dir = RUN_ROOT / f"lat_rate_{rate}_{variant}"
    latency = (run_dir / "latency_stats_30_150.txt").read_text(encoding="utf-8")
    resource = (run_dir / "resource_stats_30_150.txt").read_text(encoding="utf-8")
    verdict = (run_dir / "verdict.txt").read_text(encoding="utf-8").strip()
    rv = dict(re.findall(r"(cpu_mean|cpu_max|mem_mean|mem_max)=([0-9.]+)", resource))
    count = re.search(r"窗口內\s+(\d+)\s+筆", latency)
    rows.append({
        "target_eps": rate,
        "status": "PASS" if verdict.startswith("PASS") else "FAIL",
        "latency_samples": int(count.group(1)) if count else 0,
        "p50_ms": latency_value(latency, "p50"),
        "p90_ms": latency_value(latency, "p90"),
        "p95_ms": latency_value(latency, "p95"),
        "p99_ms": latency_value(latency, "p99"),
        "p99_9_ms": latency_value(latency, "p99.9"),
        "cpu_mean_pct": float(rv["cpu_mean"]),
        "cpu_max_pct": float(rv["cpu_max"]),
        "mem_mean_mib": float(rv["mem_mean"]),
        "mem_max_mib": float(rv["mem_max"]),
    })

RESULTS.mkdir(parents=True, exist_ok=True)
with OUT_CSV.open("w", newline="", encoding="utf-8") as handle:
    writer = csv.DictWriter(handle, fieldnames=rows[0])
    writer.writeheader()
    writer.writerows(rows)

lines = [
    "# Vector 1024B/36 欄位延遲實驗（cgroup 口徑）",
    "",
    "Vector 0.49.0 (`sha256:30141ab1ce5a7f84ef63580863e4ec561291e7c43dd0d7252985c9f2bb87aa5e`)。",
    "1 CPU、512 MiB、TCP 單連線、1 MiB/1 秒 batching、request concurrency 1、retry 0；",
    "每階段 180 秒，統計窗口 30–150 秒。CPU/MEM 為容器 cgroup 總 CPU 與 `memory.current`。",
    "2.5K 採 warm-cache 複測值；首輪 cold-cache MEM 82.51/85.23 MiB。",
    "",
    "| EPS | 狀態 | p50 | p90 | p95 | p99 | p99.9 | CPU avg/max | MEM avg/max MiB |",
    "|---:|:---:|---:|---:|---:|---:|---:|---:|---:|",
]
for row in rows:
    lines.append(
        f"| {row['target_eps']:,} | {row['status']} | {row['p50_ms']:.3f} | "
        f"{row['p90_ms']:.3f} | {row['p95_ms']:.3f} | {row['p99_ms']:.3f} | "
        f"{row['p99_9_ms']:.3f} | {row['cpu_mean_pct']:.2f}% / {row['cpu_max_pct']:.2f}% | "
        f"{row['mem_mean_mib']:.2f} / {row['mem_max_mib']:.2f} |"
    )
lines.extend([
    "",
    "## 結論",
    "",
    "- 15.25K 是本輪最高穩定點；15.5K 因單核 CPU 飽和及 generator backpressure 明確 FAIL。",
    "- 15.5K 的 generator 平均僅 14,705 EPS、send-lag p99 9.09 秒，延遲 p99 8.15 秒。",
    "- 2.5K–15.25K 的 warm-cache cgroup MEM 約 33–45 MiB；15.5K 過載時為 53–56 MiB，沒有 OOM。",
    "- 15K CPU 平均已達 96.40%，較合理的長期操作點是 12.5K（CPU 平均 80.99%）。",
])
OUT_MD.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote {OUT_CSV} ({len(rows)} rows)")
print(f"wrote {OUT_MD}")
