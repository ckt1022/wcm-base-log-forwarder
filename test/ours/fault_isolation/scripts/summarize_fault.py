#!/usr/bin/env python3
"""彙整 fault isolation 各 case 的觀測指標成 CSV（並畫吞吐時間軸 SVG）。

用法：summarize_fault.py <run_dir>
輸出：<run_dir>/summary.csv、<run_dir>/throughput_timeline.csv、<run_dir>/charts/throughput_timeline.svg（需 matplotlib）

對每個 case 目錄計算：
  host_survived        host process 是否在故障後仍存活（host_status.txt）
  records_sent         generator 送出的記錄數
  expected_received    generator 每兩筆一筆 warn（marker 本身也是 warn），filter 保留 warn+ → sent/2
  records_received     sink 收到的記錄數（recv_per_sec.csv 加總；sink 以 Go monotonic clock 分秒，不受 WSL2 wall-clock 跳躍影響）
  records_lost         expected − received
  isolated_salvaged／isolated_rejected／isolation_calls／trap_calls／trap_retries  （dut.log 的 [parse-isolate]、[trap n/m]）
  stall_zero_sec       故障後 sink 每秒收到 0 筆的秒數
  recovery_sec         故障後吞吐首次低於穩態 80% 起，到連續 3 秒回到 80% 以上的秒數（只看負載期間，1 s 解析度；0 = 沒有掉點）
  catchup_peak_per_sec 恢復時的單秒最大收到數（積壓一次送出）
  cpu_pre_median_pct   故障前（t=5–14 s）CPU 中位數
  cpu_fault_max_pct／cpu_fault_mean_pct  故障視窗（fault_at..fault_at+14 s）的 CPU
  extra_cpu_core_sec   故障視窗內高出故障前中位數的 CPU 時間（核心·秒）
  cpu_samples_excluded 被排除的取樣數（見下）
  rss_*                見下：本次原始資料的 RSS 無效，只保留供稽核
  error_txt_first      error.txt 第一行（拒絕原因摘要）
  stderr_summary       dut.log 中 plugin／host 錯誤訊息的去重摘要

取樣資料的已知問題（2026-10-14 稽核）：
  run_fault.sh 以 `sample_stats.sh ... tree` 取樣，而 tree 模式在修正前有 awk bug（讀 keep[x] 會建立空鍵），
  使「系統上所有父程序」都被加總：baseline 一開始 cpu_time_s 就是 974 s、RSS 約 3.7 GB。因此
    * RSS 絕對值無效（≈3.7 GB 幾乎全是無關程序）；
    * CPU% = DUT + 背景程序；背景程序出入取樣集合時會產生 ±140–170% 的假尖峰。
  本腳本排除 cpu_pct < 0 或 > 150、以及取樣間隔 > 2.2 s（疑似 wall-clock 跳躍，EPOCHREALTIME 受影響）的取樣，
  並以「故障視窗 − 故障前中位數」表示故障帶來的額外 CPU。sample_stats.sh 已修正，之後重跑的資料不再有此問題。
"""
import csv
import json
import re
import statistics
import sys
from pathlib import Path

CASE_ORDER = ["baseline", "loop", "mem", "panic", "oob", "file", "env"]


def read_recv(path: Path):
    rows = []
    if not path.exists():
        return rows
    with path.open() as h:
        r = csv.reader(h)
        next(r, None)
        for row in r:
            if len(row) >= 2:
                try:
                    rows.append((float(row[0]), int(row[1])))
                except ValueError:
                    pass
    return rows


def load_window(recv, secs):
    """sink 時間軸上的負載期間：第一個非零秒到 + secs − 1 秒。"""
    first = next((t for t, n in recv if n > 0), None)
    if first is None:
        return None, None
    return first, first + secs - 1


def recovery_seconds(recv, lo, hi, baseline):
    thresh = 0.8 * baseline
    win = [(t, n) for t, n in recv if lo <= t <= hi]
    dip_start = None
    for i, (t, n) in enumerate(win):
        if dip_start is None:
            if n < thresh:
                dip_start = t
        elif n >= thresh:
            nxt = [m for tt, m in win if t <= tt < t + 3]
            if len(nxt) == 3 and all(m >= thresh for m in nxt):
                return round(t - dip_start, 1)
    return 0.0 if dip_start is None else None


