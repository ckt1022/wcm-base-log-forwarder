#!/usr/bin/env python3
"""系統層 no-WASM baseline 彙整：WCM vs 原生的 5,000 EPS 延遲與容量階梯。

用法（repo 根目錄）：
  test/tools/venv/bin/python test/ours/baselines/scripts/summarize_system.py \
      test/ours/baselines/results/system_system_20261013_235508 \
      [test/ours/baselines/results/micro_20261013_181014]
輸出（寫在 system 目錄）：
  system_latency.csv    lat 模式：原始與 stepdetect 校正後的 p50/p90/p99/p99.9、偵測到的階梯、CPU/RSS
  system_capacity.csv   cap 模式：每個 variant × rate 的 PASS/FAIL、失敗原因、輸出 EPS、CPU/RSS
  system_max_pass.csv   每個 variant 的最大 PASS rate 與第一個 FAIL rate
  charts/capacity_ladder.svg
若給第二個參數，另輸出 <micro_dir>/five_method_1024_tidy.csv（長表：stage × method 的 1024 筆數字）。

延遲：每筆 = sink 收到的 wall time − generator 時戳（sched），窗 30–140 s（與 run_system_capacity.sh 相同）。
以 test/compare/batch_alignment/stepdetect.py 偵測 WSL2 wall-clock 階梯並扣除；
sink 的 recv_per_sec 與 eval_trial 判定不受影響（send-lag 用 monotonic）。
"""
import csv
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO / "test" / "compare" / "batch_alignment"))
import stepdetect as sd  # noqa: E402


def kv(text):
    return dict(re.findall(r"(\w+)=([^\s]+)", text))


def resource(d: Path):
    p = d / "resource_stats.txt"
    return kv(p.read_text()) if p.exists() else {}


