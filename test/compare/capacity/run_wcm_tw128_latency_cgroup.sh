#!/bin/bash
# item 1 公平性：WCM 時間窗主導（wcm-tw128）1024 B/36-field 延遲 sweep。
#
# 與 run_wcm_fields1k_latency_cgroup_rerun.sh 完全相同的流程、EPS 點、分析腳本、
# 容器 1 CPU/512 MiB 限制與 generator 設定，差別只有 DUT 變體 wcm-tw128
# （forwarder_membound_tw128.yaml：buffer 128 MB、1 秒時間窗），用來和 wcm-b1
# （buffer 1 MB、約 200 ms size-triggered flush）在相同負載下對照 flush policy 的影響。
#
# 使用既有 image tag wcm-base-log-forwarder:memguard（與先前 WCM 數字可比），
# 本腳本不重建任何 image。每個 EPS 點在 /home/ckt1022/.wcm-bench.lock 內獨占執行。
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VARIANT="${VARIANT_OVERRIDE:-fields1k_latency_cgroup}"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
if [[ -n "${RATES_OVERRIDE:-}" ]]; then
    read -r -a RATES <<< "$RATES_OVERRIDE"
else
    # 與 wcm-b1 正式 sweep 的 PASS 區間對齊
    RATES=(2500 5000 7500 10000 12500 15000 17500 20000 22250)
fi

for rate in "${RATES[@]}"; do
    echo "[tw128] ===== WCM tw128 cgroup rate=${rate} EPS ====="
    flock "$LOCK" env SKIP_CLOCK_CHECK=1 GEN_MODE=json-fields1k RUN_VARIANT="$VARIANT" \
        LOAD_SECS=180 DRAIN_SECS=20 \
        bash "$SCRIPT_DIR/capacity.sh" wcm-tw128 "$rate" lat
    rc=$?

    run_dir="$SCRIPT_DIR/runs/wcm-tw128/lat_rate_${rate}_${VARIANT}"
    if [[ -s "$run_dir/latency_worker_1.csv" ]]; then
        python3 "$SCRIPT_DIR/calc_latency_stream.py" \
            "$run_dir/latency_worker_1.csv" 30 150 \
            | tee "$run_dir/latency_stats_30_150.txt"
    fi
    if [[ -s "$run_dir/stats.csv" ]]; then
        awk -F, 'NR>1 && $1>=30 && $1<=150 {n++; cpu+=$2; mem+=$5; if($2>cm)cm=$2; if($5>mm)mm=$5}
            END{if(n)printf "[stats] window=30-150s samples=%d cpu_mean=%.3f%% cpu_max=%.3f%% mem_mean=%.3fMiB mem_max=%.3fMiB\n",n,cpu/n,cm,mem/n,mm}' \
            "$run_dir/stats.csv" | tee "$run_dir/resource_stats_30_150.txt"
    fi
    if (( rc != 0 )); then
        echo "[tw128] rate=${rate} FAILED; stopping"
        break
    fi
done