def read_stats(path: Path):
    out = []
    if not path.exists():
        return out
    prev = None
    with path.open() as h:
        for r in csv.DictReader(h):
            try:
                t, c, m = float(r["elapsed_s"]), float(r["cpu_pct"]), float(r["rss_mb"])
            except (KeyError, ValueError):
                continue
            dt = t - prev if prev is not None else 1.0
            prev = t
            out.append((t, c, m, dt))
    return out


def clean(s):
    return 0 <= s[1] <= 150 and s[3] <= 2.2


def stderr_summary(dut: str) -> str:
    pats = [
        r"panicked at [^\n]+\n[^\n]+",
        r"memory allocation of \d+ bytes failed",
        r"\[trap\] exceeded retries[^\n]*",
        r"\[capability\][^\n]*",
        r"\[SECURITY\][^\n]*",
    ]
    seen = []
    for p in pats:
        for m in re.findall(p, dut):
            m = re.sub(r"\s+", " ", m).strip()
            m = re.sub(r"batch \d+", "batch N", m)
            if m not in seen:
                seen.append(m)
    n_trap = len(re.findall(r"\[trap \d+/\d+\]", dut))
    if n_trap:
        seen.append(f"[trap n/m] x{n_trap} (wasm backtrace)")
    return " | ".join(seen) if seen else "(no error lines)"


