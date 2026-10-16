#!/usr/bin/env python3
"""finalize.py — 方法 1 / 2 / 3 的最終 tidy CSV（在 collect.py 之上加時鐘校正稽核、穩定性判準與方法 1）。

只讀既有原始資料，不跑任何量測。輸出（RESULTS_DIR，預設本目錄）：
  method1_time_aligned.csv      方法 1：WCM tw128（1 s 時間窗）vs wcm-b1 vs 其他產品正式 sweep，同 rate
  method2_batch_volume.csv      方法 2：1 MiB 輸入批量；p50/p99 per product × rate（另附正式 1 MiB 輸出批量對照列）
  method3_latency_5000eps.csv   方法 3：5,000 EPS × W 的 p50/p99/p99.9/excess/CPU/RSS/stable
  method3_capacity.csv          方法 3：容量 per product × W（新 session；正式 1 s 容量只放在對照欄）
  clock_correction_audit.csv    每個被引用的延遲 run：原始值、clock_drift 校正值、資料偵測校正值、採用哪一個

延遲數字的來源規則（每列 correction 欄註明）：
  * run 目錄有 clock_drift.csv（本目錄 runs/ 的新 run）→ 採 correct_latency.py 的量測式校正（clock_drift）；
    stepdetect.py 的資料偵測值只作交叉驗證（detector_p99_ms）。
  * 沒有 clock_drift.csv（方法 1 的 wcm-tw128、正式 sweep）→ 跑 stepdetect.py：
      - 30–150 s 窗內有記錄被位移 → 採偵測校正值（correction=detector）；
      - 否則原始值即乾淨值（correction=none: clean in 30–150 s）。
  穩定性 latency_stable 一律以「最終採用的（校正後）序列」的 30 s 子窗 p50 判斷（collect.py 定義）。
"""
import csv
import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import collect as C  # noqa: E402
import stepdetect as sd  # noqa: E402

OUTD = C.OUTD
CAP = C.CAP
REPO = C.REPO


def rel(p):
    p = Path(p)
    return str(p.relative_to(REPO)) if p.is_relative_to(REPO) else str(p)


def fnum(path, key):
    if not Path(path).exists():
        return ""
    m = re.search(r"\[calc\]\s+" + re.escape(key) + r"\s+=\s+([\d.]+) ms", Path(path).read_text())
    return float(m.group(1)) if m else ""


_AUDIT = {}


def latency(d: Path):
    """回傳最終採用的延遲數字 + 稽核欄位（快取）。"""
    d = Path(d)
    if d in _AUDIT:
        return _AUDIT[d]
    a = {"run_dir": rel(d)}
    if not (d / "latency_worker_1.csv").exists():
        _AUDIT[d] = a
        return a
    recv, lat = sd.load_latency(d)
    r = sd.correct_steps(recv, lat)
    t = r["t"]
    win = (t >= 30) & (t <= 150)
    raw = sd.window_stats(t, r["raw"])
    det = sd.window_stats(t, r["corr"])
    a.update(raw_p50_ms=round(raw["p50"], 3), raw_p99_ms=round(raw["p99"], 3),
             detector_p50_ms=round(det["p50"], 3), detector_p99_ms=round(det["p99"], 3),
             detector_steps=len(r["step_sizes"]),
             detector_step_sizes_ms=" ".join(f"{x:.0f}" for x in r["step_sizes"]),
             detector_step_times_s=" ".join(str(x) for x in r["step_times"]),
             detector_shifted_in_window_pct=round(100 * float(r["shifted"][win].mean()), 1),
             detector_warnings="; ".join(r["warnings"]))
    if (d / "clock_drift.csv").exists():
        ls = d / "latency_stats_30_150.txt"
        a.update(correction="clock_drift.csv (measured wall−monotonic, correct_latency.py)",
                 p50_ms=fnum(ls, "p50"), p99_ms=fnum(ls, "p99"), p99_9_ms=fnum(ls, "p99.9"),
                 mean_ms=fnum(ls, "mean"), max_ms=fnum(ls, "max"))
        rawf = d / "latency_stats_30_150_raw.txt"
        a.update(raw_p50_ms=fnum(rawf, "p50") or a["raw_p50_ms"], raw_p99_ms=fnum(rawf, "p99") or a["raw_p99_ms"])
        sub = C.subwindows(d)
        txt = (d / "subwindows_clockfix.txt").read_text().splitlines()
        a["clock_steps_measured"] = txt[1].split(":", 1)[1].strip() if len(txt) > 1 else ""
    else:
        if a["detector_shifted_in_window_pct"] > 0:
            a["correction"] = "detector (stepdetect.py; no clock_drift.csv)"
            src = r["corr"]
        else:
            a["correction"] = "none: clean in 30–150 s (detector found no step affecting the window)"
            src = r["raw"]
        s = sd.window_stats(t, src)
        a.update(p50_ms=round(s["p50"], 3), p99_ms=round(s["p99"], 3), p99_9_ms=round(s["p999"], 3),
                 mean_ms=round(s["mean"], 3), max_ms=round(s["max"], 3))
        sub = sd.subwindows(t, src)
    a["latency_stable"] = sd.stable_flag(sub)
    sw = [s for s in sub if 30 <= s[0] < 150]
    a["subwin_p50_ms"] = " ".join(f"{s[1]:.0f}" for s in sw)
    a["subwin_p99_ms"] = " ".join(f"{s[2]:.0f}" for s in sw)
    if isinstance(a.get("p99_ms"), float):
        a["detector_minus_final_p99_ms"] = round(a["detector_p99_ms"] - a["p99_ms"], 1)
    _AUDIT[d] = a
    return a


