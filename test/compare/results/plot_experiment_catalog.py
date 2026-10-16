#!/usr/bin/env python3
"""Generate dependency-free SVG figures used by EXPERIMENTS.md."""

import csv
import json
import math
from collections import defaultdict
from html import escape
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
COMPARE = ROOT / "test/compare"
OUT = COMPARE / "results/experiment_catalog_charts"
OUT.mkdir(parents=True, exist_ok=True)

COLORS = {
    "WCM": "#2563eb",
    "Vector": "#16a34a",
    "Fluentd": "#ea580c",
    "Fluent Bit Lua": "#0891b2",
    "Fluent Bit WASM": "#7c3aed",
}


def svg_start(width, height, title, subtitle):
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="#fff"/>',
        '<style>text{font-family:"Noto Sans CJK TC","Noto Sans",Arial,sans-serif;fill:#172033}'
        '.title{font-size:27px;font-weight:700}.subtitle{font-size:15px;fill:#526174}'
        '.axis{font-size:14px}.label{font-size:15px;font-weight:600}.note{font-size:13px;fill:#526174}'
        '.grid{stroke:#dfe5ec;stroke-width:1}.axisline{stroke:#526174;stroke-width:1.5}</style>',
        f'<text x="70" y="44" class="title">{escape(title)}</text>',
        f'<text x="70" y="70" class="subtitle">{escape(subtitle)}</text>',
    ]


def save(parts, path):
    parts.append("</svg>")
    path.write_text("\n".join(parts) + "\n", encoding="utf-8")


def axes(parts, x, y, width, height, x_ticks, y_ticks, x_scale, y_scale, x_label, y_label):
    for value, label in y_ticks:
        yy = y_scale(value)
        parts.append(f'<line x1="{x}" y1="{yy:.1f}" x2="{x+width}" y2="{yy:.1f}" class="grid"/>')
        parts.append(f'<text x="{x-12}" y="{yy+5:.1f}" text-anchor="end" class="axis">{escape(label)}</text>')
    for value, label in x_ticks:
        xx = x_scale(value)
        parts.append(f'<line x1="{xx:.1f}" y1="{y}" x2="{xx:.1f}" y2="{y+height}" class="grid"/>')
        parts.append(f'<text x="{xx:.1f}" y="{y+height+25}" text-anchor="middle" class="axis">{escape(label)}</text>')
    parts.append(f'<line x1="{x}" y1="{y+height}" x2="{x+width}" y2="{y+height}" class="axisline"/>')
    parts.append(f'<line x1="{x}" y1="{y}" x2="{x}" y2="{y+height}" class="axisline"/>')
    parts.append(f'<text x="{x+width/2}" y="{y+height+54}" text-anchor="middle" class="axis">{escape(x_label)}</text>')
    parts.append(f'<text x="{x-68}" y="{y+height/2}" text-anchor="middle" transform="rotate(-90 {x-68} {y+height/2})" class="axis">{escape(y_label)}</text>')


def plot_legacy_capacity_latency():
    wanted_capacity = {
        "fbit-lua_capacity_160b_five_cycle",
        "fbit-wasm_capacity_160b_five_cycle",
        "fluentd_capacity_160b_five_cycle",
        "vector_capacity_160b_five_cycle",
        "wcm_capacity_160b_five_cycle",
    }
    wanted_latency = {
        "fbit-lua_relative80_236800",
        "fbit-wasm_relative80_19200",
        "fluentd_relative80_140800",
        "vector_relative80_49600",
        "wcm_relative80_105600",
    }
    with (ROOT / "實驗結果.csv").open(newline="", encoding="utf-8-sig") as handle:
        rows = list(csv.DictReader(handle))
    cap = {r["product"].replace(" Ruby", "").replace(" VRL", "").replace(" Rust WASM", ""): r for r in rows if r["run_id"] in wanted_capacity}
    lat = {r["product"].replace(" Ruby", "").replace(" VRL", "").replace(" Rust WASM", ""): r for r in rows if r["run_id"] in wanted_latency}
    products = ["Fluent Bit Lua", "Fluentd", "WCM", "Vector", "Fluent Bit WASM"]
    parts = svg_start(1450, 870, "Historical 160-byte capacity and relative-load latency", "One CPU / 512 MiB / one TCP connection; latency is measured at each product's own 80% load")
    panels = [
        (115, "A. Confirmed sustainable input throughput", cap, "input_mb_s", 50, "Input throughput (MB/s)"),
        (490, "B. p99 latency at each product's own 80% load", lat, "latency_p99_ms", 1200, "p99 latency (ms)"),
    ]
    for top, title, data, field, ymax, ylabel in panels:
        left, width, height = 120, 1260, 245
        parts.append(f'<text x="{left}" y="{top-18}" class="label">{escape(title)}</text>')
        sy = lambda value, t=top, m=ymax: t + height - value / m * height
        for tick in range(0, int(ymax) + 1, int(ymax / 5)):
            yy = sy(tick)
            parts.append(f'<line x1="{left}" y1="{yy:.1f}" x2="{left+width}" y2="{yy:.1f}" class="grid"/>')
            parts.append(f'<text x="{left-12}" y="{yy+5:.1f}" text-anchor="end" class="axis">{tick}</text>')
        slot = width / len(products)
        for i, product in enumerate(products):
            value = float(data[product][field])
            bx, bw = left + i * slot + 52, slot - 104
            by = sy(value)
            parts.append(f'<rect x="{bx:.1f}" y="{by:.1f}" width="{bw:.1f}" height="{top+height-by:.1f}" fill="{COLORS[product]}"/>')
            parts.append(f'<text x="{bx+bw/2:.1f}" y="{by-8:.1f}" text-anchor="middle" class="label">{value:.1f}</text>')
            parts.append(f'<text x="{bx+bw/2:.1f}" y="{top+height+25}" text-anchor="middle" class="axis">{escape(product)}</text>')
        parts.append(f'<text x="32" y="{top+height/2}" text-anchor="middle" transform="rotate(-90 32 {top+height/2})" class="axis">{escape(ylabel)}</text>')
    parts.append('<text x="120" y="850" class="note">Historical runs lack complete image digests and container-scope resource samples; retain as supporting evidence, not the primary current-configuration comparison.</text>')
    save(parts, OUT / "01_legacy_160b_capacity_latency.svg")


