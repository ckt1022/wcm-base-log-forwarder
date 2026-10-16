#!/usr/bin/env python3
"""Analyze one deterministic burst run and decide whether WCM recovered."""
import csv
import json
import math
import statistics
import sys
from pathlib import Path


run = Path(sys.argv[1])
rate_window_s = int(sys.argv[2]) if len(sys.argv) > 2 else 5
output_name = sys.argv[3] if len(sys.argv) > 3 else "burst_analysis.json"
if rate_window_s < 1:
    raise SystemExit("rate window must be at least one second")


def read_csv(name):
    with (run / name).open(newline="") as handle:
        return list(csv.DictReader(handle))


phases = read_csv("phases.csv")
phase_by_name = {p["phase"]: p for p in phases}
baseline = phase_by_name["baseline"]
burst = phase_by_name["burst"]
recovery = phase_by_name["recovery"]
burst_start = float(burst["start_unix"])
burst_end = float(burst["end_unix"])
baseline_rate = int(baseline["rate_eps"])
expected_output_eps = baseline_rate * 0.4

sink = []
for row in read_csv("sink_per_sec.csv"):
    sink.append({k: float(v) for k, v in row.items()})
generator = []
for row in read_csv("generator_per_sec.csv"):
    generator.append({
        "unix_s": float(row["unix_s"]), "pending": int(row["pending"]),
        "sent_total": int(row["sent_total"]), "offered_total": int(row["offered_total"]),
    })


def expected_kept(n):
    full, rem = divmod(int(n), 5)
    return full * 2 + int(rem > 1) + int(rem > 2)


def offered_at(ts):
    for p in phases:
        start, end = float(p["start_unix"]), float(p["end_unix"])
        if ts >= end:
            continue
        if ts <= start:
            return int(p["start_count"])
        count = int(p["start_count"]) + int((ts - start) * int(p["rate_eps"]))
        return min(count, int(p["end_count"]))
    return int(phases[-1]["end_count"])


baseline_start = max(float(baseline["start_unix"]), burst_start - 60)
baseline_rows = [r for r in sink if baseline_start <= r["unix_s"] < burst_start and r["lines"] > 0]
baseline_p99 = statistics.median(r["p99_ms"] for r in baseline_rows)
p99_limit = max(1.2 * baseline_p99, baseline_p99 + 20.0)

gen_index = 0
latest_gen = None
consecutive = 0
candidate_start = None
recovered_at = None
max_backlog = 0
post_burst_p99 = []
line_window = []
p99_window = []

for row in sink:
    while gen_index < len(generator) and generator[gen_index]["unix_s"] <= row["unix_s"] + 0.5:
        latest_gen = generator[gen_index]
        gen_index += 1
    backlog = max(0, expected_kept(offered_at(row["unix_s"])) - int(row["cumulative"]))
    max_backlog = max(max_backlog, backlog)
    if row["unix_s"] < burst_end:
        continue
    if row["lines"] > 0:
        post_burst_p99.append(row["p99_ms"])
    line_window.append(row["lines"])
    if row["lines"] > 0:
        p99_window.append(row["p99_ms"])
    if len(line_window) > rate_window_s:
        line_window.pop(0)
    if len(p99_window) > 5:
        p99_window.pop(0)
    rolling_rate = statistics.fmean(line_window) if len(line_window) == rate_window_s else 0
    rate_ok = abs(rolling_rate - expected_output_eps) <= 0.05 * expected_output_eps
    # Per-second p99 has periodic batch-boundary spikes even during the baseline.
    # A five-second rolling median preserves sustained degradation detection without
    # resetting recovery on one normal batch-boundary second.
    latency_ok = len(p99_window) == 5 and statistics.median(p99_window) <= p99_limit
    backlog_ok = backlog <= expected_output_eps
    source_ok = latest_gen is not None and latest_gen["pending"] <= baseline_rate
    if rate_ok and latency_ok and backlog_ok and source_ok:
        if consecutive == 0:
            candidate_start = row["unix_s"]
        consecutive += 1
        if consecutive >= 30:
            recovered_at = candidate_start
            break
    else:
        consecutive = 0
        candidate_start = None

validation = json.loads((run / "validation_summary.json").read_text())
oom = (run / "oom.txt").read_text().strip()
delivery_ok = all(validation[k] == 0 for k in (
    "missing", "duplicate", "corrupt", "unexpected", "order_inversions", "bad_output_json"
)) and validation["received"] == validation["expected"]

stats_epoch = float((run / "stats_epoch.txt").read_text())
stats = []
for row in read_csv("stats.csv"):
    absolute = stats_epoch + float(row["elapsed_s"])
    if absolute <= float(recovery["end_unix"]) + 1:
        stats.append((float(row["cpu_pct"]), float(row["rss_mb"])))

result = {
    "status": "RECOVERED" if recovered_at is not None and delivery_ok and oom == "false" else "NOT_RECOVERED",
    "burst_rate_eps": int(burst["rate_eps"]),
    "burst_duration_s": int(burst["duration_s"]),
    "baseline_rate_eps": baseline_rate,
    "rate_window_s": rate_window_s,
    "baseline_p99_ms": round(baseline_p99, 3),
    "p99_recovery_limit_ms": round(p99_limit, 3),
    "recovery_time_s": None if recovered_at is None else round(max(0, recovered_at - burst_end), 3),
    "confirmation_time_s": None if recovered_at is None else round(max(0, recovered_at + 29 - burst_end), 3),
    "max_output_backlog": max_backlog,
    "max_source_pending": max((r["pending"] for r in generator), default=0),
    "max_post_burst_p99_ms": round(max(post_burst_p99), 3) if post_burst_p99 else None,
    "final_expected": validation["expected"],
    "final_received": validation["received"],
    "missing": validation["missing"],
    "duplicate": validation["duplicate"],
    "corrupt": validation["corrupt"],
    "cpu_mean_pct": round(statistics.fmean(v[0] for v in stats), 3),
    "cpu_max_pct": round(max(v[0] for v in stats), 3),
    "mem_mean_mib": round(statistics.fmean(v[1] for v in stats), 3),
    "mem_max_mib": round(max(v[1] for v in stats), 3),
    "oom": oom,
}
(run / output_name).write_text(json.dumps(result, indent=2) + "\n")
print(
    f"{result['status']} burst={result['burst_rate_eps']}x{result['burst_duration_s']}s "
    f"recovery={result['recovery_time_s']}s confirm={result['confirmation_time_s']}s "
    f"max_backlog={result['max_output_backlog']} max_source_pending={result['max_source_pending']} "
    f"p99_base={result['baseline_p99_ms']}ms p99_max={result['max_post_burst_p99_ms']}ms "
    f"received={result['final_received']}/{result['final_expected']} oom={oom}"
)
sys.exit(0 if result["status"] == "RECOVERED" else 1)
