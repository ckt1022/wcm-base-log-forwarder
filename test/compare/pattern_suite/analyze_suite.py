#!/usr/bin/env python3
"""analyze_suite.py — pattern_suite 最終分析：tidy CSV + SVG 圖（只讀 runs/，不改原始資料）。

用法（在本目錄）：
    python3 collect_suite.py runs     # 既有的原始整理（latency/capacity/correctness/fault/loadcheck.csv）
    python3 analyze_suite.py          # 本程式 → analysis/*.csv、analysis/charts/*.svg

延遲一律重新校正（fix_latency.py；處理 04:04 之後的「向後」時鐘階梯，舊的 latency_stats_30_150.txt 在這些 run
的 p99 被污染）。CPU 以 stats.csv 的 cpu_time 與「去除時鐘階梯後」的取樣時間重算。
無效 run：`*_INVALID_*` 目錄（host 睡眠，06:15–09:26）一律排除；其餘 run 以 gen_epoch／clock_drift 檢查是否跨過
暫停期（見 analysis/run_validity.csv）。
"""
import csv
import datetime as dt
import json
import math
import re
import statistics as st
import sys
from html import escape
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import fix_latency  # noqa: E402

RUNS = HERE / "runs"
SV = HERE.parent / "streaming_validation" / "runs"
OUT = HERE / "analysis"
CH = OUT / "charts"
CH.mkdir(parents=True, exist_ok=True)
TZ = dt.timezone(dt.timedelta(hours=8))
PAUSE = (dt.datetime(2026, 10, 15, 6, 18, 21, tzinfo=TZ), dt.datetime(2026, 10, 15, 9, 26, 53, tzinfo=TZ))
RESUME = PAUSE[1]
BACKSTEP_START = dt.datetime(2026, 10, 15, 4, 4, tzinfo=TZ)

ORDER = ["fbit-lua", "fluentd", "vector", "fbit-wasm", "wcm-batch", "wcm-stream"]
LABEL = {"fbit-lua": "Fluent Bit Lua", "fluentd": "Fluentd", "vector": "Vector", "fbit-wasm": "Fluent Bit WASM",
         "wcm-batch": "WCM-batch", "wcm-stream": "WCM-stream (k=256)"}
# 已用 dataviz validate_palette.js 檢查（light：全部 PASS；綠／橘 CVD ΔE 6.9 → 以線型＋直接標籤作第二編碼）
COLOR = {"fbit-lua": "#0891b2", "fluentd": "#ea580c", "vector": "#16a34a", "fbit-wasm": "#7c3aed",
         "wcm-batch": "#db2777", "wcm-stream": "#2563eb"}
DASH = {"fbit-lua": "2 5", "fluentd": "10 5", "vector": "4 4", "fbit-wasm": "12 4 2 4", "wcm-batch": "8 5", "wcm-stream": None}
WS = [0.01, 0.1, 0.5, 1.0]
SUB = "1024 B / 36 fields · 1 CPU · 512 MiB · single TCP conn · 30–150 s window · pattern_suite 2026-10-14/15"


# ───────────────────────── helpers ─────────────────────────
def read(p):
    try:
        return Path(p).read_text(errors="replace")
    except OSError:
        return ""


def write(name, rows, keys=None):
    if not rows:
        return
    if keys is None:
        keys = []
        for r in rows:
            for k in r:
                if k not in keys:
                    keys.append(k)
    with (OUT / name).open("w", newline="", encoding="utf-8") as h:
        w = csv.DictWriter(h, fieldnames=keys, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)
    print("wrote", OUT / name, len(rows))


def med(v):
    v = [x for x in v if x is not None]
    return round(st.median(v), 3) if v else None


def mn(v):
    v = [x for x in v if x is not None]
    return round(min(v), 3) if v else None


def mx(v):
    v = [x for x in v if x is not None]
    return round(max(v), 3) if v else None


def ts(epoch):
    return dt.datetime.fromtimestamp(epoch, TZ)


def fnum(x):
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def cpu_from_stats(d, lo=30, hi=150):
    """Δcpu_time / Δ(去階梯的取樣時間)。stats.csv 的 elapsed_s 是 wall 差：向後階梯時出現負差，向前時出現 >1.5 s 差。"""
    p = Path(d) / "stats.csv"
    if not p.exists():
        return None, None
    rows = list(csv.DictReader(p.open()))
    if len(rows) < 3:
        return None, None
    e = [float(r["elapsed_s"]) for r in rows]
    diffs = [b - a for a, b in zip(e, e[1:])]
    cad = st.median(diffs)
    mono, cum = [e[0]], 0.0
    for i, dd in enumerate(diffs, 1):
        if dd < 0.5 * cad or dd > 1.5 * cad:
            cum += cad - dd
        mono.append(e[i] + cum)
    win = [(m, float(r["cpu_time_s"]), float(r["rss_mb"])) for m, r in zip(mono, rows) if lo <= m <= hi]
    if len(win) < 2:
        return None, None
    cpu = (win[-1][1] - win[0][1]) / (win[-1][0] - win[0][0]) * 100
    return round(cpu, 3), round(st.fmean(w[2] for w in win), 3)


def parse_tag(dut, tag):
    """→ (product, W, k, variant, rep)"""
    rep = 1
    m = re.search(r"_r(\d+)$", tag)
    if m:
        rep = int(m.group(1))
        tag = tag[: m.start()]
    W = fnum((re.search(r"W([\d.]+)", tag) or [None, None])[1])
    k, variant = None, ""
    if dut != "wcm":
        return dut, W, None, "", rep
    if tag.startswith("batch_"):
        return "wcm-batch", W, None, "", rep
    m = re.match(r"stream_k(\d+)_W[\d.]+(?:_(\w+))?", tag)
    if m:
        k = int(m.group(1))
        variant = m.group(2) or ""
        prod = "wcm-stream" if k == 256 else f"wcm-stream-k{k}"
        return prod, W, k, variant, rep
    return "wcm-" + tag, W, None, "", rep


def regime(t):
    if t >= RESUME:
        return "post_resume"
    if t >= BACKSTEP_START:
        return "backstep_pre_pause"
    return "clean"


