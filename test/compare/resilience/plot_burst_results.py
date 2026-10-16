#!/usr/bin/env python3
"""Create publication-ready SVG figures for the WCM burst experiment.

The script deliberately uses only the Python standard library so that the
figures can be regenerated on the benchmark host without plotting packages.
"""

import argparse
import csv
import html
import json
import math
from pathlib import Path


COLORS = {
    "blue": "#2878B5",
    "orange": "#E07B39",
    "green": "#4C9F70",
    "red": "#C83E4D",
    "purple": "#7851A9",
    "gray": "#667085",
    "grid": "#D9DEE7",
    "text": "#20242C",
    "burst": "#F7C948",
    "recovery": "#3CA370",
}


def read_csv(path):
    with path.open(newline="") as handle:
        return list(csv.DictReader(handle))


def esc(value):
    return html.escape(str(value), quote=True)


class SVG:
    def __init__(self, width, height, title):
        self.width, self.height = width, height
        self.parts = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}" role="img" aria-label="{esc(title)}">',
            '<rect width="100%" height="100%" fill="white"/>',
            '<style>text{font-family:Arial,Helvetica,sans-serif;fill:#20242C}'
            '.title{font-size:24px;font-weight:700}.subtitle{font-size:14px;fill:#667085}'
            '.axis{font-size:12px}.panel{font-size:16px;font-weight:700}'
            '.legend{font-size:12px}.note{font-size:12px;fill:#667085}</style>',
        ]

    def add(self, value):
        self.parts.append(value)

    def text(self, x, y, value, cls="axis", anchor="start", rotate=None, fill=None):
        attrs = f'class="{cls}" text-anchor="{anchor}"'
        if rotate is not None:
            attrs += f' transform="rotate({rotate} {x} {y})"'
        if fill:
            attrs += f' fill="{fill}"'
        self.add(f'<text x="{x:.1f}" y="{y:.1f}" {attrs}>{esc(value)}</text>')

    def line(self, x1, y1, x2, y2, color, width=1, dash=None, opacity=1.0):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(
            f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" '
            f'stroke="{color}" stroke-width="{width}" opacity="{opacity}"{extra}/>'
        )

    def rect(self, x, y, w, h, fill, opacity=1.0, stroke=None):
        extra = f' stroke="{stroke}"' if stroke else ""
        self.add(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" '
            f'fill="{fill}" opacity="{opacity}"{extra}/>'
        )

    def polyline(self, points, color, width=2, dash=None):
        if not points:
            return
        data = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(
            f'<polyline points="{data}" fill="none" stroke="{color}" '
            f'stroke-width="{width}" stroke-linejoin="round" stroke-linecap="round"{extra}/>'
        )

    def circle(self, x, y, r, fill):
        self.add(f'<circle cx="{x:.2f}" cy="{y:.2f}" r="{r}" fill="{fill}"/>')

    def save(self, path):
        self.parts.append("</svg>")
        path.write_text("\n".join(self.parts) + "\n")


def load_runs(root):
    runs = []
    for scenario in ("B1", "B2", "B3"):
        run = root / f"{scenario}_rep1"
        phases = read_csv(run / "phases.csv")
        phase_map = {p["phase"]: p for p in phases}
        analysis = json.loads((run / "burst_analysis.json").read_text())
        sink = [{k: float(v) for k, v in row.items()} for row in read_csv(run / "sink_per_sec.csv")]
        generator = []
        for row in read_csv(run / "generator_per_sec.csv"):
            generator.append({
                "unix_s": float(row["unix_s"]),
                "pending": int(row["pending"]),
                "send_eps": int(row["send_eps"]),
                "sent_total": int(row["sent_total"]),
                "offered_total": int(row["offered_total"]),
            })
        runs.append({
            "name": scenario,
            "path": run,
            "phases": phases,
            "phase_map": phase_map,
            "analysis": analysis,
            "sink": sink,
            "generator": generator,
        })
    return runs


def expected_kept(count):
    full, rem = divmod(int(count), 5)
    return full * 2 + int(rem > 1) + int(rem > 2)


