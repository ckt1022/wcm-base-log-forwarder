#!/usr/bin/env python3
"""補充分析（在 analyze_micro.py / analyze_system.py 之後執行，不需重跑任何量測）。

用法: analyze_followup.py <study_dir>

產出：
  micro/analysis/micro_ns_per_record_tidy.csv   每個 (stage, lang, mode, B) 一列：ns/record、run 間範圍、
                                                相對 B=1024 與相對最佳 B 的額外成本
  micro/analysis/micro_fit_le256.csv            只用 B ≤ 256 的 t(B)=a+b·B 擬合（parse rust / C# 在 B=1024 有非線性退化）
  micro/analysis/micro_go_partial.csv           Go(TinyGo) parse reuse：B=64 起 trap，只保留 B=1/4/16 的部分資料
  micro/analysis/micro_fit_a_b.svg              a（固定每次呼叫成本）與 b（每筆成本）長條圖
  system/system_latency_corrected.csv           5,000 EPS 延遲：原始值 + 時鐘階梯校正值 + 第一次階梯前的乾淨窗口
  system/system_latency_timeline.csv            每 10 s 的最小 / 中位 / 最大延遲（原始與校正後）
  system/system_summary_vs_k.csv                每個 k 的容量、每筆 CPU 時間、5,000 EPS p50/p99、CPU、RSS
  system/system_c3_compare.csv                  C3：k=1、k=unlimited、主樹 host 端 bisection（isolation）
  system/system_capacity_vs_k.svg, system_latency_vs_k.svg, system_cpu_rss_vs_k.svg, system_c3_received.svg
"""
import csv
import glob
import gzip
import json
import os
import re
import statistics
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot  # noqa: E402

study = os.path.abspath(sys.argv[1])
MAIN = "/home/ckt1022/wcm-base-log-forwarder"
micro_out = os.path.join(study, "micro", "analysis")
sysdir = os.path.join(study, "system")
os.makedirs(micro_out, exist_ok=True)


