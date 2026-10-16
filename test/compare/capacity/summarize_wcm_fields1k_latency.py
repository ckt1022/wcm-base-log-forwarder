#!/usr/bin/env python3
"""Combine WCM 1024 B multifield latency-sweep stage summaries."""

import csv
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent
RUN_ROOT = ROOT / "runs/wcm-b1"
OUT_CSV = ROOT.parent / "results/wcm_fields1k_latency_sweep.csv"
OUT_MD = ROOT.parent / "results/wcm_fields1k_latency_sweep.md"
PATTERN = re.compile(r"lat_rate_(\d+)_fields1k_latency_sweep$")


def value(text, label):
    match = re.search(rf"\b{re.escape(label)}\s*=\s*([0-9.]+)\s*ms", text)
    if not match:
        raise ValueError(f"missing {label}")
    return float(match.group(1))


rows = []
for run_dir in RUN_ROOT.glob("lat_rate_*_fields1k_latency_sweep"):
    match = PATTERN.search(run_dir.name)
    latency_path = run_dir / "latency_stats_30_150.txt"
    resource_path = run_dir / "resource_stats_30_150.txt"
    verdict_path = run_dir / "verdict.txt"
    if not match or not latency_path.exists() or not resource_path.exists():
        continue

    latency = latency_path.read_text(encoding="utf-8")
    resource = resource_path.read_text(encoding="utf-8")
    verdict = verdict_path.read_text(encoding="utf-8").strip() if verdict_path.exists() else ""
    resource_values = dict(re.findall(r"(cpu_mean|cpu_max|mem_mean|mem_max)=([0-9.]+)", resource))
    count_match = re.search(r"\u7a97\u53e3\u5167\s+(\d+)\s+\u7b46", latency)
    rows.append({
        "target_eps": int(match.group(1)),
        "status": "PASS" if verdict.startswith("PASS") else "FAIL",
        "latency_samples": int(count_match.group(1)) if count_match else 0,
        "p50_ms": value(latency, "p50"),
        "p90_ms": value(latency, "p90"),
        "p95_ms": value(latency, "p95"),
        "p99_ms": value(latency, "p99"),
        "p99_9_ms": value(latency, "p99.9"),
        "cpu_mean_pct": float(resource_values["cpu_mean"]),
        "cpu_max_pct": float(resource_values["cpu_max"]),
        "mem_mean_mib": float(resource_values["mem_mean"]),
        "mem_max_mib": float(resource_values["mem_max"]),
    })

rows.sort(key=lambda row: row["target_eps"])
OUT_CSV.parent.mkdir(parents=True, exist_ok=True)
fields = list(rows[0]) if rows else []
with OUT_CSV.open("w", newline="", encoding="utf-8") as handle:
    writer = csv.DictWriter(handle, fieldnames=fields)
    writer.writeheader()
    writer.writerows(rows)

analysis_tail = ""
analysis_marker = "\n## 22.5K–25K 轉折區"
if OUT_MD.exists():
    previous = OUT_MD.read_text(encoding="utf-8")
    if analysis_marker in previous:
        analysis_tail = previous[previous.index(analysis_marker):].rstrip()

lines = [
    "# WCM 1024B/36 欄位延遲階梯實驗",
    "",
    "1 CPU、512 MiB、TCP 單連線、1 秒 flush；每階段 180 秒，統計窗口 30–150 秒。",
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
if analysis_tail:
    lines.extend([analysis_tail])
OUT_MD.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote {OUT_CSV} ({len(rows)} rows)")
print(f"wrote {OUT_MD}")
