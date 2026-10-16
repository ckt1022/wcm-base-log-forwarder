#!/usr/bin/env python3
"""Render WCM latency, CPU, and memory sweep as dependency-free SVG."""

import csv
import math
from pathlib import Path


HERE = Path(__file__).resolve().parent
CSV_PATH = HERE / "wcm_fields1k_latency_sweep.csv"
SVG_PATH = HERE / "wcm_fields1k_latency_sweep.svg"


def esc(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


with CSV_PATH.open(newline="", encoding="utf-8") as handle:
    rows = list(csv.DictReader(handle))
for row in rows:
    for key in row:
        if key not in {"status"}:
            row[key] = float(row[key])

width, height = 1500, 1280
left, right, panel_w, panel_h = 130, 60, 1310, 285
tops = [150, 520, 890]
x_min, x_max = 2500, 25000


def xpos(value):
    return left + (value - x_min) / (x_max - x_min) * panel_w


out = [
    f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
    '<rect width="100%" height="100%" fill="#fff"/>',
    '<style>text{font-family:Inter,Arial,"Noto Sans",sans-serif;fill:#172033}.title{font-size:29px;font-weight:700}.sub{font-size:16px;fill:#526174}.pt{font-size:20px;font-weight:700}.axis{font-size:14px}.grid{stroke:#dfe5ec}.axisline{stroke:#526174;stroke-width:1.4}.line{fill:none;stroke-width:3}.marker{stroke:#fff;stroke-width:1.5}.note{font-size:14px;fill:#66758a}</style>',
    '<text x="70" y="54" class="title">WCM 1024 B / 36-field latency sweep</text>',
    '<text x="70" y="85" class="sub">1 CPU / 512 MiB / 180 s per stage / statistics window 30–150 s / 1 s flush</text>',
]


def panel(top, title, ticks, series, transform=lambda value: value, label=lambda value: str(value)):
    lo, hi = transform(ticks[0]), transform(ticks[-1])

    def ypos(value):
        return top + panel_h - (transform(value) - lo) / (hi - lo) * panel_h

    out.append(f'<text x="{left}" y="{top-24}" class="pt">{esc(title)}</text>')
    overload_x = xpos(23000)
    out.append(f'<rect x="{overload_x:.1f}" y="{top}" width="{left+panel_w-overload_x:.1f}" height="{panel_h}" fill="#fee2e2" opacity="0.55"/>')
    for tick in ticks:
        y = ypos(tick)
        out.append(f'<line x1="{left}" y1="{y:.1f}" x2="{left+panel_w}" y2="{y:.1f}" class="grid"/>')
        out.append(f'<text x="{left-16}" y="{y+5:.1f}" text-anchor="end" class="axis">{esc(label(tick))}</text>')
    for rate in range(2500, 25001, 2500):
        x = xpos(rate)
        out.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top+panel_h}" class="grid"/>')
        out.append(f'<text x="{x:.1f}" y="{top+panel_h+27}" text-anchor="middle" class="axis">{rate//1000 if rate%1000==0 else rate/1000:g}k</text>')
    out.append(f'<line x1="{overload_x:.1f}" y1="{top}" x2="{overload_x:.1f}" y2="{top+panel_h}" stroke="#dc2626" stroke-width="2" stroke-dasharray="7 6"/>')
    out.append(f'<text x="{overload_x-8:.1f}" y="{top+18}" text-anchor="end" class="note">confirmed capacity 23k</text>')
    out.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top+panel_h}" class="axisline"/>')
    out.append(f'<line x1="{left}" y1="{top+panel_h}" x2="{left+panel_w}" y2="{top+panel_h}" class="axisline"/>')
    for name, key, color in series:
        points = [(xpos(row["target_eps"]), ypos(row[key])) for row in rows]
        out.append(f'<polyline points="{" ".join(f"{x:.1f},{y:.1f}" for x,y in points)}" class="line" stroke="{color}"/>')
        for x, y in points:
            out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="6" fill="{color}" class="marker"/>')
        lx = left + panel_w - 190 * (len(series) - series.index((name, key, color)))
        out.append(f'<line x1="{lx}" y1="{top-23}" x2="{lx+28}" y2="{top-23}" stroke="{color}" stroke-width="4"/>')
        out.append(f'<text x="{lx+36}" y="{top-17}" class="axis">{esc(name)}</text>')


panel(tops[0], "A. End-to-end latency (ms, log scale)", [30, 100, 300, 1000, 3000, 10000, 15000], [
    ("p50", "p50_ms", "#2563eb"), ("p95", "p95_ms", "#16a34a"),
    ("p99", "p99_ms", "#ea580c"), ("p99.9", "p99_9_ms", "#dc2626")], math.log10)
panel(tops[1], "B. DUT CPU utilization (%)", [0, 20, 40, 60, 80, 100, 110], [
    ("average", "cpu_mean_pct", "#2563eb"), ("maximum", "cpu_max_pct", "#dc2626")])
panel(tops[2], "C. DUT resident memory (MiB)", [0, 30, 60, 90, 120, 150, 180], [
    ("average", "mem_mean_mib", "#2563eb"), ("maximum", "mem_max_mib", "#9333ea")])
out.append('<text x="785" y="1252" text-anchor="middle" class="axis">Target input rate (EPS)</text>')
out.append('</svg>')
SVG_PATH.write_text("\n".join(out) + "\n", encoding="utf-8")
print(f"wrote {SVG_PATH}")
