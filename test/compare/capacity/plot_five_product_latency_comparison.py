#!/usr/bin/env python3
"""Render four dependency-free SVG charts for the five-product latency sweep."""

import csv
import math
from html import escape
from pathlib import Path


HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent / "results"
OUT = RESULTS / "five_product_latency_charts"

PRODUCTS = {
    "WCM": ("wcm_fields1k_latency_cgroup_rerun.csv", "#2563eb"),
    "Fluentd": ("fluentd_fields1k_latency_sweep.csv", "#ea580c"),
    "Vector": ("vector_fields1k_latency_cgroup_sweep.csv", "#16a34a"),
    "Fluent Bit WASM": ("fbit_wasm_fields1k_latency_cgroup_sweep.csv", "#7c3aed"),
    "Fluent Bit Lua": ("fbit_lua_fields1k_latency_cgroup_sweep.csv", "#0891b2"),
}


def load_rows(filename):
    with (RESULTS / filename).open(newline="", encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))
    for row in rows:
        for key in row:
            if key not in {"status"}:
                row[key] = float(row[key])
    return rows


DATA = {name: (load_rows(filename), color) for name, (filename, color) in PRODUCTS.items()}


def fmt_eps(value):
    if value == 0:
        return "0"
    return f"{value / 1000:g}K"


def polyline(points, color):
    coords = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
    return f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"/>'


