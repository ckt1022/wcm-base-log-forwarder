#!/usr/bin/env python3
"""Part A 分析：彙整 micro/results/<dir>/*.csv → summary / fit / breakdown CSV 與 SVG 圖。

用法: analyze_micro.py <results_dir> <out_dir>
  results_dir 內每個 CSV = 一個 plugin × 模式 × run（run0 為 probe，忽略）。
統計方式（與 copy_overhead 相同精神）：同一 (stage, lang, mode, B) 的所有 run 樣本合併取中位數；
另列每個 run 的中位數最小/最大值以呈現 run 間差異。
擬合：每次呼叫時間 t(B) = a + b·B（t = 該樣本總時間 / 呼叫次數），
以「相對誤差」加權最小平方（權重 1/t²），避免 B=1024 的大數值主導 a 的估計。
"""
import csv
import glob
import os
import statistics
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot  # noqa: E402

src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)

rows = []
for f in sorted(glob.glob(os.path.join(src, "*.csv"))):
    if f.endswith("_run0.csv"):
        continue
    with open(f) as fh:
        rows += list(csv.DictReader(fh))

g = defaultdict(list)
for r in rows:
    g[(r["stage"], r["lang"], r["mode"], int(r["call_size"]))].append(r)


def med(v):
    return statistics.median(v)


def p95(v):
    s = sorted(v)
    return s[min(len(s) - 1, int((len(s) - 1) * 0.95 + 0.999999))]


summary = []
for (stage, lang, mode, b), rs in sorted(g.items()):
    tot = [float(r["total_ns"]) for r in rs]
    recs = int(rs[0]["records"])
    calls = int(rs[0]["calls"])
    per_run = defaultdict(list)
    for r in rs:
        per_run[r["run"]].append(float(r["total_ns"]))
    run_meds = [med(v) for v in per_run.values()]
    d = dict(stage=stage, lang=lang, mode=mode, call_size=b, records=recs, calls=calls,
             samples=len(tot), runs=len(per_run),
             total_median_ns=med(tot), total_p95_ns=p95(tot),
             run_median_min_ns=min(run_meds), run_median_max_ns=max(run_meds),
             ns_per_record=med(tot) / recs, records_per_sec=recs * 1e9 / med(tot),
             per_call_median_ns=med(tot) / calls)
    if rs[0]["instrumented"] == "true":
        for k in ("copy_in_ns", "guest_ns", "copy_out_ns"):
            d[k.replace("_ns", "_per_call_ns")] = med([float(r[k]) for r in rs]) / calls
    summary.append(d)

# 相對於 B=1024（同 stage/lang/mode=reuse）的倍率
base = {(d["stage"], d["lang"]): d["ns_per_record"] for d in summary
        if d["mode"] == "reuse" and d["call_size"] == d["records"]}
for d in summary:
    bb = base.get((d["stage"], d["lang"]))
    d["ns_per_record_vs_full_batch"] = d["ns_per_record"] / bb if bb else ""
    d["extra_pct_vs_full_batch"] = (d["ns_per_record"] / bb - 1) * 100 if bb else ""

keys = ["stage", "lang", "mode", "call_size", "records", "calls", "samples", "runs", "total_median_ns",
        "total_p95_ns", "run_median_min_ns", "run_median_max_ns", "ns_per_record", "records_per_sec",
        "per_call_median_ns", "ns_per_record_vs_full_batch", "extra_pct_vs_full_batch",
        "copy_in_per_call_ns", "guest_per_call_ns", "copy_out_per_call_ns"]
with open(os.path.join(out, "micro_summary.csv"), "w", newline="") as fh:
    w = csv.DictWriter(fh, fieldnames=keys, extrasaction="ignore")
    w.writeheader()
    for d in summary:
        w.writerow({k: (f"{v:.3f}" if isinstance(v, float) else v) for k, v in d.items()})


def wls(points):
    """points: [(B, t)] → (a, b, r2)；最小化 Σ((a+bB-t)/t)²。"""
    sw = swx = swxx = swy = swxy = 0.0
    for x, y in points:
        w = 1.0 / (y * y)
        sw += w; swx += w * x; swxx += w * x * x; swy += w * y; swxy += w * x * y
    det = sw * swxx - swx * swx
    a = (swxx * swy - swx * swxy) / det
    b = (sw * swxy - swx * swy) / det
    ys = [y for _, y in points]
    my = sum(ys) / len(ys)
    ss_tot = sum((y - my) ** 2 for y in ys)
    ss_res = sum((y - (a + b * x)) ** 2 for x, y in points)
    maxrel = max(abs(a + b * x - y) / y for x, y in points)
    return a, b, 1 - ss_res / ss_tot if ss_tot else 1.0, maxrel


