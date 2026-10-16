#!/usr/bin/env python3
"""Generate concise, bar-based figures for the RQ2 report section."""

import csv
from html import escape
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RESULTS = ROOT / "test/compare/results"
OUT = ROOT / "report-assets/rq2"
OUT.mkdir(parents=True, exist_ok=True)

BLUE = "#2563eb"
COLORS = {
    "WCM": BLUE,
    "WCM Canonical ABI": BLUE,
    "Core JSON ABI": "#f59e0b",
    "Core Direct Memory ABI": "#64748b",
    "Fluent Bit Lua": "#0891b2",
    "Fluentd": "#ea580c",
    "Vector": "#16a34a",
    "Fluent Bit WASM": "#7c3aed",
}


def start_svg(width, height, title):
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="#ffffff"/>',
        '<style>text{font-family:Arial,"Helvetica Neue",sans-serif;fill:#172033}'
        '.title{font-size:28px;font-weight:700}'
        '.axis{font-size:15px}.label{font-size:16px;font-weight:600}.value{font-size:15px;font-weight:700}'
        '.grid{stroke:#e2e8f0;stroke-width:1}'
        '.axisline{stroke:#64748b;stroke-width:1.5}</style>',
        f'<text x="70" y="46" class="title">{escape(title)}</text>',
    ]


def save(parts, path):
    parts.append("</svg>")
    path.write_text("\n".join(parts) + "\n", encoding="utf-8")


def vertical_grid(parts, left, top, width, height, ticks, maximum, unit=""):
    for tick in ticks:
        x = left + tick / maximum * width
        parts.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top+height}" class="grid"/>')
        parts.append(f'<text x="{x:.1f}" y="{top+height+27}" text-anchor="middle" class="axis">{tick:g}{escape(unit)}</text>')
    parts.append(f'<line x1="{left}" y1="{top+height}" x2="{left+width}" y2="{top+height}" class="axisline"/>')


def figure_abi_boundary():
    source = ROOT / "test/ours/copy_overhead/comparison_results/wcm_core_json_direct_comparison.csv"
    with source.open(newline="", encoding="utf-8") as handle:
        rows = [row for row in csv.DictReader(handle) if int(row["batch_count"]) == 1024]

    methods = [
        ("WCM Canonical ABI", "wcm_boundary_median_ns"),
        ("Core JSON ABI", "core_json_boundary_median_ns"),
        ("Core Direct Memory ABI", "core_direct_boundary_median_ns"),
    ]
    stages = ["parse", "filter", "format"]
    labels = {"parse": "Parse", "filter": "Filter", "format": "Format"}
    values = {
        row["stage"]: {name: float(row[field]) / 1_000_000 for name, field in methods}
        for row in rows
    }

    width, height = 1400, 760
    left, top, plot_w, plot_h = 125, 95, 1200, 470
    maximum = 10.0
    parts = start_svg(
        width,
        height,
        "ABI Boundary Cost by Processing Stage",
    )
    for tick in range(0, 11, 2):
        y = top + plot_h - tick / maximum * plot_h
        parts.append(f'<line x1="{left}" y1="{y:.1f}" x2="{left+plot_w}" y2="{y:.1f}" class="grid"/>')
        parts.append(f'<text x="{left-14}" y="{y+5:.1f}" text-anchor="end" class="axis">{tick}</text>')
    parts.append(f'<line x1="{left}" y1="{top+plot_h}" x2="{left+plot_w}" y2="{top+plot_h}" class="axisline"/>')
    parts.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top+plot_h}" class="axisline"/>')

    group_w = plot_w / len(stages)
    bar_w, gap = 70, 22
    for stage_index, stage in enumerate(stages):
        total_w = len(methods) * bar_w + (len(methods) - 1) * gap
        group_left = left + stage_index * group_w + (group_w - total_w) / 2
        for method_index, (method, _) in enumerate(methods):
            value = values[stage][method]
            x = group_left + method_index * (bar_w + gap)
            y = top + plot_h - value / maximum * plot_h
            parts.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{bar_w}" height="{top+plot_h-y:.1f}" rx="4" fill="{COLORS[method]}"/>')
            parts.append(f'<text x="{x+bar_w/2:.1f}" y="{y-9:.1f}" text-anchor="middle" class="value">{value:.3f}</text>')
        center = left + stage_index * group_w + group_w / 2
        parts.append(f'<text x="{center:.1f}" y="{top+plot_h+38}" text-anchor="middle" class="label">{labels[stage]}</text>')

    parts.append(f'<text x="34" y="{top+plot_h/2}" text-anchor="middle" transform="rotate(-90 34 {top+plot_h/2})" class="axis">Median Boundary Cost (ms)</text>')
    legend_x = 270
    for index, (method, _) in enumerate(methods):
        x = legend_x + index * 330
        parts.append(f'<rect x="{x}" y="625" width="24" height="16" rx="3" fill="{COLORS[method]}"/>')
        parts.append(f'<text x="{x+34}" y="639" class="axis">{escape(method)}</text>')
    save(parts, OUT / "01_abi_boundary_cost_1mib.svg")


