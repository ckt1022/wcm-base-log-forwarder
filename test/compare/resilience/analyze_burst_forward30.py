#!/usr/bin/env python3
"""Analyze recovery using the first qualifying 30-second forward interval."""
import bisect
import csv
import json
import statistics
import sys
from pathlib import Path


run = Path(sys.argv[1])
output_name = sys.argv[2] if len(sys.argv) > 2 else "burst_analysis.json"
INTERVAL_SECS = 30


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
recovery_end = float(recovery["end_unix"])
baseline_rate = int(baseline["rate_eps"])
expected_output_eps = baseline_rate * 0.4

sink = [{k: float(v) for k, v in row.items()} for row in read_csv("sink_per_sec.csv")]
generator = [{
    "unix_s": float(row["unix_s"]),
    "pending": int(row["pending"]),
} for row in read_csv("generator_per_sec.csv")]
generator_times = [row["unix_s"] for row in generator]


def expected_kept(count):
    full, rem = divmod(int(count), 5)
    return full * 2 + int(rem > 1) + int(rem > 2)


def offered_at(timestamp):
    for phase in phases:
        start, end = float(phase["start_unix"]), float(phase["end_unix"])
        if timestamp >= end:
            continue
        if timestamp <= start:
            return int(phase["start_count"])
        count = int(phase["start_count"]) + int((timestamp - start) * int(phase["rate_eps"]))
        return min(count, int(phase["end_count"]))
    return int(phases[-1]["end_count"])


def latest_generator(timestamp):
    index = bisect.bisect_right(generator_times, timestamp + 0.5) - 1
    return generator[index] if index >= 0 else None


def backlog_at(row):
    return max(0, expected_kept(offered_at(row["unix_s"])) - int(row["cumulative"]))


baseline_start = max(float(baseline["start_unix"]), burst_start - 60)
baseline_rows = [row for row in sink if baseline_start <= row["unix_s"] < burst_start and row["lines"] > 0]
baseline_p99 = statistics.median(row["p99_ms"] for row in baseline_rows)
p99_limit = max(1.2 * baseline_p99, baseline_p99 + 20.0)

max_backlog = max((backlog_at(row) for row in sink), default=0)
post_burst_p99 = [row["p99_ms"] for row in sink if row["unix_s"] >= burst_end and row["lines"] > 0]
recovered_at = None
interval_output_eps = None
interval_p99_median = None

for index, first_row in enumerate(sink):
    candidate = first_row["unix_s"]
    if candidate < burst_end:
        continue
    if candidate + INTERVAL_SECS > recovery_end:
        break
    window = sink[index:index + INTERVAL_SECS]
    if len(window) < INTERVAL_SECS:
        break
    if window[-1]["unix_s"] - window[0]["unix_s"] > INTERVAL_SECS + 0.25:
        continue
    output_eps = sum(row["lines"] for row in window) / INTERVAL_SECS
    p99_values = [row["p99_ms"] for row in window if row["lines"] > 0]
    if not p99_values:
        continue
    p99_median = statistics.median(p99_values)
    end_row = window[-1]
    end_generator = latest_generator(candidate + INTERVAL_SECS)
    rate_ok = abs(output_eps - expected_output_eps) <= 0.05 * expected_output_eps
    latency_ok = p99_median <= p99_limit
    backlog_ok = backlog_at(end_row) <= expected_output_eps
    source_ok = end_generator is not None and end_generator["pending"] <= baseline_rate
    if rate_ok and latency_ok and backlog_ok and source_ok:
        recovered_at = candidate
        interval_output_eps = output_eps
        interval_p99_median = p99_median
        break

validation = json.loads((run / "validation_summary.json").read_text())
oom = (run / "oom.txt").read_text().strip()
delivery_ok = all(validation[key] == 0 for key in (
    "missing", "duplicate", "corrupt", "unexpected", "order_inversions", "bad_output_json"
)) and validation["received"] == validation["expected"]

stats_epoch = float((run / "stats_epoch.txt").read_text())
stats = []
for row in read_csv("stats.csv"):
    absolute = stats_epoch + float(row["elapsed_s"])
    if absolute <= recovery_end + 1:
        stats.append((float(row["cpu_pct"]), float(row["rss_mb"])))

recovery_time = None if recovered_at is None else max(0, recovered_at - burst_end)
result = {
    "status": "RECOVERED" if recovered_at is not None and delivery_ok and oom == "false" else "NOT_RECOVERED",
    "evaluation_method": "forward_interval",
    "rate_window_s": INTERVAL_SECS,
    "burst_rate_eps": int(burst["rate_eps"]),
    "burst_duration_s": int(burst["duration_s"]),
    "baseline_rate_eps": baseline_rate,
    "baseline_p99_ms": round(baseline_p99, 3),
    "p99_recovery_limit_ms": round(p99_limit, 3),
    "recovery_time_s": None if recovery_time is None else round(recovery_time, 3),
    "confirmation_time_s": None if recovery_time is None else round(recovery_time + INTERVAL_SECS, 3),
    "recovery_interval_output_eps": None if interval_output_eps is None else round(interval_output_eps, 3),
    "recovery_interval_p99_median_ms": None if interval_p99_median is None else round(interval_p99_median, 3),
    "max_output_backlog": max_backlog,
    "max_source_pending": max((row["pending"] for row in generator), default=0),
    "max_post_burst_p99_ms": round(max(post_burst_p99), 3) if post_burst_p99 else None,
    "final_expected": validation["expected"],
    "final_received": validation["received"],
    "missing": validation["missing"],
    "duplicate": validation["duplicate"],
    "corrupt": validation["corrupt"],
    "cpu_mean_pct": round(statistics.fmean(value[0] for value in stats), 3),
    "cpu_max_pct": round(max(value[0] for value in stats), 3),
    "mem_mean_mib": round(statistics.fmean(value[1] for value in stats), 3),
    "mem_max_mib": round(max(value[1] for value in stats), 3),
    "oom": oom,
}
(run / output_name).write_text(json.dumps(result, indent=2) + "\n")
print(
    f"{result['status']} burst={result['burst_rate_eps']}x{result['burst_duration_s']}s "
    f"recovery={result['recovery_time_s']}s confirm={result['confirmation_time_s']}s "
    f"interval_eps={result['recovery_interval_output_eps']} interval_p99={result['recovery_interval_p99_median_ms']}ms "
    f"max_backlog={result['max_output_backlog']} max_source_pending={result['max_source_pending']} "
    f"p99_base={result['baseline_p99_ms']}ms p99_max={result['max_post_burst_p99_ms']}ms "
    f"received={result['final_received']}/{result['final_expected']} oom={oom}"
)
sys.exit(0 if result["status"] == "RECOVERED" else 1)
