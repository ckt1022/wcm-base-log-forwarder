#!/usr/bin/env python3
"""
analyze.py — 讀取 run.sh 產出的 CSV，印出摘要統計表

Usage:
    python3 tools/bench/analyze.py tools/bench/results/results_<timestamp>.csv
"""

import sys
import csv
import statistics
from collections import defaultdict


def load(path: str):
    rows = []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            rows.append(row)
    return rows


def group_key(row):
    return (row["system"], row["mode"], int(row["rate_target"]))


def summarize(rows):
    groups = defaultdict(list)
    for r in rows:
        groups[group_key(r)].append(r)

    # 表頭
    col = "{:<12} {:<8} {:>10}  {:>10} {:>8}  {:>10} {:>8}  {:>10} {:>8}"
    print(col.format(
        "system", "mode", "rate_tgt",
        "avg_rate", "stdev",
        "rss_kb", "stdev",
        "wall_s", "stdev",
    ))
    print("-" * 100)

    for key in sorted(groups):
        system, mode, rate = key
        data = groups[key]

        rates   = [int(r["avg_rate"])   for r in data if r["avg_rate"]]
        rsses   = [int(r["peak_rss_kb"]) for r in data if r["peak_rss_kb"]]
        walls   = [float(r["wall_clock_s"]) for r in data if r["wall_clock_s"]]

        def fmt(vals):
            if not vals:
                return "N/A", "N/A"
            mean = statistics.mean(vals)
            sd   = statistics.stdev(vals) if len(vals) > 1 else 0
            return f"{mean:.0f}", f"±{sd:.0f}"

        r_mean, r_sd   = fmt(rates)
        rss_mean, rss_sd = fmt(rsses)
        w_mean, w_sd   = fmt(walls)

        print(col.format(
            system, mode, rate,
            r_mean, r_sd,
            rss_mean, rss_sd,
            w_mean, w_sd,
        ))


def throughput_ceiling(rows):
    """找出每個系統在哪個 rate 開始飽和（avg_rate 不再成比例增長）"""
    print("\n── Throughput Ceiling Detection ──")
    groups = defaultdict(dict)
    for r in rows:
        key = (r["system"], r["mode"])
        rate = int(r["rate_target"])
        avg  = int(r["avg_rate"]) if r["avg_rate"] else 0
        if rate not in groups[key]:
            groups[key][rate] = []
        groups[key][rate].append(avg)

    for key, rate_map in sorted(groups.items()):
        system, mode = key
        sorted_rates = sorted(rate_map)
        prev_eff = None
        ceiling = None
        for rate in sorted_rates:
            vals = rate_map[rate]
            mean = statistics.mean(vals) if vals else 0
            efficiency = mean / rate if rate else 0
            if prev_eff is not None and efficiency < prev_eff * 0.85:
                ceiling = rate
                break
            prev_eff = efficiency
        label = f"{ceiling}/s" if ceiling else "not reached"
        print(f"  {system}/{mode}: saturation point ≈ {label}")


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <results.csv>")
        sys.exit(1)

    rows = load(sys.argv[1])
    if not rows:
        print("empty CSV")
        sys.exit(1)

    print(f"\n=== Benchmark Summary ({len(rows)} runs from {sys.argv[1]}) ===\n")
    summarize(rows)
    throughput_ceiling(rows)
    print()


if __name__ == "__main__":
    main()