def figure_capacity():
    with (RESULTS / "multifield_capacity_256_2048.csv").open(newline="", encoding="utf-8") as handle:
        rows = [row for row in csv.DictReader(handle) if int(row["log_bytes"]) == 1024]
    by_product = {row["product"]: row for row in rows}
    products = ["Fluent Bit Lua", "Fluentd", "WCM", "Vector", "Fluent Bit WASM"]

    width, height = 1400, 720
    left, top, plot_w, plot_h = 270, 95, 1030, 440
    maximum = 70_000
    parts = start_svg(
        width,
        height,
        "Maximum Sustainable Throughput",
    )
    vertical_grid(parts, left, top, plot_w, plot_h, range(0, 70_001, 10_000), maximum)
    slot = plot_h / len(products)
    bar_h = 48
    for index, product in enumerate(products):
        value = int(float(by_product[product]["capacity_eps"]))
        mb_s = float(by_product[product]["input_mb_s"])
        y = top + index * slot + (slot - bar_h) / 2
        bar_w = value / maximum * plot_w
        color = COLORS[product] if product == "WCM" else "#94a3b8"
        parts.append(f'<text x="{left-18}" y="{y+31:.1f}" text-anchor="end" class="label">{escape(product)}</text>')
        parts.append(f'<rect x="{left}" y="{y:.1f}" width="{bar_w:.1f}" height="{bar_h}" rx="5" fill="{color}"/>')
        parts.append(f'<text x="{left+bar_w+12:.1f}" y="{y+30:.1f}" class="value">{value:,} EPS（{mb_s:.1f} MB/s）</text>')
    parts.append(f'<text x="{left+plot_w/2}" y="{top+plot_h+65}" text-anchor="middle" class="axis">Maximum Sustainable Input Rate (EPS)</text>')
    save(parts, OUT / "02_product_capacity_1k.svg")


def figure_equal_load_latency():
    files = {
        "WCM": "wcm_fields1k_latency_cgroup_rerun.csv",
        "Fluentd": "fluentd_fields1k_latency_sweep.csv",
        "Vector": "vector_fields1k_latency_cgroup_sweep.csv",
        "Fluent Bit Lua": "fbit_lua_fields1k_latency_cgroup_sweep.csv",
        "Fluent Bit WASM": "fbit_wasm_fields1k_latency_cgroup_sweep.csv",
    }
    values = {}
    for product, filename in files.items():
        with (RESULTS / filename).open(newline="", encoding="utf-8") as handle:
            row = next(row for row in csv.DictReader(handle) if int(float(row["target_eps"])) == 5000)
        values[product] = float(row["p99_ms"])
    products = ["WCM", "Fluentd", "Vector", "Fluent Bit Lua", "Fluent Bit WASM"]

    width, height = 1400, 720
    left, top, plot_w, plot_h = 270, 95, 1030, 440
    maximum = 1100
    parts = start_svg(
        width,
        height,
        "p99 Latency at 5,000 EPS",
    )
    vertical_grid(parts, left, top, plot_w, plot_h, range(0, 1101, 200), maximum, "")
    slot = plot_h / len(products)
    bar_h = 48
    for index, product in enumerate(products):
        value = values[product]
        y = top + index * slot + (slot - bar_h) / 2
        bar_w = value / maximum * plot_w
        color = COLORS[product] if product == "WCM" else "#94a3b8"
        parts.append(f'<text x="{left-18}" y="{y+31:.1f}" text-anchor="end" class="label">{escape(product)}</text>')
        parts.append(f'<rect x="{left}" y="{y:.1f}" width="{bar_w:.1f}" height="{bar_h}" rx="5" fill="{color}"/>')
        parts.append(f'<text x="{left+bar_w+12:.1f}" y="{y+30:.1f}" class="value">{value:,.3f} ms</text>')
    parts.append(f'<text x="{left+plot_w/2}" y="{top+plot_h+65}" text-anchor="middle" class="axis">End-to-End p99 Latency (ms)</text>')
    save(parts, OUT / "03_product_p99_at_5k.svg")


if __name__ == "__main__":
    figure_abi_boundary()
    figure_capacity()
    figure_equal_load_latency()
    print(f"wrote RQ2 report figures to {OUT}")