def offered_at(phases, timestamp):
    for phase in phases:
        start, end = float(phase["start_unix"]), float(phase["end_unix"])
        if timestamp >= end:
            continue
        if timestamp <= start:
            return int(phase["start_count"])
        count = int(phase["start_count"]) + int((timestamp - start) * int(phase["rate_eps"]))
        return min(count, int(phase["end_count"]))
    return int(phases[-1]["end_count"])


def axes(svg, x, y, w, h, xlim, ylim, xticks, yticks, y_label, log_y=False):
    def sx(value):
        return x + (value - xlim[0]) / (xlim[1] - xlim[0]) * w

    def sy(value):
        if log_y:
            low, high = math.log10(ylim[0]), math.log10(ylim[1])
            pos = (math.log10(max(value, ylim[0])) - low) / (high - low)
        else:
            pos = (value - ylim[0]) / (ylim[1] - ylim[0])
        return y + h - pos * h

    for tick in yticks:
        yy = sy(tick)
        svg.line(x, yy, x + w, yy, COLORS["grid"], 1)
        label = f"{tick / 1000:g}k" if tick >= 1000 else f"{tick:g}"
        svg.text(x - 8, yy + 4, label, anchor="end")
    for tick in xticks:
        xx = sx(tick)
        svg.line(xx, y, xx, y + h, COLORS["grid"], 1)
        svg.text(xx, y + h + 19, f"{tick:g}", anchor="middle")
    svg.line(x, y + h, x + w, y + h, COLORS["text"], 1.2)
    svg.line(x, y, x, y + h, COLORS["text"], 1.2)
    svg.text(x + w / 2, y + h + 42, "Time relative to burst start (s)", anchor="middle")
    svg.text(x - 58, y + h / 2, y_label, anchor="middle", rotate=-90)
    return sx, sy


def shade_events(svg, sx, y, h, burst_duration, recovery_time):
    x0, x1 = sx(0), sx(burst_duration)
    svg.rect(x0, y, max(1, x1 - x0), h, COLORS["burst"], opacity=0.22)
    svg.line(x0, y, x0, y + h, COLORS["red"], 1.7, dash="6 4")
    svg.line(x1, y, x1, y + h, COLORS["orange"], 1.4, dash="5 4")
    svg.text(x0 + 4, y + h - 25, "Burst start", cls="note")
    svg.text(x1 + 4, y + h - 9, "Burst end", cls="note")
    recovery_x = sx(burst_duration + recovery_time)
    svg.line(recovery_x, y, recovery_x, y + h, COLORS["recovery"], 1.7, dash="6 4")


def figure_header(svg, title, subtitle):
    svg.text(70, 38, title, cls="title")
    svg.text(70, 62, subtitle, cls="subtitle")


def panel_legend(svg, x, y, items):
    cursor = x
    for label, color, dash in items:
        svg.line(cursor, y - 4, cursor + 28, y - 4, color, 3, dash=dash)
        svg.text(cursor + 34, y, label, cls="legend")
        cursor += 34 + len(label) * 7 + 24


