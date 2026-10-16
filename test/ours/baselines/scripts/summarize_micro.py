#!/usr/bin/env python3
"""彙整 no-WASM 微基準，並與既有 WCM／Core JSON／Core Direct 合併成五方法比較。

用法: summarize_micro.py <micro_results_dir>
輸出（寫在同一目錄）:
  native_subprocess_summary.csv   每個 method/stage/batch 的 300 筆中位數與 p95
  five_method_comparison.csv      WCM、Core JSON、Core Direct、Native、Subprocess 五欄比較
  five_method_1024.md             1024 筆（約 1 MiB）代表點表格
  charts/*.svg                    total 延遲－batch 曲線（每 stage 一張）與 1024 筆分解長條圖
中位數與 p95 的計算方式與 comparison_results/README.md 相同（合併三次 run、nearest-rank p95）。
"""
import csv
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
COMPARISON = ROOT.parent / "copy_overhead" / "comparison_results" / "wcm_core_json_direct_comparison.csv"
STAGES = ["parse", "filter", "format"]
METHODS = [
    ("wcm", "WCM Canonical ABI", "#2563eb"),
    ("core_json", "Core Wasm JSON ABI", "#ea580c"),
    ("core_direct", "Core Wasm Direct Memory", "#059669"),
    ("native", "Native Rust (in-process)", "#7c3aed"),
    ("subprocess", "Subprocess + pipe", "#db2777"),
]


def p95(values):
    s = sorted(values)
    return s[max(0, math.ceil(0.95 * len(s)) - 1)]


def load_micro(d: Path):
    groups = defaultdict(list)
    for f in sorted(d.glob("*_run*.csv")):
        if f.name.startswith("spawn"):
            continue
        with f.open() as h:
            for r in csv.DictReader(h):
                groups[(r["method"], r["stage"], int(r["batch_count"]))].append(r)
    rows = []
    for (method, stage, batch), rs in sorted(groups.items()):
        ci = [int(r["copy_in_ns"]) for r in rs]
        g = [int(r["guest_ns"]) for r in rs]
        co = [int(r["copy_out_ns"]) for r in rs]
        t = [int(r["total_ns"]) for r in rs]
        b = [a + c for a, c in zip(ci, co)]
        total_med = statistics.median(t)
        boundary_med = statistics.median(b)
        rows.append({
            "method": method, "stage": stage, "batch_count": batch,
            "logical_input_bytes": batch * 1024, "samples": len(rs),
            "copy_in_median_ns": statistics.median(ci), "guest_median_ns": statistics.median(g),
            "copy_out_median_ns": statistics.median(co), "boundary_median_ns": boundary_med,
            "total_median_ns": total_med, "total_p95_ns": p95(t),
            "boundary_share_percent": round(100.0 * boundary_med / total_med, 3) if total_med else 0.0,
        })
    return rows


def load_spawn(d: Path):
    vals = []
    for f in sorted(d.glob("spawn_run*.csv")):
        with f.open() as h:
            vals += [int(r["total_ns"]) for r in csv.DictReader(h)]
    return vals


def merge(micro_rows):
    base = {}
    with COMPARISON.open() as h:
        for r in csv.DictReader(h):
            base[(r["stage"], int(r["batch_count"]))] = r
    mi = {(r["method"], r["stage"], r["batch_count"]): r for r in micro_rows}
    out = []
    for (stage, batch), r in sorted(base.items(), key=lambda kv: (STAGES.index(kv[0][0]), kv[0][1])):
        row = {"stage": stage, "batch_count": batch, "logical_input_bytes": r["logical_input_bytes"],
               "samples_per_method": r["samples_per_method"]}
        for key in ("wcm", "core_json", "core_direct"):
            for col in ("copy_in", "guest", "copy_out", "boundary", "total"):
                row[f"{key}_{col}_median_ns"] = r[f"{key}_{col}_median_ns"]
            row[f"{key}_total_p95_ns"] = r[f"{key}_total_p95_ns"]
            row[f"{key}_boundary_share_percent"] = r[f"{key}_boundary_share_percent"]
        for key in ("native", "subprocess"):
            m = mi.get((key, stage, batch))
            for col in ("copy_in", "guest", "copy_out", "boundary", "total"):
                row[f"{key}_{col}_median_ns"] = m[f"{col}_median_ns"] if m else ""
            row[f"{key}_total_p95_ns"] = m["total_p95_ns"] if m else ""
            row[f"{key}_boundary_share_percent"] = m["boundary_share_percent"] if m else ""
        wcm_total = float(r["wcm_total_median_ns"])
        for key in ("native", "subprocess"):
            v = row[f"{key}_total_median_ns"]
            row[f"{key}_total_over_wcm"] = round(float(v) / wcm_total, 6) if v != "" else ""
        out.append(row)
    return out