# ───────────────────────── 1. 延遲（全部重新校正） ─────────────────────────
def latency_rows():
    rows = []
    dirs = sorted(RUNS.glob("*/lat_rate_*")) + sorted(SV.glob("*/lat_rate_*"))
    for d in dirs:
        src = "streaming_validation" if SV in d.parents else "pattern_suite"
        m = re.match(r"lat_rate_(\d+)_(.+)", d.name)
        rate, tag = int(m.group(1)), m.group(2)
        invalid = "INVALID" in tag
        clean_tag = tag.split("_INVALID")[0]
        dut = d.parent.name
        if src == "streaming_validation":
            prod = {"stream_W0.01": "sv-wcm-stream-k64", "stream_W0.1": "sv-wcm-stream-k64",
                    "stream_W0.5": "sv-wcm-stream-k64", "stream_W1": "sv-wcm-stream-k64",
                    "ctrl_batch_W0.1": "sv-wcm-batch-ctrl", "ctrl_W0.1": "sv-fbit-lua-ctrl"}.get(clean_tag, "sv-" + clean_tag)
            W = fnum((re.search(r"W([\d.]+)", clean_tag) or [None, None])[1])
            k, variant, rep = (64 if "stream" in clean_tag else None), "", 1
        else:
            prod, W, k, variant, rep = parse_tag(dut, clean_tag)
        gs, ge = map(float, read(d / "gen_epoch.txt").split())
        t0 = ts(gs)
        vd = read(d / "verdict.txt").split()
        verdict = vd[0] if vd else ""
        old = read(d / "latency_stats_30_150.txt")
        old_p99 = fnum((re.search(r"p99\s+=\s+([-\d.]+)", old) or [None, None])[1])
        fx = fix_latency.fix(d)
        cpu, rss = cpu_from_stats(d)
        # 暫停檢查：gen 期間或 clock_drift 中出現 > 60 s 的 ΔD
        drift = list(csv.DictReader((d / "clock_drift.csv").open()))
        big = [abs(float(b["drift"]) - float(a["drift"])) for a, b in zip(drift, drift[1:])]
        spans_pause = (max(big) if big else 0) > 60 or (ts(ge) > PAUSE[0] and t0 < PAUSE[1])
        valid, reason = True, ""
        if invalid or spans_pause:
            valid, reason = False, "host paused during run (06:15–09:26), moved to *_INVALID_host_paused"
        elif verdict != "PASS":
            valid, reason = False, "FAIL: DUT overloaded at this rate (gen blocked / backlog) — latency not meaningful"
        gl = read(d / "verdict.txt")
        rows.append({
            "source": src, "dut": dut, "product": prod, "W_s": W, "k": k, "variant": variant, "rate_eps": rate, "rep": rep,
            "dir": str(d.relative_to(HERE.parent)), "gen_start": t0.strftime("%m-%d %H:%M:%S"), "regime": regime(t0) if src == "pattern_suite" else "forward_step_session",
            "verdict": verdict, "valid": valid, "exclude_reason": reason,
            "gen_avg_eps": fnum((re.search(r"gen_avg=(\d+)", gl) or [None, None])[1]),
            "n_clock_steps": fx["n_steps"], "mean_step_s": round(st.fmean(fx["step_sizes"]), 3) if fx["step_sizes"] else 0,
            "p50_ms": fx["p50"], "p99_ms": fx["p99"], "p999_ms": fx["p999"],
            "excess_p99_ms": round(fx["p99"] - 990 * W, 3) if W is not None else None,
            "p99_excl_post_step_3s_ms": fx["p99_excl_post_step_3s"],
            "p99_old_corrected_ms": old_p99, "p99_raw_ms": fx["raw_p99"],
            "flat_range_ms": fx["flat_range_ms"], "fix_issues": fx["issues"],
            "subwin_p99_ms": " ".join(str(s[2]) for s in fx["subwindows"]),
            "cpu_mean_pct": cpu, "rss_mean_mib": rss,
        })
    return rows


# ───────────────────────── 2. 容量 ─────────────────────────
def search_times():
    t = {}
    for log in ("run_suite.log",):
        for line in read(HERE / log).splitlines():
            m = re.match(r"\[(\S+)\] search (\S+) (\S+) start=", line)
            if m:
                t[(m.group(2), m.group(3))] = dt.datetime.fromisoformat(m.group(1))
    return t


def capacity_rows():
    times = search_times()
    rows = []
    for f in sorted(RUNS.glob("*/search_*.txt")):
        dut, tag = f.parent.name, f.stem[len("search_"):]
        txt = read(f)
        last = [l for l in txt.splitlines() if l.startswith("CAPACITY")][-1]
        cap = (re.search(r"eps=(\S+)", last) or [None, ""])[1]
        ff = (re.search(r"first_fail=(\S+)", last) or [None, ""])[1]
        prod, W, k, variant, rep = parse_tag(dut, tag)
        t0 = times.get((dut, tag))
        trials = re.findall(r"^trial rate=(\d+) (PASS|FAIL) :: (.*)$", txt, re.M)
        # 飽和吞吐（只對無法 bracket 的 k=1 有意義）：win_out_eps / 0.4（filter 留下 40%）
        sat = [float(re.search(r"win_out_eps=(\d+)", t[2]).group(1)) / 0.4 for t in trials if "win_out_eps" in t[2]]
        rows.append({
            "product": prod, "dut": dut, "tag": tag, "W_s": W, "k": k, "variant": variant, "rep": rep,
            "search_start": t0.strftime("%m-%d %H:%M") if t0 else "", "regime": regime(t0) if t0 else "",
            "capacity_eps": int(cap) if cap.isdigit() else None, "capacity_raw": cap, "first_fail_eps": ff,
            "trials": len(trials), "trial_rates": " ".join(f"{r}{'P' if v == 'PASS' else 'F'}" for r, v, _ in trials),
            "saturated_input_eps_median": round(st.median(sat)) if (sat and not cap.isdigit()) else None,
        })
    return rows


