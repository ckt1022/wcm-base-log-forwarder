#!/usr/bin/env python3
"""Summarize the WCM latency rerun measured with container cgroups."""

import csv
import re
from pathlib import Path


HERE = Path(__file__).resolve().parent
RUN_ROOT = HERE / "runs/wcm-b1"
RESULTS = HERE.parent / "results"
OUT_CSV = RESULTS / "wcm_fields1k_latency_cgroup_rerun.csv"
OUT_MD = RESULTS / "wcm_fields1k_latency_cgroup_rerun.md"
RATES = (2500, 5000, 7500, 10000, 12500, 15000, 17500, 20000,
         21000, 21500, 22000, 22250, 22500)


def latency_value(text, label):
    match = re.search(rf"\b{re.escape(label)}\s*=\s*([0-9.]+)\s*ms", text)
    if not match:
        raise ValueError(f"missing {label}")
    return float(match.group(1))


rows = []
for rate in RATES:
    variant = "fields1k_latency_cgroup_warmcheck" if rate == 2500 else "fields1k_latency_cgroup_rerun"
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
    "# WCM 1024B/36 欄位延遲重跑（cgroup 口徑）",
    "",
    "1 CPU、512 MiB、TCP 單連線、1 MiB/1 秒 batching；每階段 180 秒，統計窗口 30–150 秒。",
    "CPU 與 MEM 均為 DUT 容器 cgroup 的總 CPU 與 `memory.current`，與 Fluentd 重跑口徑相同。",
    "2.5K 採 warm-cache 複測值；首輪 cold-cache MEM 45.77/49.88 MiB，延遲一致。",
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
    "- 22.25K 是本輪最高穩定點；四個 30 秒子窗口 p99 約 116–134 ms。",
    "- 22.5K 明確 FAIL：CPU 平均 99.92%，p99 由首窗口 4.28 秒增至末窗口 12.22 秒，",
    "  cgroup MEM 因 backlog 增至平均 135.57、最高 160.03 MiB。",
    "- 21.5K 在 90–120 秒有一次暫時尖峰（p99 339.92 ms），下一窗口恢復 127.09 ms；",
    "  不是持續排隊，但顯示接近容量時 CPU 排程餘裕已很小。",
    "- 2.5K–22.25K 穩態的 warm-cache cgroup MEM 大致為 23–32 MiB。",
    "- 生產操作點仍建議 20K–21K；22K 以上平均 CPU 已達 93.7–95.4%，不宜保證尖峰餘裕。",
])
OUT_MD.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote {OUT_CSV} ({len(rows)} rows)")
print(f"wrote {OUT_MD}")
