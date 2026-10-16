#!/usr/bin/env python3
"""collect.py — 彙整 batch-alignment 方法 2 / 方法 3 的結果成 tidy CSV。

資料來源（優先順序）：
  1. 本目錄 runs/<dut>/<mode>_rate_<R>_<tag>/（本次在目前時鐘下新跑，含 clock-step 校正）
  2. 若新跑不存在，才退回 test/compare/capacity/runs/ 的正式資料（source 欄標 reused:...）
另外對有正式對照的點，附上 formal_* 欄（正式 sweep 的值）供核對。

延遲：30–150 s 窗（0 點 = 第一筆 receive），新跑取 correct_latency.py 的校正值；
正式資料取其 latency_stats_30_150.txt（正式量測時無時鐘跳躍問題）。
"""
import csv
import os
import re
import statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
CAP = HERE.parent / "capacity" / "runs"
RUNS = Path(os.environ.get("RUNS_DIR", HERE / "runs")).resolve()   # 重現時可指向新的輸出目錄
OUTD = Path(os.environ.get("RESULTS_DIR", HERE)).resolve()

PRODUCTS = ["wcm", "fluentd", "vector", "fbit-lua", "fbit-wasm"]
LABEL = {"wcm": "WCM", "fluentd": "Fluentd", "vector": "Vector",
         "fbit-lua": "Fluent Bit Lua", "fbit-wasm": "Fluent Bit WASM"}
W_LIST = [0.01, 0.1, 0.5, 1.0]
M2_RATES = (5000, 10000, 15000, 20000)


def wtag(w):
    return f"m3_W{w:g}"


def fnum(t, pat):
    m = re.search(pat, t)
    return float(m.group(1)) if m else ""


def subwindows(d: Path):
    """30 s 子窗口 p50/p99 列表 [(start, p50, p99)]；新跑讀 subwindows_clockfix.txt，正式資料由 CSV 計算並快取。"""
    f = d / "subwindows_clockfix.txt"
    if not f.exists():
        f = HERE / "cache" / (str(d.relative_to(CAP)).replace("/", "__") + ".sub.txt") if d.is_relative_to(CAP) else None
        if f is None:
            return []
        if not f.exists():
            src = d / "latency_worker_1.csv"
            if not src.exists():
                return []
            import numpy as np
            x = np.loadtxt(src, delimiter=",", skiprows=1, usecols=(1, 2), ndmin=2)
            rel = x[:, 0] - x[:, 0].min()
            parts = []
            for s in range(0, 180, 30):
                v = x[(rel >= s) & (rel < s + 30), 1]
                if len(v):
                    parts.append(f"{s}-{s + 30}s p50={np.percentile(v, 50):.1f} p99={np.percentile(v, 99):.1f}")
            f.parent.mkdir(exist_ok=True)
            f.write_text(" | ".join(parts) + "\n")
    line = f.read_text().splitlines()[0]
    return [(int(a), float(b), float(c)) for a, b, c in re.findall(r"(\d+)-\d+s p50=([\d.]+) p99=([\d.]+)", line)]


def parse_run(d: Path):
    if d is None or not d.is_dir():
        return None
    r = {"run_dir": str(d.relative_to(REPO)) if d.is_relative_to(REPO) else str(d)}
    v = (d / "verdict.txt").read_text().strip() if (d / "verdict.txt").exists() else ""
    r["status"] = v.split(" ")[0] if v else "INCOMPLETE"
    m = re.search(r"reasons=(\S+)", v)
    r["fail_reason"] = m.group(1) if m else ""
    m = re.search(r"win_out_eps=(\d+)", v)
    r["win_out_eps"] = int(m.group(1)) if m else ""
    ls = d / "latency_stats_30_150.txt"
    if ls.exists():
        t = ls.read_text()
        for key, name in (("mean", "mean_ms"), ("min", "min_ms"), ("p50", "p50_ms"),
                          ("p99", "p99_ms"), ("p99.9", "p99_9_ms"), ("max", "max_ms")):
            r[name] = fnum(t, r"\[calc\]\s+" + re.escape(key) + r"\s+=\s+([\d.]+) ms")
    rs = d / "resource_stats_30_150.txt"
    if rs.exists():
        t = rs.read_text()
        r["cpu_mean_pct"] = fnum(t, r"cpu_mean=([\d.]+)%")
        r["mem_mean_mib"] = fnum(t, r"mem_mean=([\d.]+)MiB")
        r["mem_max_mib"] = fnum(t, r"mem_max=([\d.]+)MiB")
    sl = d / "sink.log"
    if sl.exists():
        mm = re.search(r"done: batches=(\d+) lines=(\d+) bytes=(\d+)", sl.read_text())
        if mm:
            b, n, by = map(int, mm.groups())
            r["sink_posts"] = b
            r["records_per_post"] = round(n / b, 1) if b else ""
            r["kib_per_post"] = round(by / b / 1024, 1) if b else ""
            # 每批相當的輸入量：輸出筆數 / 0.4（filter 保留 warn+error = 40%）× 1024 B
            r["input_equiv_kib_per_post"] = round(n / b / 0.4, 1) if b else ""
            gl = d / "gen.log"
            if gl.exists():
                e = fnum(gl.read_text(), r"elapsed=([\d.]+)s")
                if e:
                    r["posts_per_s"] = round(b / e, 2)
    sw = [s for s in subwindows(d) if 30 <= s[0] < 150]
    if sw:
        r["p99_subwin_median_ms"] = round(statistics.median(s[2] for s in sw), 1)
        r["p99_subwin_max_ms"] = round(max(s[2] for s in sw), 1)
        first, last = sw[0], sw[-1]
        # 不穩定 = 延遲隨時間持續增長（以 p50 判斷，避免單一尖峰誤判）
        r["latency_stable"] = "yes" if last[1] <= 1.3 * first[1] + 20 else "NO(growing)"
    return r