def chart(filename, field, title, y_label, y_ticks, log_y=False, exclude_lua_cold_mem=False):
    width, height = 1400, 800
    left, right, top, bottom = 105, 45, 88, 95
    plot_w, plot_h = width - left - right, height - top - bottom
    x_min, x_max = 0.0, 70000.0
    y_min, y_max = float(y_ticks[0]), float(y_ticks[-1])

    def sx(value):
        return left + (value - x_min) / (x_max - x_min) * plot_w

    def sy(value):
        if log_y:
            lo, hi = math.log10(y_min), math.log10(y_max)
            return top + (hi - math.log10(max(value, y_min))) / (hi - lo) * plot_h
        return top + (y_max - value) / (y_max - y_min) * plot_h

    body = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="#ffffff"/>',
        '<style>text{font-family:"Noto Sans CJK TC","Noto Sans",sans-serif;fill:#172033}.tick{font-size:16px}.axis{font-size:18px}.note{font-size:14px;fill:#475569}.legend{font-size:16px}.title{font-size:27px;font-weight:700}.fail{font-size:13px;fill:#b91c1c;font-weight:700}</style>',
        f'<text x="{width / 2}" y="42" text-anchor="middle" class="title">{escape(title)}</text>',
        '<text x="700" y="69" text-anchor="middle" class="note">1024 B / 36 fields · 1 CPU · 512 MiB · 1 s flush · 30–150 s window</text>',
    ]

    for tick in range(0, 70001, 10000):
        x = sx(tick)
        body.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top + plot_h}" stroke="#e2e8f0"/>')
        body.append(f'<text x="{x:.1f}" y="{top + plot_h + 30}" text-anchor="middle" class="tick">{fmt_eps(tick)}</text>')
    for tick in y_ticks:
        y = sy(tick)
        body.append(f'<line x1="{left}" y1="{y:.1f}" x2="{left + plot_w}" y2="{y:.1f}" stroke="#e2e8f0"/>')
        label = f"{tick:g}"
        body.append(f'<text x="{left - 14}" y="{y + 6:.1f}" text-anchor="end" class="tick">{label}</text>')

    body.extend([
        f'<line x1="{left}" y1="{top + plot_h}" x2="{left + plot_w}" y2="{top + plot_h}" stroke="#334155" stroke-width="2"/>',
        f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top + plot_h}" stroke="#334155" stroke-width="2"/>',
        f'<text x="{left + plot_w / 2}" y="{height - 28}" text-anchor="middle" class="axis">Target rate (EPS)</text>',
        f'<text x="28" y="{top + plot_h / 2}" text-anchor="middle" transform="rotate(-90 28 {top + plot_h / 2})" class="axis">{escape(y_label)}</text>',
    ])

    for name, (rows, color) in DATA.items():
        pass_rows = [r for r in rows if r["status"] == "PASS"]
        if exclude_lua_cold_mem and name == "Fluent Bit Lua":
            pass_rows = [r for r in pass_rows if r["target_eps"] != 2500]
        points = [(sx(r["target_eps"]), sy(r[field])) for r in pass_rows]
        body.append(polyline(points, color))
        for x, y in points:
            body.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="4.2" fill="#fff" stroke="{color}" stroke-width="2.4"/>')

        first_fail = next((r for r in rows if r["status"] == "FAIL"), None)
        if first_fail:
            x, y = sx(first_fail["target_eps"]), sy(first_fail[field])
            body.append(f'<line x1="{x - 7:.1f}" y1="{y - 7:.1f}" x2="{x + 7:.1f}" y2="{y + 7:.1f}" stroke="#dc2626" stroke-width="4"/>')
            body.append(f'<line x1="{x - 7:.1f}" y1="{y + 7:.1f}" x2="{x + 7:.1f}" y2="{y - 7:.1f}" stroke="#dc2626" stroke-width="4"/>')

    legend_x, legend_y = left + 22, top + 24
    for idx, (name, (_, color)) in enumerate(DATA.items()):
        x = legend_x + (idx % 3) * 245
        y = legend_y + (idx // 3) * 29
        body.append(f'<line x1="{x}" y1="{y}" x2="{x + 31}" y2="{y}" stroke="{color}" stroke-width="4"/>')
        body.append(f'<circle cx="{x + 15.5}" cy="{y}" r="4" fill="#fff" stroke="{color}" stroke-width="2"/>')
        body.append(f'<text x="{x + 40}" y="{y + 6}" class="legend">{escape(name)}</text>')
    fx, fy = legend_x + 2 * 245, legend_y + 29
    body.append(f'<line x1="{fx + 5}" y1="{fy - 7}" x2="{fx + 19}" y2="{fy + 7}" stroke="#dc2626" stroke-width="4"/>')
    body.append(f'<line x1="{fx + 5}" y1="{fy + 7}" x2="{fx + 19}" y2="{fy - 7}" stroke="#dc2626" stroke-width="4"/>')
    body.append(f'<text x="{fx + 31}" y="{fy + 6}" class="legend">First FAIL</text>')

    if log_y:
        body.append(f'<text x="{left + plot_w - 8}" y="{top + plot_h - 12}" text-anchor="end" class="note">Logarithmic Y axis</text>')
    if exclude_lua_cold_mem:
        body.append(f'<text x="{left + plot_w - 8}" y="{top + plot_h - 12}" text-anchor="end" class="note">Lua 2.5K cold-cache point excluded</text>')
    body.append("</svg>")
    (OUT / filename).write_text("\n".join(body) + "\n", encoding="utf-8")


OUT.mkdir(parents=True, exist_ok=True)
chart("01_p50_latency_vs_eps.svg", "p50_ms", "Five-product p50 latency vs EPS", "p50 latency (ms)", [30, 100, 300, 1000, 3000, 10000, 30000], log_y=True)
chart("02_p99_latency_vs_eps.svg", "p99_ms", "Five-product p99 latency vs EPS", "p99 latency (ms)", [30, 100, 300, 1000, 3000, 10000, 30000], log_y=True)
chart("03_cpu_mean_vs_eps.svg", "cpu_mean_pct", "Five-product mean CPU vs EPS", "Mean CPU (%)", [0, 20, 40, 60, 80, 100, 110])
chart("04_memory_mean_vs_eps.svg", "mem_mean_mib", "Five-product mean memory vs EPS", "Mean cgroup memory (MiB)", [0, 50, 100, 150, 200, 250, 300], exclude_lua_cold_mem=True)
print(f"wrote four charts to {OUT}")