def wcsv(path, rows, keys=None):
    keys = keys or list(rows[0].keys())
    with open(path, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys, extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow({k: (f"{v:.3f}" if isinstance(v, float) else v) for k, v in r.items()})


# ═════════════════════════ Part A：micro ═════════════════════════
summ = list(csv.DictReader(open(os.path.join(micro_out, "micro_summary.csv"))))
for d in summ:
    d["call_size"] = int(d["call_size"])
    for k in ("ns_per_record", "per_call_median_ns", "run_median_min_ns", "run_median_max_ns"):
        d[k] = float(d[k])

tidy = []
for d in summ:
    reuse = [x for x in summ if x["stage"] == d["stage"] and x["lang"] == d["lang"] and x["mode"] == "reuse"]
    full = [x for x in reuse if x["call_size"] == 1024]
    best = min(reuse, key=lambda x: x["ns_per_record"]) if reuse else None
    recs = int(d["records"])
    tidy.append(dict(
        stage=d["stage"], lang=d["lang"], mode=d["mode"], call_size=d["call_size"], calls=d["calls"],
        runs=d["runs"], samples=d["samples"], ns_per_record=d["ns_per_record"],
        ns_per_record_run_min=d["run_median_min_ns"] / recs, ns_per_record_run_max=d["run_median_max_ns"] / recs,
        per_call_ns=d["per_call_median_ns"],
        extra_pct_vs_B1024_reuse=(d["ns_per_record"] / full[0]["ns_per_record"] - 1) * 100 if full else "",
        best_reuse_B=best["call_size"] if best else "",
        extra_pct_vs_best_reuse=(d["ns_per_record"] / best["ns_per_record"] - 1) * 100 if best else ""))
wcsv(os.path.join(micro_out, "micro_ns_per_record_tidy.csv"), tidy)


def wls(points):
    sw = swx = swxx = swy = swxy = 0.0
    for x, y in points:
        w = 1.0 / (y * y)
        sw += w; swx += w * x; swxx += w * x * x; swy += w * y; swxy += w * x * y
    det = sw * swxx - swx * swx
    a = (swxx * swy - swx * swxy) / det
    b = (sw * swxy - swx * swy) / det
    maxrel = max(abs(a + b * x - y) / y for x, y in points)
    return a, b, maxrel


main_fit = {(r["stage"], r["lang"]): r for r in csv.DictReader(open(os.path.join(micro_out, "micro_fit.csv")))}
fit2 = []
for (stage, lang), mf in sorted(main_fit.items()):
    pts = [(d["call_size"], d["per_call_median_ns"]) for d in summ
           if d["stage"] == stage and d["lang"] == lang and d["mode"] == "reuse" and d["call_size"] <= 256]
    a, b, mr = wls(pts)
    reuse = {d["call_size"]: d["ns_per_record"] for d in summ
             if d["stage"] == stage and d["lang"] == lang and d["mode"] == "reuse"}
    best = min(reuse.values())
    within = {t: min(B for B, v in reuse.items() if v <= best * (1 + t / 100)) for t in (5, 10)}
    fit2.append(dict(stage=stage, lang=lang, a_all_B_ns=float(mf["a_fixed_per_call_ns"]),
                     b_all_B_ns=float(mf["b_per_record_ns"]), max_rel_residual_all_B=float(mf["max_rel_residual"]),
                     a_le256_ns=a, b_le256_ns=b, max_rel_residual_le256=mr,
                     ns_per_record_B1=reuse[1], ns_per_record_B16=reuse[16], ns_per_record_B256=reuse[256],
                     ns_per_record_B1024=reuse[1024], best_B=min(reuse, key=reuse.get),
                     min_B_within_5pct_of_best=within[5], min_B_within_10pct_of_best=within[10],
                     extra_ns_per_record_B1_vs_best=reuse[1] - best))
wcsv(os.path.join(micro_out, "micro_fit_le256.csv"), fit2)

# Go：reuse 在 B=64 trap（TinyGo plugin out-of-bounds），保留 .failed 中已完成的 B
go = []
gsrc = os.path.join(study, "micro", "results", "formal")
grows = []
for f in sorted(glob.glob(os.path.join(gsrc, "parse_go_reuse_run*.csv.failed"))):
    grows += list(csv.DictReader(open(f)))
for f in sorted(glob.glob(os.path.join(gsrc, "parse_go_fresh-pre_run*.csv"))):
    grows += list(csv.DictReader(open(f)))
for key in sorted({(r["mode"], int(r["call_size"])) for r in grows}):
    v = [float(r["ns_per_record"]) for r in grows if (r["mode"], int(r["call_size"])) == key]
    go.append(dict(stage="parse", lang="go", mode=key[0], call_size=key[1], samples=len(v),
                   ns_per_record_median=statistics.median(v), ns_per_record_min=min(v), ns_per_record_max=max(v),
                   note="reuse: B=64 起 wasm trap（out of bounds memory access），無 B≥64 資料" if key[0] == "reuse" else
                   "每次呼叫新 instance（InstancePre）"))
if go:
    wcsv(os.path.join(micro_out, "micro_go_partial.csv"), go)

# a / b 長條圖（正式 plugin，非 instr）
labs = [f'{r["stage"]} {r["lang"]}' for r in fit2 if not r["lang"].endswith("instr")]
rows = [r for r in fit2 if not r["lang"].endswith("instr")]
svgplot.bar_chart(os.path.join(micro_out, "micro_fit_a_b.svg"),
                  "t(B) = a + b·B per stage / language (instance reused)",
                  "a = fixed cost per plugin call, b = cost per record · ns · weighted LSQ over B = 1…1024",
                  "ns", labs, ["a (per call)", "b (per record)"],
                  [[r["a_all_B_ns"] for r in rows], [r["b_all_B_ns"] for r in rows]], width=1700)

# ═════════════════════════ Part B：system ═════════════════════════
DUTS = [("wcm-percall-k1", "1"), ("wcm-percall-k16", "16"), ("wcm-percall-k256", "256"),
        ("wcm-percall-kinf", "unlimited")]
REF_LAT = os.path.join(MAIN, "test/compare/capacity/runs/wcm-b1/lat_rate_5000_fields1k_latency_cgroup_rerun")
REF = dict(capacity=25000, p50=158.444, p99=263.959, p999=295.368, cpu=23.854, mem=25.524)


def load_lat(path):
    op = gzip.open if path.endswith(".gz") else open
    with op(path, "rt") as fh:
        next(fh)
        arr = np.loadtxt((l.split(",", 1)[1] for l in fh), delimiter=",")
    return arr[:, 0], arr[:, 1]


def pct(v, p):
    s = np.sort(v)
    k = (len(s) - 1) * p / 100.0
    lo = int(k); hi = min(lo + 1, len(s) - 1)
    return float(s[lo] + (k - lo) * (s[hi] - s[lo]))


def correct_steps(recv, lat, horizon=200.0, jump=400.0, floor=10.0):
    """偵測 sink 與 generator 時間基準的階梯式偏移並扣除。

    證據：同一 run 內吞吐固定、backlog≈0，但「每秒最小延遲」突然整段抬高數百 ms 並維持不變。
    方法：1 s bin 的最小延遲序列中，相鄰差 > jump ms 視為一次階梯；每段的水準 = 段內 bin 最小值的中位數，
    offset_j = 水準_j − 水準_0。逐筆指派到「offset_j ≤ lat − floor 的最大 j」（j 不超過下一個 bin 所屬段），
    前提是自然延遲範圍（≈20–370 ms）小於最小階梯（本資料 ≥ 600 ms）。
    """
    t = recv - recv[0]
    keep = t <= horizon
    t, lat = t[keep], lat[keep]
    nb = int(t.max()) + 1
    bins = t.astype(int)
    bmin = np.full(nb, np.nan)
    for i in range(nb):
        m = lat[bins == i]
        if len(m):
            bmin[i] = m.min()
    seg_of_bin = np.zeros(nb, dtype=int)
    seg = 0
    prev = bmin[0]
    for i in range(1, nb):
        if np.isnan(bmin[i]):
            seg_of_bin[i] = seg
            continue
        if bmin[i] - prev > jump:
            seg += 1
        elif prev - bmin[i] > jump:
            raise RuntimeError("negative step detected; correction assumptions violated")
        seg_of_bin[i] = seg
        prev = bmin[i]
    nseg = seg + 1
    levels = []
    for j in range(nseg):
        idx = [i for i in range(nb) if seg_of_bin[i] == j and not np.isnan(bmin[i])]
        core = idx[1:-1] if len(idx) > 3 else idx
        levels.append(float(np.median(bmin[core])))
    offs = np.array([lv - levels[0] for lv in levels])
    nxt = np.append(seg_of_bin[1:], seg_of_bin[-1])
    maxseg = nxt[bins]
    assigned = np.zeros(len(lat), dtype=int)
    for j in range(1, nseg):
        assigned = np.where((lat - floor >= offs[j]) & (maxseg >= j), j, assigned)
    corr = lat - offs[assigned]
    first_step_t = float(t[np.argmax(assigned > 0)]) if nseg > 1 else float(t.max())
    return t, lat, corr, offs, first_step_t


lat_rows, timeline = [], []
cases = [(dut, k, glob.glob(os.path.join(sysdir, "runs", dut, "lat_rate_5000_*", "latency_worker_1.csv*"))[0])
         for dut, k in DUTS]
cases.append(("wcm-b1 (formal reference)", "unlimited (ref)", os.path.join(REF_LAT, "latency_worker_1.csv")))
for dut, k, path in cases:
    recv, lat = load_lat(path)
    t, raw, corr, offs, t1 = correct_steps(recv, lat)
    w = (t >= 30) & (t <= 150)
    clean = (t >= 10) & (t < t1 - 1)
    row = dict(dut=dut, k=k, n_window=int(w.sum()),
               raw_p50_ms=pct(raw[w], 50), raw_p99_ms=pct(raw[w], 99),
               n_clock_steps=len(offs) - 1, total_step_ms=float(offs[-1]),
               step_sizes_ms=" ".join(f"{b - a:.0f}" for a, b in zip(offs[:-1], offs[1:])),
               corr_p50_ms=pct(corr[w], 50), corr_p90_ms=pct(corr[w], 90), corr_p99_ms=pct(corr[w], 99),
               corr_p999_ms=pct(corr[w], 99.9), corr_max_ms=float(corr[w].max()), corr_min_ms=float(corr[w].min()),
               clean_window_s=f"10–{t1:.0f}" if t1 > 12 else "none",
               clean_n=int(clean.sum()),
               clean_p50_ms=pct(raw[clean], 50) if clean.sum() > 1000 else "",
               clean_p99_ms=pct(raw[clean], 99) if clean.sum() > 1000 else "")
    lat_rows.append(row)
    for b0 in range(0, 190, 10):
        m = (t >= b0) & (t < b0 + 10)
        if m.sum() == 0:
            continue
        timeline.append(dict(dut=dut, t_start_s=b0, n=int(m.sum()),
                             raw_min_ms=float(raw[m].min()), raw_p50_ms=pct(raw[m], 50), raw_max_ms=float(raw[m].max()),
                             corr_min_ms=float(corr[m].min()), corr_p50_ms=pct(corr[m], 50),
                             corr_max_ms=float(corr[m].max())))
wcsv(os.path.join(sysdir, "system_latency_corrected.csv"), lat_rows)
wcsv(os.path.join(sysdir, "system_latency_timeline.csv"), timeline)

caps = {r["dut"]: r for r in csv.DictReader(open(os.path.join(sysdir, "system_capacity.csv")))}
lraw = {r["dut"]: r for r in csv.DictReader(open(os.path.join(sysdir, "system_latency.csv")))}
summary = []
for dut, k in DUTS:
    c = int(caps[dut]["capacity_eps"])
    lr = [r for r in lat_rows if r["dut"] == dut][0]
    summary.append(dict(k=k, capacity_eps=c, capacity_confirmed=caps[dut]["confirmed"],
                        first_fail_eps=caps[dut]["first_fail_eps"],
                        cap_vs_kinf_pct=(c / int(caps["wcm-percall-kinf"]["capacity_eps"]) - 1) * 100,
                        cap_vs_formal_25000_pct=(c / REF["capacity"] - 1) * 100,
                        cpu_us_per_record_at_capacity=1e6 / c,
                        p50_corr_ms=lr["corr_p50_ms"], p99_corr_ms=lr["corr_p99_ms"],
                        p50_raw_ms=lr["raw_p50_ms"], p99_raw_ms=lr["raw_p99_ms"],
                        cpu_mean_pct=float(lraw[dut]["cpu_mean_pct"]), rss_mean_mib=float(lraw[dut]["mem_mean_mib"]),
                        rss_max_mib=float(lraw[dut]["mem_max_mib"])))
lr = [r for r in lat_rows if r["dut"].startswith("wcm-b1")][0]
summary.append(dict(k="formal wcm-b1", capacity_eps=REF["capacity"], capacity_confirmed="3 reps (manual)",
                    first_fail_eps="25500", cap_vs_kinf_pct="", cap_vs_formal_25000_pct=0.0,
                    cpu_us_per_record_at_capacity=1e6 / REF["capacity"],
                    p50_corr_ms=lr["corr_p50_ms"], p99_corr_ms=lr["corr_p99_ms"],
                    p50_raw_ms=REF["p50"], p99_raw_ms=REF["p99"], cpu_mean_pct=REF["cpu"], rss_mean_mib=REF["mem"],
                    rss_max_mib=29.36))
wcsv(os.path.join(sysdir, "system_summary_vs_k.csv"), summary)

# ── C3 比較 ──
def c3_rows(base, label, k):
    out = []
    for d in sorted(glob.glob(os.path.join(base, "C3_rep*"))):
        s = json.load(open(os.path.join(d, "validation_summary.json")))
        st = list(csv.DictReader(open(os.path.join(d, "stats.csv"))))
        cpu_time = float(st[-1]["cpu_time_s"]) - float(st[0]["cpu_time_s"]) if st else ""
        log = open(os.path.join(d, "dut.log")).read()
        # parse plugin 呼叫次數：k=1 ＝ 每筆輸入一次；整批 ＝ 每個 batch 一次；bisection ＝ 每個 batch 一次 + 隔離時的額外呼叫
        n_in = int(open(os.path.join(d, "input_count.txt")).read().split()[0])
        n_batches = len(set(re.findall(r"batch=(\d+)", log))) if k != "1" else 0
        iso_calls = sum(int(x) for x in re.findall(r"parse-isolate\].* calls=(\d+)", log))
        calls = n_in if k == "1" else n_batches + iso_calls
        out.append(dict(variant=label, k=k, run=os.path.relpath(d, MAIN), status=open(os.path.join(d, "verdict.txt")).read().split()[0],
                        expected=s["expected"], received=s["received"], missing=s["missing"], duplicate=s["duplicate"],
                        corrupt=s["corrupt"], order_inversions=s["order_inversions"],
                        whole_batch_parse_errors=log.count("[parse-error]"),
                        rejected_records=sum(int(x) for x in re.findall(r"rejected[ =](\d+)", log)),
                        parse_plugin_calls_total=calls, parse_calls_spent_on_isolation=iso_calls,
                        cpu_seconds_during_run=cpu_time,
                        rss_max_mib=max(float(x["rss_mb"]) for x in st) if st else ""))
    return out


c3 = c3_rows(os.path.join(sysdir, "correctness", "c3_k1"), "per-record (k=1)", "1")
c3 += c3_rows(os.path.join(sysdir, "correctness", "c3_kinf"), "whole batch, no isolation", "unlimited")
c3 += c3_rows(os.path.join(MAIN, "test/compare/correctness/runs/isolation_docker_20261014_002704"),
              "whole batch + host bisection on error (src/isolation.rs, docker)", "unlimited")
wcsv(os.path.join(sysdir, "system_c3_compare.csv"), c3)

# ── 圖 ──
groups = [f"k={k}" for _, k in DUTS] + ["formal wcm-b1"]
svgplot.bar_chart(os.path.join(sysdir, "system_capacity_vs_k.svg"),
                  "Max sustainable EPS vs plugin_call_size k",
                  "1024 B / 36 fields · 1 CPU · 512 MiB · single TCP · 60 s trials · k=unlimited not confirmed on rerun",
                  "max sustainable input EPS", groups, ["max sustainable EPS"],
                  [[s["capacity_eps"] for s in summary]])
svgplot.bar_chart(os.path.join(sysdir, "system_latency_vs_k.svg"),
                  "Latency at 5,000 EPS vs plugin_call_size k (clock-step corrected)",
                  "30–150 s window · corrected = minus detected sink/generator clock steps · raw p99 shown for reference",
                  "latency (ms)", groups, ["p50 corrected", "p99 corrected", "p99 raw (contaminated)"],
                  [[s["p50_corr_ms"] for s in summary], [s["p99_corr_ms"] for s in summary],
                   [s["p99_raw_ms"] for s in summary]])
svgplot.bar_chart(os.path.join(sysdir, "system_cpu_rss_vs_k.svg"),
                  "Container CPU and RSS at 5,000 EPS vs plugin_call_size k",
                  "docker stats 1 s samples · 30–150 s window mean · single run per point",
                  "CPU % of 1 core  /  RSS MiB", groups, ["CPU mean %", "RSS mean MiB"],
                  [[s["cpu_mean_pct"] for s in summary], [s["rss_mean_mib"] for s in summary]])
labs = []
for r in c3:
    short = "bisect" if "bisection" in r["variant"] else {"1": "k=1", "unlimited": "k=∞"}[r["k"]]
    labs.append(f'{short} r{r["run"].rsplit("_rep", 1)[-1]}')
svgplot.bar_chart(os.path.join(sysdir, "system_c3_received.svg"),
                  "C3 (5 % malformed JSON, 2,500 EPS): valid records delivered",
                  "expected 38,000 per run · k=1 and k=∞ from this study · bisect = main tree src/isolation.rs (docker, 3 reps)",
                  "records received", labs, ["received"], [[r["received"] for r in c3]])

for r in fit2:
    print({k: (round(v, 1) if isinstance(v, float) else v) for k, v in r.items()})
for r in lat_rows:
    print({k: (round(v, 1) if isinstance(v, float) else v) for k, v in r.items()})
for r in summary:
    print({k: (round(v, 1) if isinstance(v, float) else v) for k, v in r.items()})
for r in c3:
    print({k: v for k, v in r.items() if k != "run"})
for r in go:
    print(r)