def first_existing(*paths):
    for p in paths:
        if p is not None and p.is_dir() and (p / "verdict.txt").exists():
            return p
    return None


FIELDS_LAT = ["method", "product", "dut_label", "W_s", "rate_eps", "knobs", "source", "status", "fail_reason",
              "latency_stable", "p50_ms", "p99_ms", "p99_9_ms", "excess_p99_ms", "p99_subwin_median_ms",
              "p99_subwin_max_ms", "mean_ms", "min_ms", "max_ms",
              "cpu_mean_pct", "mem_mean_mib", "mem_max_mib",
              "sink_posts", "posts_per_s", "records_per_post", "kib_per_post", "input_equiv_kib_per_post",
              "win_out_eps", "formal_p50_ms", "formal_p99_ms", "formal_run_dir", "run_dir"]


def m3_knobs(p, w):
    if p == "wcm":
        return f"max_wait_ms={int(round(w * 1000))}, buffer_size_mb=128, mem_limit_mb=256, max_transport_bytes=1MiB (tw128 base)"
    if p == "fluentd":
        fd = ("flush_interval=1, flush_thread_interval=0.1 (formal)" if w >= 1
              else f"flush_interval={w:g}, flush_thread_interval={w:g} (window = enqueue period)")
        return fd + ", chunk_limit_size=128m"
    if p == "vector":
        return f"batch.timeout_secs={w:g}, batch.max_bytes=128MiB, batch.max_events=1e8"
    return f"[SERVICE] Flush={w:g}"


def attach_formal(r, d):
    f = parse_run(d)
    if f:
        r["formal_p50_ms"], r["formal_p99_ms"] = f.get("p50_ms", ""), f.get("p99_ms", "")
        r["formal_run_dir"] = f["run_dir"]


def method3_latency():
    rows = []
    formal_w1 = {
        "fbit-lua": CAP / "fbit-lua/lat_rate_5000_fields1k_latency_cgroup_sweep",
        "fbit-wasm": CAP / "fbit-wasm/lat_rate_5000_fields1k_latency_cgroup_sweep",
    }
    for p in PRODUCTS:
        ws = set(W_LIST)
        for d in (RUNS / p).glob("lat_rate_5000_m3_W*"):
            ws.add(float(d.name.split("_W")[1]))
        for w in sorted(ws):
            d = first_existing(RUNS / p / f"lat_rate_5000_{wtag(w)}")
            src = "new"
            if d is None and w == 1.0 and p in formal_w1:
                d, src = formal_w1[p], "reused:formal 1 s sweep"
            if d is None and w == 1.0 and p == "wcm":
                d = first_existing(CAP / "wcm-tw128/lat_rate_5000_fields1k_latency_cgroup")
                src = "reused:agent A wcm-tw128 run"
            r = parse_run(d)
            if r is None:
                continue
            r.update(method="M3_time_window", product=p, dut_label=LABEL[p], W_s=w, rate_eps=5000,
                     knobs=m3_knobs(p, w), source=src)
            if isinstance(r.get("p99_ms"), float):
                r["excess_p99_ms"] = round(r["p99_ms"] - 0.99 * w * 1000, 1)
            if w == 1.0 and p in formal_w1 and src == "new":
                attach_formal(r, formal_w1[p])
            rows.append(r)
    return rows