def plot_latency(runs, output):
    svg = SVG(1450, 1130, "WCM burst latency over time")
    figure_header(svg, "WCM burst latency over time", "1024-byte, 36-field records; latency axis uses a logarithmic scale")
    xlim, xticks = (-60, 120), (-60, -30, 0, 30, 60, 90, 120)
    for index, run in enumerate(runs):
        top = 105 + index * 330
        burst = run["phase_map"]["burst"]
        burst_start = float(burst["start_unix"])
        duration = int(burst["duration_s"])
        analysis = run["analysis"]
        svg.text(85, top - 14, f'{run["name"]}: 33,500 EPS for {duration} s', cls="panel")
        sx, sy = axes(svg, 105, top, 1270, 245, xlim, (20, 30000), xticks,
                      (20, 50, 100, 200, 500, 1000, 5000, 10000, 30000),
                      "Latency (ms, log)", log_y=True)
        shade_events(svg, sx, top, 245, duration, analysis["recovery_time_s"])
        rows = [r for r in run["sink"] if xlim[0] <= r["unix_s"] - burst_start <= xlim[1] and r["lines"] > 0]
        for key, color in (("p50_ms", COLORS["blue"]), ("p99_ms", COLORS["red"]), ("p99_9_ms", COLORS["purple"])):
            svg.polyline([(sx(r["unix_s"] - burst_start), sy(max(r[key], 20))) for r in rows], color, 1.8)
        svg.line(sx(xlim[0]), sy(analysis["p99_recovery_limit_ms"]), sx(xlim[1]),
                 sy(analysis["p99_recovery_limit_ms"]), COLORS["gray"], 1.3, dash="3 4")
        svg.text(sx(duration + analysis["recovery_time_s"]) + 5, top + 16,
                 f'Recovered: +{duration + analysis["recovery_time_s"]:.1f}s', cls="note")
    panel_legend(svg, 105, 1090, [
        ("p50", COLORS["blue"], None), ("p99", COLORS["red"], None),
        ("p99.9", COLORS["purple"], None), ("p99 recovery limit", COLORS["gray"], "3 4"),
        ("recovery start", COLORS["green"], "6 4"), ("burst interval", COLORS["burst"], None),
    ])
    svg.save(output)


def plot_throughput(runs, output):
    svg = SVG(1450, 1130, "WCM burst throughput over time")
    figure_header(svg, "WCM burst throughput over time",
                  "Source input and filtered output; WCM retains 2 of every 5 deterministic test records")
    xlim, xticks = (-60, 120), (-60, -30, 0, 30, 60, 90, 120)
    for index, run in enumerate(runs):
        top = 105 + index * 330
        burst = run["phase_map"]["burst"]
        burst_start = float(burst["start_unix"])
        duration = int(burst["duration_s"])
        analysis = run["analysis"]
        svg.text(85, top - 14, f'{run["name"]}: 33,500 EPS for {duration} s', cls="panel")
        sx, sy = axes(svg, 105, top, 1270, 245, xlim, (0, 36000), xticks,
                      (0, 5000, 10000, 15000, 20000, 25000, 30000, 35000), "Throughput (EPS)")
        shade_events(svg, sx, top, 245, duration, analysis["recovery_time_s"])
        gen = [r for r in run["generator"] if xlim[0] <= r["unix_s"] - burst_start <= xlim[1]]
        sink = [r for r in run["sink"] if xlim[0] <= r["unix_s"] - burst_start <= xlim[1]]
        svg.polyline([(sx(r["unix_s"] - burst_start), sy(r["send_eps"])) for r in gen], COLORS["blue"], 1.8)
        svg.polyline([(sx(r["unix_s"] - burst_start), sy(r["lines"])) for r in sink], COLORS["red"], 1.8)
        expected = []
        for t in range(xlim[0], xlim[1] + 1):
            rate = 33500 if 0 <= t < duration else 11000
            expected.append((sx(t), sy(rate * 0.4)))
        svg.polyline(expected, COLORS["green"], 1.5, dash="7 4")
    panel_legend(svg, 105, 1090, [
        ("actual source send EPS", COLORS["blue"], None),
        ("actual filtered output EPS", COLORS["red"], None),
        ("expected filtered output EPS", COLORS["green"], "7 4"),
    ])
    svg.save(output)


