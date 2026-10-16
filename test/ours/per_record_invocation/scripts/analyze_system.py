#!/usr/bin/env python3
"""Part B 分析：彙整 system/runs 與 system/correctness → CSV 與 SVG。

用法: analyze_system.py <study_dir>
輸出: <study_dir>/system/system_capacity.csv, system_trials.csv, system_latency.csv,
      system_correctness.csv, system_capacity_vs_k.svg, system_p99_vs_k.svg
"""
import csv
import glob
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot  # noqa: E402

study = sys.argv[1]
sysdir = os.path.join(study, "system")
DUTS = [("wcm-percall-k1", "1"), ("wcm-percall-k16", "16"), ("wcm-percall-k256", "256"),
        ("wcm-percall-kinf", "unlimited")]
# 正式 wcm-b1（memguard image）參考值：test/compare/EXPERIMENTS.md 與
# test/compare/capacity/runs/wcm-b1/lat_rate_*_fields1k_latency_cgroup_rerun
FORMAL = {"capacity": 25000, "lat": {
    5000: dict(p50=158.444, p99=263.959, p999=295.368, cpu=23.854, mem=25.524),
    10000: dict(p50=106.764, p99=168.080, p999=198.373, cpu=46.066, mem=23.320),
    15000: dict(p50=87.087, p99=137.818, p999=163.793, cpu=64.226, mem=26.999),
    20000: dict(p50=78.625, p99=129.826, p999=150.936, cpu=85.171, mem=27.796)}}


def read(p):
    return open(p).read() if os.path.exists(p) else ""


# ── 容量試驗 ──
trials, caps = [], []
for dut, k in DUTS:
    for d in sorted(glob.glob(os.path.join(sysdir, "runs", dut, "cap_rate_*"))):
        v = read(os.path.join(d, "verdict.txt")).strip()
        if not v:
            continue
        m = re.search(r"rate=(\d+)", v)
        g = lambda key: (re.search(key + r"=([^\s]+)", v) or [None, ""])[1]
        trials.append(dict(dut=dut, k=k, run=os.path.basename(d), rate=int(m.group(1)), verdict=v.split()[0],
                           gen_avg=g("gen_avg"), lag_p99=g("lag_p99"), backlog=g("backlog"),
                           sink_total=g("sink_total"), expected=g("expected"), oom=g("oom"),
                           reasons=g("reasons")))
    res = read(os.path.join(sysdir, "runs", dut, "search_result.txt"))
    m = re.search(r"CAPACITY dut=\S+ eps=(\S+)(?: first_fail=(\d+) confirmed=(\S+))?", res)
    passes = sorted({t["rate"] for t in trials if t["dut"] == dut and t["verdict"] == "PASS"})
    fails = sorted({t["rate"] for t in trials if t["dut"] == dut and t["verdict"] == "FAIL"})
    caps.append(dict(dut=dut, k=k, capacity_eps=m.group(1) if m else "",
                     first_fail_eps=(m.group(2) or "") if m else "", confirmed=(m.group(3) or "") if m else "",
                     max_pass_eps=max(passes) if passes else "", min_fail_eps=min(fails) if fails else "",
                     n_trials=sum(1 for t in trials if t["dut"] == dut)))


