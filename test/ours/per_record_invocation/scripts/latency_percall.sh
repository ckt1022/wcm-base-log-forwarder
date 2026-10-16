#!/bin/bash
# latency_percall.sh — test/compare/capacity/run_wcm_fields1k_latency_cgroup_rerun.sh 的複本。
# 條件逐字相同：json-fields1k、lat 模式、LOAD_SECS=180、DRAIN_SECS=20、
# 延遲取第 30–150 秒（calc_latency_stream.py，正式腳本唯讀使用）、container cgroup CPU/MEM。
# 差異：DUT 可選（percall 變體）、每個速率點單獨持有共用量測鎖、結果寫入本研究目錄、
#       不因單點 FAIL 而中止（每個變體只量少數指定速率）。
#
# 用法: latency_percall.sh <dut> <rate> [rate...]
#   環境變數 VARIANT（預設 fields1k_latency_cgroup）可區分重複量測。
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STUDY_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CAP_DIR="/home/ckt1022/wcm-base-log-forwarder/test/compare/capacity"
DUT="${1:?usage: latency_percall.sh <dut> <rate...>}"; shift
VARIANT="${VARIANT:-fields1k_latency_cgroup}"

for rate in "$@"; do
    echo "[sweep] ===== $DUT rate=${rate} EPS ====="
    SKIP_CLOCK_CHECK=1 GEN_MODE=json-fields1k RUN_VARIANT="$VARIANT" \
        LOAD_SECS=180 DRAIN_SECS=20 \
        flock /home/ckt1022/.wcm-bench.lock "$SCRIPT_DIR/capacity_percall.sh" "$DUT" "$rate" lat
    rc=$?

    run_dir="$STUDY_DIR/system/runs/$DUT/lat_rate_${rate}_${VARIANT}"
    if [[ -s "$run_dir/latency_worker_1.csv" ]]; then
        python3 "$CAP_DIR/calc_latency_stream.py" "$run_dir/latency_worker_1.csv" 30 150 \
            | tee "$run_dir/latency_stats_30_150.txt"
        # 原始逐筆延遲 CSV 很大（數百 MB），統計完成後壓縮保存
        gzip -f "$run_dir/latency_worker_1.csv"
    fi
    if [[ -s "$run_dir/stats.csv" ]]; then
        awk -F, '
            NR > 1 && $1 >= 30 && $1 <= 150 {
                n++; cpu += $2; mem += $5
                if ($2 > cpu_max) cpu_max = $2
                if ($5 > mem_max) mem_max = $5
            }
            END {
                if (n == 0) exit 1
                printf "[stats] window=30-150s samples=%d cpu_mean=%.3f%% cpu_max=%.3f%% mem_mean=%.3fMiB mem_max=%.3fMiB\n", n, cpu/n, cpu_max, mem/n, mem_max
            }
        ' "$run_dir/stats.csv" | tee "$run_dir/resource_stats_30_150.txt"
    fi
    echo "[sweep] rate=${rate} rc=${rc}"
done