def resources(d):
    f = Path(d) / "resource_stats_30_150.txt"
    if not f.exists():
        return {}
    t = f.read_text()
    out = {}
    for k, pat in (("cpu_mean_pct", r"cpu_mean=([\d.]+)%"), ("mem_mean_mib", r"mem_mean=([\d.]+)MiB"),
                   ("mem_max_mib", r"mem_max=([\d.]+)MiB")):
        m = re.search(pat, t)
        out[k] = float(m.group(1)) if m else ""
    out["cpu_basis"] = "Δcpu_time/Δmonotonic" if "mono" in t else "mean of 1 s docker samples (wall)"
    return out


def status(d):
    d = Path(d)
    v = (d / "verdict.txt").read_text().strip() if (d / "verdict.txt").exists() else ""
    if v:
        m = re.search(r"reasons=(\S+)", v)
        return v.split()[0], (m.group(1) if m else "")
    oom = (d / "oom.txt").read_text().strip() if (d / "oom.txt").exists() else ""
    return ("OOM-killed (no verdict)" if oom == "true" else "INCOMPLETE"), ("oom=true" if oom == "true" else "")


LAT_FIELDS = ["status", "fail_reason", "latency_stable", "p50_ms", "p99_ms", "p99_9_ms", "mean_ms", "max_ms",
              "correction", "raw_p50_ms", "raw_p99_ms", "detector_p99_ms", "detector_steps",
              "subwin_p50_ms", "subwin_p99_ms", "cpu_mean_pct", "mem_mean_mib", "mem_max_mib"]


def fill(row, d):
    """以最終校正值覆蓋 collect.parse_run 的延遲欄位。"""
    a = latency(d)
    for k in ("p50_ms", "p99_ms", "p99_9_ms", "mean_ms", "max_ms", "latency_stable", "correction",
              "raw_p50_ms", "raw_p99_ms", "detector_p99_ms", "detector_steps", "subwin_p50_ms", "subwin_p99_ms"):
        row[k] = a.get(k, "")
    row.update(resources(d))
    return row