# ── SVG ───────────────────────────────────────────────────────────────────────
FONT = 'font-family="Inter,Segoe UI,sans-serif"'


def fmt_time(ns):
    if ns >= 1e6:
        return f"{ns / 1e6:g} ms"
    if ns >= 1e3:
        return f"{ns / 1e3:g} µs"
    return f"{ns:g} ns"


def line_chart(stage, merged, path):
    W, H, L, T, R, B = 1200, 760, 122, 132, 1128, 656
    pts = {}
    for key, _, _ in METHODS:
        series = []
        for row in merged:
            if row["stage"] != stage:
                continue
            v = row[f"{key}_total_median_ns"]
            if v not in ("", None) and float(v) > 0:
                series.append((int(row["batch_count"]), float(v)))
        pts[key] = series
    allv = [v for s in pts.values() for _, v in s]
    lo = 10 ** math.floor(math.log10(min(allv)))
    hi = 10 ** math.ceil(math.log10(max(allv)))
    batches = sorted({b for s in pts.values() for b, _ in s})
    bx0, bx1 = math.log2(batches[0]), math.log2(batches[-1])

    def x(b):
        return L + 40 + (math.log2(b) - bx0) / (bx1 - bx0) * (R - L - 80)

    def y(v):
        return B - 32 - (math.log10(v) - math.log10(lo)) / (math.log10(hi) - math.log10(lo)) * (B - T - 64)

    o = [f'<?xml version="1.0" encoding="UTF-8"?>',
         f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="title desc">',
         f'<title id="title">{stage.title()} Total Latency — Five Methods</title>',
         f'<desc id="desc">Log-log chart of median total time by batch size for WCM, Core JSON, Core Direct, native Rust and subprocess.</desc>',
         '<rect width="100%" height="100%" fill="#f8fafc"/>',
         f'<rect x="{L}" y="{T}" width="{R - L}" height="{B - T}" rx="8" fill="#ffffff" stroke="#cbd5e1"/>',
         f'<text x="{L}" y="52" {FONT} font-size="29" font-weight="700" fill="#0f172a">{stage.title()} Total — WCM vs Core ABIs vs No-WASM Baselines</text>',
         f'<text x="{L}" y="82" {FONT} font-size="15" fill="#475569">Median of 300 samples per point (3 runs × 100) · log₂ batch axis · log₁₀ time axis · native/subprocess pinned to one core (native run, no cgroup)</text>']
    e = math.log10(lo)
    while e <= math.log10(hi) + 1e-9:
        for m in (1, 2, 5):
            v = m * 10 ** e
            if lo <= v <= hi:
                yy = y(v)
                o.append(f'<line x1="{L}" y1="{yy:.2f}" x2="{R}" y2="{yy:.2f}" stroke="#e2e8f0" stroke-width="1"/>')
                o.append(f'<text x="{L - 14}" y="{yy + 5:.2f}" text-anchor="end" {FONT} font-size="13" fill="#475569">{fmt_time(v)}</text>')
        e += 1
    for b in batches:
        xx = x(b)
        o.append(f'<line x1="{xx:.2f}" y1="{T}" x2="{xx:.2f}" y2="{B}" stroke="#f1f5f9" stroke-width="1"/>')
        o.append(f'<text x="{xx:.2f}" y="{B + 24}" text-anchor="middle" {FONT} font-size="13" fill="#475569">{b}</text>')
    o.append(f'<text x="{(L + R) / 2}" y="{B + 54}" text-anchor="middle" {FONT} font-size="15" fill="#334155">Batch size (records, ~1 KiB each)</text>')
    for i, (key, label, color) in enumerate(METHODS):
        s = pts[key]
        if not s:
            continue
        d = " ".join(f"{'M' if j == 0 else 'L'}{x(b):.2f},{y(v):.2f}" for j, (b, v) in enumerate(s))
        o.append(f'<path d="{d}" fill="none" stroke="{color}" stroke-width="3"/>')
        for b, v in s:
            o.append(f'<circle cx="{x(b):.2f}" cy="{y(v):.2f}" r="4.5" fill="{color}"/>')
        lx = L + 10 + (i % 3) * 330
        ly = 104 + (i // 3) * 0
        lx = L + i * 200
        o.append(f'<line x1="{lx}" y1="{H - 30}" x2="{lx + 28}" y2="{H - 30}" stroke="{color}" stroke-width="4"/>')
        o.append(f'<text x="{lx + 36}" y="{H - 25}" {FONT} font-size="13" fill="#0f172a">{label}</text>')
    o.append("</svg>")
    path.write_text("\n".join(o) + "\n")


def breakdown_chart(merged, path, batch=1024):
    W, H = 1500, 640
    o = [f'<?xml version="1.0" encoding="UTF-8"?>',
         f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img">',
         '<rect width="100%" height="100%" fill="#f8fafc"/>',
         f'<text x="40" y="44" {FONT} font-size="26" font-weight="700" fill="#0f172a">Copy-in / Logic / Copy-out at {batch} records (~1 MiB)</text>',
         f'<text x="40" y="70" {FONT} font-size="14" fill="#475569">Stacked medians (ms); stacked parts are component medians, so they need not sum exactly to the total median.</text>']
    colors = {"copy_in": "#94a3b8", "guest": "#2563eb", "copy_out": "#f59e0b"}
    panel_w = 460
    for si, stage in enumerate(STAGES):
        row = next(r for r in merged if r["stage"] == stage and int(r["batch_count"]) == batch)
        vals = {k: [float(row[f"{k}_{c}_median_ns"] or 0) / 1e6 for c in ("copy_in", "guest", "copy_out")] for k, _, _ in METHODS}
        vmax = max(sum(v) for v in vals.values()) * 1.15
        px = 40 + si * (panel_w + 20)
        o.append(f'<rect x="{px}" y="90" width="{panel_w}" height="480" rx="10" fill="#fff" stroke="#cbd5e1"/>')
        o.append(f'<text x="{px + panel_w / 2}" y="118" text-anchor="middle" {FONT} font-size="18" font-weight="700">{stage.title()}</text>')
        bw = 60
        for mi, (key, label, color) in enumerate(METHODS):
            bx = px + 30 + mi * 85
            ybase = 520
            acc = 0.0
            for ci, comp in enumerate(("copy_in", "guest", "copy_out")):
                v = vals[key][ci]
                h = v / vmax * 370
                o.append(f'<rect x="{bx}" y="{ybase - acc - h:.2f}" width="{bw}" height="{h:.2f}" fill="{colors[comp]}"/>')
                acc += h
            o.append(f'<text x="{bx + bw / 2}" y="{ybase - acc - 6:.2f}" text-anchor="middle" {FONT} font-size="12" fill="#0f172a">{sum(vals[key]):.2f}</text>')
            short = {"wcm": "WCM", "core_json": "Core JSON", "core_direct": "Direct", "native": "Native", "subprocess": "Subproc"}[key]
            o.append(f'<text x="{bx + bw / 2}" y="{ybase + 20}" text-anchor="middle" {FONT} font-size="12" fill="{color}">{short}</text>')
    for i, (comp, label) in enumerate((("copy_in", "copy-in"), ("guest", "logic"), ("copy_out", "copy-out"))):
        o.append(f'<rect x="{40 + i * 140}" y="598" width="18" height="18" fill="{colors[comp]}"/>')
        o.append(f'<text x="{64 + i * 140}" y="612" {FONT} font-size="14">{label}</text>')
    o.append("</svg>")
    path.write_text("\n".join(o) + "\n")


def main():
    d = Path(sys.argv[1]).resolve()
    micro = load_micro(d)
    with (d / "native_subprocess_summary.csv").open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=list(micro[0].keys()))
        w.writeheader()
        w.writerows(micro)
    merged = merge(micro)
    with (d / "five_method_comparison.csv").open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=list(merged[0].keys()))
        w.writeheader()
        w.writerows(merged)
    charts = d / "charts"
    charts.mkdir(exist_ok=True)
    for stage in STAGES:
        line_chart(stage, merged, charts / f"{stage}_total_five_methods.svg")
    breakdown_chart(merged, charts / "breakdown_1024_five_methods.svg")

    spawn = load_spawn(d)
    lines = ["# 五方法微基準：1024 筆（約 1 MiB）", "",
             "| 階段 | 方法 | Copy-in | Logic | Copy-out | Total | Total p95 | 邊界占比 |",
             "|---|---|---:|---:|---:|---:|---:|---:|"]
    for stage in STAGES:
        row = next(r for r in merged if r["stage"] == stage and int(r["batch_count"]) == 1024)
        for key, label, _ in METHODS:
            g = lambda c: float(row[f"{key}_{c}"] or 0) / 1e6
            lines.append(f"| {stage.title()} | {label} | {g('copy_in_median_ns'):.3f} ms | {g('guest_median_ns'):.3f} ms | "
                         f"{g('copy_out_median_ns'):.3f} ms | {g('total_median_ns'):.3f} ms | {g('total_p95_ns'):.3f} ms | "
                         f"{float(row[f'{key}_boundary_share_percent'] or 0):.1f}% |")
    if spawn:
        lines += ["", f"Subprocess child 冷啟動（spawn → 第一個 ping 回應）：n={len(spawn)}，"
                      f"中位數 {statistics.median(spawn) / 1e6:.3f} ms，p95 {p95(spawn) / 1e6:.3f} ms。"]
    (d / "five_method_1024.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
