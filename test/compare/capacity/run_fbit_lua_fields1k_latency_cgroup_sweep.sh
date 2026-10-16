#!/bin/bash
# Fluent Bit Lua 1024 B/36-field latency sweep using container cgroups.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
VARIANT="${VARIANT_OVERRIDE:-fields1k_latency_cgroup_sweep}"
CONF="$REPO_ROOT/test/compare/fluentbit/latency/conf/capacity_lua_1s.conf"
if [[ -n "${RATES_OVERRIDE:-}" ]]; then
    read -r -a RATES <<< "$RATES_OVERRIDE"
else
    RATES=(2500 5000 7500 10000 12500 15000 17500 20000 22500 25000 \
           30000 35000 40000 45000 50000 55000 60000 65000 70000)
fi

for rate in "${RATES[@]}"; do
    echo "[sweep] ===== Fluent Bit Lua cgroup rate=${rate} EPS ====="
    SKIP_CLOCK_CHECK=1 GEN_MODE=json-fields1k RUN_VARIANT="$VARIANT" \
        LOAD_SECS=180 DRAIN_SECS=20 CONF_OVERRIDE="$CONF" \
        "$SCRIPT_DIR/capacity.sh" fbit-lua "$rate" lat
    rc=$?

    run_dir="$SCRIPT_DIR/runs/fbit-lua/lat_rate_${rate}_${VARIANT}"
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
        echo "[sweep] rate=${rate} is the first failed stage; stopping"
        break
    fi
done