def plot_correctness():
    rows = []
    for path in [
        COMPARE / "correctness/runs/wcm_correctness_formal_20260804/summary.csv",
        COMPARE / "correctness/runs/wcm_correctness_edge_20260804/summary.csv",
    ]:
        with path.open(newline="", encoding="utf-8") as handle:
            rows.extend(csv.DictReader(handle))
    agg = defaultdict(lambda: {"expected": 0, "received": 0, "missing": 0, "pass": 0, "runs": 0})
    for row in rows:
        item = agg[row["scenario"]]
        for key in ("expected", "received", "missing"):
            item[key] += int(row[key if key != "expected" else "expected_output"])
        item["runs"] += 1
        item["pass"] += row["status"] == "PASS"
    scenarios = ["C1", "C2", "C3", "C4"]
    parts = svg_start(1400, 720, "WCM correctness results", "Three repetitions per scenario; exact expected/received counts and validation verdicts")
    left, top, width, height = 125, 125, 1200, 440
    ymax = 1_200_000
    sy = lambda value: top + height - value / ymax * height
    for tick in (0, 300000, 600000, 900000, 1200000):
        yy = sy(tick)
        parts.append(f'<line x1="{left}" y1="{yy:.1f}" x2="{left+width}" y2="{yy:.1f}" class="grid"/>')
        parts.append(f'<text x="{left-12}" y="{yy+5:.1f}" text-anchor="end" class="axis">{tick/1_000_000:g}M</text>')
    slot = width / 4
    for i, scenario in enumerate(scenarios):
        item = agg[scenario]
        center = left + slot * (i + 0.5)
        for offset, key, color in [(-48, "expected", "#94a3b8"), (12, "received", "#2563eb")]:
            value = item[key]
            yy = sy(value)
            parts.append(f'<rect x="{center+offset:.1f}" y="{yy:.1f}" width="36" height="{top+height-yy:.1f}" fill="{color}"/>')
        verdict = f'{item["pass"]}/{item["runs"]} PASS'
        verdict_color = "#15803d" if item["pass"] == item["runs"] else "#b91c1c"
        parts.append(f'<text x="{center:.1f}" y="{top+height+31}" text-anchor="middle" class="label">{scenario}</text>')
        parts.append(f'<text x="{center:.1f}" y="{top+height+54}" text-anchor="middle" class="note" fill="{verdict_color}">{verdict}</text>')
        parts.append(f'<text x="{center:.1f}" y="{top+height+74}" text-anchor="middle" class="note">missing={item["missing"]:,}</text>')
    parts.append('<rect x="1020" y="91" width="22" height="14" fill="#94a3b8"/><text x="1050" y="104" class="axis">Expected</text>')
    parts.append('<rect x="1145" y="91" width="22" height="14" fill="#2563eb"/><text x="1175" y="104" class="axis">Received</text>')
    parts.append('<text x="125" y="685" class="note">C3 deliberately injects 5% malformed JSON. The observed batch-level parser behavior discarded every contaminated batch, so C3 failed while C1, C2 and C4 were exact.</text>')
    save(parts, OUT / "02_wcm_correctness.svg")


