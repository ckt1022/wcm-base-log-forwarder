#!/usr/bin/env python3
"""collect_suite.py — 把 pattern_suite 的原始結果整理成 CSV（只讀原始資料，不改任何檔案）。

用法：collect_suite.py <runs_dir>     （預設 test/compare/pattern_suite/runs）
輸出：<runs_dir>/../analysis/
  latency.csv       每個 lat run：dut、rate、tag、verdict、p50/p99/p99.9（clock-step corrected）、CPU、RSS、OOM
  capacity.csv      每個 search：dut、tag、capacity、first_fail、trials
  correctness.csv   P1 malformed sweep（每個 run 一列，含 isolation 統計）
  fault.csv         所有 fault case：host 狀態、送出／收到、零吞吐秒數、恢復時間、PROBE 行、refusal、RSS
  loadcheck.csv     P5 --check-plugin 分段計時（每個 plugin × grant 的 3 reps 中位數）
"""
import csv
import json
import re
import statistics
import sys
from pathlib import Path

runs = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parent / "runs").resolve()
out = runs.parent / "analysis"
out.mkdir(exist_ok=True)


def read(p: Path) -> str:
    try:
        return p.read_text(errors="replace")
    except OSError:
        return ""


def num(pattern: str, text: str):
    m = re.search(pattern, text)
    return float(m.group(1)) if m else None


def write(name, rows):
    if not rows:
        return
    keys = []
    for r in rows:
        for k in r:
            if k not in keys:
                keys.append(k)
    with (out / name).open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)
    print(out / name, len(rows))


# ── latency ──
lat = []
for d in sorted(runs.glob("*/lat_rate_*")):
    m = re.match(r"lat_rate_(\d+)_(.+)", d.name)
    st, rs, vd = read(d / "latency_stats_30_150.txt"), read(d / "resource_stats_30_150.txt"), read(d / "verdict.txt")
    lat.append({
        "dut": d.parent.name, "rate": int(m.group(1)), "tag": m.group(2),
        "verdict": vd.split()[0] if vd.split() else "",
        "p50_ms": num(r"p50\s+=\s+([\d.]+)", st), "p99_ms": num(r"p99\s+=\s+([\d.]+)", st),
        "p999_ms": num(r"p99\.9\s+=\s+([\d.]+)", st),
        "cpu_mean_pct": num(r"cpu_mean=([\d.]+)", rs), "cpu_max_pct": num(r"cpu_max=([\d.]+)", rs),
        "rss_mean_mib": num(r"mem_mean=([\d.]+)", rs), "rss_max_mib": num(r"mem_max=([\d.]+)", rs),
        "oom": read(d / "oom.txt").strip(), "verdict_line": vd.strip()[:200],
    })
write("latency.csv", lat)

# ── capacity ──
cap = []
for f in sorted(runs.glob("*/search_*.txt")):
    txt = read(f)
    lines = [l for l in txt.splitlines() if l.startswith("CAPACITY")]
    last = lines[-1] if lines else ""
    cap.append({
        "dut": f.parent.name, "tag": f.stem[len("search_"):],
        "capacity_eps": (re.search(r"eps=(\S+)", last) or [None, ""])[1],
        "first_fail": (re.search(r"first_fail=(\S+)", last) or [None, ""])[1],
        "trials": len(re.findall(r"^trial ", txt, re.M)),
        "fail_reasons": ";".join(sorted(set(re.findall(r"reasons=(\S+?)\(", txt)))),
    })
write("capacity.csv", cap)

