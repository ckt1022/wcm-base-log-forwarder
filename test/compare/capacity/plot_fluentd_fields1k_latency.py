#!/usr/bin/env python3
"""Create a dependency-free SVG for the Fluentd latency sweep."""

import csv
import math
from pathlib import Path


HERE = Path(__file__).resolve().parent
CSV_PATH = HERE.parent / "results/fluentd_fields1k_latency_sweep.csv"
OUT_PATH = HERE.parent / "results/fluentd_fields1k_latency_sweep.svg"
WIDTH, HEIGHT = 1100, 1000
LEFT, RIGHT = 92, 35
PLOT_WIDTH = WIDTH - LEFT - RIGHT
PANELS = ((100, 315), (390, 605), (680, 895))
COLORS = ("#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd")


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def x_pos(rate):
    return LEFT + (rate - 2.5) / (50 - 2.5) * PLOT_WIDTH


def y_linear(value, low, high, top, bottom):
    return bottom - (value - low) / (high - low) * (bottom - top)


def y_log(value, low, high, top, bottom):
    return y_linear(math.log10(value), math.log10(low), math.log10(high), top, bottom)


def line(x1, y1, x2, y2, **attrs):
    style = " ".join(f'{key.rstrip("_").replace("_", "-")}="{value}"' for key, value in attrs.items())
    svg.append(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" {style}/>')


def text(x, y, value, **attrs):
    style = " ".join(f'{key.rstrip("_").replace("_", "-")}="{val}"' for key, val in attrs.items())
    svg.append(f'<text x="{x:.1f}" y="{y:.1f}" {style}>{esc(value)}</text>')


def polyline(points, color):
    coords = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
    svg.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="2"/>')
    for x, y in points:
        svg.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="3" fill="{color}"/>')


with CSV_PATH.open(encoding="utf-8") as handle:
    rows = list(csv.DictReader(handle))
rates = [int(row["target_eps"]) / 1000 for row in rows]

svg = [
    f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">',
    '<rect width="100%" height="100%" fill="white"/>',
    '<style>text{font-family:Arial,sans-serif;fill:#222}.tick{font-size:12px}.label{font-size:14px}.title{font-size:22px;font-weight:bold}.subtitle{font-size:14px;fill:#555}</style>',
]
text(WIDTH / 2, 34, "Fluentd 1024 B / 36-field latency sweep", text_anchor="middle", class_="title")
text(WIDTH / 2, 57, "1 CPU, 512 MiB, 180 s/stage; statistics window 30–150 s", text_anchor="middle", class_="subtitle")

# Shared panel backgrounds and X guides.
for top, bottom in PANELS:
    svg.append(f'<rect x="{x_pos(45):.1f}" y="{top}" width="{x_pos(50)-x_pos(45):.1f}" height="{bottom-top}" fill="#d62728" opacity="0.08"/>')
    for rate in (10, 20, 30, 40, 50):
        x = x_pos(rate)
        line(x, top, x, bottom, stroke="#ddd", stroke_width="1")
    line(LEFT, bottom, WIDTH - RIGHT, bottom, stroke="#333", stroke_width="1")
    line(LEFT, top, LEFT, bottom, stroke="#333", stroke_width="1")
    line(x_pos(45), top, x_pos(45), bottom, stroke="#2ca02c", stroke_width="1.5", stroke_dasharray="6 4")

# Latency (log scale).
top, bottom = PANELS[0]
for tick in (10, 30, 100, 300, 1000, 2000):
    y = y_log(tick, 10, 2000, top, bottom)
    line(LEFT, y, WIDTH - RIGHT, y, stroke="#ddd", stroke_width="1")
    text(LEFT - 10, y + 4, tick, text_anchor="end", class_="tick")
latency_fields = (("p50_ms", "p50"), ("p90_ms", "p90"), ("p95_ms", "p95"), ("p99_ms", "p99"), ("p99_9_ms", "p99.9"))
for index, (field, label) in enumerate(latency_fields):
    points = [(x_pos(rate), y_log(float(row[field]), 10, 2000, top, bottom)) for rate, row in zip(rates, rows)]
    polyline(points, COLORS[index])
    lx = LEFT + 100 + index * 145
    line(lx, top + 17, lx + 24, top + 17, stroke=COLORS[index], stroke_width="3")
    text(lx + 31, top + 22, label, class_="label")
text(24, (top + bottom) / 2, "Latency (ms, log)", transform=f"rotate(-90 24 {(top+bottom)/2})", text_anchor="middle", class_="label")
text(x_pos(48), top + 42, "50k FAIL", text_anchor="middle", fill="#d62728", class_="label")

# CPU.
top, bottom = PANELS[1]
for tick in (0, 20, 40, 60, 80, 100):
    y = y_linear(tick, 0, 105, top, bottom)
    line(LEFT, y, WIDTH - RIGHT, y, stroke="#ddd", stroke_width="1")
    text(LEFT - 10, y + 4, tick, text_anchor="end", class_="tick")
for index, (field, label) in enumerate((("cpu_mean_pct", "average"), ("cpu_max_pct", "maximum"))):
    points = [(x_pos(rate), y_linear(float(row[field]), 0, 105, top, bottom)) for rate, row in zip(rates, rows)]
    polyline(points, COLORS[index])
    lx = LEFT + 300 + index * 180
    line(lx, top + 17, lx + 24, top + 17, stroke=COLORS[index], stroke_width="3")
    text(lx + 31, top + 22, label, class_="label")
text(24, (top + bottom) / 2, "Container CPU (%)", transform=f"rotate(-90 24 {(top+bottom)/2})", text_anchor="middle", class_="label")

# Memory.
top, bottom = PANELS[2]
for tick in (0, 50, 100, 150, 200):
    y = y_linear(tick, 0, 200, top, bottom)
    line(LEFT, y, WIDTH - RIGHT, y, stroke="#ddd", stroke_width="1")
    text(LEFT - 10, y + 4, tick, text_anchor="end", class_="tick")
for index, (field, label) in enumerate((("mem_mean_mib", "average"), ("mem_max_mib", "maximum"))):
    points = [(x_pos(rate), y_linear(float(row[field]), 0, 200, top, bottom)) for rate, row in zip(rates, rows)]
    polyline(points, COLORS[index])
    lx = LEFT + 300 + index * 180
    line(lx, top + 17, lx + 24, top + 17, stroke=COLORS[index], stroke_width="3")
    text(lx + 31, top + 22, label, class_="label")
text(24, (top + bottom) / 2, "Cgroup memory.current (MiB)", transform=f"rotate(-90 24 {(top+bottom)/2})", text_anchor="middle", class_="label")

for rate in (10, 20, 30, 40, 45, 50):
    text(x_pos(rate), PANELS[2][1] + 24, rate, text_anchor="middle", class_="tick")
text(WIDTH / 2, 940, "Target rate (kEPS)", text_anchor="middle", class_="label")
text(WIDTH / 2, 972, "Green dashed line: highest 180 s PASS (45k); red area: transition to 50k failure", text_anchor="middle", class_="subtitle")

svg.append("</svg>")
OUT_PATH.write_text("\n".join(svg) + "\n", encoding="utf-8")
print(f"wrote {OUT_PATH}")