fits = []
for (stage, lang) in sorted({(d["stage"], d["lang"]) for d in summary if d["mode"] == "reuse"}):
    pts = [(d["call_size"], d["per_call_median_ns"]) for d in summary
           if d["stage"] == stage and d["lang"] == lang and d["mode"] == "reuse"]
    if len(pts) < 3:
        continue
    a, b, r2, maxrel = wls(pts)
    full = [d for d in summary if d["stage"] == stage and d["lang"] == lang and d["mode"] == "reuse"]
    bmax = max(d["call_size"] for d in full)
    nsfull = [d for d in full if d["call_size"] == bmax][0]["ns_per_record"]
    cross = {}
    for thr in (5, 10):
        ok = sorted(d["call_size"] for d in full if d["ns_per_record"] <= nsfull * (1 + thr / 100))
        cross[thr] = ok[0] if ok else ""
    # 模型推算：a/(b·B) ≤ 5% → B ≥ 20a/b
    fr = dict(stage=stage, lang=lang, a_fixed_per_call_ns=a, b_per_record_ns=b, r2=r2,
              max_rel_residual=maxrel, a_over_b_records=a / b if b > 0 else "",
              model_B_for_5pct=20 * a / b if b > 0 else "", model_B_for_10pct=10 * a / b if b > 0 else "",
              measured_min_B_within_5pct=cross[5], measured_min_B_within_10pct=cross[10],
              ns_per_record_B1=[d for d in full if d["call_size"] == 1][0]["ns_per_record"] if any(d["call_size"] == 1 for d in full) else "",
              ns_per_record_full=nsfull)
    for comp in ("copy_in", "guest", "copy_out"):
        cp = [(d["call_size"], d.get(comp + "_per_call_ns")) for d in full if d.get(comp + "_per_call_ns")]
        if len(cp) >= 3 and all(v > 0 for _, v in cp):
            ca, cb, _, _ = wls(cp)
            fr[comp + "_a_ns"] = ca
            fr[comp + "_b_ns"] = cb
    fits.append(fr)

fkeys = ["stage", "lang", "a_fixed_per_call_ns", "b_per_record_ns", "r2", "max_rel_residual", "a_over_b_records",
         "model_B_for_5pct", "model_B_for_10pct", "measured_min_B_within_5pct", "measured_min_B_within_10pct",
         "ns_per_record_B1", "ns_per_record_full", "copy_in_a_ns", "copy_in_b_ns", "guest_a_ns", "guest_b_ns",
         "copy_out_a_ns", "copy_out_b_ns"]
with open(os.path.join(out, "micro_fit.csv"), "w", newline="") as fh:
    w = csv.DictWriter(fh, fieldnames=fkeys, extrasaction="ignore")
    w.writeheader()
    for d in fits:
        w.writerow({k: (f"{v:.3f}" if isinstance(v, float) else v) for k, v in d.items()})

# ── 圖：每個 stage 的 ns/record vs B（reuse），fresh-pre B=1 以虛線點標示 ──
for stage in ("parse", "filter", "format"):
    series = []
    for lang in sorted({d["lang"] for d in summary if d["stage"] == stage}):
        pts = [(d["call_size"], d["ns_per_record"]) for d in summary
               if d["stage"] == stage and d["lang"] == lang and d["mode"] == "reuse"]
        if len(pts) >= 2:
            series.append((f"{lang} reuse", pts, {}))
    for lang in sorted({d["lang"] for d in summary if d["stage"] == stage}):
        pts = [(d["call_size"], d["ns_per_record"]) for d in summary
               if d["stage"] == stage and d["lang"] == lang and d["mode"] == "fresh-pre"]
        if pts:
            series.append((f"{lang} fresh/call", pts, {"dash": True}))
    if not series:
        continue
    svgplot.line_chart(
        os.path.join(out, f"micro_{stage}_ns_per_record_vs_B.svg"),
        f"{stage}: ns/record vs records per call (B)",
        "1024 records × ~1 KB · Wasmtime 42 · pinned 1 core · median of 3 runs × 100 samples (fresh: 3 × 30)",
        "records per plugin call B (log)", "ns per record (log)", series, xlog=True, ylog=True,
        xticks=[1, 4, 16, 64, 256, 1024])

# 相對 full batch 的額外成本（%），reuse，只畫正式 plugin
series = []
for d0 in fits:
    if d0["lang"].endswith("instr"):
        continue
    pts = [(d["call_size"], max(0.1, d["extra_pct_vs_full_batch"])) for d in summary
           if d["stage"] == d0["stage"] and d["lang"] == d0["lang"] and d["mode"] == "reuse"
           and d["extra_pct_vs_full_batch"] != ""]
    series.append((f'{d0["stage"]} {d0["lang"]}', pts, {}))
if series:
    svgplot.line_chart(
        os.path.join(out, "micro_extra_cost_pct_vs_B.svg"),
        "Extra cost vs full batch (B=1024), instance reused",
        "ns/record(B) / ns/record(1024) − 1 · values ≤ 0 clipped to 0.1 % · dashed lines at 5 % / 10 % not drawn",
        "records per plugin call B (log)", "extra cost % (log)", series, xlog=True, ylog=True,
        xticks=[1, 4, 16, 64, 256, 1024], ymin=0.1, yticks=[0.1, 0.3, 1, 3, 5, 10, 30, 100, 300, 1000])
print(f"wrote {out}/micro_summary.csv, micro_fit.csv and charts ({len(rows)} samples)")