def main():
    run = Path(sys.argv[1]).resolve()
    out_rows, timeline = [], []
    dirs = [p for p in run.iterdir() if p.is_dir() and (p / "generator_summary.json").exists()]
    dirs.sort(key=lambda p: CASE_ORDER.index(p.name) if p.name in CASE_ORDER else 99)
    for case_dir in dirs:
        gen = json.loads((case_dir / "generator_summary.json").read_text())
        secs = round(float(gen.get("elapsed_sec", 40)))
        fault_at = float(gen.get("fault_at_sec", 15))
        recv = read_recv(case_dir / "recv_per_sec.csv")
        received = sum(n for _, n in recv)
        dut = (case_dir / "dut.log").read_text(errors="replace") if (case_dir / "dut.log").exists() else ""
        err = case_dir / "error.txt"
        err_text = err.read_text(errors="replace").splitlines() if err.exists() else []
        host = (case_dir / "host_status.txt").read_text().strip() if (case_dir / "host_status.txt").exists() else "?"

        iso = re.findall(r"\[parse-isolate\].*?salvaged=(\d+) rejected=(\d+) calls=(\d+) trap_calls=(\d+)", dut)
        salvaged = sum(int(a) for a, *_ in iso)
        rejected = sum(int(b) for _, b, *_ in iso)
        calls = sum(int(c) for *_, c, _ in iso)
        trap_calls = sum(int(d) for *_, d in iso)
        retries = len(re.findall(r"\[trap \d+/\d+\]", dut))

        first, last = load_window(recv, secs)
        pre = [n for t, n in recv if first + 2 <= t < first + fault_at]
        baseline = statistics.median(pre) if pre else 0
        lo = first + fault_at - 1
        recovery = recovery_seconds(recv, lo, last, baseline)
        post = [(t, n) for t, n in recv if lo <= t <= last]
        zero = sum(1 for _, n in post if n == 0)
        peak = max((n for _, n in post), default=0)

        stats = read_stats(case_dir / "stats.csv")
        pre_s = [s for s in stats if 5 <= s[0] < fault_at and clean(s)]
        win_s = [s for s in stats if fault_at <= s[0] <= fault_at + 14]
        win_c = [s for s in win_s if clean(s)]
        cpu_pre = statistics.median(s[1] for s in pre_s) if pre_s else 0.0
        extra = sum(max(0.0, s[1] - cpu_pre) * s[3] / 100 for s in win_c)
        rss_pre = statistics.median(s[2] for s in pre_s) if pre_s else 0.0

        sent = int(gen.get("sent", 0))
        expected = sent // 2
        out_rows.append({
            "case": case_dir.name,
            "marker": gen.get("marker", ""),
            "host_survived": host,
            "records_sent": sent,
            "expected_received": expected,
            "records_received": received,
            "records_lost": expected - received,
            "isolated_salvaged": salvaged,
            "isolated_rejected": rejected,
            "isolation_calls": calls,
            "trap_calls": trap_calls,
            "trap_retries": retries,
            "steady_recv_per_sec_median": round(baseline, 1),
            "stall_zero_sec": zero,
            "recovery_sec": recovery if recovery is not None else "n/a",
            "catchup_peak_per_sec": peak,
            "cpu_pre_median_pct": round(cpu_pre, 1),
            "cpu_fault_max_pct": round(max(s[1] for s in win_c), 1) if win_c else "",
            "cpu_fault_mean_pct": round(statistics.fmean(s[1] for s in win_c), 1) if win_c else "",
            "extra_cpu_core_sec": round(extra, 2),
            "cpu_samples_excluded": len(win_s) - len(win_c),
            "rss_pre_median_mib_INVALID": round(rss_pre, 1),
            "rss_fault_max_minus_pre_mib_INVALID": round(max((s[2] for s in win_s), default=rss_pre) - rss_pre, 1),
            "error_txt_lines": len(err_text),
            "error_txt_first": err_text[0] if err_text else "",
            "stderr_summary": stderr_summary(dut),
        })
        for t, n in recv:
            if t <= last + 3:
                timeline.append({"case": case_dir.name, "sec_from_fault": round(t - (first + fault_at), 1), "records": n})

    out_csv = run / "summary.csv"
    with out_csv.open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=list(out_rows[0].keys()))
        w.writeheader()
        w.writerows(out_rows)
    tl_csv = run / "throughput_timeline.csv"
    with tl_csv.open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=["case", "sec_from_fault", "records"])
        w.writeheader()
        w.writerows(timeline)
    for r in out_rows:
        print(", ".join(f"{k}={v}" for k, v in r.items() if k not in ("error_txt_first", "stderr_summary")))
    print(f"\nwrote {out_csv}\nwrote {tl_csv}")

    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib 不存在，略過圖表（可用 test/tools/venv/bin/python 執行）")
        return
    colors = {"baseline": "#8a8985", "loop": "#2a78d6", "mem": "#eb6834", "panic": "#1baf7a", "oob": "#eda100"}
    fig, ax = plt.subplots(figsize=(8.5, 4.2))
    for case in ["baseline", "panic", "oob", "mem", "loop"]:
        pts = [(r["sec_from_fault"], r["records"]) for r in timeline if r["case"] == case and -8 <= r["sec_from_fault"] <= 20]
        if not pts:
            continue
        x, y = zip(*pts)
        ax.plot(x, y, lw=2, color=colors[case], label=case, drawstyle="steps-mid")
    ax.axvline(0, color="#52514e", lw=1, ls="--")
    ax.set_ylim(0, 6000)
    ax.text(0.2, 5700, "marker sent (approx.)", color="#52514e", fontsize=9, va="top")
    ax.set_xlabel("seconds from marker (sink monotonic clock, 1 s bins)")
    ax.set_ylabel("records received per second")
    ax.set_title("Throughput at the sink around a single poison record (5,000 EPS in, 50% kept)", fontsize=11)
    ax.set_ylim(0, 6000)
    ax.grid(axis="y", color="#e6e5e0", lw=0.8)
    for s in ("top", "right"):
        ax.spines[s].set_visible(False)
    lp = max((r["records"] for r in timeline if r["case"] == "loop"), default=0)
    if lp > 6000:
        lt = next(r["sec_from_fault"] for r in timeline if r["case"] == "loop" and r["records"] == lp)
        ax.annotate(f"loop catch-up {lp:,}/s (clipped)", xy=(lt, 5900), xytext=(lt + 1.5, 5000),
                    fontsize=9, color="#0b0b0b", arrowprops=dict(arrowstyle="->", color="#52514e"))
    ax.legend(frameon=False, ncol=5, loc="lower left", fontsize=9)
    fig.tight_layout()
    (run / "charts").mkdir(exist_ok=True)
    fig.savefig(run / "charts" / "throughput_timeline.svg")
    print(f"wrote {run / 'charts' / 'throughput_timeline.svg'}")


if __name__ == "__main__":
    main()