def plot_queues(runs, output):
    svg = SVG(1450, 1130, "WCM queue accumulation and recovery")
    figure_header(svg, "WCM queue accumulation and recovery",
                  "Panel-specific linear scales show output backlog and source-side pending records")
    xlim, xticks = (-60, 120), (-60, -30, 0, 30, 60, 90, 120)
    for index, run in enumerate(runs):
        top = 105 + index * 330
        burst = run["phase_map"]["burst"]
        burst_start = float(burst["start_unix"])
        duration = int(burst["duration_s"])
        analysis = run["analysis"]
        gen = [r for r in run["generator"] if xlim[0] <= r["unix_s"] - burst_start <= xlim[1]]
        sink = [r for r in run["sink"] if xlim[0] <= r["unix_s"] - burst_start <= xlim[1]]
        backlogs = [(r["unix_s"] - burst_start,
                     max(0, expected_kept(offered_at(run["phases"], r["unix_s"])) - int(r["cumulative"])))
                    for r in sink]
        max_y = max([analysis["max_output_backlog"], analysis["max_source_pending"], 1000]) * 1.12
        step = 5000 if max_y < 50000 else (25000 if max_y < 200000 else 50000)
        top_tick = math.ceil(max_y / step) * step
        yticks = list(range(0, top_tick + 1, step))
        svg.text(85, top - 14,
                 f'{run["name"]}: max backlog {analysis["max_output_backlog"]:,}; max pending {analysis["max_source_pending"]:,}',
                 cls="panel")
        sx, sy = axes(svg, 105, top, 1270, 245, xlim, (0, top_tick), xticks, yticks, "Records")
        shade_events(svg, sx, top, 245, duration, analysis["recovery_time_s"])
        svg.polyline([(sx(t), sy(v)) for t, v in backlogs], COLORS["red"], 2)
        svg.polyline([(sx(r["unix_s"] - burst_start), sy(r["pending"])) for r in gen], COLORS["blue"], 2)
    panel_legend(svg, 105, 1090, [
        ("output backlog", COLORS["red"], None), ("source pending", COLORS["blue"], None),
    ])
    svg.save(output)


def bar_panel(svg, x, y, w, h, title, labels, values, color, unit="", formatter=None):
    svg.text(x, y - 14, title, cls="panel")
    max_v = max(values) * 1.18 if max(values) else 1
    for i in range(5):
        value = max_v * i / 4
        yy = y + h - i / 4 * h
        svg.line(x, yy, x + w, yy, COLORS["grid"], 1)
        svg.text(x - 7, yy + 4, f"{value:,.0f}", anchor="end")
    slot = w / len(values)
    for i, (label, value) in enumerate(zip(labels, values)):
        bx = x + i * slot + slot * 0.22
        bw = slot * 0.56
        bh = value / max_v * h
        svg.rect(bx, y + h - bh, bw, bh, color, opacity=0.9)
        svg.text(bx + bw / 2, y + h + 20, label, anchor="middle")
        shown = formatter(value) if formatter else f"{value:,.1f}{unit}"
        svg.text(bx + bw / 2, y + h - bh - 7, shown, cls="legend", anchor="middle")
    svg.line(x, y + h, x + w, y + h, COLORS["text"], 1.2)
    svg.line(x, y, x, y + h, COLORS["text"], 1.2)


def plot_summary(runs, output):
    svg = SVG(1450, 920, "WCM burst severity summary")
    figure_header(svg, "Effect of burst duration on WCM recovery",
                  "One run per duration; bars are descriptive results, not confidence intervals")
    labels = [f'{r["phase_map"]["burst"]["duration_s"]} s' for r in runs]
    analyses = [r["analysis"] for r in runs]
    bar_panel(svg, 105, 120, 555, 285, "Maximum post-burst p99 latency (ms)", labels,
              [a["max_post_burst_p99_ms"] for a in analyses], COLORS["red"], formatter=lambda v: f"{v:,.0f}")
    bar_panel(svg, 815, 120, 555, 285, "Recovery start time after burst end (s)", labels,
              [a["recovery_time_s"] for a in analyses], COLORS["green"], formatter=lambda v: f"{v:.1f}")
    bar_panel(svg, 105, 530, 555, 285, "Maximum output backlog (records)", labels,
              [a["max_output_backlog"] for a in analyses], COLORS["orange"], formatter=lambda v: f"{v:,.0f}")
    bar_panel(svg, 815, 530, 555, 285, "Maximum source pending (records)", labels,
              [a["max_source_pending"] for a in analyses], COLORS["blue"], formatter=lambda v: f"{v:,.0f}")
    svg.text(725, 880,
             "All runs recovered with zero missing, duplicate, corrupt, or reordered records; no OOM occurred.",
             cls="note", anchor="middle")
    svg.save(output)