def plot_short_burst():
    sources = {
        "WCM": "wcm_burst_forward30_formal_20260805",
        "Vector": "vector_burst_forward30_formal_20260805",
        "Fluentd": "fluentd_burst_forward30_formal_20260805",
        "Fluent Bit WASM": "fbit_wasm_burst_forward30_formal_20260806",
    }
    data = {}
    for product, run in sources.items():
        path = COMPARE / "resilience/runs" / run / "summary.csv"
        with path.open(newline="", encoding="utf-8") as handle:
            data[product] = [{k: v for k, v in row.items()} for row in csv.DictReader(handle)]
    parts = svg_start(1450, 900, "Short-burst recovery comparison", "Burst at 150% M for 5, 15 and 30 seconds; recovery requires 30 seconds of stable output")
    products = list(sources)
    for panel, (top, field, title, ymax, ylabel) in enumerate([
        (130, "recovery_time_s", "A. Recovery time after burst end", 80, "Recovery time (s)"),
        (510, "max_post_burst_p99_ms", "B. Maximum post-burst p99 latency", 30000, "Maximum p99 (ms)"),
    ]):
        left, width, height = 115, 1260, 260
        parts.append(f'<text x="{left}" y="{top-20}" class="label">{escape(title)}</text>')
        sx = lambda value: left + (value - 5) / 25 * width
        sy = lambda value, t=top, m=ymax: t + height - value / m * height
        xticks = [(5, "5"), (15, "15"), (30, "30")]
        yticks = [(v, f"{v/1000:g}k" if v >= 1000 else str(v)) for v in [i * ymax / 4 for i in range(5)]]
        axes(parts, left, top, width, height, xticks, yticks, sx, sy, "Burst duration (s)", ylabel)
        for product in products:
            points = []
            for row in data[product]:
                points.append((sx(float(row["burst_duration_s"])), sy(float(row[field]))))
            coords = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
            color = COLORS[product]
            parts.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="3"/>')
            for x, y in points:
                parts.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="5" fill="#fff" stroke="{color}" stroke-width="3"/>')
    for i, product in enumerate(products):
        x = 830 + (i % 2) * 260
        y = 95 + (i // 2) * 25
        parts.append(f'<line x1="{x}" y1="{y}" x2="{x+30}" y2="{y}" stroke="{COLORS[product]}" stroke-width="4"/>')
        parts.append(f'<text x="{x+40}" y="{y+5}" class="axis">{escape(product)}</text>')
    save(parts, OUT / "03_short_burst_recovery.svg")


def bin_rows(rows, time_key, value_keys, offset, width=30):
    bins = defaultdict(lambda: defaultdict(list))
    for row in rows:
        t = float(row[time_key]) - offset
        if not 0 <= t <= 3600:
            continue
        index = int(t // width)
        for key in value_keys:
            value = float(row[key])
            if value > 0 or key not in {"p99_ms", "lines"}:
                bins[index][key].append(value)
    output = []
    for index in sorted(bins):
        item = {"minute": (index * width + width / 2) / 60}
        for key, values in bins[index].items():
            item[key] = max(values) if key == "p99_ms" else sum(values) / len(values)
        output.append(item)
    return output


def plot_stability():
    run = COMPARE / "resilience/runs/wcm_stability_20k_1h_manual_r1_20260809/S1_rep1"
    with (run / "phases.csv").open(newline="", encoding="utf-8") as handle:
        phases = {row["phase"]: row for row in csv.DictReader(handle)}
    with (run / "sink_per_sec.csv").open(newline="", encoding="utf-8") as handle:
        sink = list(csv.DictReader(handle))
    with (run / "stats.csv").open(newline="", encoding="utf-8") as handle:
        stats = list(csv.DictReader(handle))
    steady_start = float(phases["steady"]["start_unix"])
    warm_duration = float(phases["warmup"]["duration_s"])
    sink_bins = bin_rows(sink, "unix_s", ["lines", "p99_ms"], steady_start)
    stat_bins = bin_rows(stats, "elapsed_s", ["cpu_pct", "rss_mb"], warm_duration)
    parts = svg_start(1450, 1120, "WCM one-hour stability timeline", "20,000 input EPS (80% of the manually confirmed 25,000 EPS capacity); 30-second bins")
    panels = [
        (120, sink_bins, "lines", 8000, "Filtered output (EPS)"),
        (355, sink_bins, "p99_ms", 200, "Maximum p99 per bin (ms)"),
        (590, stat_bins, "cpu_pct", 110, "Mean CPU (%)"),
        (825, stat_bins, "rss_mb", 60, "Mean RSS (MiB)"),
    ]
    colors = ["#16a34a", "#dc2626", "#2563eb", "#7c3aed"]
    for (top, rows, key, ymax, ylabel), color in zip(panels, colors):
        left, width, height = 115, 1260, 155
        sx = lambda value: left + value / 60 * width
        sy = lambda value, t=top, m=ymax: t + height - value / m * height
        xticks = [(v, str(v)) for v in range(0, 61, 10)]
        yticks = [(v, f"{v:g}") for v in [i * ymax / 4 for i in range(5)]]
        axes(parts, left, top, width, height, xticks, yticks, sx, sy, "Steady-window time (minutes)", ylabel)
        points = [(sx(row["minute"]), sy(row[key])) for row in rows if key in row]
        coords = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
        parts.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="2.4"/>')
    parts.append('<text x="115" y="1100" class="note">Output was exact; p99 remained below 168 ms, with no loss, duplication, corruption, reordering, OOM or restart.</text>')
    save(parts, OUT / "04_wcm_one_hour_stability.svg")


if __name__ == "__main__":
    plot_legacy_capacity_latency()
    plot_correctness()
    plot_short_burst()
    plot_stability()
    print(f"wrote catalog charts to {OUT}")
