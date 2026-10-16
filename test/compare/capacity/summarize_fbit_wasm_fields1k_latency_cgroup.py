#!/usr/bin/env python3
"""Summarize Fluent Bit WASM 1024 B multifield cgroup latency results."""

import csv
import re
from pathlib import Path


HERE = Path(__file__).resolve().parent
RUN_ROOT = HERE / "runs/fbit-wasm"
RESULTS = HERE.parent / "results"
OUT_CSV = RESULTS / "fbit_wasm_fields1k_latency_cgroup_sweep.csv"
OUT_MD = RESULTS / "fbit_wasm_fields1k_latency_cgroup_sweep.md"
RATES = (2500, 5000, 5500, 5600, 5700)


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
    "# Fluent Bit WASM 1024B/36 欄位延遲實驗（cgroup 口徑）",
    "",
    "Fluent Bit 5.0.9 (`sha256:bbce9a7b36b0ddd4fcb01a5320815808d1b8b5e1e100725ac95c2b6714d64964`)。",
    "1 CPU、512 MiB、TCP 單連線、1 秒 flush、單一 output worker；每階段 180 秒，統計窗口 30–150 秒。",
    "Fluent Bit 使用原生約 2 MiB soft chunk，沒有可精確對齊 1 MiB 的 output batch 參數。",
    "CPU/MEM 為容器 cgroup 總 CPU 與 `memory.current`。2.5K 採 warm-cache 複測值；",
    "首輪 cold-cache MEM 301.73/302.64 MiB。",
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
    "- 5.5K 是本輪最高穩定點；5.6K 因單核 CPU 飽和與 generator backpressure FAIL。",
    "- 5.6K send-lag p99 1.31 秒、延遲 p99 2.29 秒；5.7K 分別為 2.92 秒與 3.54 秒。",
    "- 2.5K–5.5K 的 warm-cache cgroup MEM 約 281–285 MiB，主因是設定的 256 MiB WASM heap。",
    "- 5.5K CPU 平均已達 99.13%，較合理的長期操作點是 5K（CPU 平均 91.79%），但餘裕仍不大。",
    "- 2.5K–5.5K 的延遲均約受 1 秒 flush 主導，未出現其他產品的明顯下降曲線。",
])
OUT_MD.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote {OUT_CSV} ({len(rows)} rows)")
print(f"wrote {OUT_MD}")