# ───────────────────────────── 方法 1 ─────────────────────────────
M1_RATES = [2500, 5000, 7500, 10000, 12500, 15000, 17500, 20000, 22250]
M1_SRC = [
    ("wcm-tw128", "WCM tw128 (method 1)", "wcm-tw128/lat_rate_{r}_fields1k_latency_cgroup",
     "time window: max_wait_ms=1000, buffer_size_mb=128 (size never triggers), mem_limit_mb=256", "time 1 s"),
    ("wcm-b1", "WCM b1 (formal)", "wcm-b1/lat_rate_{r}_fields1k_latency_cgroup_rerun",
     "buffer_size_mb=1 (input bytes), max_wait_ms=1000, mem_limit_mb=128", "input size 1 MiB (≈0.2 s @5k)"),
    ("fbit-lua", "Fluent Bit Lua (formal)", "fbit-lua/lat_rate_{r}_fields1k_latency_cgroup_sweep",
     "[SERVICE] Flush=1", "time 1 s"),
    ("fbit-wasm", "Fluent Bit WASM (formal)", "fbit-wasm/lat_rate_{r}_fields1k_latency_cgroup_sweep",
     "[SERVICE] Flush=1", "time 1 s"),
    ("fluentd", "Fluentd (formal)", "fluentd/lat_rate_{r}_fields1k_latency_sweep",
     "chunk_limit_size=1m, flush_interval=1", "output size 1 MiB or 1 s"),
    ("vector", "Vector (formal)", "vector/lat_rate_{r}_fields1k_latency_cgroup_sweep",
     "batch.max_bytes=1MiB, timeout_secs=1", "output size 1 MiB or 1 s"),
]


def method1():
    rows = []
    for r in M1_RATES:
        for key, label, pat, knobs, trig in M1_SRC:
            d = CAP / pat.format(r=r)
            if not d.is_dir():
                continue
            st, why = status(d)
            row = {"method": "M1_time_aligned", "rate_eps": r, "dut": key, "dut_label": label, "knobs": knobs,
                   "flush_trigger": trig, "status": st, "fail_reason": why}
            if (d / "latency_worker_1.csv").exists() and st == "PASS":
                fill(row, d)
            row["run_dir"] = rel(d)
            rows.append(row)
    return rows


# ───────────────────────────── 方法 2 / 3 ─────────────────────────────
def redo(rows):
    out = []
    for r in rows:
        d = REPO / r["run_dir"]
        fill(r, d)
        w = r.get("W_s", r.get("W_eq_s"))
        if isinstance(r.get("p99_ms"), float) and w not in ("", None):
            r["excess_p99_ms"] = round(r["p99_ms"] - 0.99 * float(w) * 1000, 1)
        if r.get("status") != "PASS":
            r["note"] = "FAILED PASS criteria (overloaded) — latency not valid"
        elif r.get("latency_stable") != "yes":
            r["note"] = "latency growing — not a steady state"
        out.append(r)
    return out


def method3_latency():
    rows = redo(C.method3_latency())
    formal = {"fbit-lua": CAP / "fbit-lua/lat_rate_5000_fields1k_latency_cgroup_sweep",
              "fbit-wasm": CAP / "fbit-wasm/lat_rate_5000_fields1k_latency_cgroup_sweep",
              "wcm": CAP / "wcm-tw128/lat_rate_5000_fields1k_latency_cgroup"}
    for r in rows:
        if r["W_s"] == 1.0 and r["product"] in formal:
            a = latency(formal[r["product"]])
            r["formal_p50_ms"], r["formal_p99_ms"] = a.get("p50_ms", ""), a.get("p99_ms", "")
            r["formal_run_dir"] = a["run_dir"] + ("  [method-1 run, detector-corrected]" if r["product"] == "wcm" else "")
    return rows


REASON = [("gen_no_done_line", "no verdict (gen connection reset)"), ("backlog", "C2 backlog > 3 s of output"), ("gen_blocked", "C1 generator blocked (send-lag p99 > 10 ms / rate shortfall)"),
          ("loss", "C3 loss")]


def why(dut, tag, rate):
    f = C.RUNS / dut / f"cap_rate_{rate}_{tag}" / "verdict.txt"
    if not f.exists():
        return ""
    v = f.read_text()
    oom = (f.parent / "oom.txt").read_text().strip() if (f.parent / "oom.txt").exists() else ""
    if oom == "true":
        return "C4 OOM-killed (512 MiB, mem-guard could not hold)"
    reasons = v.split("reasons=", 1)[1] if "reasons=" in v else v
    for k, txt in REASON:
        if k in reasons:
            m = re.search(r"lag_p99=([\d.]+)ms", v)
            return txt + (f" [lag_p99={m.group(1)} ms]" if k == "gen_blocked" and m else "")
    return ""


