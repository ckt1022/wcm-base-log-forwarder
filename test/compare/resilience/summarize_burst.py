#!/usr/bin/env python3
"""Combine WCM burst/recovery run analyses."""
import csv
import json
import statistics
import sys
from pathlib import Path


root = Path(sys.argv[1])
product = sys.argv[2] if len(sys.argv) > 2 else "WCM"
analysis_name = sys.argv[3] if len(sys.argv) > 3 else "burst_analysis.json"
output_suffix = sys.argv[4] if len(sys.argv) > 4 else ""
rows = []
for run in sorted(root.glob("B*_rep*")):
    if not (run / analysis_name).exists():
        continue
    scenario, repeat = run.name.split("_rep")
    item = json.loads((run / analysis_name).read_text())
    item = {"scenario": scenario, "repeat": int(repeat), **item}
    rows.append(item)

out_csv = root / f"summary{output_suffix}.csv"
if not rows:
    raise SystemExit(f"no completed burst analyses under {root}")
with out_csv.open("w", newline="") as handle:
    writer = csv.DictWriter(handle, fieldnames=rows[0].keys())
    writer.writeheader()
    writer.writerows(rows)

method = rows[0].get("evaluation_method", "rolling_confirmation")
if method == "forward_interval":
    recovery_description = [
        "Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,",
        "median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending",
        "each no more than one baseline second.",
    ]
else:
    recovery_description = [
        "Recovery requires 30 consecutive seconds",
        f"with {rows[0].get('rate_window_s', 5)}-second output rate within ±5% of baseline, p99 returned to its baseline threshold,",
        "output backlog no more than one baseline second, and source pending no more than one baseline second.",
    ]

lines = [
    f"# {product} deterministic burst/recovery experiment", "",
    "1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.",
    f"Baseline is {rows[0]['baseline_rate_eps']:,} input EPS; burst is {rows[0]['burst_rate_eps']:,} input EPS.",
    *recovery_description,
    "", "| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |",
    "|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|",
]
for r in rows:
    lines.append(
        f"| {r['scenario']} | {r['repeat']} | {r['burst_duration_s']}s | {r['status']} | "
        f"{r['recovery_time_s']} | {r['confirmation_time_s']} | {r['max_output_backlog']:,} | "
        f"{r['max_source_pending']:,} | {r['baseline_p99_ms']:.1f} / {r['max_post_burst_p99_ms']:.1f} | "
        f"{r['missing']} | {r['cpu_mean_pct']:.2f}% / {r['cpu_max_pct']:.2f}% | "
        f"{r['mem_mean_mib']:.2f} / {r['mem_max_mib']:.2f} |"
    )
lines += ["", "## Aggregate", ""]
for scenario in sorted({r["scenario"] for r in rows}):
    group = [r for r in rows if r["scenario"] == scenario]
    recovered = [r for r in group if r["status"] == "RECOVERED"]
    times = [r["recovery_time_s"] for r in recovered]
    time_text = "n/a" if not times else f"{statistics.fmean(times):.2f}s (range {min(times):.2f}–{max(times):.2f}s)"
    lines.append(f"- {scenario}: {len(recovered)}/{len(group)} recovered; mean recovery {time_text}.")
(root / f"summary{output_suffix}.md").write_text("\n".join(lines) + "\n")
print(out_csv)
print(root / f"summary{output_suffix}.md")
