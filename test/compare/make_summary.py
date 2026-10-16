#!/usr/bin/env python3
# make_summary.py — 彙整一批延遲實驗結果成單一 txt
# 用法：make_summary.py <實驗父資料夾> [附加條件說明...]
# 掃描 <父資料夾>/*/ 內的 <資料夾名>.csv（latency）與 <資料夾名>_stats.csv（資源取樣），
# 依名稱中的 flush 值排序，寫出 <父資料夾>/<父資料夾名>_summary.txt
# 延遲統計沿用 calc_latency.py 的 30~230 秒穩態窗口；
# 資源相對上限比例直接取 stats CSV 的 cpu_lim_pct / mem_pct（取樣當下的實際限制值）
import csv
import datetime
import glob
import os
import re
import subprocess
import sys

WINDOW = ("30", "230")


def main():
    if len(sys.argv) < 2:
        print("用法：make_summary.py <實驗父資料夾> [附加條件說明...]", file=sys.stderr)
        return 2
    base = os.path.abspath(sys.argv[1])
    extra = " ".join(sys.argv[2:])
    calc = os.path.join(os.path.dirname(os.path.abspath(__file__)), "calc_latency.py")
    parent = os.path.basename(base)
    out_path = os.path.join(base, f"{parent}_summary.txt")

    def flush_key(path):
        m = re.search(r"_([\d.]+)_", os.path.basename(path.rstrip("/")))
        return float(m.group(1)) if m else float("inf")

    dirs = sorted(glob.glob(os.path.join(base, "*/")), key=flush_key)

    lines = [f"{parent} 延遲實驗彙總",
             f"產生時間：{datetime.datetime.now():%Y-%m-%d %H:%M:%S}"]
    if extra:
        lines.append(f"條件：{extra}")
    lines.append(f"延遲統計取第 {WINDOW[0]}~{WINDOW[1]} 秒穩態窗口")
    lines.append("=" * 72)

    n_done = 0
    for d in dirs:
        name = os.path.basename(d.rstrip("/"))
        lat_csv = os.path.join(d, f"{name}.csv")
        stats_csv = os.path.join(d, f"{name}_stats.csv")
        if not os.path.isfile(lat_csv):
            continue
        n_done += 1

        lines.append("")
        lines.append(f"── {name} {'─' * max(1, 50 - len(name))}")

        r = subprocess.run(["python3", calc, lat_csv, *WINDOW],
                           capture_output=True, text=True)
        lines += [l for l in r.stdout.rstrip().splitlines()
                  if not l.startswith("[calc] 來源")]

        if os.path.isfile(stats_csv):
            rows = list(csv.DictReader(open(stats_csv)))
            if rows:
                cpu = [float(x["cpu_pct"]) for x in rows]
                clim = [float(x["cpu_lim_pct"]) for x in rows]
                rss = [float(x["rss_mb"]) for x in rows]
                mp = [float(x["mem_pct"]) for x in rows]
                lines.append(f"[stats] 資源用量（{len(rows)} 筆取樣，每秒一筆）")
                lines.append(f"[stats]   cpu  mean = {sum(cpu)/len(cpu):6.1f} %   "
                             f"max = {max(cpu):6.1f} %（相對上限 max {max(clim):.1f}%）")
                lines.append(f"[stats]   rss  mean = {sum(rss)/len(rss):6.2f} MB  "
                             f"max = {max(rss):6.2f} MB（相對上限 max {max(mp):.1f}%）")
        else:
            lines.append("[stats] （無資源取樣檔）")

    if n_done == 0:
        print(f"[summary] {base} 下找不到任何實驗結果 CSV", file=sys.stderr)
        return 1

    with open(out_path, "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"[summary] 已彙整 {n_done} 輪 → {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