def method3_capacity():
    rows = C.method3_capacity()
    for r in rows:
        tag = C.wtag(r["W_s"])
        r["first_fail_reason"] = why(r["product"], tag, r.get("first_fail_eps", ""))
        r["fails_below_capacity"] = " ".join(
            f"{x}:{why(r['product'], tag, x).split(' ')[0]}" for x, v in re.findall(r"(\d+):(PASS|FAIL)", r.get("trials", ""))
            if v == "FAIL" and int(x) < int(r.get("capacity_eps") or 0)) or ""
        if r["W_s"] == 1.0:
            fc = r.get("formal_capacity_eps")
            r["formal_vs_new_pct"] = round((int(r["capacity_eps"]) / fc - 1) * 100, 1) if fc else ""
            r["note"] = ("formal capacity = different session (2026-08-09, 3 reps) — NOT mixed into the W curve"
                         + ("; formal WCM = wcm-b1 (1 MiB input size trigger), not tw128" if r["product"] == "wcm" else ""))
    return rows


def write(name, rows, fields):
    OUTD.mkdir(parents=True, exist_ok=True)
    with (OUTD / name).open("w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow({k: r.get(k, "") for k in fields})
    print(f"wrote {name} ({len(rows)} rows)")


if __name__ == "__main__":
    m1 = method1()
    write("method1_time_aligned.csv", m1,
          ["method", "rate_eps", "dut", "dut_label", "flush_trigger", "knobs"] + LAT_FIELDS + ["cpu_basis", "run_dir"])
    m2 = redo(C.method2())
    write("method2_batch_volume.csv", m2,
          ["method", "variant", "product", "dut_label", "rate_eps", "W_eq_s", "knobs", "source"] + LAT_FIELDS +
          ["excess_p99_ms", "sink_posts", "posts_per_s", "records_per_post", "kib_per_post", "input_equiv_kib_per_post",
           "win_out_eps", "note", "run_dir"])
    m3 = method3_latency()
    m3cap = method3_capacity()
    capd = {(r["product"], r["W_s"]): r.get("capacity_eps") for r in m3cap}
    for r in m3:
        c = capd.get((r["product"], r["W_s"]))
        if c and int(c) < 5000:
            r["note"] = (r.get("note", "") + "; " if r.get("note") else "") + \
                f"5,000 EPS is above this setting's searched capacity ({c}) — marginal point"
            if r.get("status") == "PASS":
                r["note"] += f" (p99 grows across sub-windows: {r.get('subwin_p99_ms', '')} ms)"
    write("method3_latency_5000eps.csv", m3,
          ["method", "product", "dut_label", "W_s", "rate_eps", "knobs", "source"] + LAT_FIELDS[:6] +
          ["excess_p99_ms"] + LAT_FIELDS[6:] +
          ["sink_posts", "posts_per_s", "records_per_post", "kib_per_post", "win_out_eps",
           "formal_p50_ms", "formal_p99_ms", "formal_run_dir", "note", "run_dir"])
    write("method3_capacity.csv", m3cap,
          ["method", "product", "dut_label", "W_s", "capacity_eps", "first_fail_eps", "first_fail_reason",
           "fails_below_capacity", "knobs", "trials", "source", "formal_capacity_eps", "formal_vs_new_pct",
           "formal_source", "note"])
    audit = sorted(_AUDIT.values(), key=lambda a: a["run_dir"])
    write("clock_correction_audit.csv", audit,
          ["run_dir", "correction", "p50_ms", "p99_ms", "raw_p50_ms", "raw_p99_ms", "detector_p50_ms", "detector_p99_ms",
           "detector_minus_final_p99_ms", "detector_steps", "detector_step_sizes_ms", "detector_step_times_s",
           "detector_shifted_in_window_pct", "detector_warnings", "clock_steps_measured", "latency_stable",
           "subwin_p50_ms", "subwin_p99_ms"])
