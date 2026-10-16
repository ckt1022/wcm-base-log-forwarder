#!/usr/bin/env python3
"""逐筆錯誤隔離（Item 4）前後的正確性比較：合併五個 run 目錄的 summary.csv。

用法（repo 根目錄）：
  python3 test/compare/correctness/summarize.py test/compare/correctness/runs/native_legacy_c3
  python3 test/compare/correctness/summarize.py test/compare/correctness/runs/native_isolation_c1c3c4
  python3 test/compare/correctness/summarize.py test/compare/correctness/runs/isolation_docker_20261014_002704
  test/tools/venv/bin/python test/compare/correctness/isolation_comparison/compare.py
（20260804 的兩個正式目錄只讀其既有 summary.csv，不重新產生。）
輸出：comparison.csv（每個 run 一列）、comparison_by_scenario.csv（每組 × 情境彙總）、charts/c3_before_after.svg
"""
import csv
import statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUNS = HERE.parent / "runs"
GROUPS = [
    # (label, 目錄, 執行環境, 程式版本)
    ("container_before", "wcm_correctness_formal_20260804", "container 1 CPU/512 MiB", "pre-change (memguard image)"),
    ("container_before", "wcm_correctness_edge_20260804", "container 1 CPU/512 MiB", "pre-change (memguard image)"),
    ("native_legacy", "native_legacy_c3", "native, taskset dut=4", "new binary, parse_isolation=false"),
    ("native_isolation", "native_isolation_c1c3c4", "native, taskset dut=4", "new binary, parse_isolation=true"),
    ("container_isolation", "isolation_docker_20261014_002704", "container 1 CPU/512 MiB", "isolation image (new code)"),
]
FIELDS = ["group", "run_dir", "environment", "code", "scenario", "repeat", "status", "expected_output", "received",
          "missing", "duplicate", "corrupt", "reorder", "parse_error_batches", "isolated_batches", "salvaged_lines",
          "rejected_lines", "isolation_calls", "cpu_mean_pct", "cpu_max_pct", "mem_mean_mib", "mem_max_mib", "oom"]


def main():
    rows = []
    for label, d, env, code in GROUPS:
        with (RUNS / d / "summary.csv").open() as h:
            for r in csv.DictReader(h):
                out = {"group": label, "run_dir": d, "environment": env, "code": code}
                for f in FIELDS[4:]:
                    out[f] = r.get(f, "")
                rows.append(out)
    with (HERE / "comparison.csv").open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)

    agg = []
    keys = []
    for r in rows:
        k = (r["group"], r["scenario"])
        if k not in keys:
            keys.append(k)
    for g, sc in keys:
        rs = [r for r in rows if r["group"] == g and r["scenario"] == sc]
        agg.append({
            "group": g, "scenario": sc, "environment": rs[0]["environment"], "reps": len(rs),
            "pass": sum(r["status"] == "PASS" for r in rs),
            "expected_total": sum(int(r["expected_output"]) for r in rs),
            "received_total": sum(int(r["received"]) for r in rs),
            "missing_total": sum(int(r["missing"]) for r in rs),
            "dup_corrupt_reorder_total": sum(int(r["duplicate"]) + int(r["corrupt"]) + int(r["reorder"]) for r in rs),
            "rejected_lines_per_run": "/".join(r["rejected_lines"] or "-" for r in rs),
            "isolation_calls_per_run": "/".join(r["isolation_calls"] or "-" for r in rs),
            "cpu_mean_pct_avg": round(statistics.fmean(float(r["cpu_mean_pct"]) for r in rs), 2),
            "mem_max_mib_max": round(max(float(r["mem_max_mib"]) for r in rs), 2),
        })
    with (HERE / "comparison_by_scenario.csv").open("w", newline="") as h:
        w = csv.DictWriter(h, fieldnames=list(agg[0].keys()))
        w.writeheader()
        w.writerows(agg)
    for a in agg:
        print(a)

    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib 不存在，略過圖表")
        return
    c3 = [a for a in agg if a["scenario"] == "C3"]
    names = {"container_before": "container\nbefore (08-04)", "native_legacy": "native\nlegacy mode",
             "native_isolation": "native\nisolation", "container_isolation": "container\nisolation (10-14)"}
    fig, ax = plt.subplots(figsize=(7.5, 3.8))
    xs = range(len(c3))
    vals = [a["received_total"] / a["reps"] for a in c3]
    cols = ["#eb6834" if v == 0 else "#2a78d6" for v in vals]
    ax.bar(xs, vals, width=0.55, color=cols)
    ax.axhline(38000, color="#52514e", lw=1, ls="--")
    ax.text(len(c3) - 0.5, 38600, "expected 38,000 per run", ha="right", fontsize=9, color="#52514e")
    for x, v, a in zip(xs, vals, c3):
        ax.text(x, v + 600 if v else 600, f"{v:,.0f}\n({a['pass']}/{a['reps']} PASS)", ha="center", fontsize=9, color="#0b0b0b")
    ax.set_xticks(list(xs), [names[a["group"]] for a in c3], fontsize=9)
    ax.set_ylabel("records delivered per run (mean of 3)")
    ax.set_ylim(0, 45000)
    ax.set_title("C3 (5% malformed JSON): delivered records before vs after per-record isolation", fontsize=10)
    ax.grid(axis="y", color="#e6e5e0", lw=0.8)
    for s in ("top", "right"):
        ax.spines[s].set_visible(False)
    fig.tight_layout()
    (HERE / "charts").mkdir(exist_ok=True)
    fig.savefig(HERE / "charts" / "c3_before_after.svg")
    print("wrote", HERE / "charts" / "c3_before_after.svg")


if __name__ == "__main__":
    main()