def main():
    sysdir = Path(sys.argv[1]).resolve()
    lat_rows, cap_rows = [], []
    for d in sorted(p for p in sysdir.iterdir() if p.is_dir() and (p / "verdict.txt").exists()):
        variant, mode, rate = d.name.split("_")
        rate = int(rate)
        vtxt = (d / "verdict.txt").read_text().strip().splitlines()[0]
        v = kv(vtxt)
        res = resource(d)
        base = {
            "variant": variant, "rate_eps": rate, "verdict": vtxt.split()[0],
            "fail_reasons": v.get("reasons", ""),
            "gen_avg_eps": v.get("gen_avg", "").rstrip("/s"),
            "gen_lag_p99_ms": v.get("lag_p99", "").rstrip("ms"),
            "backlog": v.get("backlog", ""),
            "win_out_eps": v.get("win_out_eps", ""),
            "cpu_mean_pct": res.get("cpu_mean", "").rstrip("%"),
            "cpu_max_pct": res.get("cpu_max", "").rstrip("%"),
            "rss_mean_mib": res.get("rss_mean", "").replace("MiB", ""),
            "rss_max_mib": res.get("rss_max", "").replace("MiB", ""),
        }
        if mode == "lat":
            recv, lat = sd.load_latency(str(d))
            r = sd.correct_steps(recv, lat)
            raw = sd.window_stats(r["t"], r["raw"], 30, 140)
            cor = sd.window_stats(r["t"], r["corr"], 30, 140)
            sub = sd.subwindows(r["t"], r["corr"])
            row = dict(base)
            row.update({
                "window_s": "30-140", "n": cor["n"],
                "raw_p50_ms": round(raw["p50"], 2), "raw_p99_ms": round(raw["p99"], 2),
                "corr_p50_ms": round(cor["p50"], 2),
                "corr_p90_ms": round(sd.pct(r["corr"][(r["t"] >= 30) & (r["t"] <= 140)], 90), 2),
                "corr_p99_ms": round(cor["p99"], 2), "corr_p999_ms": round(cor["p999"], 2),
                "corr_max_ms": round(cor["max"], 2),
                "steps_detected": len(r["step_sizes"]),
                "step_sizes_ms": "/".join(f"{s:.0f}" for s in r["step_sizes"]),
                "detector_warnings": "; ".join(r["warnings"]),
                "latency_stable": sd.stable_flag(sub),
                "subwindow_p99_ms": "/".join(f"{s}:{p99:.0f}" for s, _, p99 in sub),
            })
            lat_rows.append(row)
        else:
            cap_rows.append(base)

    def write(name, rows):
        if not rows:
            return
        with (sysdir / name).open("w", newline="") as h:
            w = csv.DictWriter(h, fieldnames=list(rows[0].keys()))
            w.writeheader()
            w.writerows(rows)
        print("wrote", sysdir / name)

    lat_rows.sort(key=lambda r: r["variant"])
    cap_rows.sort(key=lambda r: (r["variant"], r["rate_eps"]))
    write("system_latency.csv", lat_rows)
    write("system_capacity.csv", cap_rows)

    mx = []
    for variant in sorted({r["variant"] for r in cap_rows}):
        rs = [r for r in cap_rows if r["variant"] == variant]
        passes = [r["rate_eps"] for r in rs if r["verdict"] == "PASS"]
        fails = [r["rate_eps"] for r in rs if r["verdict"] == "FAIL"]
        max_pass = max(passes) if passes else None
        first_fail = min((f for f in fails if max_pass is None or f > max_pass), default=None)
        non_mono = [f for f in fails if max_pass is not None and f < max_pass]
        mx.append({
            "variant": variant, "max_pass_eps": max_pass, "first_fail_above_eps": first_fail,
            "capacity_interval": f"[{max_pass}, {first_fail})",
            "first_fail_reason": next((r["fail_reasons"] for r in rs if r["rate_eps"] == first_fail), ""),
            "saturated_out_eps_max": max(float(r["win_out_eps"]) for r in rs),
            "non_monotonic_fails": "/".join(map(str, non_mono)),
            "trials_per_rate": 1, "load_secs": 60,
        })
    write("system_max_pass.csv", mx)
    for r in lat_rows + mx:
        print(r)

    if len(sys.argv) > 2:
        mdir = Path(sys.argv[2]).resolve()
        src = mdir / "five_method_comparison.csv"
        methods = [("wcm", "WCM Canonical ABI"), ("core_json", "Core Wasm JSON ABI"),
                   ("core_direct", "Core Wasm Direct Memory"), ("native", "Native Rust (in-process)"),
                   ("subprocess", "Subprocess + pipe")]
        out = []
        with src.open() as h:
            for r in csv.DictReader(h):
                if r["batch_count"] != "1024":
                    continue
                for key, label in methods:
                    out.append({
                        "stage": r["stage"], "method": key, "method_label": label, "batch_count": 1024,
                        "copy_in_ms": round(float(r[f"{key}_copy_in_median_ns"]) / 1e6, 3),
                        "logic_ms": round(float(r[f"{key}_guest_median_ns"]) / 1e6, 3),
                        "copy_out_ms": round(float(r[f"{key}_copy_out_median_ns"]) / 1e6, 3),
                        "total_median_ms": round(float(r[f"{key}_total_median_ns"]) / 1e6, 3),
                        "total_p95_ms": round(float(r[f"{key}_total_p95_ns"]) / 1e6, 3),
                        "boundary_share_pct": round(float(r[f"{key}_boundary_share_percent"]), 1),
                        "total_over_native": round(float(r[f"{key}_total_median_ns"]) / float(r["native_total_median_ns"]), 1),
                        "data_date": "2026-10-13" if key in ("native", "subprocess") else "2026-08-15..17 (copy_overhead)",
                    })
        with (mdir / "five_method_1024_tidy.csv").open("w", newline="") as h:
            w = csv.DictWriter(h, fieldnames=list(out[0].keys()))
            w.writeheader()
            w.writerows(out)
        print("wrote", mdir / "five_method_1024_tidy.csv")

    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib 不存在，略過圖表")
        return
    fig, ax = plt.subplots(figsize=(7.5, 4.2))
    colors = {"native": "#eb6834", "wcm": "#2a78d6"}
    labels = {"native": "native (built-in Rust stages)", "wcm": "WCM (Rust plugins)"}
    for variant in ["wcm", "native"]:
        rs = [r for r in cap_rows if r["variant"] == variant]
        x = [r["rate_eps"] / 1000 for r in rs]
        y = [float(r["win_out_eps"]) / 0.4 / 1000 for r in rs]   # 輸出 EPS ÷ filter 保留率 0.4 = 等效輸入 EPS
        ax.plot(x, y, color=colors[variant], lw=2, label=labels[variant])
        for xi, yi, r in zip(x, y, rs):
            ok = r["verdict"] == "PASS"
            ax.plot(xi, yi, marker="o" if ok else "X", ms=8, color=colors[variant],
                    markerfacecolor=colors[variant] if ok else "#fcfcfb", markeredgewidth=2)
    ax.plot([0, 100], [0, 100], color="#b0afa9", lw=1, ls="--")
    ax.text(78, 66, "offered = processed", color="#52514e", fontsize=9, rotation=33)
    ax.set_xlabel("offered rate (k EPS, 60 s trial, one trial per rate)")
    ax.set_ylabel("processed rate (k EPS, = sink output / 0.4)")
    ax.set_title("Capacity ladder, native process pinned to 1 core: filled = PASS, X = FAIL", fontsize=10)
    ax.set_xlim(0, 105)
    ax.set_ylim(0, 105)
    ax.grid(color="#e6e5e0", lw=0.8)
    for s in ("top", "right"):
        ax.spines[s].set_visible(False)
    ax.legend(frameon=False, loc="upper left", fontsize=9)
    fig.tight_layout()
    (sysdir / "charts").mkdir(exist_ok=True)
    fig.savefig(sysdir / "charts" / "capacity_ladder.svg")
    print("wrote", sysdir / "charts" / "capacity_ladder.svg")


if __name__ == "__main__":
    main()