FORMAL_CAP = {  # W = 1.0 s 正式容量（時間主導，已核對 sink POST 頻率），3 輪 PASS
    "fbit-lua": (67500, 68000, "capacity/runs/fbit-lua/cap_rate_67500_fields1k_capacity_manual_r{1,2,3}_20260809"),
    "fluentd": (52000, 52500, "capacity/runs/fluentd/cap_rate_52000_fields1k_capacity_manual_r{1,2,3}_20260809"),
    "vector": (15000, 15500, "capacity/runs/vector/cap_rate_15000_fields1k_capacity_manual_r{1,2,3}_20260809"),
    "fbit-wasm": (5400, 5500, "capacity/runs/fbit-wasm/cap_rate_5400_fields1k_capacity_manual_r{1,2,3}_20260809"),
    "wcm": (25000, 25500, "capacity/runs/wcm-b1/cap_rate_25000_fields1k_capacity_manual_r{1,2,3}_20260809 (wcm-b1, NOT time-dominated)"),
}


def method3_capacity():
    rows = []
    for p in PRODUCTS:
        for w in W_LIST:
            f = RUNS / p / f"search_{wtag(w)}.txt"
            row = {"method": "M3_time_window", "product": p, "dut_label": LABEL[p], "W_s": w, "knobs": m3_knobs(p, w)}
            if w == 1.0:
                row["formal_capacity_eps"], _, row["formal_source"] = FORMAL_CAP[p]
            if f.exists():
                t = f.read_text()
                trials = re.findall(r"trial rate=(\d+) (PASS|FAIL)", t)
                m = re.findall(r"CAPACITY .*eps=(\d+) first_fail=(\d+)", t)
                if m:
                    row.update(capacity_eps=int(m[-1][0]), first_fail_eps=int(m[-1][1]))
                else:
                    row["note"] = "search incomplete / unbracketed"
                row.update(trials=" ".join(f"{r}:{v}" for r, v in trials), source="new (coarse search, 1 trial/rate)")
            elif w == 1.0 and p != "wcm":
                cap, ff, src = FORMAL_CAP[p]
                row.update(capacity_eps=cap, first_fail_eps=ff, trials="3x PASS (formal)", source="reused:" + src)
            else:
                continue
            rows.append(row)
    return rows


def method2():
    rows = []
    for rate in M2_RATES:
        weq = 1048576 / (rate * 1024)
        for p in PRODUCTS:
            formal = None
            if p == "wcm":
                formal = CAP / f"wcm-b1/lat_rate_{rate}_fields1k_latency_cgroup_rerun"
                knobs = "buffer_size_mb=1 (input bytes), max_wait_ms=1000, max_transport_bytes=1MiB (= formal wcm-b1)"
            else:
                knobs = {
                    "fluentd": "chunk_limit_records=432 (full at 0.95x => ~410 records), chunk_limit_size=128m, flush_interval=10, flush_thread_interval=0.01",
                    "vector": "batch.max_events=410, batch.max_bytes=128MiB, batch.timeout_secs=10",
                    "fbit-lua": f"Flush={weq:.4f} (W_eq, time only)",
                    "fbit-wasm": f"Flush={weq:.4f} (W_eq, time only)",
                }[p]
            d = first_existing(RUNS / p / f"lat_rate_{rate}_m2_vol1MiB")
            src = "new"
            if d is None and formal is not None:
                d, src = formal, "reused:formal wcm-b1 sweep"
            r = parse_run(d)
            if r is None:
                continue
            r.update(method="M2_batch_volume", variant="input_1MiB", product=p, dut_label=LABEL[p],
                     rate_eps=rate, W_eq_s=round(weq, 4), knobs=knobs, source=src)
            if isinstance(r.get("p99_ms"), float):
                r["excess_p99_ms"] = round(r["p99_ms"] - 0.99 * weq * 1000, 1)
            if formal is not None and src == "new":
                attach_formal(r, formal)
            rows.append(r)
        # 對照：正式 sweep 的 1 MiB「輸出」size trigger（Fluentd / Vector），全部重用
        for p, d, knobs in (
            ("fluentd", CAP / f"fluentd/lat_rate_{rate}_fields1k_latency_sweep",
             "chunk_limit_size=1m (output bytes), flush_interval=1 (formal latency conf)"),
            ("vector", CAP / f"vector/lat_rate_{rate}_fields1k_latency_cgroup_sweep",
             "batch.max_bytes=1MiB (output bytes), timeout_secs=1 (formal latency conf)"),
        ):
            r = parse_run(d)
            if r is None:
                continue
            r.update(method="M2_batch_volume", variant="output_1MiB_formal", product=p, dut_label=LABEL[p],
                     rate_eps=rate, W_eq_s=round(weq, 4), knobs=knobs, source="reused:formal sweep")
            rows.append(r)
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
    write("method3_latency_5000eps.csv", method3_latency(), FIELDS_LAT)
    write("method3_capacity.csv", method3_capacity(),
          ["method", "product", "dut_label", "W_s", "capacity_eps", "first_fail_eps", "knobs", "trials", "source",
           "formal_capacity_eps", "formal_source", "note"])
    write("method2_batch_volume.csv", method2(),
          ["method", "variant", "product", "dut_label", "rate_eps", "W_eq_s"] + FIELDS_LAT[5:])