# ── correctness (P1) ──
corr = []
for d in sorted((runs / "correctness").glob("p1_*/C3_rep*")):
    m = re.match(r"p1_(\w+?)_mr(.+)", d.parent.name)
    gen, sink = {}, {}
    try:
        gen = json.loads(read(d / "generator_summary.json") or "{}")
        sink = json.loads(read(d / "validation_summary.json") or "{}")
    except json.JSONDecodeError:
        pass
    dut = read(d / "dut.log")
    cpu, rss = [], []
    try:
        dur = float(gen.get("elapsed_sec", 0))
        with (d / "stats.csv").open() as h:
            for it in csv.DictReader(h):
                if 1 <= float(it["elapsed_s"]) <= dur:
                    cpu.append(float(it["cpu_pct"])); rss.append(float(it["rss_mb"]))
    except (OSError, ValueError, KeyError):
        pass
    vd = read(d / "verdict.txt").split()
    corr.append({
        "mode": m.group(1), "malformed_pct": m.group(2), "rep": d.name.split("rep")[-1],
        "verdict": vd[0] if vd else "", "input": gen.get("count"), "malformed_input": gen.get("malformed"),
        "expected": sink.get("expected"), "received": sink.get("received"), "missing": sink.get("missing"),
        "delivered_over_expected": (sink["received"] / sink["expected"]) if sink.get("expected") else None,
        "parse_error_batches": len(re.findall(r"\[parse-error\]", dut)),
        "isolated_batches": len(re.findall(r"\[parse-isolate\]", dut)),
        "salvaged_lines": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?salvaged=(\d+)", dut)),
        "rejected_lines": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?rejected=(\d+)", dut)),
        "isolation_parse_calls": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?calls=(\d+)", dut)),
        "cpu_mean_pct": round(statistics.fmean(cpu), 3) if cpu else None,
        "rss_mean_mib": round(statistics.fmean(rss), 3) if rss else None,
        "rss_max_mib": max(rss) if rss else None,
    })
write("correctness.csv", corr)

# ── fault ──
fault = []
for d in sorted((runs / "fault").glob("*/*")):
    if not d.is_dir() or not (d / "meta.txt").exists():
        continue
    dut = read(d / "dut.log")
    gen = {}
    try:
        gen = json.loads(read(d / "generator_summary.json") or "{}")
    except json.JSONDecodeError:
        pass
    recv = []
    for line in read(d / "recv_per_sec.csv").splitlines()[1:]:
        parts = line.split(",")
        try:
            recv.append(int(parts[1]))
        except (IndexError, ValueError):
            pass
    # 零吞吐：第一個非零秒之後、最後一個非零秒之前的 0 秒數；尾端連續 0 秒另計（停滯到結束）
    nz = [i for i, n in enumerate(recv) if n > 0]
    zero_mid = sum(1 for n in recv[nz[0]:nz[-1] + 1] if n == 0) if nz else None
    zero_tail = (len(recv) - 1 - nz[-1]) if nz else None
    rss = []
    try:
        with (d / "stats.csv").open() as h:
            rss = [float(it["rss_mb"]) for it in csv.DictReader(h)]
    except (OSError, ValueError, KeyError):
        pass
    fault.append({
        "group": d.parent.name, "case": d.name, "host": read(d / "host_status.txt").strip(),
        "refusal": " ".join(read(d / "refusal.txt").splitlines()[:2]),
        "sent": gen.get("sent"), "gen_terminated": gen.get("terminated"), "expected_out": (gen["sent"] + 1) // 2 if gen.get("sent") else None,
        "received": sum(recv) if recv else None, "sink_seconds": len(recv),
        "zero_sec_mid": zero_mid, "zero_sec_tail": zero_tail,
        "isolate": (re.search(r"\[parse-isolate\][^\n]*", dut) or [""])[0][:200],
        "trap_lines": len(re.findall(r"\[trap \d+/\d+\]", dut)),
        "probe": " | ".join(re.findall(r"\[parse_chaos\] PROBE[^\n]*", dut))[:300],
        "rss_max_mib": max(rss) if rss else None, "rss_median_mib": statistics.median(rss) if rss else None,
    })
write("fault.csv", fault)
for s in sorted((runs / "fault").glob("*/summary.csv")):
    print("summarize_fault:", s)

# ── loadcheck ──
agg = {}
for f in sorted((runs / "p5_loadcheck").glob("loadcheck_r*.csv")):
    with f.open() as h:
        for r in csv.DictReader(l for l in h if not l.startswith("#")):
            agg.setdefault((r["stage"], r["plugin"], r["grant"]), []).append(r)
lc = []
for (stage, plugin, grant), rs in agg.items():
    def med(k):
        v = [float(r[k]) for r in rs if r.get(k)]
        return round(statistics.median(v), 3) if v else None
    lc.append({"stage": stage, "plugin": plugin, "grant": grant, "result": rs[0]["result"], "reps": len(rs),
               "wall_ms_median": med("wall_ms"), "compile_ms_median": med("compile_ms"), "check_ms_median": med("check_ms"),
               "link_ms_median": med("link_ms"), "instantiate_pre_ms_median": med("instantiate_pre_ms")})
write("loadcheck.csv", lc)
