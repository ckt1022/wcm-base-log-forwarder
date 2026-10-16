#!/usr/bin/env python3
"""plot.py — batch-alignment SVG 圖（無外部相依，風格同 capacity/plot_five_product_latency_comparison.py）。"""
import csv
import math
from html import escape
from pathlib import Path

HERE = Path(__file__).resolve().parent
import os  # noqa: E402
SRC = Path(os.environ.get("RESULTS_DIR", HERE))
OUT = SRC / "charts"
OUT.mkdir(exist_ok=True)

ORDER = ["wcm", "fluentd", "vector", "fbit-wasm", "fbit-lua"]
LABEL = {"wcm": "WCM", "fluentd": "Fluentd", "vector": "Vector",
         "fbit-wasm": "Fluent Bit WASM", "fbit-lua": "Fluent Bit Lua"}
COLOR = {"wcm": "#2563eb", "fluentd": "#ea580c", "vector": "#16a34a",
         "fbit-wasm": "#7c3aed", "fbit-lua": "#0891b2"}
SUB = "1024 B / 36 fields · 1 CPU · 512 MiB · single TCP conn · 30–150 s window"


def load(name):
    p = SRC / name
    if not p.exists():
        return []
    with p.open(encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


class Chart:
    def __init__(self, title, x_label, y_label, x_ticks, y_ticks, log_x=False, log_y=False,
                 sub=SUB, x_fmt=None, y_fmt=None):
        self.w, self.h = 1400, 800
        self.l, self.r, self.t, self.b = 110, 45, 88, 95
        self.pw, self.ph = self.w - self.l - self.r, self.h - self.t - self.b
        self.xt, self.yt = x_ticks, y_ticks
        self.lx, self.ly = log_x, log_y
        self.x0, self.x1 = x_ticks[0], x_ticks[-1]
        self.y0, self.y1 = y_ticks[0], y_ticks[-1]
        xf = x_fmt or (lambda v: f"{v:g}")
        yf = y_fmt or (lambda v: f"{v:g}")
        self.body = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" viewBox="0 0 {self.w} {self.h}">',
            '<rect width="100%" height="100%" fill="#ffffff"/>',
            '<style>text{font-family:"Noto Sans CJK TC","Noto Sans",sans-serif;fill:#172033}.tick{font-size:16px}.axis{font-size:18px}.note{font-size:14px;fill:#475569}.legend{font-size:16px}.title{font-size:27px;font-weight:700}.pt{font-size:13px;fill:#334155}.fail{font-size:13px;fill:#b91c1c;font-weight:700}</style>',
            f'<text x="{self.w / 2}" y="42" text-anchor="middle" class="title">{escape(title)}</text>',
            f'<text x="{self.w / 2}" y="69" text-anchor="middle" class="note">{escape(sub)}</text>',
        ]
        for tk in x_ticks:
            x = self.sx(tk)
            self.body.append(f'<line x1="{x:.1f}" y1="{self.t}" x2="{x:.1f}" y2="{self.t + self.ph}" stroke="#e2e8f0"/>')
            self.body.append(f'<text x="{x:.1f}" y="{self.t + self.ph + 30}" text-anchor="middle" class="tick">{escape(xf(tk))}</text>')
        for tk in y_ticks:
            y = self.sy(tk)
            self.body.append(f'<line x1="{self.l}" y1="{y:.1f}" x2="{self.l + self.pw}" y2="{y:.1f}" stroke="#e2e8f0"/>')
            self.body.append(f'<text x="{self.l - 14}" y="{y + 6:.1f}" text-anchor="end" class="tick">{escape(yf(tk))}</text>')
        self.body += [
            f'<line x1="{self.l}" y1="{self.t + self.ph}" x2="{self.l + self.pw}" y2="{self.t + self.ph}" stroke="#334155" stroke-width="2"/>',
            f'<line x1="{self.l}" y1="{self.t}" x2="{self.l}" y2="{self.t + self.ph}" stroke="#334155" stroke-width="2"/>',
            f'<text x="{self.l + self.pw / 2}" y="{self.h - 28}" text-anchor="middle" class="axis">{escape(x_label)}</text>',
            f'<text x="28" y="{self.t + self.ph / 2}" text-anchor="middle" transform="rotate(-90 28 {self.t + self.ph / 2})" class="axis">{escape(y_label)}</text>',
        ]
        if log_y:
            self.note("Logarithmic Y axis", corner="br", dy=0)

    def sx(self, v):
        if self.lx:
            lo, hi = math.log10(self.x0), math.log10(self.x1)
            return self.l + (math.log10(max(v, self.x0)) - lo) / (hi - lo) * self.pw
        return self.l + (v - self.x0) / (self.x1 - self.x0) * self.pw

    def sy(self, v):
        if self.ly:
            lo, hi = math.log10(self.y0), math.log10(self.y1)
            return self.t + (hi - math.log10(max(v, self.y0))) / (hi - lo) * self.ph
        return self.t + (self.y1 - v) / (self.y1 - self.y0) * self.ph

    def line(self, pts, color, dash=None, width=3, markers=True, labels=None):
        if not pts:
            return
        coords = " ".join(f"{self.sx(x):.1f},{self.sy(y):.1f}" for x, y in pts)
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.body.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="{width}"{d} stroke-linejoin="round" stroke-linecap="round"/>')
        if markers:
            for i, (x, y) in enumerate(pts):
                self.body.append(f'<circle cx="{self.sx(x):.1f}" cy="{self.sy(y):.1f}" r="4.6" fill="#fff" stroke="{color}" stroke-width="2.4"/>')
                if labels:
                    self.body.append(f'<text x="{self.sx(x) + 8:.1f}" y="{self.sy(y) - 8:.1f}" class="pt">{escape(labels[i])}</text>')

    def cross(self, x, y):
        X, Y = self.sx(x), self.sy(y)
        self.body.append(f'<line x1="{X - 7:.1f}" y1="{Y - 7:.1f}" x2="{X + 7:.1f}" y2="{Y + 7:.1f}" stroke="#dc2626" stroke-width="4"/>')
        self.body.append(f'<line x1="{X - 7:.1f}" y1="{Y + 7:.1f}" x2="{X + 7:.1f}" y2="{Y - 7:.1f}" stroke="#dc2626" stroke-width="4"/>')

    def legend(self, items, x0=None, y0=None, cols=3, colw=250):
        x0 = self.l + 22 if x0 is None else x0
        y0 = self.t + 24 if y0 is None else y0
        for i, (name, color, dash) in enumerate(items):
            x = x0 + (i % cols) * colw
            y = y0 + (i // cols) * 29
            d = f' stroke-dasharray="{dash}"' if dash else ""
            self.body.append(f'<line x1="{x}" y1="{y}" x2="{x + 31}" y2="{y}" stroke="{color}" stroke-width="4"{d}/>')
            self.body.append(f'<text x="{x + 40}" y="{y + 6}" class="legend">{escape(name)}</text>')

    def note(self, text, corner="br", dy=0):
        if corner == "br":
            self.body.append(f'<text x="{self.l + self.pw - 8}" y="{self.t + self.ph - 12 - dy}" text-anchor="end" class="note">{escape(text)}</text>')
        else:
            self.body.append(f'<text x="{self.l + 12}" y="{self.t + self.ph - 12 - dy}" class="note">{escape(text)}</text>')

    def save(self, name):
        self.body.append("</svg>")
        (OUT / name).write_text("\n".join(self.body) + "\n", encoding="utf-8")
        print("wrote", OUT / name)


def wfmt(v):
    return f"{v:g} s"


def by_product(rows, key="product"):
    d = {}
    for r in rows:
        d.setdefault(r[key], []).append(r)
    return d


def main():
    lat = load("method3_latency_5000eps.csv")
    cap = load("method3_capacity.csv")
    m2 = load("method2_batch_volume.csv")
    W_TICKS = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0]
    items = [(LABEL[p], COLOR[p], None) for p in ORDER]

    # (a) p99 vs W
    c = Chart("p99 latency vs batching window W @ 5,000 EPS (time-dominated)", "Batching window W (s, log)",
              "p99 latency (ms, log)", W_TICKS, [5, 10, 20, 50, 100, 200, 500, 1000, 2000], log_x=True, log_y=True,
              x_fmt=wfmt)
    c.line([(w, 990 * w) for w in (0.01, 1.0)], "#94a3b8", dash="8 6", width=2, markers=False)
    for p, rs in by_product(lat).items():
        pts = sorted((float(r["W_s"]), float(r["p99_ms"])) for r in rs if r["status"] == "PASS" and num(r["p99_ms"]))
        c.line(pts, COLOR[p])
        for r in rs:
            if r["status"] != "PASS" and num(r.get("p99_ms")) and num(r["p99_ms"]) <= 2000:
                c.cross(float(r["W_s"]), float(r["p99_ms"]))
    c.legend(items + [("0.99·W (pure batching wait)", "#94a3b8", "8 6")])
    c.note("Fluent Bit WASM W ≥ 0.1 s: FAILED at 5,000 EPS in the 180 s runs (p99 12–17 s, off-scale) — only W = 0.01 s is valid", dy=22)
    c.note("all values clock-step corrected (clock_drift.csv); single 180 s run per point", dy=44)
    c.save("a_p99_vs_window_5000eps.svg")

    # (d) excess latency vs W
    ex = [float(r["excess_p99_ms"]) for r in lat if r["status"] == "PASS" and num(r.get("excess_p99_ms")) is not None]
    top = max(ex + [100])
    yt = [0, 50, 100, 150, 200, 250, 300, 400, 500, 750, 1000, 1500, 2000]
    yt = [t for t in yt if t <= top * 1.15] + [next(t for t in yt if t > top * 1.15)] if top * 1.15 < 2000 else yt
    lo = min(ex + [0])
    if lo < 0:
        yt = [math.floor(lo / 50) * 50] + yt
    c = Chart("Excess latency (p99 − 0.99·W) vs W @ 5,000 EPS", "Batching window W (s, log)",
              "Excess p99 latency (ms)", W_TICKS, yt, log_x=True, x_fmt=wfmt)
    for p, rs in by_product(lat).items():
        pts = sorted((float(r["W_s"]), float(r["excess_p99_ms"])) for r in rs
                     if r["status"] == "PASS" and num(r.get("excess_p99_ms")) is not None)
        c.line(pts, COLOR[p])
    c.legend(items)
    c.note("excess = processing + transport + flush-polling cost beyond the batching wait")
    c.save("d_excess_latency_vs_window.svg")

    # (b) capacity vs W
    caps = [num(r["capacity_eps"]) for r in cap if num(r["capacity_eps"])]
    c = Chart("Max sustainable EPS vs batching window W (time-dominated)", "Batching window W (s, log)",
              "Capacity (EPS)", W_TICKS, [0, 10000, 20000, 30000, 40000, 50000, 60000, 70000, 80000][: 2 + int(max(caps + [10000]) // 10000)],
              log_x=True, x_fmt=wfmt, y_fmt=lambda v: f"{v / 1000:g}K")
    for p, rs in by_product(cap).items():
        pts = sorted((float(r["W_s"]), float(r["capacity_eps"])) for r in rs if num(r["capacity_eps"]))
        c.line(pts, COLOR[p], labels=[f"{y / 1000:g}K" for _, y in pts])
    c.legend(items)
    c.note("coarse search ≈5 %, one 60 s trial per rate, all points from this session (formal 1 s capacities are NOT plotted)")
    c.note("WCM: W ≤ 0.5 s limited by backlog (C2), W = 1 s by OOM (C4); others by generator blocking (C1)", dy=22)
    c.save("b_capacity_vs_window.svg")

    # (c) trade-off: capacity (x) vs p99@5000 (y)
    lat_idx = {(r["product"], float(r["W_s"])): r for r in lat}
    xmax = max(caps + [10000])
    xt = [0, 10000, 20000, 30000, 40000, 50000, 60000, 70000, 80000][: 2 + int(xmax // 10000)]
    c = Chart("Latency/throughput trade-off: capacity vs p99 @ 5,000 EPS", "Max sustainable EPS at window W",
              "p99 latency @ 5,000 EPS (ms, log)", xt, [5, 10, 20, 50, 100, 200, 500, 1000, 2000], log_y=True,
              x_fmt=lambda v: f"{v / 1000:g}K")
    for p, rs in by_product(cap).items():
        pts, labels = [], []
        for r in sorted(rs, key=lambda r: float(r["W_s"])):
            lr = lat_idx.get((p, float(r["W_s"])))
            if lr and num(r["capacity_eps"]) and num(lr.get("p99_ms")) and lr["status"] == "PASS":
                pts.append((float(r["capacity_eps"]), float(lr["p99_ms"])))
                labels.append(f"W={float(r['W_s']):g}")
        c.line(pts, COLOR[p], labels=labels)
    c.legend(items)
    c.note("better = lower-right; each line connects one product's W points (W = 0.01, 0.1, 0.5, 1 s)", dy=22)
    c.note("Fluent Bit WASM: only W = 0.01 s passes at 5,000 EPS", dy=44)
    c.save("c_tradeoff_capacity_vs_p99.svg")

    # (e) method 2: p99 vs rate at 1 MiB batch volume
    c = Chart("Method 2: p99 vs rate at equal batch volume (1 MiB of input per batch)", "Target rate (EPS)",
              "p99 latency (ms, log)", [0, 5000, 10000, 15000, 20000, 25000], [20, 50, 100, 200, 500, 1000, 2000],
              log_y=True, x_fmt=lambda v: f"{v / 1000:g}K")
    weq = [(r_, 0.99 * 1048576 / (r_ * 1024) * 1000) for r_ in range(4000, 25001, 1000)]
    c.line(weq, "#94a3b8", dash="8 6", width=2, markers=False)
    for p, rs in by_product([r for r in m2 if r["variant"] == "input_1MiB"]).items():
        pts = sorted((float(r["rate_eps"]), float(r["p99_ms"])) for r in rs if r["status"] == "PASS" and num(r["p99_ms"]))
        c.line(pts, COLOR[p])
        for r in rs:
            if r["status"] != "PASS" and num(r.get("p99_ms")):
                c.cross(float(r["rate_eps"]), float(r["p99_ms"]))
    for p, rs in by_product([r for r in m2 if r["variant"] == "output_1MiB_formal"]).items():
        pts = sorted((float(r["rate_eps"]), float(r["p99_ms"])) for r in rs if r["status"] == "PASS" and num(r["p99_ms"]))
        c.line(pts, COLOR[p], dash="4 5", width=2)
    b1 = [r for r in load("method1_time_aligned.csv") if r["dut"] == "wcm-b1" and r["status"] == "PASS"
          and 5000 <= float(r["rate_eps"]) <= 20000]
    c.line(sorted((float(r["rate_eps"]), float(r["p99_ms"])) for r in b1), COLOR["wcm"], dash="2 5", width=2)
    c.legend(items + [("0.99·W_eq", "#94a3b8", "8 6"), ("dashed = formal 1 MiB *output* batch", "#475569", "4 5"),
                      ("dotted blue = formal wcm-b1 (same config)", COLOR["wcm"], "2 5")], colw=250)
    c.note("solid = 1 MiB of input (≈410 output records) per batch, this session, clock-step corrected; red × = FAIL", dy=22)
    c.save("e_method2_p99_vs_rate.svg")


    # (f) method 1: WCM tw128 (1 s time window) vs wcm-b1 vs 1 s-timer peers
    m1 = load("method1_time_aligned.csv")
    c = Chart("Method 1: WCM with a 1 s time window (tw128) vs wcm-b1 — latency vs rate", "Target rate (EPS)",
              "latency (ms, log)", [0, 2500, 5000, 7500, 10000, 12500, 15000, 17500, 20000, 22500],
              [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000, 50000], log_y=True, x_fmt=lambda v: f"{v / 1000:g}K",
              sub=SUB + " · tw128 corrected by stepdetect.py")
    c.line([(0, 990), (22500, 990)], "#94a3b8", dash="8 6", width=2, markers=False)

    def ser(dut, key):
        return sorted((float(r["rate_eps"]), float(r[key])) for r in m1
                      if r["dut"] == dut and r["status"] == "PASS" and num(r.get(key)))
    c.line(ser("wcm-tw128", "p99_ms"), COLOR["wcm"], width=3.5)
    c.line(ser("wcm-tw128", "p50_ms"), COLOR["wcm"], dash="10 6", width=2.5)
    c.line(ser("wcm-tw128", "raw_p99_ms"), "#cbd5e1", dash="3 4", width=2)
    c.line(ser("wcm-b1", "p99_ms"), "#1e3a8a", width=2.5)
    c.line(ser("wcm-b1", "p50_ms"), "#1e3a8a", dash="10 6", width=2)
    c.line(ser("fbit-lua", "p99_ms"), COLOR["fbit-lua"], width=2)
    c.line(ser("fbit-wasm", "p99_ms"), COLOR["fbit-wasm"], width=2)
    oom = [float(r["rate_eps"]) for r in m1 if r["dut"] == "wcm-tw128" and r["status"].startswith("OOM")]
    for x in oom:
        c.cross(x, 3000)
        c.body.append(f'<text x="{c.sx(x) - 10:.1f}" y="{c.sy(3000) - 14:.1f}" text-anchor="end" class="fail">tw128 OOM-killed at 22.25K</text>')
    c.legend([("WCM tw128 p99 (1 s window)", COLOR["wcm"], None), ("WCM tw128 p50", COLOR["wcm"], "10 6"),
              ("WCM tw128 p99 RAW (clock-contaminated)", "#cbd5e1", "3 4"), ("WCM b1 p99 (1 MiB input)", "#1e3a8a", None),
              ("WCM b1 p50", "#1e3a8a", "10 6"), ("Fluent Bit Lua p99 (Flush=1)", COLOR["fbit-lua"], None),
              ("Fluent Bit WASM p99 (Flush=1)", COLOR["fbit-wasm"], None), ("0.99 s", "#94a3b8", "8 6")],
             cols=3, colw=410, y0=c.t + 20)
    c.note("tw128 excess over the 1 s window grows with rate (batch is processed after the window closes)", dy=22)
    c.save("f_method1_tw128_vs_b1.svg")


if __name__ == "__main__":
    main()
