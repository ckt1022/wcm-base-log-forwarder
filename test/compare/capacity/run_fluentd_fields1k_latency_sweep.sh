#!/bin/bash
# Fluentd 1024 B/36-field latency sweep, 2500 EPS steps until first capacity failure.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
START_RATE="${START_RATE:-2500}"
STEP="${STEP:-2500}"
MAX_RATE="${MAX_RATE:-60000}"
VARIANT="fields1k_latency_sweep"
CONF="$REPO_ROOT/test/compare/fluentd/latency/conf/capacity_1m_1s.conf"

rate="$START_RATE"
while (( rate <= MAX_RATE )); do
    echo "[sweep] ===== fluentd rate=${rate} EPS ====="
    SKIP_CLOCK_CHECK=1 GEN_MODE=json-fields1k RUN_VARIANT="$VARIANT" \
        LOAD_SECS=180 DRAIN_SECS=20 \
        CONF_OVERRIDE="$CONF" \
        "$SCRIPT_DIR/capacity.sh" fluentd "$rate" lat
    rc=$?

    run_dir="$SCRIPT_DIR/runs/fluentd/lat_rate_${rate}_${VARIANT}"
    if [[ -s "$run_dir/latency_worker_1.csv" ]]; then
        python3 "$SCRIPT_DIR/calc_latency_stream.py" \
            "$run_dir/latency_worker_1.csv" 30 150 \
            | tee "$run_dir/latency_stats_30_150.txt"
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

    if (( rc != 0 )); then
        echo "[sweep] fluentd rate=${rate} is the first failed stage; stopping"
        break
    fi
    rate=$(( rate + STEP ))
done

