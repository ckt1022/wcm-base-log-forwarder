#!/usr/bin/env python3
"""Summarize a constant-rate stability run after warm-up."""
import csv
import json
import statistics
import sys
from pathlib import Path


run = Path(sys.argv[1])


def rows(name):
    with (run / name).open(newline="") as handle:
        return list(csv.DictReader(handle))


phases = {row["phase"]: row for row in rows("phases.csv")}
steady = phases["steady"]
start = float(steady["start_unix"])
end = float(steady["end_unix"])
duration = float(steady["duration_s"])
rate = int(steady["rate_eps"])
expected_output_eps = rate * 0.4

sink = [{key: float(value) for key, value in row.items()} for row in rows("sink_per_sec.csv")]
steady_sink = [row for row in sink if start <= row["unix_s"] < end and row["lines"] > 0]


def median_window(column, lower, upper):
    values = [row[column] for row in steady_sink if lower <= row["unix_s"] < upper]
    return statistics.median(values) if values else 0.0


output_eps = sum(row["lines"] for row in steady_sink) / duration
first_p99 = median_window("p99_ms", start, start + 300)
last_p99 = median_window("p99_ms", end - 300, end)
first_p999 = median_window("p99_9_ms", start, start + 300)
last_p999 = median_window("p99_9_ms", end - 300, end)

stats_epoch = float((run / "stats_epoch.txt").read_text())
stats = []
for row in rows("stats.csv"):
    absolute = stats_epoch + float(row["elapsed_s"])
    if start <= absolute < end:
        stats.append((absolute, float(row["cpu_pct"]), float(row["rss_mb"])))

first_rss = [rss for timestamp, _, rss in stats if timestamp < start + 300]
last_rss = [rss for timestamp, _, rss in stats if timestamp >= end - 300]
first_rss_mean = statistics.fmean(first_rss)
last_rss_mean = statistics.fmean(last_rss)
x_mean = statistics.fmean(timestamp for timestamp, _, _ in stats)
y_mean = statistics.fmean(rss for _, _, rss in stats)
denominator = sum((timestamp - x_mean) ** 2 for timestamp, _, _ in stats)
slope_per_hour = (sum((timestamp - x_mean) * (rss - y_mean) for timestamp, _, rss in stats) / denominator * 3600) if denominator else 0.0

validation = json.loads((run / "validation_summary.json").read_text())
oom = (run / "oom.txt").read_text().strip() == "true"
delivery_ok = validation["received"] == validation["expected"] and all(
    validation[key] == 0 for key in (
        "missing", "duplicate", "corrupt", "unexpected", "order_inversions", "bad_output_json"
    )
)
throughput_ok = abs(output_eps - expected_output_eps) <= expected_output_eps * 0.05

result = {
    "status": "COMPLETED" if delivery_ok and not oom and throughput_ok else "FAILED",
    "duration_s": int(duration),
    "input_rate_eps": rate,
    "expected_output_eps": round(expected_output_eps, 3),
    "actual_output_eps": round(output_eps, 3),
    "output_deviation_pct": round((output_eps / expected_output_eps - 1) * 100, 3),
    "p50_median_ms": round(statistics.median(row["p50_ms"] for row in steady_sink), 3),
    "p99_median_ms": round(statistics.median(row["p99_ms"] for row in steady_sink), 3),
    "p99_max_ms": round(max(row["p99_ms"] for row in steady_sink), 3),
    "p999_median_ms": round(statistics.median(row["p99_9_ms"] for row in steady_sink), 3),
    "p999_max_ms": round(max(row["p99_9_ms"] for row in steady_sink), 3),
    "first_5m_p99_ms": round(first_p99, 3),
    "last_5m_p99_ms": round(last_p99, 3),
    "p99_drift_ms": round(last_p99 - first_p99, 3),
    "first_5m_p999_ms": round(first_p999, 3),
    "last_5m_p999_ms": round(last_p999, 3),
    "cpu_mean_pct": round(statistics.fmean(cpu for _, cpu, _ in stats), 3),
    "cpu_max_pct": round(max(cpu for _, cpu, _ in stats), 3),
    "mem_mean_mib": round(y_mean, 3),
    "mem_max_mib": round(max(rss for _, _, rss in stats), 3),
    "first_5m_mem_mib": round(first_rss_mean, 3),
    "last_5m_mem_mib": round(last_rss_mean, 3),
    "mem_delta_mib": round(last_rss_mean - first_rss_mean, 3),
    "mem_slope_mib_per_hour": round(slope_per_hour, 3),
    "expected": validation["expected"],
    "received": validation["received"],
    "missing": validation["missing"],
    "duplicate": validation["duplicate"],
    "corrupt": validation["corrupt"],
    "reorder": validation["order_inversions"],
    "oom": oom,
}
(run / "stability_analysis.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, ensure_ascii=False))
