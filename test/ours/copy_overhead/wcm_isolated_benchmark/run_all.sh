#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
rounds=${ROUNDS:-100}
warmup=${WARMUP:-10}
runs=${RUNS:-3}
interval=${INTERVAL:-20}
cpu=${CPU:-2}
read -r -a stages <<< "${STAGES:-parse filter format}"
timestamp=$(date +%Y%m%d_%H%M%S)
output_dir="$benchmark_dir/results/wcm_isolated_${timestamp}"

for value_name in rounds warmup runs interval cpu; do
    value=${!value_name}
    if [[ ! "$value" =~ ^[0-9]+$ ]]; then
        echo "ERROR: ${value_name^^} must be a non-negative integer: $value" >&2
        exit 2
    fi
done
if [[ "$rounds" -eq 0 || "$runs" -eq 0 || "${#stages[@]}" -eq 0 ]]; then
    echo "ERROR: ROUNDS, RUNS, and STAGES must be non-empty/non-zero" >&2
    exit 2
fi
for stage in "${stages[@]}"; do
    case "$stage" in
        parse|filter|format) ;;
        *) echo "ERROR: invalid stage in STAGES: $stage" >&2; exit 2 ;;
    esac
done

mkdir -p -- "$output_dir"
temporary=""
cleanup() {
    if [[ -n "$temporary" ]]; then
        rm -f -- "$temporary"
    fi
}
trap cleanup EXIT

total=$((${#stages[@]} * runs))
completed=0
expected=$((1 + 11 * rounds))
echo "Output directory: $output_dir" >&2
echo "Experiments: $total; interval: ${interval}s; CPU: $cpu; mode: isolated fresh-pool" >&2

for stage in "${stages[@]}"; do
    for run in $(seq 1 "$runs"); do
        output="$output_dir/${stage}_rust_isolated_run${run}.csv"
        temporary=$(mktemp "$output_dir/.${stage}-run${run}-XXXXXX.csv")
        echo "[$((completed + 1))/$total] stage=$stage run=$run/$runs" >&2
        CPU="$cpu" "$benchmark_dir/run_one.sh" \
            "$stage" 64 32768 "$rounds" "$warmup" > "$temporary"

        lines=$(wc -l < "$temporary")
        if [[ "$lines" -ne "$expected" ]]; then
            echo "ERROR: $stage expected $expected CSV lines, got $lines" >&2
            exit 1
        fi
        if ! awk -F, 'NR == 1 { for (i=1; i<=NF; i++) if ($i == "instance_mode") mode=i; next } mode == 0 || $mode != "fresh-pool" { exit 1 }' "$temporary"; then
            echo "ERROR: $stage contains a non-fresh sample" >&2
            exit 1
        fi
        if [[ "$stage" == format ]] && ! awk -F, 'NR == 1 { for (i=1; i<=NF; i++) if ($i == "marker_count") marker=i; next } marker == 0 || $marker != 2 { exit 1 }' "$temporary"; then
            echo "ERROR: format marker validation failed" >&2
            exit 1
        fi

        mv -- "$temporary" "$output"
        temporary=""
        completed=$((completed + 1))
        echo "  saved: $output" >&2
        if [[ "$completed" -lt "$total" && "$interval" -gt 0 ]]; then
            echo "  cooling down for ${interval}s..." >&2
            sleep "$interval"
        fi
    done
done

echo "All $total isolated experiments completed: $output_dir" >&2
