#!/usr/bin/env python3
"""Render the five-product multifield capacity comparison as dependency-free SVG."""

import csv
import math
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
CSV_PATH = ROOT / "test/compare/results/multifield_capacity_256_2048.csv"
SVG_PATH = ROOT / "test/compare/results/multifield_capacity_256_2048.svg"

PRODUCTS = [
    "Fluent Bit Lua",
    "Fluentd",
    "WCM",
    "Vector",
    "Fluent Bit WASM",
]
COLORS = {
    "Fluent Bit Lua": "#2563eb",
    "Fluentd": "#dc2626",
    "WCM": "#16a34a",
    "Vector": "#9333ea",
    "Fluent Bit WASM": "#ea580c",
}


def load_rows():
    grouped = {product: [] for product in PRODUCTS}
    with CSV_PATH.open(newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            row["log_bytes"] = int(row["log_bytes"])
            row["capacity_eps"] = int(row["capacity_eps"])
            row["input_mb_s"] = float(row["input_mb_s"])
            grouped[row["product"]].append(row)
    for rows in grouped.values():
        rows.sort(key=lambda row: row["log_bytes"])
    return grouped


def esc(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def render():
    grouped = load_rows()
    width, height = 1600, 1320
    left, right = 145, 70
    panel_w = width - left - right
    panel_h = 285
    panel_tops = [165, 535, 905]
    sizes = [128, 256, 512, 768, 1024, 1536, 2048]
    x_min, x_max = 128, 2048

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="#ffffff"/>',
        '<style>text{font-family:Inter,Arial,"Noto Sans",sans-serif;fill:#172033}.title{font-size:30px;font-weight:700}.subtitle{font-size:17px;fill:#526174}.axis{font-size:15px}.panel-title{font-size:21px;font-weight:700}.grid{stroke:#dfe5ec;stroke-width:1}.axisline{stroke:#526174;stroke-width:1.4}.line{fill:none;stroke-width:3}.marker{stroke:#fff;stroke-width:2}.legend{font-size:16px;font-weight:600}.note{font-size:14px;fill:#66758a}</style>',
        '<text x="80" y="58" class="title">Multifield JSON capacity: five products, 128–2048 bytes</text>',
        '<text x="80" y="90" class="subtitle">1 CPU / 512 MiB / TCP single connection / 1 s flush / 40% retained / confirmed sustainable capacity</text>',
    ]

    legend_x, legend_y = 760, 55
    for index, product in enumerate(PRODUCTS):
        x = legend_x + (index % 3) * 260
        y = legend_y + (index // 3) * 34
        color = COLORS[product]
        out.append(f'<line x1="{x}" y1="{y}" x2="{x+34}" y2="{y}" stroke="{color}" stroke-width="4"/>')
        out.append(f'<circle cx="{x+17}" cy="{y}" r="6" fill="{color}"/>')
        out.append(f'<text x="{x+45}" y="{y+6}" class="legend">{esc(product)}</text>')

    def x_pos(size):
        return left + (size - x_min) / (x_max - x_min) * panel_w

    def panel(top, title, y_ticks, y_value, y_transform=lambda value: value, tick_label=str):
        transformed = [y_transform(value) for value in y_ticks]
        y_lo, y_hi = min(transformed), max(transformed)

        def y_pos(value):
            transformed_value = y_transform(value)
            return top + panel_h - (transformed_value - y_lo) / (y_hi - y_lo) * panel_h

        out.append(f'<text x="{left}" y="{top-28}" class="panel-title">{esc(title)}</text>')
        for tick in y_ticks:
            y = y_pos(tick)
            out.append(f'<line x1="{left}" y1="{y:.1f}" x2="{left+panel_w}" y2="{y:.1f}" class="grid"/>')
            out.append(f'<text x="{left-18}" y="{y+5:.1f}" text-anchor="end" class="axis">{esc(tick_label(tick))}</text>')
        for size in sizes:
            x = x_pos(size)
            out.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top+panel_h}" class="grid"/>')
            out.append(f'<text x="{x:.1f}" y="{top+panel_h+28}" text-anchor="middle" class="axis">{size}</text>')
        out.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top+panel_h}" class="axisline"/>')
        out.append(f'<line x1="{left}" y1="{top+panel_h}" x2="{left+panel_w}" y2="{top+panel_h}" class="axisline"/>')

        for product in PRODUCTS:
            rows = grouped[product]
            points = [(x_pos(row["log_bytes"]), y_pos(y_value(product, row))) for row in rows]
            point_text = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
            color = COLORS[product]
            out.append(f'<polyline points="{point_text}" class="line" stroke="{color}"/>')
            for x, y in points:
                out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="7" fill="{color}" class="marker"/>')

    panel(
        panel_tops[0],
        "A. Maximum sustainable input rate (EPS, logarithmic scale)",
        [2000, 5000, 10000, 20000, 50000, 100000, 250000],
        lambda _product, row: row["capacity_eps"],
        y_transform=math.log10,
        tick_label=lambda value: f"{value//1000}k" if value >= 1000 else str(value),
    )
    panel(
        panel_tops[1],
        "B. Maximum sustainable input throughput (decimal MB/s)",
        [0, 10, 20, 30, 40, 50, 60, 70, 80],
        lambda _product, row: row["input_mb_s"],
        tick_label=lambda value: str(value),
    )
    maxima = {product: max(row["input_mb_s"] for row in rows) for product, rows in grouped.items()}
    panel(
        panel_tops[2],
        "C. Throughput normalized to each product's own peak",
        [0, 20, 40, 60, 80, 100],
        lambda product, row: row["input_mb_s"] / maxima[product] * 100,
        tick_label=lambda value: f"{value}%",
    )
    out.append(f'<text x="{left+panel_w/2:.1f}" y="1270" text-anchor="middle" class="axis">Input JSON size (bytes, excluding newline)</text>')
    out.append('<text x="80" y="1302" class="note">Field count scales with size: 4, 8–9, 17–18, 27, 36, 54, 72 top-level fields. Curves therefore combine per-record, per-byte, and per-field costs.</text>')
    out.append('</svg>')
    SVG_PATH.write_text("\n".join(out) + "\n", encoding="utf-8")


if __name__ == "__main__":
    render()