def plot_resources(runs, output):
    svg = SVG(1450, 550, "WCM CPU and memory during burst experiments")
    figure_header(svg, "WCM resource usage during burst experiments",
                  "Container cgroup measurements; one CPU and 512 MiB memory limit")
    labels = [f'{r["phase_map"]["burst"]["duration_s"]} s' for r in runs]
    analyses = [r["analysis"] for r in runs]

    def grouped(x, y, w, h, title, mean, maximum, ymax, unit):
        svg.text(x, y - 14, title, cls="panel")
        for i in range(5):
            value = ymax * i / 4
            yy = y + h - i / 4 * h
            svg.line(x, yy, x + w, yy, COLORS["grid"], 1)
            svg.text(x - 7, yy + 4, f"{value:.0f}", anchor="end")
        slot = w / len(labels)
        for i, label in enumerate(labels):
            bw = slot * 0.24
            base = x + i * slot + slot * 0.22
            for j, (value, color) in enumerate(((mean[i], COLORS["blue"]), (maximum[i], COLORS["orange"]))):
                bh = value / ymax * h
                bx = base + j * bw
                svg.rect(bx, y + h - bh, bw, bh, color, opacity=0.9)
                svg.text(bx + bw / 2, y + h - bh - 6, f"{value:.1f}", cls="legend", anchor="middle")
            svg.text(base + bw, y + h + 20, label, anchor="middle")
        svg.line(x, y + h, x + w, y + h, COLORS["text"], 1.2)
        svg.line(x, y, x, y + h, COLORS["text"], 1.2)
        svg.text(x - 52, y + h / 2, unit, anchor="middle", rotate=-90)

    grouped(105, 125, 555, 300, "CPU usage", [a["cpu_mean_pct"] for a in analyses],
            [a["cpu_max_pct"] for a in analyses], 110, "%")
    grouped(815, 125, 555, 300, "Resident memory", [a["mem_mean_mib"] for a in analyses],
            [a["mem_max_mib"] for a in analyses], 200, "MiB")
    panel_legend(svg, 520, 500, [("mean", COLORS["blue"], None), ("maximum", COLORS["orange"], None)])
    svg.save(output)


def write_readme(output_dir):
    text = """# WCM burst experiment figures

All figures are vector SVG files and can be inserted directly into a thesis.

1. `01_latency_timeline.svg`: p50, p99, and p99.9 latency around burst onset. The yellow region is the nominal burst interval; the green dashed line marks the first recovered second.
2. `02_throughput_timeline.svg`: actual source send rate and filtered output rate. WCM intentionally retains two of every five deterministic records in this workload.
3. `03_queue_timeline.svg`: output backlog and source-side pending records, showing queue accumulation and drainage.
4. `04_burst_summary.svg`: relationship between burst duration, maximum latency, recovery time, and queue depth.
5. `05_resource_summary.svg`: container CPU and resident-memory mean and maximum.

Suggested thesis caveat: each scenario currently has one run. In B3 the generator's send-lag p99 was 10.58 s, so its nominal 33,500 EPS for 30 s was not a perfectly rectangular input pulse. Use B3 as evidence of recovery from accumulated pressure, and repeat it with a better-isolated generator before making precise duration-effect claims.
"""
    (output_dir / "README.md").write_text(text)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run_root", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    output = args.output or args.run_root / "figures"
    output.mkdir(parents=True, exist_ok=True)
    runs = load_runs(args.run_root)
    plot_latency(runs, output / "01_latency_timeline.svg")
    plot_throughput(runs, output / "02_throughput_timeline.svg")
    plot_queues(runs, output / "03_queue_timeline.svg")
    plot_summary(runs, output / "04_burst_summary.svg")
    plot_resources(runs, output / "05_resource_summary.svg")
    write_readme(output)
    for path in sorted(output.iterdir()):
        print(path)


if __name__ == "__main__":
    main()