def wcsv(name, rows, keys=None):
    if not rows:
        return
    keys = keys or list(rows[0].keys())
    with open(os.path.join(sysdir, name), "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)


wcsv("system_trials.csv", trials)
wcsv("system_capacity.csv", caps)

# ── 延遲 / 資源 ──
lat = []
for dut, k in DUTS:
    for d in sorted(glob.glob(os.path.join(sysdir, "runs", dut, "lat_rate_*"))):
        s = read(os.path.join(d, "latency_stats_30_150.txt"))
        r = read(os.path.join(d, "resource_stats_30_150.txt"))
        v = read(os.path.join(d, "verdict.txt")).strip()
        if not s:
            continue
        get = lambda key: float(re.search(r"\]\s+" + re.escape(key) + r"\s+=\s+([\d.]+)", s).group(1))
        rr = lambda key: (re.search(key + r"=([\d.]+)", r) or [None, ""])[1]
        rate = int(re.search(r"lat_rate_(\d+)", d).group(1))
        lat.append(dict(dut=dut, k=k, rate=rate, run=os.path.basename(d), verdict=v.split()[0] if v else "",
                        mean_ms=get("mean"), p50_ms=get("p50"), p90_ms=get("p90"), p99_ms=get("p99"),
                        p999_ms=get("p99.9"), max_ms=get("max"),
                        cpu_mean_pct=rr("cpu_mean"), cpu_max_pct=rr("cpu_max"),
                        mem_mean_mib=rr("mem_mean"), mem_max_mib=rr("mem_max")))
wcsv("system_latency.csv", lat)

# ── C3 正確性 ──
corr = []
for d in sorted(glob.glob(os.path.join(sysdir, "correctness", "*", "C*_rep*"))):
    v = read(os.path.join(d, "verdict.txt")).strip()
    if not v:
        continue
    cfg = read(os.path.join(os.path.dirname(d), "forwarder.yaml"))
    m = re.search(r"plugin_call_size:\s*(\d+)", cfg)
    s = json.loads(read(os.path.join(d, "validation_summary.json")) or "{}")
    dut_log = read(os.path.join(d, "dut.log"))
    stats = list(csv.DictReader(open(os.path.join(d, "stats.csv")))) if os.path.exists(os.path.join(d, "stats.csv")) else []
    cpu = [float(x["cpu_pct"]) for x in stats]
    mem = [float(x["rss_mb"]) for x in stats]
    corr.append(dict(run=os.path.relpath(d, sysdir), k=m.group(1) if m else "unlimited", status=v.split()[0],
                     expected=s.get("expected"), received=s.get("received"), missing=s.get("missing"),
                     duplicate=s.get("duplicate"), corrupt=s.get("corrupt"), unexpected=s.get("unexpected"),
                     order_inversions=s.get("order_inversions"), bad_output_json=s.get("bad_output_json"),
                     parse_error_batches=dut_log.count("[parse-error]"),
                     subcall_reject_lines=dut_log.count("[parse-subcall]"),
                     rejected_records=sum(int(x) for x in re.findall(r"rejected (\d+)/", dut_log)),
                     cpu_avg_pct=f"{sum(cpu)/len(cpu):.2f}" if cpu else "", cpu_max_pct=max(cpu) if cpu else "",
                     mem_avg_mib=f"{sum(mem)/len(mem):.2f}" if mem else "", mem_max_mib=max(mem) if mem else ""))
wcsv("system_correctness.csv", corr)

# ── 圖 ──
capv = [float(c["capacity_eps"]) if re.fullmatch(r"\d+", str(c["capacity_eps"])) else None for c in caps]
if any(v is not None for v in capv):
    svgplot.bar_chart(os.path.join(sysdir, "system_capacity_vs_k.svg"),
                      "Max sustainable EPS vs plugin_call_size k",
                      "1024 B / 36 fields · 1 CPU · 512 MiB · single TCP · search.sh criteria · formal wcm-b1 = 25,000",
                      "max sustainable input EPS", [f"k={k}" for _, k in DUTS], ["percall image"], [capv])
rates = sorted({x["rate"] for x in lat})
if rates:
    vals = []
    for rate in rates:
        row = []
        for dut, _ in DUTS:
            m = [x for x in lat if x["dut"] == dut and x["rate"] == rate]
            row.append(m[0]["p99_ms"] if m else None)
        vals.append(row)
    svgplot.bar_chart(os.path.join(sysdir, "system_p99_vs_k.svg"),
                      "p99 latency vs k (RAW, contaminated by clock steps; see system_latency_vs_k.svg)",
                      "1024 B / 36 fields · 180 s load · 30–150 s window · formal wcm-b1 @5,000 EPS p99 = 263.959 ms",
                      "p99 latency (ms)", [f"k={k}" for _, k in DUTS], [f"{r:,} EPS" for r in rates], vals, unit="")
print(f"trials={len(trials)} latency_points={len(lat)} correctness_runs={len(corr)}")
for c in caps:
    print(c)
for x in lat:
    print({k: x[k] for k in ("dut", "rate", "p50_ms", "p99_ms", "p999_ms", "cpu_mean_pct", "mem_mean_mib")})
for c in corr:
    print(c)
