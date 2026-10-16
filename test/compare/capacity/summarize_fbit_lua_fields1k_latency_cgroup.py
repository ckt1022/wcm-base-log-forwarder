#!/usr/bin/env python3
"""Summarize Fluent Bit Lua 1024 B multifield cgroup latency results."""

import csv
import re
from pathlib import Path


HERE = Path(__file__).resolve().parent
RUN_ROOT = HERE / "runs/fbit-lua"
RESULTS = HERE.parent / "results"
OUT_CSV = RESULTS / "fbit_lua_fields1k_latency_cgroup_sweep.csv"
OUT_MD = RESULTS / "fbit_lua_fields1k_latency_cgroup_sweep.md"
RATES = (2500, 5000, 7500, 10000, 12500, 15000, 17500, 20000, 22500,
         25000, 30000, 35000, 40000, 45000, 50000, 55000, 60000, 65000, 70000)


def latency_value(text, label):
    match = re.search(rf"\b{re.escape(label)}\s*=\s*([0-9.]+)\s*ms", text)
    if not match:
        raise ValueError(f"missing {label}")
    return float(match.group(1))


rows = []
for rate in RATES:
    run_dir = RUN_ROOT / f"lat_rate_{rate}_fields1k_latency_cgroup_sweep"
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
    "# Fluent Bit Lua 1024B/36 欄位延遲實驗（cgroup 口徑）",
    "",
    "Fluent Bit 5.0.9 (`sha256:bbce9a7b36b0ddd4fcb01a5320815808d1b8b5e1e100725ac95c2b6714d64964`)。",
    "1 CPU、512 MiB、TCP 單連線、1 秒 flush、單一 output worker；每階段 180 秒，統計窗口 30–150 秒。",
    "Fluent Bit 使用原生約 2 MiB soft chunk，沒有可精確對齊 1 MiB 的 output batch 參數。",
    "CPU/MEM 為容器 cgroup 總 CPU 與 `memory.current`。2.5K 為 cold-cache 首輪；",
    "warm-check 因工具執行額度限制未能補跑，故低流量記憶體基線應以 5K 以上判讀。",
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
    "- 65K 是本輪最高正式 PASS；70K 因 send-lag p99 50 ms 超過 10 ms 規則而 FAIL。",
    "- 70K 平均輸入仍達 69,999 EPS，延遲 p99 約 1.01 秒，尚未形成持續秒級排隊；",
    "  但 CPU 平均 98.66%，已沒有突發流量餘裕。",
    "- 60K CPU 平均 84.70%、65K 為 91.16%；較合理的長期操作範圍是 55K–60K。",
    "- 5K–65K warm-cache cgroup MEM 約 9–40 MiB；70K 最高 44 MiB，沒有 OOM。",
    "- 全部穩態點延遲約受 1 秒 flush 主導，p50 約 0.52–0.61 秒、p99 約 1.01 秒。",
])
OUT_MD.write_text("\n".join(lines) + "\n", encoding="utf-8")
print(f"wrote {OUT_CSV} ({len(rows)} rows)")
print(f"wrote {OUT_MD}")
