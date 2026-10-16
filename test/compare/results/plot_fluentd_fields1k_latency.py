#!/usr/bin/env python3
"""Render Fluentd latency, CPU, and memory sweep as dependency-free SVG."""

import csv
import math
from pathlib import Path


HERE = Path(__file__).resolve().parent
with (HERE / "fluentd_fields1k_latency_sweep.csv").open(newline="", encoding="utf-8") as handle:
    rows = list(csv.DictReader(handle))
for row in rows:
    for key in row:
        if key != "status":
            row[key] = float(row[key])

W, H, LEFT, RIGHT, PH = 1500, 1280, 130, 60, 285
PW = W - LEFT - RIGHT
TOPS = (150, 520, 890)
XMIN, XMAX = 2500, 50000
out = [
    f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">',
    '<rect width="100%" height="100%" fill="#fff"/>',
    '<style>text{font-family:Inter,Arial,"Noto Sans",sans-serif;fill:#172033}.title{font-size:29px;font-weight:700}.sub{font-size:16px;fill:#526174}.pt{font-size:20px;font-weight:700}.axis{font-size:14px}.grid{stroke:#dfe5ec}.axisline{stroke:#526174;stroke-width:1.4}.line{fill:none;stroke-width:3}.marker{stroke:#fff;stroke-width:1.5}.note{font-size:14px;fill:#66758a}</style>',
    '<text x="70" y="54" class="title">Fluentd 1024 B / 36-field latency sweep</text>',
    '<text x="70" y="85" class="sub">1 CPU / 512 MiB / 180 s per stage / statistics window 30–150 s / 1 s flush</text>',
]


def xp(value):
    return LEFT + (value - XMIN) / (XMAX - XMIN) * PW


def panel(top, title, ticks, series, transform=lambda v: v, fmt=lambda v: str(v)):
    lo, hi = transform(ticks[0]), transform(ticks[-1])

    def yp(value):
        return top + PH - (transform(value) - lo) / (hi - lo) * PH

    out.append(f'<text x="{LEFT}" y="{top-24}" class="pt">{title}</text>')
    boundary = xp(45000)
    out.append(f'<rect x="{boundary:.1f}" y="{top}" width="{LEFT+PW-boundary:.1f}" height="{PH}" fill="#fee2e2" opacity="0.55"/>')
    for tick in ticks:
        y = yp(tick)
        out.append(f'<line x1="{LEFT}" y1="{y:.1f}" x2="{LEFT+PW}" y2="{y:.1f}" class="grid"/>')
        out.append(f'<text x="{LEFT-16}" y="{y+5:.1f}" text-anchor="end" class="axis">{fmt(tick)}</text>')
    xticks = (2500, 5000, 10000, 15000, 20000, 25000, 30000, 35000, 40000, 45000, 50000)
    for tick in xticks:
        x = xp(tick)
        out.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top+PH}" class="grid"/>')
        out.append(f'<text x="{x:.1f}" y="{top+PH+27}" text-anchor="middle" class="axis">{tick/1000:g}k</text>')
    out.append(f'<line x1="{boundary:.1f}" y1="{top}" x2="{boundary:.1f}" y2="{top+PH}" stroke="#dc2626" stroke-width="2" stroke-dasharray="7 6"/>')
    out.append(f'<text x="{boundary-8:.1f}" y="{top+18}" text-anchor="end" class="note">highest 180 s PASS: 45k</text>')
    out.append(f'<line x1="{LEFT}" y1="{top}" x2="{LEFT}" y2="{top+PH}" class="axisline"/>')
    out.append(f'<line x1="{LEFT}" y1="{top+PH}" x2="{LEFT+PW}" y2="{top+PH}" class="axisline"/>')
    for index, (name, key, color) in enumerate(series):
        points = [(xp(r["target_eps"]), yp(r[key])) for r in rows]
        out.append(f'<polyline points="{" ".join(f"{x:.1f},{y:.1f}" for x,y in points)}" class="line" stroke="{color}"/>')
        for x, y in points:
            out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="6" fill="{color}" class="marker"/>')
        lx = LEFT + 650 + index * 150
        out.append(f'<line x1="{lx}" y1="{top-23}" x2="{lx+27}" y2="{top-23}" stroke="{color}" stroke-width="4"/>')
        out.append(f'<text x="{lx+34}" y="{top-17}" class="axis">{name}</text>')


panel(TOPS[0], "A. End-to-end latency (ms, log scale)", [8, 20, 50, 100, 300, 1000, 2000], [
    ("p50", "p50_ms", "#2563eb"), ("p95", "p95_ms", "#16a34a"),
    ("p99", "p99_ms", "#ea580c"), ("p99.9", "p99_9_ms", "#dc2626")], math.log10)
panel(TOPS[1], "B. Container CPU utilization (%)", [0, 20, 40, 60, 80, 100, 110], [
    ("average", "cpu_mean_pct", "#2563eb"), ("maximum", "cpu_max_pct", "#dc2626")])
panel(TOPS[2], "C. Container cgroup memory.current (MiB)", [0, 40, 80, 120, 160, 200], [
    ("average", "mem_mean_mib", "#2563eb"), ("maximum", "mem_max_mib", "#9333ea")])
out.append('<text x="785" y="1252" text-anchor="middle" class="axis">Target input rate (EPS)</text>')
out.append('</svg>')
(HERE / "fluentd_fields1k_latency_sweep.svg").write_text("\n".join(out) + "\n", encoding="utf-8")
