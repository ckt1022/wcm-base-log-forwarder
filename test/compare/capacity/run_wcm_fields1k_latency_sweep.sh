#!/bin/bash
# WCM 1024 B/36-field latency sweep. The 2500 EPS stage may be run separately
# as a measurement-chain validation; START_RATE defaults to the next stage.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
START_RATE="${START_RATE:-5000}"
STEP="${STEP:-2500}"
MAX_RATE="${MAX_RATE:-30000}"
STOP_ON_FAIL="${STOP_ON_FAIL:-1}"
VARIANT="fields1k_latency_sweep"

rate="$START_RATE"
while (( rate <= MAX_RATE )); do
    echo "[sweep] ===== rate=${rate} EPS ====="
    SKIP_CLOCK_CHECK=1 GEN_MODE=json-fields1k RUN_VARIANT="$VARIANT" \
        LOAD_SECS=180 DRAIN_SECS=20 \
        "$SCRIPT_DIR/capacity.sh" wcm-b1 "$rate" lat
    rc=$?

    run_dir="$SCRIPT_DIR/runs/wcm-b1/lat_rate_${rate}_${VARIANT}"
    python3 "$SCRIPT_DIR/calc_latency_stream.py" \
        "$run_dir/latency_worker_1.csv" 30 150 \
        | tee "$run_dir/latency_stats_30_150.txt"

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

    if (( rc != 0 )) && [[ "$STOP_ON_FAIL" == "1" ]]; then
        echo "[sweep] rate=${rate} is the first stage beyond sustainable capacity; stopping"
        break
    elif (( rc != 0 )); then
        echo "[sweep] rate=${rate} failed; continuing because STOP_ON_FAIL=$STOP_ON_FAIL"
    fi
    rate=$(( rate + STEP ))
done