# ───────────────────────── SVG ─────────────────────────
class Chart:
    """風格同 ../batch_alignment/plot.py（1400×800、白底、Noto Sans CJK TC、淡灰格線）。"""

    def __init__(self, title, x_label, y_label, x_ticks, y_ticks, log_x=False, log_y=False,
                 sub=SUB, x_fmt=None, y_fmt=None, w=1400, h=800):
        self.w, self.h = w, h
        self.l, self.r, self.t, self.b = 110, 45, 88, 95
        self.pw, self.ph = self.w - self.l - self.r, self.h - self.t - self.b
        self.lx, self.ly = log_x, log_y
        if x_ticks is None:
            x_ticks_, y_ticks_ = [0, 1], [0, 1]
        else:
            x_ticks_, y_ticks_ = x_ticks, y_ticks
        self.x0, self.x1 = x_ticks_[0], x_ticks_[-1]
        self.y0, self.y1 = y_ticks_[0], y_ticks_[-1]
        xf = x_fmt or (lambda v: f"{v:g}")
        yf = y_fmt or (lambda v: f"{v:g}")
        self.body = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" viewBox="0 0 {self.w} {self.h}">',
            f'<title>{escape(title)}</title>',
            '<rect width="100%" height="100%" fill="#ffffff"/>',
            '<style>text{font-family:"Noto Sans CJK TC","Noto Sans",sans-serif;fill:#172033}.tick{font-size:16px}.axis{font-size:18px}.note{font-size:14px;fill:#475569}.legend{font-size:16px}.title{font-size:27px;font-weight:700}.pt{font-size:13px;fill:#334155}.fail{font-size:13px;fill:#b91c1c;font-weight:700}.cell{font-size:15px}.hdr{font-size:15px;font-weight:700}</style>',
            f'<text x="{self.w / 2}" y="42" text-anchor="middle" class="title">{escape(title)}</text>',
            f'<text x="{self.w / 2}" y="69" text-anchor="middle" class="note">{escape(sub)}</text>',
        ]
        if x_ticks is None:
            return
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

    def line(self, pts, color, dash=None, width=2.5, markers=True, labels=None, hollow=False, tip=None):
        if not pts:
            return
        coords = " ".join(f"{self.sx(x):.1f},{self.sy(y):.1f}" for x, y in pts)
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.body.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="{width}"{d} stroke-linejoin="round" stroke-linecap="round"/>')
        if markers:
            for i, (x, y) in enumerate(pts):
                t = f"<title>{escape(tip[i])}</title>" if tip else ""
                fill = "#fff" if hollow or True else color
                self.body.append(f'<circle cx="{self.sx(x):.1f}" cy="{self.sy(y):.1f}" r="5" fill="{fill}" stroke="{color}" stroke-width="2.4">{t}</circle>')
                if labels and labels[i]:
                    self.body.append(f'<text x="{self.sx(x) + 8:.1f}" y="{self.sy(y) - 9:.1f}" class="pt">{escape(labels[i])}</text>')

    def dot(self, x, y, color, r=4, tip=None, filled=True):
        t = f"<title>{escape(tip)}</title>" if tip else ""
        f = color if filled else "#fff"
        self.body.append(f'<circle cx="{self.sx(x):.1f}" cy="{self.sy(y):.1f}" r="{r}" fill="{f}" stroke="#fff" stroke-width="1.5" opacity="0.85">{t}</circle>')

    def vbar(self, x, ylo, yhi, color, w=6):
        X = self.sx(x)
        self.body.append(f'<line x1="{X:.1f}" y1="{self.sy(ylo):.1f}" x2="{X:.1f}" y2="{self.sy(yhi):.1f}" stroke="{color}" stroke-width="2"/>')
        for yy in (ylo, yhi):
            self.body.append(f'<line x1="{X - w:.1f}" y1="{self.sy(yy):.1f}" x2="{X + w:.1f}" y2="{self.sy(yy):.1f}" stroke="{color}" stroke-width="2"/>')

    def hbar(self, xlo, xhi, y, color, w=6):
        Y = self.sy(y)
        self.body.append(f'<line x1="{self.sx(xlo):.1f}" y1="{Y:.1f}" x2="{self.sx(xhi):.1f}" y2="{Y:.1f}" stroke="{color}" stroke-width="2"/>')
        for xx in (xlo, xhi):
            self.body.append(f'<line x1="{self.sx(xx):.1f}" y1="{Y - w:.1f}" x2="{self.sx(xx):.1f}" y2="{Y + w:.1f}" stroke="{color}" stroke-width="2"/>')

    def rect(self, x0, x1, y0, y1, color, tip=None, label=None):
        X0, X1 = self.sx(x0), self.sx(x1)
        Y0, Y1 = self.sy(y1), self.sy(y0)
        t = f"<title>{escape(tip)}</title>" if tip else ""
        self.body.append(f'<rect x="{X0:.1f}" y="{Y0:.1f}" width="{X1 - X0:.1f}" height="{max(Y1 - Y0, 0):.1f}" rx="4" fill="{color}">{t}</rect>')
        if label:
            self.body.append(f'<text x="{(X0 + X1) / 2:.1f}" y="{Y0 - 8:.1f}" text-anchor="middle" class="pt">{escape(label)}</text>')

    def cross(self, x, y):
        X, Y = self.sx(x), self.sy(y)
        self.body.append(f'<line x1="{X - 7:.1f}" y1="{Y - 7:.1f}" x2="{X + 7:.1f}" y2="{Y + 7:.1f}" stroke="#dc2626" stroke-width="4"/>')
        self.body.append(f'<line x1="{X - 7:.1f}" y1="{Y + 7:.1f}" x2="{X + 7:.1f}" y2="{Y - 7:.1f}" stroke="#dc2626" stroke-width="4"/>')

    def text(self, x, y, s, cls="pt", anchor="start"):
        self.body.append(f'<text x="{self.sx(x):.1f}" y="{self.sy(y):.1f}" text-anchor="{anchor}" class="{cls}">{escape(s)}</text>')

    def legend(self, items, x0=None, y0=None, cols=3, colw=260):
        x0 = self.l + 22 if x0 is None else x0
        y0 = self.t + 24 if y0 is None else y0
        for i, (name, color, dash) in enumerate(items):
            x = x0 + (i % cols) * colw
            y = y0 + (i // cols) * 29
            d = f' stroke-dasharray="{dash}"' if dash else ""
            self.body.append(f'<line x1="{x}" y1="{y}" x2="{x + 31}" y2="{y}" stroke="{color}" stroke-width="4"{d}/>')
            self.body.append(f'<text x="{x + 40}" y="{y + 6}" class="legend">{escape(name)}</text>')

    def note(self, text, dy=0, corner="br"):
        if corner == "br":
            self.body.append(f'<text x="{self.l + self.pw - 8}" y="{self.t + self.ph - 12 - dy}" text-anchor="end" class="note">{escape(text)}</text>')
        else:
            self.body.append(f'<text x="{self.l + 12}" y="{self.t + self.ph - 12 - dy}" class="note">{escape(text)}</text>')

    def save(self, name):
        self.body.append("</svg>")
        (CH / name).write_text("\n".join(self.body) + "\n", encoding="utf-8")
        print("wrote", CH / name)


def wfmt(v):
    return f"{v:g} s"


def kfmt(v):
    return f"{v / 1000:g}K"


# ───────────────────────── main ─────────────────────────
def main():
    # ---------- latency ----------
    lat = latency_rows()
    write("latency_runs_corrected.csv", lat)

    # run validity：runs.log 的耗時 + gen 期間
    val = []
    for line in read(HERE / "logs/runs.log").splitlines():
        m = re.match(r"(\S+) rc=(\d+) dur=(\d+)s dut=(\S+) rate=(\d+) mode=(\S+) tag=(\S+) :: (\S+)", line)
        if not m:
            continue
        end = dt.datetime.fromisoformat(m.group(1))
        dur = int(m.group(3))
        start = end - dt.timedelta(seconds=dur)
        mode = m.group(6)
        normal = (80, 130) if mode == "cap" else (190, 240)
        overl = start < PAUSE[1] and end > PAUSE[0]
        flag = "OVERLAPS_PAUSE" if overl else ("DURATION_ANOMALY" if not normal[0] <= dur <= normal[1] else "ok")
        val.append({"end": m.group(1), "dut": m.group(4), "rate": m.group(5), "mode": mode, "tag": m.group(7),
                    "dur_s": dur, "verdict": m.group(8), "check": flag})
    write("run_validity.csv", val)

    # ---------- fix_latency 交叉驗證：在「向前」階梯的舊 run 上必須重現 correct_latency.py ----------
    vrows = []
    for d in sorted(list((HERE.parent / "batch_alignment/runs").glob("*/lat_rate_*")) + list(SV.glob("*/lat_rate_*"))):
        if not (d / "clock_drift.csv").exists() or not (d / "latency_stats_30_150.txt").exists():
            continue
        old = fnum((re.search(r"p99\s+=\s+([-\d.]+)", read(d / "latency_stats_30_150.txt")) or [None, None])[1])
        f = fix_latency.fix(d)
        vd = read(d / "verdict.txt").split()
        vrows.append({"dir": str(d.relative_to(HERE.parent)), "verdict": vd[0] if vd else "", "steps": " ".join(f"{x:+.2f}" for x in f["step_sizes"]),
                      "p99_correct_latency_py": old, "p99_fix_latency_py": f["p99"], "abs_diff_ms": round(abs(f["p99"] - old), 3)})
    write("fix_latency_validation.csv", vrows)

    # ---------- capacity ----------
    cap = capacity_rows()
    write("capacity_searches.csv", cap)

    # ---------- method 3 summary ----------
    m3 = []
    for p in ORDER:
        for W in WS:
            L = [r for r in lat if r["source"] == "pattern_suite" and r["product"] == p and r["W_s"] == W
                 and r["variant"] == "" and r["rate_eps"] == 5000 and "INVALID" not in r["dir"]]
            Lv = [r for r in L if r["valid"]]
            rate_used = 5000
            if p == "fbit-wasm" and not Lv:
                L4 = [r for r in lat if r["product"] == p and r["W_s"] == W and r["rate_eps"] == 4000 and r["valid"]]
                if L4:
                    Lv, rate_used = L4, 4000
            C = [r for r in cap if r["product"] == p and r["W_s"] == W and r["capacity_eps"]]
            p99 = [r["p99_ms"] for r in Lv]
            p99s = [r["p99_excl_post_step_3s_ms"] for r in Lv]
            pre = [r["p99_ms"] for r in Lv if r["regime"] != "post_resume"]
            caps = [r["capacity_eps"] for r in C]
            m3.append({
                "product": p, "label": LABEL[p], "W_s": W,
                "lat_rate_eps": rate_used if Lv else "", "lat_reps_valid": len(Lv), "lat_reps_total": len(L),
                "lat_fail_reps_5000": sum(1 for r in L if r["verdict"] != "PASS"),
                "p50_median_ms": med([r["p50_ms"] for r in Lv]),
                "p99_median_ms": med(p99), "p99_min_ms": mn(p99), "p99_max_ms": mx(p99),
                "p99_pre_resume_median_ms": med(pre), "p99_pre_resume_n": len(pre),
                "p99s_median_ms": med(p99s), "p99s_min_ms": mn(p99s), "p99s_max_ms": mx(p99s),
                "excess_s_median_ms": round(med(p99s) - 990 * W, 3) if p99s else None,
                "excess_s_min_ms": round(mn(p99s) - 990 * W, 3) if p99s else None,
                "excess_s_max_ms": round(mx(p99s) - 990 * W, 3) if p99s else None,
                "excess_median_ms": round(med(p99) - 990 * W, 3) if p99 else None,
                "excess_min_ms": round(mn(p99) - 990 * W, 3) if p99 else None,
                "excess_max_ms": round(mx(p99) - 990 * W, 3) if p99 else None,
                "cpu_median_pct": med([r["cpu_mean_pct"] for r in Lv]),
                "cpu_pre_resume_median_pct": med([r["cpu_mean_pct"] for r in Lv if r["regime"] != "post_resume"]),
                "rss_median_mib": med([r["rss_mean_mib"] for r in Lv]),
                "fail_p99_5000_ms": " ".join(str(round(r["p99_ms"])) for r in L if r["verdict"] != "PASS"),
                "cap_reps": len(caps), "cap_median_eps": med(caps), "cap_min_eps": mn(caps), "cap_max_eps": mx(caps),
                "cap_values": " ".join(f"{r['capacity_eps']}({r['search_start']})" for r in C),
                "p99_values_ms": " ".join(f"r{r['rep']}:{r['p99_ms']:.0f}[{r['regime'][:4]}]" for r in sorted(Lv, key=lambda r: r["rep"])),
            })
    write("m3_tradeoff.csv", m3)

    # ---------- P2 k sweep ----------
    ks = []
    for k in (1, 16, 64, 256):
        C = [r for r in cap if r["k"] == k and r["W_s"] == 0.1 and r["variant"] == ""]
        L = [r for r in lat if r["source"] == "pattern_suite" and r["k"] == k and r["W_s"] == 0.1 and r["variant"] == "" and r["rate_eps"] == 5000]
        L1 = [r for r in L if r["rep"] == 1]
        caps = [r["capacity_eps"] for r in C if r["capacity_eps"]]
        sat = [r["saturated_input_eps_median"] for r in C if r["saturated_input_eps_median"]]
        ks.append({
            "k": k, "cap_reps": len(C), "cap_values": " ".join(f"{r['capacity_raw']}({r['search_start']})" for r in C),
            "cap_median_eps": med(caps), "cap_min_eps": mn(caps), "cap_max_eps": mx(caps),
            "first_fail": " ".join(r["first_fail_eps"] for r in C),
            "k1_saturated_input_eps_median": med(sat),
            "lat5000_r1_verdict": L1[0]["verdict"] if L1 else "", "lat5000_r1_gen_avg_eps": L1[0]["gen_avg_eps"] if L1 else None,
            "lat5000_r1_p50_ms": L1[0]["p50_ms"] if L1 else None, "lat5000_r1_p99_ms": L1[0]["p99_ms"] if L1 else None,
            "lat5000_r1_cpu_pct": L1[0]["cpu_mean_pct"] if L1 else None,
            "lat5000_all_reps_p99_ms": " ".join(f"{r['p99_ms']:.0f}" for r in L),
        })
    write("p2_k_sweep.csv", ks)

    # ---------- P1a bisection ----------
    corr = list(csv.DictReader((OUT / "correctness.csv").open()))
    b = []
    base_cpu = {}
    for mode in ("batch", "stream"):
        for rate in ("0", "0.1", "1", "5", "20"):
            R = [r for r in corr if r["mode"] == mode and r["malformed_pct"] == rate]
            cpu = [float(r["cpu_mean_pct"]) for r in R]
            calls = [int(r["isolation_parse_calls"]) for r in R]
            mal = int(R[0]["malformed_input"])
            if rate == "0":
                base_cpu[mode] = med(cpu)
            b.append({
                "mode": mode, "malformed_pct": float(rate), "reps": len(R), "pass": sum(r["verdict"] == "PASS" for r in R),
                "malformed_input": mal, "expected": R[0]["expected"],
                "delivered_pct_min": round(100 * min(float(r["delivered_over_expected"]) for r in R), 3),
                "rejected_eq_malformed": all(int(r["rejected_lines"]) == mal for r in R),
                "isolated_batches_median": med([int(r["isolated_batches"]) for r in R]),
                "isolation_calls_median": med(calls), "isolation_calls_min": mn(calls), "isolation_calls_max": mx(calls),
                "calls_per_malformed": round(med(calls) / mal, 2) if mal else None,
                "cpu_median_pct": med(cpu), "cpu_min_pct": mn(cpu), "cpu_max_pct": mx(cpu),
                "cpu_delta_vs_0pct_pp": round(med(cpu) - base_cpu[mode], 2),
                "rss_mean_median_mib": med([float(r["rss_mean_mib"]) for r in R]),
                "rss_max_median_mib": med([float(r["rss_max_mib"]) for r in R]),
            })
    write("p1_bisection.csv", b)

    # ---------- P1b timeout / recovery ----------
    def fsum(group):
        return list(csv.DictReader((RUNS / "fault" / group / "summary.csv").open()))
    rec = []
    for group, T, pat in (("p1_timeout", 1000, "loop_t1000"), ("p4_watchdog", 2000, "loop_t2000"),
                          ("p1_timeout", 5000, "loop_t5000"), ("p1_timeout", 30000, "loop_t30000"),
                          ("p4_reuse", 2000, "loop_noreuse")):
        R = [r for r in fsum(group) if r["case"].startswith(pat)]
        rs = [float(r["recovery_sec"]) for r in R]
        rec.append({
            "config": pat.replace("loop_", ""), "stage_timeout_ms": T, "reuse": "off" if "noreuse" in pat else "on",
            "reps": len(R), "host_alive": sum(r["host_survived"] == "alive" for r in R),
            "records_lost_values": " ".join(r["records_lost"] for r in R),
            "stall_zero_sec_values": " ".join(r["stall_zero_sec"] for r in R),
            "recovery_sec_median": med(rs), "recovery_sec_min": mn(rs), "recovery_sec_max": mx(rs),
            "recovery_over_T": round(med(rs) / (T / 1000), 2),
            "isolation_calls": R[0]["isolation_calls"], "trap_calls": R[0]["trap_calls"],
            "extra_cpu_core_sec_median": med([float(r["extra_cpu_core_sec"]) for r in R]),
            "catchup_peak_per_sec_median": med([float(r["catchup_peak_per_sec"]) for r in R]),
        })
    nw = [r for r in fsum("p4_watchdog") if r["case"] == "loop_noepoch_r1"][0]
    rec.append({"config": "noepoch (watchdog off)", "stage_timeout_ms": "n/a", "reuse": "on", "reps": 1, "host_alive": 1,
                "records_lost_values": nw["records_lost"], "stall_zero_sec_values": nw["stall_zero_sec"],
                "recovery_sec_median": None, "recovery_over_T": None, "isolation_calls": 0, "trap_calls": 0,
                "extra_cpu_core_sec_median": nw["extra_cpu_core_sec"], "catchup_peak_per_sec_median": nw["catchup_peak_per_sec"]})
    write("p1_poison_recovery_vs_timeout.csv", rec)

    # ---------- P4 epoch / reuse ----------
    er = []
    for variant, name in (("", "epoch on / reuse on"), ("noepoch", "epoch off"), ("noreuse", "reuse off")):
        R = [r for r in lat if r["rate_eps"] == 15000 and r["k"] == 256 and r["variant"] == variant]
        cpu = [r["cpu_mean_pct"] for r in R]
        p99 = [r["p99_ms"] for r in R]
        er.append({"config": name, "variant": variant or "baseline", "reps": len(R), "verdicts": " ".join(r["verdict"] for r in R),
                   "regimes": " ".join(sorted({r["regime"] for r in R})),
                   "cpu_values_pct": " ".join(f"{c:.1f}" for c in cpu), "cpu_median_pct": med(cpu), "cpu_min_pct": mn(cpu), "cpu_max_pct": mx(cpu),
                   "p99_values_ms": " ".join(f"{p:.0f}" for p in p99), "p99_median_ms": med(p99),
                   "p50_median_ms": med([r["p50_ms"] for r in R]), "rss_median_mib": med([r["rss_mean_mib"] for r in R])})
    base = er[0]["cpu_median_pct"]
    for r in er:
        r["cpu_delta_vs_baseline_pp"] = round(r["cpu_median_pct"] - base, 2)
        r["cpu_delta_rel_pct"] = round(100 * (r["cpu_median_pct"] - base) / base, 1)
    write("p4_epoch_reuse.csv", er)

    # ---------- P4 watchdog timeline ----------
    tl = list(csv.DictReader((RUNS / "fault/p4_watchdog/throughput_timeline.csv").open()))
    write("p4_watchdog_timeline.csv", tl)

    # ---------- P4 FAULT_MEM ----------
    fault = list(csv.DictReader((OUT / "fault.csv").open()))
    ms = fsum("p4_mem")
    mem = []
    for r in sorted(ms, key=lambda r: r["case"]):
        f = [x for x in fault if x["group"] == "p4_mem" and x["case"] == r["case"]][0]
        mem.append({"case": r["case"], "host": r["host_survived"], "records_lost": r["records_lost"], "recovery_sec": r["recovery_sec"],
                    "isolation_calls": r["isolation_calls"], "trap_calls": r["trap_calls"],
                    "rss_median_mib_fixed_sampler": round(float(f["rss_median_mib"]), 2), "rss_max_mib_fixed_sampler": f["rss_max_mib"],
                    "rss_peak_minus_baseline_mib": round(float(f["rss_max_mib"]) - 41.36, 2),
                    "extra_cpu_core_sec": r["extra_cpu_core_sec"], "stderr": r["stderr_summary"][:120]})
    write("p4_fault_mem.csv", mem)

    # ---------- P5 capability ----------
    cp = []
    probes = {
        "net_sockdeny": ("FAULT_NET", "sockets: deny（只授權 filesystem: inert）", "載入期"),
        "fileenv_capdeny": ("FAULT_FILE/ENV", "全部 deny（無 capabilities: 區塊）", "載入期"),
        "net_inert": ("FAULT_NET", "filesystem: inert + sockets: inert", "執行期"),
        "file_inert": ("FAULT_FILE", "filesystem: inert + sockets: inert", "執行期"),
        "env_inert": ("FAULT_ENV", "filesystem: inert + sockets: inert", "執行期"),
    }
    for key, (marker, grant, phase) in probes.items():
        R = [x for x in fault if x["group"] == "p5_cap" and x["case"].startswith(key)]
        outs, details = [], set()
        for x in R:
            d = RUNS / "fault/p5_cap" / x["case"]
            if (d / "refusal.txt").exists():
                t = read(d / "refusal.txt")
                outs.append(re.search(r"outcome=(\S+)", t).group(1))
                n = re.search(r"(\d+) 個未授權 import", t)
                details.add(f"{n.group(1)} 個未授權 import" if n else "")
            else:
                pr = re.search(r"PROBE \S+ .*?outcome=(\S+)(?: detail=(.*))?", read(d / "dut.log"))
                outs.append(pr.group(1) if pr else "NO_PROBE")
                if pr and pr.group(2):
                    details.add(pr.group(2)[:90])
                elif pr:
                    details.add("vars=0 bytes=0")
        sm = [s for s in fsum("p5_cap") if s["case"].startswith(key)]
        cp.append({"probe": marker, "grant": grant, "where": phase, "reps": len(R), "outcomes": " ".join(outs),
                   "records_lost": " ".join(s["records_lost"] for s in sm) if sm else "n/a (never started)",
                   "host": " ".join(s["host_survived"] for s in sm) if sm else "exit 1 at load",
                   "refusal_elapsed_s": " ".join(re.search(r"elapsed_s=(\S+)", x["refusal"]).group(1) for x in R if x["refusal"]),
                   "detail": " | ".join(sorted(details))})
    write("p5_capability_probes.csv", cp)

    lc = list(csv.DictReader((OUT / "loadcheck.csv").open()))
    lcs = []
    for grant in ("default", "inert-fs inert-sockets"):
        R = [r for r in lc if r["grant"] == grant]
        chk = [float(r["check_ms_median"]) for r in R if r["check_ms_median"]]
        load = [float(r["compile_ms_median"]) + float(r["link_ms_median"] or 0) + float(r["instantiate_pre_ms_median"] or 0)
                for r in R if r["compile_ms_median"]]
        ratio = [float(r["check_ms_median"]) / float(r["compile_ms_median"]) * 100 for r in R if r["check_ms_median"] and r["compile_ms_median"]]
        lcs.append({"grant": grant, "plugins": len(R), "load_ok": sum(r["result"] == "LOAD_OK" for r in R),
                    "load_refused": sum(r["result"] == "LOAD_REFUSED" for r in R),
                    "check_ms_median": med(chk), "check_ms_max": mx(chk),
                    "compile_link_pre_ms_median": med(load), "compile_link_pre_ms_min": mn(load), "compile_link_pre_ms_max": mx(load),
                    "check_over_compile_pct_max": round(max(ratio), 4) if ratio else None})
    write("p5_loadcheck_summary.csv", lcs)

    # ======================= charts =======================
    by = {(r["product"], r["W_s"]): r for r in m3}

    # (a1) trade-off
    c = Chart("Method 3 trade-off: capacity vs p99 @ 5,000 EPS (median of reps)", "Max sustainable EPS at window W",
              "p99 latency @ 5,000 EPS (ms, log)", [0, 10000, 20000, 30000, 40000, 50000, 60000, 70000],
              [10, 20, 50, 100, 200, 500, 1000, 2000, 5000], log_y=True, x_fmt=kfmt)
    for p in ORDER:
        pts, labels, tips = [], [], []
        for W in WS:
            r = by[(p, W)]
            if r["cap_median_eps"] and r["p99s_median_ms"]:
                pts.append((r["cap_median_eps"], r["p99s_median_ms"]))
                labels.append(f"W={W:g}" + (" @4k" if r["lat_rate_eps"] == 4000 else ""))
                tips.append(f"{LABEL[p]} W={W:g}: cap {r['cap_values']} · p99 {r['p99_values_ms']}")
                if r["cap_reps"] > 1:
                    c.hbar(r["cap_min_eps"], r["cap_max_eps"], r["p99s_median_ms"], COLOR[p])
                if r["lat_reps_valid"] > 1:
                    c.vbar(r["cap_median_eps"], r["p99s_min_ms"], r["p99s_max_ms"], COLOR[p])
        c.line(pts, COLOR[p], dash=DASH[p], labels=labels, tip=tips)
    c.legend([(LABEL[p], COLOR[p], DASH[p]) for p in ORDER], cols=3, colw=300)
    c.note("lower-right is better · bars = range over reps (p99: 3 reps; capacity: 3 reps at W=0.1 only, 1 rep otherwise)", dy=44)
    c.note("FB WASM fails at 5,000 EPS for every W → its p99 is the 4,000 EPS fallback (W ≥ 0.1); W=0.01 has no valid latency", dy=22)
    c.note("p99 = clock-corrected (fix_latency.py), excluding 3 s after each backward wall-clock step (Fluentd timer stalls; others unchanged ±10 ms)")
    c.save("a1_m3_tradeoff_capacity_vs_p99.svg")

    # (a2) excess vs W
    exs = [v for r in m3 for v in (r["excess_s_min_ms"], r["excess_s_max_ms"]) if v is not None]
    top = max(exs)
    yt = [t for t in (-100, 0, 100, 200, 300, 400, 500, 750, 1000, 1500, 2000) if t <= top + 150]
    c = Chart("Excess latency (p99 − 0.99·W) vs W @ 5,000 EPS (median, range over reps)", "Batching / flush window W (s, log)",
              "Excess p99 latency (ms)", [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0], yt + [yt[-1] + 250], log_x=True, x_fmt=wfmt)
    off = {p: 1 + 0.035 * (i - 2.5) for i, p in enumerate(ORDER)}
    for p in ORDER:
        pts = []
        for W in WS:
            r = by[(p, W)]
            if r["excess_s_median_ms"] is None:
                continue
            x = W * off[p]
            pts.append((x, r["excess_s_median_ms"]))
            if r["lat_reps_valid"] > 1:
                c.vbar(x, r["excess_s_min_ms"], r["excess_s_max_ms"], COLOR[p], w=4)
        c.line(pts, COLOR[p], dash=DASH[p], labels=[None] * (len(pts) - 1) + [LABEL[p]])
    c.legend([(LABEL[p], COLOR[p], DASH[p]) for p in ORDER], cols=3, colw=300)
    c.note("excess = processing + transport + flush-polling cost beyond the window; FB WASM points are at 4,000 EPS", dy=22)
    c.note("p99 excludes 3 s after each backward clock step (only Fluentd changes); x offset per product; r3 = post-resume runs included")
    c.save("a2_m3_excess_vs_window.svg")

    # (a3) p99 vs W
    c = Chart("p99 latency vs window W @ 5,000 EPS (median of valid reps)", "Batching / flush window W (s, log)",
              "p99 latency (ms, log)", [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0], [10, 20, 50, 100, 200, 500, 1000, 2000],
              log_x=True, log_y=True, x_fmt=wfmt)
    c.line([(0.01, 9.9), (1.0, 990)], "#94a3b8", dash="8 6", width=2, markers=False)
    for p in ORDER:
        pts = [(W, by[(p, W)]["p99s_median_ms"]) for W in WS if by[(p, W)]["p99s_median_ms"]]
        c.line(pts, COLOR[p], dash=DASH[p])
    c.legend([(LABEL[p], COLOR[p], DASH[p]) for p in ORDER] + [("0.99·W", "#94a3b8", "8 6")], cols=4, colw=300)
    c.note("FB WASM: 4,000 EPS fallback at W ≥ 0.1 (5,000 EPS FAILED 12/12); p99 excl. 3 s after backward clock steps")
    c.save("a3_m3_p99_vs_window.svg")

    # (b) capacity vs k
    c = Chart("WCM streaming: capacity vs micro-batch size k (W = 0.1 s)", "micro_batch_records k (log)",
              "Capacity (EPS)", [1, 4, 16, 64, 256], [0, 5000, 10000, 15000, 20000, 25000, 30000], log_x=True, y_fmt=kfmt)
    pts = []
    for r in ks:
        C = [x for x in cap if x["k"] == r["k"] and x["W_s"] == 0.1 and x["variant"] == ""]
        for x in C:
            if x["capacity_eps"]:
                c.dot(r["k"], x["capacity_eps"], COLOR["wcm-stream"], r=6, tip=f"k={r['k']} {x['tag']}: {x['capacity_eps']} ({x['search_start']})")
        if r["cap_median_eps"]:
            pts.append((r["k"], r["cap_median_eps"]))
    c.line(pts, COLOR["wcm-stream"], labels=[f"median {kfmt(y)}" for _, y in pts])
    k1 = ks[0]
    c.cross(1, k1["k1_saturated_input_eps_median"])
    c.text(1.08, k1["k1_saturated_input_eps_median"] + 1200, f"k=1: FAIL at every rate ≥ 4.5K (3/3 searches);", "fail")
    c.text(1.08, k1["k1_saturated_input_eps_median"] + 300, f"saturated throughput ≈ {k1['k1_saturated_input_eps_median'] / 1000:.1f}K EPS at 100 % CPU", "fail")
    for B, lab in ((21750, "batch mode, phase 0 (21.75K)"),):
        c.line([(1, B), (256, B)], COLOR["wcm-batch"], dash="8 5", width=2, markers=False)
        c.text(2, B + 500, lab)
    c.note("dots = individual searches (3 reps per k; ≈5 % resolution); 1 CPU / 512 MiB container")
    c.save("b_capacity_vs_k.svg")

    # (c) bisection
    xl = {0: "0 %", 1: "0.1 %", 2: "1 %", 3: "5 %", 4: "20 %"}
    rates = [0.0, 0.1, 1.0, 5.0, 20.0]
    mcol = {"batch": COLOR["wcm-batch"], "stream": COLOR["wcm-stream"]}
    mlab = {"batch": "batch mode (W=0.1, ~250 rec/call)", "stream": "streaming k=256 (~12 rec/call at 2.5K EPS)"}
    for fname, title, key, ylab, yt, fmt in (
        ("c1_bisection_delivered.svg", "Poison bisection: delivered / expected vs malformed rate (C3, 3 reps)", "delivered_pct_min",
         "delivered / expected (%, min of 3 reps)", [99, 99.25, 99.5, 99.75, 100, 100.25], lambda v: f"{v:g}"),
        ("c2_bisection_cpu.svg", "Poison bisection: CPU vs malformed rate (C3, 2,500 EPS, 3 reps)", "cpu_median_pct",
         "DUT CPU (% of 1 core, median; bars = range)", [0, 5, 10, 15, 20, 25, 30], lambda v: f"{v:g}"),
        ("c3_bisection_parse_calls.svg", "Poison bisection: extra parse calls vs malformed rate (100,000 input lines)", "isolation_calls_median",
         "isolation parse calls (median of 3)", [0, 20000, 40000, 60000, 80000, 100000, 120000], kfmt)):
        c = Chart(title, "malformed input rate", ylab, [-0.3, 0, 1, 2, 3, 4, 4.3], yt, x_fmt=lambda v: xl.get(v, ""), y_fmt=fmt)
        for mode in ("batch", "stream"):
            R = [r for r in b if r["mode"] == mode]
            pts = [(i, r[key]) for i, r in enumerate(R)]
            labs = []
            for i, r in enumerate(R):
                if key == "cpu_median_pct":
                    c.vbar(i, r["cpu_min_pct"], r["cpu_max_pct"], mcol[mode], w=5)
                    labs.append(f"{r['cpu_median_pct']:.1f}")
                elif key == "isolation_calls_median":
                    labs.append(f"{r['isolation_calls_median']:.0f} ({r['calls_per_malformed']}/bad)" if r["calls_per_malformed"] else "0")
                else:
                    labs.append(f"{r['delivered_pct_min']:.0f} %, rejected = malformed" if i == 4 else "")
            c.line(pts, mcol[mode], dash="8 5" if mode == "batch" else None, labels=labs)
        c.legend([(mlab[m], mcol[m], "8 5" if m == "batch" else None) for m in ("batch", "stream")], cols=2, colw=520)
        if key == "delivered_pct_min":
            c.note("all 30 runs PASS: received = expected, missing/dup/corrupt/reorder = 0, rejected lines = malformed inputs exactly")
        elif key == "isolation_calls_median":
            c.note("labels: calls (calls per malformed line); without errors the bisection costs zero extra calls")
        else:
            c.note("CPU from 1 s samples during the 40 s load; native C3 harness in container (not comparable to fault-case CPU)")
        c.save(fname)

    # (d) recovery vs timeout
    c = Chart("Poison record (FAULT_LOOP): stall until recovery vs stage_timeout_ms", "stage_timeout_ms (log)",
              "zero-throughput stall → recovery (s, log)", [1000, 2000, 5000, 10000, 30000], [1, 2, 5, 10, 20, 50, 100, 200],
              log_x=True, log_y=True, x_fmt=lambda v: f"{v / 1000:g} s")
    c.line([(1000, 4), (30000, 120)], "#94a3b8", dash="8 6", width=2, markers=False)
    rr = [r for r in rec if r["reuse"] == "on" and r["recovery_sec_median"]]
    c.line([(r["stage_timeout_ms"], r["recovery_sec_median"]) for r in rr], COLOR["wcm-stream"],
           labels=[f"{r['recovery_sec_median']:g} s (n={r['reps']}, lost {r['records_lost_values'].split()[0]})" for r in rr])
    for r in rr:
        if r["recovery_sec_max"] != r["recovery_sec_min"]:
            c.vbar(r["stage_timeout_ms"], r["recovery_sec_min"], r["recovery_sec_max"], COLOR["wcm-stream"])
    c.legend([("measured recovery (median of 3; all 3 reps identical at 1 s resolution)", COLOR["wcm-stream"], None),
              ("4 × stage_timeout (3 whole-batch attempts + bisection probes)", "#94a3b8", "8 6")], cols=1, colw=700)
    c.note("watchdog OFF (epoch_interruption=false): stall never ends — 59 s at 0 records/s until the harness stopped the run", dy=22)
    c.note("streaming k=256, 5,000 EPS, poison injected at t=15 s; native run, sink 1 s bins; 12 isolation calls, only the poison line lost")
    c.save("d_poison_recovery_vs_timeout.svg")

    # (e) epoch / reuse CPU
    c = Chart("Watchdog & disposable-instance overhead @ 15,000 EPS (streaming k=256, W=0.1)", "", "DUT CPU (% of 1 core)",
              [0, 1, 2, 3], [0, 20, 40, 60, 80, 100], x_fmt=lambda v: "")
    cols = ["#2563eb", "#64748b", "#db2777"]
    for i, r in enumerate(er):
        x = i + 0.5
        c.rect(x - 0.28, x + 0.28, 0, r["cpu_median_pct"], cols[i],
               tip=f"{r['config']}: {r['cpu_values_pct']}", label=f"{r['cpu_median_pct']:.1f} % ({r['cpu_delta_vs_baseline_pp']:+.1f} pp)")
        c.vbar(x, r["cpu_min_pct"], r["cpu_max_pct"], "#0f172a", w=8)
        c.body.append(f'<text x="{c.sx(x):.1f}" y="{c.t + c.ph + 30}" text-anchor="middle" class="tick">{escape(r["config"])}</text>')
        c.body.append(f'<text x="{c.sx(x):.1f}" y="{c.t + c.ph + 52}" text-anchor="middle" class="note">p99 {r["p99_values_ms"]} ms</text>')
    c.note("3 reps each (bars = range), all in the clean-clock period 02:41–03:14; reuse on = the epoch-on runs (same config)", dy=22)
    c.note("epoch off removes the watchdog entirely (see d / f); reuse off rebuilds format/transport instances per call")
    c.save("e_epoch_reuse_cpu.svg")

    # (f) timeline
    c = Chart("FAULT_LOOP with and without the watchdog: sink throughput timeline", "seconds from poison injection",
              "records / s at sink", [-15, 0, 15, 30, 45, 60], [0, 5000, 10000, 15000, 20000, 25000], y_fmt=kfmt)
    tcol = {"loop_noepoch_r1": "#b91c1c"}
    for case in ("loop_t2000_r1", "loop_t2000_r2", "loop_t2000_r3", "loop_noepoch_r1"):
        pts = [(float(r["sec_from_fault"]), float(r["records"])) for r in tl if r["case"] == case and -15 <= float(r["sec_from_fault"]) <= 60]
        c.line(pts, tcol.get(case, COLOR["wcm-stream"]), dash=None if case.endswith("r1") else "6 4", width=2.2 if "noepoch" in case else 1.8, markers=False)
    c.text(10, 1500, "watchdog OFF: 0 records/s from t≈1 s to end (59 s), CPU ≈ 98 %", "fail")
    c.text(9, 22800, "watchdog ON (2 s): 7 s stall, catch-up burst ≈22K/s, 1 line lost", "pt")
    c.legend([("epoch watchdog ON, stage_timeout 2 s (3 reps)", COLOR["wcm-stream"], None),
              ("watchdog OFF (epoch_interruption=false, 1 rep)", "#b91c1c", None)], cols=1, colw=600)
    c.note("offered load 2,500 output records/s (5,000 EPS × 50 % pass); native run, streaming k=256")
    c.save("f_watchdog_timeline.svg")

    # (g) capability table
    g = Chart("Capability-scoped stage: probe outcomes (parse_chaos plugin, 3 reps each)", None, None, None, None, w=1400, h=520)
    hdr = ["probe", "grant (parse stage)", "where stopped", "outcome (3 reps)", "records lost", "evidence"]
    xs = [40, 190, 560, 700, 930, 1060]
    y = 120
    g.body.append(f'<rect x="30" y="{y - 26}" width="1340" height="36" fill="#f1f5f9"/>')
    for x, h in zip(xs, hdr):
        g.body.append(f'<text x="{x}" y="{y}" class="hdr">{escape(h)}</text>')
    for r in cp:
        y += 52
        oc = sorted(set(r["outcomes"].split()))
        ev = r["detail"].split(" | ")[0][:44]
        cells = [r["probe"], r["grant"][:40], r["where"], f"{'/'.join(oc)} ×{r['reps']}", r["records_lost"][:18], ev]
        for x, cv in zip(xs, cells):
            g.body.append(f'<text x="{x}" y="{y}" class="cell">{escape(cv)}</text>')
        g.body.append(f'<line x1="30" y1="{y + 18}" x2="1370" y2="{y + 18}" stroke="#e2e8f0"/>')
    l0, l1 = lcs
    y += 60
    g.body.append(f'<text x="40" y="{y}" class="note">{escape(f"load-time import check: median {l0['check_ms_median']} ms (max {max(l0['check_ms_max'], l1['check_ms_max'])} ms) vs compile+link+instantiate_pre {l0['compile_link_pre_ms_min']:.0f}–{l0['compile_link_pre_ms_max']:.0f} ms → ≤ {max(l0['check_over_compile_pct_max'], l1['check_over_compile_pct_max']):.3f} % of load time")}</text>')
    g.body.append(f'<text x="40" y="{y + 24}" class="note">{escape(f"26 plugins: default-deny LOAD_OK {l0['load_ok']} / REFUSED {l0['load_refused']}; with inert-fs + inert-sockets LOAD_OK {l1['load_ok']} / REFUSED {l1['load_refused']}")}</text>')
    g.save("g_capability_probe_table.svg")


if __name__ == "__main__":
    main()
